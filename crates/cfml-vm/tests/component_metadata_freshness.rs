//! `getComponentMetaData("dotted.path")` is memoized per REQUEST (per VM), never
//! across requests: an edited `.cfc` must be picked up by the next request in
//! dev mode. The metadata memo (`Vm::component_path_meta_cache` /
//! `component_inherit_meta_cache`) lives on the per-request VM exactly so that
//! stays true — this test fails loudly if either is ever promoted to a
//! cross-request (process/ServerState) cache without a freshness check.
//!
//! It also pins the in-request contract the memo must preserve: two calls in the
//! SAME request return equal-but-independent structs, so a caller mutating what
//! it was handed (ColdBox `Util.getInheritedMetaData` does) cannot poison the
//! next reader.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;

fn compile_page(source: &str) -> BytecodeProgram {
    let processed = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script(source)
    } else {
        source.to_string()
    };
    let ast = Parser::new(processed).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

/// One "request": a brand-new VM over the same on-disk page, exactly as the
/// serve loop builds one per request.
fn run_request(page_path: &str, source: &str) -> String {
    let mut vm = CfmlVirtualMachine::new(compile_page(source));
    vm.source_file = Some(page_path.to_string());
    vm.base_template_path = Some(page_path.to_string());
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.execute().expect("execute");
    vm.get_output().trim().to_string()
}

fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("write fixture");
}

#[test]
fn edited_component_metadata_is_visible_to_the_next_request() {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml_gcm_freshness_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("mkdir");

    let base = pkg.join("MetaBase.cfc");
    let child = pkg.join("MetaChild.cfc");
    write(
        &base,
        "component { public string function baseAlpha() { return \"a\"; } }",
    );
    write(
        &child,
        "component extends=\"pkg.MetaBase\" { public string function childOne() { return \"c\"; } }",
    );

    // The page prints: <own functions>|<parent name>|<parent functions>
    let page_path = dir.join("index.cfm");
    let page_src = r#"<cfscript>
function names( required array functions ) {
    local.out = [];
    for ( local.f in arguments.functions ) { arrayAppend( local.out, local.f.name ); }
    arraySort( local.out, "text" );
    return arrayToList( local.out );
}
md = getComponentMetaData( "pkg.MetaChild" );
writeOutput( names( md.functions ) & "|" & md.extends.name & "|" & names( md.extends.functions ) );
</cfscript>"#;
    write(&page_path, page_src);
    let page = page_path.to_string_lossy().to_string();

    assert_eq!(
        "childOne|pkg.MetaBase|baseAlpha",
        run_request(&page, page_src),
        "first request sees the original chain"
    );

    // Edit BOTH levels on disk and issue a fresh request. A per-request memo is
    // gone by now; a cross-request one would still answer with the stale shape.
    write(
        &base,
        "component { public string function baseBeta() { return \"b\"; } \
         public string function baseGamma() { return \"g\"; } }",
    );
    write(
        &child,
        "component extends=\"pkg.MetaBase\" { public string function childTwo() { return \"c2\"; } }",
    );

    assert_eq!(
        "childTwo|pkg.MetaBase|baseBeta,baseGamma",
        run_request(&page, page_src),
        "a fresh request must see the edited components (metadata memo is request-scoped)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repeated_calls_in_one_request_are_equal_but_independent() {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml_gcm_independent_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("mkdir");
    write(
        &pkg.join("IndepBase.cfc"),
        "component { public string function baseAlpha() { return \"a\"; } }",
    );
    write(
        &pkg.join("IndepChild.cfc"),
        "component extends=\"pkg.IndepBase\" { public string function childOne() { return \"c\"; } }",
    );

    let page_path = dir.join("index.cfm");
    let page_src = r#"<cfscript>
a = getComponentMetaData( "pkg.IndepChild" );
a.clobbered = "yes";
a.extends.name = "clobbered";
structDelete( a, "functions" );
b = getComponentMetaData( "pkg.IndepChild" );
writeOutput( ( structKeyExists( b, "clobbered" ) ? "LEAKED" : "clean" )
    & "|" & b.extends.name
    & "|" & arrayLen( b.functions ) );
</cfscript>"#;
    write(&page_path, page_src);

    assert_eq!(
        "clean|pkg.IndepBase|1",
        run_request(&page_path.to_string_lossy(), page_src),
        "each call must hand back an independent copy"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
