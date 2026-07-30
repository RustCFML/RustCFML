//! The Application.cfc pseudo-constructor must run against a live, writable
//! `application` scope — matching Lucee. Three distinct properties, all verified
//! cross-engine against Lucee 7.0.4.34:
//!
//! 1. A write inside the PC (`application.x = v`) succeeds and is readable back
//!    within the same PC (the guard-set-then-return idiom, e.g. Preside's
//!    `_getDefaultStatelessUrlPatterns`), even though `this.name` / the real named
//!    scope is not yet bound.
//! 2. Those PC writes are NOT visible to the page body, which sees the named
//!    scope bound after the PC completes. (Lucee: page sees nothing either.)
//! 3. The PC scope PERSISTS ACROSS REQUESTS — the next request's PC still sees
//!    what the previous request's PC wrote, so a guard-once block runs exactly
//!    once. On Lucee this is the pre-`this.name` default application scope; a PC
//!    counter increments 1,2,3,4 across requests there.
//!
//! Regression guard for two bugs:
//!   - "app-scope write is a silent no-op during the PC": the write vanished and
//!     the read-back returned Null (later throwing "Variable is undefined"),
//!     breaking Preside boot. (Property 1.)
//!   - "PC got a FRESH EMPTY scope every request": every PC guard-once block
//!     re-ran forever. On a real Preside site this made
//!     `Bootstrap._setupCustomTagPaths`'s recursive `DirectoryList` over every
//!     extension run on EVERY request — a warm page went 35ms → 204ms.
//!     (Property 3.)

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

    // First (cold) request: nothing inherited, so the guard-once branch runs.
    let out = run_request(&server_state, vfs.clone(), "index.cfm");
    assert_eq!(
        out, "written-in-pc|SKE-TRUE|built|NOT-PERSISTED|NO-PREV|GUARD-RAN",
        "cold request: PC must read back its own application-scope write (and Preside's \
         guard-set-return must return the built value); the write must NOT be visible to \
         the page body; with an empty scope the guard-once branch must execute"
    );

    // Second (warm) request: the PC scope persists, so the PC sees the previous
    // request's write and the guard-once branch is SKIPPED. This is the property
    // that keeps Preside's per-request extension walk from re-running forever.
    let out2 = run_request(&server_state, vfs.clone(), "index.cfm");
    assert_eq!(
        out2, "written-in-pc|SKE-TRUE|built|NOT-PERSISTED|SAW-PREV|GUARD-SKIPPED",
        "warm request: the PC application scope must persist across requests (Lucee \
         parity) so a guard-once block runs exactly once — while still staying invisible \
         to the page body"
    );

    // Third request: still skipped — persistence is stable, not a one-shot.
    let out3 = run_request(&server_state, vfs, "index.cfm");
    assert_eq!(
        out3, "written-in-pc|SKE-TRUE|built|NOT-PERSISTED|SAW-PREV|GUARD-SKIPPED",
        "third request: PC scope persistence must be stable across many requests"
    );
}
