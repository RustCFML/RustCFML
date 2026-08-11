//! Optional multi-tier JIT for the RustCFML VM (Cranelift backend).
//!
//! This whole module only compiles under `--features jit` on a non-wasm32
//! target (the Cargo manifest gates the cranelift crates off wasm32, and the
//! `mod jit;` declaration in `lib.rs` is `#[cfg(feature = "jit")]`). The
//! stack-based interpreter in `lib.rs` is always the default execution engine
//! and the universal fallback — the JIT is a pure, opt-in optimisation.
//!
//! # Tiering
//! * **Tier 0** — the interpreter (unchanged).
//! * **Tier 1** (this module) — *integer numeric kernels*: a whole
//!   [`BytecodeFunction`] is compiled to native code iff a static scan proves
//!   every op is in a supported, side-effect-free, integer-only subset
//!   (arithmetic + counted loops). Anything else stays on the interpreter.
//!
//! # Correctness model
//! The JIT never changes observable behaviour. A Tier-1 function is pure (no
//! calls, output, globals, throws, or heap mutation), so when native execution
//! hits something it can't represent exactly (currently only divide-by-zero) it
//! sets a *bail* flag and [`JitEngine::try_call`] returns `None`; the caller then
//! runs the interpreter on the same `(func, args)` from scratch. Re-running a
//! pure function yields an identical result. Integer arithmetic uses wrapping
//! i64 ops, bit-exact with the interpreter's `CfmlValue::Int(i + j)` in release
//! builds. See `JIT_DESIGN.md` for the full rationale and gotchas.

use cfml_codegen::{BytecodeFunction, BytecodeOp};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlResult;
use rustc_hash::FxHashMap;
use std::cell::Cell;
use std::collections::HashMap;
use std::ptr;
use std::sync::{Arc, RwLock};

mod analysis;
pub(super) mod arena;
mod boxed;
mod builtins;
pub mod coverage;
mod osr;
mod shims;
mod translate;

pub use analysis::{BindingRet, UdfRefBinding};

/// ABI of every Tier-1 compiled function.
///
/// * `args`  — pointer to `nargs` little-endian `i64`s. Each slot's bit
///   pattern depends on the param's kind for this specialization:
///   * `Int`   — the unwrapped `i64`.
///   * `Float` — the `f64`'s `to_bits()` reinterpretation.
///   * `Boxed` — an Option-γ tagged `usize` (v0.89.0: always `TAG_PTR`, i.e.
///     a raw `*mut CfmlValue` produced by [`boxed::box_value`]).
/// * `nargs` — number of valid entries behind `args`.
/// * `bail`  — out-param; the callee stores `1` here to signal *deopt* (fall
///   back to the interpreter) and `0` (or leaves it untouched) on success.
///
/// Returns an `i64`, valid only when `*bail == 0`. The return's interpretation
/// is governed by the per-cache-entry [`RetKind`]: `Int`/`Float` as above, and
/// `Boxed` as a tagged `usize` reinterpreted as an `i64` (engine reclaims via
/// [`boxed::reclaim_tagged`]).
pub type CompiledFn = unsafe extern "C" fn(args: *const i64, nargs: i64, bail: *mut i64) -> i64;

/// Tri-state return kind for a compiled body, used at the outer ABI boundary
/// to re-interpret the `i64` result the compiled function hands back. The
/// UDF→UDF dispatcher only ever handles `Int`/`Float` callees (see
/// [`udf_binding_still_valid`]); a `Boxed` return is reachable only via the
/// outermost `try_call` entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RetKind {
    Int,
    Float,
    Boxed,
}

impl RetKind {
    fn from_analysis(k: analysis::Kind) -> Self {
        match k {
            analysis::Kind::Int => RetKind::Int,
            analysis::Kind::Float => RetKind::Float,
            analysis::Kind::Boxed => RetKind::Boxed,
            _ => unreachable!("ret_kind validated to Int/Float/Boxed by analyser"),
        }
    }
    /// 0/1/2 encoding matching the `expected_ret_kind` argument the JIT'd
    /// caller passes to `cfml_call_jit_udf`.
    fn as_code(self) -> i64 {
        match self {
            RetKind::Int => 0,
            RetKind::Float => 1,
            RetKind::Boxed => 2,
        }
    }

    fn to_binding(self) -> BindingRet {
        match self {
            RetKind::Int => BindingRet::Int,
            RetKind::Float => BindingRet::Float,
            RetKind::Boxed => BindingRet::Boxed,
        }
    }
}

/// Per-function compilation outcome. A `global_id` absent from the cache simply
/// hasn't crossed the hotness threshold yet.
enum CacheEntry {
    /// The static analysis rejected this function; never attempt it again.
    Unjittable,
    /// Successfully compiled; call this pointer.
    /// * `ret_float` — `true` when the function's return kind is `Float`
    ///   (the `i64` the body returns is the `f64` result's bit pattern,
    ///   re-wrapped as `CfmlValue::Double`).
    /// * `builtins` — every allowlisted builtin name the compiled body
    ///   calls; the engine re-checks each one against the live VM on every
    ///   invocation so a runtime `function abs(x) { … }` still wins.
    /// * `udfs` — every UDF callee the compiled body references
    ///   (deduped by `(global_id, sig)`). The engine re-checks each at every
    ///   invocation so a stale binding (callee re-registered, recompiled
    ///   with a different sig, or evicted) forces a fallback to the
    ///   interpreter for the whole call.
    Compiled {
        ptr: CompiledFn,
        ret_kind: RetKind,
        builtins: Vec<&'static str>,
        udfs: Vec<UdfRefBinding>,
    },
}

/// Invocation counters keyed by `(global_id, signature)` — each call
/// specialization warms up independently. A `(func, kinds)` pair becomes a
/// compilation candidate once its count reaches `threshold`.
struct HotnessTracker {
    counts: FxHashMap<CacheKey, u32>,
    threshold: u32,
}

impl HotnessTracker {
    fn new(threshold: u32) -> Self {
        Self { counts: FxHashMap::default(), threshold }
    }

    /// Record one invocation of `key`; return `true` exactly once, on the call
    /// that crosses the threshold, so compilation is attempted a single time.
    /// After that the count is parked at `threshold + 1`.
    fn record_and_is_hot(&mut self, key: CacheKey) -> bool {
        let c = self.counts.entry(key).or_insert(0);
        if *c > self.threshold {
            return false;
        }
        *c += 1;
        *c == self.threshold + 1
    }
}

/// Composite cache key: `(global_id, signature)` where `signature` packs
/// `(nargs, kind-bitmap)` so each `(func, param-kind-tuple)` specialization
/// caches independently. v0.89.0 widens the encoding from a 1-bit float
/// bitmap (32 max params) to a 2-bit kind tuple (16 max params) admitting a
/// third kind, `Boxed`. Code:
/// * `0b00` → `Int`
/// * `0b01` → `Float`
/// * `0b10` → `Boxed` (any non-numeric `CfmlValue`, crossed as a tagged
///   `usize` per [`boxed`])
/// * `0b11` → reserved
type CacheKey = (u32, u64);

const MAX_JIT_PARAMS: usize = 16;

const KIND_INT: u64 = 0b00;
const KIND_FLOAT: u64 = 0b01;
const KIND_BOXED: u64 = 0b10;

fn pack_sig<I: ExactSizeIterator<Item = u64>>(kinds: I) -> Option<u64> {
    let nargs = kinds.len();
    if nargs > MAX_JIT_PARAMS {
        return None;
    }
    let mut packed: u64 = 0;
    for (i, k) in kinds.enumerate() {
        packed |= (k & 0b11) << (i * 2);
    }
    Some((packed << 32) | (nargs as u64))
}

/// Build the cache signature for a call.
fn signature_for(args: &[CfmlValue]) -> Option<u64> {
    let kinds = args.iter().map(|a| match a {
        CfmlValue::Int(_) => KIND_INT,
        CfmlValue::Double(_) => KIND_FLOAT,
        _ => KIND_BOXED,
    });
    pack_sig(kinds.collect::<Vec<_>>().into_iter())
}

/// Build the same cache signature directly from an analysis [`analysis::Kind`]
/// vector. Returns `None` if any kind isn't an ABI-admissible concrete kind
/// (`Int`/`Float`/`Boxed`).
fn sig_from_kinds(kinds: &[analysis::Kind]) -> Option<u64> {
    let codes: Option<Vec<u64>> = kinds
        .iter()
        .map(|k| match k {
            analysis::Kind::Int => Some(KIND_INT),
            analysis::Kind::Float => Some(KIND_FLOAT),
            analysis::Kind::Boxed => Some(KIND_BOXED),
            _ => None,
        })
        .collect();
    pack_sig(codes?.into_iter())
}

/// Static metadata about a user-defined function. Returned by the
/// `udf_lookup` callback passed into [`JitEngine::try_call`]. Lets the JIT
/// resolve a `LoadGlobal(name)` to a stable `(global_id, arity)` pair
/// without dragging the VM's `user_functions` map through this crate.
#[derive(Clone, Copy, Debug)]
pub struct UdfMeta {
    pub global_id: u32,
    pub nparams: usize,
}

/// Revalidate a UDF binding captured at compile time against the current
/// cache. Phase-2 (v0.87.0) relaxed this: a binding can survive when the
/// callee is not yet compiled (the runtime dispatcher bails *this call*
/// only, not the whole function — and the binding becomes "live" the
/// moment the callee compiles).
///
/// * `Compiled` whose `ret_kind` matches the binding's expectation → valid.
/// * `Compiled` with a *different* `ret_kind` → invalid (would silently
///   miscompile if invoked; bail=2 in the dispatcher handles this at
///   runtime, but we also reject here so a steady-state mismatch doesn't
///   keep paying the dispatcher cost).
/// * `Unjittable` → still valid: the runtime bail keeps the caller correct
///   (bail=1 falls back to interpreter for this call only), and refusing to
///   admit the caller would force it onto the interpreter forever just
///   because one of its callees can't JIT.
/// * `None` (not yet compiled) → valid for the same reason: deferred
///   compilation of the callee is exactly the case Phase-2 unlocks.
fn udf_binding_still_valid(
    cache: &FxHashMap<CacheKey, CacheEntry>,
    binding: UdfRefBinding,
) -> bool {
    match cache.get(&(binding.global_id, binding.sig)) {
        Some(CacheEntry::Compiled { ret_kind, .. }) => {
            ret_kind.to_binding() == binding.ret_kind
        }
        Some(CacheEntry::Unjittable) | None => true,
    }
}

// ── UDF→UDF dispatch: thread-local engine pointer + libcall ──────────────────
//
// The compiled body of a JIT'd caller invokes `cfml_call_jit_udf` (defined
// in `translate.rs` as an `extern "C"`) for every UDF callsite. That
// dispatcher needs to consult the engine's cache, but the compiled function
// ABI doesn't carry an engine pointer. Instead, `try_call` stashes
// `*const JitEngine` in a thread-local for the duration of the compiled
// call (including any nested JIT→JIT recursion) and the dispatcher reads
// it. The pointer is immutable while a body is running: cache inserts only
// happen between outer `try_call` invocations (OSR is interpreter-driven,
// so it can't fire while a whole-function JIT body is executing).

thread_local! {
    static ENGINE_PTR: Cell<*const JitEngine> = const { Cell::new(ptr::null()) };
}

/// Stash `engine` as the active dispatch context, run `f`, then restore the
/// previous value. Restoring (rather than clearing) supports nested
/// `try_call` invocations from outside the JIT — e.g., a non-JIT'd CFML fn
/// calls a JIT'd helper which interpreter-calls back into the VM which
/// reaches another `try_call`. The outer pointer must be reinstated on
/// return; otherwise a re-entry up the chain would see a stale or null
/// pointer.
fn run_compiled_with_engine(
    engine: &JitEngine,
    f: CompiledFn,
    func: &BytecodeFunction,
    args: &[CfmlValue],
    ret_kind: RetKind,
) -> (Option<CfmlResult>, i64) {
    let prev = ENGINE_PTR.with(|c| c.replace(engine as *const JitEngine));
    let result = run_compiled(f, func, args, ret_kind);
    ENGINE_PTR.with(|c| c.set(prev));
    result
}

/// The actual dispatcher behind `cfml_call_jit_udf` in `translate.rs`. Looks
/// up `(global_id, sig)` in the active engine's cache. Bail semantics:
/// * `*bail = 0` — success, value returned to caller.
/// * `*bail = 1` — normal deopt: callee not yet compiled, callee is known
///   `Unjittable`, or the callee's own body bailed. Caller's cache entry is
///   left intact (this is a transient or per-call failure).
/// * `*bail = 2` — speculation mismatch: caller's `expected_ret_kind` does
///   not match the cached callee's `ret_kind`. The outer `try_call` evicts
///   the caller from the cache so it re-analyses on its next hot trip.
///
/// `expected_ret_kind` is the 0/1/2 encoding from [`RetKind::as_code`]:
/// `0=Int / 1=Float / 2=Boxed`. v0.90.1 widens this from the previous
/// boolean `expected_ret_float`.
pub(crate) fn dispatch_jit_udf(
    global_id: u32,
    sig: u64,
    expected_ret_kind: i64,
    args: *const i64,
    nargs: i64,
    bail: *mut i64,
) -> i64 {
    let engine_ptr = ENGINE_PTR.with(|c| c.get());
    if engine_ptr.is_null() {
        // Defensive: should never happen because the compiled body is only
        // called inside `run_compiled_with_engine`. Bail anyway so we
        // degrade gracefully.
        unsafe { *bail = 1 };
        return 0;
    }
    // SAFETY: `engine_ptr` was set by `run_compiled_with_engine` immediately
    // before the compiled body started running and is restored on return.
    // The engine outlives the call (it owns the JITModule and thus the
    // compiled code itself). The dispatcher only reads `cache`; no mutating
    // reentrancy is possible.
    let engine = unsafe { &*engine_ptr };
    match engine.cache.get(&(global_id, sig)) {
        Some(CacheEntry::Compiled { ptr, ret_kind, .. }) => {
            // Phase 2: caller may have speculatively bound this site with
            // `ret_kind = Int` (the default for a not-yet-compiled foreign
            // callee at analysis time). If the now-cached callee returns
            // a different kind, invoking it would silently miscompile
            // (the i64 result would be reinterpreted as the wrong kind).
            // Bail out with code 2 so the outer try_call evicts the caller.
            if ret_kind.as_code() != expected_ret_kind {
                unsafe { *bail = 2 };
                return 0;
            }
            unsafe { (*ptr)(args, nargs, bail) }
        }
        _ => {
            unsafe { *bail = 1 };
            0
        }
    }
}

/// Composite key for the per-loop OSR cache: `(global_id, region_start_ip)`.
/// One entry per (function, hot-loop site); a subsequent call that observes a
/// different kind layout for the same loop simply bails (no re-specialization).
type OsrKey = (u32, usize);

/// Successfully compiled OSR loop body — see [`osr::CompiledLoop`].
struct OsrCompiled {
    ptr: osr::CompiledLoop,
    slots: Vec<osr::OsrSlot>,
    exit_ip: usize,
    referenced_builtins: Vec<&'static str>,
    /// v0.91.0 — UDF callees referenced inside the compiled region. Each
    /// must still resolve to a Compiled cache entry with a matching
    /// `(global_id, sig, ret_kind)` at every outer invocation; drift evicts
    /// the OSR entry so it re-specialises.
    referenced_udfs: Vec<UdfRefBinding>,
}

enum OsrCacheEntry {
    Unjittable,
    Compiled(OsrCompiled),
}

/// The JIT engine, owned by the VM (one per VM instance; child cfthread VMs get
/// their own). Holds the Cranelift module that owns all executable memory, the
/// reusable compilation context, the per-function cache, and the profiler.
///
/// OSR (On-Stack Replacement) state lives alongside the whole-function cache:
/// hot loops in `__main__` (or any non-eligible enclosing function) compile
/// their body region to native code on a separate `osr_cache`, sharing the
/// single [`translate::Backend`] (and therefore the single JIT module, shim
/// symbols, and executable memory). See `JIT_OSR_DESIGN.md`.
pub struct JitEngine {
    cache: FxHashMap<CacheKey, CacheEntry>,
    hot: HotnessTracker,
    backend: translate::Backend,
    /// Per-(function, loop-start) cache. Once a loop is compiled or marked
    /// `Unjittable` it stays that way for the life of this engine.
    osr_cache: FxHashMap<OsrKey, OsrCacheEntry>,
    /// Hotness counter for OSR loop sites. A loop becomes a compilation
    /// candidate when its back-edge has been observed `threshold` times. Uses
    /// the same threshold as whole-function JIT; loop back-edges trip the
    /// counter faster than function calls, so OSR engages almost immediately
    /// on any loop that runs more than a handful of iterations.
    osr_hot: FxHashMap<OsrKey, u32>,
    /// Test/introspection counter — incremented exactly when a loop body is
    /// successfully compiled to native code. Distinct from the whole-fn
    /// counter so e2e tests can assert OSR specifically fired.
    osr_compiled: usize,
}

impl JitEngine {
    /// Construct the engine unless disabled. Returns `None` when
    /// `RUSTCFML_JIT=0`/`false`/`off` is set, or when the host ISA can't be
    /// initialised (in which case we silently stay on the interpreter).
    pub fn maybe_new() -> Option<Self> {
        match std::env::var("RUSTCFML_JIT") {
            Ok(v) if matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no") => {
                return None;
            }
            _ => {}
        }
        // Threshold is overridable for benchmarking/tests; default 50 trips
        // quickly without compiling genuinely cold functions.
        let threshold = std::env::var("RUSTCFML_JIT_THRESHOLD")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(50);
        Self::new_with_threshold(threshold)
    }

    /// Construct the engine with an explicit hotness threshold, ignoring the
    /// `RUSTCFML_JIT` / `RUSTCFML_JIT_THRESHOLD` environment entirely.
    /// Deterministic regardless of the process environment — tests use this
    /// (via `CfmlVirtualMachine::jit_set_threshold`) instead of `env::set_var`,
    /// which races when parallel test threads mutate the shared environment.
    pub fn new_with_threshold(threshold: u32) -> Option<Self> {
        let backend = translate::Backend::new().ok()?;
        Some(Self {
            cache: FxHashMap::default(),
            hot: HotnessTracker::new(threshold),
            backend,
            osr_cache: FxHashMap::default(),
            osr_hot: FxHashMap::default(),
            osr_compiled: 0,
        })
    }

    /// Count of OSR-compiled loop bodies currently in cache. Used by tests to
    /// confirm OSR (not just whole-fn JIT) actually fired.
    pub fn osr_compiled_count(&self) -> usize {
        self.osr_compiled
    }

    /// Number of functions currently holding a compiled native body. Used by
    /// tests and `--features jit` introspection to confirm the JIT fired.
    pub fn compiled_count(&self) -> usize {
        self.cache
            .values()
            .filter(|e| matches!(e, CacheEntry::Compiled { .. }))
            .count()
    }

    /// Convenience for tests: a no-op shadowing checker (nothing is shadowed).
    #[cfg(test)]
    fn try_call_unshadowed(
        &mut self,
        func: &BytecodeFunction,
        args: &[CfmlValue],
    ) -> Option<CfmlResult> {
        self.try_call(func, args, &mut |_| false, &|_| None)
    }

    /// T3.1 slot-locals interop: the analyser/translator work by NAME, so a
    /// function whose codegen assigned local slots is compiled from a
    /// normalized copy with every slot op rewritten back to its named twin —
    /// semantically identical (a slot IS the `var`-declared local it names).
    /// Runs once per (function, signature) at compile time, never on the
    /// cached fast path.
    fn normalize_slot_ops(func: &BytecodeFunction) -> BytecodeFunction {
        let mut f = func.clone();
        for op in &mut f.instructions {
            let new = match &*op {
                BytecodeOp::DeclareSlot(_, n) => Some(BytecodeOp::DeclareLocal(n.clone())),
                BytecodeOp::LoadSlot(_, n) => Some(BytecodeOp::LoadLocal(n.clone())),
                BytecodeOp::TryLoadSlot(_, n) => Some(BytecodeOp::TryLoadLocal(n.clone())),
                BytecodeOp::StoreSlot(_, n) => Some(BytecodeOp::StoreLocal(n.clone())),
                BytecodeOp::IncrementSlot(_, n) => Some(BytecodeOp::Increment(n.clone())),
                BytecodeOp::DecrementSlot(_, n) => Some(BytecodeOp::Decrement(n.clone())),
                BytecodeOp::AddSlotConst(_, n, k) => {
                    Some(BytecodeOp::AddLocalConst(n.clone(), *k))
                }
                BytecodeOp::MulSlotConst(_, n, k) => {
                    Some(BytecodeOp::MulLocalConst(n.clone(), *k))
                }
                BytecodeOp::JumpIfSlotCmpConstFalse(_, n, c, cmp, t) => {
                    Some(BytecodeOp::JumpIfLocalCmpConstFalse(n.clone(), *c, *cmp, *t))
                }
                BytecodeOp::ForSlotStep(_, n, l, cmp, s, t) => {
                    Some(BytecodeOp::ForLoopStep(n.clone(), *l, *cmp, *s, *t))
                }
                BytecodeOp::LoadSlotKey(_, n) => Some(BytecodeOp::LoadLocalKey(n.clone())),
                BytecodeOp::TryLoadSlotKey(_, n) => {
                    Some(BytecodeOp::TryLoadLocalKey(n.clone()))
                }
                BytecodeOp::LoadSlotProperty(_, n, p) => {
                    Some(BytecodeOp::LoadLocalProperty(n.clone(), p.clone()))
                }
                BytecodeOp::TryLoadSlotProperty(_, n, p) => {
                    Some(BytecodeOp::TryLoadLocalProperty(n.clone(), p.clone()))
                }
                BytecodeOp::StoreSlotProperty(_, n, p) => {
                    Some(BytecodeOp::StoreLocalProperty(n.clone(), p.clone()))
                }
                BytecodeOp::ArrayAppendSlot(_, n) => {
                    Some(BytecodeOp::ArrayAppendLocal(n.clone()))
                }
                _ => None,
            };
            if let Some(new) = new {
                *op = new;
            }
        }
        f.slot_names = Vec::new();
        f
    }

    /// The dispatch hook, called at the top of `execute_function_with_args`.
    ///
    /// Returns `Some(result)` only when a compiled native body ran to completion
    /// for these exact arguments; in every other case (`not hot yet`, `rejected`,
    /// `args not all Int`, `arity mismatch`, or a runtime `bail`) it returns
    /// `None` and the caller proceeds with the interpreter — so this can never
    /// change behaviour, only skip interpretation when it is provably equivalent.
    pub fn try_call(
        &mut self,
        func: &BytecodeFunction,
        args: &[CfmlValue],
        is_shadowed: &mut dyn FnMut(&str) -> bool,
        udf_lookup: &dyn Fn(&str) -> Option<UdfMeta>,
    ) -> Option<CfmlResult> {
        // Bail immediately if any arg isn't numeric, or if there are too many.
        let sig = signature_for(args)?;
        let key: CacheKey = (func.global_id, sig);

        // Fast path: already-known outcome for this exact signature.
        match self.cache.get(&key) {
            Some(CacheEntry::Unjittable) => return None,
            Some(CacheEntry::Compiled { ptr, ret_kind, builtins, udfs }) => {
                // Shadowing guard — a user-defined `abs` (etc.) defined after
                // the JIT cached this body must still take precedence.
                for n in builtins {
                    if is_shadowed(n) {
                        return None;
                    }
                }
                // UDF binding validation: every referenced callee must still
                // be Compiled with the matching signature. A drift here
                // means the callee has been redefined, evicted, or a name
                // now resolves to a different global_id.
                for u in udfs {
                    if !udf_binding_still_valid(&self.cache, *u) {
                        return None;
                    }
                }
                let ptr = *ptr;
                let rk = *ret_kind;
                let (result, bail) = run_compiled_with_engine(self, ptr, func, args, rk);
                if bail == 2 {
                    // Phase-2: speculation mismatch surfaced from a UDF
                    // callsite. Evict the caller so it re-analyses against
                    // the now-known callee `ret_float` on its next hot trip.
                    // `Unjittable` would pin it; instead remove the entry
                    // so the cold path re-runs.
                    self.cache.remove(&key);
                }
                return result;
            }
            None => {}
        }

        // Not cached yet — only act once this (func, signature) is hot.
        if !self.hot.record_and_is_hot(key) {
            return None;
        }

        // Build the param-kind vector that drives this specialization.
        let mut kinds: Vec<analysis::Kind> = Vec::with_capacity(args.len());
        for a in args {
            kinds.push(match a {
                CfmlValue::Int(_) => analysis::Kind::Int,
                CfmlValue::Double(_) => analysis::Kind::Float,
                _ => analysis::Kind::Boxed,
            });
        }

        // T3.1: analyse/compile from the slot-normalized copy when needed.
        let normalized = if func.slot_names.is_empty() {
            None
        } else {
            Some(Self::normalize_slot_ops(func))
        };
        let afunc: &BytecodeFunction = normalized.as_ref().unwrap_or(func);

        // ── Analyse with a UDF resolver that consults this engine's cache.
        //
        // Self-recursion is admitted optimistically: when the callee's
        // (global_id, sig) matches the caller's, the resolver answers
        // `Some(ret_float=false)` so the operand-stack typing proceeds
        // assuming an Int return. A Float-returning self-recursive function
        // would have an inconsistent operand-stack typing and is rejected
        // by the post-analysis self-call kind check below.
        let entry = {
            let cache_ref = &self.cache;
            let caller_gid = func.global_id;
            let caller_sig = sig;
            let resolver =
                |name: &str, arg_kinds: &[analysis::Kind]| -> Option<UdfRefBinding> {
                    let meta = udf_lookup(name)?;
                    if meta.nparams != arg_kinds.len() {
                        return None;
                    }
                    let callee_sig = sig_from_kinds(arg_kinds)?;
                    // Self-call: same fn + same sig as the specialization
                    // we're compiling. Optimistically assume Int return; the
                    // post-check below catches Float/Boxed-returning cases.
                    if meta.global_id == caller_gid && callee_sig == caller_sig {
                        return Some(UdfRefBinding {
                            global_id: caller_gid,
                            sig: caller_sig,
                            ret_kind: BindingRet::Int,
                        });
                    }
                    // Phase-2: foreign UDF binding.
                    // * If callee is already Compiled, bind with its real
                    //   `ret_kind` — the fast path with no runtime checks.
                    // * If callee is `Unjittable`, refuse the optimisation
                    //   (it would always bail at runtime; staying on the
                    //   interpreter for this caller is cheaper).
                    // * If callee is *not yet compiled*, speculate
                    //   `ret_kind = Int` and let the runtime dispatcher
                    //   surface `*bail = 2` if/when the callee compiles
                    //   with a different kind. The outer `try_call`
                    //   evicts the caller on bail=2 so the next hot trip
                    //   re-specializes against the now-known callee.
                    //
                    // v0.90.1: Boxed-arg and Boxed-ret callees are now
                    // admitted (sig_has_boxed rejection lifted, RetKind::
                    // Boxed binding allowed). The IR-level Boxed pipeline
                    // (v0.90.0) consumes tagged-pointer returns the same
                    // way it consumes a `Concat` result.
                    match cache_ref.get(&(meta.global_id, callee_sig)) {
                        Some(CacheEntry::Compiled { ret_kind, .. }) => Some(UdfRefBinding {
                            global_id: meta.global_id,
                            sig: callee_sig,
                            ret_kind: ret_kind.to_binding(),
                        }),
                        Some(CacheEntry::Unjittable) => None,
                        None => Some(UdfRefBinding {
                            global_id: meta.global_id,
                            sig: callee_sig,
                            ret_kind: BindingRet::Int,
                        }),
                    }
                };
            match analysis::analyze(afunc, &kinds, &resolver) {
                Some(plan) => {
                    let ret_kind = RetKind::from_analysis(plan.ret_kind);
                    // Self-recursion kind check: the resolver optimistically
                    // typed a self-call as `BindingRet::Int`. If the body's
                    // actual return kind is Float or Boxed, that speculation
                    // was wrong. Reject so we don't silently miscompile.
                    let self_call_mistyped = !matches!(ret_kind, RetKind::Int)
                        && plan
                            .referenced_udfs
                            .iter()
                            .any(|u| u.global_id == caller_gid && u.sig == caller_sig);
                    if self_call_mistyped {
                        CacheEntry::Unjittable
                    } else {
                        let builtins = plan.referenced_builtins.clone();
                        let udfs = plan.referenced_udfs.clone();
                        match self.backend.compile(afunc, &plan) {
                            Ok(ptr) => CacheEntry::Compiled { ptr, ret_kind, builtins, udfs },
                            Err(_) => CacheEntry::Unjittable,
                        }
                    }
                }
                None => CacheEntry::Unjittable,
            }
        };

        // Insert into the cache BEFORE running so a self-recursive body
        // finds itself in `dispatch_jit_udf` on its very first invocation.
        self.cache.insert(key, entry);

        // Re-fetch + run, applying the same shadowing/UDF-validity checks
        // as the fast path. (Cache lookup is cheap; this keeps the
        // post-insert run path single-source-of-truth.)
        match self.cache.get(&key) {
            Some(CacheEntry::Compiled { ptr, ret_kind, builtins, udfs }) => {
                if builtins.iter().any(|n| is_shadowed(n)) {
                    return None;
                }
                for u in udfs {
                    if !udf_binding_still_valid(&self.cache, *u) {
                        return None;
                    }
                }
                let ptr = *ptr;
                let rk = *ret_kind;
                let (result, bail) = run_compiled_with_engine(self, ptr, func, args, rk);
                if bail == 2 {
                    self.cache.remove(&key);
                }
                result
            }
            _ => None,
        }
    }

    /// Try to run a hot loop natively via OSR. Returns:
    /// * `Some(exit_ip)` — the compiled body ran to completion. `locals`
    ///   and `closure_env` have been updated with the post-loop slot values;
    ///   the caller must advance the interpreter `ip` to this value.
    /// * `None` — either not hot yet, not eligible, kind mismatch, runtime
    ///   bail (divide-by-zero), or a shadowed builtin. In every case the
    ///   interpreter must continue in-place: `locals`/`closure_env` may have
    ///   been *partially* updated (bail path writes back current slot
    ///   values), so the natural fall-through — `ip = ForLoopStep.target`
    ///   to re-execute the body once more — is the safe resume point.
    ///
    /// `is_shadowed` is consulted both on the cache-hit fast path *and* on
    /// the first call after compilation — same model as `try_call`. A
    /// user-defined `function abs(x){}` wins over the JIT'd native call.
    ///
    /// Region semantics (see `JIT_OSR_DESIGN.md`):
    /// * `region_start` = `ForLoopStep.target` (loop body start).
    /// * `region_end_excl` = `ForLoopStep_ip + 1` (the dispatch loop's `ip`
    ///   value AFTER it incremented past the ForLoopStep op).
    /// * The interpreter must have already executed its own step (counter +=
    ///   1) and computed `matched == true` before calling this — the OSR
    ///   body enters at `region_start` and assumes the counter is at the
    ///   value it should run the body with.
    pub fn try_run_loop(
        &mut self,
        func: &BytecodeFunction,
        region_start: usize,
        region_end_excl: usize,
        locals: &mut ValueMap,
        closure_env: Option<&Arc<RwLock<ValueMap>>>,
        is_shadowed: &mut dyn FnMut(&str) -> bool,
        udf_lookup: &dyn Fn(&str) -> Option<UdfMeta>,
    ) -> Option<usize> {
        let key: OsrKey = (func.global_id, region_start);

        // Fast path: known outcome.
        match self.osr_cache.get(&key) {
            Some(OsrCacheEntry::Unjittable) => return None,
            Some(OsrCacheEntry::Compiled(c)) => {
                for n in &c.referenced_builtins {
                    if is_shadowed(n) {
                        return None;
                    }
                }
                // v0.91.0 — every referenced UDF must still bind the same
                // specialization. Drift forces interpretation for this call.
                for u in &c.referenced_udfs {
                    if !udf_binding_still_valid(&self.cache, *u) {
                        return None;
                    }
                }
                let result = run_osr_compiled(c, locals, closure_env);
                // Bail=2 (speculation mismatch) is surfaced inside
                // `run_osr_compiled` via the same bail flag as bail=1 today —
                // both turn into `None`. Eviction-on-mismatch happens at the
                // *whole-function* layer (try_call). For OSR the simplest
                // safe response is: stay Compiled, but bail this trip. A
                // subsequent referenced_udfs check above will catch any
                // permanent drift.
                return result;
            }
            None => {}
        }

        // Hotness: a back-edge fires this many times before we look.
        let threshold = self.hot.threshold;
        let count = self.osr_hot.entry(key).or_insert(0);
        if *count > threshold {
            return None;
        }
        *count += 1;
        if *count <= threshold {
            return None;
        }

        // Crossed threshold — analyse + (maybe) compile, exactly once.
        let caller_kinds = build_caller_kinds(func, region_start, region_end_excl, locals);
        // UDF resolver: mirror the whole-fn try_call resolver. Consult the
        // engine's cache + caller-provided `udf_lookup` to bind any
        // `LoadGlobal(name)` of a non-builtin to its (global_id, sig)
        // specialization. Phase-2 speculation: not-yet-compiled callees
        // bind with `BindingRet::Int` and the dispatcher surfaces bail=2
        // at runtime on mismatch (handled defensively above).
        let cache_ref = &self.cache;
        let udf_resolver_closure = |name: &str, arg_kinds: &[analysis::Kind]| -> Option<UdfRefBinding> {
            let meta = udf_lookup(name)?;
            if meta.nparams != arg_kinds.len() {
                return None;
            }
            let sig = sig_from_kinds(arg_kinds)?;
            let ret = match cache_ref.get(&(meta.global_id, sig)) {
                Some(CacheEntry::Compiled { ret_kind, .. }) => ret_kind.to_binding(),
                Some(CacheEntry::Unjittable) => return None,
                None => BindingRet::Int, // speculate; dispatcher surfaces bail=2 on mismatch
            };
            Some(UdfRefBinding {
                global_id: meta.global_id,
                sig,
                ret_kind: ret,
            })
        };
        let entry = match osr::analyze_loop(
            func,
            region_start,
            region_end_excl,
            &caller_kinds,
            &udf_resolver_closure,
        ) {
            Some(plan) => match osr::compile_loop(&mut self.backend, func, &plan) {
                Ok(ptr) => {
                    self.osr_compiled += 1;
                    OsrCacheEntry::Compiled(OsrCompiled {
                        ptr,
                        slots: plan.slots,
                        exit_ip: plan.region_end_excl,
                        referenced_builtins: plan.referenced_builtins,
                        referenced_udfs: plan.referenced_udfs,
                    })
                }
                Err(_) => OsrCacheEntry::Unjittable,
            },
            None => OsrCacheEntry::Unjittable,
        };
        let run = match &entry {
            OsrCacheEntry::Compiled(c) => {
                if c.referenced_builtins.iter().any(|n| is_shadowed(n)) {
                    None
                } else if c
                    .referenced_udfs
                    .iter()
                    .any(|u| !udf_binding_still_valid(&self.cache, *u))
                {
                    None
                } else {
                    run_osr_compiled(c, locals, closure_env)
                }
            }
            OsrCacheEntry::Unjittable => None,
        };
        self.osr_cache.insert(key, entry);
        run
    }
}

/// Build the caller_kinds map for an OSR analysis attempt: scan the region for
/// local-name references and resolve each against the current value in
/// `locals`. Names whose live value isn't a numeric `Int`/`Double` are
/// silently dropped — the analyser then rejects any region that reads them.
fn build_caller_kinds(
    func: &BytecodeFunction,
    region_start: usize,
    region_end_excl: usize,
    locals: &ValueMap,
) -> HashMap<String, analysis::Kind> {
    let mut kinds: HashMap<String, analysis::Kind> = HashMap::new();
    for ip in region_start..region_end_excl {
        let name = match &func.instructions[ip] {
            BytecodeOp::LoadLocal(n)
            | BytecodeOp::StoreLocal(n)
            | BytecodeOp::Increment(n)
            | BytecodeOp::Decrement(n)
            | BytecodeOp::AddLocalConst(n, _)
            | BytecodeOp::MulLocalConst(n, _)
            | BytecodeOp::JumpIfLocalCmpConstFalse(n, _, _, _)
            | BytecodeOp::ForLoopStep(n, _, _, _, _)
            | BytecodeOp::DeclareLocal(n)
            // v0.99.8 — the fused `local.prop` op names a local receiver
            // that's never written elsewhere in the loop body but must be
            // seeded as Boxed so analyze_loop accepts the member read.
            | BytecodeOp::LoadLocalProperty(n, _)
            | BytecodeOp::TryLoadLocalProperty(n, _) => n,
            _ => continue,
        };
        let lower = name.to_ascii_lowercase();
        if kinds.contains_key(&lower) {
            continue;
        }
        // Case-insensitive lookup against the live locals map.
        let v = locals
            .get(name.as_str())
            .or_else(|| locals.iter().find(|(k, _)| k.eq_ignore_ascii_case(&lower)).map(|(_, v)| v));
        let k = match v {
            Some(CfmlValue::Int(_)) => analysis::Kind::Int,
            Some(CfmlValue::Double(_)) => analysis::Kind::Float,
            // v0.91.0: any other live value crosses as a Boxed slot. The
            // analyser still rejects ops that can't consume Boxed (arithmetic,
            // comparison, …) so a non-string non-numeric local that's actually
            // read by a non-Boxed-admissible op will still reject.
            Some(_) => analysis::Kind::Boxed,
            None => continue,
        };
        kinds.insert(lower, k);
    }
    kinds
}

/// Marshal each slot of `c` from `locals` into a packed `i64` buffer, invoke
/// the compiled body, then write every slot back to `locals` (and `closure_env`
/// when the env shares the name). Returns `Some(exit_ip)` on the success path
/// and `None` on the bail path — on bail the locals reflect the trapping
/// iteration's pre-failure state, exactly what the interpreter needs to
/// re-execute the body once more.
fn run_osr_compiled(
    c: &OsrCompiled,
    locals: &mut ValueMap,
    closure_env: Option<&Arc<RwLock<ValueMap>>>,
) -> Option<usize> {
    // v0.91.0 — install a per-call arena. Mid-body Boxed allocations
    // (`cfml_jit_concat_boxed`, `cfml_jit_box_int`, …) land here, and the
    // entry-side boxes we allocate below are also tracked (via
    // `arena::box_into_active`) so a single `drain_except(None)` reclaims
    // every leaked `Box<CfmlValue>` regardless of whether the slot held its
    // original value or a body-overwritten tag at exit.
    let mut arena = arena::Arena::new();

    // ── Marshal in. A kind mismatch (slot was Int at compile, now Double,
    // or missing entirely) forces a bail to the interpreter. Boxed slots
    // accept any non-Int/Float CfmlValue; we clone-then-box so the live
    // `locals` entry is untouched until writeback.
    let mut buf: Vec<i64> = Vec::with_capacity(c.slots.len());
    {
        // Scope the ArenaGuard so it drops before we touch `arena` again
        // (drain). See gotcha #16 in JIT_NEXT_SESSION.md.
        let _g = arena::ArenaGuard::install(&mut arena);
        for slot in &c.slots {
            // Case-insensitive lookup, matching the interpreter's behaviour.
            let v = locals.get(&slot.name).or_else(|| {
                locals
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&slot.name))
                    .map(|(_, v)| v)
            });
            match (slot.kind, v) {
                (analysis::Kind::Int, Some(CfmlValue::Int(i))) => buf.push(*i),
                (analysis::Kind::Float, Some(CfmlValue::Double(d))) => {
                    buf.push(d.to_bits() as i64)
                }
                (analysis::Kind::Boxed, Some(val)) => {
                    let tag = arena::box_into_active(val.clone());
                    buf.push(tag as i64);
                }
                _ => {
                    drop(_g);
                    arena.drain_except(None);
                    return None;
                }
            }
        }

        // ── Run.
        let mut bail: i64 = 0;
        // SAFETY: `c.ptr` is a Cranelift-emitted function with matching ABI
        // (`*mut i64`, `*mut i64`) -> `()`; the slot buffer + bail pointer are
        // both valid for the duration of the call.
        unsafe {
            (c.ptr)(buf.as_mut_ptr(), &mut bail as *mut i64);
        }

        // ── Read back live values BEFORE draining the arena. For Boxed slots
        // we borrow the tag and clone the inner CfmlValue so the underlying
        // boxes can be safely reclaimed below.
        let mut writes: Vec<CfmlValue> = Vec::with_capacity(c.slots.len());
        for (i, slot) in c.slots.iter().enumerate() {
            let val = match slot.kind {
                analysis::Kind::Int => CfmlValue::Int(buf[i]),
                analysis::Kind::Float => CfmlValue::Double(f64::from_bits(buf[i] as u64)),
                analysis::Kind::Boxed => {
                    // SAFETY: heap-tag tags came from our entry
                    // `box_into_active` or a shim allocation; SMI tags
                    // (v0.99.6) carry no heap memory. `materialize_tagged`
                    // handles both (synthesises Int for SMI, clones for
                    // heap). The heap box is reclaimed by the arena drain
                    // immediately below.
                    unsafe { boxed::materialize_tagged(buf[i] as usize) }
                }
                _ => unreachable!("OSR slots are always Int / Float / Boxed"),
            };
            writes.push(val);
        }

        // Drop the guard before draining (the active-pointer in the
        // thread-local must be cleared before we mutate `arena` directly).
        drop(_g);
        arena.drain_except(None);

        // Write back every slot — success and bail alike. The compiled body
        // populates buf in both paths so the interpreter sees consistent state.
        for (slot, val) in c.slots.iter().zip(writes.into_iter()) {
            if let Some(existing_key) = locals
                .keys()
                .find(|k| k.eq_ignore_ascii_case(&slot.name))
                .cloned()
            {
                locals.insert(existing_key, val.clone());
            } else {
                locals.insert(slot.name.clone(), val.clone());
            }
            if let Some(env) = closure_env {
                let mut m = env.write().unwrap();
                if let Some(existing_key) = m
                    .keys()
                    .find(|k| k.eq_ignore_ascii_case(&slot.name))
                    .cloned()
                {
                    m.insert(existing_key, val);
                }
            }
        }

        if bail != 0 {
            None
        } else {
            Some(c.exit_ip)
        }
    }
}

/// Marshal `args` across the ABI boundary and invoke a compiled body.
///
/// Returns `(result, bail_code)`. `bail_code = 0` ⇒ success and `result` is
/// `Some(Ok(...))`. Any non-zero bail makes `result = None` and the bail code
/// distinguishes transient deopt (`1`: div-by-zero / callee cache miss /
/// callee bailed) from a Phase-2 speculation mismatch (`2`: UDF→UDF
/// `ret_float` disagreement). The outer `try_call` evicts the caller's cache
/// entry on bail=2 so the next hot trip re-specializes against the now-known
/// callee `ret_float`. `Double` args cross as `f64::to_bits` so the compiled
/// prologue can `load F64` directly; on success the `i64` result is
/// re-wrapped as `CfmlValue::Int` or `Double`.
fn run_compiled(
    f: CompiledFn,
    func: &BytecodeFunction,
    args: &[CfmlValue],
    ret_kind: RetKind,
) -> (Option<CfmlResult>, i64) {
    // Tier-1 binds exactly the declared params positionally; defaults/var-args
    // are rejected at analysis time, so arity must match precisely.
    if args.len() != func.params.len() {
        return (None, 0);
    }
    // v0.90.0: install a per-call arena for mid-body shim allocations.
    // Boxed inputs are tracked in the same arena (uniform drain semantics).
    // Nested `run_compiled` (interpreter re-entry from inside a JIT'd
    // callee) pushes/pops its own arena via `ArenaGuard`.
    let mut arena = arena::Arena::new();
    let _guard = arena::ArenaGuard::install(&mut arena);

    let mut raw: Vec<i64> = Vec::with_capacity(args.len());
    for a in args {
        match a {
            CfmlValue::Int(i) => raw.push(*i),
            CfmlValue::Double(d) => raw.push(d.to_bits() as i64),
            other => {
                // Boxed arg: track in the arena so the post-call drain
                // reclaims it uniformly with shim-allocated boxes.
                let tagged = arena::box_into_active(other.clone());
                raw.push(tagged as i64);
            }
        }
    }
    let mut bail: i64 = 0;
    let result = unsafe { f(raw.as_ptr(), raw.len() as i64, &mut bail as *mut i64) };

    // Drop the active-arena binding before we touch `arena` directly.
    drop(_guard);

    // On a Boxed return the body's `result` is a tagged pointer that
    // currently lives in this call's arena. The arena drains every box
    // EXCEPT that one; the engine then reclaims it to wrap as CfmlValue.
    let keep_for_return: Option<usize> =
        if bail == 0 && matches!(ret_kind, RetKind::Boxed) {
            Some(result as usize)
        } else {
            None
        };
    arena.drain_except(keep_for_return);

    if bail != 0 {
        return (None, bail); // runtime deopt → interpret; surface code to caller
    }
    let value = match ret_kind {
        RetKind::Int => CfmlValue::Int(result),
        RetKind::Float => CfmlValue::Double(f64::from_bits(result as u64)),
        RetKind::Boxed => unsafe { boxed::reclaim_tagged(result as usize) },
    };
    (Some(Ok(value)), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_codegen::compiler::CfmlCompiler;
    use cfml_compiler::parser::Parser;

    /// Compile CFML `src` and return the named function's bytecode,
    /// slot-normalized the same way `try_call` does before analysis (the
    /// codegen slot pass rewrites `var` locals to `*Slot*` ops the analyser
    /// doesn't model — see [`JitEngine::normalize_slot_ops`]).
    fn compile_fn(src: &str, name: &str) -> BytecodeFunction {
        let ast = Parser::new(src.to_string()).parse().expect("parse");
        let program = CfmlCompiler::new().compile(ast);
        let f = program
            .functions
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("function {name} not found in program"))
            .as_ref()
            .clone();
        if f.slot_names.is_empty() {
            f
        } else {
            JitEngine::normalize_slot_ops(&f)
        }
    }

    /// Test-helper: default kind vector — every declared param pinned to Int.
    fn int_kinds(func: &BytecodeFunction) -> Vec<analysis::Kind> {
        func.params.iter().map(|_| analysis::Kind::Int).collect()
    }

    /// Analyse + compile `func`, then invoke the native body with `args`.
    /// Returns `(result, bailed)`.
    fn jit_run(func: &BytecodeFunction, args: &[i64]) -> (i64, bool) {
        let kinds = int_kinds(func);
        let plan = analysis::analyze_no_udfs(func, &kinds).expect("function should be JIT-eligible");
        let mut backend = translate::Backend::new().expect("backend init");
        let ptr = backend.compile(func, &plan).expect("compile");
        let mut bail: i64 = 0;
        let r = unsafe { ptr(args.as_ptr(), args.len() as i64, &mut bail as *mut i64) };
        (r, bail != 0)
    }

    /// Analyse + compile a `Float`-returning `func`, invoke it, and re-interpret
    /// the returned bits as `f64` (mirroring `run_compiled`'s ret-float path).
    /// Returns `(f64, bailed)`.
    fn jit_run_f64(func: &BytecodeFunction, args: &[i64]) -> (f64, bool) {
        let kinds = int_kinds(func);
        let plan = analysis::analyze_no_udfs(func, &kinds).expect("function should be JIT-eligible");
        assert_eq!(plan.ret_kind, analysis::Kind::Float, "expected a Float return");
        let mut backend = translate::Backend::new().expect("backend init");
        let ptr = backend.compile(func, &plan).expect("compile");
        let mut bail: i64 = 0;
        let r = unsafe { ptr(args.as_ptr(), args.len() as i64, &mut bail as *mut i64) };
        (f64::from_bits(r as u64), bail != 0)
    }

    /// Analyse + compile `func` with a caller-chosen kind tuple (each `Kind`
    /// matches one declared param). `args` carries raw 8-byte slots — Int as
    /// i64, Double as `f64::to_bits` (mirroring `run_compiled`'s marshaling).
    fn jit_run_kinds(
        func: &BytecodeFunction,
        kinds: &[analysis::Kind],
        args: &[i64],
    ) -> (f64, bool, bool) {
        let plan = analysis::analyze_no_udfs(func, kinds).expect("function should be JIT-eligible");
        let ret_float = plan.ret_kind == analysis::Kind::Float;
        let mut backend = translate::Backend::new().expect("backend init");
        let ptr = backend.compile(func, &plan).expect("compile");
        let mut bail: i64 = 0;
        let r = unsafe { ptr(args.as_ptr(), args.len() as i64, &mut bail as *mut i64) };
        if ret_float {
            (f64::from_bits(r as u64), ret_float, bail != 0)
        } else {
            (r as f64, ret_float, bail != 0)
        }
    }

    #[test]
    fn straight_line_arithmetic_matches() {
        let f = compile_fn("function poly(a, b) { return a * b + a - 1; }", "poly");
        // poly(6, 7) = 42 + 6 - 1 = 47
        assert_eq!(jit_run(&f, &[6, 7]), (47, false));
        // poly(-3, 10) = -30 + -3 - 1 = -34
        assert_eq!(jit_run(&f, &[-3, 10]), (-34, false));
    }

    #[test]
    fn counted_loop_sum_matches() {
        // Variable-bound loop: condition is `i <= n` (not const), exercising
        // LoadLocal/Lte/JumpIfFalse/Increment/Jump.
        let f = compile_fn(
            "function sumTo(n) { var t = 0; for (var i = 1; i <= n; i++) { t = t + i; } return t; }",
            "sumTo",
        );
        for n in [0i64, 1, 10, 100, 1000] {
            let expected = n * (n + 1) / 2;
            assert_eq!(jit_run(&f, &[n]), (expected, false), "sumTo({n})");
        }
    }

    #[test]
    fn const_bound_loop_uses_fused_ops() {
        // Const bound `i <= 100` triggers ForLoopStep + JumpIfLocalCmpConstFalse.
        let f = compile_fn(
            "function sumK() { var t = 0; for (var i = 1; i <= 100; i++) { t = t + i; } return t; }",
            "sumK",
        );
        assert_eq!(jit_run(&f, &[]), (5050, false));
    }

    #[test]
    fn factorial_loop_matches() {
        let f = compile_fn(
            "function fact(n) { var r = 1; for (var i = 2; i <= n; i++) { r = r * i; } return r; }",
            "fact",
        );
        // 10! = 3628800
        assert_eq!(jit_run(&f, &[10]), (3_628_800, false));
    }

    #[test]
    fn divide_by_zero_bails() {
        let f = compile_fn("function dz(a, b) { return a % b; }", "dz");
        assert_eq!(jit_run(&f, &[10, 3]), (1, false));
        let (_r, bailed) = jit_run(&f, &[10, 0]);
        assert!(bailed, "divide-by-zero must set the bail flag");
    }

    #[test]
    fn intdiv_matches_and_bails() {
        let f = compile_fn("function idiv(a, b) { return a \\ b; }", "idiv");
        assert_eq!(jit_run(&f, &[17, 5]), (3, false));
        assert!(jit_run(&f, &[1, 0]).1, "intdiv by zero must bail");
    }

    #[test]
    fn float_divide_returns_double() {
        // function avg(a, b) { return (a + b) / 2; }  → Double
        let f = compile_fn("function avg(a, b) { return (a + b) / 2; }", "avg");
        let (r, bailed) = jit_run_f64(&f, &[3, 4]);
        assert!(!bailed);
        assert_eq!(r, 3.5);
        assert_eq!(jit_run_f64(&f, &[10, 10]).0, 10.0);
    }

    #[test]
    fn float_divide_by_zero_bails() {
        // CFML `/` throws on zero divisor; the JIT must bail → re-interpret throws.
        let f = compile_fn("function d(a, b) { return a / b; }", "d");
        assert_eq!(jit_run_f64(&f, &[7, 2]).0, 3.5);
        assert!(jit_run_f64(&f, &[7, 0]).1, "divide by zero must bail");
    }

    #[test]
    fn float_literal_arithmetic_matches() {
        // function f(a) { return a * 1.5 + 0.25; }  → Double
        let f = compile_fn("function f(a) { return a * 1.5 + 0.25; }", "f");
        assert_eq!(jit_run_f64(&f, &[2]).0, 3.25);
        assert_eq!(jit_run_f64(&f, &[-4]).0, -5.75);
    }

    #[test]
    fn float_accumulator_loop_matches() {
        // Harmonic-ish sum with a Float accumulator and an Int loop counter.
        let f = compile_fn(
            "function h(n) { var s = 0.0; for (var i = 1; i <= n; i++) { s = s + 1 / i; } return s; }",
            "h",
        );
        let (r, bailed) = jit_run_f64(&f, &[4]);
        assert!(!bailed);
        let expected = 1.0 + 0.5 + 1.0 / 3.0 + 0.25;
        assert!((r - expected).abs() < 1e-12, "got {r}, want {expected}");
    }

    #[test]
    fn double_param_runs_specialised() {
        // function f(a, b) { return a * b + 1; }  with a=Double, b=Int.
        // The fixpoint upgrades the result to Float; ret_kind = Float.
        let f = compile_fn("function f(a, b) { return a * b + 1; }", "f");
        let kinds = [analysis::Kind::Float, analysis::Kind::Int];
        let raw: [i64; 2] = [2.5_f64.to_bits() as i64, 4];
        let (r, ret_float, bailed) = jit_run_kinds(&f, &kinds, &raw);
        assert!(!bailed);
        assert!(ret_float, "result of float*int+int is Float");
        assert_eq!(r, 11.0);
    }

    #[test]
    fn double_param_pure_int_op_still_returns_double() {
        // function f(a) { return a + 1; }  with a=Double.
        let f = compile_fn("function f(a) { return a + 1; }", "f");
        let kinds = [analysis::Kind::Float];
        let raw: [i64; 1] = [(-3.25_f64).to_bits() as i64];
        let (r, ret_float, _) = jit_run_kinds(&f, &kinds, &raw);
        assert!(ret_float);
        assert_eq!(r, -2.25);
    }

    #[test]
    fn float_mod_matches_fmod() {
        // function fm(a) { return (a / 1) % 0.3; }  → Double via cfml_fmod shim.
        // `(a/1)` forces Float on the lhs while keeping an int-typed param.
        let f = compile_fn("function fm(a) { return (a / 1) % 0.3; }", "fm");
        let (r, bailed) = jit_run_f64(&f, &[2]);
        assert!(!bailed);
        // 2.0 % 0.3 in IEEE-754: 2.0 = 6*0.3 + 0.2, but fmod truncates toward 0.
        let expected = 2.0_f64 % 0.3_f64;
        assert!((r - expected).abs() < 1e-12, "got {r}, want {expected}");
    }

    #[test]
    fn pow_matches_powf() {
        // function p(a, b) { return a ^ b; }  → Double via cfml_pow shim.
        let f = compile_fn("function p(a, b) { return a ^ b; }", "p");
        assert_eq!(jit_run_f64(&f, &[2, 10]).0, 1024.0);
        assert_eq!(jit_run_f64(&f, &[3, 0]).0, 1.0);
        let (r, _) = jit_run_f64(&f, &[5, 3]);
        assert_eq!(r, 125.0);
    }

    #[test]
    fn pow_with_float_operand() {
        // function p(a) { return (a / 1) ^ 0.5; }  → sqrt(a)
        let f = compile_fn("function p(a) { return (a / 1) ^ 0.5; }", "p");
        let (r, bailed) = jit_run_f64(&f, &[16]);
        assert!(!bailed);
        assert_eq!(r, 4.0);
    }

    // ── Option A: JIT → native builtin calls ────────────────────────────────

    #[test]
    fn abs_int_overload_returns_int() {
        // function f(a) { return abs(a); }  → Int result (the Int overload)
        let f = compile_fn("function f(a) { return abs(a); }", "f");
        assert_eq!(jit_run(&f, &[5]), (5, false));
        assert_eq!(jit_run(&f, &[-5]), (5, false));
        assert_eq!(jit_run(&f, &[0]), (0, false));
    }

    #[test]
    fn abs_float_overload_returns_double() {
        // function f(a) { return abs(a / 1); }  → forces Float, calls
        // cfml_abs_f64, returns Double.
        let f = compile_fn("function f(a) { return abs(a / 1); }", "f");
        let (r, bailed) = jit_run_f64(&f, &[-7]);
        assert!(!bailed);
        assert_eq!(r, 7.0);
    }

    #[test]
    fn min_returns_double_even_for_ints() {
        // CFML's `min(3, 5)` returns Double(3.0). The JIT must match.
        let f = compile_fn("function f(a, b) { return min(a, b); }", "f");
        let (r, bailed) = jit_run_f64(&f, &[3, 5]);
        assert!(!bailed);
        assert_eq!(r, 3.0);
        assert_eq!(jit_run_f64(&f, &[10, 4]).0, 4.0);
    }

    #[test]
    fn max_returns_double() {
        let f = compile_fn("function f(a, b) { return max(a, b); }", "f");
        assert_eq!(jit_run_f64(&f, &[3, 5]).0, 5.0);
        assert_eq!(jit_run_f64(&f, &[-3, -10]).0, -3.0);
    }

    #[test]
    fn nested_builtin_calls_compose() {
        // function f(a) { return max(abs(a), 5); }
        // -7 → max(7, 5) → 7.0; 2 → max(2, 5) → 5.0
        let f = compile_fn("function f(a) { return max(abs(a), 5); }", "f");
        assert_eq!(jit_run_f64(&f, &[-7]).0, 7.0);
        assert_eq!(jit_run_f64(&f, &[2]).0, 5.0);
    }

    #[test]
    fn builtin_inside_loop_matches() {
        // function f(n) { var t = 0; for (var i = 1; i <= n; i++) { t = t + abs(i - 3); } return t; }
        let f = compile_fn(
            "function f(n) { var t = 0; for (var i = 1; i <= n; i++) { t = t + abs(i - 3); } return t; }",
            "f",
        );
        // |1-3|+|2-3|+|3-3|+|4-3|+|5-3| = 2+1+0+1+2 = 6
        assert_eq!(jit_run(&f, &[5]), (6, false));
    }

    #[test]
    fn unknown_builtin_rejects() {
        // `len` is a real CFML builtin but not in the JIT shim allowlist —
        // analysis must reject. (`sin` was the original choice but it's been
        // promoted into the allowlist; pick a name that can't be JIT'd
        // because its semantics aren't pure-numeric.)
        let f = compile_fn("function f(a) { return len(a); }", "f");
        let kinds = int_kinds(&f);
        assert!(analysis::analyze_no_udfs(&f, &kinds).is_none());
    }

    #[test]
    fn referenced_builtins_recorded() {
        // The plan must list every allowlisted name the body calls.
        let f = compile_fn(
            "function f(a, b) { return max(abs(a), min(b, 10)); }",
            "f",
        );
        let plan = analysis::analyze_no_udfs(&f, &int_kinds(&f)).expect("eligible");
        let names: std::collections::BTreeSet<&str> =
            plan.referenced_builtins.iter().copied().collect();
        let expected: std::collections::BTreeSet<&str> = ["abs", "max", "min"].into_iter().collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn shadow_check_short_circuits_jit() {
        // First compile so the engine has a cached entry; then call try_call
        // with a shadow-checker that fires for "abs" — must return None.
        let f = compile_fn("function f(a) { return abs(a); }", "f");
        let mut engine = JitEngine {
            cache: rustc_hash::FxHashMap::default(),
            hot: HotnessTracker::new(0), // any non-cached call compiles on the first hit
            backend: translate::Backend::new().expect("backend"),
            osr_cache: rustc_hash::FxHashMap::default(),
            osr_hot: rustc_hash::FxHashMap::default(),
            osr_compiled: 0,
        };
        let args = vec![CfmlValue::Int(-7)];
        // First call: not hot yet (record_and_is_hot returns true at threshold+1
        // == 1, since threshold=0 → fires on the 1st call).
        let _ = engine.try_call_unshadowed(&f, &args);
        // Now compiled. With shadowing, must bail.
        let mut sh = |n: &str| n.eq_ignore_ascii_case("abs");
        assert!(engine.try_call(&f, &args, &mut sh, &|_| None).is_none());
        // Without shadowing, still works.
        let r = engine
            .try_call_unshadowed(&f, &args)
            .expect("compiled path must run")
            .expect("no runtime error");
        assert!(matches!(r, CfmlValue::Int(7)));
    }

    #[test]
    fn intdiv_truncates_float_operand() {
        // function f(a) { return (a / 2) \ 1; }  → float a/2 truncated to Int.
        let f = compile_fn("function f(a) { return (a / 2) \\ 1; }", "f");
        // 7/2 = 3.5 → \1 = 3 ; 9/2 = 4.5 → 4
        assert_eq!(jit_run(&f, &[7]), (3, false));
        assert_eq!(jit_run(&f, &[9]), (4, false));
    }
}
