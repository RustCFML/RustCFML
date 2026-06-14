//! Regression: `<cfcookie expires=...>` must render the `Set-Cookie` header's
//! `Expires=` as an RFC-1123 / cookie-date GMT string (Lucee parity).
//!
//! `cfcookie` accepts `expires` as a date, a number of days, "now", or "never".
//! Lucee renders all of them as a proper cookie date, e.g.
//!   Set-Cookie: deviceid=…; Expires=Tue, 14-Jul-2026 06:30:57 GMT; HttpOnly
//! which browsers parse and persist.
//!
//! RustCFML currently pushes the raw value straight into the header
//! (crates/cfml-vm/src/lib.rs `__cfcookie` → `format!("; Expires={}", expires.as_string())`),
//! so a date renders as `Expires=2026-12-31 00:00:00` and a day count as
//! `Expires=30`. Per RFC 6265 the cookie-date parser needs a *month name* (and
//! GMT); a numeric/`30` value fails to parse, so the browser treats the cookie
//! as a SESSION cookie — it is dropped on browser close. Real-world hit: a
//! "keep me signed in for 30 days" remember-me cookie that doesn't survive a
//! browser restart.
//!
//! Verified against Lucee 7: the same `cfcookie expires="<date>"` renders
//! `Expires=Tue, 14-Jul-2026 06:30:57 GMT`.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionStore};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

const APP: &str = r##"
component {
    this.name = "cfcookie-expires-test";
    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

/// Execute a CFML page and return the `Set-Cookie` header values it emitted.
fn set_cookies(page: &str) -> Vec<String> {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), APP.as_bytes().to_vec());
    files.insert("index.cfm".to_string(), page.as_bytes().to_vec());
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));

    let page_path = format!("{}/index.cfm", VROOT);
    let source = vfs.read_to_string(&page_path).unwrap();
    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };
    let ast = Parser::new(processed).parse().unwrap();
    let program = CfmlCompiler::new().compile(ast);

    let mut server_state = ServerState::with_production(false);
    server_state.sessions = Arc::new(MemoryStore::new()) as Arc<dyn SessionStore>;

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    for scope in ["url", "cgi", "form"] {
        vm.globals
            .entry(scope.to_string())
            .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));
    }
    vm.server_state = Some(server_state);
    vm.session_id = Some("sid-cookie".to_string());

    let _ = vm.execute_with_lifecycle();

    vm.response_headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .collect()
}

/// The `Expires=` segment of the Set-Cookie for `name`, if present.
fn expires_of(cookies: &[String], name: &str) -> Option<String> {
    let prefix = format!("{}=", name);
    let line = cookies.iter().find(|c| c.starts_with(&prefix))?;
    line.split(';')
        .map(|s| s.trim())
        .find_map(|seg| seg.strip_prefix("Expires=").map(|s| s.to_string()))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A browser-parseable cookie date must carry a month NAME and GMT (RFC 1123 /
/// RFC 6265 sane-cookie-date) — not a numeric "2026-12-31" or a bare "30".
fn is_cookie_date(v: &str) -> bool {
    v.contains("GMT") && MONTHS.iter().any(|m| v.contains(m))
}

#[test]
fn cfcookie_expires_date_renders_rfc1123() {
    let cookies = set_cookies(
        r##"<cfcookie name="c_date" value="v" expires="#createDateTime(2026, 12, 31, 1, 2, 3)#" httponly="true">"##,
    );
    let exp = expires_of(&cookies, "c_date")
        .expect("c_date cookie must carry an Expires");
    assert!(
        is_cookie_date(&exp),
        "a date `expires` must render as an RFC-1123 GMT cookie date, got: Expires={}",
        exp
    );
}

#[test]
fn cfcookie_expires_daycount_renders_future_date() {
    let cookies = set_cookies(r#"<cfcookie name="c_days" value="v" expires="30">"#);
    let exp = expires_of(&cookies, "c_days")
        .expect("c_days cookie must carry an Expires");
    assert!(
        is_cookie_date(&exp),
        "a numeric `expires` (days) must render as a future RFC-1123 GMT cookie date, got: Expires={}",
        exp
    );
}

#[test]
fn cfcookie_expires_never_renders_far_future_date() {
    let cookies = set_cookies(r#"<cfcookie name="c_never" value="v" expires="never">"#);
    let exp = expires_of(&cookies, "c_never")
        .expect("c_never cookie must carry an Expires");
    assert!(
        is_cookie_date(&exp),
        "`expires=never` must render as a far-future RFC-1123 GMT cookie date, got: Expires={}",
        exp
    );
}
