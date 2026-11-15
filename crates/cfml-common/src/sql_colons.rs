//! `::` in SQL run with parameters (GH #513).
//!
//! Whenever `queryExecute` is given a `params` argument (an array or a struct,
//! even an empty one), Lucee passes the SQL through `QueryParamConverter`,
//! which reads `::` outside quotes and comments as an ESCAPED colon and emits
//! one `:`. So SQL written for Lucee spells a PostgreSQL cast `::::` when it
//! has params (Preside's PostgreSQL adapter does: `USING (col::::int)`), and a
//! bare `::` there becomes `:`. Without params, Lucee leaves the SQL alone, as
//! it does for `<cfquery>` whose only params are `<cfqueryparam>` tags.
//!
//! Lucee's scanner, as probed on 7.1.2.191, protects single-quoted strings
//! (`''` inside one simply closes and reopens it), double-quoted identifiers
//! and `--` / `/* */` comments. It does NOT know backslash escapes or
//! PostgreSQL dollar quotes, so `::` inside those is collapsed too; this
//! follows it exactly, since SQL written for Lucee is spelled for that.

use std::borrow::Cow;

/// Collapse every `::` outside quotes and comments to `:`.
pub fn collapse_escaped_colons(sql: &str) -> Cow<'_, str> {
    if !sql.contains("::") {
        return Cow::Borrowed(sql);
    }
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    // Copy runs of bytes at once; every delimiter we look for is ASCII, so a
    // run boundary is always a char boundary.
    let mut run_start = 0;
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'\'' | b'"') => {
                // To the matching close quote (or the end).
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            b':' if bytes.get(i + 1) == Some(&b':') => {
                out.push_str(&sql[run_start..=i]);
                i += 2;
                run_start = i;
            }
            _ => i += 1,
        }
    }
    if run_start < sql.len() {
        out.push_str(&sql[run_start..]);
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::collapse_escaped_colons as c;

    #[test]
    fn a_double_colon_outside_quotes_is_one_colon() {
        assert_eq!(c("select '12'::::int as v"), "select '12'::int as v");
        assert_eq!(c("select '12'::int as v"), "select '12':int as v");
        assert_eq!(c("select :x::::int as v"), "select :x::int as v");
        assert_eq!(c("select ?::::int as v"), "select ?::int as v");
        // Three colons: the first pair collapses, the third stays.
        assert_eq!(c("select '12':::x"), "select '12'::x");
    }

    #[test]
    fn quotes_identifiers_and_comments_are_left_alone() {
        assert_eq!(c("select length('a::::b')"), "select length('a::::b')");
        assert_eq!(c("select 'it''s', '1'::::int"), "select 'it''s', '1'::int");
        assert_eq!(c(r#"select 1 as "a::::b""#), r#"select 1 as "a::::b""#);
        assert_eq!(c("select /* it's a::b */ '1'::::int"), "select /* it's a::b */ '1'::int");
        assert_eq!(c("select 1 -- it's a::b\n, '1'::::int"), "select 1 -- it's a::b\n, '1'::int");
    }

    #[test]
    fn backslash_escapes_and_dollar_quotes_are_not_special_as_on_lucee() {
        // Lucee 7.1.2: `select E'it\'s::::x'` returns "it's::x".
        assert_eq!(c(r"select E'it\'s::::x'"), r"select E'it\'s::x'");
        // Lucee 7.1.2: `select $$a::::b$$` returns "a::b".
        assert_eq!(c("select $$a::::b$$"), "select $$a::b$$");
    }

    #[test]
    fn sql_without_a_double_colon_is_borrowed_unchanged() {
        assert!(matches!(c("select @a := 1"), std::borrow::Cow::Borrowed(_)));
        assert_eq!(c("select 'é::'"), "select 'é::'");
        assert_eq!(c("select 'é'::::text"), "select 'é'::text");
    }
}
