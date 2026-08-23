use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlMapping, CfmlVirtualMachine};
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

fn run_page(source: &str) -> String {
    let mut files = HashMap::new();
    files.insert("index.cfm".to_string(), source.as_bytes().to_vec());
    files.insert(
        "lib/tables/person.cfc".to_string(),
        b"<cfcomponent></cfcomponent>".to_vec(),
    );
    files.insert(
        "lib/tables/nested/address.cfc".to_string(),
        b"<cfcomponent></cfcomponent>".to_vec(),
    );
    files.insert("lib/tables/readme.txt".to_string(), b"ignore".to_vec());

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    vm.mappings = vec![CfmlMapping {
        name: "/lib/".to_string(),
        path: format!("{}/lib", VROOT),
        from_application: true,
    }];
    vm.refresh_mappings_fingerprint();
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }

    vm.execute().unwrap();
    vm.get_output().trim().to_string()
}

#[test]
fn cfdirectory_resolves_leading_slash_mapping_path() {
    let output = run_page(
        r##"
<cfdirectory action="list" directory="/lib/tables" name="q" filter="*.cfc" recurse="true">
<cfoutput>#q.recordCount#</cfoutput>
"##,
    );

    assert_eq!("2", output);
}

#[test]
fn cfdirectory_keeps_existing_absolute_filesystem_path() {
    let output = run_page(
        r##"
<cfdirectory action="list" directory="/app/lib/tables" name="q" filter="*.cfc">
<cfoutput>#q.recordCount#</cfoutput>
"##,
    );

    assert_eq!("1", output);
}
