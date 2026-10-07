//! Case-insensitive string helpers that do not allocate on the common path.
//!
//! `compareNoCase`, the `<`/`>` string branch and string `==` used to lowercase
//! BOTH operands into fresh `String`s on every call. ColdBox's
//! `getInheritedMetaData` compares every function name of a class with every
//! function name of its parent, so a Preside boot made hundreds of thousands of
//! those copies (≈125 B and ≈0.3 µs per `compareNoCase`).

use std::cmp::Ordering;

/// Order `a` and `b` as `a.to_lowercase().cmp(&b.to_lowercase())` does —
/// byte-for-byte the same answer — without allocating when both are ASCII.
/// Non-ASCII input falls back to full Unicode lowercasing (final-sigma and
/// multi-char expansions make a per-char fold differ from `str::to_lowercase`).
pub fn cmp_ignore_case(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        let (x, y) = (a.as_bytes(), b.as_bytes());
        for (p, q) in x.iter().zip(y) {
            match p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase()) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        return x.len().cmp(&y.len());
    }
    a.to_lowercase().cmp(&b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(a: &str, b: &str) -> Ordering {
        a.to_lowercase().cmp(&b.to_lowercase())
    }

    #[test]
    fn matches_lowercase_then_compare() {
        let cases = [
            ("", ""),
            ("a", ""),
            ("", "a"),
            ("abc", "ABC"),
            ("abc", "abd"),
            ("ABD", "abc"),
            ("init", "Init"),
            ("getName", "getname"),
            ("a", "B"),
            ("Z", "a"),
            ("_x", "X"),
            ("[", "a"),
            ("abc", "abcd"),
            ("ÄBC", "äbc"),
            ("straße", "STRASSE"),
            ("ΟΔΟΣ", "οδος"),
            ("İ", "i"),
            ("é", "E"),
        ];
        for (a, b) in cases {
            assert_eq!(cmp_ignore_case(a, b), reference(a, b), "{a:?} vs {b:?}");
            assert_eq!(cmp_ignore_case(b, a), reference(b, a), "{b:?} vs {a:?}");
        }
    }
}
