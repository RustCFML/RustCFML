//! Shared plumbing for the extracted VM-intercept modules (roadmap Part -1 / P2).
//!
//! # Why a sentinel rather than a smarter `handles()`
//! Each extracted module exposes `handles(name)` plus a `dispatch_*` holding the bodies
//! moved verbatim out of `call_function`. That works only while every guarded name is
//! guaranteed to return — and several are not. `cfdirectory` returns for
//! `action = "list"` and **falls through to the generic builtin path for every other
//! action**; `test_existence_cache.cfm` caught an args-blind `handles()` turning that
//! fall-through into a hard error.
//!
//! Predicting fall-through inside `handles()` would mean duplicating each branch's inner
//! conditions (the action string, argument shapes) in a second place, where they would
//! immediately start drifting from the code they mirror. Instead a dispatch that reaches
//! its end returns [`unhandled`], and the caller treats that as "carry on down
//! `call_function`" — exactly what the original `if` chain did by falling out of the block.
//!
//! The sentinel is a private marker, never surfaced to CFML: the caller either converts it
//! back into fall-through or, if it somehow escapes, it reads as an internal error rather
//! than a plausible-looking CFML message.

use cfml_common::vm::CfmlError;

/// Marker text identifying the "this module did not handle the call" sentinel. The NUL
/// prefix makes accidental collision with a genuine CFML error message impossible.
const UNHANDLED_MARKER: &str = "\u{0}rcfml:intercept-unhandled";

/// Returned by a `dispatch_*` that fell out of all its branches.
#[inline]
pub(crate) fn unhandled() -> CfmlError {
    CfmlError::runtime(UNHANDLED_MARKER.to_string())
}

/// True if `e` is the fall-through sentinel from [`unhandled`].
#[inline]
pub(crate) fn is_unhandled(e: &CfmlError) -> bool {
    e.message == UNHANDLED_MARKER
}
