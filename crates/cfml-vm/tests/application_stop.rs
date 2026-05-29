//! End-to-end coverage for `applicationStop()`.
//!
//! The function has to mutate the VM's shared application store, so this test
//! drives real lifecycle execution instead of testing the stdlib shim.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";
const APP_NAME: &str = "application-stop-test";

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

fn run_request(server_state: &ServerState, vfs: Arc<dyn Vfs>, page: &str) {
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
    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));
    vm.globals
        .entry("cgi".to_string())
        .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));
    vm.globals
        .entry("form".to_string())
        .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));

    vm.server_state = Some(server_state.clone());

    vm.execute_with_lifecycle().unwrap();
}

#[test]
fn application_stop_clears_shared_application_state() {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/Application.cfc")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "index.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/index.cfm")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "stop.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/stop.cfm")
            .as_bytes()
            .to_vec(),
    );

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    run_request(&server_state, vfs.clone(), "index.cfm");
    let started_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(started_app.started);
    assert!(started_app
        .variables
        .keys()
        .any(|key| key.eq_ignore_ascii_case("seed")));

    run_request(&server_state, vfs.clone(), "stop.cfm");
    let stopped_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(
        !stopped_app.started,
        "applicationStop() must mark the application unstarted"
    );
    assert!(
        stopped_app.variables.is_empty(),
        "applicationStop() must clear application scope variables"
    );
    assert!(
        stopped_app.cached_functions.is_empty(),
        "applicationStop() must discard cached lifecycle functions"
    );
    assert_eq!(0, stopped_app.cached_functions_original_offset);

    run_request(&server_state, vfs, "index.cfm");
    let restarted_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(restarted_app.started);
    assert!(restarted_app
        .variables
        .keys()
        .any(|key| key.eq_ignore_ascii_case("seed")));
}
