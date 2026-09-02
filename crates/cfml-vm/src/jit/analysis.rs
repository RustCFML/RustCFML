//! Static analysis that decides whether a [`BytecodeFunction`] is a JIT
//! candidate and, if so, produces the [`Plan`] the translator consumes.
//!
//! A function is accepted only when *every reachable instruction* is in the
//! supported, side-effect-free, numeric subset and several safety properties
//! hold. Tier-1 covers **integer** kernels; Tier-1.5 (this revision) additionally
//! covers **floating-point** kernels — functions that produce `Double`s via
//! `Double` literals, the `/` operator, or float-typed locals. Arguments are
//! still bound as integers at the ABI boundary (the engine bails to the
//! interpreter unless every argument is a `CfmlValue::Int`), so float-ness only
//! ever arises *inside* an accepted function.
//!
//! The checks (all over the **reachable** CFG only, so dead trailing
//! `Null; Return` epilogues never disqualify a function):
//!
//! 1. **No defaulted params.** Args are bound positionally. `__main__` is now
//!    admissible on its merits: it is called with zero args, has no defaults,
//!    and the op-by-op pass below rejects every side-effecting top-level
//!    construct (writeOutput, includes, cfquery, struct/array, …) anyway.
//!    Whole-function admission of `__main__` (v0.91.0) lets a hot per-request
//!    `__main__` whose body is a pure numeric / Boxed-concat loop compile
//!    natively; pure-CLI runs never cross the hotness threshold so this is
//!    only a serve-mode unlock in practice.
//! 2. **Op-subset** — only the numeric/arithmetic/counted-loop ops below.
//! 3. **No reserved scope names** in local ops (`variables`, `arguments`, …).
//! 4. **Slot kinds** — every local slot is *uniformly* `Int` or `Float`. A
//!    monotonic fixpoint upgrades a slot to `Float` as soon as a `Float` value is
//!    stored into it; the consistency pass then rejects any path that stores an
//!    `Int`-kind value into a `Float` slot (that would be a *path-dependent type*
//!    the monomorphic JIT cannot reproduce). Param slots are pinned `Int` (they
//!    arrive as integers across the ABI), so a function that reassigns a param to
//!    a float result is rejected.
//! 5. **Operand-stack discipline** — within each basic block the operand stack
//!    starts and ends empty; a boolean (comparison/logical) result is never
//!    stored into a local nor returned.
//! 6. **Definite assignment** — a local is never read on a path where it may be
//!    unassigned (preserves the interpreter's "undefined var ⇒ error").
//! 7. **No fall-off** — control cannot reach the end of the body without a
//!    `Return`, and every `Return` agrees on the value kind (the function's
//!    [`Plan::ret_kind`], either all `Int` or all `Float`).
//!
//! Given the above, the native body always produces a value of the statically
//! known [`Plan::ret_kind`] that the engine re-wraps (`Int` ⇒ `CfmlValue::Int`,
//! `Float` ⇒ `CfmlValue::Double` from the returned bit pattern) with no
//! observable difference.

use cfml_codegen::{BytecodeOp, BytecodeFunction};
use std::collections::{BTreeSet, HashMap};

use super::builtins;

/// A reachable basic block: the half-open instruction range `[start, end)`.
pub struct Block {
    pub start: usize,
    pub end: usize,
}

/// Operand-stack / local value kind. Shared with the translator so the two
/// agree exactly on how each op is typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A guaranteed integer (literal, int local, or integer arithmetic result).
    Int,
    /// A guaranteed float/`Double` (Double literal, `/` result, or float local).
    Float,
    /// A boolean (comparison or logical result). May only be consumed by a
    /// branch or another logical/comparison op — never stored or returned.
    Bool,
    /// A reference to an allowlisted builtin function pushed by
    /// `LoadGlobal(name)`. The string is the lowercased CFML name; overload
    /// resolution against [`builtins::SHIMS`] happens at the matching
    /// `Call(n)` when actual arg kinds are known. `Builtin` is non-numeric,
    /// so existing arithmetic / comparison / storage paths reject it just
    /// like `Bool` — only `Call` consumes it.
    Builtin(&'static str),
    /// A reference to a JIT-eligible user-defined function pushed by
    /// `LoadGlobal(name)`. The `usize` indexes into the per-analysis
    /// `udf_names` table (see [`UdfResolver`]). Like `Builtin`, this kind is
    /// non-numeric — only `Call` consumes it — and the actual binding to a
    /// `(global_id, sig)` cache entry is resolved at the matching `Call(n)`
    /// when concrete arg kinds are known.
    UdfRef(usize),
    /// **Option-γ tag-pointer polymorphic value** (v0.89.0+). A Boxed slot
    /// holds a tagged `usize` — in v0.89.0 always a heap-allocated
    /// `Box<CfmlValue>` (tag `0b000`). The analyser only admits Boxed at
    /// the outer ABI boundary: a param can be Boxed, a slot can carry Boxed
    /// via store-flow from a Boxed param, and a return can yield Boxed.
    /// **No other operation** consumes Boxed in v0.89.0 — arithmetic,
    /// comparison, branching, and call-args all reject it, forcing the
    /// interpreter for any body that mixes polymorphic and operating ops.
    /// v0.90.0 lifts that restriction by emitting tag/untag IR for `+`,
    /// concat, member access, etc.
    Boxed,
}

impl Kind {
    /// `true` for the numeric kinds (everything but `Bool` / `Builtin`).
    fn is_num(self) -> bool {
        matches!(self, Kind::Int | Kind::Float)
    }
}

/// Result kind of a `+`/`-`/`*`/`%` on numeric or Boxed operands.
///
/// * Pure Int,Int → Int (raw `iadd`/`isub`/`imul`).
/// * Any Float operand (Int|Float, Float|Float) → Float (promote both, use
///   the float op).
/// * Any Boxed operand → Boxed. The translator emits a tag-check fast path
///   (SMI Int + SMI Int → inline `iadd` + retag) with the existing
///   `cfml_jit_add_boxed` shim as the slow path. Bool / Builtin / UdfRef
///   reject as before.
pub fn num_bin_kind(a: Kind, b: Kind) -> Option<Kind> {
    let admissible = |k: Kind| matches!(k, Kind::Int | Kind::Float | Kind::Boxed);
    if !admissible(a) || !admissible(b) {
        return None;
    }
    if a == Kind::Boxed || b == Kind::Boxed {
        return Some(Kind::Boxed);
    }
    Some(if a == Kind::Float || b == Kind::Float {
        Kind::Float
    } else {
        Kind::Int
    })
}

/// Tri-state return-kind tag for a [`UdfRefBinding`]. Parallel to
/// `crate::jit::RetKind`; lives in `analysis` so the binding type is
/// self-contained (no cycle from `analysis` back up to `mod.rs`).
///
/// v0.90.1 widens this from a `ret_float: bool` to admit `Boxed`. A
/// JIT'd caller may now invoke another JIT'd UDF whose specialization
/// returns a tagged-pointer `CfmlValue`; the result enters the caller's
/// operand stack as [`Kind::Boxed`] and flows through mid-body Boxed ops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindingRet {
    Int,
    Float,
    Boxed,
}

impl BindingRet {
    /// Operand-stack kind the caller pushes after a successful Call.
    pub fn to_kind(self) -> Kind {
        match self {
            BindingRet::Int => Kind::Int,
            BindingRet::Float => Kind::Float,
            BindingRet::Boxed => Kind::Boxed,
        }
    }
}

/// What the engine needs to know about a JIT-eligible UDF call resolved at
/// analysis time. Filled in by the [`UdfResolver`] callback at each `Call(n)`
/// against a [`Kind::UdfRef`] marker.
#[derive(Clone, Copy, Debug)]
pub struct UdfRefBinding {
    /// The callee function's stable `global_id`.
    pub global_id: u32,
    /// The callee's cache signature (`nargs` + 2-bit kind tuple — see
    /// `signature_for` in `mod.rs`). Identifies the specific specialization
    /// that must already be `Compiled` (or speculatively bound).
    pub sig: u64,
    /// Tri-state return kind of that specialization (v0.90.1).
    pub ret_kind: BindingRet,
}

impl UdfRefBinding {
    /// Operand-stack kind the caller pushes after a successful Call.
    pub fn ret_kind(&self) -> Kind {
        self.ret_kind.to_kind()
    }
}

/// Resolve `(callee_name, arg_kinds)` → a binding (if the callee is currently
/// a `Compiled` cache entry for that exact signature). Returning `None`
/// rejects the caller's analysis. Closes over the VM's `user_functions` map
/// and the engine's cache. A no-op resolver (`|_,_| None`) disables UDF→UDF
/// JIT calls and gives the same behaviour as before this feature.
pub type UdfResolver<'a> = dyn Fn(&str, &[Kind]) -> Option<UdfRefBinding> + 'a;

/// Everything the translator needs to emit Cranelift IR for an accepted fn.
pub struct Plan {
    /// Lowercased local name → slot index (slot = Cranelift `Variable` number).
    pub local_slot: HashMap<String, usize>,
    /// Param index → slot index (args arrive positionally and seed these slots).
    pub param_slots: Vec<usize>,
    /// Slot index → its uniform value kind (`Int` or `Float`; never `Bool`).
    pub slot_kind: Vec<Kind>,
    /// The kind every `Return` yields (`Int` or `Float`).
    pub ret_kind: Kind,
    /// Reachable blocks, sorted by `start`.
    pub blocks: Vec<Block>,
    /// Leader ip → index into `blocks`.
    pub block_at: HashMap<usize, usize>,
    /// Lowercased allowlist names referenced via `LoadGlobal` anywhere in the
    /// reachable code. The engine re-checks each one against the live VM at
    /// every call so a runtime user-defined `abs` (or a `globals["abs"]` entry)
    /// shadows the JIT'd path and forces interpretation.
    pub referenced_builtins: Vec<&'static str>,
    /// Per-`Call`-ip binding for UDF calls the analyser accepted. The
    /// translator looks up the binding at each `Call` op site to emit the
    /// libcall (`cfml_call_jit_udf`) with the correct `(global_id, sig)`.
    pub udf_call_at: HashMap<usize, UdfRefBinding>,
    /// Deduped list of UDF callees referenced by the body. The engine
    /// revalidates each at every invocation: if the callee's cache entry has
    /// been displaced, redefined, or shadowed since this body was compiled,
    /// the caller falls back to the interpreter for that call.
    pub referenced_udfs: Vec<UdfRefBinding>,
}

impl Plan {
    /// Slot for a (case-insensitive) local name, if referenced by the function.
    pub fn slot_of(&self, name: &str) -> Option<usize> {
        self.local_slot.get(&lower(name)).copied()
    }
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// The instruction indices making up each **null-delete guard** the codegen
/// emits around a `=` / `var` assignment whose RHS may be Null (v0.137.0,
/// PR #112). The exact, only shape that produces an `UnsetPath` is:
///
/// ```text
///   ip   : JumpIfNotNull(ip+4)   // PEEK the RHS; non-null → jump to the store
///   ip+1 : Pop                   // drop the Null
///   ip+2 : UnsetPath(path)       // delete the target key (undefined afterwards)
///   ip+3 : Jump(END)             // null path: skip the store
///   ip+4 : <store op…>           // the JumpIfNotNull target
///   …
///   END  :                       // (the Jump target)
/// ```
///
/// The native code compiles this as *"evaluate the RHS; if it is Null, **deopt**
/// to the interpreter (which performs the delete / undefined-read throw); else
/// store"* — so the guarded value flows straight into the store and the
/// `Pop`/`UnsetPath`/`Jump` trio never executes in native code. Recognising the
/// idiom here (and in the translator) lets a hot function containing such an
/// assignment JIT again instead of being silently rejected by the op-subset
/// check. `JumpIfNotNull`/`UnsetPath` outside this exact shape (e.g. Elvis /
/// null-coalescing, which place the default expression between the jump and its
/// target) are NOT matched and still disqualify the function.
pub(super) struct NullGuards {
    /// ips holding a guard's `JumpIfNotNull` (a peek → deopt-if-null).
    pub jump: std::collections::HashSet<usize>,
    /// ips holding a guard's `Pop`, `UnsetPath`, and skip-`Jump` (the
    /// deopt-on-null path — never executed in native code). Also suppressed
    /// from leader detection so the guarded value flows straight from the RHS
    /// into the store within a single basic block.
    pub skip: std::collections::HashSet<usize>,
}

pub(super) fn null_guard_sites(code: &[BytecodeOp]) -> NullGuards {
    let mut jump = std::collections::HashSet::new();
    let mut skip = std::collections::HashSet::new();
    for ip in 0..code.len() {
        if let BytecodeOp::JumpIfNotNull(t) = code[ip] {
            if t == ip + 4
                && matches!(code.get(ip + 1), Some(BytecodeOp::Pop))
                && matches!(code.get(ip + 2), Some(BytecodeOp::UnsetPath(_)))
                && matches!(code.get(ip + 3), Some(BytecodeOp::Jump(_)))
            {
                jump.insert(ip);
                skip.insert(ip + 1);
                skip.insert(ip + 2);
                skip.insert(ip + 3);
            }
        }
    }
    NullGuards { jump, skip }
}

fn is_reserved_scope(name: &str) -> bool {
    matches!(
        lower(name).as_str(),
        "local"
            | "variables"
            | "arguments"
            | "this"
            | "super"
            | "request"
            | "application"
            | "session"
            | "server"
            | "cgi"
            | "url"
            | "form"
            | "cookie"
            | "client"
            | "thread"
    )
}

/// No-op UDF resolver: never binds a UDF call. Use this when calling
/// `analyze` from contexts that don't have access to the engine cache
/// (in-crate tests). Yields the pre-Phase-1 behaviour: any non-builtin
/// `LoadGlobal` rejects the function.
#[allow(dead_code)] // test-only entry point
pub fn no_udf_resolver(_name: &str, _arg_kinds: &[Kind]) -> Option<UdfRefBinding> {
    None
}

/// Convenience: analyse without UDF→UDF call support. Identical to
/// `analyze(func, kinds, &no_udf_resolver)`. Used by in-crate tests and by
/// the e2e tests in `tests/jit_numeric.rs` so they don't have to construct
/// an engine to hand a resolver to.
#[allow(dead_code)] // test-only entry point
pub fn analyze_no_udfs(func: &BytecodeFunction, param_kinds: &[Kind]) -> Option<Plan> {
    analyze(func, param_kinds, &no_udf_resolver)
}

/// Decide whether `func` can be compiled for a specific *call signature* (one
/// [`Kind`] per declared parameter; `Bool` is rejected by the caller). `None`
/// ⇒ keep on the interpreter. Different signatures produce independent
/// specializations — each `(func, param_kinds)` pair is its own cache entry.
///
/// `udf_resolver` is consulted when the body calls another user-defined
/// function; it returns a binding when the callee already has a `Compiled`
/// cache entry for the matching arg-kind signature, and `None` otherwise (in
/// which case the caller is also rejected — Phase 1 only admits leaf-first
/// warmup, not mutual recursion or forward references to uncompiled UDFs).
/// Does `func` write to any name that is neither a parameter nor a local it
/// declares? See the call site in [`analyze`] for why that disqualifies it.
///
/// Slot ops are absent here by construction: `normalize_slot_ops` rewrites
/// every one back to its named twin before analysis.
fn writes_a_name_it_does_not_own(func: &BytecodeFunction) -> bool {
    let mut owned: std::collections::HashSet<&str> =
        func.params.iter().map(|p| p.as_str()).collect();
    // Declaration can follow first write in the op stream (a loop body reached
    // by a back edge), so gather every declaration before judging any write.
    for op in &func.instructions {
        if let BytecodeOp::DeclareLocal(name) = op {
            owned.insert(name.as_str());
        }
    }
    func.instructions.iter().any(|op| {
        let written = match op {
            BytecodeOp::StoreLocal(name)
            | BytecodeOp::StoreLocalProperty(name, _)
            | BytecodeOp::Increment(name)
            | BytecodeOp::Decrement(name)
            | BytecodeOp::AddLocalConst(name, _)
            | BytecodeOp::MulLocalConst(name, _)
            | BytecodeOp::ArrayAppendLocal(name) => Some(name),
            BytecodeOp::ForLoopStep(name, _, _, _, _) => Some(name),
            _ => None,
        };
        matches!(written, Some(n) if !owned.contains(n.as_str()))
    })
}

pub fn analyze(
    func: &BytecodeFunction,
    param_kinds: &[Kind],
    udf_resolver: &UdfResolver<'_>,
) -> Option<Plan> {
    // `__main__` is admissible on its merits — see the module-level doc.
    // Args are bound positionally; defaulted params need the runtime preamble.
    if func.has_default.iter().any(|d| *d) {
        return None;
    }
    if param_kinds.len() != func.params.len() {
        return None;
    }
    // A frame that writes a name it does not OWN participates in the
    // interpreter's parent-scope writeback, and a compiled body cannot: its
    // locals are native storage, so the diff-and-propagate that the frame
    // teardown performs has nothing to read. Two live cases, both silent:
    //
    //   * a closure assigning an enclosing `var` (`function(v){ captured = v; }`)
    //     — the defining frame never saw the write, so `captured` stayed "";
    //   * classic localMode, where an unscoped write inside a function is a
    //     write to the CALLER's scope by definition.
    //
    // A parameter and a `var`-declared local are frame-owned and stay
    // admissible, which is what keeps ordinary numeric bodies compilable.
    // Deliberately mode-independent: the effective localMode depends on the
    // application default, which is VM state the analyser cannot see, and
    // guessing it wrong here is a wrong ANSWER rather than a slow one.
    if writes_a_name_it_does_not_own(func) {
        return None;
    }
    // Admissible ABI param kinds: Int, Float, Boxed.
    if param_kinds
        .iter()
        .any(|k| !matches!(k, Kind::Int | Kind::Float | Kind::Boxed))
    {
        return None;
    }

    let code = &func.instructions;
    let n = code.len();
    if n == 0 {
        return None;
    }

    // Null-delete assignment guards (PR #112). Computed up front: the guard's
    // skip-`Jump` must NOT create a basic-block leader (else the store would
    // start a block with the guarded value already live on the operand stack,
    // breaking the empty-stack-at-boundary invariant). See [`null_guard_sites`].
    let null_guards = null_guard_sites(code);

    // ── 1. Leaders & basic blocks ───────────────────────────────────────────
    let mut leader_set: BTreeSet<usize> = BTreeSet::new();
    leader_set.insert(0);
    for (ip, op) in code.iter().enumerate() {
        // A guard's internal `Pop`/`UnsetPath`/`Jump` are elided in native
        // code — they must not split blocks.
        if null_guards.skip.contains(&ip) {
            continue;
        }
        match op {
            BytecodeOp::Jump(t)
            | BytecodeOp::JumpIfFalse(t)
            | BytecodeOp::JumpIfTrue(t)
            | BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, t)
            | BytecodeOp::ForLoopStep(_, _, _, _, t) => {
                if *t >= n {
                    return None; // jump past end of body — bail out of JIT
                }
                leader_set.insert(*t);
                if ip + 1 < n {
                    leader_set.insert(ip + 1);
                }
            }
            BytecodeOp::Return => {
                if ip + 1 < n {
                    leader_set.insert(ip + 1);
                }
            }
            _ => {}
        }
    }
    let leaders: Vec<usize> = leader_set.iter().copied().collect();
    let mut block_at: HashMap<usize, (usize, usize)> = HashMap::new();
    for (i, &start) in leaders.iter().enumerate() {
        let end = leaders.get(i + 1).copied().unwrap_or(n);
        block_at.insert(start, (start, end));
    }

    // ── 2. Reachability from entry (structural terminator reading) ──────────
    let succ = |_start: usize, end: usize| -> Option<Vec<usize>> {
        let term = &code[end - 1];
        Some(match term {
            BytecodeOp::Jump(t) => vec![*t],
            BytecodeOp::JumpIfFalse(t)
            | BytecodeOp::JumpIfTrue(t)
            | BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, t)
            | BytecodeOp::ForLoopStep(_, _, _, _, t) => {
                if end >= n {
                    return None;
                }
                vec![*t, end]
            }
            BytecodeOp::Return => vec![],
            _ => {
                if end >= n {
                    return None; // fall-off the end without a Return
                }
                vec![end]
            }
        })
    };

    let mut reachable: BTreeSet<usize> = BTreeSet::new();
    let mut work = vec![0usize];
    while let Some(start) = work.pop() {
        if !reachable.insert(start) {
            continue;
        }
        let (s, e) = match block_at.get(&start) {
            Some(&r) => r,
            None => return None,
        };
        let succs = succ(s, e)?;
        for t in succs {
            if !block_at.contains_key(&t) {
                return None;
            }
            if !reachable.contains(&t) {
                work.push(t);
            }
        }
    }

    let reach_sorted: Vec<usize> = reachable.iter().copied().collect();
    let mut plan_blocks: Vec<Block> = Vec::with_capacity(reach_sorted.len());
    let mut plan_block_at: HashMap<usize, usize> = HashMap::new();
    for (idx, &start) in reach_sorted.iter().enumerate() {
        let (s, e) = block_at[&start];
        plan_block_at.insert(s, idx);
        plan_blocks.push(Block { start: s, end: e });
    }

    // ── 3. Intern locals + op-subset + reserved-scope + local events ────────
    let mut local_slot: HashMap<String, usize> = HashMap::new();
    let mut locals: Vec<String> = Vec::new();
    let intern = |name: &str, locals: &mut Vec<String>, map: &mut HashMap<String, usize>| -> usize {
        let key = lower(name);
        if let Some(&s) = map.get(&key) {
            s
        } else {
            let s = locals.len();
            locals.push(key.clone());
            map.insert(key, s);
            s
        }
    };
    let mut param_slots: Vec<usize> = Vec::with_capacity(func.params.len());
    for p in &func.params {
        param_slots.push(intern(p, &mut locals, &mut local_slot));
    }

    #[derive(Clone, Copy)]
    enum Ev {
        Read(usize),
        Write(usize),
    }
    let mut block_events: Vec<Vec<Ev>> = vec![Vec::new(); plan_blocks.len()];
    let mut referenced_builtins: BTreeSet<&'static str> = BTreeSet::new();
    // Names of UDFs referenced via `LoadGlobal` inside the reachable code.
    // Interned at Pass 1 so the operand-stack simulator can push a Copy-able
    // `Kind::UdfRef(idx)` marker; the resolver is queried at every
    // simulate_block invocation (Infer fixpoint + Check) to look up the
    // binding using the concrete arg kinds at the matching `Call`.
    let mut udf_name_idx: HashMap<String, usize> = HashMap::new();
    let mut udf_names: Vec<String> = Vec::new();
    let intern_udf =
        |name: &str, idx: &mut HashMap<String, usize>, list: &mut Vec<String>| -> usize {
            let lower = lower(name);
            if let Some(&i) = idx.get(&lower) {
                i
            } else {
                let i = list.len();
                list.push(lower.clone());
                idx.insert(lower, i);
                i
            }
        };

    for (bidx, blk) in plan_blocks.iter().enumerate() {
        let events = &mut block_events[bidx];
        for ip in blk.start..blk.end {
            // A null-delete guard's `Pop`/`UnsetPath` carry no dataflow on the
            // native success path (the guarded value flows straight to the
            // store; a Null deopts) — admit them as no-ops.
            if null_guards.skip.contains(&ip) {
                continue;
            }
            match &code[ip] {
                // A guard's `JumpIfNotNull` peeks the RHS for the deopt-if-null
                // check; no local dataflow event.
                BytecodeOp::JumpIfNotNull(_) if null_guards.jump.contains(&ip) => {}
                // value-producing / value-consuming ops with no local event
                BytecodeOp::Integer(_)
                | BytecodeOp::Double(_)
                | BytecodeOp::True
                | BytecodeOp::False
                | BytecodeOp::Add
                | BytecodeOp::Sub
                | BytecodeOp::Mul
                | BytecodeOp::Div
                | BytecodeOp::Mod
                | BytecodeOp::Pow
                | BytecodeOp::IntDiv
                | BytecodeOp::Negate
                | BytecodeOp::Eq
                | BytecodeOp::Neq
                | BytecodeOp::Lt
                | BytecodeOp::Lte
                | BytecodeOp::Gt
                | BytecodeOp::Gte
                | BytecodeOp::And
                | BytecodeOp::Or
                | BytecodeOp::Xor
                | BytecodeOp::Not
                | BytecodeOp::Jump(_)
                | BytecodeOp::JumpIfFalse(_)
                | BytecodeOp::JumpIfTrue(_)
                | BytecodeOp::Pop
                | BytecodeOp::Dup
                | BytecodeOp::Return
                | BytecodeOp::LineInfo(_, _)
                // v0.90.0: `String(literal)` produces a Boxed value;
                // `Concat` consumes two arbitrary values and produces a
                // Boxed string. Pass 1 doesn't track stack kinds, so we
                // simply admit them here and let simulate_block enforce
                // the operand-kind rules.
                | BytecodeOp::String(_)
                | BytecodeOp::Concat => {}

                BytecodeOp::LoadLocal(name) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                }
                // v0.99.5 — fused LoadLocal + GetProperty for `local.prop`.
                // Treats the local as a Read (so dataflow knows it's used).
                // simulate_block enforces that the local's slot is Boxed
                // (the shim takes a tagged ptr; Int/Float slots can't
                // carry a struct).
                BytecodeOp::LoadLocalProperty(name, _prop)
                | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                }
                // `local.foo` read. The unfused form (LoadLocal("local") +
                // GetProperty) already bails here via the reserved-scope guard
                // on LoadLocal, so bailing keeps JIT admission identical — any
                // function reading the per-call `local` scope by name stays an
                // interpreter function.
                BytecodeOp::LoadLocalKey(_) | BytecodeOp::TryLoadLocalKey(_) => {
                    return None;
                }
                // v0.99.5 — `obj.prop` where obj is on the stack. Operand
                // tracking happens in simulate_block; Pass 1 just admits.
                BytecodeOp::GetProperty(_) | BytecodeOp::TryGetProperty(_) => {}
                // v0.100.0 — `obj.prop = value`. Operand tracking happens
                // in simulate_block; Pass 1 just admits.
                BytecodeOp::SetProperty(_) => {}
                // v0.100.0 — fused `local.prop = value`. Treats the local
                // as a Read+Write (value not stored back through the slot,
                // but the in-place mutation conceptually touches it).
                BytecodeOp::StoreLocalProperty(name, _prop) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                }
                BytecodeOp::StoreLocal(name) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Write(s));
                }
                BytecodeOp::Increment(name)
                | BytecodeOp::Decrement(name)
                | BytecodeOp::AddLocalConst(name, _)
                | BytecodeOp::MulLocalConst(name, _) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                    events.push(Ev::Write(s));
                }
                BytecodeOp::JumpIfLocalCmpConstFalse(name, _, _, _) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                }
                BytecodeOp::ForLoopStep(name, _, _, _, _) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    let s = intern(name, &mut locals, &mut local_slot);
                    events.push(Ev::Read(s));
                    events.push(Ev::Write(s));
                }
                BytecodeOp::DeclareLocal(name) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    intern(name, &mut locals, &mut local_slot);
                }

                // LoadGlobal(name): allowlisted builtin names take precedence
                // (cheaper dispatch); otherwise we intern the name as a UDF
                // candidate. The actual cache binding is resolved by the
                // udf_resolver at the matching Call(n) when arg kinds are
                // known. Names that match neither reject the function.
                // v0.614.0 — compile-time-bound builtin call. Same admission
                // rule as `LoadGlobal(builtin)` + `Call(n)`: the name must be
                // shim-backed, otherwise the function is rejected (there is no
                // UDF fallback — codegen only emits this for real builtins).
                BytecodeOp::CallBuiltin(name, _) => {
                    match builtins::canonical_name(name.as_str()) {
                        Some(n) => {
                            referenced_builtins.insert(n);
                        }
                        None => return None,
                    }
                }
                BytecodeOp::LoadGlobal(name) => {
                    if let Some(n) = builtins::canonical_name(name) {
                        referenced_builtins.insert(n);
                    } else {
                        intern_udf(name, &mut udf_name_idx, &mut udf_names);
                        // Optimistic admit at Pass 1 — actual rejection
                        // happens in simulate_block / Check if no binding
                        // exists for the call site.
                    }
                }
                // Call(n) covers both builtin and UDF dispatch; simulate_block
                // pops the marker and selects the right path based on Kind.
                BytecodeOp::Call(_) => {}

                // everything else (Null, Concat, heap, CallNamed, …)
                _ => return None,
            }
        }
    }

    // ── 4. Slot-kind fixpoint (monotonic Int → Float upgrades) ──────────────
    let nslots = locals.len();
    let mut slot_kind = vec![Kind::Int; nslots]; // locals start Int
    // Seed param slots from the call signature: a `Double` argument makes its
    // param slot `Float`. The fixpoint can still upgrade non-param locals.
    for (i, &p) in param_slots.iter().enumerate() {
        slot_kind[p] = param_kinds[i];
    }
    loop {
        let mut changed = false;
        for blk in &plan_blocks {
            // Infer mode: discard the per-ip UDF bindings (we only care
            // about kind upgrades here); the resolver may be called more
            // than once but that's fine — it's expected to be cheap and
            // pure.
            let mut _scratch: HashMap<usize, UdfRefBinding> = HashMap::new();
            simulate_block(
                code,
                blk,
                &local_slot,
                &mut slot_kind,
                &udf_names,
                udf_resolver,
                &mut _scratch,
                &null_guards,
                Mode::Infer { changed: &mut changed },
            )?;
        }
        if !changed {
            break;
        }
    }

    // A param slot's final kind must equal its seeded ABI kind. The fixpoint
    // only upgrades from Int → Float / Boxed, so this only fires when an
    // `Int` param is reassigned to a non-Int result (a path-dependent type
    // the monomorphic JIT can't model). `Float` / `Boxed` params stay their
    // kind by construction.
    for (i, &p) in param_slots.iter().enumerate() {
        if slot_kind[p] != param_kinds[i] {
            return None;
        }
    }

    // ── 5. Consistency + kind validation pass (records the return kind) ─────
    let mut ret_kind: Option<Kind> = None;
    let mut udf_call_at: HashMap<usize, UdfRefBinding> = HashMap::new();
    for blk in &plan_blocks {
        simulate_block(
            code,
            blk,
            &local_slot,
            &mut slot_kind,
            &udf_names,
            udf_resolver,
            &mut udf_call_at,
            &null_guards,
            Mode::Check { ret_kind: &mut ret_kind },
        )?;
    }
    let ret_kind = ret_kind?; // a function with no reachable Return is rejected

    // Dedupe referenced_udfs by (global_id, sig). The engine consults this
    // list once per outer call to revalidate every callee is still cached.
    let mut seen: BTreeSet<(u32, u64)> = BTreeSet::new();
    let mut referenced_udfs: Vec<UdfRefBinding> = Vec::new();
    for binding in udf_call_at.values() {
        if seen.insert((binding.global_id, binding.sig)) {
            referenced_udfs.push(*binding);
        }
    }

    // ── 6. Definite-assignment fixpoint over reachable blocks ───────────────
    let nblk = plan_blocks.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nblk];
    for (bidx, blk) in plan_blocks.iter().enumerate() {
        let succs = succ(blk.start, blk.end)?;
        for t in succs {
            let ti = plan_block_at[&t];
            preds[ti].push(bidx);
        }
    }

    let full: Vec<bool> = vec![true; nslots];
    let mut assigned_in: Vec<Vec<bool>> = vec![full.clone(); nblk];
    let mut entry_in = vec![false; nslots];
    for &s in &param_slots {
        entry_in[s] = true;
    }
    assigned_in[0] = entry_in;

    let transfer = |bidx: usize, ain: &[bool]| -> Vec<bool> {
        let mut a = ain.to_vec();
        for ev in &block_events[bidx] {
            if let Ev::Write(s) = ev {
                a[*s] = true;
            }
        }
        a
    };

    let mut changed = true;
    let mut assigned_out: Vec<Vec<bool>> = vec![full.clone(); nblk];
    while changed {
        changed = false;
        for bidx in 0..nblk {
            if bidx != 0 {
                let mut newin = if preds[bidx].is_empty() {
                    vec![false; nslots]
                } else {
                    vec![true; nslots]
                };
                for &p in &preds[bidx] {
                    for s in 0..nslots {
                        newin[s] = newin[s] && assigned_out[p][s];
                    }
                }
                if newin != assigned_in[bidx] {
                    assigned_in[bidx] = newin;
                    changed = true;
                }
            }
            let newout = transfer(bidx, &assigned_in[bidx]);
            if newout != assigned_out[bidx] {
                assigned_out[bidx] = newout;
                changed = true;
            }
        }
    }

    for bidx in 0..nblk {
        let mut cur = assigned_in[bidx].clone();
        for ev in &block_events[bidx] {
            match ev {
                Ev::Read(s) => {
                    if !cur[*s] {
                        return None;
                    }
                }
                Ev::Write(s) => cur[*s] = true,
            }
        }
    }

    Some(Plan {
        local_slot,
        param_slots,
        slot_kind,
        ret_kind,
        blocks: plan_blocks,
        block_at: plan_block_at,
        referenced_builtins: referenced_builtins.into_iter().collect(),
        udf_call_at,
        referenced_udfs,
    })
}

/// Mode for [`simulate_block`].
enum Mode<'a> {
    /// Fixpoint inference: upgrade a slot to `Float` when a `Float` value is
    /// stored into it, flagging `changed`. Never rejects for kind reasons.
    Infer { changed: &'a mut bool },
    /// Final validation: reject an `Int`-kind value stored into a `Float` slot,
    /// reject `Bool` stores/returns, and record the (uniform) return kind.
    Check { ret_kind: &'a mut Option<Kind> },
}

/// Abstractly interpret one basic block's operand stack with the current slot
/// kinds. Returns `Some(())` if the block is well-formed (operand stack starts
/// and ends empty, no underflow, supported kinds), `None` to reject the whole
/// function. The op-subset has already been validated, so any op not handled
/// here is treated as a rejection (defensive).
///
/// `udf_names` and `udf_resolver` resolve `Kind::UdfRef` markers at the
/// matching `Call`. `udf_bindings` accumulates per-ip bindings so the
/// translator can look them up later (populated by Check, scratched by
/// Infer).
fn simulate_block(
    code: &[BytecodeOp],
    blk: &Block,
    local_slot: &HashMap<String, usize>,
    slot_kind: &mut [Kind],
    udf_names: &[String],
    udf_resolver: &UdfResolver<'_>,
    udf_bindings: &mut HashMap<usize, UdfRefBinding>,
    null_guards: &NullGuards,
    mut mode: Mode,
) -> Option<()> {
    let slot_index = |name: &str| -> Option<usize> { local_slot.get(&lower(name)).copied() };
    let mut stack: Vec<Kind> = Vec::new();

    macro_rules! pop {
        () => {
            match stack.pop() {
                Some(k) => k,
                None => return None,
            }
        };
    }
    // Pop one value but reject a function reference (`Kind::Builtin(_)` or
    // `Kind::UdfRef(_)`) — those markers are only consumable by
    // `BytecodeOp::Call`; any other op attempting to use one disqualifies
    // the whole function. Also rejects `Kind::Boxed` — v0.89.0 only admits
    // Boxed at three sites (LoadLocal of a Boxed slot, StoreLocal into a
    // Boxed slot, Return) which call [`pop_assignable!`] instead.
    macro_rules! pop_value {
        () => {{
            let k = pop!();
            if matches!(k, Kind::Builtin(_) | Kind::UdfRef(_) | Kind::Boxed) {
                return None;
            }
            k
        }};
    }
    // Pop one value for assignment/return — like `pop_value!` but admits
    // `Kind::Boxed` (handled by the caller against the target slot's kind
    // or the return-kind tracker).
    macro_rules! pop_assignable {
        () => {{
            let k = pop!();
            if matches!(k, Kind::Builtin(_) | Kind::UdfRef(_)) {
                return None;
            }
            k
        }};
    }

    for ip in blk.start..blk.end {
        // Null-delete guard: the `Pop`/`UnsetPath` pair is the deopt-on-null
        // path, never taken in native code — skip it so the guarded value
        // stays on the operand stack and flows into the store at `ip+3`.
        if null_guards.skip.contains(&ip) {
            continue;
        }
        match &code[ip] {
            // A guard's `JumpIfNotNull` is a peek (deopt-if-null at translate
            // time); the value remains on the stack, kind unchanged.
            BytecodeOp::JumpIfNotNull(_) if null_guards.jump.contains(&ip) => {}
            BytecodeOp::Integer(_) => stack.push(Kind::Int),
            BytecodeOp::Double(_) => stack.push(Kind::Float),
            BytecodeOp::True | BytecodeOp::False => stack.push(Kind::Bool),

            // v0.90.0: a string literal becomes a freshly-boxed
            // `CfmlValue::String` allocated into the active per-call arena
            // by the `cfml_jit_string_literal` shim. The kind is Boxed.
            BytecodeOp::String(_) => stack.push(Kind::Boxed),

            // v0.90.0: `&` concat. Always yields a Boxed String (per
            // `BytecodeOp::Concat` semantics in lib.rs). Either operand
            // may be Int / Float / Boxed; Bool / Builtin / UdfRef are
            // rejected by `pop_value!` (a fn-reference can never reach
            // a concat). Int / Float operands are auto-boxed in codegen
            // before the `cfml_jit_concat_boxed` call.
            BytecodeOp::Concat => {
                let b = pop!();
                let a = pop!();
                if !matches!(a, Kind::Int | Kind::Float | Kind::Boxed)
                    || !matches!(b, Kind::Int | Kind::Float | Kind::Boxed)
                {
                    return None;
                }
                stack.push(Kind::Boxed);
            }

            BytecodeOp::LoadLocal(name) => {
                stack.push(slot_kind[slot_index(name)?]);
            }
            // v0.99.5 — LoadLocalProperty pushes Boxed unconditionally.
            // The local's slot kind must already be Boxed (the shim takes
            // a tagged ptr to a CfmlValue::Struct; a numeric slot would
            // need on-the-fly boxing the JIT can't do for these ops).
            // In Infer mode that means: don't upgrade — if the slot is
            // still Int, the final consistency pass will reject. Check
            // mode rejects directly.
            BytecodeOp::LoadLocalProperty(name, _prop)
            | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                let s = slot_index(name)?;
                if slot_kind[s] != Kind::Boxed {
                    if let Mode::Check { .. } = &mode {
                        return None;
                    }
                    // In Infer mode, just don't push — the analyser may
                    // upgrade the slot kind via later writes and the next
                    // fixpoint iteration revisits. (Return None for this
                    // simulate to abort the block — `analyze` rejects the
                    // whole function on any block error, which is the
                    // safe response.)
                    return None;
                }
                stack.push(Kind::Boxed);
            }
            // v0.99.5 — GetProperty pops Boxed, pushes Boxed. TryGetProperty
            // shares the exact stack shape (differs only on the miss path,
            // which the shim bails on).
            BytecodeOp::GetProperty(_) | BytecodeOp::TryGetProperty(_) => {
                let v = pop!();
                if v != Kind::Boxed {
                    return None;
                }
                stack.push(Kind::Boxed);
            }
            // v0.100.0 — SetProperty pops value (Int/Float/Boxed) + obj
            // (Boxed); pushes obj back. The codegen boxes non-Boxed values
            // via the same `ensure_boxed` helper Concat uses. Stack effect
            // is "pop 2, push 1" — net consume 1 (the value).
            BytecodeOp::SetProperty(_) => {
                let val = pop!();
                let obj = pop!();
                if obj != Kind::Boxed {
                    return None;
                }
                if !matches!(val, Kind::Int | Kind::Float | Kind::Boxed) {
                    return None;
                }
                stack.push(Kind::Boxed);
            }
            // v0.100.0 — StoreLocalProperty pops value (Int/Float/Boxed);
            // no push. Local slot must be Boxed (the shim takes a tagged
            // ptr to a CfmlValue::Struct).
            BytecodeOp::StoreLocalProperty(name, _prop) => {
                let s = slot_index(name)?;
                if slot_kind[s] != Kind::Boxed {
                    if let Mode::Check { .. } = &mode {
                        return None;
                    }
                    return None;
                }
                let val = pop!();
                if !matches!(val, Kind::Int | Kind::Float | Kind::Boxed) {
                    return None;
                }
            }
            BytecodeOp::StoreLocal(name) => {
                let s = slot_index(name)?;
                let v = pop_assignable!();
                if v == Kind::Bool {
                    return None; // a boolean must never enter a local
                }
                match &mut mode {
                    Mode::Infer { changed } => {
                        // Monotonic upgrades from the default `Int`: a non-Int
                        // store on an Int slot promotes the slot's kind. Any
                        // other mismatch is left for the Check pass to reject.
                        if slot_kind[s] == Kind::Int && v != Kind::Int {
                            slot_kind[s] = v;
                            **changed = true;
                        }
                    }
                    Mode::Check { .. } => {
                        if v != slot_kind[s] {
                            return None; // mixed-kind slot — not monomorphic
                        }
                    }
                }
            }

            // v0.99.6/v0.99.7 — Add/Sub/Mul admit Boxed operands (SMI fast
            // path + matching add/sub/mul_boxed slow shim).
            BytecodeOp::Add | BytecodeOp::Sub | BytecodeOp::Mul => {
                let b = pop!();
                let a = pop!();
                stack.push(num_bin_kind(a, b)?);
            }
            // `%`: Int,Int → Int (`srem`); any float operand → Float (via the
            // `cfml_fmod` libcall shim — see translate.rs). Boxed not yet
            // admitted (would need a `cfml_jit_mod_boxed` slow path).
            BytecodeOp::Mod => {
                let b = pop!();
                let a = pop!();
                if !a.is_num() || !b.is_num() {
                    return None;
                }
                stack.push(num_bin_kind(a, b)?);
            }
            // `^`: always Float (interpreter uses `f64::powf` on `to_number(.)`
            // of either operand). Translated as a call to the `cfml_pow` shim.
            BytecodeOp::Pow => {
                let b = pop!();
                let a = pop!();
                if !a.is_num() || !b.is_num() {
                    return None;
                }
                stack.push(Kind::Float);
            }
            // `/` always yields a Double; operands must be numeric.
            BytecodeOp::Div => {
                let b = pop!();
                let a = pop!();
                if !a.is_num() || !b.is_num() {
                    return None;
                }
                stack.push(Kind::Float);
            }
            // `\` always yields an Int (operands truncated to i64).
            BytecodeOp::IntDiv => {
                let b = pop!();
                let a = pop!();
                if !a.is_num() || !b.is_num() {
                    return None;
                }
                stack.push(Kind::Int);
            }
            BytecodeOp::Negate => {
                let a = pop!();
                if !a.is_num() {
                    return None;
                }
                stack.push(a);
            }

            BytecodeOp::Eq
            | BytecodeOp::Neq
            | BytecodeOp::Lt
            | BytecodeOp::Lte
            | BytecodeOp::Gt
            | BytecodeOp::Gte => {
                let _ = pop_value!();
                let _ = pop_value!();
                stack.push(Kind::Bool);
            }
            BytecodeOp::And | BytecodeOp::Or | BytecodeOp::Xor => {
                let _ = pop_value!();
                let _ = pop_value!();
                stack.push(Kind::Bool);
            }
            BytecodeOp::Not => {
                let _ = pop_value!();
                stack.push(Kind::Bool);
            }

            // Integer-constant read-modify-write — the slot must be an Int.
            BytecodeOp::Increment(name)
            | BytecodeOp::Decrement(name)
            | BytecodeOp::AddLocalConst(name, _)
            | BytecodeOp::MulLocalConst(name, _) => {
                if slot_kind[slot_index(name)?] != Kind::Int {
                    return None;
                }
            }
            BytecodeOp::JumpIfLocalCmpConstFalse(name, _, _, _) => {
                if slot_kind[slot_index(name)?] != Kind::Int {
                    return None;
                }
            }
            BytecodeOp::ForLoopStep(name, _, _, _, _) => {
                if slot_kind[slot_index(name)?] != Kind::Int {
                    return None;
                }
            }

            BytecodeOp::Jump(_) => {}
            BytecodeOp::JumpIfFalse(_) | BytecodeOp::JumpIfTrue(_) => {
                let _ = pop_value!();
            }

            BytecodeOp::Pop => {
                let _ = stack.pop(); // tolerate an empty stack
            }
            BytecodeOp::Dup => {
                let k = *stack.last()?;
                if matches!(k, Kind::Builtin(_) | Kind::UdfRef(_) | Kind::Boxed) {
                    return None; // fn-ref / Boxed must flow directly into their dedicated consumer
                }
                stack.push(k);
            }

            BytecodeOp::Return => {
                let v = pop_assignable!();
                if v == Kind::Bool {
                    return None;
                }
                if let Mode::Check { ret_kind } = &mut mode {
                    match ret_kind {
                        Some(existing) if *existing != v => return None, // mixed ret kind
                        _ => **ret_kind = Some(v),
                    }
                }
            }

            BytecodeOp::DeclareLocal(_) | BytecodeOp::LineInfo(_, _) => {}

            // Push a function-reference marker. Allowlisted builtins win
            // (resolved by lookup_overload at the matching Call); otherwise
            // we push a UdfRef marker whose binding is decided at the
            // matching Call when arg kinds are known.
            BytecodeOp::LoadGlobal(name) => {
                if let Some(canon) = builtins::canonical_name(name) {
                    stack.push(Kind::Builtin(canon));
                } else {
                    // The Pass-1 sweep already interned every reachable
                    // `LoadGlobal` name as a UDF candidate, so the index
                    // must exist; if not, the function is malformed.
                    let idx = *udf_names
                        .iter()
                        .position(|n| n.eq_ignore_ascii_case(name))
                        .as_ref()?;
                    stack.push(Kind::UdfRef(idx));
                }
            }
            // `CallBuiltin(name, n)` is `LoadGlobal(name)` + `Call(n)` fused, so
            // it arrives WITHOUT the fn-ref marker this arm expects. Synthesize
            // the marker underneath the args and share the identical body rather
            // than duplicating overload resolution.
            BytecodeOp::Call(_) | BytecodeOp::CallBuiltin(..) => {
                let n = match &code[ip] {
                    BytecodeOp::Call(n) => *n,
                    BytecodeOp::CallBuiltin(name, argc) => {
                        let argc = *argc as usize;
                        let canon = builtins::canonical_name(name.as_str())?;
                        if stack.len() < argc {
                            return None;
                        }
                        let at = stack.len() - argc;
                        stack.insert(at, Kind::Builtin(canon));
                        argc
                    }
                    _ => unreachable!(),
                };
                // Stack shape (top first): arg_n, …, arg_1, fn-ref marker.
                if stack.len() < n + 1 {
                    return None;
                }
                let split = stack.len() - n;
                let arg_kinds: Vec<Kind> = stack[split..].to_vec();
                // No nested function refs, no booleans at any arg position.
                // (Per-marker arms below tighten this further: builtins
                // accept only Int/Float; UDF callsites also admit Boxed.)
                if arg_kinds
                    .iter()
                    .any(|k| matches!(k, Kind::Builtin(_) | Kind::UdfRef(_) | Kind::Bool))
                {
                    return None;
                }
                stack.truncate(split);
                let marker = stack.pop()?;
                match marker {
                    Kind::Builtin(name) => {
                        // v0.92.0: builtin shims may now also accept Boxed
                        // operands (Option-γ tag-pointer) for string/array
                        // surface shims (len, uCase, …). Bool / Builtin /
                        // UdfRef are still hard-rejected; the per-overload
                        // `KindReq` decides which of Int / Float / Boxed an
                        // accepted shim actually takes at this position.
                        if arg_kinds
                            .iter()
                            .any(|k| !matches!(k, Kind::Int | Kind::Float | Kind::Boxed))
                        {
                            return None;
                        }
                        let shim_idx = builtins::lookup_overload(name, &arg_kinds)?;
                        stack.push(builtins::SHIMS[shim_idx].ret_kind);
                    }
                    Kind::UdfRef(idx) => {
                        // UDF callsites cross args as i64 (Int / Float bits
                        // / tagged Boxed ptr). The resolver decides whether
                        // the callee specialization for `arg_kinds` exists.
                        if arg_kinds
                            .iter()
                            .any(|k| !matches!(k, Kind::Int | Kind::Float | Kind::Boxed))
                        {
                            return None;
                        }
                        // Resolve the binding using the concrete arg kinds.
                        // The resolver consults the engine's cache; if the
                        // callee isn't currently Compiled with this exact
                        // signature, the analysis rejects.
                        let name = udf_names.get(idx)?;
                        let binding = udf_resolver(name, &arg_kinds)?;
                        // Record the binding so the translator can find it
                        // by IP. Check mode persists this; Infer's scratch
                        // map is discarded after the fixpoint.
                        udf_bindings.insert(ip, binding);
                        stack.push(binding.ret_kind());
                    }
                    _ => return None,
                }
            }

            _ => return None,
        }
    }

    if !stack.is_empty() {
        return None; // operand stack must be empty at the block boundary
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_codegen::CmpOp;

    /// Test-helper: analyse with every declared param pinned to `Int`.
    fn analyze_int(f: &BytecodeFunction) -> Option<Plan> {
        let kinds: Vec<Kind> = f.params.iter().map(|_| Kind::Int).collect();
        analyze_no_udfs(f, &kinds)
    }

    fn mkfn(params: &[&str], instrs: Vec<BytecodeOp>) -> BytecodeFunction {
        BytecodeFunction {
            name: "f".to_string(),
            params: params.iter().map(|s| s.to_string()).collect(),
            param_keys: Default::default(),
            args_needed: Default::default(),
            args_never_escapes: Default::default(),
            params_marker: Default::default(),
            cfc_body: Default::default(),
            required_params: params.iter().map(|_| true).collect(),
            has_default: params.iter().map(|_| false).collect(),
            instructions: instrs,
            source_file: None,
            global_id: 1,
            declared_local_mode: None,
            param_types: params.iter().map(|_| None).collect(),
            return_type: None,
            param_annotations: params.iter().map(|_| Vec::new()).collect(),
            is_component_method: false,
            access: cfml_common::dynamic::CfmlAccess::Public,
            metadata: Vec::new(),
            is_generated_accessor: false,
            output_suppressed: false,
            is_template_frame: false,
            chain_tier: 0,
            slot_names: Vec::new(),
        }
    }

    #[test]
    fn accepts_simple_arithmetic() {
        let f = mkfn(
            &["a", "b"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::LoadLocal("b".into()),
                BytecodeOp::Mul,
                BytecodeOp::Integer(1),
                BytecodeOp::Add,
                BytecodeOp::Return,
            ],
        );
        let plan = analyze_int(&f).expect("should be JIT-eligible");
        assert_eq!(plan.param_slots.len(), 2);
        assert!(plan.slot_of("A").is_some(), "case-insensitive local lookup");
        assert_eq!(plan.ret_kind, Kind::Int);
    }

    #[test]
    fn rejects_reserved_scope() {
        let f = mkfn(
            &[],
            vec![BytecodeOp::LoadLocal("variables".into()), BytecodeOp::Return],
        );
        assert!(analyze_int(&f).is_none());
    }

    #[test]
    fn admits_string_literal_return_as_boxed() {
        // v0.90.0: a String literal now admits with `Kind::Boxed`. The
        // function `function f() { return "x"; }` is a valid Tier-1 body.
        let f = mkfn(&[], vec![BytecodeOp::String("x".into()), BytecodeOp::Return]);
        let plan = analyze_int(&f).expect("string-literal return now admissible");
        assert_eq!(plan.ret_kind, Kind::Boxed);
    }

    #[test]
    fn rejects_read_before_assign() {
        let f = mkfn(&[], vec![BytecodeOp::LoadLocal("x".into()), BytecodeOp::Return]);
        assert!(analyze_int(&f).is_none());
    }

    #[test]
    fn rejects_returning_boolean() {
        let f = mkfn(
            &["a"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::Integer(0),
                BytecodeOp::Gt,
                BytecodeOp::Return,
            ],
        );
        assert!(analyze_int(&f).is_none());
    }

    #[test]
    fn rejects_void_via_trailing_null() {
        let f = mkfn(&[], vec![BytecodeOp::Null, BytecodeOp::Return]);
        assert!(analyze_int(&f).is_none());
    }

    #[test]
    fn ignores_unreachable_trailing_null() {
        let f = mkfn(
            &["a"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::Return,
                BytecodeOp::Null,
                BytecodeOp::Return,
            ],
        );
        assert!(analyze_int(&f).is_some(), "dead Null epilogue must not disqualify");
    }

    #[test]
    fn accepts_counted_loop() {
        let f = mkfn(
            &[],
            vec![
                // `var sum` / `var i` — real codegen emits a declaration for a
                // var-declared local, and without one these writes are UNSCOPED
                // and inadmissible (they would need parent-scope writeback).
                BytecodeOp::DeclareLocal("sum".into()),
                BytecodeOp::DeclareLocal("i".into()),
                BytecodeOp::Integer(0),
                BytecodeOp::StoreLocal("sum".into()),
                BytecodeOp::Integer(1),
                BytecodeOp::StoreLocal("i".into()),
                BytecodeOp::JumpIfLocalCmpConstFalse("i".into(), 10, CmpOp::Lte, 11),
                BytecodeOp::LoadLocal("sum".into()),
                BytecodeOp::LoadLocal("i".into()),
                BytecodeOp::Add,
                BytecodeOp::StoreLocal("sum".into()),
                BytecodeOp::ForLoopStep("i".into(), 10, CmpOp::Lte, 1, 7),
                BytecodeOp::LoadLocal("sum".into()),
                BytecodeOp::Return,
            ],
        );
        assert!(analyze_int(&f).is_some());
    }

    #[test]
    fn accepts_float_divide() {
        // function f(a, b) { return a / b; }  → Double result
        let f = mkfn(
            &["a", "b"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::LoadLocal("b".into()),
                BytecodeOp::Div,
                BytecodeOp::Return,
            ],
        );
        let plan = analyze_int(&f).expect("float divide should be eligible");
        assert_eq!(plan.ret_kind, Kind::Float);
    }

    #[test]
    fn accepts_float_local_accumulator() {
        // function f() { var s = 0.0; s = s + 1; return s; }  → s is Float
        let f = mkfn(
            &[],
            vec![
                // The `var` in the comment above, which real codegen emits and
                // this body needs: an undeclared write is UNSCOPED and would
                // require parent-scope writeback the compiled body cannot do.
                BytecodeOp::DeclareLocal("s".into()),
                BytecodeOp::Double(0.0),
                BytecodeOp::StoreLocal("s".into()),
                BytecodeOp::LoadLocal("s".into()),
                BytecodeOp::Integer(1),
                BytecodeOp::Add,
                BytecodeOp::StoreLocal("s".into()),
                BytecodeOp::LoadLocal("s".into()),
                BytecodeOp::Return,
            ],
        );
        let plan = analyze_int(&f).expect("float accumulator should be eligible");
        assert_eq!(plan.ret_kind, Kind::Float);
        assert_eq!(plan.slot_kind[plan.slot_of("s").unwrap()], Kind::Float);
    }

    #[test]
    fn rejects_mixed_kind_slot() {
        // var x = 0; x = 1.5; return x;  → Int store into an upgraded-Float slot
        let f = mkfn(
            &[],
            vec![
                BytecodeOp::Integer(0),
                BytecodeOp::StoreLocal("x".into()),
                BytecodeOp::Double(1.5),
                BytecodeOp::StoreLocal("x".into()),
                BytecodeOp::LoadLocal("x".into()),
                BytecodeOp::Return,
            ],
        );
        assert!(analyze_int(&f).is_none(), "path-dependent slot type must reject");
    }

    #[test]
    fn accepts_float_seeded_param() {
        // function f(a) { return a / 2; }  with `a` seeded as Float (caller
        // passed a Double). Slot is Float from the start; no path-dependent
        // type arises.
        let f = mkfn(
            &["a"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::Integer(2),
                BytecodeOp::Div,
                BytecodeOp::Return,
            ],
        );
        let plan = analyze_no_udfs(&f, &[Kind::Float]).expect("float-seeded param accepted");
        assert_eq!(plan.ret_kind, Kind::Float);
        assert_eq!(plan.slot_kind[plan.param_slots[0]], Kind::Float);
    }

    #[test]
    fn rejects_float_param_reassignment() {
        // function f(a) { a = a / 2; return a; }  → param slot would become Float
        let f = mkfn(
            &["a"],
            vec![
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::Integer(2),
                BytecodeOp::Div,
                BytecodeOp::StoreLocal("a".into()),
                BytecodeOp::LoadLocal("a".into()),
                BytecodeOp::Return,
            ],
        );
        assert!(analyze_int(&f).is_none(), "float-reassigned param must reject");
    }
}
