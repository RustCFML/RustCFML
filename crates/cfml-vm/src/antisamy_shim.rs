//! `org.owasp.validator.html.AntiSamy` / `.Policy` / `.CleanResults`.
//!
//! AntiSamy is a widely-used HTML sanitiser in the CFML world — Preside runs
//! **every non-admin request parameter** through it as its XSS filter, so an
//! application that reaches for it must keep working with no source changes.
//! The shim exposes exactly the surface such callers use:
//!
//! ```cfml
//! antiSamy  = createObject( "java", "org.owasp.validator.html.AntiSamy", jars );
//! policy    = createObject( "java", "org.owasp.validator.html.Policy", jars )
//!                 .getInstance( createObject( "java", "java.io.File" ).init( path ) );
//! cleanHtml = antiSamy.scan( dirtyHtml, policy ).getCleanHtml();
//! ```
//!
//! The jar-path argument is ignored, as the other third-party shims do — there
//! is no JVM to load jars into, and the sanitiser is native
//! ([`cfml_sanitize`]).
//!
//! A `Policy` is opaque to callers: it is only ever handed back to `scan()`, so
//! it is represented as the policy file's path and resolved through
//! `cfml_sanitize`'s parse cache. Callers typically build a handful of policies
//! once and then sanitise with them on every request.
//!
//! Unknown methods **throw**, matching the other third-party shims (BCrypt,
//! SnakeYAML, commons-imaging) rather than returning a silent null — a
//! sanitiser that quietly answered `null` would be a security failure that
//! looks like working code.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const ANTISAMY_CLASS: &str = "org.owasp.validator.html.antisamy";
pub const POLICY_CLASS: &str = "org.owasp.validator.html.policy";
pub const CLEAN_RESULTS_CLASS: &str = "org.owasp.validator.html.cleanresults";

pub fn is_antisamy_class(class_lower: &str) -> bool {
    matches!(class_lower, ANTISAMY_CLASS | POLICY_CLASS | CLEAN_RESULTS_CLASS)
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

/// `org.owasp.validator.html.PolicyException` — what the Java library raises
/// for an unreadable or malformed policy, and what callers catch.
fn policy_exception(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("org.owasp.validator.html.PolicyException: {}", message),
        CfmlErrorType::Custom("org.owasp.validator.html.PolicyException".to_string()),
    )
}

/// `ScanException` — raised when the input itself cannot be scanned (e.g. it
/// exceeds the policy's `maxInputSize`).
fn scan_exception(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("org.owasp.validator.html.ScanException: {}", message),
        CfmlErrorType::Custom("org.owasp.validator.html.ScanException".to_string()),
    )
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn field(object: &CfmlValue, key: &str) -> Option<String> {
    match object {
        CfmlValue::Struct(s) => s.get(key).map(|v| v.as_string()),
        _ => None,
    }
}

/// Pull a filesystem path out of whatever `Policy.getInstance()` was handed: a
/// `java.io.File` shim, or a plain string (some callers skip the File hop).
fn policy_path(arg: &CfmlValue) -> Option<String> {
    if let Some(path) = field(arg, "__path") {
        if !path.is_empty() {
            return Some(path);
        }
    }
    match arg {
        CfmlValue::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

pub fn handle_antisamy(method: &str, args: Vec<CfmlValue>, _object: &CfmlValue) -> CfmlResult {
    match method {
        // The Java class is constructed with `new AntiSamy()` and CFML's
        // createObject may or may not be followed by `.init()`; both land here.
        "init" => Ok(CfmlValue::strukt(shim(ANTISAMY_CLASS))),
        "scan" => {
            let html = args.first().map(|v| v.as_string()).unwrap_or_default();
            let policy_arg = args.get(1).cloned().unwrap_or(CfmlValue::Null);
            let path = field(&policy_arg, "__policy_path").ok_or_else(|| {
                policy_exception(
                    "scan() requires a Policy built by Policy.getInstance(); \
                     got a value that is not one",
                )
            })?;
            let policy = cfml_sanitize::policy_for_file(&path).map_err(policy_exception)?;
            let cleaned = cfml_sanitize::sanitize(&html, &policy).map_err(scan_exception)?;

            let mut results = shim(CLEAN_RESULTS_CLASS);
            results.insert("__clean_html".to_string(), CfmlValue::string(cleaned));
            Ok(CfmlValue::strukt(results))
        }
        other => Err(unsupported("AntiSamy", other)),
    }
}

pub fn handle_policy(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => Ok(CfmlValue::strukt(shim(POLICY_CLASS))),
        // Static on the Java side; called on the un-init'ed class object here,
        // which is the shape CFML callers use.
        "getinstance" => {
            let arg = args.first().cloned().unwrap_or(CfmlValue::Null);
            let path = policy_path(&arg).ok_or_else(|| {
                policy_exception("getInstance() requires a java.io.File or a path string")
            })?;
            // Parse eagerly so a missing or malformed policy fails HERE, where
            // the Java library raises PolicyException and callers catch it —
            // not later, mid-request, inside scan().
            cfml_sanitize::policy_for_file(&path).map_err(policy_exception)?;

            let mut m = shim(POLICY_CLASS);
            m.insert("__policy_path".to_string(), CfmlValue::string(path));
            Ok(CfmlValue::strukt(m))
        }
        "getdirective" => {
            let path = field(object, "__policy_path").unwrap_or_default();
            let name = args
                .first()
                .map(|v| v.as_string().to_ascii_lowercase())
                .unwrap_or_default();
            let policy = cfml_sanitize::policy_for_file(&path).map_err(policy_exception)?;
            Ok(match policy.directives.get(&name) {
                Some(v) => CfmlValue::string(v.clone()),
                None => CfmlValue::Null,
            })
        }
        other => Err(unsupported("Policy", other)),
    }
}

pub fn handle_clean_results(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "getcleanhtml" => Ok(CfmlValue::string(
            field(object, "__clean_html").unwrap_or_default(),
        )),
        // The real CleanResults reports what it changed. We do not track
        // per-change messages, and an empty list would read as "nothing was
        // removed" — which is a dangerous thing to assert falsely — so these
        // are refused rather than answered wrongly.
        "geterrormessages" | "getnumberoferrors" => Err(CfmlError::new(
            format!(
                "org.owasp.validator.html.CleanResults.{}() is not supported: RustCFML's \
                 sanitiser does not record per-change messages, and reporting zero would \
                 falsely imply nothing was removed",
                method
            ),
            CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
        )),
        other => Err(unsupported("CleanResults", other)),
    }
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "org.owasp.validator.html.{}.{}() is not supported by RustCFML's AntiSamy shim",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}
