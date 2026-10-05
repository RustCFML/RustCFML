//! Type-aware value comparison for QoQ — the single comparator used by every
//! comparison operator, ORDER BY, DISTINCT/GROUP BY keys and MIN/MAX.
//!
//! Port of BoxLang's `QoQCompare`, adapted to RustCFML's native value types
//! (there is no column-type metadata — the actual `CfmlValue` variant decides
//! the comparison): two strings compare lexically (case-insensitive) even when
//! they look numeric; two numbers compare numerically; mixed number/string
//! coerces to numeric when both look numeric, else lexically.

use std::cmp::Ordering;

use cfml_common::dynamic::CfmlValue;

enum Class {
    Num(f64),
    Str(String),
}

fn classify(v: &CfmlValue) -> Option<Class> {
    match v {
        CfmlValue::Null => None,
        CfmlValue::Int(i) => Some(Class::Num(*i as f64)),
        CfmlValue::Double(d) => Some(Class::Num(*d)),
        CfmlValue::Bool(b) => Some(Class::Num(if *b { 1.0 } else { 0.0 })),
        CfmlValue::String(s) => Some(Class::Str((**s).clone())),
        // Arrays/structs/etc. aren't expected as QoQ cell values; compare by
        // their string form as a last resort.
        other => Some(Class::Str(other.as_string())),
    }
}

fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Case-insensitive lexical compare. ASCII-only fast path is allocation-free
/// (byte-by-byte fold to lowercase); only falls back to the allocating
/// `to_lowercase()` path when either string contains a non-ASCII byte.
fn str_cmp(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let min_len = ab.len().min(bb.len());
    let mut i = 0;
    while i < min_len {
        let ba = ab[i];
        let bbi = bb[i];
        if ba >= 0x80 || bbi >= 0x80 {
            return a.to_lowercase().cmp(&b.to_lowercase());
        }
        let ca = ba.to_ascii_lowercase();
        let cb = bbi.to_ascii_lowercase();
        if ca != cb {
            return ca.cmp(&cb);
        }
        i += 1;
    }
    // Common prefix matched. If the tail (longer side) contains non-ASCII, the
    // Unicode comparison rules may say otherwise — fall back. Otherwise the
    // shorter string is less than the longer.
    let tail = if ab.len() > min_len { &ab[min_len..] } else { &bb[min_len..] };
    for &t in tail {
        if t >= 0x80 {
            return a.to_lowercase().cmp(&b.to_lowercase());
        }
    }
    ab.len().cmp(&bb.len())
}

fn num_cmp(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

/// SQL comparison with three-valued semantics: `None` when either operand is
/// NULL (the result is "unknown"); otherwise `Some(ordering)`.
///
/// Same-type fast paths (String/String, Int/Int, Double/Double, Bool/Bool,
/// Bool/Int) borrow directly from the inputs without allocating. The slower
/// mixed-type path via `classify` still allocates for non-String values that
/// need stringifying (Arrays/Structs etc., rare as QoQ cell values).
pub fn compare_sql(a: &CfmlValue, b: &CfmlValue) -> Option<Ordering> {
    if matches!(a, CfmlValue::Null) || matches!(b, CfmlValue::Null) {
        return None;
    }
    match (a, b) {
        (CfmlValue::String(sa), CfmlValue::String(sb)) => return Some(str_cmp(sa, sb)),
        (CfmlValue::Int(ia), CfmlValue::Int(ib)) => return Some(ia.cmp(ib)),
        (CfmlValue::Double(da), CfmlValue::Double(db)) => return Some(da.total_cmp(db)),
        (CfmlValue::Bool(ba), CfmlValue::Bool(bb)) => return Some(ba.cmp(bb)),
        (CfmlValue::Bool(ba), CfmlValue::Int(ib)) => {
            return Some((if *ba { 1i64 } else { 0i64 }).cmp(ib));
        }
        (CfmlValue::Int(ia), CfmlValue::Bool(bb)) => {
            return Some(ia.cmp(&(if *bb { 1i64 } else { 0i64 })));
        }
        // Dates compare as instants; a date against a date string parses the
        // string, against a number reads it as a numeric date.
        (CfmlValue::DateTime(da), CfmlValue::DateTime(db)) => {
            return Some(da.epoch_nanos().cmp(&db.epoch_nanos()));
        }
        (CfmlValue::DateTime(d), other) => return Some(date_vs(d, other)),
        (other, CfmlValue::DateTime(d)) => return Some(date_vs(d, other).reverse()),
        _ => {}
    }
    let (ca, cb) = (classify(a)?, classify(b)?);
    Some(match (ca, cb) {
        (Class::Num(x), Class::Num(y)) => num_cmp(x, y),
        (Class::Str(x), Class::Str(y)) => str_cmp(&x, &y),
        (Class::Num(x), Class::Str(y)) => match parse_num(&y) {
            Some(yn) => num_cmp(x, yn),
            None => str_cmp(&fmt_num(x), &y),
        },
        (Class::Str(x), Class::Num(y)) => match parse_num(&x) {
            Some(xn) => num_cmp(xn, y),
            None => str_cmp(&x, &fmt_num(y)),
        },
    })
}

fn date_vs(d: &cfml_common::datetime::CfmlDate, other: &CfmlValue) -> Ordering {
    use cfml_common::datetime::{parse, CfmlDate};
    let as_date = match other {
        CfmlValue::Int(i) => CfmlDate::from_numeric(*i as f64),
        CfmlValue::Double(f) => CfmlDate::from_numeric(*f),
        CfmlValue::String(s) => parse::parse_date(s.trim()),
        _ => None,
    };
    match as_date {
        Some(o) => d.epoch_nanos().cmp(&o.epoch_nanos()),
        None => str_cmp(&d.to_cfml_string(), &other.as_string()),
    }
}

/// Total order for sorting / MIN / MAX. NULL sorts before every non-NULL value.
pub fn compare_total(a: &CfmlValue, b: &CfmlValue) -> Ordering {
    match (matches!(a, CfmlValue::Null), matches!(b, CfmlValue::Null)) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => compare_sql(a, b).unwrap_or(Ordering::Equal),
    }
}

/// SQL equality: `None` (unknown) when either side is NULL, else
/// `Some(values are equal)`.
pub fn sql_equal(a: &CfmlValue, b: &CfmlValue) -> Option<bool> {
    compare_sql(a, b).map(|o| o == Ordering::Equal)
}

/// Render a number the way CFML stringifies it (no trailing `.0`).
fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        (n as i64).to_string()
    } else {
        n.to_string()
    }
}

/// Append one value's type-tagged key bytes to `key`. Shared between
/// `group_key` and column-major dedup so we can hash a row without materialising
/// a `Vec<CfmlValue>` of its cells.
pub fn append_group_key(key: &mut String, v: &CfmlValue) {
    match v {
        CfmlValue::Null => key.push_str("N\u{1}"),
        CfmlValue::Bool(b) => {
            key.push('B');
            key.push(if *b { 'T' } else { 'F' });
            key.push('\u{1}');
        }
        CfmlValue::Int(i) => {
            key.push('#');
            key.push_str(&i.to_string());
            key.push('\u{1}');
        }
        CfmlValue::Double(d) => {
            key.push('#');
            key.push_str(&fmt_num(*d));
            key.push('\u{1}');
        }
        CfmlValue::String(s) => {
            key.push('S');
            key.push_str(&s.to_lowercase());
            key.push('\u{1}');
        }
        CfmlValue::DateTime(d) => {
            key.push('D');
            key.push_str(&d.epoch_nanos().to_string());
            key.push('\u{1}');
        }
        other => {
            key.push('S');
            key.push_str(&other.as_string().to_lowercase());
            key.push('\u{1}');
        }
    }
}

/// A canonical, type-tagged key string for DISTINCT and GROUP BY partitioning.
/// The type tag prevents `1` (Int) and `"1"` (String) from collapsing together.
pub fn group_key(values: &[CfmlValue]) -> String {
    let mut key = String::new();
    for v in values {
        append_group_key(&mut key, v);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nulls_sort_first() {
        assert_eq!(compare_total(&CfmlValue::Null, &CfmlValue::Int(1)), Ordering::Less);
        assert_eq!(compare_total(&CfmlValue::Int(1), &CfmlValue::Null), Ordering::Greater);
        assert_eq!(compare_total(&CfmlValue::Null, &CfmlValue::Null), Ordering::Equal);
    }

    #[test]
    fn numeric_cross_type() {
        assert_eq!(sql_equal(&CfmlValue::Int(1), &CfmlValue::Double(1.0)), Some(true));
        assert_eq!(compare_sql(&CfmlValue::Int(2), &CfmlValue::Double(10.0)), Some(Ordering::Less));
    }

    #[test]
    fn strings_compare_lexically_case_insensitive() {
        // Two strings compare lexically even when numeric-looking.
        assert_eq!(
            compare_sql(&CfmlValue::string("10"), &CfmlValue::string("9")),
            Some(Ordering::Less)
        );
        assert_eq!(
            sql_equal(&CfmlValue::string("ABC"), &CfmlValue::string("abc")),
            Some(true)
        );
    }

    #[test]
    fn mixed_number_string_coerces_when_numeric() {
        assert_eq!(sql_equal(&CfmlValue::Int(10), &CfmlValue::string("10")), Some(true));
        assert_eq!(
            compare_sql(&CfmlValue::Int(10), &CfmlValue::string("abc")),
            Some(str_cmp("10", "abc"))
        );
    }

    #[test]
    fn null_comparison_is_unknown() {
        assert_eq!(sql_equal(&CfmlValue::Null, &CfmlValue::Int(1)), None);
        assert_eq!(compare_sql(&CfmlValue::Null, &CfmlValue::string("x")), None);
    }

    #[test]
    fn group_key_distinguishes_types() {
        assert_ne!(
            group_key(&[CfmlValue::Int(1)]),
            group_key(&[CfmlValue::string("1")])
        );
        assert_eq!(
            group_key(&[CfmlValue::string("A")]),
            group_key(&[CfmlValue::string("a")])
        );
    }
}
