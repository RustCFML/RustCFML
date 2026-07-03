//! The Application.cfc pseudo-constructor must run against a live, writable
//! transient `application` scope — matching Lucee. A write inside the PC
//! (`application.x = v`) must succeed and be readable back within the same PC
//! (the standard guard-set-then-return idiom, e.g. Preside's
//! `_getDefaultStatelessUrlPatterns`), even though `this.name` / the real named
//! scope is not yet bound. Those PC-local writes are DISCARDED once the named
//! scope binds for the page body — Lucee loses them too (cross-engine verified).
//!
//! Regression guard for the "app-scope write is a silent no-op during the PC"
//! bug: before the fix the write vanished and the read-back returned Null
//! (later throwing "Variable is undefined" once undefined member reads were made
//! to throw), which broke Preside boot.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use std::collections::HashMap;
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

fn run_request(server_state: &ServerState, vfs: Arc<dyn Vfs>, page: &str) -> String {
    let page_path = format!("{}/{}", VROOT, page);
    let program = compile_page(&vfs, &page_path);

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
    for scope in ["cgi", "url", "form"] {
        vm.globals
            .entry(scope.to_string())
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    }
    vm.server_state = Some(server_state.clone());
    vm.execute_with_lifecycle().unwrap();
    vm.output_buffer.trim().to_string()
}

#[test]
fn pseudo_constructor_has_writable_transient_application_scope() {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        include_str!("../../../tests/lifecycle/application_pc_appscope/Application.cfc")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "index.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_pc_appscope/index.cfm")
            .as_bytes()
            .to_vec(),
    );

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    let out = run_request(&server_state, vfs.clone(), "index.cfm");
    assert_eq!(
        out, "written-in-pc|SKE-TRUE|built|NOT-PERSISTED",
        "PC must read back its own application-scope write (and Preside's guard-set-return \
         must return the built value); those writes must NOT persist into the page body"
    );

    // A second (warm) request must behave identically — the PC always gets a
    // fresh transient scope, so the guard runs and rebuilds every request, and
    // the prior request's PC writes never leaked into the persisted scope.
    let out2 = run_request(&server_state, vfs, "index.cfm");
    assert_eq!(
        out2, "written-in-pc|SKE-TRUE|built|NOT-PERSISTED",
        "warm request: PC transient scope behaves identically and stays isolated from \
         the persisted application scope"
    );
}
