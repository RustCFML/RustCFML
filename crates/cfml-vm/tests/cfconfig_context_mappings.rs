//! GH #305 — `.cfconfig.json` `mappings` / `customTagPaths` must apply to a
//! request that resolves NO `Application.cfc`.
//!
//! Both used to be seeded only inside the Application.cfc load path, so two
//! byte-identical directories differing only in the presence of an
//! `Application.cfc` resolved `/lib` — and `<cf_greet>` — differently, with no
//! warning and no error. Lucee applies config mappings unconditionally: they are
//! a context-level facility, not an application-level one.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_config::RustCfmlConfig;
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

/// Run `web/t.cfm` against a cfconfig baseline carrying a `/lib` mapping and a
/// custom-tag path. `with_app_cfc` controls the only difference between the two
/// arrangements: whether an `Application.cfc` sits beside the page.
fn run(page: &str, with_app_cfc: bool) -> String {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("web/t.cfm".to_string(), page.as_bytes().to_vec());
    files.insert("reallib/marker.cfm".to_string(), b"lib".to_vec());
    files.insert(
        "tags/greet.cfm".to_string(),
        b"<cfoutput>CUSTOMTAG_OK</cfoutput>".to_vec(),
    );
    if with_app_cfc {
        files.insert(
            "web/Application.cfc".to_string(),
            b"component { this.name = \"gh305\"; }".to_vec(),
        );
    }

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/web/t.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut cfg = RustCfmlConfig::default();
    cfg.mappings
        .insert("/lib".to_string(), format!("{}/reallib", VROOT));
    cfg.custom_tag_paths.push(format!("{}/tags", VROOT));

    let mut server_state = ServerState::with_production(false);
    server_state.webroot = Some(PathBuf::from(VROOT));
    server_state.cfconfig = Arc::new(cfg);

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

    vm.execute_with_lifecycle().unwrap();
    vm.get_output().trim().to_string()
}

#[test]
fn cfconfig_mapping_applies_without_application_cfc() {
    assert_eq!(
        format!("{}/reallib/marker.cfm", VROOT),
        run(r##"<cfoutput>#expandPath("/lib/marker.cfm")#</cfoutput>"##, false)
    );
}

#[test]
fn cfconfig_mapping_still_applies_with_application_cfc() {
    assert_eq!(
        format!("{}/reallib/marker.cfm", VROOT),
        run(r##"<cfoutput>#expandPath("/lib/marker.cfm")#</cfoutput>"##, true)
    );
}

#[test]
fn cfconfig_custom_tag_path_applies_without_application_cfc() {
    assert_eq!("CUSTOMTAG_OK", run("<cf_greet>", false));
}

#[test]
fn cfconfig_custom_tag_path_still_applies_with_application_cfc() {
    assert_eq!("CUSTOMTAG_OK", run("<cf_greet>", true));
}
