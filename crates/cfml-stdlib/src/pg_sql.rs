//! PostgreSQL SQL preparation shared by the two PostgreSQL backends:
//!
//!   * the host `postgres`-crate driver in `builtins.rs` (`execute_postgres`),
//!   * the Cloudflare Worker Hyperdrive driver (`cfml-worker`), which talks to
//!     `postgres.js` over JSPI.
//!
//! Both speak PostgreSQL's `$1`/`$2` positional placeholder dialect, but CFML
//! source uses `?` (positional) or `:name` (named) placeholders, so the SQL
//! must be rewritten. Both backends also reject *multi-statement* parameterized
//! SQL in a single round-trip (the host `postgres` crate's `execute`/`query`
//! and `postgres.js`'s `unsafe(sql, params)` each accept exactly one command),
//! so framework-generated "delete + insert replacement" mutations have to be
//! split into one parameterized statement each.
//!
//! This module is intentionally **not** feature-gated and depends only on
//! `cfml-common`, so it compiles for the `wasm32-unknown-unknown` worker target
//! (which never enables the host `postgres_db` feature) as well as the host.
//!
//! See `docs/compatibility-notes/postgres-*.md` for the issues this addresses.

use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::CfmlError;

/// One PostgreSQL-ready statement: SQL with `$1..$n` placeholders plus the
/// ordered, `QueryColumn`-flattened parameter values that fill them. Each
/// statement renumbers its placeholders from `$1`, so the `params` slice is
/// self-contained.
#[derive(Debug, Clone)]
pub struct PgStatement {
    pub sql: String,
    pub params: Vec<CfmlValue>,
}

/// True for result-returning SQL (`SELECT`, `WITH ... SELECT`, `CALL`). Callers
/// use this to decide whether to split on `;` (mutations) or treat the SQL as a
/// single statement (selects, which we never split).
pub fn is_pg_select(sql: &str) -> bool {
    let t = sql.trim_start();
    starts_kw(t, "SELECT") || starts_kw(t, "WITH") || starts_kw(t, "CALL")
}

fn starts_kw(s: &str, kw: &str) -> bool {
    s.len() >= kw.len()
        && s.as_bytes()[..kw.len()].eq_ignore_ascii_case(kw.as_bytes())
        // next char must be a boundary, so `selection` doesn't match `SELECT`
        && s[kw.len()..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true)
}

/// Prepare CFML SQL + params for a PostgreSQL backend.
///
/// * `split` — when true, the SQL is split on top-level `;` into one
///   `PgStatement` per statement (used for mutations). When false the whole
///   string is treated as a single statement (used for selects).
/// * Positional (`?`) params are consumed left-to-right across statements; each
///   statement renumbers its own placeholders from `$1`.
/// * Named (`:name`) params are looked up per statement (case-insensitively),
///   so a name reused within a statement maps to the same `$n`.
/// * `QueryColumn` proxies are flattened to their scalar (first-row) value,
///   matching scalar query-column coercion elsewhere.
///
/// Errors if the positional parameter count doesn't match what the SQL
/// consumes.
pub fn prepare_pg_statements(
    sql: &str,
    params_arg: &CfmlValue,
    split: bool,
) -> Result<Vec<PgStatement>, CfmlError> {
    let statements: Vec<String> = if split {
        split_sql_statements(sql)
    } else {
        vec![sql.to_string()]
    };

    match params_arg {
        CfmlValue::Array(arr) => {
            let flat: Vec<CfmlValue> =
                arr.iter().map(|v| v.query_column_scalar().clone()).collect();
            let mut cursor = 0usize;
            let mut out = Vec::with_capacity(statements.len());
            for stmt in statements {
                let (rewritten, count) = rewrite_positional(&stmt);
                let end = cursor + count;
                if end > flat.len() {
                    return Err(CfmlError::runtime(format!(
                        "queryExecute: not enough positional parameters — a statement needs {} \
                         placeholder(s) but only {} of {} supplied remain",
                        count,
                        flat.len() - cursor,
                        flat.len()
                    )));
                }
                out.push(PgStatement {
                    sql: rewritten,
                    params: flat[cursor..end].to_vec(),
                });
                cursor = end;
            }
            if cursor != flat.len() {
                return Err(CfmlError::runtime(format!(
                    "queryExecute: {} positional parameter(s) supplied but the SQL only consumes {}",
                    flat.len(),
                    cursor
                )));
            }
            Ok(out)
        }
        CfmlValue::Struct(map) => {
            let mut out = Vec::with_capacity(statements.len());
            for stmt in statements {
                let (rewritten, names) = rewrite_named(&stmt);
                let params = names
                    .iter()
                    .map(|name| {
                        let raw = map
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(name))
                            .map(|(_, v)| v.query_column_scalar().clone())
                            .unwrap_or(CfmlValue::Null);
                        // Named param may be a cfqueryparam-style struct
                        // ({value, cfsqltype, null, ...}); unwrap to the
                        // bind value (or NULL when null=true) before
                        // driver binding — matches Lucee semantics.
                        crate::builtins::cfqueryparam_unwrap(&raw)
                    })
                    .collect();
                out.push(PgStatement {
                    sql: rewritten,
                    params,
                });
            }
            Ok(out)
        }
        // Null or any other scalar: no params; pass statements through verbatim.
        _ => Ok(statements
            .into_iter()
            .map(|sql| PgStatement {
                sql,
                params: vec![],
            })
            .collect()),
    }
}

/// Rewrite positional `?` placeholders to `$1..$n`, skipping quoted string
/// literals, quoted identifiers (and their doubled-quote escapes) and
/// dollar-quoted bodies. Returns the rewritten SQL and the number of
/// placeholders consumed.
fn rewrite_positional(stmt: &str) -> (String, usize) {
    let mut out = String::with_capacity(stmt.len() + 8);
    let mut count = 0usize;
    let chars: Vec<char> = stmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '?' => {
                count += 1;
                out.push('$');
                out.push_str(&count.to_string());
                i += 1;
            }
            '\'' | '"' => i = copy_quoted_at(&chars, i, &mut out),
            '$' if dollar_delim_len(&chars, i).is_some() => {
                let len = dollar_delim_len(&chars, i).unwrap();
                i = copy_dollar_quoted(&chars, i, len, &mut out);
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    (out, count)
}

/// Rewrite named `:name` placeholders to `$1..$n` (per statement), skipping
/// quoted regions, dollar-quoted bodies and the `::` cast operator. Returns the
/// rewritten SQL and the ordered list of distinct names in first-seen order (a
/// name reused within the statement reuses its `$n`).
fn rewrite_named(stmt: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(stmt.len() + 8);
    let mut order: Vec<String> = Vec::new();
    let chars: Vec<char> = stmt.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\'' || c == '"' {
            i = copy_quoted_at(&chars, i, &mut out);
            continue;
        }
        if let Some(len) = dollar_delim_len(&chars, i) {
            i = copy_dollar_quoted(&chars, i, len, &mut out);
            continue;
        }
        // `:name` — but not `::` (cast) and not `x:y` where x is part of an
        // identifier (e.g. a time literal handled inside quotes already).
        if c == ':'
            && chars.get(i + 1) != Some(&':')
            && (i == 0 || (!chars[i - 1].is_alphanumeric() && chars[i - 1] != ':'))
            && chars
                .get(i + 1)
                .map(|n| n.is_alphanumeric() || *n == '_')
                .unwrap_or(false)
        {
            let start = i + 1;
            let mut end = start;
            while end < n && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let name: String = chars[start..end].iter().collect();
            let idx = match order.iter().position(|x| x.eq_ignore_ascii_case(&name)) {
                Some(pos) => pos + 1,
                None => {
                    order.push(name);
                    order.len()
                }
            };
            out.push('$');
            out.push_str(&idx.to_string());
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, order)
}

/// Copy the quoted region opening at `chars[i]` (a `'` or `"`) into `out`,
/// handling SQL doubled-quote escapes (`''`, `""`). Returns the index just past
/// the closing quote (or EOF for an unterminated region).
fn copy_quoted_at(chars: &[char], i: usize, out: &mut String) -> usize {
    let q = chars[i];
    out.push(q);
    let mut j = i + 1;
    while j < chars.len() {
        out.push(chars[j]);
        if chars[j] == q {
            if chars.get(j + 1) == Some(&q) {
                out.push(q);
                j += 2;
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    j
}

/// If a PostgreSQL dollar-quote delimiter (`$$` or `$tag$`) opens at `chars[i]`,
/// return its length in chars. The tag follows unquoted-identifier rules
/// (leading letter or underscore), which is what stops an already-rewritten
/// `$1` placeholder from looking like a delimiter.
fn dollar_delim_len(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'$') {
        return None;
    }
    let mut j = i + 1;
    if matches!(chars.get(j), Some(&c) if c.is_ascii_alphabetic() || c == '_') {
        j += 1;
        while matches!(chars.get(j), Some(&c) if c.is_ascii_alphanumeric() || c == '_') {
            j += 1;
        }
    }
    (chars.get(j) == Some(&'$')).then(|| j + 1 - i)
}

/// Copy a dollar-quoted region verbatim into `out`, given its opening delimiter
/// of `len` chars starts at `chars[i]`. Everything inside is opaque — `;`,
/// quotes, comment markers and `?`/`:name` placeholders alike — up to the
/// *identical* closing tag (a differently-tagged `$$` nested inside stays part
/// of the body). Returns the index just past the closing delimiter, or EOF for
/// an unterminated region.
fn copy_dollar_quoted(chars: &[char], i: usize, len: usize, out: &mut String) -> usize {
    out.extend(&chars[i..i + len]);
    let mut j = i + len;
    while j < chars.len() {
        if chars.len() - j >= len && chars[j..j + len] == chars[i..i + len] {
            out.extend(&chars[j..j + len]);
            return j + len;
        }
        out.push(chars[j]);
        j += 1;
    }
    j
}

/// Split SQL into statements on top-level `;`, respecting single/double quoted
/// regions (and their doubled-quote escapes), dollar-quoted bodies
/// (`$$ … $$` / `$tag$ … $tag$`) and `--` / `/* */` comments so a `;` inside
/// them doesn't cause a bad split. Trims whitespace and drops empty
/// statements; always returns at least one (possibly empty) statement.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut stmts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // line comment
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < n && chars[i] != '\n' {
                cur.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // block comment
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            cur.push('/');
            cur.push('*');
            i += 2;
            while i < n && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                cur.push(chars[i]);
                i += 1;
            }
            if i < n {
                cur.push('*');
                cur.push('/');
                i += 2;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            i = copy_quoted_at(&chars, i, &mut cur);
            continue;
        }
        // PostgreSQL dollar-quoted body ($$ … $$ / $tag$ … $tag$): opaque, so a
        // `;` inside a CREATE FUNCTION / DO body doesn't split the statement.
        if let Some(len) = dollar_delim_len(&chars, i) {
            i = copy_dollar_quoted(&chars, i, len, &mut cur);
            continue;
        }
        if c == ';' {
            if !cur.trim().is_empty() {
                stmts.push(cur.trim().to_string());
            }
            cur.clear();
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    if !cur.trim().is_empty() {
        stmts.push(cur.trim().to_string());
    }
    if stmts.is_empty() {
        stmts.push(String::new());
    }
    stmts
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::CfmlValue;
    use indexmap::IndexMap;

    fn arr(vals: Vec<CfmlValue>) -> CfmlValue {
        CfmlValue::array(vals)
    }
    fn s(v: &str) -> CfmlValue {
        CfmlValue::string(v.to_string())
    }
    // CfmlValue has no PartialEq; compare by stringified form for tests.
    fn params_str(p: &PgStatement) -> Vec<String> {
        p.params.iter().map(|v| v.as_string()).collect()
    }

    #[test]
    fn positional_single_statement() {
        let p = prepare_pg_statements(
            "select * from profiles where id = ?",
            &arr(vec![s("41048aa7-27c9-4517-a93e-82bf7c76cc66")]),
            false,
        )
        .unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].sql, "select * from profiles where id = $1");
        assert_eq!(p[0].params.len(), 1);
    }

    #[test]
    fn positional_question_mark_in_string_literal_is_skipped() {
        let p =
            prepare_pg_statements("select '? not a param', x where y = ?", &arr(vec![s("v")]), false)
                .unwrap();
        assert_eq!(p[0].sql, "select '? not a param', x where y = $1");
    }

    #[test]
    fn multi_statement_mutation_split_and_renumbered() {
        // The PR #54 example shape.
        let sql = "
            delete from profile_role where profile_id = ?;
            insert into profile_role (profile_id, role_id) values (?, ?);
        ";
        let p = prepare_pg_statements(
            sql,
            &arr(vec![CfmlValue::Int(1), CfmlValue::Int(1), CfmlValue::Int(7)]),
            true,
        )
        .unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].sql, "delete from profile_role where profile_id = $1");
        assert_eq!(params_str(&p[0]), vec!["1"]);
        assert_eq!(
            p[1].sql,
            "insert into profile_role (profile_id, role_id) values ($1, $2)"
        );
        assert_eq!(params_str(&p[1]), vec!["1", "7"]);
    }

    #[test]
    fn select_is_not_split_even_with_trailing_semicolon() {
        let p = prepare_pg_statements("select 1; ", &CfmlValue::Null, false).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].sql, "select 1; ");
    }

    #[test]
    fn param_count_mismatch_too_few() {
        let e = prepare_pg_statements("update t set a = ?, b = ?", &arr(vec![s("x")]), false);
        assert!(e.is_err());
    }

    #[test]
    fn param_count_mismatch_too_many() {
        let e = prepare_pg_statements(
            "update t set a = ?",
            &arr(vec![s("x"), s("y")]),
            false,
        );
        assert!(e.is_err());
    }

    #[test]
    fn named_params_reused_within_statement() {
        let mut m = cfml_common::dynamic::ValueMap::default();
        m.insert("id".to_string(), CfmlValue::Int(5));
        m.insert("name".to_string(), s("bob"));
        let p = prepare_pg_statements(
            "update t set name = :name where id = :id or parent = :id",
            &CfmlValue::strukt(m),
            false,
        )
        .unwrap();
        assert_eq!(
            p[0].sql,
            "update t set name = $1 where id = $2 or parent = $2"
        );
        assert_eq!(params_str(&p[0]), vec!["bob", "5"]);
    }

    #[test]
    fn named_params_cast_operator_not_treated_as_placeholder() {
        let mut m = cfml_common::dynamic::ValueMap::default();
        m.insert("id".to_string(), s("abc"));
        let p = prepare_pg_statements(
            "select id::text from t where id = :id",
            &CfmlValue::strukt(m),
            false,
        )
        .unwrap();
        assert_eq!(p[0].sql, "select id::text from t where id = $1");
        assert_eq!(params_str(&p[0]), vec!["abc"]);
    }

    #[test]
    fn semicolon_inside_string_literal_does_not_split() {
        let p = prepare_pg_statements(
            "insert into t (note) values ('a; b')",
            &CfmlValue::Null,
            true,
        )
        .unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].sql, "insert into t (note) values ('a; b')");
    }

    #[test]
    fn query_column_param_flattened_to_first_row() {
        let col = CfmlValue::QueryColumn(std::sync::Arc::new(vec![s("first"), s("second")]), 0);
        let p = prepare_pg_statements("select * from t where x = ?", &arr(vec![col]), false)
            .unwrap();
        assert_eq!(params_str(&p[0]), vec!["first"]);
    }

    // --- GH #288: dollar-quoted bodies are opaque to the splitter/rewriters ---

    #[test]
    fn dollar_quoted_function_body_semicolon_does_not_split() {
        let sql = "CREATE OR REPLACE FUNCTION f(p text) RETURNS text AS $$
            SELECT s.value FROM t s WHERE s.key = p LIMIT 1;
        $$ LANGUAGE sql; SELECT 2";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2, "got: {stmts:#?}");
        assert!(stmts[0].ends_with("LANGUAGE sql"));
        assert!(stmts[0].contains("LIMIT 1;"));
        assert_eq!(stmts[1], "SELECT 2");
    }

    #[test]
    fn tagged_dollar_quote_do_block_does_not_split() {
        let stmts = split_sql_statements("DO $body$ BEGIN NULL; END $body$; SELECT 1");
        assert_eq!(stmts.len(), 2, "got: {stmts:#?}");
        assert_eq!(stmts[0], "DO $body$ BEGIN NULL; END $body$");
        assert_eq!(stmts[1], "SELECT 1");
    }

    #[test]
    fn nested_untagged_quote_stays_inside_outer_tag() {
        let stmts = split_sql_statements("DO $outer$ a; $$ b; $$ c; $outer$; SELECT 1");
        assert_eq!(stmts.len(), 2, "got: {stmts:#?}");
        assert_eq!(stmts[0], "DO $outer$ a; $$ b; $$ c; $outer$");
    }

    #[test]
    fn dollar_signs_inside_ordinary_quotes_are_untouched() {
        let stmts = split_sql_statements("SELECT '$$ not a quote'; SELECT 1");
        assert_eq!(stmts.len(), 2, "got: {stmts:#?}");
        assert_eq!(stmts[0], "SELECT '$$ not a quote'");
    }

    #[test]
    fn placeholders_inside_dollar_quoted_body_are_not_rewritten() {
        // `?` and `:name` inside a function body are pl/pgsql text, not binds.
        let (sql, count) = rewrite_positional("CREATE FUNCTION f() AS $$ SELECT '?' $$; ");
        assert_eq!(count, 0);
        assert!(sql.contains("'?'"), "got: {sql}");
        let (sql, names) = rewrite_named("DO $b$ SELECT x :: text WHERE y = :real $b$");
        assert!(names.is_empty(), "got: {names:?}");
        assert!(sql.contains(":real"), "got: {sql}");
    }

    #[test]
    fn positional_placeholder_is_not_mistaken_for_a_dollar_quote() {
        // `$1 ... $2` must not read as a `$1$`-style delimiter, and rewriting
        // a second time must still see the rest of the statement.
        let (sql, count) = rewrite_positional("select ? , $1 , ? from t");
        assert_eq!(count, 2);
        assert_eq!(sql, "select $1 , $1 , $2 from t");
    }

    #[test]
    fn is_pg_select_matches_keywords() {
        assert!(is_pg_select("  select 1"));
        assert!(is_pg_select("WITH x AS (select 1) select * from x"));
        assert!(is_pg_select("CALL foo()"));
        assert!(!is_pg_select("selection_from_t"));
        assert!(!is_pg_select("delete from t"));
        assert!(!is_pg_select("insert into t values (1)"));
    }
}
