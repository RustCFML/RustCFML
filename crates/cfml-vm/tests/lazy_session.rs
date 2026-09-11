//! End-to-end test for `this.lazySessionCreation`.
//!
//! Wires up a tiny in-memory VFS containing an Application.cfc + a
//! page, runs `execute_with_lifecycle` directly, and asserts on the
//! shared `MemoryStore`'s contents.
//!
//! Default (eager) mode: a request always materialises a session
//! record + fires `onSessionStart` once. Lazy mode: only writes
//! trigger creation, reads against a non-existent session are silent.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionStore};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

fn run_request(app_cfc: &str, page_cfm: &str, lazy: bool, sid: Option<&str>) -> Arc<MemoryStore> {
    // Lazy is now the engine-wide default (issue #88), so eager mode must
    // opt out explicitly. Always inject the flag so the test pins the mode
    // it intends regardless of the engine default.
    let flag = if lazy {
        "this.sessionManagement = true; this.lazySessionCreation = true;"
    } else {
        "this.sessionManagement = true; this.lazySessionCreation = false;"
    };
    let app_with_flag = app_cfc.replace("this.sessionManagement = true;", flag);

    // EmbeddedFs keys are relative to base_dir.
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), app_with_flag.into_bytes());
    files.insert("index.cfm".to_string(), page_cfm.as_bytes().to_vec());
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));

    let page_path = format!("{}/index.cfm", VROOT);
    let source = vfs.read_to_string(&page_path).unwrap();
    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };
    let ast = Parser::new(processed).parse().unwrap();
    let program = CfmlCompiler::new().compile(ast);

    let store = Arc::new(MemoryStore::new());
    let mut server_state = ServerState::with_production(false);
    server_state.sessions = store.clone() as Arc<dyn SessionStore>;

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(std::sync::Arc::from(page_path.as_str()));
    vm.base_template_path = Some(page_path);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("cgi".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("form".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    vm.server_state = Some(server_state);
    vm.session_id = sid.map(String::from);

    let _ = vm.execute_with_lifecycle();
    store
}

/// The fixture's `this.name` — sessions are keyed by `(app, id)`, so store
/// assertions must query under this same partition.
const APP_NAME: &str = "lazy-session-test";

const APP_BASE: &str = r##"
component {
    this.name              = "lazy-session-test";
    this.sessionManagement = true;
    this.sessionTimeout    = createTimeSpan(0, 1, 0, 0);

    function onApplicationStart() { application.started = true; }

    function onSessionStart() {
        application.sessionStarts = (application.sessionStarts ?: 0) + 1;
    }

    function onRequest(targetPage) {
        include "#targetPage#";
    }
}
"##;

#[test]
fn eager_mode_creates_record_for_every_visit() {
    // Default: even a page that never touches session creates a record.
    let store = run_request(APP_BASE, "<cfoutput>hello</cfoutput>", false, Some("sid-A"));
    assert!(store.contains(APP_NAME, "sid-A"), "eager mode must create the record");
}

#[test]
fn lazy_mode_skips_record_when_page_never_writes_session() {
    let store = run_request(
        APP_BASE,
        "<cfoutput>hello — session never touched</cfoutput>",
        true,
        Some("sid-B"),
    );
    assert!(
        !store.contains(APP_NAME, "sid-B"),
        "lazy mode must NOT create a record when the page doesn't write session.X"
    );
}

#[test]
fn lazy_mode_creates_record_on_first_session_write() {
    let store = run_request(
        APP_BASE,
        r#"<cfscript> session.cart = []; </cfscript>"#,
        true,
        Some("sid-C"),
    );
    assert!(
        store.contains(APP_NAME, "sid-C"),
        "lazy mode must create the record on first session write"
    );
    let data = store.get(APP_NAME, "sid-C").unwrap();
    assert!(
        data.variables.keys().any(|k| k.eq_ignore_ascii_case("cart")),
        "the user write should be persisted alongside the lazy-created record"
    );
}

#[test]
fn lazy_mode_does_not_create_record_on_session_read_only() {
    // Read-only access to non-existent session keys returns defaults
    // (or Null with nullSupport off → empty string) and must NOT
    // materialise a record. Matches the Preside-CMS pattern.
    let store = run_request(
        APP_BASE,
        r#"<cfscript>
            // Defensive read — should not trigger session creation.
            var exists = structKeyExists(session, "user");
            writeOutput("exists=" & exists);
        </cfscript>"#,
        true,
        Some("sid-D"),
    );
    assert!(
        !store.contains(APP_NAME, "sid-D"),
        "lazy mode must NOT create a record on read-only session access"
    );
}
