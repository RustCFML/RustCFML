//! `${VAR:default}` placeholder expansion — Lucee-compatible.
//!
//! Syntax (identical to Lucee's `.CFConfig.json` importer,
//! `lucee.runtime.config.CFConfigImport#replacePlaceHolder`):
//!
//! ```text
//! ${VAR_NAME}             — env var, empty string if unset
//! ${VAR_NAME:fallback}    — env var with literal fallback
//! ```
//!
//! Resolution order for a placeholder name, mirroring Lucee's
//! `SystemUtil.getSystemPropOrEnvVar`:
//!
//! 1. environment variable with that exact name
//! 2. environment variable with `.` → `_` and upper-cased
//!    (so `${my.setting}` also finds `MY_SETTING`)
//! 3. **legacy RustCFML alias**: if the name starts with `env.`, the remainder
//!    as an environment variable — so pre-v0.548 configs written as
//!    `${env.DB_HOST}` keep working
//! 4. the literal fallback after the first `:`, or empty string if there is none
//!
//! Lucee's step between (1) and (2) — Java system properties — has no
//! equivalent here, so it is skipped.
//!
//! A placeholder is always consumed, even when nothing resolves; the name is
//! not restricted to a namespace. Only the first `}` closes a placeholder (no
//! nesting), and an unterminated `${` is left verbatim, both as in Lucee.
//! Substitution is one-pass: expanded values are NOT re-scanned, so an env var
//! whose value itself contains `${...}` does not recurse.

use std::env;

pub fn expand_env_vars(input: &str) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(offset) = after.find('}') else {
            // Unterminated placeholder — leave the remainder verbatim.
            break;
        };
        out.push_str(&rest[..start]);
        let inner = &after[..offset];
        let (name, fallback) = match inner.find(':') {
            Some(idx) => (&inner[..idx], &inner[idx + 1..]),
            None => (inner, ""),
        };
        out.push_str(&lookup(name).unwrap_or_else(|| fallback.to_string()));
        rest = &after[offset + 1..];
    }
    out.push_str(rest);
    out
}

/// Lucee's `getSystemPropOrEnvVar` chain, plus the legacy `env.` alias.
fn lookup(name: &str) -> Option<String> {
    if let Ok(v) = env::var(name) {
        return Some(v);
    }
    let converted = name.replace('.', "_").to_uppercase();
    if converted != name {
        if let Ok(v) = env::var(&converted) {
            return Some(v);
        }
    }
    if let Some(bare) = name.strip_prefix("env.") {
        if let Ok(v) = env::var(bare) {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_var<F: FnOnce()>(name: &str, value: &str, f: F) {
        let prev = env::var(name).ok();
        env::set_var(name, value);
        f();
        match prev {
            Some(v) => env::set_var(name, v),
            None => env::remove_var(name),
        }
    }

    #[test]
    fn no_placeholders_is_unchanged() {
        assert_eq!(expand_env_vars("hello world"), "hello world");
        assert_eq!(expand_env_vars(""), "");
    }

    #[test]
    fn env_var_resolves() {
        with_var("RUSTCFML_TEST_X", "abc", || {
            assert_eq!(expand_env_vars("${RUSTCFML_TEST_X}"), "abc");
            assert_eq!(
                expand_env_vars("prefix-${RUSTCFML_TEST_X}-suffix"),
                "prefix-abc-suffix"
            );
        });
    }

    #[test]
    fn fallback_used_when_unset() {
        env::remove_var("RUSTCFML_TEST_MISSING");
        assert_eq!(
            expand_env_vars("${RUSTCFML_TEST_MISSING:localhost}"),
            "localhost"
        );
    }

    #[test]
    fn empty_when_no_fallback_and_unset() {
        env::remove_var("RUSTCFML_TEST_EMPTY");
        assert_eq!(expand_env_vars("${RUSTCFML_TEST_EMPTY}"), "");
    }

    #[test]
    fn fallback_may_contain_colons() {
        env::remove_var("RUSTCFML_TEST_URL");
        assert_eq!(
            expand_env_vars("${RUSTCFML_TEST_URL:http://localhost:8080/path}"),
            "http://localhost:8080/path"
        );
    }

    #[test]
    fn dotted_name_falls_back_to_upper_underscore_env_var() {
        with_var("RUSTCFML_TEST_DOTTED", "yes", || {
            assert_eq!(expand_env_vars("${rustcfml.test.dotted}"), "yes");
        });
    }

    #[test]
    fn legacy_env_prefix_still_resolves() {
        with_var("RUSTCFML_TEST_LEGACY", "old", || {
            assert_eq!(expand_env_vars("${env.RUSTCFML_TEST_LEGACY}"), "old");
        });
        env::remove_var("RUSTCFML_TEST_LEGACY_MISSING");
        assert_eq!(
            expand_env_vars("${env.RUSTCFML_TEST_LEGACY_MISSING:localhost}"),
            "localhost"
        );
    }

    #[test]
    fn unresolved_placeholder_is_consumed_not_preserved() {
        env::remove_var("other.X");
        env::remove_var("OTHER_X");
        env::remove_var("X");
        assert_eq!(expand_env_vars("${other.X}"), "");
    }

    #[test]
    fn unclosed_placeholder_is_preserved() {
        assert_eq!(expand_env_vars("${RUSTCFML_TEST_X"), "${RUSTCFML_TEST_X");
        assert_eq!(expand_env_vars("a ${b"), "a ${b");
    }

    #[test]
    fn first_brace_closes_the_placeholder() {
        env::remove_var("RUSTCFML_TEST_BRACE");
        assert_eq!(
            expand_env_vars("${RUSTCFML_TEST_BRACE:a}b}"),
            "ab}"
        );
    }

    #[test]
    fn multiple_placeholders() {
        with_var("RUSTCFML_TEST_A", "1", || {
            with_var("RUSTCFML_TEST_B", "2", || {
                assert_eq!(
                    expand_env_vars("${RUSTCFML_TEST_A}-${RUSTCFML_TEST_B}"),
                    "1-2"
                );
            });
        });
    }

    #[test]
    fn env_value_with_dollar_brace_is_not_recursed() {
        with_var("RUSTCFML_TEST_REC", "${OTHER}", || {
            assert_eq!(expand_env_vars("${RUSTCFML_TEST_REC}"), "${OTHER}");
        });
    }

    #[test]
    fn non_ascii_text_survives() {
        with_var("RUSTCFML_TEST_UTF", "café", || {
            assert_eq!(
                expand_env_vars("naïve ${RUSTCFML_TEST_UTF} — ok"),
                "naïve café — ok"
            );
        });
    }
}
