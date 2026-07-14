//! `isDefined("<lifecycleMethod>")` inside an Application.cfc lifecycle method
//! must return true when the handler was attached to the component via
//! `include` (Mura/Masa's `<cfinclude template="onApplicationStart_method.cfm">`
//! idiom) — exactly as it does on Lucee.
//!
//! Regression guard for the Masa CMS boot blocker: the include-attached
//! lifecycle methods were attached as keys ON the component struct (so lifecycle
//! DISPATCH worked) but were filtered out of the component's `__variables` scope,
//! so `isDefined("onApplicationStart")` — the exact guard Mura/Masa use in
//! onRequestStart to decide whether to run onApplicationStart / render the setup
//! wizard — returned false. That silently skipped the whole init+setup block and
//! fell through to un-built managers. Component methods must be visible in the
//! `variables` scope, matching a normal component.

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
fn isdefined_sees_include_attached_lifecycle_method() {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        include_str!("../../../tests/lifecycle/application_isdefined_lifecycle/Application.cfc")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "onApplicationStart_method.cfm".to_string(),
        include_str!(
            "../../../tests/lifecycle/application_isdefined_lifecycle/onApplicationStart_method.cfm"
        )
        .as_bytes()
        .to_vec(),
    );
    files.insert(
        "onRequestStart_method.cfm".to_string(),
        include_str!(
            "../../../tests/lifecycle/application_isdefined_lifecycle/onRequestStart_method.cfm"
        )
        .as_bytes()
        .to_vec(),
    );
    files.insert(
        "index.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_isdefined_lifecycle/index.cfm")
            .as_bytes()
            .to_vec(),
    );

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    let out = run_request(&server_state, vfs, "index.cfm");
    assert_eq!(
        out, "onAppStart=true|viaVariables=true|booted=true",
        "isDefined() inside onRequestStart must see the include-attached \
         onApplicationStart handler (bare and via variables.), and \
         onApplicationStart must have fired"
    );
}
