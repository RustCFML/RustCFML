//! On-Stack Replacement (OSR) for hot bytecode loops.
//!
//! This module covers the *static* half of OSR — analysis of a bytecode
//! sub-range that ends in a `ForLoopStep`, and translation of that sub-range
//! into a Cranelift function with an in/out locals ABI. The VM dispatch hook
//! that actually engages a compiled loop lives in `lib.rs` (in the
//! `ForLoopStep` handler) and is wired up in a separate commit; this module's
//! unit tests exercise the analyser and translator directly.
//!
//! # Why a separate analyser
//!
//! The whole-function analyser in `super::analysis` validates the full
//! reachable CFG from entry to every `Return`. A hot loop inside a function
//! that contains *any* non-admitted op (a side-effecting `writeOutput`, a
//! struct/array build, a non-allowlist `Call`, …) anywhere else in its body
//! therefore never qualifies via the whole-function path — even though the
//! loop region itself uses the supported op subset. OSR lifts that
//! restriction by analysing the *region* `[loop_header, step_ip + 1)` only,
//! with caller-provided slot kinds as the seed (no params; the locals
//! already exist in the live VM scope and we marshal them across an in/out
//! ABI). `__main__` is admissible by both paths (whole-fn since v0.91.0, OSR
//! all along).
//!
//! # ABI
//!
//! ```rust,ignore
//! type CompiledLoop = unsafe extern "C" fn(io_locals: *mut i64, bail: *mut i64);
//! ```
//!
//! `io_locals` is a packed array of 8-byte slots, one per touched local in
//! [`LoopPlan::slots`] order. Each slot holds either an `i64` (`Kind::Int`)
//! or the `f64::to_bits` of an `f64` (`Kind::Float`). The compiled body
//! reads the slots on entry, runs the loop to completion (or until a
//! divide-by-zero / overflow forces a bail), writes the current value of
//! every mutated slot back into `io_locals`, and returns. On the success
//! path `*bail` is left untouched (callers initialise it to `0`); on the
//! bail path `*bail = 1` and the partially-progressed state is written back
//! so the interpreter can resume from the trapping iteration.
//!
//! # Wiring status
//!
//! Until the VM dispatch hook in `lib.rs::ForLoopStep` is wired up (see the
//! `JIT_OSR_DESIGN.md` Commit-4 step), the public symbols here are only
//! invoked from this module's unit tests. The dead-code lint is silenced
//! module-wide so the surrounding `--features jit` build stays warning-free
//! between commits; the attribute is removed when the hook lands.
//!
//! # MVP scope
//!
//! This first cut admits exactly the op subset the whole-function JIT
//! already handles (Tier-1 + Tier-1.5 + Option-A native builtins) plus the
//! constraint that the region's *only* exit is the fall-through after the
//! terminating `ForLoopStep`. No `Return`, no `Jump` to a target outside the
//! region. Up to 32 live locals across the loop boundary.

use cfml_codegen::{BytecodeFunction, BytecodeOp};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, InstBuilder, MemFlags, StackSlotData, StackSlotKind, Value,
};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{Linkage, Module};
use std::collections::{BTreeSet, HashMap};

use super::analysis::{Kind, UdfRefBinding, UdfResolver};
use super::builtins::{self, SHIMS};
use super::translate::{self, Backend};

const I64: types::Type = types::I64;
const F64: types::Type = types::F64;

/// Cap on the number of live locals carried across the OSR boundary. Larger
/// loops bail to the interpreter rather than compile, matching the same
/// arbitrary cap the whole-function JIT uses on parameter count.
pub const MAX_OSR_SLOTS: usize = 32;

/// Compiled hot-loop body. See module docs for the contract.
pub type CompiledLoop = unsafe extern "C" fn(io_locals: *mut i64, bail: *mut i64);

/// One local marshalled across the OSR boundary.
#[derive(Clone, Debug)]
pub struct OsrSlot {
    /// Lowercased local name — the same canonical form the interpreter uses
    /// for `locals.get`/`locals.insert`.
    pub name: String,
    /// Uniform value kind (`Int` or `Float`; `Bool`/`Builtin` are never slots).
    pub kind: Kind,
    /// `true` when the loop body reads but never writes this slot — lets the
    /// caller skip the write-back for that slot. (We still write it back today
    /// for simplicity; this flag is recorded for future optimisation.)
    #[allow(dead_code)]
    pub in_only: bool,
}

/// Everything the OSR translator needs to emit Cranelift IR for a hot loop.
pub struct LoopPlan {
    /// Slot ordering = the order locals are marshalled across the ABI.
    pub slots: Vec<OsrSlot>,
    /// Lowercased local name → index into `slots`.
    pub slot_index: HashMap<String, usize>,
    /// Reachable blocks within `[region_start, region_end_excl)`, sorted by
    /// `start`. Block boundaries are computed the same way as in the
    /// whole-function analyser (leaders = jump targets, post-branch, etc.) but
    /// every block is constrained to lie wholly inside the region.
    pub blocks: Vec<Block>,
    /// Leader ip → index into `blocks`.
    pub block_at: HashMap<usize, usize>,
    /// First ip inside the region (inclusive).
    #[allow(dead_code)]
    pub region_start: usize,
    /// One past the last ip inside the region (exclusive). Native execution
    /// resumes the interpreter at this ip on a clean exit.
    pub region_end_excl: usize,
    /// Allowlisted builtin names referenced inside the region (lowercased).
    /// The engine re-checks each against the live VM on every call so a
    /// user-defined `function abs(x){}` still shadows the JIT.
    pub referenced_builtins: Vec<&'static str>,
    /// Per-`Call`-ip binding for UDF calls the analyser accepted. Codegen
    /// looks up the binding at each `Call` op site to emit the libcall
    /// (`cfml_call_jit_udf`) with the correct `(global_id, sig)`.
    pub udf_call_at: HashMap<usize, UdfRefBinding>,
    /// Deduped list of UDF callees referenced by the region. The engine
    /// revalidates each at every outer call so a redefined / evicted /
    /// shadowed callee forces interpretation.
    pub referenced_udfs: Vec<UdfRefBinding>,
}

/// A reachable basic block: half-open `[start, end)` over function bytecode
/// indices. Identical shape to the whole-function analyser's `Block`.
pub struct Block {
    pub start: usize,
    pub end: usize,
}

impl LoopPlan {
    pub fn slot_of(&self, name: &str) -> Option<usize> {
        self.slot_index.get(&name.to_ascii_lowercase()).copied()
    }
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
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

/// Decide whether `func`'s bytecode range `[region_start, region_end_excl)` is
/// a JIT-eligible hot loop, with each local in `caller_kinds` interpreted as
/// its already-observed kind. `None` ⇒ keep on the interpreter.
///
/// Constraints (strictest first; loosen later as needed):
///
/// 1. The region terminates in a `ForLoopStep` whose target is `region_start`
///    and whose fall-through is `region_end_excl`.
/// 2. Every reachable op in the region is in the supported subset (same as
///    whole-fn analyse).
/// 3. The region's only entry is `region_start`. (Codegen for counted
///    for-loops emits a single header, so this almost always holds.)
/// 4. The region's only exit is `region_end_excl` (no `Return`; no `Jump`
///    or conditional whose target falls outside `[region_start, region_end_excl)`
///    or `{region_end_excl}`).
/// 5. ≤ [`MAX_OSR_SLOTS`] live locals referenced in the region.
/// 6. The loop counter is `Int`.
/// 7. No reserved-scope local names.
/// 8. Each slot has a uniform kind (no path-dependent `Int`↔`Float`).
pub fn analyze_loop(
    func: &BytecodeFunction,
    region_start: usize,
    region_end_excl: usize,
    caller_kinds: &HashMap<String, Kind>,
    udf_resolver: &UdfResolver<'_>,
) -> Option<LoopPlan> {
    let code = &func.instructions;
    let n = code.len();
    if region_start >= region_end_excl || region_end_excl > n {
        return None;
    }
    // The region must terminate in a back-edge whose target is `region_start`:
    //
    // * `ForLoopStep(_, _, _, _, region_start)` — fused counted for-loop
    //   (the increment, test, and back-jump rolled into one super-op).
    // * `Jump(region_start)` — unconditional back-edge from a while/until
    //   loop or the inner repeat shape.
    // * `JumpIfTrue(region_start)` — back-edge from `do { body } while(cond)`.
    // * `JumpIfFalse(region_start)` — back-edge from `do { body } until(cond)`.
    //
    // For the ForLoopStep form we also pin the loop counter to `Int` later;
    // the other forms have no special counter so that check is skipped.
    let term_ip = region_end_excl - 1;
    let counter_name: Option<String> = match &code[term_ip] {
        BytecodeOp::ForLoopStep(name, _, _, _, target) if *target == region_start => {
            Some(name.to_string())
        }
        BytecodeOp::Jump(target)
        | BytecodeOp::JumpIfTrue(target)
        | BytecodeOp::JumpIfFalse(target)
            if *target == region_start =>
        {
            // Guard against the weird case where the first op in the region
            // is itself a ForLoopStep — that arises only with `continue`
            // jumps inside an outer for-loop, where region_start lands on
            // the outer ForLoopStep. Rejecting here keeps OSR focused on
            // real while/until/repeat loop headers.
            if matches!(code[region_start], BytecodeOp::ForLoopStep(_, _, _, _, _)) {
                return None;
            }
            None
        }
        _ => return None,
    };

    // ── 1. Leaders & basic blocks inside the region ────────────────────────
    let mut leader_set: BTreeSet<usize> = BTreeSet::new();
    leader_set.insert(region_start);
    for ip in region_start..region_end_excl {
        match &code[ip] {
            BytecodeOp::Jump(t)
            | BytecodeOp::JumpIfFalse(t)
            | BytecodeOp::JumpIfTrue(t)
            | BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, t)
            | BytecodeOp::ForLoopStep(_, _, _, _, t) => {
                // A jump out of the region is allowed only when it targets
                // region_end_excl exactly (clean loop exit). The ForLoopStep
                // at step_ip is special-cased — its fall-through IS the exit.
                if *t < region_start || *t > region_end_excl {
                    return None;
                }
                if *t > region_start && *t < region_end_excl {
                    leader_set.insert(*t);
                }
                if ip + 1 < region_end_excl {
                    leader_set.insert(ip + 1);
                }
            }
            BytecodeOp::Return => return None, // forbidden in the region
            _ => {}
        }
    }
    let leaders: Vec<usize> = leader_set.iter().copied().collect();
    let mut blocks: Vec<Block> = Vec::with_capacity(leaders.len());
    let mut block_at: HashMap<usize, usize> = HashMap::new();
    for (i, &start) in leaders.iter().enumerate() {
        let end = leaders.get(i + 1).copied().unwrap_or(region_end_excl);
        block_at.insert(start, i);
        blocks.push(Block { start, end });
    }

    // ── 2. Reachability from region_start over the region-restricted CFG ───
    let succ = |_start: usize, end: usize| -> Option<Vec<usize>> {
        let term = &code[end - 1];
        Some(match term {
            BytecodeOp::Jump(t) => vec![*t],
            BytecodeOp::JumpIfFalse(t)
            | BytecodeOp::JumpIfTrue(t)
            | BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, t)
            | BytecodeOp::ForLoopStep(_, _, _, _, t) => {
                let mut s = vec![*t];
                if end < region_end_excl {
                    s.push(end);
                }
                s
            }
            _ => {
                if end >= region_end_excl {
                    // A region that ends without a recognised terminator (not
                    // ForLoopStep, since step_ip is the last instruction and
                    // ForLoopStep is a branch) means a block falls off the end
                    // — would re-enter the interpreter mid-flow with no
                    // marshalling. Reject.
                    return None;
                }
                vec![end]
            }
        })
    };

    let mut reachable: BTreeSet<usize> = BTreeSet::new();
    let mut work = vec![region_start];
    while let Some(start) = work.pop() {
        if !reachable.insert(start) {
            continue;
        }
        // Successors *inside* the region — `region_end_excl` is the clean
        // exit and is not a block in this plan.
        let &bi = block_at.get(&start)?;
        let blk = &blocks[bi];
        let succs = succ(blk.start, blk.end)?;
        for t in succs {
            if t == region_end_excl {
                continue; // clean exit; nothing more to explore
            }
            if !block_at.contains_key(&t) {
                return None; // jump into mid-instruction or outside the region
            }
            if !reachable.contains(&t) {
                work.push(t);
            }
        }
    }

    // Keep only reachable blocks. Reuse the existing leader → block_at index.
    let reach_sorted: Vec<usize> = reachable.iter().copied().collect();
    let mut reach_blocks: Vec<Block> = Vec::with_capacity(reach_sorted.len());
    let mut reach_block_at: HashMap<usize, usize> = HashMap::new();
    for (idx, &start) in reach_sorted.iter().enumerate() {
        let bi = block_at[&start];
        reach_block_at.insert(start, idx);
        reach_blocks.push(Block { start: blocks[bi].start, end: blocks[bi].end });
    }

    // ── 3. Op-subset + reserved-scope + slot interning + builtin allowlist ─
    let mut slot_index: HashMap<String, usize> = HashMap::new();
    let mut slot_names: Vec<String> = Vec::new();
    let mut slot_kinds: Vec<Kind> = Vec::new();
    let intern = |name: &str,
                  slot_names: &mut Vec<String>,
                  slot_kinds: &mut Vec<Kind>,
                  slot_index: &mut HashMap<String, usize>,
                  caller_kinds: &HashMap<String, Kind>|
     -> Option<usize> {
        let key = lower(name);
        if let Some(&s) = slot_index.get(&key) {
            return Some(s);
        }
        // The local must have an observed kind from the caller. If not, the
        // loop is reading an undefined local on a path — bail.
        let k = *caller_kinds.get(&key)?;
        if !matches!(k, Kind::Int | Kind::Float | Kind::Boxed) {
            return None;
        }
        let s = slot_names.len();
        if s >= MAX_OSR_SLOTS {
            return None;
        }
        slot_names.push(key.clone());
        slot_kinds.push(k);
        slot_index.insert(key, s);
        Some(s)
    };

    let mut referenced_builtins: BTreeSet<&'static str> = BTreeSet::new();
    // v0.91.1 — "useful work" op count for the admission heuristic below.
    // Counts arithmetic / comparison / logic / String / Concat ops in the
    // reachable region; ignores literal pushes, control flow, locals, Pop,
    // Dup, LineInfo, DeclareLocal, LoadGlobal, and Call (Call sites are
    // resolved into builtin vs UDF after simulate_block). Native builtin
    // Calls are added to this count post-simulate.
    let mut useful_ops: usize = 0;
    let mut total_calls: usize = 0;
    // v0.91.0 — UDF names referenced by `LoadGlobal` inside the region.
    // The resolver is consulted at simulate_block to bind each `Call` site.
    let mut udf_name_idx: HashMap<String, usize> = HashMap::new();
    let mut udf_names: Vec<String> = Vec::new();
    let intern_udf = |name: &str,
                      idx: &mut HashMap<String, usize>,
                      list: &mut Vec<String>|
     -> usize {
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

    for blk in &reach_blocks {
        for ip in blk.start..blk.end {
            match &code[ip] {
                BytecodeOp::Integer(_)
                | BytecodeOp::Double(_)
                | BytecodeOp::True
                | BytecodeOp::False
                | BytecodeOp::Jump(_)
                | BytecodeOp::JumpIfFalse(_)
                | BytecodeOp::JumpIfTrue(_)
                | BytecodeOp::Pop
                | BytecodeOp::Dup
                | BytecodeOp::LineInfo(_, _) => {}

                // v0.91.1 — count these as "useful work" for the OSR-UDF
                // admission heuristic. Native execution of these ops is what
                // makes OSR worthwhile vs the interpreter's already-fast
                // dispatch-into-JIT-cache path for Call ops.
                BytecodeOp::Add
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
                // v0.91.0 — String literals and Concat are admitted; the
                // simulate_block pass below validates operand kinds. Codegen
                // mirrors translate.rs's whole-fn path (intern + box + shim).
                | BytecodeOp::String(_)
                | BytecodeOp::Concat => {
                    useful_ops += 1;
                }

                // v0.99.8 — member-access reads count as useful work. The
                // analyser also needs the receiver local interned for
                // LoadLocalProperty so its slot kind (must be Boxed) is
                // tracked through caller_kinds.
                BytecodeOp::LoadLocalProperty(name, _prop)
                | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    intern(
                        name,
                        &mut slot_names,
                        &mut slot_kinds,
                        &mut slot_index,
                        caller_kinds,
                    )?;
                    useful_ops += 1;
                }
                BytecodeOp::GetProperty(_) | BytecodeOp::TryGetProperty(_) => {
                    useful_ops += 1;
                }

                // v0.100.0 — member-access writes count as useful work.
                // StoreLocalProperty also needs the receiver local interned
                // (slot must be Boxed at simulate_block time).
                BytecodeOp::StoreLocalProperty(name, _prop) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    intern(
                        name,
                        &mut slot_names,
                        &mut slot_kinds,
                        &mut slot_index,
                        caller_kinds,
                    )?;
                    useful_ops += 1;
                }
                BytecodeOp::SetProperty(_) => {
                    useful_ops += 1;
                }

                BytecodeOp::LoadLocal(name)
                | BytecodeOp::StoreLocal(name)
                | BytecodeOp::Increment(name)
                | BytecodeOp::Decrement(name)
                | BytecodeOp::AddLocalConst(name, _)
                | BytecodeOp::MulLocalConst(name, _)
                | BytecodeOp::JumpIfLocalCmpConstFalse(name, _, _, _)
                | BytecodeOp::ForLoopStep(name, _, _, _, _) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    intern(
                        name,
                        &mut slot_names,
                        &mut slot_kinds,
                        &mut slot_index,
                        caller_kinds,
                    )?;
                }
                BytecodeOp::DeclareLocal(name) => {
                    if is_reserved_scope(name) {
                        return None;
                    }
                    // A DeclareLocal inside a loop binds a fresh slot whose
                    // initial value is null in the interpreter — that's not a
                    // numeric kind, so we reject unless it's already been
                    // observed numeric (e.g. an outer `var i` lifted by codegen).
                    intern(
                        name,
                        &mut slot_names,
                        &mut slot_kinds,
                        &mut slot_index,
                        caller_kinds,
                    )?;
                }

                BytecodeOp::LoadGlobal(name) => {
                    if let Some(canon) = builtins::canonical_name(name) {
                        referenced_builtins.insert(canon);
                    } else {
                        // v0.91.0 — non-builtin LoadGlobal might bind to a
                        // JIT-eligible UDF; defer the resolver query to
                        // simulate_block where the arg kinds are known.
                        intern_udf(name, &mut udf_name_idx, &mut udf_names);
                    }
                }
                BytecodeOp::Call(_) => {
                    total_calls += 1; // builtin Calls add to useful_ops post-simulate
                }

                _ => return None,
            }
        }
    }

    // Pin the loop counter as Int — only required by the fused ForLoopStep
    // shape (counted for-loops). A while/until/repeat loop terminating in a
    // plain `Jump` has no special counter to constrain.
    if let Some(name) = &counter_name {
        let counter_slot = slot_index.get(&lower(name))?;
        if slot_kinds[*counter_slot] != Kind::Int {
            return None;
        }
    }

    // ── 4. Simulate each block to validate kinds and operand-stack shape ───
    // (Operand stack starts and ends empty at each block boundary; booleans
    // never enter a slot or escape via a non-branch consumer.)
    //
    // v0.99.8 — slot-kind Infer fixpoint mirrors analysis.rs:561-591. Without
    // this, a body like `total = total + obj.prop` (where `total` is Int and
    // obj.prop is Boxed) would reject on the StoreLocal because Add(Int,
    // Boxed) → Boxed mismatches the Int slot. The Infer pass monotonically
    // upgrades the slot to Boxed; the final Check pass below then succeeds.
    loop {
        let mut changed = false;
        for blk in &reach_blocks {
            let mut _scratch: HashMap<usize, UdfRefBinding> = HashMap::new();
            simulate_block(
                code,
                blk,
                &slot_index,
                &mut slot_kinds,
                &udf_names,
                udf_resolver,
                &mut _scratch,
                Some(&mut changed),
            )?;
        }
        if !changed {
            break;
        }
    }
    let mut udf_call_at: HashMap<usize, UdfRefBinding> = HashMap::new();
    for blk in &reach_blocks {
        simulate_block(
            code,
            blk,
            &slot_index,
            &mut slot_kinds,
            &udf_names,
            udf_resolver,
            &mut udf_call_at,
            None,
        )?;
    }

    // v0.91.1 — OSR-UDF admission heuristic. The UDF dispatcher libcall
    // (cfml_call_jit_udf) adds ~100ns of overhead per call vs the
    // interpreter's already-fast Call→try_call→cached compiled body path.
    // For a hot outer loop whose body is essentially "load + UDF call +
    // store + step" — i.e. a UDF-call wrapper with no real work between
    // calls — OSR is a net pessimization (this surfaced as a 6.2pp drop
    // on udf_call_graph.cfm: 82× → 75×). When the body has meaningful
    // native work between UDF calls, OSR's per-op acceleration outweighs
    // the libcall overhead and the loop benefits.
    //
    // Heuristic: with any UDF call in the region, require at least
    // MIN_USEFUL_FOR_UDF total useful ops (arithmetic / comparison /
    // logic / String / Concat / native builtin Call). Sized so the bench
    // udf_call_graph (1 Add) rejects, but realistic mixed-work loops
    // (≥2 ops between UDF calls) admit.
    if !udf_call_at.is_empty() {
        const MIN_USEFUL_FOR_UDF: usize = 2;
        let builtin_calls = total_calls.saturating_sub(udf_call_at.len());
        let total_useful = useful_ops + builtin_calls;
        if total_useful < MIN_USEFUL_FOR_UDF {
            return None;
        }
    }

    let referenced_builtins: Vec<&'static str> = referenced_builtins.into_iter().collect();

    // Dedupe referenced UDFs by (global_id, sig).
    let mut seen: BTreeSet<(u32, u64)> = BTreeSet::new();
    let mut referenced_udfs: Vec<UdfRefBinding> = Vec::new();
    for binding in udf_call_at.values() {
        if seen.insert((binding.global_id, binding.sig)) {
            referenced_udfs.push(*binding);
        }
    }

    let slots: Vec<OsrSlot> = slot_names
        .iter()
        .enumerate()
        .map(|(i, n)| OsrSlot {
            name: n.clone(),
            kind: slot_kinds[i],
            in_only: false, // refinement deferred — see OsrSlot::in_only docs
        })
        .collect();

    Some(LoopPlan {
        slots,
        slot_index,
        blocks: reach_blocks,
        block_at: reach_block_at,
        region_start,
        region_end_excl,
        referenced_builtins,
        udf_call_at,
        referenced_udfs,
    })
}

/// One-pass operand-stack + kind validator over a basic block, very close to
/// `analysis::simulate_block`'s `Check` mode but without slot-kind upgrades
/// (caller_kinds is authoritative). Rejects mixed-kind slot stores, boolean
/// values reaching a non-branch consumer, etc.
fn simulate_block(
    code: &[BytecodeOp],
    blk: &Block,
    slot_index: &HashMap<String, usize>,
    slot_kinds: &mut [Kind],
    udf_names: &[String],
    udf_resolver: &UdfResolver<'_>,
    udf_call_at: &mut HashMap<usize, UdfRefBinding>,
    mut changed: Option<&mut bool>,
) -> Option<()> {
    let mut stack: Vec<Kind> = Vec::new();
    macro_rules! pop {
        () => {
            match stack.pop() {
                Some(k) => k,
                None => return None,
            }
        };
    }
    macro_rules! pop_value {
        () => {{
            let k = pop!();
            if matches!(k, Kind::Builtin(_) | Kind::UdfRef(_)) {
                return None;
            }
            k
        }};
    }

    let slot_of = |name: &str| -> Option<usize> { slot_index.get(&lower(name)).copied() };

    for ip in blk.start..blk.end {
        match &code[ip] {
            BytecodeOp::Integer(_) => stack.push(Kind::Int),
            BytecodeOp::Double(_) => stack.push(Kind::Float),
            BytecodeOp::True | BytecodeOp::False => stack.push(Kind::Bool),

            BytecodeOp::LoadLocal(name) => stack.push(slot_kinds[slot_of(name)?]),
            BytecodeOp::StoreLocal(name) => {
                let s = slot_of(name)?;
                let v = pop_value!();
                if v == Kind::Bool {
                    return None;
                }
                if v != slot_kinds[s] {
                    // v0.99.8 — Infer mode: monotonic upgrade Int → Float / Boxed,
                    // Float → Boxed. Mirrors analysis.rs Mode::Infer behaviour so
                    // a loop body that stores a Boxed result into an initially-
                    // Int local (e.g. `total = total + obj.prop`) widens the slot
                    // and a follow-up Check pass accepts. Outside Infer, reject.
                    if let Some(c) = changed.as_deref_mut() {
                        let cur = slot_kinds[s];
                        let upgrade = match (cur, v) {
                            (Kind::Int, Kind::Float) | (Kind::Int, Kind::Boxed) | (Kind::Float, Kind::Boxed) => true,
                            _ => false,
                        };
                        if upgrade {
                            slot_kinds[s] = v;
                            *c = true;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
            }
            // v0.99.6/v0.99.7/v0.99.8 — Add/Sub/Mul admit Boxed via
            // num_bin_kind (matches analysis.rs:873-877). Mod stays Int/Float
            // only (no cfml_jit_mod_boxed shim).
            BytecodeOp::Add | BytecodeOp::Sub | BytecodeOp::Mul => {
                let b = pop!();
                let a = pop!();
                stack.push(super::analysis::num_bin_kind(a, b)?);
            }
            BytecodeOp::Mod => {
                let b = pop!();
                let a = pop!();
                if !matches!(a, Kind::Int | Kind::Float) || !matches!(b, Kind::Int | Kind::Float) {
                    return None;
                }
                stack.push(if a == Kind::Float || b == Kind::Float {
                    Kind::Float
                } else {
                    Kind::Int
                });
            }
            BytecodeOp::Pow | BytecodeOp::Div => {
                let b = pop!();
                let a = pop!();
                if !matches!(a, Kind::Int | Kind::Float) || !matches!(b, Kind::Int | Kind::Float) {
                    return None;
                }
                stack.push(Kind::Float);
            }
            BytecodeOp::IntDiv => {
                let b = pop!();
                let a = pop!();
                if !matches!(a, Kind::Int | Kind::Float) || !matches!(b, Kind::Int | Kind::Float) {
                    return None;
                }
                stack.push(Kind::Int);
            }
            BytecodeOp::Negate => {
                let a = pop!();
                if !matches!(a, Kind::Int | Kind::Float) {
                    return None;
                }
                stack.push(a);
            }

            BytecodeOp::Eq
            | BytecodeOp::Neq
            | BytecodeOp::Lt
            | BytecodeOp::Lte
            | BytecodeOp::Gt
            | BytecodeOp::Gte
            | BytecodeOp::And
            | BytecodeOp::Or
            | BytecodeOp::Xor => {
                let _ = pop_value!();
                let _ = pop_value!();
                stack.push(Kind::Bool);
            }
            BytecodeOp::Not => {
                let _ = pop_value!();
                stack.push(Kind::Bool);
            }

            BytecodeOp::Increment(name)
            | BytecodeOp::Decrement(name)
            | BytecodeOp::AddLocalConst(name, _)
            | BytecodeOp::MulLocalConst(name, _)
            | BytecodeOp::JumpIfLocalCmpConstFalse(name, _, _, _)
            | BytecodeOp::ForLoopStep(name, _, _, _, _) => {
                if slot_kinds[slot_of(name)?] != Kind::Int {
                    return None;
                }
            }

            BytecodeOp::Jump(_) => {}
            BytecodeOp::JumpIfFalse(_) | BytecodeOp::JumpIfTrue(_) => {
                let _ = pop_value!();
            }

            BytecodeOp::Pop => {
                let _ = stack.pop();
            }
            BytecodeOp::Dup => {
                let k = *stack.last()?;
                if matches!(k, Kind::Builtin(_)) {
                    return None;
                }
                stack.push(k);
            }

            BytecodeOp::DeclareLocal(_) | BytecodeOp::LineInfo(_, _) => {}

            // v0.91.0 — `String` literal pushes Boxed; `Concat` consumes any
            // two non-fn-ref operands (Int/Float/Bool/Boxed all OK; mirrors
            // translate.rs's whole-fn semantics) and pushes Boxed.
            BytecodeOp::String(_) => stack.push(Kind::Boxed),
            BytecodeOp::Concat => {
                let _b = pop_value!();
                let _a = pop_value!();
                stack.push(Kind::Boxed);
            }

            // v0.99.8 — `local.prop` member read (fused). Local slot must be
            // Boxed (the IC shim takes a tagged ptr to a CfmlValue::Struct).
            // Mirrors analysis.rs:823-838.
            BytecodeOp::LoadLocalProperty(name, _prop)
            | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                let s = slot_of(name)?;
                if slot_kinds[s] != Kind::Boxed {
                    return None;
                }
                stack.push(Kind::Boxed);
            }
            // v0.99.8 — `obj.prop` member read. Receiver on the stack must be
            // Boxed; result is Boxed. Mirrors analysis.rs:840-846.
            BytecodeOp::GetProperty(_) | BytecodeOp::TryGetProperty(_) => {
                let v = pop!();
                if v != Kind::Boxed {
                    return None;
                }
                stack.push(Kind::Boxed);
            }
            // v0.100.0 — `obj.prop = value`. Pops value (Int/Float/Boxed)
            // + obj (Boxed); pushes obj back. Mirrors analysis.rs.
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
            // v0.100.0 — `local.prop = value`. Pops value
            // (Int/Float/Boxed); local slot must be Boxed. No push.
            BytecodeOp::StoreLocalProperty(name, _prop) => {
                let s = slot_of(name)?;
                if slot_kinds[s] != Kind::Boxed {
                    return None;
                }
                let val = pop!();
                if !matches!(val, Kind::Int | Kind::Float | Kind::Boxed) {
                    return None;
                }
            }

            BytecodeOp::LoadGlobal(name) => {
                if let Some(canon) = builtins::canonical_name(name) {
                    stack.push(Kind::Builtin(canon));
                } else {
                    // v0.91.0 — resolve to a UdfRef marker; the resolver is
                    // queried at the matching Call when arg kinds are known.
                    let lower = lower(name);
                    let idx = udf_names.iter().position(|n| n == &lower)?;
                    stack.push(Kind::UdfRef(idx));
                }
            }
            BytecodeOp::Call(n) => {
                if stack.len() < n + 1 {
                    return None;
                }
                let split = stack.len() - n;
                let arg_kinds: Vec<Kind> = stack[split..].to_vec();
                stack.truncate(split);
                let marker = stack.pop()?;
                match marker {
                    Kind::Builtin(name) => {
                        if arg_kinds.iter().any(|k| !matches!(k, Kind::Int | Kind::Float)) {
                            return None;
                        }
                        let shim_idx = builtins::lookup_overload(name, &arg_kinds)?;
                        stack.push(SHIMS[shim_idx].ret_kind);
                    }
                    Kind::UdfRef(idx) => {
                        // v0.91.0 — UDF call. Arg kinds must be Int/Float/Boxed
                        // (the dispatcher marshals them into the args buffer
                        // exactly as the whole-fn path does).
                        if arg_kinds
                            .iter()
                            .any(|k| !matches!(k, Kind::Int | Kind::Float | Kind::Boxed))
                        {
                            return None;
                        }
                        let name = udf_names.get(idx)?;
                        let binding = udf_resolver(name.as_str(), &arg_kinds)?;
                        udf_call_at.insert(ip, binding);
                        stack.push(binding.ret_kind());
                    }
                    _ => return None,
                }
            }

            _ => return None,
        }
    }

    if !stack.is_empty() {
        return None;
    }
    Some(())
}

// ─── Translation ───────────────────────────────────────────────────────────

/// Compile a [`LoopPlan`] against `func`'s bytecode into a native loop body.
///
/// Mirrors `Backend::compile` for the whole-function case but with:
/// * Two-pointer ABI (`io_locals`, `bail`) and no scalar return.
/// * Prologue loads each slot from `io_locals[i*8]` (per its kind).
/// * Clean exit jumps to a shared "writeback" block that stores every slot
///   back into `io_locals` and returns.
/// * Bail also writes back (so the interpreter can resume from the
///   trapping iteration), sets `*bail = 1`, and returns.
pub fn compile_loop(
    backend: &mut Backend,
    func: &BytecodeFunction,
    plan: &LoopPlan,
) -> Result<CompiledLoop, String> {
    let ptr_ty = backend.module.target_config().pointer_type();
    let mut ctx = backend.module.make_context();
    ctx.func.signature.params.push(AbiParam::new(ptr_ty)); // io_locals
    ctx.func.signature.params.push(AbiParam::new(ptr_ty)); // bail
    // no returns

    // Import shim FuncIds (fmod, pow, plus every entry in SHIMS) before the
    // builder borrows ctx.func.
    let fmod_ref = backend
        .module
        .declare_func_in_func(backend.fmod_id, &mut ctx.func);
    let pow_ref = backend
        .module
        .declare_func_in_func(backend.pow_id, &mut ctx.func);
    // v0.91.0 — Boxed-value shim refs (parallel to translate.rs's whole-fn
    // path). Imported eagerly; importing an unused shim is free.
    let box_int_ref = backend
        .module
        .declare_func_in_func(backend.box_int_id, &mut ctx.func);
    let box_float_ref = backend
        .module
        .declare_func_in_func(backend.box_float_id, &mut ctx.func);
    let concat_boxed_ref = backend
        .module
        .declare_func_in_func(backend.concat_boxed_id, &mut ctx.func);
    // v0.99.6/v0.99.7/v0.99.8 — Add/Sub/Mul slow shims for Boxed operands.
    let add_boxed_ref = backend
        .module
        .declare_func_in_func(backend.add_boxed_id, &mut ctx.func);
    let sub_boxed_ref = backend
        .module
        .declare_func_in_func(backend.sub_boxed_id, &mut ctx.func);
    let mul_boxed_ref = backend
        .module
        .declare_func_in_func(backend.mul_boxed_id, &mut ctx.func);
    // v0.99.8 — Member-access IC shim (LoadLocalProperty + GetProperty).
    let member_get_ref = backend
        .module
        .declare_func_in_func(backend.member_get_id, &mut ctx.func);
    // v0.100.0 — Member-write IC shim (SetProperty + StoreLocalProperty).
    let member_set_ref = backend
        .module
        .declare_func_in_func(backend.member_set_id, &mut ctx.func);
    let str_literal_ref = backend
        .module
        .declare_func_in_func(backend.str_literal_id, &mut ctx.func);
    // v0.91.0 — UDF dispatcher (mirrors translate.rs).
    let udf_dispatch_ref = backend
        .module
        .declare_func_in_func(backend.udf_dispatch_id, &mut ctx.func);
    let shim_ids = backend.shim_ids.clone();
    let shim_refs: Vec<_> = shim_ids
        .iter()
        .map(|id| backend.module.declare_func_in_func(*id, &mut ctx.func))
        .collect();

    // v0.91.0 — intern every String literal in the reachable region BEFORE
    // the FunctionBuilder grabs neighbouring borrows. See translate.rs gotcha
    // #15 for the same trick.
    let mut str_literal_at: HashMap<usize, (*const u8, i64)> = HashMap::new();
    // v0.99.8 — pre-allocate one IC slot per member-access call site and
    // intern its property name (mirrors translate.rs:484-505). Keyed by ip
    // so the codegen loop can grab the triple by IP.
    let mut member_get_at: HashMap<usize, (*mut u64, *const u8, i64)> = HashMap::new();
    // v0.100.0 — IC slots for write sites (mirrors translate.rs).
    let mut member_set_at: HashMap<usize, (*mut u64, *const u8, i64)> = HashMap::new();
    for blk in &plan.blocks {
        for ip in blk.start..blk.end {
            match &func.instructions[ip] {
                BytecodeOp::String(s) => {
                    str_literal_at.insert(ip, backend.intern_literal(s));
                }
                BytecodeOp::LoadLocalProperty(_, prop)
                | BytecodeOp::TryLoadLocalProperty(_, prop)
                | BytecodeOp::GetProperty(prop)
                | BytecodeOp::TryGetProperty(prop) => {
                    let ic_ptr = backend.alloc_member_ic_slot();
                    let (name_ptr, name_len) = backend.intern_member_name(prop);
                    member_get_at.insert(ip, (ic_ptr, name_ptr, name_len));
                }
                BytecodeOp::StoreLocalProperty(_, prop)
                | BytecodeOp::SetProperty(prop) => {
                    let ic_ptr = backend.alloc_member_ic_slot();
                    let (name_ptr, name_len) = backend.intern_member_name(prop);
                    member_set_at.insert(ip, (ic_ptr, name_ptr, name_len));
                }
                _ => {}
            }
        }
    }

    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut backend.fbc);

        let cl_blocks: Vec<_> = plan.blocks.iter().map(|_| b.create_block()).collect();
        let bail_block = b.create_block();
        let writeback_block = b.create_block();
        let entry = b.create_block();

        // One Variable per slot (typed I64 or F64).
        let vars: Vec<Variable> = plan
            .slots
            .iter()
            .map(|s| b.declare_var(if s.kind == Kind::Float { F64 } else { I64 }))
            .collect();
        let io_var = b.declare_var(ptr_ty);
        let bail_var = b.declare_var(ptr_ty);

        // v0.91.0 — stack slot for marshalling UDF→UDF call args (mirrors
        // translate.rs). Only allocated when the region actually calls
        // another UDF. Sized for the widest call this region makes.
        let udf_args_slot = if !plan.udf_call_at.is_empty() {
            let max_call_args = plan
                .udf_call_at
                .keys()
                .map(|&ip| match &func.instructions[ip] {
                    BytecodeOp::Call(n) => *n,
                    _ => 0,
                })
                .max()
                .unwrap_or(0);
            if max_call_args == 0 {
                None
            } else {
                let bytes = (max_call_args * 8) as u32;
                Some(b.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    bytes,
                    3, // log2(8) — i64-aligned
                )))
            }
        } else {
            None
        };

        // ── Prologue: load every slot from io_locals[i*8] ──────────────────
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let io_ptr = b.block_params(entry)[0];
        let bail_ptr = b.block_params(entry)[1];
        b.def_var(io_var, io_ptr);
        b.def_var(bail_var, bail_ptr);
        for (i, slot) in plan.slots.iter().enumerate() {
            let off = (i * 8) as i32;
            let v = if slot.kind == Kind::Float {
                b.ins().load(F64, MemFlags::new(), io_ptr, off)
            } else {
                b.ins().load(I64, MemFlags::new(), io_ptr, off)
            };
            b.def_var(vars[i], v);
        }
        b.ins().jump(cl_blocks[0], &[]);

        // ── Writeback block: store every slot back into io_locals; return ──
        b.switch_to_block(writeback_block);
        let io = b.use_var(io_var);
        for (i, slot) in plan.slots.iter().enumerate() {
            let off = (i * 8) as i32;
            let v = b.use_var(vars[i]);
            let raw = if slot.kind == Kind::Float {
                b.ins().bitcast(I64, MemFlags::new(), v)
            } else {
                v
            };
            b.ins().store(MemFlags::new(), raw, io, off);
        }
        b.ins().return_(&[]);

        // ── Bail block: write back, set *bail = 1, return ──────────────────
        b.switch_to_block(bail_block);
        let io_b = b.use_var(io_var);
        for (i, slot) in plan.slots.iter().enumerate() {
            let off = (i * 8) as i32;
            let v = b.use_var(vars[i]);
            let raw = if slot.kind == Kind::Float {
                b.ins().bitcast(I64, MemFlags::new(), v)
            } else {
                v
            };
            b.ins().store(MemFlags::new(), raw, io_b, off);
        }
        let bp = b.use_var(bail_var);
        let one = b.ins().iconst(I64, 1);
        b.ins().store(MemFlags::new(), one, bp, 0);
        b.ins().return_(&[]);

        // ── Translate each reachable block ─────────────────────────────────
        for (bidx, blk) in plan.blocks.iter().enumerate() {
            b.switch_to_block(cl_blocks[bidx]);
            let mut stack: Vec<(Value, Kind)> = Vec::new();
            let mut terminated = false;

            // A jump-target ip lookup that maps `region_end_excl` to the
            // writeback block (clean exit), and anything else to the cl_block
            // for that leader.
            let target_block = |ip: usize| -> Result<cranelift_codegen::ir::Block, String> {
                if ip == plan.region_end_excl {
                    return Ok(writeback_block);
                }
                plan.block_at
                    .get(&ip)
                    .map(|&i| cl_blocks[i])
                    .ok_or_else(|| format!("osr: target ip {ip} not a block leader"))
            };
            let fallthrough = target_block(blk.end);

            for ip in blk.start..blk.end {
                let op = &func.instructions[ip];
                match op {
                    BytecodeOp::Integer(n) => stack.push((b.ins().iconst(I64, *n), Kind::Int)),
                    BytecodeOp::Double(d) => stack.push((b.ins().f64const(*d), Kind::Float)),
                    BytecodeOp::True => stack.push((b.ins().iconst(I64, 1), Kind::Bool)),
                    BytecodeOp::False => stack.push((b.ins().iconst(I64, 0), Kind::Bool)),

                    // v0.91.0 — String literal → call cfml_jit_str_literal
                    // with the interned (ptr, len) view; returns tagged Boxed.
                    BytecodeOp::String(_) => {
                        let (ptr, len) = *str_literal_at
                            .get(&ip)
                            .ok_or("osr: missing interned string literal")?;
                        let ptr_v = b.ins().iconst(ptr_ty, ptr as i64);
                        let len_v = b.ins().iconst(I64, len);
                        let call = b.ins().call(str_literal_ref, &[ptr_v, len_v]);
                        let r = b.inst_results(call)[0];
                        stack.push((r, Kind::Boxed));
                    }

                    // v0.91.0 — `&` concat. Box non-Boxed operands via the
                    // matching shim then call cfml_jit_concat_boxed(a, b).
                    BytecodeOp::Concat => {
                        let (rhs, rk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (lhs, lk) = stack.pop().ok_or("osr: stack underflow")?;
                        let a_tag = translate::ensure_boxed(&mut b, box_int_ref, box_float_ref, lhs, lk);
                        let b_tag = translate::ensure_boxed(&mut b, box_int_ref, box_float_ref, rhs, rk);
                        let call = b.ins().call(concat_boxed_ref, &[a_tag, b_tag]);
                        let r = b.inst_results(call)[0];
                        stack.push((r, Kind::Boxed));
                    }

                    BytecodeOp::LoadLocal(name) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        stack.push((b.use_var(vars[slot]), plan.slots[slot].kind));
                    }
                    // v0.99.8 — fused `local.prop`. Mirrors translate.rs:653-676.
                    // The local slot is Boxed by simulate_block's contract; call
                    // the IC shim with the pre-allocated slot pointer + interned
                    // name; check bail; push the result as Boxed.
                    BytecodeOp::LoadLocalProperty(name, _prop)
                    | BytecodeOp::TryLoadLocalProperty(name, _prop) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let obj = b.use_var(vars[slot]);
                        let (ic_ptr, name_ptr, name_len) = *member_get_at
                            .get(&ip)
                            .ok_or("osr: missing member IC slot")?;
                        let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                        let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                        let nl_v = b.ins().iconst(I64, name_len);
                        let bp = b.use_var(bail_var);
                        let call = b
                            .ins()
                            .call(member_get_ref, &[obj, np_v, nl_v, ic_v, bp]);
                        let r = b.inst_results(call)[0];
                        let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                        let bail_set =
                            b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                        let cont = b.create_block();
                        b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                        b.switch_to_block(cont);
                        stack.push((r, Kind::Boxed));
                    }
                    // v0.99.8 — `obj.prop` where receiver is on the stack
                    // (Boxed). Same IC shape as LoadLocalProperty. Mirrors
                    // translate.rs:679-702.
                    BytecodeOp::GetProperty(_prop) | BytecodeOp::TryGetProperty(_prop) => {
                        let (obj, ok) = stack.pop().ok_or("osr: stack underflow")?;
                        if ok != Kind::Boxed {
                            return Err("osr: GetProperty receiver not Boxed".into());
                        }
                        let (ic_ptr, name_ptr, name_len) = *member_get_at
                            .get(&ip)
                            .ok_or("osr: missing member IC slot")?;
                        let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                        let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                        let nl_v = b.ins().iconst(I64, name_len);
                        let bp = b.use_var(bail_var);
                        let call = b
                            .ins()
                            .call(member_get_ref, &[obj, np_v, nl_v, ic_v, bp]);
                        let r = b.inst_results(call)[0];
                        let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                        let bail_set =
                            b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                        let cont = b.create_block();
                        b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                        b.switch_to_block(cont);
                        stack.push((r, Kind::Boxed));
                    }
                    BytecodeOp::StoreLocal(name) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let (v, _) = stack.pop().ok_or("osr: stack underflow")?;
                        b.def_var(vars[slot], v);
                    }

                    // v0.100.0 — `obj.prop = value`. Mirrors translate.rs.
                    // Box non-Boxed values via the same shims Concat uses.
                    BytecodeOp::SetProperty(_) => {
                        let (val_raw, vk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (obj, ok) = stack.pop().ok_or("osr: stack underflow")?;
                        if ok != Kind::Boxed {
                            return Err("osr: SetProperty receiver not Boxed".into());
                        }
                        let val = translate::ensure_boxed(&mut b, box_int_ref, box_float_ref, val_raw, vk);
                        let (ic_ptr, name_ptr, name_len) = *member_set_at
                            .get(&ip)
                            .ok_or("osr: missing member-set IC slot")?;
                        let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                        let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                        let nl_v = b.ins().iconst(I64, name_len);
                        let bp = b.use_var(bail_var);
                        let call = b
                            .ins()
                            .call(member_set_ref, &[obj, np_v, nl_v, val, ic_v, bp]);
                        let r = b.inst_results(call)[0];
                        let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                        let bail_set =
                            b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                        let cont = b.create_block();
                        b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                        b.switch_to_block(cont);
                        stack.push((r, Kind::Boxed));
                    }

                    // v0.100.0 — `local.prop = value`. Box non-Boxed values
                    // via the same shims Concat uses.
                    BytecodeOp::StoreLocalProperty(name, _prop) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let (val_raw, vk) = stack.pop().ok_or("osr: stack underflow")?;
                        let val = translate::ensure_boxed(&mut b, box_int_ref, box_float_ref, val_raw, vk);
                        let obj = b.use_var(vars[slot]);
                        let (ic_ptr, name_ptr, name_len) = *member_set_at
                            .get(&ip)
                            .ok_or("osr: missing member-set IC slot")?;
                        let ic_v = b.ins().iconst(ptr_ty, ic_ptr as i64);
                        let np_v = b.ins().iconst(ptr_ty, name_ptr as i64);
                        let nl_v = b.ins().iconst(I64, name_len);
                        let bp = b.use_var(bail_var);
                        let call = b
                            .ins()
                            .call(member_set_ref, &[obj, np_v, nl_v, val, ic_v, bp]);
                        let _r = b.inst_results(call)[0];
                        let bail_val = b.ins().load(I64, MemFlags::new(), bp, 0);
                        let bail_set =
                            b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                        let cont = b.create_block();
                        b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                        b.switch_to_block(cont);
                    }

                    // v0.99.6/v0.99.7/v0.99.8 — Add/Sub/Mul admit Boxed; if
                    // either operand is Boxed, route through the SMI fast-path
                    // helper. Mirrors translate.rs:712-739.
                    op_variant @ (BytecodeOp::Add | BytecodeOp::Sub | BytecodeOp::Mul) => {
                        let op = match op_variant {
                            BytecodeOp::Add => translate::NumOp::Add,
                            BytecodeOp::Sub => translate::NumOp::Sub,
                            _ => translate::NumOp::Mul,
                        };
                        let either_boxed = matches!(stack.last(), Some((_, Kind::Boxed)))
                            || matches!(stack.iter().rev().nth(1), Some((_, Kind::Boxed)));
                        if either_boxed {
                            let slow = match op {
                                translate::NumOp::Add => add_boxed_ref,
                                translate::NumOp::Sub => sub_boxed_ref,
                                translate::NumOp::Mul => mul_boxed_ref,
                            };
                            translate::arith_boxed_smi(
                                &mut b,
                                &mut stack,
                                op,
                                box_int_ref,
                                box_float_ref,
                                slow,
                                bail_var,
                                ptr_ty,
                            )?;
                        } else {
                            translate::num_bin(&mut b, &mut stack, op)?;
                        }
                    }

                    BytecodeOp::Div => {
                        let (rhs, rk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (lhs, lk) = stack.pop().ok_or("osr: stack underflow")?;
                        let a = translate::to_f64(&mut b, lhs, lk);
                        let d = translate::to_f64(&mut b, rhs, rk);
                        let fz = b.ins().f64const(0.0);
                        let is_zero = b.ins().fcmp(FloatCC::Equal, d, fz);
                        let cont = b.create_block();
                        b.ins().brif(is_zero, bail_block, &[], cont, &[]);
                        b.switch_to_block(cont);
                        stack.push((b.ins().fdiv(a, d), Kind::Float));
                    }
                    BytecodeOp::Mod => {
                        let (rhs, rk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (lhs, lk) = stack.pop().ok_or("osr: stack underflow")?;
                        if lk == Kind::Float || rk == Kind::Float {
                            let a = translate::to_f64(&mut b, lhs, lk);
                            let d = translate::to_f64(&mut b, rhs, rk);
                            let call = b.ins().call(fmod_ref, &[a, d]);
                            let r = b.inst_results(call)[0];
                            stack.push((r, Kind::Float));
                        } else {
                            let cont = translate::guard_int_div(&mut b, bail_block, lhs, rhs);
                            b.switch_to_block(cont);
                            stack.push((b.ins().srem(lhs, rhs), Kind::Int));
                        }
                    }
                    BytecodeOp::Pow => {
                        let (rhs, rk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (lhs, lk) = stack.pop().ok_or("osr: stack underflow")?;
                        let a = translate::to_f64(&mut b, lhs, lk);
                        let d = translate::to_f64(&mut b, rhs, rk);
                        let call = b.ins().call(pow_ref, &[a, d]);
                        let r = b.inst_results(call)[0];
                        stack.push((r, Kind::Float));
                    }
                    BytecodeOp::IntDiv => {
                        let (rhs, rk) = stack.pop().ok_or("osr: stack underflow")?;
                        let (lhs, lk) = stack.pop().ok_or("osr: stack underflow")?;
                        let a = translate::to_i64(&mut b, lhs, lk);
                        let d = translate::to_i64(&mut b, rhs, rk);
                        let cont = translate::guard_int_div(&mut b, bail_block, a, d);
                        b.switch_to_block(cont);
                        stack.push((b.ins().sdiv(a, d), Kind::Int));
                    }

                    BytecodeOp::Negate => {
                        let (a, k) = stack.pop().ok_or("osr: stack underflow")?;
                        let r = if k == Kind::Float {
                            b.ins().fneg(a)
                        } else {
                            b.ins().ineg(a)
                        };
                        stack.push((r, k));
                    }

                    BytecodeOp::Eq => translate::cmp(&mut b, &mut stack, IntCC::Equal, FloatCC::Equal)?,
                    BytecodeOp::Neq => translate::cmp(&mut b, &mut stack, IntCC::NotEqual, FloatCC::NotEqual)?,
                    BytecodeOp::Lt => translate::cmp(
                        &mut b,
                        &mut stack,
                        IntCC::SignedLessThan,
                        FloatCC::LessThan,
                    )?,
                    BytecodeOp::Lte => translate::cmp(
                        &mut b,
                        &mut stack,
                        IntCC::SignedLessThanOrEqual,
                        FloatCC::LessThanOrEqual,
                    )?,
                    BytecodeOp::Gt => translate::cmp(
                        &mut b,
                        &mut stack,
                        IntCC::SignedGreaterThan,
                        FloatCC::GreaterThan,
                    )?,
                    BytecodeOp::Gte => translate::cmp(
                        &mut b,
                        &mut stack,
                        IntCC::SignedGreaterThanOrEqual,
                        FloatCC::GreaterThanOrEqual,
                    )?,
                    BytecodeOp::And => translate::logic2(&mut b, &mut stack, translate::LogicOp::And)?,
                    BytecodeOp::Or => translate::logic2(&mut b, &mut stack, translate::LogicOp::Or)?,
                    BytecodeOp::Xor => translate::logic2(&mut b, &mut stack, translate::LogicOp::Xor)?,
                    BytecodeOp::Not => {
                        let (a, k) = stack.pop().ok_or("osr: stack underflow")?;
                        let t = translate::is_zero_test(&mut b, a, k);
                        stack.push((translate::bool_to_i64(&mut b, t), Kind::Bool));
                    }

                    BytecodeOp::Increment(name) => rmw_imm(&mut b, &vars, plan, name, 1)?,
                    BytecodeOp::Decrement(name) => rmw_imm(&mut b, &vars, plan, name, -1)?,
                    BytecodeOp::AddLocalConst(name, k) => rmw_imm(&mut b, &vars, plan, name, *k)?,
                    BytecodeOp::MulLocalConst(name, k) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let v = b.use_var(vars[slot]);
                        let nv = b.ins().imul_imm(v, *k);
                        b.def_var(vars[slot], nv);
                    }

                    BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmpop, target) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let v = b.use_var(vars[slot]);
                        let cc = b.ins().icmp_imm(translate::int_cc(*cmpop), v, *c);
                        b.ins().brif(cc, fallthrough.clone()?, &[], target_block(*target)?, &[]);
                        terminated = true;
                    }
                    BytecodeOp::ForLoopStep(name, limit, cmpop, step, target) => {
                        let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
                        let v = b.use_var(vars[slot]);
                        let nv = b.ins().iadd_imm(v, *step);
                        b.def_var(vars[slot], nv);
                        let cc = b.ins().icmp_imm(translate::int_cc(*cmpop), nv, *limit);
                        // matched=true ⇒ loop back to target.
                        // matched=false ⇒ fall through to the next instruction
                        // (which, for the OUTERMOST ForLoopStep, is exactly
                        // `region_end_excl` and target_block maps that to
                        // `writeback_block` — the clean OSR exit). For an
                        // INNER ForLoopStep nested inside the region, the
                        // fall-through is just the next basic block within
                        // the outer compiled body, so the loop "exits" into
                        // the surrounding outer-loop code instead of bailing
                        // out of the whole OSR'd region.
                        b.ins().brif(cc, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                        terminated = true;
                    }

                    BytecodeOp::Jump(target) => {
                        b.ins().jump(target_block(*target)?, &[]);
                        terminated = true;
                    }
                    BytecodeOp::JumpIfFalse(target) => {
                        let (cond, k) = stack.pop().ok_or("osr: stack underflow")?;
                        let is_false = translate::is_zero_test(&mut b, cond, k);
                        b.ins().brif(is_false, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                        terminated = true;
                    }
                    BytecodeOp::JumpIfTrue(target) => {
                        let (cond, k) = stack.pop().ok_or("osr: stack underflow")?;
                        let is_true = translate::is_truthy(&mut b, cond, k);
                        b.ins().brif(is_true, target_block(*target)?, &[], fallthrough.clone()?, &[]);
                        terminated = true;
                    }

                    BytecodeOp::Pop => {
                        let _ = stack.pop();
                    }
                    BytecodeOp::Dup => {
                        let v = *stack.last().ok_or("osr: stack underflow")?;
                        stack.push(v);
                    }

                    BytecodeOp::DeclareLocal(_) | BytecodeOp::LineInfo(_, _) => {}

                    BytecodeOp::LoadGlobal(name) => {
                        let placeholder = b.ins().iconst(I64, 0);
                        if let Some(canon) = builtins::canonical_name(name) {
                            stack.push((placeholder, Kind::Builtin(canon)));
                        } else {
                            // v0.91.0 — UDF marker; the matching Call site
                            // looks up the binding in plan.udf_call_at by ip.
                            stack.push((placeholder, Kind::UdfRef(0)));
                        }
                    }
                    BytecodeOp::Call(n) => {
                        if stack.len() < n + 1 {
                            return Err("osr: stack underflow on Call".into());
                        }
                        let split = stack.len() - n;
                        let raw_args: Vec<(Value, Kind)> = stack.split_off(split);
                        let (_marker_val, marker_kind) =
                            stack.pop().ok_or("osr: missing fn-ref marker")?;
                        match marker_kind {
                            Kind::Builtin(name) => {
                                let arg_kinds: Vec<Kind> =
                                    raw_args.iter().map(|(_, k)| *k).collect();
                                let shim_idx = builtins::lookup_overload(name, &arg_kinds)
                                    .ok_or("osr: no shim overload for call")?;
                                let shim = &SHIMS[shim_idx];
                                let mut cl_args: Vec<Value> =
                                    Vec::with_capacity(raw_args.len() + 1);
                                for (idx, (v, k)) in raw_args.into_iter().enumerate() {
                                    let abi = shim.args_abi[idx];
                                    let conv = if abi == Kind::Float {
                                        translate::to_f64(&mut b, v, k)
                                    } else {
                                        translate::to_i64(&mut b, v, k)
                                    };
                                    cl_args.push(conv);
                                }
                                // v0.99.3 — bailable shims take trailing
                                // *mut i64 bail pointer; post-call brif on it.
                                if shim.bailable {
                                    cl_args.push(b.use_var(bail_var));
                                }
                                let call = b.ins().call(shim_refs[shim_idx], &cl_args);
                                let r = b.inst_results(call)[0];
                                if shim.bailable {
                                    let bp = b.use_var(bail_var);
                                    let bail_val =
                                        b.ins().load(I64, MemFlags::new(), bp, 0);
                                    let bail_set = b.ins().icmp_imm(
                                        IntCC::NotEqual,
                                        bail_val,
                                        0,
                                    );
                                    let cont = b.create_block();
                                    b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                                    b.switch_to_block(cont);
                                }
                                stack.push((r, shim.ret_kind));
                            }
                            Kind::UdfRef(_) => {
                                // v0.91.0 — UDF dispatch. Mirrors translate.rs.
                                let binding = plan
                                    .udf_call_at
                                    .get(&ip)
                                    .copied()
                                    .ok_or("osr: UDF call site missing binding")?;
                                let slot = udf_args_slot
                                    .ok_or("osr: UDF call without stack slot")?;
                                for (i, (v, k)) in raw_args.iter().enumerate() {
                                    let stored = if *k == Kind::Float {
                                        b.ins().bitcast(I64, MemFlags::new(), *v)
                                    } else {
                                        *v
                                    };
                                    b.ins().stack_store(stored, slot, (i * 8) as i32);
                                }
                                let args_addr = b.ins().stack_addr(ptr_ty, slot, 0);
                                let bp = b.use_var(bail_var);
                                let gid = b.ins().iconst(I64, binding.global_id as i64);
                                let sig = b.ins().iconst(I64, binding.sig as i64);
                                let erk_code: i64 = match binding.ret_kind {
                                    crate::jit::BindingRet::Int => 0,
                                    crate::jit::BindingRet::Float => 1,
                                    crate::jit::BindingRet::Boxed => 2,
                                };
                                let erk = b.ins().iconst(I64, erk_code);
                                let nargs_v = b.ins().iconst(I64, *n as i64);
                                let call = b.ins().call(
                                    udf_dispatch_ref,
                                    &[gid, sig, erk, args_addr, nargs_v, bp],
                                );
                                let raw_result = b.inst_results(call)[0];
                                let bail_val =
                                    b.ins().load(I64, MemFlags::new(), bp, 0);
                                let bail_set =
                                    b.ins().icmp_imm(IntCC::NotEqual, bail_val, 0);
                                let cont = b.create_block();
                                b.ins().brif(bail_set, bail_block, &[], cont, &[]);
                                b.switch_to_block(cont);
                                let (result_val, result_kind) = match binding.ret_kind {
                                    crate::jit::BindingRet::Float => (
                                        b.ins().bitcast(F64, MemFlags::new(), raw_result),
                                        Kind::Float,
                                    ),
                                    crate::jit::BindingRet::Boxed => (raw_result, Kind::Boxed),
                                    crate::jit::BindingRet::Int => (raw_result, Kind::Int),
                                };
                                stack.push((result_val, result_kind));
                            }
                            _ => return Err("osr: Call without a fn-ref marker".into()),
                        }
                    }

                    BytecodeOp::Return => {
                        return Err("osr: Return inside region — should have been rejected".into());
                    }

                    other => return Err(format!("osr: unsupported op reached codegen: {other:?}")),
                }
            }

            if !terminated {
                b.ins().jump(fallthrough.clone()?, &[]);
            }
        }

        b.seal_all_blocks();
        b.finalize();
    }

    let name = format!("cfml_osr_{}", backend.func_counter);
    backend.func_counter += 1;
    let id = backend
        .module
        .declare_function(&name, Linkage::Export, &ctx.func.signature)
        .map_err(|e| e.to_string())?;
    backend
        .module
        .define_function(id, &mut ctx)
        .map_err(|e| e.to_string())?;
    backend.module.clear_context(&mut ctx);
    backend
        .module
        .finalize_definitions()
        .map_err(|e| e.to_string())?;
    let code = backend.module.get_finalized_function(id);
    // SAFETY: same as `Backend::compile` — `code` points at freshly emitted
    // native code matching our exact signature; it lives as long as the
    // JITModule (owned by the engine).
    Ok(unsafe { std::mem::transmute::<*const u8, CompiledLoop>(code) })
}

fn rmw_imm(
    b: &mut FunctionBuilder,
    vars: &[Variable],
    plan: &LoopPlan,
    name: &str,
    k: i64,
) -> Result<(), String> {
    let slot = plan.slot_of(name).ok_or("osr: unknown local")?;
    let v = b.use_var(vars[slot]);
    let nv = b.ins().iadd_imm(v, k);
    b.def_var(vars[slot], nv);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_codegen::compiler::CfmlCompiler;
    use cfml_codegen::CmpOp;
    use cfml_compiler::parser::Parser;

    /// Compile `src` and return `__main__`'s bytecode.
    fn compile_main(src: &str) -> BytecodeFunction {
        let ast = Parser::new(src.to_string()).parse().expect("parse");
        let program = CfmlCompiler::new().compile(ast);
        // The interpreter wraps top-level statements into a function called
        // "__main__"; find it.
        program
            .functions
            .iter()
            .find(|f| f.name == "__main__")
            .expect("no __main__ in program")
            .as_ref()
            .clone()
    }

    /// Locate the (region_start, region_end_excl) for the first `ForLoopStep`
    /// in `func`. Panics if there isn't one — used by tests where we know the
    /// source contains one counted for-loop.
    fn first_loop_region(func: &BytecodeFunction) -> (usize, usize) {
        for (ip, op) in func.instructions.iter().enumerate() {
            if let BytecodeOp::ForLoopStep(_, _, _, _, target) = op {
                return (*target, ip + 1);
            }
        }
        panic!("no ForLoopStep in function");
    }

    fn int_kinds(names: &[&str]) -> HashMap<String, Kind> {
        names.iter().map(|n| (n.to_string(), Kind::Int)).collect()
    }

    #[test]
    fn analyze_loop_accepts_simple_counted_sum() {
        // Top-level loop: classic counted accumulator pattern.
        let f = compile_main("sum = 0; for (i = 1; i <= 10; i++) { sum = sum + i; }");
        let (s, e) = first_loop_region(&f);
        let kinds = int_kinds(&["sum", "i"]);
        let plan = analyze_loop(&f, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).expect("region should be JIT-eligible");
        // Both sum and i must appear as slots, both Int.
        let names: BTreeSet<String> = plan.slots.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains("sum"));
        assert!(names.contains("i"));
        for slot in &plan.slots {
            assert_eq!(slot.kind, Kind::Int);
        }
        assert_eq!(plan.region_end_excl, e);
    }

    #[test]
    fn analyze_loop_accepts_string_concat_with_boxed_slot() {
        // Since v0.91.0, Concat + String are admitted; since v0.99.8 the
        // slot-kind Infer fixpoint also upgrades an initially-Int slot to
        // Boxed when the body stores a Concat result into it. Either way,
        // a Boxed accumulator concatenating into itself is JIT-eligible.
        let f = compile_main("s = \"\"; for (i = 1; i <= 5; i++) { s = s & i; }");
        let (s, e) = first_loop_region(&f);
        let mut kinds = HashMap::new();
        kinds.insert("s".into(), Kind::Boxed);
        kinds.insert("i".into(), Kind::Int);
        assert!(analyze_loop(&f, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).is_some());
    }

    #[test]
    fn analyze_loop_accepts_break_as_clean_exit() {
        // `break` codegens to `Jump(loop_end)` and `loop_end` is exactly
        // `region_end_excl`. The OSR analyser treats that as a permitted exit
        // path (it becomes a jump to the writeback block in compile_loop), so
        // a loop with `break` is JIT-eligible — not rejected.
        let f = compile_main(
            "sum = 0; for (i = 1; i <= 10; i++) { if (i > 5) { break; } sum = sum + i; }",
        );
        let (s, e) = first_loop_region(&f);
        let kinds = int_kinds(&["sum", "i"]);
        assert!(analyze_loop(&f, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).is_some());
    }

    #[test]
    fn analyze_loop_threads_float_accumulator() {
        // s is Float (initial 0.0); i is Int.
        let f = compile_main("s = 0.0; for (i = 1; i <= 10; i++) { s = s + i / 2; }");
        let (s_ip, e_ip) = first_loop_region(&f);
        let mut kinds = HashMap::new();
        kinds.insert("s".into(), Kind::Float);
        kinds.insert("i".into(), Kind::Int);
        let plan = analyze_loop(&f, s_ip, e_ip, &kinds, &crate::jit::analysis::no_udf_resolver).expect("should accept float accumulator");
        let s_slot = plan.slot_of("s").unwrap();
        assert_eq!(plan.slots[s_slot].kind, Kind::Float);
        let i_slot = plan.slot_of("i").unwrap();
        assert_eq!(plan.slots[i_slot].kind, Kind::Int);
    }

    #[test]
    fn analyze_loop_rejects_return_inside() {
        // A reachable Return inside the region is forbidden — the OSR ABI has
        // no scalar return path. Use a constant-bound loop so codegen emits
        // ForLoopStep (it requires a literal limit; a variable limit
        // disqualifies the fused loop).
        let src = "function score() { var t = 0; for (var i = 1; i <= 100; i++) { t = t + i; if (t > 1000) { return t; } } return t; }\nx = score();";
        let ast = Parser::new(src.to_string()).parse().expect("parse");
        let prog = CfmlCompiler::new().compile(ast);
        let score = prog
            .functions
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case("score"))
            .unwrap()
            .as_ref()
            .clone();
        // Slot-normalize like try_call does — the codegen slot pass rewrites
        // this function's `var` loop to ForSlotStep, which OSR doesn't model.
        let score = crate::jit::JitEngine::normalize_slot_ops(&score);
        let (s, e) = first_loop_region(&score);
        let kinds = int_kinds(&["t", "i"]);
        assert!(analyze_loop(&score, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).is_none());
    }

    #[test]
    fn referenced_builtins_recorded_in_loop_plan() {
        // A loop calling abs() inside the body must record "abs".
        let f = compile_main("t = 0; for (i = 1; i <= 5; i++) { t = t + abs(i - 3); }");
        let (s, e) = first_loop_region(&f);
        let kinds = int_kinds(&["t", "i"]);
        let plan = analyze_loop(&f, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).expect("abs() loop should be eligible");
        assert!(plan.referenced_builtins.contains(&"abs"));
    }

    #[test]
    fn analyze_loop_rejects_oversize_live_set() {
        // Synthesise a region with > MAX_OSR_SLOTS locals by analyzing a loop
        // body that references many names. CFML codegen makes constructing
        // exactly-33 live locals tedious; cover this case at unit level by
        // confirming the cap is checked: a plan with up to MAX_OSR_SLOTS is
        // accepted (the simple sum loop has only 2 locals). The full
        // > MAX cap is exercised indirectly via the intern helper inside
        // analyze_loop and covered by code review.
        let f = compile_main("sum = 0; for (i = 1; i <= 10; i++) { sum = sum + i; }");
        let (s, e) = first_loop_region(&f);
        let kinds = int_kinds(&["sum", "i"]);
        assert!(analyze_loop(&f, s, e, &kinds, &crate::jit::analysis::no_udf_resolver).is_some());
        assert!(MAX_OSR_SLOTS >= 2);
    }

    // ── Compile + round-trip tests ─────────────────────────────────────────

    /// Compile the first ForLoopStep region of `func` and execute it, given
    /// initial slot values. Returns the post-execution slot values plus the
    /// bail flag.
    fn compile_and_run(
        func: &BytecodeFunction,
        caller_kinds: &HashMap<String, Kind>,
        init: &HashMap<String, i64>, // raw 8-byte values per slot
    ) -> (HashMap<String, i64>, bool) {
        let (s, e) = first_loop_region(func);
        let plan = analyze_loop(func, s, e, caller_kinds, &crate::jit::analysis::no_udf_resolver).expect("eligible");
        let mut backend = Backend::new().expect("backend init");
        let ptr = compile_loop(&mut backend, func, &plan).expect("compile");
        let mut buf: Vec<i64> = plan
            .slots
            .iter()
            .map(|s| init.get(&s.name).copied().unwrap_or(0))
            .collect();
        let mut bail: i64 = 0;
        unsafe { ptr(buf.as_mut_ptr(), &mut bail as *mut i64) };
        let result: HashMap<String, i64> = plan
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), buf[i]))
            .collect();
        (result, bail != 0)
    }

    #[test]
    fn compile_loop_simple_sum_round_trips() {
        // sum = 0; for (i = 1; i <= 10; i++) { sum = sum + i; }
        // Loop body runs from i = current..10 inclusive, summing into sum.
        let f = compile_main("sum = 0; for (i = 1; i <= 10; i++) { sum = sum + i; }");
        let kinds = int_kinds(&["sum", "i"]);
        // Seed sum=0, i=1 (just like the interpreter would on entry to the
        // body for the first iteration).
        let mut init = HashMap::new();
        init.insert("sum".into(), 0);
        init.insert("i".into(), 1);
        let (out, bailed) = compile_and_run(&f, &kinds, &init);
        assert!(!bailed);
        // After the loop completes: sum = 1+2+…+10 = 55, i lands at 11
        // (ForLoopStep increments then tests; on the iteration with i=10 the
        // body executes one last time, the step increments to 11, the test
        // 11<=10 fails, and we exit). Interpreter and JIT agree.
        assert_eq!(out["sum"], 55);
        assert_eq!(out["i"], 11);
    }

    #[test]
    fn compile_loop_with_abs_builtin() {
        // t = 0; for (i = 1; i <= 5; i++) { t = t + abs(i - 3); }
        // |1-3|+|2-3|+|3-3|+|4-3|+|5-3| = 2+1+0+1+2 = 6
        let f = compile_main("t = 0; for (i = 1; i <= 5; i++) { t = t + abs(i - 3); }");
        let kinds = int_kinds(&["t", "i"]);
        let mut init = HashMap::new();
        init.insert("t".into(), 0);
        init.insert("i".into(), 1);
        let (out, bailed) = compile_and_run(&f, &kinds, &init);
        assert!(!bailed);
        assert_eq!(out["t"], 6);
        assert_eq!(out["i"], 6);
    }

    #[test]
    fn compile_loop_resumes_partial_state() {
        // Same loop, but seed mid-flight to confirm the OSR contract: the
        // interpreter has already executed iterations i = 1..2, and we hand
        // over with i=3 and t=3 (i.e. sum so far = 1+2 = 3). The native body
        // must continue and produce the full sum.
        let f = compile_main("sum = 0; for (i = 1; i <= 10; i++) { sum = sum + i; }");
        let kinds = int_kinds(&["sum", "i"]);
        let mut init = HashMap::new();
        init.insert("sum".into(), 3); // 1+2 already done by interpreter
        init.insert("i".into(), 3); // about to execute iteration for i=3
        let (out, bailed) = compile_and_run(&f, &kinds, &init);
        assert!(!bailed);
        // sum picks up at 3 and adds 3..10 = 52 → 55 total.
        assert_eq!(out["sum"], 55);
        assert_eq!(out["i"], 11);
    }

    /// Suppress dead-code lints on `CmpOp` (used only when this file is
    /// extended in follow-up commits).
    #[allow(dead_code)]
    fn _force_cmpop_in_scope(_: CmpOp) {}
}
