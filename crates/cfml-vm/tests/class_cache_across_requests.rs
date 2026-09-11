//! The cross-request half of the component-construction cache
//! (`ServerState::class_caches`), which the CFML suite can only observe through
//! a live server.
//!
//! A class's method tables, own-method table, `super` struct and flyweight
//! blueprint are built on its first construction in a request and replayed on
//! every later one. Before v0.661.0 they lived on the per-request VM, so a page
//! that constructs most classes once — the Preside shape — paid the
//! first-construction price for every class on every request and never saw the
//! replay path at all. Pins three properties:
//!
//! 1. The second request adopts what the first published (and the entry is
//!    genuinely populated — the equality alone would pass with the cache inert).
//! 2. An instance built from adopted tables behaves exactly like the one that
//!    built them: methods, `super`, inherited private state, metadata.
//! 3. Dev mode sees an edit at ANY level of the `extends` chain on the next
//!    request. A child's `variables` table holds the parent's methods too, so
//!    the cache key is the chain generation, not the file's own.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};

fn compile_page(source: &str) -> BytecodeProgram {
    let processed = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script(source)
    } else {
        source.to_string()
    };
    let ast = Parser::new(processed).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

/// One "request": a fresh VM sharing `ss`, exactly as the serve loop builds one.
fn run_request(ss: &ServerState, page_path: &str, source: &str) -> String {
    let mut vm = CfmlVirtualMachine::new(compile_page(source));
    vm.source_file = Some(std::sync::Arc::from(page_path));
    vm.base_template_path = Some(page_path.to_string());
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    for scope in ["url", "cgi", "form"] {
        vm.globals
            .entry(scope.to_string())
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    }
    vm.server_state = Some(ss.clone());
    vm.execute().expect("execute");
    vm.get_output().trim().to_string()
}

/// Write a fixture and push its mtime forward, so the bytecode cache's dev-mode
/// freshness check cannot mistake a same-instant rewrite for the old file.
fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("write fixture");
    let bumped = std::time::SystemTime::now() + std::time::Duration::from_secs(3);
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(bumped))
        .expect("bump mtime");
}

struct Fixture {
    dir: std::path::PathBuf,
    base: std::path::PathBuf,
    child: std::path::PathBuf,
    page: String,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml_class_cache_{}_{}_{:?}",
        tag,
        std::process::id(),
        std::thread::current().id()
    ));
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("mkdir");
    let base = pkg.join("CcBase.cfc");
    let child = pkg.join("CcChild.cfc");
    write(
        &base,
        "component { \
           variables.fromBase = \"base-private\"; \
           this.baseData = 7; \
           public string function baseAlpha() { return \"a\"; } \
           public string function shared() { return \"base\"; } \
         }",
    );
    write(
        &child,
        "component extends=\"pkg.CcBase\" { \
           public string function childOne() { return \"c\"; } \
           public string function shared() { return \"child+\" & super.shared(); } \
           public string function privateFromBase() { return variables.fromBase; } \
         }",
    );
    let page = dir.join("index.cfm");
    std::fs::write(&page, b"").expect("page");
    Fixture {
        dir,
        base,
        child,
        page: page.to_string_lossy().to_string(),
    }
}

/// Constructs the child ONCE and prints everything the cached tables decide.
const PAGE: &str = r#"<cfscript>
function names( required array functions ) {
    local.out = [];
    for ( local.f in arguments.functions ) { arrayAppend( local.out, local.f.name ); }
    arraySort( local.out, "textnocase" );
    return arrayToList( local.out );
}
o = new pkg.CcChild();
writeOutput( o.childOne() & "|" & o.baseAlpha() & "|" & o.shared() & "|" & o.privateFromBase()
    & "|" & o.baseData & "|" & names( getMetadata( o ).functions ) /* own functions only; inherited live under .extends */
    & "|" & names( getComponentMetaData( "pkg.CcChild" ).functions )
    & "|" & listSort( structKeyList( o ), "textnocase" ) );
</cfscript>"#;

const EXPECTED: &str = "c|a|child+base|base-private|7|childOne,privateFromBase,shared|childOne,privateFromBase,shared|baseAlpha,baseData,childOne,privateFromBase,shared";

#[test]
fn second_request_adopts_the_tables_the_first_request_published() {
    let fx = fixture("adopt");
    let ss = ServerState::with_production(true);

    assert_eq!(EXPECTED, run_request(&ss, &fx.page, PAGE), "first request");

    // The cache must actually hold the class now — with the cache inert, the
    // second request would rebuild and the equality below would still pass.
    let child_src = fx.child.to_string_lossy().to_string();
    {
        let caches = ss.class_caches.read();
        let entry = caches
            .iter()
            .find(|(k, _)| k.ends_with("CcChild.cfc"))
            .map(|(_, e)| e.clone())
            .unwrap_or_else(|| panic!("no class cache entry for {child_src}; keys: {:?}", caches.keys().collect::<Vec<_>>()));
        assert!(entry.method_tables.is_some(), "method tables published");
        assert!(entry.own_table.is_some(), "own-method table published");
        assert!(!entry.blueprints.is_empty(), "blueprint published");
        let base = caches
            .iter()
            .find(|(k, _)| k.ends_with("CcBase.cfc"))
            .map(|(_, e)| e.clone())
            .expect("parent has an entry");
        assert!(base.super_value.is_some(), "parent's super struct published");
    }

    assert_eq!(
        EXPECTED,
        run_request(&ss, &fx.page, PAGE),
        "second request (adopted tables) is indistinguishable from the first"
    );
    assert_eq!(EXPECTED, run_request(&ss, &fx.page, PAGE), "third request");

    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[test]
fn dev_mode_sees_a_parent_edit_through_the_child_on_the_next_request() {
    let fx = fixture("parent_edit");
    let ss = ServerState::with_production(false);

    assert_eq!(EXPECTED, run_request(&ss, &fx.page, PAGE), "first request");
    assert_eq!(EXPECTED, run_request(&ss, &fx.page, PAGE), "second request (cached)");

    // Edit ONLY the parent: a new method, a changed body. The child's file is
    // untouched, so a cache keyed on the child's own compile alone would keep
    // serving the old inherited table.
    write(
        &fx.base,
        "component { \
           variables.fromBase = \"base-private-2\"; \
           this.baseData = 8; \
           public string function baseAlpha() { return \"a2\"; } \
           public string function baseBeta() { return \"b\"; } \
           public string function shared() { return \"base2\"; } \
         }",
    );
    assert_eq!(
        "c|a2|child+base2|base-private-2|8|childOne,privateFromBase,shared|childOne,privateFromBase,shared|baseAlpha,baseBeta,baseData,childOne,privateFromBase,shared",
        run_request(&ss, &fx.page, PAGE),
        "a parent edit is visible through the child on the next request"
    );

    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[test]
fn dev_mode_sees_a_child_edit_on_the_next_request() {
    let fx = fixture("child_edit");
    let ss = ServerState::with_production(false);

    assert_eq!(EXPECTED, run_request(&ss, &fx.page, PAGE), "first request");

    write(
        &fx.child,
        "component extends=\"pkg.CcBase\" { \
           public string function childOne() { return \"c2\"; } \
           public string function childTwo() { return \"t\"; } \
           public string function shared() { return \"child2+\" & super.shared(); } \
           public string function privateFromBase() { return variables.fromBase; } \
         }",
    );
    assert_eq!(
        "c2|a|child2+base|base-private|7|childOne,childTwo,privateFromBase,shared|childOne,childTwo,privateFromBase,shared|baseAlpha,baseData,childOne,childTwo,privateFromBase,shared",
        run_request(&ss, &fx.page, PAGE),
        "a child edit is visible on the next request"
    );

    let _ = std::fs::remove_dir_all(&fx.dir);
}
