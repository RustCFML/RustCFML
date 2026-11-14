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

// ---------------------------------------------------------------------------
// GH #515: a parent's `__`-prefixed methods after a CHILD edit.
//
// The method-table code treated every `__`-prefixed function as an engine
// entry (`__cfc_body__`, `__cfc_static_init__`) and left it out of the class
// tables. A first construction never noticed: the body's own map entries
// carried such methods. But once the child is edited, its chain generation
// changes and it is rebuilt from scratch against a parent REPLAYED from the
// cross-request cache — and a replayed parent defines no per-instance method
// entries, so `__setupThing` was in neither its map nor its tables:
// "No matching function [__SETUPTHING] found" until restart. Preside's
// Config.cfc (`__setupEnvironmentVariables()`) hit it on every fwreinit after
// the site's Config.cfc was edited.
// ---------------------------------------------------------------------------

const UNDERSCORE_BASE: &str = "component { \
   public void function configure() { __setupThing(); setupOther(); } \
   private void function __setupThing() { variables.thing = \"ok\"; } \
   private void function setupOther() { variables.other = \"ok\"; } \
   private string function __tag() { return \"base-tag\"; } \
   public string function getThing() { \
     return ( variables.thing ?: \"unset\" ) & \"/\" & ( variables.other ?: \"unset\" ); \
   } \
 }";

/// The issue's shape: the child only reaches the parent's `__` methods through
/// inherited code and an unqualified inherited call — no `super.__x()`, which
/// has its own test below.
fn underscore_child(marker: &str) -> String {
    format!(
        "component extends=\"pkg.UsBase\" {{ \
           public void function configure() {{ super.configure(); }} \
           public string function tag() {{ return \"{marker}:\" & __tag(); }} \
         }}"
    )
}

const UNDERSCORE_PAGE: &str = r#"<cfscript>
try {
    c = new pkg.UsChild();
    c.configure();
    writeOutput( "OK " & c.getThing() & " " & c.tag() );
} catch ( any e ) {
    writeOutput( "FAIL " & e.message );
}
</cfscript>"#;

fn underscore_fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml_class_cache_us_{}_{}_{:?}",
        tag,
        std::process::id(),
        std::thread::current().id()
    ));
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).expect("mkdir");
    write(&pkg.join("UsBase.cfc"), UNDERSCORE_BASE);
    let child = pkg.join("UsChild.cfc");
    write(&child, &underscore_child("v1"));
    let page = dir.join("index.cfm");
    std::fs::write(&page, b"").expect("page");
    (dir, child, page.to_string_lossy().to_string())
}

#[test]
fn parent_underscore_methods_survive_a_child_edit_gh515() {
    let (dir, child, page) = underscore_fixture("child_edit");
    let ss = ServerState::with_production(false);

    let v1 = "OK ok/ok v1:base-tag";
    assert_eq!(v1, run_request(&ss, &page, UNDERSCORE_PAGE), "first request");
    assert_eq!(v1, run_request(&ss, &page, UNDERSCORE_PAGE), "second request (replayed)");

    // Edit ONLY the child. The parent is unchanged, so it replays from the
    // cross-request cache while the child is rebuilt against it.
    write(&child, &underscore_child("v2"));
    let v2 = "OK ok/ok v2:base-tag";
    assert_eq!(
        v2,
        run_request(&ss, &page, UNDERSCORE_PAGE),
        "the parent's __-prefixed private methods still resolve after a child edit"
    );
    assert_eq!(v2, run_request(&ss, &page, UNDERSCORE_PAGE), "and on the request after");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parent_underscore_methods_resolve_on_replay_in_production_gh515() {
    let (dir, _child, page) = underscore_fixture("production");
    let ss = ServerState::with_production(true);

    let v1 = "OK ok/ok v1:base-tag";
    for n in 1..=3 {
        assert_eq!(v1, run_request(&ss, &page, UNDERSCORE_PAGE), "request {n}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The same `__` rule kept the parent's `__` methods out of `__super`, so
/// `super.__x()` returned an empty string even on the very first construction.
#[test]
fn super_reaches_a_parents_underscore_method_gh515() {
    let (dir, child, page) = underscore_fixture("super");
    write(
        &child,
        "component extends=\"pkg.UsBase\" { \
           public string function viaSuper() { return \"super:\" & super.__tag(); } \
         }",
    );
    let ss = ServerState::with_production(false);
    let page_src = r#"<cfscript>
try { writeOutput( new pkg.UsChild().viaSuper() ); }
catch ( any e ) { writeOutput( "FAIL " & e.message ); }
</cfscript>"#;
    for n in 1..=2 {
        assert_eq!("super:base-tag", run_request(&ss, &page, page_src), "request {n}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
