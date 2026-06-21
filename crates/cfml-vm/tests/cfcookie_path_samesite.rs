//! Regression: `<cfcookie>` must emit Lucee-compatible `Path` and `SameSite`
//! cookie attributes.
//!
//! Lucee defaults omitted `path` to `/`, so a cookie set from `/easy/check.cfm`
//! is site-wide. RustCFML previously omitted `Path` entirely, leaving browsers
//! to scope the cookie to the request directory (`/easy`).
//!
//! Lucee also emits the `samesite` attribute supplied to `<cfcookie>`. RustCFML
//! previously ignored it in the VM-level `__cfcookie` handler.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionStore};
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

const APP: &str = r##"
component {
    this.name = "cfcookie-path-samesite-test";
    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

/// Execute a CFML page from a non-root path and return `Set-Cookie` headers.
fn set_cookies(page: &str) -> Vec<String> {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), APP.as_bytes().to_vec());
    files.insert("easy/check.cfm".to_string(), page.as_bytes().to_vec());
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));

    let page_path = format!("{}/easy/check.cfm", VROOT);
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
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
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

fn cookie(cookies: &[String], name: &str) -> String {
    let prefix = format!("{}=", name);
    cookies
        .iter()
        .find(|c| c.starts_with(&prefix))
        .cloned()
        .unwrap_or_default()
}

fn attr_value(cookie_line: &str, attr_name: &str) -> Option<String> {
    let prefix = format!("{}=", attr_name).to_ascii_lowercase();
    cookie_line.split(';').find_map(|segment| {
        let trimmed = segment.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with(&prefix) {
            Some(trimmed[prefix.len()..].to_string())
        } else {
            None
        }
    })
}

#[test]
fn cfcookie_default_path_is_root() {
    let cookies = set_cookies(r#"<cfcookie name="root_path" value="v">"#);
    let c = cookie(&cookies, "root_path");
    assert_eq!(
        attr_value(&c, "Path").as_deref(),
        Some("/"),
        "omitted path must default to Path=/, got: {}",
        c
    );
}

#[test]
fn cfcookie_explicit_path_wins() {
    let cookies = set_cookies(r#"<cfcookie name="custom_path" value="v" path="/custom">"#);
    let c = cookie(&cookies, "custom_path");
    assert_eq!(
        attr_value(&c, "Path").as_deref(),
        Some("/custom"),
        "explicit path must be preserved, got: {}",
        c
    );
}

#[test]
fn cfcookie_samesite_lax_is_emitted() {
    let cookies = set_cookies(r#"<cfcookie name="same_lax" value="v" samesite="Lax">"#);
    let c = cookie(&cookies, "same_lax");
    assert_eq!(
        attr_value(&c, "SameSite").as_deref(),
        Some("Lax"),
        "samesite=\"Lax\" must emit SameSite=Lax, got: {}",
        c
    );
}

#[test]
fn cfcookie_samesite_strict_and_none_are_emitted() {
    let cookies = set_cookies(
        r#"
        <cfcookie name="same_strict" value="v" samesite="Strict">
        <cfcookie name="same_none" value="v" samesite="None" secure="true">
        "#,
    );

    let strict = cookie(&cookies, "same_strict");
    assert_eq!(
        attr_value(&strict, "SameSite").as_deref(),
        Some("Strict"),
        "samesite=\"Strict\" must emit SameSite=Strict, got: {}",
        strict
    );

    let none = cookie(&cookies, "same_none");
    assert_eq!(
        attr_value(&none, "SameSite").as_deref(),
        Some("None"),
        "samesite=\"None\" must emit SameSite=None, got: {}",
        none
    );
}

#[test]
fn cfcookie_samesite_expression_is_evaluated() {
    let cookies = set_cookies(
        r##"
        <cfset sameSitePolicy = "Lax">
        <cfcookie name="same_expr" value="v" samesite="#sameSitePolicy#">
        "##,
    );
    let c = cookie(&cookies, "same_expr");
    assert_eq!(
        attr_value(&c, "SameSite").as_deref(),
        Some("Lax"),
        "samesite=\"#sameSitePolicy#\" must evaluate and emit SameSite=Lax, got: {}",
        c
    );
}

#[test]
fn cfcookie_omitted_samesite_does_not_emit_attribute() {
    let cookies = set_cookies(r#"<cfcookie name="same_omitted" value="v">"#);
    let c = cookie(&cookies, "same_omitted");
    assert!(
        attr_value(&c, "SameSite").is_none(),
        "omitted samesite must not invent a SameSite attribute, got: {}",
        c
    );
}
