//! CFML numeric-string parsing — the single definition of "is this string a
//! number?" shared by `isNumeric`, the equality/comparison operators and
//! numeric coercion.
//!
//! Rust's `f64::from_str` is almost exactly Lucee's rule already: it accepts
//! `007`, `.5`, `5.`, `+.5`, `1e+2`, `1E2` and surrounding whitespace once
//! trimmed, and rejects `1_000`, `1d`, `1.2.3`, `--1`, `1e` and `1,0`. The ONE
//! divergence is the IEEE special spellings — Rust accepts `inf`, `Infinity`,
//! `-Infinity` and `NaN`, where Lucee reports every one of them non-numeric
//! (verified against Lucee 7.1.0.204). Letting them through made
//! `isNumeric("INF")` true, and would have made `"INF" eq "Infinity"` true once
//! equality became numeric-aware (GH #398).

/// The numeric value of a CFML string, or `None` when the string is not a
/// number by CFML's rules.
pub fn numeric_string_value(s: &str) -> Option<f64> {
    // `is_finite` rejects both the infinities and NaN in one test.
    s.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Whether a string is numeric by CFML's rules — `isNumeric`'s string branch.
pub fn is_numeric_string(s: &str) -> bool {
    numeric_string_value(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_what_lucee_accepts() {
        for s in [
            "0", "007", "000000", ".5", "5.", "+.5", "-0", "1e2", "1e+2", "1E2", "  12  ", "1.0",
        ] {
            assert!(is_numeric_string(s), "{s} should be numeric");
        }
    }

    #[test]
    fn rejects_what_lucee_rejects() {
        // The IEEE spellings are the whole reason this helper exists: they are
        // the only place `f64::from_str` is more permissive than CFML.
        for s in [
            "inf", "Infinity", "-Infinity", "INF", "nan", "NaN", "1_000", "1d", "1f", "1e", "e1",
            "1.2.3", "--1", "1,0", "0x10", "", " ", "abc",
        ] {
            assert!(!is_numeric_string(s), "{s} should NOT be numeric");
        }
    }

    #[test]
    fn zero_padded_and_scaled_forms_share_a_value() {
        // The GH #398 rows: textually different, numerically identical.
        for (a, b) in [("000", "000000"), ("01", "1"), ("1.0", "1"), ("1e2", "100"), (" 1", "1")] {
            assert_eq!(numeric_string_value(a), numeric_string_value(b), "{a} vs {b}");
        }
    }
}
