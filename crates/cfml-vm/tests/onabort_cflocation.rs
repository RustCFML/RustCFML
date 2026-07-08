//! Regression: a `cflocation`/`location()` redirect issued from
//! `Application.cfc` `onRequestStart` must fire `onAbort` (like `cfabort`
//! does), not silently end the request.
//!
//! Frameworks that run the whole request inside `onRequestStart` and defer
//! teardown to `onAbort` (Preside's ColdBox bootstrap) persist their session
//! and set the session cookie in `onAbort`. Before the fix, a redirect during
//! request processing — e.g. the post-login relocate — returned without firing
//! `onAbort`, so the session cookie was never written and the user could not
//! stay logged in.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const VROOT: &str = "/app";

fn compile_page(vfs: &Arc<dyn Vfs>, path: &str) -> BytecodeProgram {
    let source = vfs.read_to_string(path).unwrap();
    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };
    let ast = Parser::new(processed).parse().unwrap();
    CfmlCompiler::new().compile(ast)
}

#[test]
fn cflocation_in_on_request_start_fires_on_abort() {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        r##"
component {
    this.name = "onabort-cflocation-test";

    function onRequestStart(targetPage) {
        // Process the request, then redirect — exactly the shape of a
        // framework that relocates from within onRequestStart.
        location(url="/dashboard.cfm", addToken=false);
    }

    function onAbort(targetPage) {
        // A framework persists its session (and sets the session cookie) here.
        cookie name="onabort_cookie" value="fired";
    }

    function onRequestEnd(targetPage) {
        cookie name="onrequestend_cookie" value="fired";
    }
}
"##
        .as_bytes()
        .to_vec(),
    );
    files.insert("index.cfm".to_string(), b"<cfset ok = true>".to_vec());

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut server_state = ServerState::with_production(false);
    server_state.webroot = Some(PathBuf::from(VROOT));

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    vm.server_state = Some(server_state);

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

    vm.execute_with_lifecycle().unwrap();

    let cookies: Vec<String> = vm
        .response_headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Set-Cookie"))
        .map(|(_, v)| v.clone())
        .collect();
    let joined = cookies.join(" ; ");

    // onAbort must have fired — its cookie must be on the response.
    assert!(
        joined.contains("onabort_cookie=fired"),
        "onAbort did not fire on a cflocation redirect from onRequestStart; Set-Cookie headers: {joined:?}"
    );
    // onRequestEnd must NOT fire on an abort (the request was redirected).
    assert!(
        !joined.contains("onrequestend_cookie"),
        "onRequestEnd wrongly fired on a cflocation abort; Set-Cookie headers: {joined:?}"
    );
    // The redirect itself must be intact.
    let location = vm
        .response_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Location"))
        .map(|(_, v)| v.clone());
    assert_eq!(location.as_deref(), Some("/dashboard.cfm"));
}
