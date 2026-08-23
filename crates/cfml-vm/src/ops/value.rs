//! Stack-only op handlers: literals, arithmetic, comparison, logical operators,
//! and array/struct construction. Every function here reads and writes the
//! operand stack and nothing else — no VM state, no frame locals, no jumps —
//! which is why they need no context beyond `&mut Vec<CfmlValue>`.
//!
//! Bodies were moved verbatim from the `execute_function_body` dispatch match
//! (roadmap P3 slice 1). See `super` for the rules.
//!
//! Thirteen of these carry `#[inline(always)]` rather than `#[inline]`. They are
//! the trivial ops LLVM chose to emit out of line after extraction — a call that
//! did not exist when the body sat inside the dispatch match. Forcing the inline
//! restores the original codegen and measures ~1% on realistic workloads (Preside
//! TestBox quick suite: all 3 interleaved rounds faster, median -1.12%; Wheels
//! core suite same direction; CLI suite neutral). Anything larger is left to
//! LLVM's judgement, which declined for good reason on the big handlers.

use crate::{
    arith_binary_op, arith_numeric_op, arith_operand, binary_op, cfml_compare, cfml_equal,
    cfml_strict_equal, compare_op, to_number, CfmlVirtualMachine,
};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn op_null(stack: &mut Vec<CfmlValue>) {
    stack.push(CfmlValue::Null)
}

#[inline]
pub(crate) fn op_true(stack: &mut Vec<CfmlValue>) {
    stack.push(CfmlValue::Bool(true))
}

#[inline]
pub(crate) fn op_false(stack: &mut Vec<CfmlValue>) {
    stack.push(CfmlValue::Bool(false))
}

#[inline]
pub(crate) fn op_integer(stack: &mut Vec<CfmlValue>, n: i64) {
    stack.push(CfmlValue::Int(n))
}

#[inline]
pub(crate) fn op_double(stack: &mut Vec<CfmlValue>, d: f64) {
    stack.push(CfmlValue::Double(d))
}

#[inline(always)]
pub(crate) fn op_string(stack: &mut Vec<CfmlValue>, s: &str) {
    stack.push(CfmlValue::string(s.to_string()))
}

// ---------------------------------------------------------------------------
// Stack shuffling
// ---------------------------------------------------------------------------

#[inline(always)]
pub(crate) fn op_pop(stack: &mut Vec<CfmlValue>) {
    stack.pop();
}

#[inline(always)]
pub(crate) fn op_dup(stack: &mut Vec<CfmlValue>) {
    if let Some(val) = stack.last() {
        stack.push(val.clone());
    }
}

#[inline(always)]
pub(crate) fn op_swap(stack: &mut Vec<CfmlValue>) {
    let len = stack.len();
    if len >= 2 {
        stack.swap(len - 1, len - 2);
    }
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn op_add(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| {
        Ok(match (&a, &b) {
            (CfmlValue::Int(i), CfmlValue::Int(j)) => CfmlValue::Int(i + j),
            (CfmlValue::Double(x), CfmlValue::Double(y)) => CfmlValue::Double(x + y),
            (CfmlValue::Int(i), CfmlValue::Double(d)) => CfmlValue::Double(*i as f64 + d),
            (CfmlValue::Double(d), CfmlValue::Int(i)) => CfmlValue::Double(d + *i as f64),
            // CFML `+` is ARITHMETIC ONLY — `&` concatenates — so "2" + "1"
            // adds to 3 and a non-numeric operand THROWS rather than falling
            // back to concatenation (GH #350). See `arith_operand`.
            _ => CfmlValue::Double(arith_operand(&a)? + arith_operand(&b)?),
        })
    })
}

#[inline]
pub(crate) fn op_sub(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| arith_numeric_op(&a, &b, |x, y| x - y))
}

#[inline]
pub(crate) fn op_mul(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| arith_numeric_op(&a, &b, |x, y| x * y))
}

#[inline]
pub(crate) fn op_mod(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| {
        Ok(match (&a, &b) {
            (CfmlValue::Int(i), CfmlValue::Int(j)) if *j != 0 => CfmlValue::Int(i % j),
            _ => CfmlValue::Double(arith_operand(&a)? % arith_operand(&b)?),
        })
    })
}

#[inline]
pub(crate) fn op_pow(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| {
        Ok(CfmlValue::Double(arith_operand(&a)?.powf(arith_operand(&b)?)))
    })
}

#[inline]
pub(crate) fn op_int_div(stack: &mut Vec<CfmlValue>) -> Result<(), CfmlError> {
    arith_binary_op(stack, |a, b| {
        let x = arith_operand(&a)? as i64;
        let y = arith_operand(&b)? as i64;
        Ok(if y == 0 {
            CfmlValue::Int(0)
        } else {
            CfmlValue::Int(x / y)
        })
    })
}

#[inline(always)]
pub(crate) fn op_negate(stack: &mut Vec<CfmlValue>) {
    if let Some(val) = stack.pop() {
        match val {
            CfmlValue::Int(i) => stack.push(CfmlValue::Int(-i)),
            CfmlValue::Double(d) => stack.push(CfmlValue::Double(-d)),
            _ => {
                if let Some(n) = to_number(&val) {
                    stack.push(CfmlValue::Double(-n));
                } else {
                    stack.push(CfmlValue::Int(0));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn op_eq(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_equal(a, b));
}

#[inline]
pub(crate) fn op_neq(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| !cfml_equal(a, b));
}

#[inline]
pub(crate) fn op_strict_eq(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_strict_equal(a, b));
}

#[inline]
pub(crate) fn op_strict_neq(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| !cfml_strict_equal(a, b));
}

#[inline]
pub(crate) fn op_lt(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_compare(a, b) < 0);
}

#[inline]
pub(crate) fn op_lte(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_compare(a, b) <= 0);
}

#[inline]
pub(crate) fn op_gt(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_compare(a, b) > 0);
}

#[inline]
pub(crate) fn op_gte(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| cfml_compare(a, b) >= 0);
}

// ---------------------------------------------------------------------------
// CFML-specific operators
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn op_contains(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| {
        let haystack = a.as_string().to_lowercase();
        let needle = b.as_string().to_lowercase();
        haystack.contains(&needle)
    });
}

#[inline]
pub(crate) fn op_does_not_contain(stack: &mut Vec<CfmlValue>) {
    compare_op(stack, |a, b| {
        let haystack = a.as_string().to_lowercase();
        let needle = b.as_string().to_lowercase();
        !haystack.contains(&needle)
    });
}

// ---------------------------------------------------------------------------
// Logical
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn op_and(stack: &mut Vec<CfmlValue>) {
    binary_op(stack, |a, b| CfmlValue::Bool(a.is_true() && b.is_true()));
}

#[inline]
pub(crate) fn op_or(stack: &mut Vec<CfmlValue>) {
    binary_op(stack, |a, b| CfmlValue::Bool(a.is_true() || b.is_true()));
}

#[inline(always)]
pub(crate) fn op_not(stack: &mut Vec<CfmlValue>) {
    if let Some(a) = stack.pop() {
        stack.push(CfmlValue::Bool(!a.is_true()));
    }
}

#[inline]
pub(crate) fn op_xor(stack: &mut Vec<CfmlValue>) {
    binary_op(stack, |a, b| CfmlValue::Bool(a.is_true() ^ b.is_true()));
}

#[inline]
pub(crate) fn op_eqv(stack: &mut Vec<CfmlValue>) {
    binary_op(stack, |a, b| CfmlValue::Bool(a.is_true() == b.is_true()));
}

#[inline]
pub(crate) fn op_imp(stack: &mut Vec<CfmlValue>) {
    binary_op(stack, |a, b| CfmlValue::Bool(!a.is_true() || b.is_true()));
}

// ---------------------------------------------------------------------------
// Construction / containers
// ---------------------------------------------------------------------------

#[inline(always)]
pub(crate) fn op_build_array(stack: &mut Vec<CfmlValue>, count: usize) {
    let mut elements = Vec::new();
    for _ in 0..count {
        if let Some(val) = stack.pop() {
            elements.push(val);
        }
    }
    elements.reverse();
    stack.push(CfmlValue::array(elements));
}

#[inline(always)]
pub(crate) fn op_build_struct(stack: &mut Vec<CfmlValue>, count: usize) {
    let mut pairs = Vec::new();
    for _ in 0..count {
        let value = stack.pop().unwrap_or(CfmlValue::Null);
        let key = stack.pop().unwrap_or(CfmlValue::string(String::new()));
        // §3.5: the map key must be owned, but `key` was just popped
        // off the stack — move its String out instead of copying.
        pairs.push((key.into_string(), value));
    }
    let mut map = ValueMap::default();
    for (k, v) in pairs.into_iter().rev() {
        map.insert(k, v);
    }
    stack.push(CfmlValue::strukt(map));
}

#[inline(always)]
pub(crate) fn op_concat_arrays(stack: &mut Vec<CfmlValue>) {
    let right = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
    let left = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
    if let (CfmlValue::Array(a), CfmlValue::Array(b)) = (left, right) {
        // Concatenation produces a NEW array (not a mutation of
        // either operand), so snapshot both into a fresh Vec.
        let mut v = a.snapshot();
        v.extend(b.iter());
        stack.push(CfmlValue::array(v));
    } else {
        stack.push(CfmlValue::array(Vec::new()));
    }
}

#[inline(always)]
pub(crate) fn op_merge_structs(stack: &mut Vec<CfmlValue>) {
    let right = stack.pop().unwrap_or(CfmlValue::strukt(ValueMap::default()));
    let left = stack.pop().unwrap_or(CfmlValue::strukt(ValueMap::default()));
    if let (CfmlValue::Struct(a), CfmlValue::Struct(b)) = (left, right) {
        let mut m = a.snapshot();
        for (k, v) in b.iter() {
            m.insert(k, v);
        }
        stack.push(CfmlValue::strukt(m));
    } else {
        stack.push(CfmlValue::strukt(ValueMap::default()));
    }
}

// ---------------------------------------------------------------------------
// Misc stack-only ops
// ---------------------------------------------------------------------------

#[inline(always)]
pub(crate) fn op_is_null(stack: &mut Vec<CfmlValue>) {
    if let Some(val) = stack.pop() {
        stack.push(CfmlValue::Bool(matches!(val, CfmlValue::Null)));
    } else {
        stack.push(CfmlValue::Bool(true));
    }
}

#[inline(always)]
pub(crate) fn op_get_static_property(stack: &mut Vec<CfmlValue>, member: &str) {
    let holder = stack.pop().unwrap_or(CfmlValue::Null);
    let val = CfmlVirtualMachine::read_static_member(&holder, member).unwrap_or(CfmlValue::Null);
    stack.push(val);
}

#[inline(always)]
pub(crate) fn op_catch_match(stack: &mut Vec<CfmlValue>, catch_type: &str) {
    // Peek (do NOT consume) the exception value the catch handler
    // was entered with, and push whether its `type` matches this
    // clause's declared type.
    let exc_type = match stack.last() {
        Some(CfmlValue::Struct(s)) => s.get("type").map(|v| v.as_string()).unwrap_or_default(),
        _ => String::new(),
    };
    let matches = CfmlVirtualMachine::catch_type_matches(catch_type, &exc_type);
    stack.push(CfmlValue::Bool(matches));
}
