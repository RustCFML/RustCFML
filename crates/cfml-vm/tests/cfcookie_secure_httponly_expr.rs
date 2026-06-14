//! Regression: `<cfcookie secure="#expr#">` / `httponly="#expr#"` must EVALUATE
//! the expression (Lucee parity), not literal-match the attribute text.
//!
//! Every CFML tag attribute supports `#expr#` interpolation. RustCFML's cfcookie
//! tag parser special-cases `secure`/`httponly`: it compares the RAW attribute
//! string to "true"/"yes" at parse time
//! (crates/cfml-compiler/src/tag_parser.rs, the `"cfcookie"` arm), so an
//! expression like `secure="#true#"` or `secure="#myBool#"` is seen as the
//! literal text `#true#` → not "true"/"yes" → emitted as `false`. The flag is
//! therefore ALWAYS dropped when set via an expression; only a literal
//! `secure="true"` works.
//!
//! Real-world hit: an app that sets the Secure flag dynamically — e.g. from the
//! request scheme / a config flag, `secure="#isHttps#"` — silently ships
//! non-Secure cookies on RustCFML while working on Lucee.
//!
//! Verified against Lucee 7: `cfcookie ... secure="#expr#"` evaluates the
//! expression and emits `; Secure` when it is true.

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
    this.name = "cfcookie-secure-expr-test";
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

fn cookie(cookies: &[String], name: &str) -> String {
    let prefix = format!("{}=", name);
    cookies
        .iter()
        .find(|c| c.starts_with(&prefix))
        .cloned()
        .unwrap_or_default()
}

fn has_attr(cookie_line: &str, attr: &str) -> bool {
    cookie_line
        .split(';')
        .any(|seg| seg.trim().eq_ignore_ascii_case(attr))
}

#[test]
fn cfcookie_secure_literal_is_emitted_control() {
    // Control: a literal secure="true" already works today.
    let cookies = set_cookies(r#"<cfcookie name="lit" value="v" secure="true" httponly="true">"#);
    let c = cookie(&cookies, "lit");
    assert!(has_attr(&c, "Secure"), "literal secure=\"true\" should emit Secure, got: {}", c);
    assert!(has_attr(&c, "HttpOnly"), "literal httponly=\"true\" should emit HttpOnly, got: {}", c);
}

#[test]
fn cfcookie_secure_expression_is_evaluated() {
    let cookies = set_cookies(r##"<cfcookie name="exp" value="v" secure="#(1 EQ 1)#">"##);
    let c = cookie(&cookies, "exp");
    assert!(
        has_attr(&c, "Secure"),
        "secure=\"#(1 EQ 1)#\" must evaluate to true and emit Secure, got: {}",
        c
    );
}

#[test]
fn cfcookie_httponly_expression_is_evaluated() {
    let cookies = set_cookies(r##"<cfcookie name="exph" value="v" httponly="#(1 EQ 1)#">"##);
    let c = cookie(&cookies, "exph");
    assert!(
        has_attr(&c, "HttpOnly"),
        "httponly=\"#(1 EQ 1)#\" must evaluate to true and emit HttpOnly, got: {}",
        c
    );
}

#[test]
fn cfcookie_secure_variable_expression_is_evaluated() {
    let cookies = set_cookies(
        r##"<cfset isSecure = true><cfcookie name="var" value="v" secure="#isSecure#">"##,
    );
    let c = cookie(&cookies, "var");
    assert!(
        has_attr(&c, "Secure"),
        "secure=\"#isSecure#\" (a boolean var) must emit Secure, got: {}",
        c
    );
}
