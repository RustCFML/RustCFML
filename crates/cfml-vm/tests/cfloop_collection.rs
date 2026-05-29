//! Regression coverage for Lucee-style cfloop collection item/index behavior.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
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

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
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

    vm.execute().unwrap();
    vm.get_output()
}

#[test]
fn collection_loop_with_item_and_index_exposes_value_and_key() {
    let output = run_page(
        r##"
<cfset tables = {
    route = {
        fields = {
            profiles = {}
        }
    }
} />
<cfloop collection="#tables#" item="table" index="tableName">
    <cfset table.visited = tableName />
</cfloop>
<cfoutput>#tables.route.visited ?: "missing"#</cfoutput>
"##,
    );

    assert_eq!("route", output.trim());
}

#[test]
fn collection_loop_item_mutation_writes_back_to_collection() {
    let output = run_page(
        r##"
<cfset schema = {
    route = {
        fields = {
            profiles = {}
        }
    }
} />
<cfloop collection="#schema.route.fields#" item="field" index="fieldName">
    <cfset field.generated = fieldName />
</cfloop>
<cfoutput>#schema.route.fields.profiles.generated ?: "missing"#</cfoutput>
"##,
    );

    assert_eq!("profiles", output.trim());
}
