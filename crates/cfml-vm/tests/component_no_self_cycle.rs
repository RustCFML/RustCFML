//! Regression coverage for the per-request Application.cfc reference-cycle leak.
//!
//! When an Application.cfc is loaded, its component instance is visible in its
//! own body locals under the component's (often auto-generated, e.g.
//! "Anonymous") name. The VM stores the component body's data variables back
//! ON the template as `__variables`. If that self-alias is copied into
//! `__variables`, the template ends up owning a pointer to itself
//! (`template → __variables → <self-alias> → template`): an `Arc` reference
//! cycle that is never freed when the per-request VM drops. Under serve mode
//! that is a steady per-request heap leak (every request re-instantiates
//! Application.cfc), confirmed via a dhat heap profile that attributed ~16
//! still-live blocks per request to `load_application_cfc`.
//!
//! `load_application_cfc` now skips any body local that aliases the component
//! itself when building `__variables`. This test asserts the loaded component
//! graph contains no reference cycle.

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

fn fixture() -> HashMap<String, Vec<u8>> {
    let mut files = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        include_str!("../../../tests/lifecycle/component_no_self_cycle/Application.cfc")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "index.cfm".to_string(),
        include_str!("../../../tests/lifecycle/component_no_self_cycle/index.cfm")
            .as_bytes()
            .to_vec(),
    );
    files
}

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

/// Depth-first walk that reports `true` if any struct/array reachable from `v`
/// is revisited while still ON the current recursion path — i.e. a genuine
/// reference cycle. Struct identity is the backing-store pointer.
fn has_cycle(v: &CfmlValue, on_path: &mut Vec<usize>) -> bool {
    match v {
        CfmlValue::Struct(s) => {
            let p = s.backing_ptr();
            if on_path.contains(&p) {
                return true;
            }
            on_path.push(p);
            let cyclic = s.snapshot().into_iter().any(|(_, val)| has_cycle(&val, on_path));
            on_path.pop();
            cyclic
        }
        CfmlValue::Array(a) => a.iter().any(|val| has_cycle(&val, on_path)),
        _ => false,
    }
}

#[test]
fn application_cfc_instance_has_no_self_reference_cycle() {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(fixture(), VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
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
    vm.server_state = Some(ServerState::with_production(false));

    vm.execute_with_lifecycle().unwrap();
    assert_eq!("ok", vm.output_buffer.trim());

    // The instantiated Application.cfc component is left in `globals` (under its
    // name or the auto-generated "Anonymous" key) as a Struct carrying __name.
    // Locate it and assert its object graph contains no reference cycle.
    let component = vm
        .globals
        .iter()
        .find(|(_, v)| match v {
            CfmlValue::Struct(s) => s.snapshot().iter().any(|(k, _)| k == "__name"),
            _ => false,
        })
        .map(|(_, v)| v.clone())
        .expect("Application.cfc component should be present in globals after load");

    let mut on_path = Vec::new();
    assert!(
        !has_cycle(&component, &mut on_path),
        "loaded Application.cfc component must not contain a reference cycle \
         (regression: the component's self-alias was copied into __variables, \
         forming template → __variables → self and leaking the instance every request)"
    );
}
