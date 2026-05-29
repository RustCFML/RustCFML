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

#[test]
fn component_method_names_take_precedence_over_struct_member_helpers() {
    let mut files = HashMap::new();
    files.insert(
        "index.cfm".to_string(),
        r##"
<cfset service = CreateObject("component", "/lib/service") />
<cfoutput>#service.delete(id="abc")#|#service.count()#</cfoutput>
"##
        .as_bytes()
        .to_vec(),
    );
    files.insert(
        "lib/service.cfc".to_string(),
        r#"
<cfcomponent>
    <cffunction name="delete">
        <cfargument name="id" required="true" />
        <cfreturn "deleted:" & arguments.id />
    </cffunction>

    <cffunction name="count">
        <cfreturn "component-count" />
    </cffunction>
</cfcomponent>
"#
        .as_bytes()
        .to_vec(),
    );

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
    }];
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }

    vm.execute().unwrap();
    assert_eq!(
        "deleted:abc|component-count",
        vm.get_output().split_whitespace().collect::<String>()
    );
}
