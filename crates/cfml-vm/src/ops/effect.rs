//! Op handlers that need the live VM and may redirect control: division and
//! concatenation (both of which can raise a catchable error), `throw`/`rethrow`,
//! output, and the two custom-tag/jump ops.
//!
//! Signature shape (roadmap P3 slice 2): `(vm, stack, ip, …payload)`. Two notes
//! on the faithful translation of the original match arms:
//!
//! * **`continue` becomes `return`.** The dispatch `match` is the last statement
//!   in the interpreter loop body, so `continue` inside an arm is *exactly*
//!   equivalent to falling off the end of that arm. Every `ip = X; continue;`
//!   therefore lowers to `*ip = X; return …;` with no change in behaviour.
//! * **`return Err(e)` stays `return Err(e)`.** The arms lived in a function
//!   returning `CfmlResult`, so a bare `return Err` propagated out of the whole
//!   frame; the call site now does the same with `?`.
//!
//! No handler here needs to `break` the interpreter loop — the only op that does
//! is `Halt`, a one-line `break` left inline, since a handler cannot break its
//! caller's loop and routing that through a return enum would be pure noise.
//! An `OpFlow` enum arrives with the frame-state slice, which has ops that
//! genuinely return early (`Return`, `cfexit`).

use crate::CfmlVirtualMachine;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;

/// `/` — CFML throws a catchable `Expression` error on division by zero.
#[inline]
pub(crate) fn op_div(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    if let (Some(b), Some(a)) = (stack.pop(), stack.pop()) {
        let x = crate::to_arith_number(&a).unwrap_or(0.0);
        let y = crate::to_arith_number(&b).unwrap_or(1.0);
        if y == 0.0 {
            // CFML throws on division by zero
            let mut exception = ValueMap::default();
            exception.insert(
                "message".to_string(),
                CfmlValue::string("Division by zero is not allowed.".to_string()),
            );
            exception.insert(
                "type".to_string(),
                CfmlValue::string("Expression".to_string()),
            );
            exception.insert("detail".to_string(), CfmlValue::string(String::new()));
            exception.insert("tagcontext".to_string(), vm.build_tag_context());
            CfmlVirtualMachine::add_root_cause(&mut exception);
            let error_val = CfmlValue::strukt(exception);
            vm.last_exception = Some(error_val.clone());
            if let Some(handler) = vm.try_stack.pop() {
                while stack.len() > handler.stack_depth {
                    stack.pop();
                }
                vm.restore_capture_state(&handler);
                stack.push(error_val);
                *ip = handler.catch_ip;
                return Ok(());
            } else {
                return Err(CfmlError::runtime(
                    "Division by zero is not allowed.".to_string(),
                ));
            }
        } else {
            stack.push(CfmlValue::Double(x / y));
        }
    }
    Ok(())
}

/// `&` string concatenation.
#[inline]
pub(crate) fn op_concat(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    if let (Some(b), Some(a)) = (stack.pop(), stack.pop()) {
        // Lucee parity: `&` (and multi-part `"a#x#b"`) on a complex
        // value throws a catchable `expression` error — "Can't cast
        // Complex Object Type [Struct] to String" — instead of
        // dumping it. The dump was not just wrong-vs-Lucee: on a
        // densely shared object graph it expanded to an O(2^depth)
        // string and hung the process (ColdBox boot). Left operand
        // is checked first, matching CFML left-to-right evaluation.
        let sa = match a.to_string_strict() {
            Ok(s) => s,
            Err(e) => match vm.raise_catchable(stack, &e.message, "expression") {
                Ok(catch_ip) => {
                    *ip = catch_ip;
                    return Ok(());
                }
                Err(e) => return Err(e),
            },
        };
        let sb = match b.to_string_strict() {
            Ok(s) => s,
            Err(e) => match vm.raise_catchable(stack, &e.message, "expression") {
                Ok(catch_ip) => {
                    *ip = catch_ip;
                    return Ok(());
                }
                Err(e) => return Err(e),
            },
        };
        stack.push(CfmlValue::string(format!("{}{}", sa, sb)));
    }
    Ok(())
}

/// `throw` — routes to an open handler if there is one, else propagates.
#[inline]
pub(crate) fn op_throw(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    let raw = stack
        .pop()
        .unwrap_or(CfmlValue::string("Unknown error".to_string()));
    // The bare statement form `throw "msg";` (and `throw expr;`)
    // pushes a simple value. Wrap any non-struct value into a
    // proper exception struct so `cfcatch.message`/`.type` resolve
    // — mirroring the throw(...) function-call form. A struct value
    // (re-throw of a caught exception) is preserved verbatim.
    let error_val = match raw {
        CfmlValue::Struct(_) => raw,
        other => {
            let mut m = ValueMap::default();
            m.insert("message".to_string(), CfmlValue::string(other.as_string()));
            m.insert(
                "type".to_string(),
                CfmlValue::string("Application".to_string()),
            );
            m.insert("detail".to_string(), CfmlValue::string(String::new()));
            m.insert("tagcontext".to_string(), vm.build_tag_context());
            CfmlVirtualMachine::add_root_cause(&mut m);
            CfmlValue::strukt(m)
        }
    };
    vm.last_exception = Some(error_val.clone());
    if let Some(handler) = vm.try_stack.pop() {
        // Unwind stack
        while stack.len() > handler.stack_depth {
            stack.pop();
        }
        vm.restore_capture_state(&handler);
        stack.push(error_val);
        *ip = handler.catch_ip;
    } else {
        // Propagate the original message (not the serialized
        // struct) so resolve_catch_error_val matches last_exception
        // and reuses the full cfcatch struct — preserving the
        // error's `type`/`detail` across the frame boundary.
        let mut err = CfmlError::runtime(match &error_val {
            CfmlValue::Struct(s) => s
                .get("message")
                .map(|m| m.as_string())
                .unwrap_or_else(|| error_val.as_string()),
            _ => error_val.as_string(),
        });
        err.stack_trace = vm.build_stack_trace();
        return Err(err);
    }
    Ok(())
}

/// `rethrow` — same routing as `throw`, re-using the last exception.
#[inline]
pub(crate) fn op_rethrow(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    let error_val = vm
        .last_exception
        .clone()
        .unwrap_or(CfmlValue::string("No exception to rethrow".to_string()));
    if let Some(handler) = vm.try_stack.pop() {
        while stack.len() > handler.stack_depth {
            stack.pop();
        }
        vm.restore_capture_state(&handler);
        stack.push(error_val);
        *ip = handler.catch_ip;
    } else {
        // Propagate the original message (not the serialized
        // struct) so resolve_catch_error_val matches last_exception
        // and reuses the full cfcatch struct — preserving the
        // error's `type`/`detail` across the frame boundary.
        let mut err = CfmlError::runtime(match &error_val {
            CfmlValue::Struct(s) => s
                .get("message")
                .map(|m| m.as_string())
                .unwrap_or_else(|| error_val.as_string()),
            _ => error_val.as_string(),
        });
        err.stack_trace = vm.build_stack_trace();
        return Err(err);
    }
    Ok(())
}

/// Statement-level output (`Print`), newline-terminated.
#[inline]
pub(crate) fn op_print(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
) -> Result<(), CfmlError> {
    if let Some(val) = stack.pop() {
        // Lucee parity: outputting a complex value throws a
        // catchable `expression` error rather than dumping it.
        let s = match val.to_string_strict() {
            Ok(s) => s,
            Err(e) => match vm.raise_catchable(stack, &e.message, "expression") {
                Ok(catch_ip) => {
                    *ip = catch_ip;
                    return Ok(());
                }
                Err(e) => return Err(e),
            },
        };
        vm.output_buffer.push_str(&s);
        vm.output_buffer.push('\n');
    }
    Ok(())
}

/// Trailing op of a lowered `__cfcustomtag_end()`; re-enters the tag body when
/// `<cfexit method="loop">` armed a repeat.
#[inline]
pub(crate) fn op_tag_loop_back(
    vm: &mut CfmlVirtualMachine,
    stack: &mut Vec<CfmlValue>,
    ip: &mut usize,
    body_start: usize,
) {
    // Trailing op of a lowered `__cfcustomtag_end()` statement,
    // standing in for the `Pop` an expression statement would
    // normally get.
    stack.pop();
    if vm.pending_tag_loop {
        // `<cfexit method="loop">`: the end handler has already
        // re-armed the tag state and pushed a fresh capture
        // buffer, so re-entering the body from the top resumes
        // the same tag instance. Only the body repeats — the
        // start phase ran once and is not revisited.
        vm.pending_tag_loop = false;
        *ip = body_start;
    }
}

/// A `break`/`continue` is jumping out of `n` custom tag bodies whose
/// `__cfcustomtag_end()` will never run.
#[inline]
pub(crate) fn op_abandon_tag_pairs(vm: &mut CfmlVirtualMachine, n: usize) {
    vm.abandon_tag_pairs(n);
}

/// Null-coalescing jump: peek, and jump when the top of stack is not null
/// (leaving the value in place either way).
#[inline]
pub(crate) fn op_jump_if_not_null(stack: &[CfmlValue], ip: &mut usize, target: usize) {
    // Peek at the top of stack - if not null, jump (leave value on stack)
    // If null, continue (leave null on stack)
    if let Some(val) = stack.last() {
        if !matches!(val, CfmlValue::Null) {
            *ip = target;
        }
    }
}
