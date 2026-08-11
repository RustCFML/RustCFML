//! Per-op interpreter handlers, extracted one slice at a time out of the giant
//! `execute_function_body` dispatch match (perf roadmap **P3**).
//!
//! # Why this module exists
//!
//! The dispatch match is ~7,000 lines in one function. That shape blocks the
//! only credible route to Lucee-class performance (roadmap P4/P5): a
//! **admission-free Tier-0 baseline JIT** must be able to emit a direct call to
//! the interpreter's own logic for any op it does not inline natively. You
//! cannot call into the middle of a `match` arm — so every op's body has to be
//! reachable as a function first. The 2026-08-10 admission scan is what forces
//! this: 83.2% of Preside's functions are permanently ineligible for the
//! current pure-kernel JIT and 97.6% of op-weight lives inside them, so
//! *widening admission* is dead and *removing the admission requirement* is the
//! only remaining architecture. Shimming needs handlers.
//!
//! # Rules for this module (keep it boring)
//!
//! * **Bodies move verbatim.** A slice is a pure code move: no behaviour
//!   changes, no cleanups, no "while I'm here" fixes. The gate for every slice
//!   is *all suites at exact baseline* plus a perf-neutral A/B. Anything that
//!   looks worth fixing gets noted and done in its own commit, so a regression
//!   is never ambiguous about which change caused it.
//! * **Narrowest signature that suffices.** A handler takes only the frame
//!   state it actually touches (`&mut Vec<CfmlValue>` for the stack-only ops
//!   here). That is deliberate: Tier-0 emits a direct call per op, so fewer
//!   arguments means a cheaper shim and no context struct to materialize — and
//!   the signature doubles as machine-checked documentation of what each op can
//!   reach. Later slices escalate as needed (`&mut CfmlVirtualMachine`, then a
//!   frame-context struct for the ops that jump or unwind).
//! * **`#[inline]` everywhere.** These are hot and tiny; the extraction must not
//!   cost a call. Perf-neutrality is a release gate, not an aspiration.
//!
//! Slice 1 (this commit): the 39 arms that touch *only* the operand stack and
//! have no control flow — literals, arithmetic, comparison, logical operators,
//! array/struct construction and the two container merge ops. These are also
//! exactly the ops Tier-0 will inline natively first, so they are the natural
//! starting point.
//!
//! Slice 2: the arms needing the live VM plus `ip` — `Div`/`Concat` (catchable
//! errors), `Throw`/`Rethrow`, `Print`, and the custom-tag/jump ops — in
//! `effect.rs`, which documents how `continue` and `return Err` translate.
//!
//! Slice 3: the four hot property/index arms (`GetProperty`/`TryGetProperty`,
//! `GetIndex`, `SetProperty`, `GetKeys`) in `access.rs` — ~694 lines.

pub(crate) mod access;
pub(crate) mod effect;
pub(crate) mod value;
