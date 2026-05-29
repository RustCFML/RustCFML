//! Regression coverage for complex arguments passed through property paths.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

fn run_page(source: &str, application_scope: Option<IndexMap<String, CfmlValue>>) -> String {
    let mut files = HashMap::new();
    files.insert("index.cfm".to_string(), source.as_bytes().to_vec());

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    vm.application_scope = application_scope.map(|scope| Arc::new(Mutex::new(scope)));

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
fn nested_struct_argument_mutation_writes_back_to_caller_path() {
    let output = run_page(
        r##"
<cffunction name="mutate">
    <cfargument name="target" />
    <cfset arguments.target.foo = "bar" />
</cffunction>
<cfset holder = { child = {} } />
<cfset mutate(holder.child) />
<cfoutput>#holder.child.foo ?: "missing"#</cfoutput>
"##,
        None,
    );

    assert_eq!("bar", output.trim());
}

#[test]
fn application_scope_argument_mutation_writes_back_to_nested_scope_path() {
    let output = run_page(
        r##"
<cffunction name="mutate">
    <cfargument name="target" />
    <cfset arguments.target.foo = "bar" />
</cffunction>
<cfset application.service = {} />
<cfset mutate(application.service) />
<cfoutput>#application.service.foo ?: "missing"#</cfoutput>
"##,
        Some(IndexMap::new()),
    );

    assert_eq!("bar", output.trim());
}

#[test]
fn assigned_struct_reference_observes_later_nested_mutation() {
    let output = run_page(
        r##"
<cfset original = { child = { value = "before" } } />
<cfset alias = original.child />
<cfset original.child.value = "after" />
<cfoutput>#alias.value#</cfoutput>
"##,
        None,
    );

    assert_eq!("after", output.trim());
}

#[test]
fn nested_aliases_survive_complex_argument_writeback() {
    let output = run_page(
        r##"
<cffunction name="enrich">
    <cfargument name="schema" />
    <cfset arguments.schema.route.fields.profiles.generated = "yes" />
</cffunction>

<cfset input = {
    route = {
        fields = {
            profiles = {}
        }
    }
} />
<cfset out = {} />
<cfset out.route = input.route />
<cfset enrich(input) />
<cfoutput>#input.route.fields.profiles.generated ?: "missing"#|#out.route.fields.profiles.generated ?: "missing"#</cfoutput>
"##,
        None,
    );

    assert_eq!("yes|yes", output.trim());
}
