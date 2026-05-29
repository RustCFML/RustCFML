//! Regression coverage for Application.cfc lifecycle target-page arguments.
//!
//! Requests are executed from physical source paths, but CFML lifecycle
//! methods receive web-root-relative paths such as `/_moopa.cfm`.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use indexmap::IndexMap;
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
fn request_lifecycle_methods_receive_webroot_relative_target_page() {
    let output = run_lifecycle_target_page_request(format!("{}/_moopa.cfm", VROOT));

    assert_eq!(
        "start=/_moopa.cfm|request=/_moopa.cfm|end=/_moopa.cfm",
        output
    );
}

#[test]
fn lifecycle_target_page_canonicalizes_source_before_stripping_webroot() {
    let output = run_lifecycle_target_page_request("_moopa.cfm");

    assert_eq!(
        "start=/_moopa.cfm|request=/_moopa.cfm|end=/_moopa.cfm",
        output
    );
}

fn run_lifecycle_target_page_request(source_file: impl Into<String>) -> String {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        r##"
component {
    this.name = "lifecycle-target-page-test";

    function onRequestStart(targetPage) {
        writeOutput("start=" & arguments.targetPage & "|");
    }

    function onRequest(targetPage) {
        writeOutput("request=" & arguments.targetPage);
    }

    function onRequestEnd(targetPage) {
        writeOutput("|end=" & arguments.targetPage);
    }
}
"##
        .as_bytes()
        .to_vec(),
    );
    files.insert("_moopa.cfm".to_string(), b"<cfset ok = true>".to_vec());

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/_moopa.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut server_state = ServerState::with_production(false);
    server_state.webroot = Some(PathBuf::from(VROOT));

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(source_file.into());
    vm.base_template_path = Some(page_path);
    vm.server_state = Some(server_state);

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

    vm.execute_with_lifecycle().unwrap();

    vm.output_buffer
}
