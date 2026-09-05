//! Per-op interpreter handlers, extracted one slice at a time out of the giant
//! `execute_function_body` dispatch match (perf roadmap **P3**).
//!
//! # Why this module exists
//!
//! The dispatch match is ~7,000 lines in one function, which is hostile to read,
//! to profile per-op, and to change with any confidence about blast radius.
//! Extracting each arm into a named handler makes an op's cost attributable and
//! its behaviour testable in isolation.
//!
//! ⚠️ The ORIGINAL motivation was a planned admission-free Tier-0 baseline JIT,
//! which needed every op body callable as a function (you cannot call into the
//! middle of a `match` arm). **The JIT was removed in v0.653.0** — it admitted
//! 13 of 1,345 functions on Preside and was a net slowdown; see known-issues
//! §77. The extraction is kept on its own merits, above. Do not reintroduce a
//! JIT-shaped constraint here on the strength of the old rationale.
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
//!   here). The signature doubles as machine-checked documentation of what each
//!   op can actually reach. Later slices escalate as needed (`&mut CfmlVirtualMachine`, then a
//!   frame-context struct for the ops that jump or unwind).
//! * **`#[inline]` everywhere.** These are hot and tiny; the extraction must not
//!   cost a call. Perf-neutrality is a release gate, not an aspiration.
//!
//! Slice 1 (this commit): the 39 arms that touch *only* the operand stack and
//! have no control flow — literals, arithmetic, comparison, logical operators,
//! array/struct construction and the two container merge ops.
//!
//! Slice 2: the arms needing the live VM plus `ip` — `Div`/`Concat` (catchable
//! errors), `Throw`/`Rethrow`, `Print`, and the custom-tag/jump ops — in
//! `effect.rs`, which documents how `continue` and `return Err` translate.
//!
//! Slice 3: the four hot property/index arms (`GetProperty`/`TryGetProperty`,
//! `GetIndex`, `SetProperty`, `GetKeys`) in `access.rs` — ~694 lines.
//!
//! Slice 4: the frame-state arms needing exactly one frame field (`locals` or the
//! slot vector) in `frame.rs`. A `FrameCtx` struct is deliberately deferred until
//! an op genuinely needs most of the frame — true of the call/store ops, not these.
//!
//! Slice 5: 22 more frame arms in `locals.rs` — try/except bookkeeping, the fused
//! local arithmetic ops, local load/append, the super-call ops. The three
//! jump arms (`Jump`, `JumpIfFalse`, `JumpIfTrue`) are deferred to the
//! `FrameCtx` slice, since they drive `ip` directly.

pub(crate) mod access;
pub(crate) mod effect;
pub(crate) mod frame;
pub(crate) mod locals;
pub(crate) mod value;
