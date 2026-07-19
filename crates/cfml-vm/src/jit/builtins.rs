//! Static allowlist of pure Tier-1 builtin shims the JIT may call.
//!
//! Each [`Shim`] is one signature (overloads share a `name`). The JIT only
//! permits a `LoadGlobal(name)` + `Call(n)` pair where `name` matches an entry
//! here and the actual operand kinds match some overload's `args_req`. Because
//! the shims are `extern "C"` Rust fns whose semantics mirror the interpreter
//! (`cfml-stdlib::builtins::fn_abs/min/max`), the JIT result is bit-identical to
//! the interpreter result for every accepted call shape.
//!
//! Shadowing safety lives in the engine: at `try_call` time we re-check that
//! each referenced builtin name is not shadowed in the VM's `user_functions` /
//! `globals` — if it is, we bail to the interpreter so a user-defined `abs`
//! still wins.
//!
//! Adding a new builtin = one `extern "C"` fn + one [`Shim`] entry; both
//! `analysis` and `translate` read the table by index, so no other edits.

use super::analysis::Kind;

/// What an argument kind must be for a [`Shim`] overload to apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KindReq {
    /// Must be exactly `Kind::Int`.
    Int,
    /// Must be exactly `Kind::Float`.
    Float,
    /// `Kind::Int` or `Kind::Float` — promoted at the ABI to the shim's
    /// `args_abi` slot via `to_f64` / `to_i64`.
    Numeric,
    /// Must be exactly `Kind::Boxed` (Option-γ tag-pointer). v0.92.0+ —
    /// crosses to the shim as a raw `i64` holding the tagged pointer; the
    /// shim borrows the underlying `CfmlValue` via `boxed::borrow_tagged`.
    Boxed,
}

impl KindReq {
    fn matches(self, k: Kind) -> bool {
        match (self, k) {
            (KindReq::Int, Kind::Int) => true,
            (KindReq::Float, Kind::Float) => true,
            (KindReq::Numeric, Kind::Int | Kind::Float) => true,
            (KindReq::Boxed, Kind::Boxed) => true,
            _ => false,
        }
    }
}

/// One ABI-level shim. Multiple `Shim`s may share `name` (overloads).
pub struct Shim {
    /// Lowercase CFML builtin name (must match `vm.builtins` key).
    pub name: &'static str,
    /// Per-arg acceptance rule (length = arity).
    pub args_req: &'static [KindReq],
    /// Per-arg ABI kind in the emitted IR (the operand value is converted to
    /// this kind before the `call`). Length = arity.
    pub args_abi: &'static [Kind],
    /// Result kind produced by the shim.
    pub ret_kind: Kind,
    /// Cranelift module symbol name (registered with `JITBuilder::symbol`).
    pub sym: &'static str,
    /// Raw fn pointer for `JITBuilder::symbol` to hand off to the linker.
    pub addr: *const u8,
    /// v0.99.3 — when true, the shim takes a trailing `*mut i64` bail
    /// parameter and may set `*bail = 1` to signal a runtime error. The
    /// translator appends the bail pointer to the call arglist and emits
    /// a post-call `brif bail, bail_block, cont` (mirrors the UDF Call
    /// dispatcher's bail pattern). When false the shim is infallible:
    /// signature is `(args...) -> ret`, no trailing arg, no post-check.
    pub bailable: bool,
}

// `Shim`'s `addr` is a function pointer; we share the table across threads.
// Function pointers are trivially `Send + Sync` semantically — Rust's stdlib
// just declines the auto-derive because of `*const u8`. The pointers live in
// this crate's read-only data section for the life of the process.
unsafe impl Sync for Shim {}

// ── extern "C" shims ────────────────────────────────────────────────────────
//
// Each mirrors a `cfml-stdlib::builtins::fn_*` entry exactly. They never
// allocate, never throw, and never touch the VM — pure functions of their
// arguments, safe to call from JIT'd code.

/// Mirrors `fn_abs` for `CfmlValue::Int(i)` → `Int(i.abs())`.
/// `i64::abs` panics on `INT_MIN` in debug; the interpreter does the same.
/// To keep the JIT side panic-free (and to let the interpreter fall through
/// for that one pathological input), we return `INT_MIN` for `INT_MIN` —
/// matching the *release-build* interpreter semantics (`i.abs()` wraps in
/// release because the underlying `wrapping_neg` is the same op).
extern "C" fn cfml_abs_i64(x: i64) -> i64 {
    x.wrapping_abs()
}

/// Mirrors `fn_abs` for `CfmlValue::Double(d)` → `Double(d.abs())`.
extern "C" fn cfml_abs_f64(x: f64) -> f64 {
    x.abs()
}

/// Mirrors `fn_min`: both operands promoted to `f64` via `get_float`, result
/// always `Double(a.min(b))`.
extern "C" fn cfml_min_f64(a: f64, b: f64) -> f64 {
    a.min(b)
}

/// Mirrors `fn_max`: both operands promoted to `f64`, result `Double(a.max(b))`.
extern "C" fn cfml_max_f64(a: f64, b: f64) -> f64 {
    a.max(b)
}

// ── Single-arg pure-math shims ────────────────────────────────────────────
// Every shim below is `extern "C"` and mirrors a `cfml-stdlib::builtins::fn_*`
// entry verbatim. Operand is always promoted to `f64` at the ABI boundary
// (`KindReq::Numeric` / `Kind::Float`), matching `get_float(args, 0)` in the
// interpreter. Return is `f64` for math functions and `i64` for the rounding /
// sign / truncation family — same as the interpreter's `CfmlValue::Double`
// vs `CfmlValue::Int`.

/// `fn_floor` — `x.floor() as i64`. CFML returns an `Int`.
extern "C" fn cfml_floor_i64(x: f64) -> i64 {
    x.floor() as i64
}

/// `fn_ceiling` — `x.ceil() as i64`. CFML returns an `Int`.
extern "C" fn cfml_ceiling_i64(x: f64) -> i64 {
    x.ceil() as i64
}

/// `fn_round` (1-arg form only) — half-up toward positive infinity, matching
/// Lucee/Adobe's `Math.round`. Bit-exact with `(x + 0.5).floor() as i64`.
/// Rust's `f64::round` is half-away-from-zero, which would diverge on negatives.
/// CFML returns `Int`.
extern "C" fn cfml_round_i64(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// `fn_sgn` — `1` / `-1` / `0` for positive / negative / zero. CFML returns `Int`.
extern "C" fn cfml_sgn_i64(x: f64) -> i64 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// `fn_fix` — truncate toward zero. `x.trunc() as i64`. CFML returns `Int`.
extern "C" fn cfml_fix_i64(x: f64) -> i64 {
    x.trunc() as i64
}

/// `fn_sqr` — square root. CFML uses the name `sqr` (not `sqrt`).
extern "C" fn cfml_sqr_f64(x: f64) -> f64 {
    x.sqrt()
}

extern "C" fn cfml_exp_f64(x: f64) -> f64 {
    x.exp()
}
/// `fn_log` — natural log (`ln`). CFML's `log` IS the natural log.
extern "C" fn cfml_log_f64(x: f64) -> f64 {
    x.ln()
}
extern "C" fn cfml_log10_f64(x: f64) -> f64 {
    x.log10()
}
extern "C" fn cfml_sin_f64(x: f64) -> f64 {
    x.sin()
}
extern "C" fn cfml_cos_f64(x: f64) -> f64 {
    x.cos()
}
extern "C" fn cfml_tan_f64(x: f64) -> f64 {
    x.tan()
}
extern "C" fn cfml_asin_f64(x: f64) -> f64 {
    x.asin()
}
extern "C" fn cfml_acos_f64(x: f64) -> f64 {
    x.acos()
}
extern "C" fn cfml_atan_f64(x: f64) -> f64 {
    x.atan()
}

// ── Bit-twiddling shims ───────────────────────────────────────────────────
// Mirror `fn_bit_and/or/xor/not/shln/shrn` in cfml-stdlib. Operands are
// always Int at the ABI; `bitNot` truncates to Java's 32-bit `int` semantics
// (Lucee/Adobe parity) before complementing.

extern "C" fn cfml_bit_and_i64(a: i64, b: i64) -> i64 {
    a & b
}
extern "C" fn cfml_bit_or_i64(a: i64, b: i64) -> i64 {
    a | b
}
extern "C" fn cfml_bit_xor_i64(a: i64, b: i64) -> i64 {
    a ^ b
}
/// `fn_bit_not` truncates to 32 bits before complementing to match the
/// interpreter's Java-int semantics: `(!(x as i32)) as i64`.
extern "C" fn cfml_bit_not_i64(x: i64) -> i64 {
    (!(x as i32)) as i64
}
extern "C" fn cfml_bit_shln_i64(a: i64, b: i64) -> i64 {
    a << b
}
extern "C" fn cfml_bit_shrn_i64(a: i64, b: i64) -> i64 {
    a >> b
}

// ── incrementValue / decrementValue — typed overloads ───────────────────
// `fn_increment_value` / `fn_decrement_value` in cfml-stdlib pattern-match
// on `Int` and `Double` and add/subtract 1; the JIT only ever sees those
// two kinds, so two overloads each cover the JIT-eligible paths exactly.
// Non-numeric coercion stays in the interpreter (we'd bail in analysis).

extern "C" fn cfml_increment_value_i64(x: i64) -> i64 {
    x.wrapping_add(1)
}
extern "C" fn cfml_increment_value_f64(x: f64) -> f64 {
    x + 1.0
}
extern "C" fn cfml_decrement_value_i64(x: i64) -> i64 {
    x.wrapping_sub(1)
}
extern "C" fn cfml_decrement_value_f64(x: f64) -> f64 {
    x - 1.0
}

// ── bitMaskRead / bitMaskSet / bitMaskClear — 3 / 4-arg Int → Int ────────
// Mirror `fn_bit_mask_read` / `fn_bit_mask_set` / `fn_bit_mask_clear` in
// cfml-stdlib bit-for-bit.

extern "C" fn cfml_bit_mask_read_i64(number: i64, start: i64, length: i64) -> i64 {
    (number >> start) & ((1i64 << length) - 1)
}
extern "C" fn cfml_bit_mask_set_i64(number: i64, mask: i64, start: i64, length: i64) -> i64 {
    let clear_mask = ((1i64 << length) - 1) << start;
    (number & !clear_mask) | ((mask & ((1i64 << length) - 1)) << start)
}
extern "C" fn cfml_bit_mask_clear_i64(number: i64, start: i64, length: i64) -> i64 {
    number & !(((1i64 << length) - 1) << start)
}

/// `fn_pow(base, exp)` — `base.powf(exp)`. The `^` infix operator already
/// calls `translate::cfml_pow` (a private extern in the translate module);
/// this adds the function-call form too so `pow(2, 10)` JITs identically
/// to `2 ^ 10`.
extern "C" fn cfml_pow_fn_f64(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

// ── v0.92.0 Boxed-argument shims ────────────────────────────────────────────
// Each one takes a tagged `i64` (TAG_PTR-encoded `*const CfmlValue` or
// inline SMI from `boxed.rs`), materialises the underlying value via
// `boxed::materialize_tagged` (handles both heap and SMI tags — see
// gotcha #32 in JIT_NEXT_SESSION.md), and either returns an `i64`
// (numeric ret_kind) or constructs a fresh `CfmlValue` and pushes it
// into the active arena (Boxed ret_kind).
//
// All six mirror their `cfml-stdlib::builtins::fn_*` counterparts bit-for-bit
// and are infallible by design: `len` is defensive (returns 0 for unknown
// types in interp), and the case/trim shims use `as_string()` which works on
// every `CfmlValue` variant. No bail path needed.

/// Mirrors `fn_len`: `String → s.len()` (bytes), `Bool/Int/Double → chars of
/// the stringified form`, `Array/Struct/Binary → element count`,
/// `QueryColumn → chars of stringified first row`, anything else → `0`.
extern "C" fn cfml_len_boxed_i64(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    match &v {
        CfmlValue::String(s) => s.len() as i64,
        CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) => {
            v.as_string().chars().count() as i64
        }
        CfmlValue::Array(a) => a.len() as i64,
        CfmlValue::Struct(s) => s.len() as i64,
        CfmlValue::Binary(b) => b.len() as i64,
        CfmlValue::QueryColumn(..) => v.as_string().chars().count() as i64,
        _ => 0,
    }
}

/// Mirrors `fn_ucase`: `CfmlValue::string(v.as_string().to_uppercase())`,
/// boxed into the active arena. Result kind: `Boxed`.
extern "C" fn cfml_ucase_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().to_uppercase())) as i64
}

/// Mirrors `fn_lcase`.
extern "C" fn cfml_lcase_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().to_lowercase())) as i64
}

/// Mirrors `fn_trim`.
extern "C" fn cfml_trim_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().trim().to_string())) as i64
}

/// Mirrors `fn_ltrim`.
extern "C" fn cfml_ltrim_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().trim_start().to_string())) as i64
}

/// Mirrors `fn_rtrim`.
extern "C" fn cfml_rtrim_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().trim_end().to_string())) as i64
}

/// Mirrors `fn_reverse`: `chars().rev().collect()` of the stringified value.
/// Lucee/RustCFML `reverse()` is string-only (arrays go through `arrayReverse`).
extern "C" fn cfml_reverse_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s: String = v.as_string().chars().rev().collect();
    super::arena::box_into_active(CfmlValue::string(s)) as i64
}

/// Mirrors `fn_asc`: first char of stringified arg as its `i64` code point,
/// or `0` for empty. CFML returns `Int`.
extern "C" fn cfml_asc_boxed_i64(tagged: i64) -> i64 {
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    v.as_string().chars().next().map_or(0, |c| c as i64)
}

/// Mirrors `fn_strip_cr`: drops every `\r` from the stringified arg.
extern "C" fn cfml_strip_cr_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::string(v.as_string().replace('\r', ""))) as i64
}

/// Mirrors `fn_html_edit_format`: `& < > "` → entity refs.
extern "C" fn cfml_html_edit_format_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v
        .as_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    super::arena::box_into_active(CfmlValue::string(s)) as i64
}

/// Mirrors `fn_html_code_format`: htmlEditFormat then `<pre>…</pre>` wrap.
extern "C" fn cfml_html_code_format_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let inner = v
        .as_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    super::arena::box_into_active(CfmlValue::string(format!("<pre>{inner}</pre>"))) as i64
}

/// Mirrors `fn_encode_for_html`: the OWASP/ESAPI HTMLEntityCodec for an HTML
/// *content* context — every char below U+0100 is encoded except the immune set
/// `, . - _` and space; named entity where one exists, else a lowercase hex
/// numeric entity. MUST stay byte-identical to cfml-stdlib `fn_encode_for_html`.
extern "C" fn cfml_encode_for_html_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let mut s = String::new();
    for c in v.as_string().chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => s.push(c),
            ',' | '.' | '-' | '_' | ' ' => s.push(c),
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            c if (c as u32) >= 0x100 => s.push(c),
            c => s.push_str(&format!("&#x{:x};", c as u32)),
        }
    }
    super::arena::box_into_active(CfmlValue::string(s)) as i64
}

// ── v0.99.2 single-arg Boxed→Boxed string shims ─────────────────────────────
// urlEncodedFormat / urlDecode / jsStringFormat — all bit-exact mirrors of
// the cfml-stdlib::builtins::fn_url_encode / fn_url_decode / fn_js_string_format
// implementations. Infallible.

/// Mirrors `fn_url_encode` (registered as `urlEncodedFormat`).
extern "C" fn cfml_url_encoded_format_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    use std::fmt::Write;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '*' => result.push(c),
            // Space -> %20 (Lucee/ACF parity), not `+`. See GH #270.
            _ => {
                for b in c.to_string().as_bytes() {
                    let _ = write!(result, "%{:02X}", b);
                }
            }
        }
    }
    super::arena::box_into_active(CfmlValue::string(result)) as i64
}

/// Mirrors `fn_url_decode`.
extern "C" fn cfml_url_decode_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let mut result = String::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let hex: String = chars.by_ref().take(2).collect();
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    bytes.push(byte);
                }
                if chars.peek() != Some(&'%') {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    } else {
                        for b in &bytes {
                            result.push(*b as char);
                        }
                    }
                    bytes.clear();
                }
            }
            '+' => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(' ');
            }
            _ => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(c);
            }
        }
    }
    if !bytes.is_empty() {
        if let Ok(decoded) = String::from_utf8(bytes.clone()) {
            result.push_str(&decoded);
        }
    }
    super::arena::box_into_active(CfmlValue::string(result)) as i64
}

/// Mirrors `fn_js_string_format`: escapes `\`, `'`, `"`, `\n`, `\r`, `\t`.
extern "C" fn cfml_js_string_format_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let escaped = v
        .as_string()
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    super::arena::box_into_active(CfmlValue::string(escaped)) as i64
}

// ── v0.99.2 multi-arg Boxed shims ────────────────────────────────────────────
// left / right / mid / repeatString — accept a leading Boxed string and one
// or two Int counts. find / findNoCase return Int from two Boxed strings.
// replace / replaceNoCase 3-arg forms (scope defaults to "one"). All
// infallible — argument coercion via `.max(0)` / `.saturating_sub`.

/// Mirrors `fn_left(string, count)`.
extern "C" fn cfml_left_boxed_int(tagged: i64, count: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let n = count.max(0) as usize;
    let chars: Vec<char> = s.chars().collect();
    let out: String = chars[..n.min(chars.len())].iter().collect();
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_right(string, count)`.
extern "C" fn cfml_right_boxed_int(tagged: i64, count: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let n = count.max(0) as usize;
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    let out: String = chars[start..].iter().collect();
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_mid(string, start, length)` (3-arg form).
extern "C" fn cfml_mid_boxed_int_int(tagged: i64, start: i64, length: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let start = (start.max(1) as usize).saturating_sub(1);
    let length = length.max(0) as usize;
    let chars: Vec<char> = s.chars().collect();
    let out: String = if start >= chars.len() {
        String::new()
    } else {
        let end = (start + length).min(chars.len());
        chars[start..end].iter().collect()
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_repeat_string(string, count)`.
extern "C" fn cfml_repeat_string_boxed_int(tagged: i64, count: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    let n = count.max(0) as usize;
    super::arena::box_into_active(CfmlValue::string(s.repeat(n))) as i64
}

/// Mirrors `fn_find(substring, string)` (2-arg form). 1-based index of the
/// first occurrence, or 0 if not found.
extern "C" fn cfml_find_boxed_boxed_i64(needle: i64, hay: i64) -> i64 {
    let n = unsafe { super::boxed::materialize_tagged(needle as usize) };
    let h = unsafe { super::boxed::materialize_tagged(hay as usize) };
    let substring = n.as_string();
    let string = h.as_string();
    if let Some(pos) = string.find(&*substring) {
        (pos + 1) as i64
    } else {
        0
    }
}

/// Mirrors `fn_find_no_case(substring, string)`.
extern "C" fn cfml_find_no_case_boxed_boxed_i64(needle: i64, hay: i64) -> i64 {
    let n = unsafe { super::boxed::materialize_tagged(needle as usize) };
    let h = unsafe { super::boxed::materialize_tagged(hay as usize) };
    let substring = n.as_string().to_lowercase();
    let string = h.as_string().to_lowercase();
    if let Some(pos) = string.find(&substring) {
        (pos + 1) as i64
    } else {
        0
    }
}

/// Mirrors `fn_replace(string, find, replaceWith)` with default scope="one"
/// (single replacement, case-sensitive).
extern "C" fn cfml_replace_3_boxed(tag_s: i64, tag_f: i64, tag_r: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let vs = unsafe { super::boxed::materialize_tagged(tag_s as usize) };
    let vf = unsafe { super::boxed::materialize_tagged(tag_f as usize) };
    let vr = unsafe { super::boxed::materialize_tagged(tag_r as usize) };
    let out = vs.as_string().replacen(&*vf.as_string(), &vr.as_string(), 1);
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

// ── v0.99.3 — fallible shim (uses the new bail plumbing) ────────────────────
// `arrayLen(QueryColumn)` errors in the interpreter (Lucee@7 parity); the JIT
// can't constant-fold this away because the underlying kind of a Boxed arg is
// only known at runtime. Pattern: shim takes a trailing `*mut i64`; on error
// it writes `*bail = 1` and returns 0. The translator emits a post-call
// `brif bail, bail_block, cont` so execution falls through to the interpreter
// re-run path on bail.
//
// Mirrors `fn_array_len`: Array → len, Struct (arguments-scope shape) →
// count-of-numeric-keys, anything else → 0, **QueryColumn → bail to
// interpreter so it throws the same `Can't cast` runtime error.**
extern "C" fn cfml_array_len_boxed_i64(tagged: i64, bail: *mut i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    match &v {
        CfmlValue::Array(a) => a.len() as i64,
        CfmlValue::QueryColumn(..) => {
            // Surface as a bail; interpreter throws on re-run.
            unsafe {
                *bail = 1;
            }
            0
        }
        CfmlValue::Struct(s) => {
            // Arguments-scope shape: count entries with numeric keys (1-based
            // positional args). Plain struct with no numeric keys returns 0
            // (same as interpreter).
            s.keys()
                .into_iter()
                .filter(|k| k.parse::<usize>().is_ok())
                .count() as i64
        }
        _ => 0,
    }
}

// ── v0.99.3 infallible struct shims (no bail) ──────────────────────────────

/// Mirrors `fn_struct_key_list(struct)` with default delimiter `,`. Returns
/// `""` for non-struct inputs (infallible — same as interpreter).
extern "C" fn cfml_struct_key_list_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let out = if let CfmlValue::Struct(s) = &v {
        // Inline `visible_struct_keys`: hide the arguments-scope markers.
        // `all_keys()` includes shared-table methods (component flyweight), as
        // this path did when methods lived in the per-instance map.
        let keys: Vec<String> = s.all_keys();
        let visible: Vec<String> = if keys.iter().any(|k| k == "__arguments_scope") {
            keys.into_iter()
                .filter(|k| k != "__arguments_scope" && k != "__arguments_params")
                .collect()
        } else {
            keys
        };
        visible.join(",")
    } else {
        String::new()
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_replace_no_case(string, find, replaceWith)` with default scope="one".
extern "C" fn cfml_replace_no_case_3_boxed(tag_s: i64, tag_f: i64, tag_r: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let vs = unsafe { super::boxed::materialize_tagged(tag_s as usize) };
    let vf = unsafe { super::boxed::materialize_tagged(tag_f as usize) };
    let vr = unsafe { super::boxed::materialize_tagged(tag_r as usize) };
    let string = vs.as_string();
    let find = vf.as_string();
    let replace_with = vr.as_string();
    let find_lower = find.to_lowercase();
    let lower = string.to_lowercase();
    let out = if let Some(pos) = lower.find(&find_lower) {
        let mut result = String::new();
        result.push_str(&string[..pos]);
        result.push_str(&replace_with);
        result.push_str(&string[pos + find.len()..]);
        result
    } else {
        string.to_string()
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

// ── v0.101.0 — type-predicate + collection-predicate Boxed shims ────────────
// All mirror their `cfml-stdlib::builtins::fn_is_*` / `fn_*_is_empty` /
// `fn_*_count` / `fn_*_key_exists` / `fn_*_contains` counterparts bit-for-bit.
// Bool-returning shims wrap `CfmlValue::Bool(b)` into the active arena and
// return it as Boxed — `Kind::Bool` can't escape the stack to a local/return,
// so we go through Boxed to preserve interp semantics (Bool vs Int(1) differ
// in stringification: "YES"/"NO"/"true"/"false" vs "1"/"0").
//
// IMPORTANT: every shim materialises its Boxed input via
// `materialize_tagged`, NOT `borrow_tagged`. Since v0.99.6 the member-IC
// can return SMI Int inline (low-bit-tagged i61), and `borrow_tagged`
// panics on non-TAG_PTR. `materialize_tagged` synthesises `CfmlValue::Int`
// for SMI and Arc-clones the pointee for heap tags (cheap for String /
// Array / Struct, fine elsewhere). Owned `CfmlValue` lives the body's
// lifetime — drops on return.

/// Mirrors `fn_is_numeric`: Int/Double → true; String → parses as f64
/// (after trim); everything else (incl. Bool) → false. A CFML boolean is NOT
/// numeric (Lucee/ACF/BoxLang parity).
extern "C" fn cfml_is_numeric_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let b = match &v {
        CfmlValue::Int(_) | CfmlValue::Double(_) => true,
        CfmlValue::String(s) => s.trim().parse::<f64>().is_ok(),
        _ => false,
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_is_array`: `CfmlValue::Array(_)` plus the hybrid `arguments`
/// scope (a Struct carrying the `__arguments_scope` marker). QueryColumn
/// excluded (Lucee@7 parity).
extern "C" fn cfml_is_array_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let is = match &v {
        CfmlValue::Array(_) => true,
        CfmlValue::Struct(s) => s.contains_key("__arguments_scope"),
        _ => false,
    };
    super::arena::box_into_active(CfmlValue::Bool(is)) as i64
}

/// Mirrors `fn_is_struct`.
extern "C" fn cfml_is_struct_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::Bool(matches!(v, CfmlValue::Struct(_)))) as i64
}

/// Mirrors `fn_is_boolean`: Bool/Int/Double → true; String that parses as f64
/// or matches {true,false,yes,no} (case-insensitive, trimmed) → true; else false.
extern "C" fn cfml_is_boolean_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let b = match &v {
        CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) => true,
        CfmlValue::String(s) => {
            let trimmed = s.trim();
            let lower = trimmed.to_lowercase();
            matches!(lower.as_str(), "true" | "false" | "yes" | "no")
                || trimmed.parse::<f64>().is_ok()
        }
        _ => false,
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_is_simple_value`: Bool/Int/Double/String → true; else false.
extern "C" fn cfml_is_simple_value_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let b = matches!(
        &v,
        CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::String(_)
    );
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_is_null`: true only for `CfmlValue::Null`. A Boxed argument
/// is always present at the call site (the ABI carries a tag), so the
/// interpreter's "missing arg" branch is unreachable from JIT'd code.
/// SMI-tagged inputs are guaranteed non-Null (SMI encodes Int only), so
/// the materialised value's Null match is precise.
extern "C" fn cfml_is_null_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    super::arena::box_into_active(CfmlValue::Bool(matches!(v, CfmlValue::Null))) as i64
}

/// Mirrors `fn_array_is_empty`: empty if Array && len==0; non-Array → true
/// (matches `_ => Bool(true)` interp fallback).
extern "C" fn cfml_array_is_empty_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let b = match &v {
        CfmlValue::Array(a) => a.is_empty(),
        _ => true,
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_struct_is_empty`.
extern "C" fn cfml_struct_is_empty_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let b = match &v {
        CfmlValue::Struct(s) => s.is_empty(),
        _ => true,
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_struct_count`: count of *visible* keys (hides the
/// `__arguments_scope` / `__arguments_params` markers used internally for
/// the arguments scope). Non-Struct → 0.
extern "C" fn cfml_struct_count_boxed_i64(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    if let CfmlValue::Struct(s) = &v {
        let keys: Vec<String> = s.keys();
        let n = if keys.iter().any(|k| k == "__arguments_scope") {
            keys.iter()
                .filter(|k| k.as_str() != "__arguments_scope" && k.as_str() != "__arguments_params")
                .count()
        } else {
            keys.len()
        };
        n as i64
    } else {
        0
    }
}

/// Mirrors `fn_list_len(list)` with default delimiter `,`. Empty string → 0.
/// Stringifies non-string args via `as_string()` (interp does the same via
/// `get_str`).
extern "C" fn cfml_list_len_boxed_i64(tagged: i64) -> i64 {
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let s = v.as_string();
    if s.is_empty() {
        return 0;
    }
    // CFML list split: each char in `delimiters` is a separate delimiter;
    // empty items dropped. Default delimiter is a single comma.
    s.split(',').filter(|p| !p.is_empty()).count() as i64
}

/// Mirrors `fn_array_to_list(array)` with default delimiter `,`. Non-Array →
/// empty string.
extern "C" fn cfml_array_to_list_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let out = if let CfmlValue::Array(a) = &v {
        let items: Vec<String> = a.iter().map(|x| x.as_string()).collect();
        items.join(",")
    } else {
        String::new()
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_struct_key_exists(struct, key)`: case-insensitive presence
/// check that hides the internal `__arguments_*` markers. Non-Struct receiver
/// → false.
extern "C" fn cfml_struct_key_exists_boxed_boxed(tag_s: i64, tag_k: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let vs = unsafe { super::boxed::materialize_tagged(tag_s as usize) };
    let vk = unsafe { super::boxed::materialize_tagged(tag_k as usize) };
    let b = if let CfmlValue::Struct(s) = &vs {
        let key = vk.as_string();
        let lower = key.to_lowercase();
        let keys: Vec<String> = s.keys();
        let found_key = if keys.iter().any(|k| k == &key) {
            Some(key.clone())
        } else {
            keys.into_iter().find(|k| k.to_lowercase() == lower)
        };
        match found_key {
            Some(k)
                if (k == "__arguments_scope" || k == "__arguments_params")
                    && s.contains_key("__arguments_scope") =>
            {
                false
            }
            Some(_) => true,
            None => false,
        }
    } else {
        false
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_array_contains(array, value)`: case-sensitive stringified
/// membership. Non-Array → false.
extern "C" fn cfml_array_contains_boxed_boxed(tag_a: i64, tag_v: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let va = unsafe { super::boxed::materialize_tagged(tag_a as usize) };
    let vv = unsafe { super::boxed::materialize_tagged(tag_v as usize) };
    let b = if let CfmlValue::Array(arr) = &va {
        let needle = vv.as_string();
        arr.iter().any(|x| x.as_string() == needle)
    } else {
        false
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

/// Mirrors `fn_array_contains_no_case(array, value)`.
extern "C" fn cfml_array_contains_no_case_boxed_boxed(tag_a: i64, tag_v: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let va = unsafe { super::boxed::materialize_tagged(tag_a as usize) };
    let vv = unsafe { super::boxed::materialize_tagged(tag_v as usize) };
    let b = if let CfmlValue::Array(arr) = &va {
        let needle = vv.as_string().to_lowercase();
        arr.iter().any(|x| x.as_string().to_lowercase() == needle)
    } else {
        false
    };
    super::arena::box_into_active(CfmlValue::Bool(b)) as i64
}

// ── v0.103.0 — Boxed-aware array + list shims ────────────────────────────────
// arrayFirst/arrayLast THROW on non-array (see cfml-stdlib::fn_array_first /
// fn_array_last → `Err(...)`); the JIT bails so the interpreter re-runs the
// function and surfaces the same runtime error. arraySum/arrayAvg are
// infallible (Int(0) sentinel for non-array / empty, Double otherwise). The
// list-family shims are all infallible string-in/string-out with default
// delimiter `,`.

/// Mirrors `fn_array_first`: Array → first element (or Null if empty); else
/// BAIL to interpreter for the `argument must be an array` throw.
extern "C" fn cfml_array_first_boxed(tagged: i64, bail: *mut i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    if let CfmlValue::Array(arr) = &v {
        let first = arr.first().unwrap_or(CfmlValue::Null);
        super::arena::box_into_active(first) as i64
    } else {
        unsafe {
            *bail = 1;
        }
        0
    }
}

/// Mirrors `fn_array_last`: Array → last element (or Null if empty); else
/// BAIL to interpreter.
extern "C" fn cfml_array_last_boxed(tagged: i64, bail: *mut i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    if let CfmlValue::Array(arr) = &v {
        let last = arr.last().unwrap_or(CfmlValue::Null);
        super::arena::box_into_active(last) as i64
    } else {
        unsafe {
            *bail = 1;
        }
        0
    }
}

/// Mirrors `fn_array_sum`: Array → sum-of-as_string-as-f64 (Double); else 0.
extern "C" fn cfml_array_sum_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let out = if let CfmlValue::Array(arr) = &v {
        // get_float coerces via query_column_scalar then parses; for plain
        // Int/Double/String entries this matches `as_string().parse::<f64>()`
        // with 0.0 fallback. Stay bit-exact with the interp helper by going
        // through `as_string()` (covers QueryColumn proxies via their
        // stringify).
        let sum: f64 = arr
            .iter()
            .map(|x| match &x {
                CfmlValue::Int(i) => *i as f64,
                CfmlValue::Double(d) => *d,
                _ => x.as_string().parse::<f64>().unwrap_or(0.0),
            })
            .sum();
        CfmlValue::Double(sum)
    } else {
        CfmlValue::Int(0)
    };
    super::arena::box_into_active(out) as i64
}

/// Mirrors `fn_array_avg`: Array && non-empty → sum/len (Double); else 0.
extern "C" fn cfml_array_avg_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let out = if let CfmlValue::Array(arr) = &v {
        if arr.is_empty() {
            CfmlValue::Int(0)
        } else {
            let n = arr.len() as f64;
            let sum: f64 = arr
                .iter()
                .map(|x| match &x {
                    CfmlValue::Int(i) => *i as f64,
                    CfmlValue::Double(d) => *d,
                    _ => x.as_string().parse::<f64>().unwrap_or(0.0),
                })
                .sum();
            CfmlValue::Double(sum / n)
        }
    } else {
        CfmlValue::Int(0)
    };
    super::arena::box_into_active(out) as i64
}

// Inline copy of `cfml-stdlib::cfml_list_split` to avoid a cross-crate `pub`
// leak: each delimiter character is its own delimiter; empty items dropped.
fn list_split<'a>(list: &'a str, delimiters: &str) -> Vec<&'a str> {
    if list.is_empty() {
        return Vec::new();
    }
    list.split(|c: char| delimiters.contains(c))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Mirrors `fn_list_first` with default delimiter `,`.
extern "C" fn cfml_list_first_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let list = v.as_string();
    let items = list_split(&list, ",");
    let first = items.first().copied().unwrap_or("").to_string();
    super::arena::box_into_active(CfmlValue::string(first)) as i64
}

/// Mirrors `fn_list_last` with default delimiter `,`.
extern "C" fn cfml_list_last_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let list = v.as_string();
    let items = list_split(&list, ",");
    let last = items.last().copied().unwrap_or("").to_string();
    super::arena::box_into_active(CfmlValue::string(last)) as i64
}

/// Mirrors `fn_list_rest` (default delimiter `,`): literal substring from
/// the start of element 2 to end, preserving interior/trailing empties +
/// original delimiter chars. Empty-collapsing only over the leading run +
/// the run that ends element 1.
extern "C" fn cfml_list_rest_boxed(tagged: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let list = v.as_string();
    let is_delim = |c: char| c == ',';
    let mut iter = list.char_indices().peekable();
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            iter.next();
        } else {
            break;
        }
    }
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            break;
        }
        iter.next();
    }
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            iter.next();
        } else {
            break;
        }
    }
    let rest = match iter.peek() {
        Some(&(i, _)) => &list[i..],
        None => "",
    };
    super::arena::box_into_active(CfmlValue::string(rest.to_string())) as i64
}

/// Mirrors `fn_list_get_at(list, index)` with default delimiter `,`. Index is
/// 1-based; saturating_sub(1) clamps 0 → 0. Out-of-range → empty string.
extern "C" fn cfml_list_get_at_boxed_i64(tagged: i64, index: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let v = unsafe { super::boxed::materialize_tagged(tagged as usize) };
    let list = v.as_string();
    let idx = (index as usize).saturating_sub(1);
    let items = list_split(&list, ",");
    let out = items.get(idx).copied().unwrap_or("").to_string();
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_list_append(list, value)` with default delimiter `,`. Empty
/// list short-circuits to bare value (no leading delim).
extern "C" fn cfml_list_append_boxed_boxed(tag_l: i64, tag_v: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let vl = unsafe { super::boxed::materialize_tagged(tag_l as usize) };
    let vv = unsafe { super::boxed::materialize_tagged(tag_v as usize) };
    let list = vl.as_string();
    let value = vv.as_string();
    let out = if list.is_empty() {
        value
    } else {
        format!("{},{}", list, value)
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// Mirrors `fn_list_prepend(list, value)` with default delimiter `,`.
extern "C" fn cfml_list_prepend_boxed_boxed(tag_l: i64, tag_v: i64) -> i64 {
    use cfml_common::dynamic::CfmlValue;
    let vl = unsafe { super::boxed::materialize_tagged(tag_l as usize) };
    let vv = unsafe { super::boxed::materialize_tagged(tag_v as usize) };
    let list = vl.as_string();
    let value = vv.as_string();
    let out = if list.is_empty() {
        value
    } else {
        format!("{},{}", value, list)
    };
    super::arena::box_into_active(CfmlValue::string(out)) as i64
}

/// The complete shim table. Order matters for `lookup_overload`: more specific
/// signatures (e.g. `abs(Int)`) must precede broader ones (`abs(Numeric)`).
pub static SHIMS: &[Shim] = &[
    Shim {
        name: "abs",
        args_req: &[KindReq::Int],
        args_abi: &[Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_abs_i64",
        addr: cfml_abs_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "abs",
        args_req: &[KindReq::Float],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_abs_f64",
        addr: cfml_abs_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "min",
        args_req: &[KindReq::Numeric, KindReq::Numeric],
        args_abi: &[Kind::Float, Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_min_f64",
        addr: cfml_min_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "max",
        args_req: &[KindReq::Numeric, KindReq::Numeric],
        args_abi: &[Kind::Float, Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_max_f64",
        addr: cfml_max_f64 as *const u8,
        bailable: false,
    },
    // ── Single-arg numeric → Int (rounding / sign / truncation) ──────────
    Shim {
        name: "floor",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Int,
        sym: "cfml_floor_i64",
        addr: cfml_floor_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "ceiling",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Int,
        sym: "cfml_ceiling_i64",
        addr: cfml_ceiling_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "round",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Int,
        sym: "cfml_round_i64",
        addr: cfml_round_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "sgn",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Int,
        sym: "cfml_sgn_i64",
        addr: cfml_sgn_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "fix",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Int,
        sym: "cfml_fix_i64",
        addr: cfml_fix_i64 as *const u8,
        bailable: false,
    },
    // ── Single-arg numeric → Float (transcendentals) ─────────────────────
    Shim {
        name: "sqr",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_sqr_f64",
        addr: cfml_sqr_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "exp",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_exp_f64",
        addr: cfml_exp_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "log",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_log_f64",
        addr: cfml_log_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "log10",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_log10_f64",
        addr: cfml_log10_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "sin",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_sin_f64",
        addr: cfml_sin_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "cos",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_cos_f64",
        addr: cfml_cos_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "tan",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_tan_f64",
        addr: cfml_tan_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "asin",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_asin_f64",
        addr: cfml_asin_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "acos",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_acos_f64",
        addr: cfml_acos_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "atan",
        args_req: &[KindReq::Numeric],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_atan_f64",
        addr: cfml_atan_f64 as *const u8,
        bailable: false,
    },
    // ── Bit-twiddling: 1 / 2-arg Int → Int ───────────────────────────────
    Shim {
        name: "bitand",
        args_req: &[KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_and_i64",
        addr: cfml_bit_and_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitor",
        args_req: &[KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_or_i64",
        addr: cfml_bit_or_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitxor",
        args_req: &[KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_xor_i64",
        addr: cfml_bit_xor_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitnot",
        args_req: &[KindReq::Int],
        args_abi: &[Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_not_i64",
        addr: cfml_bit_not_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitshln",
        args_req: &[KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_shln_i64",
        addr: cfml_bit_shln_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitshrn",
        args_req: &[KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_shrn_i64",
        addr: cfml_bit_shrn_i64 as *const u8,
        bailable: false,
    },
    // ── incrementValue / decrementValue — typed overloads ────────────────
    Shim {
        name: "incrementvalue",
        args_req: &[KindReq::Int],
        args_abi: &[Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_increment_value_i64",
        addr: cfml_increment_value_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "incrementvalue",
        args_req: &[KindReq::Float],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_increment_value_f64",
        addr: cfml_increment_value_f64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "decrementvalue",
        args_req: &[KindReq::Int],
        args_abi: &[Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_decrement_value_i64",
        addr: cfml_decrement_value_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "decrementvalue",
        args_req: &[KindReq::Float],
        args_abi: &[Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_decrement_value_f64",
        addr: cfml_decrement_value_f64 as *const u8,
        bailable: false,
    },
    // ── bitMaskRead / bitMaskSet / bitMaskClear — Int → Int ──────────────
    Shim {
        name: "bitmaskread",
        args_req: &[KindReq::Int, KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_mask_read_i64",
        addr: cfml_bit_mask_read_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitmaskset",
        args_req: &[KindReq::Int, KindReq::Int, KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int, Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_mask_set_i64",
        addr: cfml_bit_mask_set_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "bitmaskclear",
        args_req: &[KindReq::Int, KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Int, Kind::Int, Kind::Int],
        ret_kind: Kind::Int,
        sym: "cfml_bit_mask_clear_i64",
        addr: cfml_bit_mask_clear_i64 as *const u8,
        bailable: false,
    },
    // ── 2-arg pow() builtin (function-call form of the `^` infix) ────────
    Shim {
        name: "pow",
        args_req: &[KindReq::Numeric, KindReq::Numeric],
        args_abi: &[Kind::Float, Kind::Float],
        ret_kind: Kind::Float,
        sym: "cfml_pow_fn_f64",
        addr: cfml_pow_fn_f64 as *const u8,
        bailable: false,
    },
    // ── v0.92.0 — Boxed-argument string/array shims ──────────────────────
    Shim {
        name: "len",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_len_boxed_i64",
        addr: cfml_len_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "ucase",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_ucase_boxed",
        addr: cfml_ucase_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "lcase",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_lcase_boxed",
        addr: cfml_lcase_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "trim",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_trim_boxed",
        addr: cfml_trim_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "ltrim",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_ltrim_boxed",
        addr: cfml_ltrim_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "rtrim",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_rtrim_boxed",
        addr: cfml_rtrim_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "reverse",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_reverse_boxed",
        addr: cfml_reverse_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "asc",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_asc_boxed_i64",
        addr: cfml_asc_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "stripcr",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_strip_cr_boxed",
        addr: cfml_strip_cr_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "htmleditformat",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_html_edit_format_boxed",
        addr: cfml_html_edit_format_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "htmlcodeformat",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_html_code_format_boxed",
        addr: cfml_html_code_format_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "encodeforhtml",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_encode_for_html_boxed",
        addr: cfml_encode_for_html_boxed as *const u8,
        bailable: false,
    },
    // ── v0.99.2 — more single-arg Boxed→Boxed string shims ───────────────
    Shim {
        name: "urlencodedformat",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_url_encoded_format_boxed",
        addr: cfml_url_encoded_format_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "urldecode",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_url_decode_boxed",
        addr: cfml_url_decode_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "jsstringformat",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_js_string_format_boxed",
        addr: cfml_js_string_format_boxed as *const u8,
        bailable: false,
    },
    // ── v0.99.2 — multi-arg Boxed shims ──────────────────────────────────
    Shim {
        name: "left",
        args_req: &[KindReq::Boxed, KindReq::Int],
        args_abi: &[Kind::Boxed, Kind::Int],
        ret_kind: Kind::Boxed,
        sym: "cfml_left_boxed_int",
        addr: cfml_left_boxed_int as *const u8,
        bailable: false,
    },
    Shim {
        name: "right",
        args_req: &[KindReq::Boxed, KindReq::Int],
        args_abi: &[Kind::Boxed, Kind::Int],
        ret_kind: Kind::Boxed,
        sym: "cfml_right_boxed_int",
        addr: cfml_right_boxed_int as *const u8,
        bailable: false,
    },
    Shim {
        name: "mid",
        args_req: &[KindReq::Boxed, KindReq::Int, KindReq::Int],
        args_abi: &[Kind::Boxed, Kind::Int, Kind::Int],
        ret_kind: Kind::Boxed,
        sym: "cfml_mid_boxed_int_int",
        addr: cfml_mid_boxed_int_int as *const u8,
        bailable: false,
    },
    Shim {
        name: "repeatstring",
        args_req: &[KindReq::Boxed, KindReq::Int],
        args_abi: &[Kind::Boxed, Kind::Int],
        ret_kind: Kind::Boxed,
        sym: "cfml_repeat_string_boxed_int",
        addr: cfml_repeat_string_boxed_int as *const u8,
        bailable: false,
    },
    Shim {
        name: "find",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_find_boxed_boxed_i64",
        addr: cfml_find_boxed_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "findnocase",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_find_no_case_boxed_boxed_i64",
        addr: cfml_find_no_case_boxed_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "replace",
        args_req: &[KindReq::Boxed, KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_replace_3_boxed",
        addr: cfml_replace_3_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "replacenocase",
        args_req: &[KindReq::Boxed, KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_replace_no_case_3_boxed",
        addr: cfml_replace_no_case_3_boxed as *const u8,
        bailable: false,
    },
    // ── v0.99.3 — fallible builtin shim (uses new bail plumbing) ─────────
    Shim {
        name: "arraylen",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_array_len_boxed_i64",
        addr: cfml_array_len_boxed_i64 as *const u8,
        bailable: true,
    },
    // ── v0.99.3 — infallible struct introspection shim ───────────────────
    Shim {
        name: "structkeylist",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_struct_key_list_boxed",
        addr: cfml_struct_key_list_boxed as *const u8,
        bailable: false,
    },
    // ── v0.101.0 — type/collection predicates + collection introspection ─
    Shim {
        name: "isnumeric",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_numeric_boxed",
        addr: cfml_is_numeric_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "isarray",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_array_boxed",
        addr: cfml_is_array_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "isstruct",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_struct_boxed",
        addr: cfml_is_struct_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "isboolean",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_boolean_boxed",
        addr: cfml_is_boolean_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "issimplevalue",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_simple_value_boxed",
        addr: cfml_is_simple_value_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "isnull",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_is_null_boxed",
        addr: cfml_is_null_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "arrayisempty",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_is_empty_boxed",
        addr: cfml_array_is_empty_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "structisempty",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_struct_is_empty_boxed",
        addr: cfml_struct_is_empty_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "structcount",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_struct_count_boxed_i64",
        addr: cfml_struct_count_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "listlen",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Int,
        sym: "cfml_list_len_boxed_i64",
        addr: cfml_list_len_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "arraytolist",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_to_list_boxed",
        addr: cfml_array_to_list_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "structkeyexists",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_struct_key_exists_boxed_boxed",
        addr: cfml_struct_key_exists_boxed_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "arraycontains",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_contains_boxed_boxed",
        addr: cfml_array_contains_boxed_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "arraycontainsnocase",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_contains_no_case_boxed_boxed",
        addr: cfml_array_contains_no_case_boxed_boxed as *const u8,
        bailable: false,
    },
    // ── v0.103.0 — Boxed-aware array + list shims ────────────────────────
    Shim {
        name: "arrayfirst",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_first_boxed",
        addr: cfml_array_first_boxed as *const u8,
        bailable: true,
    },
    Shim {
        name: "arraylast",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_last_boxed",
        addr: cfml_array_last_boxed as *const u8,
        bailable: true,
    },
    Shim {
        name: "arraysum",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_sum_boxed",
        addr: cfml_array_sum_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "arrayavg",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_array_avg_boxed",
        addr: cfml_array_avg_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "listfirst",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_first_boxed",
        addr: cfml_list_first_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "listlast",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_last_boxed",
        addr: cfml_list_last_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "listrest",
        args_req: &[KindReq::Boxed],
        args_abi: &[Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_rest_boxed",
        addr: cfml_list_rest_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "listgetat",
        args_req: &[KindReq::Boxed, KindReq::Int],
        args_abi: &[Kind::Boxed, Kind::Int],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_get_at_boxed_i64",
        addr: cfml_list_get_at_boxed_i64 as *const u8,
        bailable: false,
    },
    Shim {
        name: "listappend",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_append_boxed_boxed",
        addr: cfml_list_append_boxed_boxed as *const u8,
        bailable: false,
    },
    Shim {
        name: "listprepend",
        args_req: &[KindReq::Boxed, KindReq::Boxed],
        args_abi: &[Kind::Boxed, Kind::Boxed],
        ret_kind: Kind::Boxed,
        sym: "cfml_list_prepend_boxed_boxed",
        addr: cfml_list_prepend_boxed_boxed as *const u8,
        bailable: false,
    },
];

/// Lowercased lookup: `true` iff some shim has this exact name. Currently only
/// used by tests in this file — the production path goes through
/// [`canonical_name`] so the analyser also gets the `&'static str` interner.
#[cfg(test)]
pub fn name_is_known(name: &str) -> bool {
    canonical_name(name).is_some()
}

/// Returns the canonical `&'static str` for `name` (case-insensitive lookup
/// into [`SHIMS`]), or `None` if no overload matches. The returned slice is the
/// shim table's own name field, so it doubles as a stable interned identifier
/// that callers can stash in [`super::analysis::Kind::Builtin`].
pub fn canonical_name(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    SHIMS.iter().find(|s| s.name == lower).map(|s| s.name)
}

/// Resolve a `(name, arg_kinds)` to the matching shim index in [`SHIMS`].
///
/// Returns `None` when no overload of `name` accepts these exact kinds. Walks
/// the table in declaration order, so put more specific overloads first.
pub fn lookup_overload(name: &str, arg_kinds: &[Kind]) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    SHIMS.iter().enumerate().find_map(|(i, s)| {
        if s.name != lower {
            return None;
        }
        if s.args_req.len() != arg_kinds.len() {
            return None;
        }
        if s.args_req.iter().zip(arg_kinds.iter()).all(|(req, k)| req.matches(*k)) {
            Some(i)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_lookup_is_case_insensitive() {
        assert!(name_is_known("abs"));
        assert!(name_is_known("ABS"));
        assert!(name_is_known("AbS"));
        assert!(!name_is_known("nope"));
    }

    #[test]
    fn overload_prefers_specific_int() {
        // abs(Int) → Int (not promoted to Float)
        let idx = lookup_overload("abs", &[Kind::Int]).unwrap();
        assert_eq!(SHIMS[idx].ret_kind, Kind::Int);
    }

    #[test]
    fn overload_picks_float_when_arg_is_float() {
        let idx = lookup_overload("abs", &[Kind::Float]).unwrap();
        assert_eq!(SHIMS[idx].ret_kind, Kind::Float);
    }

    #[test]
    fn min_accepts_any_numeric_combo() {
        for kinds in [
            [Kind::Int, Kind::Int],
            [Kind::Int, Kind::Float],
            [Kind::Float, Kind::Int],
            [Kind::Float, Kind::Float],
        ] {
            let idx = lookup_overload("min", &kinds).expect("min should accept any numeric combo");
            assert_eq!(SHIMS[idx].ret_kind, Kind::Float);
        }
    }

    #[test]
    fn unknown_name_or_arity_rejects() {
        assert!(lookup_overload("nope", &[Kind::Int]).is_none());
        assert!(lookup_overload("abs", &[Kind::Int, Kind::Int]).is_none());
        assert!(lookup_overload("min", &[Kind::Int]).is_none());
    }

    #[test]
    fn boxed_overloads_match_only_boxed_args() {
        use super::super::analysis::Kind;
        // Single-arg Boxed shims: must accept exactly Boxed; Int/Float must
        // miss. Covers the v0.99.0 / v0.99.1 / v0.99.2 surface.
        for name in [
            "len",
            "ucase",
            "lcase",
            "trim",
            "ltrim",
            "rtrim",
            "reverse",
            "asc",
            "stripcr",
            "htmleditformat",
            "htmlcodeformat",
            "encodeforhtml",
            "urlencodedformat",
            "urldecode",
            "jsstringformat",
        ] {
            assert!(
                lookup_overload(name, &[Kind::Boxed]).is_some(),
                "{name}(Boxed) must match"
            );
            assert!(
                lookup_overload(name, &[Kind::Int]).is_none(),
                "{name}(Int) must not match"
            );
            assert!(
                lookup_overload(name, &[Kind::Float]).is_none(),
                "{name}(Float) must not match"
            );
        }
        // len + asc return Int; the case/trim/format family return Boxed.
        for name in ["len", "asc"] {
            let idx = lookup_overload(name, &[Kind::Boxed]).unwrap();
            assert_eq!(SHIMS[idx].ret_kind, Kind::Int, "{name} ret must be Int");
        }
        for name in [
            "ucase",
            "lcase",
            "trim",
            "ltrim",
            "rtrim",
            "reverse",
            "stripcr",
            "htmleditformat",
            "htmlcodeformat",
            "encodeforhtml",
            "urlencodedformat",
            "urldecode",
            "jsstringformat",
        ] {
            let idx = lookup_overload(name, &[Kind::Boxed]).unwrap();
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed, "{name} ret must be Boxed");
        }

        // Multi-arg Boxed shims (v0.99.2).
        // (Boxed, Int) → Boxed: left, right, repeatString.
        for name in ["left", "right", "repeatstring"] {
            assert!(
                lookup_overload(name, &[Kind::Boxed, Kind::Int]).is_some(),
                "{name}(Boxed, Int) must match"
            );
            assert!(
                lookup_overload(name, &[Kind::Int, Kind::Int]).is_none(),
                "{name}(Int, Int) must not match"
            );
            let idx = lookup_overload(name, &[Kind::Boxed, Kind::Int]).unwrap();
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
        }
        // (Boxed, Int, Int) → Boxed: mid.
        {
            let idx = lookup_overload("mid", &[Kind::Boxed, Kind::Int, Kind::Int])
                .expect("mid(Boxed,Int,Int) must match");
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
            assert!(lookup_overload("mid", &[Kind::Boxed, Kind::Int]).is_none());
        }
        // (Boxed, Boxed) → Int: find, findNoCase.
        for name in ["find", "findnocase"] {
            let idx = lookup_overload(name, &[Kind::Boxed, Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed,Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Int);
            assert!(lookup_overload(name, &[Kind::Int, Kind::Int]).is_none());
        }
        // (Boxed, Boxed, Boxed) → Boxed: replace, replaceNoCase.
        for name in ["replace", "replacenocase"] {
            let idx = lookup_overload(name, &[Kind::Boxed, Kind::Boxed, Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed,Boxed,Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
            assert!(lookup_overload(name, &[Kind::Boxed, Kind::Boxed]).is_none());
        }

        // v0.101.0 — type/collection predicates return Boxed (CfmlValue::Bool).
        for name in [
            "isnumeric",
            "isarray",
            "isstruct",
            "isboolean",
            "issimplevalue",
            "isnull",
            "arrayisempty",
            "structisempty",
            "arraytolist",
        ] {
            let idx = lookup_overload(name, &[Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed, "{name} ret must be Boxed");
            assert!(lookup_overload(name, &[Kind::Int]).is_none());
        }
        // v0.101.0 — Int-returning collection introspection: structCount/listLen.
        for name in ["structcount", "listlen"] {
            let idx = lookup_overload(name, &[Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Int, "{name} ret must be Int");
        }
        // v0.101.0 — 2-arg (Boxed, Boxed) → Boxed predicates.
        for name in ["structkeyexists", "arraycontains", "arraycontainsnocase"] {
            let idx = lookup_overload(name, &[Kind::Boxed, Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed,Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
            assert!(lookup_overload(name, &[Kind::Boxed]).is_none());
        }

        // v0.103.0 — array shims: 1-arg (Boxed) → Boxed; arrayFirst/Last are
        // bailable, arraySum/Avg are infallible.
        for name in ["arrayfirst", "arraylast", "arraysum", "arrayavg"] {
            let idx = lookup_overload(name, &[Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
            assert!(lookup_overload(name, &[Kind::Int]).is_none());
        }
        assert!(SHIMS[lookup_overload("arrayfirst", &[Kind::Boxed]).unwrap()].bailable);
        assert!(SHIMS[lookup_overload("arraylast", &[Kind::Boxed]).unwrap()].bailable);
        assert!(!SHIMS[lookup_overload("arraysum", &[Kind::Boxed]).unwrap()].bailable);
        assert!(!SHIMS[lookup_overload("arrayavg", &[Kind::Boxed]).unwrap()].bailable);

        // v0.103.0 — list shims: 1-arg (Boxed) → Boxed (listFirst/Last/Rest).
        for name in ["listfirst", "listlast", "listrest"] {
            let idx = lookup_overload(name, &[Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
        }
        // listGetAt(Boxed, Int) → Boxed.
        let idx = lookup_overload("listgetat", &[Kind::Boxed, Kind::Int])
            .expect("listgetat(Boxed,Int) must match");
        assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
        assert!(lookup_overload("listgetat", &[Kind::Boxed]).is_none());
        // listAppend / listPrepend (Boxed, Boxed) → Boxed.
        for name in ["listappend", "listprepend"] {
            let idx = lookup_overload(name, &[Kind::Boxed, Kind::Boxed])
                .unwrap_or_else(|| panic!("{name}(Boxed,Boxed) must match"));
            assert_eq!(SHIMS[idx].ret_kind, Kind::Boxed);
        }
    }

    #[test]
    fn array_list_shims_match_interpreter() {
        // v0.103.0 — spot-check the new shims against their interp
        // counterparts. Each shim arena-boxes a CfmlValue that the JIT
        // caller would observe via Kind::Boxed slot/return.
        use super::super::arena::{Arena, ArenaGuard};
        use super::super::boxed;
        use cfml_common::dynamic::{CfmlArray, CfmlValue};

        let mut arena = Arena::new();
        let _g = ArenaGuard::install(&mut arena);

        let extract = |tagged: i64| -> CfmlValue {
            unsafe { boxed::materialize_tagged(tagged as usize) }
        };

        // arrayFirst / arrayLast on a populated array.
        let arr = boxed::box_value(CfmlValue::Array(CfmlArray::new(vec![
            CfmlValue::Int(10),
            CfmlValue::Int(20),
            CfmlValue::Int(30),
        ]))) as i64;
        let mut bail = 0i64;
        let first = cfml_array_first_boxed(arr, &mut bail);
        assert_eq!(bail, 0);
        assert!(matches!(extract(first), CfmlValue::Int(10)));
        let last = cfml_array_last_boxed(arr, &mut bail);
        assert_eq!(bail, 0);
        assert!(matches!(extract(last), CfmlValue::Int(30)));

        // arrayFirst on non-array BAILS.
        let not_arr = boxed::box_value(CfmlValue::string("nope")) as i64;
        let _ = cfml_array_first_boxed(not_arr, &mut bail);
        assert_eq!(bail, 1, "arrayFirst on non-array must bail");

        // arraySum / arrayAvg on mixed numerics.
        let nums = boxed::box_value(CfmlValue::Array(CfmlArray::new(vec![
            CfmlValue::Int(1),
            CfmlValue::Double(2.5),
            CfmlValue::string("3"),
        ]))) as i64;
        let sum = cfml_array_sum_boxed(nums);
        assert!(matches!(extract(sum), CfmlValue::Double(d) if (d - 6.5).abs() < 1e-9));
        let avg = cfml_array_avg_boxed(nums);
        assert!(matches!(extract(avg), CfmlValue::Double(d) if (d - 6.5/3.0).abs() < 1e-9));

        // arraySum on non-array → Int(0); arrayAvg on empty → Int(0).
        let empty = boxed::box_value(CfmlValue::Array(CfmlArray::new(Vec::new()))) as i64;
        assert!(matches!(extract(cfml_array_avg_boxed(empty)), CfmlValue::Int(0)));
        assert!(matches!(extract(cfml_array_sum_boxed(not_arr)), CfmlValue::Int(0)));

        // listFirst / listLast / listRest.
        let csv = boxed::box_value(CfmlValue::string("a,b,c,d")) as i64;
        assert_eq!(extract(cfml_list_first_boxed(csv)).as_string(), "a");
        assert_eq!(extract(cfml_list_last_boxed(csv)).as_string(), "d");
        assert_eq!(extract(cfml_list_rest_boxed(csv)).as_string(), "b,c,d");

        // listGetAt: 1-based.
        assert_eq!(extract(cfml_list_get_at_boxed_i64(csv, 2)).as_string(), "b");
        // Out-of-range → empty.
        assert_eq!(extract(cfml_list_get_at_boxed_i64(csv, 99)).as_string(), "");

        // listAppend / listPrepend.
        let list = boxed::box_value(CfmlValue::string("x,y")) as i64;
        let val = boxed::box_value(CfmlValue::string("z")) as i64;
        assert_eq!(
            extract(cfml_list_append_boxed_boxed(list, val)).as_string(),
            "x,y,z"
        );
        assert_eq!(
            extract(cfml_list_prepend_boxed_boxed(list, val)).as_string(),
            "z,x,y"
        );
        // Empty list: bare value, no leading delim.
        let empty_str = boxed::box_value(CfmlValue::string("")) as i64;
        assert_eq!(
            extract(cfml_list_append_boxed_boxed(empty_str, val)).as_string(),
            "z"
        );
    }

    #[test]
    fn predicate_shims_match_interpreter() {
        // v0.101.0 — spot-check the type/collection predicates against
        // their cfml-stdlib::builtins::fn_* counterparts. Bit-exact: the
        // shim arena-boxes the same CfmlValue::Bool the interpreter would
        // return, so JIT'd callers see the same value when this flows out
        // through a `Kind::Boxed` slot or return.
        use super::super::arena::{Arena, ArenaGuard};
        use super::super::boxed;
        use cfml_common::dynamic::{CfmlArray, CfmlStruct, CfmlValue};
        use indexmap::IndexMap;

        let mut arena = Arena::new();
        let _g = ArenaGuard::install(&mut arena);

        let extract_bool = |tagged: i64| -> bool {
            let v = unsafe { boxed::borrow_tagged(tagged as usize) };
            match v {
                CfmlValue::Bool(b) => *b,
                other => panic!("expected Bool, got {other:?}"),
            }
        };

        // isNumeric: Int / Double / numeric-string → true; Bool → false.
        let int_arg = boxed::box_value(CfmlValue::Int(42)) as i64;
        let dbl_arg = boxed::box_value(CfmlValue::Double(1.5)) as i64;
        let bool_arg = boxed::box_value(CfmlValue::Bool(true)) as i64;
        let num_str = boxed::box_value(CfmlValue::string(" 3.14 ")) as i64;
        let alpha_str = boxed::box_value(CfmlValue::string("hello")) as i64;
        assert!(extract_bool(cfml_is_numeric_boxed(int_arg)));
        assert!(extract_bool(cfml_is_numeric_boxed(dbl_arg)));
        assert!(!extract_bool(cfml_is_numeric_boxed(bool_arg)));
        assert!(extract_bool(cfml_is_numeric_boxed(num_str)));
        assert!(!extract_bool(cfml_is_numeric_boxed(alpha_str)));

        // isBoolean: numeric + "yes"/"no"/"true"/"false" (case-insens, trimmed).
        let yes_str = boxed::box_value(CfmlValue::string(" YES ")) as i64;
        assert!(extract_bool(cfml_is_boolean_boxed(yes_str)));
        assert!(extract_bool(cfml_is_boolean_boxed(num_str)));
        assert!(!extract_bool(cfml_is_boolean_boxed(alpha_str)));

        // isArray / isStruct / isSimpleValue / isNull discriminate.
        let arr_arg =
            boxed::box_value(CfmlValue::Array(CfmlArray::new(vec![CfmlValue::Int(1)]))) as i64;
        let mut m = cfml_common::dynamic::ValueMap::default();
        m.insert("k".to_string(), CfmlValue::Int(1));
        let struct_arg = boxed::box_value(CfmlValue::Struct(CfmlStruct::new(m))) as i64;
        let null_arg = boxed::box_value(CfmlValue::Null) as i64;
        assert!(extract_bool(cfml_is_array_boxed(arr_arg)));
        assert!(!extract_bool(cfml_is_array_boxed(struct_arg)));
        assert!(extract_bool(cfml_is_struct_boxed(struct_arg)));
        assert!(!extract_bool(cfml_is_struct_boxed(arr_arg)));
        assert!(extract_bool(cfml_is_simple_value_boxed(int_arg)));
        assert!(extract_bool(cfml_is_simple_value_boxed(alpha_str)));
        assert!(!extract_bool(cfml_is_simple_value_boxed(arr_arg)));
        assert!(!extract_bool(cfml_is_simple_value_boxed(struct_arg)));
        assert!(extract_bool(cfml_is_null_boxed(null_arg)));
        assert!(!extract_bool(cfml_is_null_boxed(int_arg)));

        // arrayIsEmpty / structIsEmpty default true for non-collection types.
        let empty_arr = boxed::box_value(CfmlValue::Array(CfmlArray::new(Vec::new()))) as i64;
        let empty_struct =
            boxed::box_value(CfmlValue::Struct(CfmlStruct::new(cfml_common::dynamic::ValueMap::default()))) as i64;
        assert!(extract_bool(cfml_array_is_empty_boxed(empty_arr)));
        assert!(!extract_bool(cfml_array_is_empty_boxed(arr_arg)));
        assert!(extract_bool(cfml_array_is_empty_boxed(int_arg)));
        assert!(extract_bool(cfml_struct_is_empty_boxed(empty_struct)));
        assert!(!extract_bool(cfml_struct_is_empty_boxed(struct_arg)));

        // structCount visible-key handling.
        assert_eq!(cfml_struct_count_boxed_i64(struct_arg), 1);
        assert_eq!(cfml_struct_count_boxed_i64(empty_struct), 0);
        assert_eq!(cfml_struct_count_boxed_i64(int_arg), 0);
        let mut argscope = cfml_common::dynamic::ValueMap::default();
        argscope.insert("__arguments_scope".to_string(), CfmlValue::Bool(true));
        argscope.insert("__arguments_params".to_string(), CfmlValue::Bool(true));
        argscope.insert("a".to_string(), CfmlValue::Int(1));
        argscope.insert("b".to_string(), CfmlValue::Int(2));
        let argscope_tag =
            boxed::box_value(CfmlValue::Struct(CfmlStruct::new(argscope))) as i64;
        assert_eq!(
            cfml_struct_count_boxed_i64(argscope_tag),
            2,
            "structCount hides __arguments_* markers"
        );

        // listLen + arrayToList.
        let csv = boxed::box_value(CfmlValue::string("a,b,c,d")) as i64;
        assert_eq!(cfml_list_len_boxed_i64(csv), 4);
        let blank = boxed::box_value(CfmlValue::string("")) as i64;
        assert_eq!(cfml_list_len_boxed_i64(blank), 0);
        let many_arr = boxed::box_value(CfmlValue::Array(CfmlArray::new(vec![
            CfmlValue::Int(1),
            CfmlValue::Int(2),
            CfmlValue::Int(3),
        ]))) as i64;
        let joined = cfml_array_to_list_boxed(many_arr);
        let v = unsafe { boxed::borrow_tagged(joined as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "1,2,3"));
        let joined_nonarray = cfml_array_to_list_boxed(int_arg);
        let v = unsafe { boxed::borrow_tagged(joined_nonarray as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == ""));

        // structKeyExists (CI, hides __arguments_*).
        let k_present = boxed::box_value(CfmlValue::string("K")) as i64;
        let k_absent = boxed::box_value(CfmlValue::string("missing")) as i64;
        let k_argmarker = boxed::box_value(CfmlValue::string("__arguments_scope")) as i64;
        assert!(extract_bool(cfml_struct_key_exists_boxed_boxed(
            struct_arg, k_present
        )));
        assert!(!extract_bool(cfml_struct_key_exists_boxed_boxed(
            struct_arg, k_absent
        )));
        assert!(!extract_bool(cfml_struct_key_exists_boxed_boxed(
            argscope_tag, k_argmarker
        )));
        // Non-struct receiver → false.
        assert!(!extract_bool(cfml_struct_key_exists_boxed_boxed(
            int_arg, k_present
        )));

        // arrayContains / arrayContainsNoCase.
        let str_arr = boxed::box_value(CfmlValue::Array(CfmlArray::new(vec![
            CfmlValue::string("Foo"),
            CfmlValue::string("bar"),
        ]))) as i64;
        let needle_case = boxed::box_value(CfmlValue::string("FOO")) as i64;
        let needle_exact = boxed::box_value(CfmlValue::string("Foo")) as i64;
        let needle_missing = boxed::box_value(CfmlValue::string("baz")) as i64;
        assert!(extract_bool(cfml_array_contains_boxed_boxed(
            str_arr,
            needle_exact
        )));
        assert!(!extract_bool(cfml_array_contains_boxed_boxed(
            str_arr,
            needle_case
        )));
        assert!(extract_bool(cfml_array_contains_no_case_boxed_boxed(
            str_arr,
            needle_case
        )));
        assert!(!extract_bool(cfml_array_contains_no_case_boxed_boxed(
            str_arr,
            needle_missing
        )));
        // Non-array receiver → false.
        assert!(!extract_bool(cfml_array_contains_boxed_boxed(
            int_arg,
            needle_exact
        )));

        drop(_g);
        for t in [
            int_arg, dbl_arg, bool_arg, num_str, alpha_str, yes_str, arr_arg, struct_arg,
            null_arg, empty_arr, empty_struct, argscope_tag, csv, blank, many_arr, str_arr,
            needle_case, needle_exact, needle_missing, k_present, k_absent, k_argmarker,
        ] {
            drop(unsafe { boxed::reclaim_tagged(t as usize) });
        }
        arena.drain_except(None);
    }

    #[test]
    fn boxed_shims_match_interpreter() {
        use super::super::arena::{Arena, ArenaGuard};
        use super::super::boxed;
        use cfml_common::dynamic::CfmlValue;

        let mut arena = Arena::new();
        let _g = ArenaGuard::install(&mut arena);

        // len on a String returns byte length.
        let s = boxed::box_value(CfmlValue::string("héllo")) as i64;
        assert_eq!(cfml_len_boxed_i64(s), "héllo".len() as i64);
        // len on Int stringifies and counts chars: "12345" → 5.
        let i = boxed::box_value(CfmlValue::Int(12345)) as i64;
        assert_eq!(cfml_len_boxed_i64(i), 5);
        // len on Array.
        let a = boxed::box_value(CfmlValue::Array(cfml_common::dynamic::CfmlArray::new(vec![
            CfmlValue::Int(1),
            CfmlValue::Int(2),
            CfmlValue::Int(3),
        ]))) as i64;
        assert_eq!(cfml_len_boxed_i64(a), 3);

        // ucase / lcase / trim / ltrim / rtrim — produce arena-allocated boxes
        // whose underlying value matches the interpreter.
        let mixed = boxed::box_value(CfmlValue::string("  AbCd  ")) as i64;
        let upper = cfml_ucase_boxed(mixed);
        let v = unsafe { boxed::borrow_tagged(upper as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "  ABCD  "));

        let lower = cfml_lcase_boxed(mixed);
        let v = unsafe { boxed::borrow_tagged(lower as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "  abcd  "));

        let t = cfml_trim_boxed(mixed);
        let v = unsafe { boxed::borrow_tagged(t as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "AbCd"));

        let lt = cfml_ltrim_boxed(mixed);
        let v = unsafe { boxed::borrow_tagged(lt as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "AbCd  "));

        let rt = cfml_rtrim_boxed(mixed);
        let v = unsafe { boxed::borrow_tagged(rt as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "  AbCd"));

        // reverse — chars().rev() on the stringified value.
        let rev_in = boxed::box_value(CfmlValue::string("abcdé")) as i64;
        let rev = cfml_reverse_boxed(rev_in);
        let v = unsafe { boxed::borrow_tagged(rev as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "édcba"));

        // asc — first char as i64; empty string → 0.
        assert_eq!(cfml_asc_boxed_i64(rev_in), 'a' as i64);
        let empty = boxed::box_value(CfmlValue::string("")) as i64;
        assert_eq!(cfml_asc_boxed_i64(empty), 0);

        // stripCr — \r dropped, \n retained.
        let cr_in = boxed::box_value(CfmlValue::string("a\r\nb\rc")) as i64;
        let stripped = cfml_strip_cr_boxed(cr_in);
        let v = unsafe { boxed::borrow_tagged(stripped as usize) };
        assert!(matches!(v, CfmlValue::String(s) if s.as_str() == "a\nbc"));

        // htmlEditFormat — & < > " escaped, ' and / left alone.
        let html_in = boxed::box_value(CfmlValue::string("a<b>&c\"d'/e")) as i64;
        let edited = cfml_html_edit_format_boxed(html_in);
        let v = unsafe { boxed::borrow_tagged(edited as usize) };
        assert!(
            matches!(v, CfmlValue::String(s) if s.as_str() == "a&lt;b&gt;&amp;c&quot;d'/e")
        );

        // htmlCodeFormat — wraps htmlEditFormat in <pre>…</pre>.
        let coded = cfml_html_code_format_boxed(html_in);
        let v = unsafe { boxed::borrow_tagged(coded as usize) };
        assert!(
            matches!(v, CfmlValue::String(s) if s.as_str() == "<pre>a&lt;b&gt;&amp;c&quot;d'/e</pre>")
        );

        // encodeForHtml — adds ' and / escapes on top of htmlEditFormat.
        let encoded = cfml_encode_for_html_boxed(html_in);
        let v = unsafe { boxed::borrow_tagged(encoded as usize) };
        assert!(
            matches!(v, CfmlValue::String(s) if s.as_str() == "a&lt;b&gt;&amp;c&quot;d&#x27;&#x2f;e")
        );

        drop(_g);
        // Reclaim the manually-boxed inputs; arena drains the shim outputs.
        drop(unsafe { boxed::reclaim_tagged(s as usize) });
        drop(unsafe { boxed::reclaim_tagged(i as usize) });
        drop(unsafe { boxed::reclaim_tagged(a as usize) });
        drop(unsafe { boxed::reclaim_tagged(mixed as usize) });
        drop(unsafe { boxed::reclaim_tagged(rev_in as usize) });
        drop(unsafe { boxed::reclaim_tagged(empty as usize) });
        drop(unsafe { boxed::reclaim_tagged(cr_in as usize) });
        drop(unsafe { boxed::reclaim_tagged(html_in as usize) });
        arena.drain_except(None);
    }

    #[test]
    fn pre_v0101_shims_accept_smi_int_inputs() {
        // Regression for the v0.99.6+ SMI hazard: when an Int flows out of
        // the member-IC as a tag-pointer SMI and is passed to a v0.99.0–
        // v0.99.3 shim, the shim must `materialize_tagged` (not
        // `borrow_tagged`, which panics on non-TAG_PTR). Pre-fix this would
        // have aborted; post-fix the shim sees `CfmlValue::Int(n)` exactly
        // as if it had been heap-boxed.
        use super::super::arena::{Arena, ArenaGuard};
        use super::super::boxed;

        let mut arena = Arena::new();
        let _g = ArenaGuard::install(&mut arena);

        // SMI-tagged Int 42 — fits in i61.
        let smi = boxed::try_tag_smi_int(42).expect("42 fits in SMI");
        assert!(boxed::is_smi_int(smi));

        // len("42") = 2 chars via the Int/Double arm.
        assert_eq!(cfml_len_boxed_i64(smi as i64), 2);

        // ucase("42") = "42" — verifies the Boxed→Boxed string path.
        let out = cfml_ucase_boxed(smi as i64);
        let v = unsafe { boxed::borrow_tagged(out as usize) };
        assert!(matches!(v, cfml_common::dynamic::CfmlValue::String(s) if s.as_str() == "42"));

        // asc("42") = '4' as i64 = 52 — verifies the Boxed→Int (non-len) path.
        assert_eq!(cfml_asc_boxed_i64(smi as i64), '4' as i64);

        // 2-arg shim: find("4", "42") = 1 — verifies multi-arg SMI in both slots.
        let needle = boxed::try_tag_smi_int(4).expect("4 fits in SMI");
        assert_eq!(cfml_find_boxed_boxed_i64(needle as i64, smi as i64), 1);

        // arrayLen on an SMI Int → 0 (no bail; matches interp fn_array_len fallthrough).
        let mut bail: i64 = 0;
        assert_eq!(cfml_array_len_boxed_i64(smi as i64, &mut bail), 0);
        assert_eq!(bail, 0);

        drop(_g);
        arena.drain_except(None);
        // SMI values need no reclaim — they don't own heap memory.
    }

    #[test]
    fn shim_semantics_match_interpreter() {
        // Spot-check a handful of values to confirm the extern "C" shims do
        // what `fn_abs`/`fn_min`/`fn_max` do.
        assert_eq!(cfml_abs_i64(-5), 5);
        assert_eq!(cfml_abs_i64(7), 7);
        assert_eq!(cfml_abs_i64(0), 0);
        // wrapping_abs(INT_MIN) = INT_MIN — release-mode parity.
        assert_eq!(cfml_abs_i64(i64::MIN), i64::MIN);
        assert_eq!(cfml_abs_f64(-1.5), 1.5);
        assert_eq!(cfml_min_f64(3.0, 5.0), 3.0);
        assert_eq!(cfml_max_f64(3.0, 5.0), 5.0);
        // f64::min/max propagate NaN per the second operand; same as f64::min in the interp.
        assert!(cfml_min_f64(f64::NAN, 1.0).is_nan() || cfml_min_f64(f64::NAN, 1.0) == 1.0);
    }
}
