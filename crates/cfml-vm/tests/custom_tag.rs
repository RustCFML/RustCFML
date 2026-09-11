//! Custom tag lifecycle behavior.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
use indexmap::IndexMap;
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

fn run_page(files: HashMap<String, Vec<u8>>) -> String {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(std::sync::Arc::from(page_path.as_str()));
    vm.base_template_path = Some(page_path);
    vm.custom_tag_paths = vec![format!("{}/tags", VROOT)];
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("cgi".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("form".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    vm.execute().unwrap();
    vm.get_output()
}

fn fixture(entries: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(path, source)| (path.to_string(), source.as_bytes().to_vec()))
        .collect()
}

#[test]
fn self_closing_custom_tag_runs_end_phase_with_start_locals() {
    let files = fixture(&[
        (
            "index.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/self_closing_end_phase/index.cfm"),
        ),
        (
            "tags/capture.cfm",
            include_str!(
                "../../../tests/lifecycle/custom_tag/self_closing_end_phase/tags/capture.cfm"
            ),
        ),
    ]);

    assert_eq!("ok", run_page(files).trim());
}

#[test]
fn self_closing_custom_tag_reports_has_end_tag() {
    let files = fixture(&[
        (
            "index.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/self_closing_has_end_tag/index.cfm"),
        ),
        (
            "tags/requires_end.cfm",
            include_str!(
                "../../../tests/lifecycle/custom_tag/self_closing_has_end_tag/tags/requires_end.cfm"
            ),
        ),
    ]);

    assert_eq!("true", run_page(files).trim());
}

#[test]
fn body_custom_tag_end_phase_keeps_start_locals() {
    let files = fixture(&[
        (
            "index.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/body_end_start_locals/index.cfm"),
        ),
        (
            "tags/wrap.cfm",
            include_str!(
                "../../../tests/lifecycle/custom_tag/body_end_start_locals/tags/wrap.cfm"
            ),
        ),
    ]);

    assert_eq!("start:body", run_page(files).trim());
}

#[test]
fn body_custom_tag_generated_content_precedes_end_phase_output() {
    let files = fixture(&[
        (
            "index.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/generated_content_order/index.cfm"),
        ),
        (
            "tags/layout.cfm",
            include_str!(
                "../../../tests/lifecycle/custom_tag/generated_content_order/tags/layout.cfm"
            ),
        ),
    ]);

    let output = run_page(files);
    let body_start = output.find("<body>").unwrap();
    let body_content = output.find("body").unwrap();
    let body_end = output.find("</body>").unwrap();

    assert!(body_start < body_content);
    assert!(body_content < body_end);
}

#[test]
fn custom_tag_template_has_local_scope() {
    let files = fixture(&[
        (
            "index.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/local_scope/index.cfm"),
        ),
        (
            "tags/localtest.cfm",
            include_str!("../../../tests/lifecycle/custom_tag/local_scope/tags/localtest.cfm"),
        ),
    ]);

    assert_eq!("ok", run_page(files).trim());
}
