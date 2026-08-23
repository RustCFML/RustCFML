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

fn build(index: &str, cfc_name: &str, cfc_body: &str) -> CfmlVirtualMachine {
    let mut files = HashMap::new();
    files.insert("index.cfm".to_string(), index.as_bytes().to_vec());
    files.insert(
        format!("lib/{}.cfc", cfc_name),
        cfc_body.as_bytes().to_vec(),
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
        from_application: true,
    }];
    vm.refresh_mappings_fingerprint();
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm
}

fn run_page(index: &str, cfc_name: &str, cfc_body: &str) -> String {
    let mut vm = build(index, cfc_name, cfc_body);
    vm.execute().unwrap();
    vm.get_output().split_whitespace().collect::<String>()
}

#[test]
fn component_method_names_take_precedence_over_struct_member_helpers() {
    let output = run_page(
        r##"
<cfset service = CreateObject("component", "/lib/service") />
<cfoutput>#service.delete(id="abc")#|#service.count()#</cfoutput>
"##,
        "service",
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
"#,
    );

    assert_eq!("deleted:abc|component-count", output);
}

#[test]
fn struct_helpers_never_shadow_on_missing_method_on_components() {
    // A helper-named call (count/delete) with no matching method must reach
    // onMissingMethod rather than dispatching to structCount/structDelete.
    let output = run_page(
        r##"
<cfset service = CreateObject("component", "/lib/service") />
<cfoutput>#service.count()#|#service.delete("x")#</cfoutput>
"##,
        "service",
        r#"
<cfcomponent>
    <cffunction name="onMissingMethod">
        <cfargument name="missingMethodName" />
        <cfargument name="missingMethodArguments" />
        <cfreturn "missing:" & arguments.missingMethodName />
    </cffunction>
</cfcomponent>
"#,
    );

    assert_eq!("missing:count|missing:delete", output);
}

#[test]
fn undefined_component_method_throws_instead_of_struct_helper_or_null() {
    // No matching method, no onMissingMethod: a helper-named call must error
    // (matching Lucee) rather than fall back to structKeyExists or return null.
    let mut vm = build(
        r##"
<cfset service = CreateObject("component", "/lib/service") />
<cfoutput>#service.keyExists("x")#</cfoutput>
"##,
        "service",
        r#"
<cfcomponent>
    <cffunction name="ping"><cfreturn "pong" /></cffunction>
</cfcomponent>
"#,
    );
    let err = vm
        .execute()
        .expect_err("undefined helper-named component method should error");
    assert!(
        err.message.contains("has no function with name"),
        "unexpected error message: {}",
        err.message
    );
}
