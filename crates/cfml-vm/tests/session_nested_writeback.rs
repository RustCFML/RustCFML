//! Regression: nested writes into an already-assigned `session` sub-struct
//! must be present in the `SessionData` handed to `SessionStore::set`.
//!
//! The CFML shape is the ubiquitous "build an auth struct incrementally" one:
//!
//! ```cfml
//! session.auth = {};                 // top-level assign
//! session.auth.isLoggedIn = true;    // nested write
//! session.auth.user        = "mat";  // nested write
//! ```
//!
//! Lucee/ACF persist the whole `auth` struct. The value committed to the
//! store must therefore carry `isLoggedIn` and `user` — not just the empty
//! `{}` from the first line.
//!
//! Why this is a store-layer test and not a CFML-suite test: within a single
//! request the `session` scope is a live reference, so the nested keys are
//! readable regardless (covered by tests/core/test_session_scope_persist.cfm).
//! The defect only surfaces in what gets *persisted* — the `SessionData`
//! passed to `set()`. A `MemoryStore`-backed app would mask it (the live
//! struct survives in-process across requests), but any serialising store
//! (memcached, datasource) writes exactly this payload, so an app on an
//! external session store silently loses the nested keys: e.g. a framework
//! that does `session.auth = {}` then `session.auth.isLoggedIn = true` finds
//! the user logged out on the very next request.
//!
//! The `SpyStore` records the `SessionData` given to `set()` so the test can
//! assert the persisted payload directly, with no external service.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionData, SessionStore};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const VROOT: &str = "/app";

/// A `SessionStore` that delegates to a real `MemoryStore` but records every
/// `set()` payload, so the test can inspect exactly what a serialising store
/// would have written.
struct SpyStore {
    inner: MemoryStore,
    sets: Mutex<Vec<(String, SessionData)>>,
}

impl SpyStore {
    fn new() -> Self {
        Self {
            inner: MemoryStore::new(),
            sets: Mutex::new(Vec::new()),
        }
    }

    /// The most recently recorded `set()` payload for `id`.
    fn last_set(&self, id: &str) -> Option<SessionData> {
        self.sets
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, _)| k == id)
            .map(|(_, d)| d.clone())
    }
}

impl SessionStore for SpyStore {
    fn get(&self, app: &str, id: &str) -> Option<SessionData> {
        self.inner.get(app, id)
    }
    fn set(&self, app: &str, id: &str, data: SessionData) {
        self.sets
            .lock()
            .unwrap()
            .push((id.to_string(), data.clone()));
        self.inner.set(app, id, data);
    }
    fn remove(&self, app: &str, id: &str) {
        self.inner.remove(app, id);
    }
    fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
        self.inner.rotate(app, old_id, new_id);
    }
    fn contains(&self, app: &str, id: &str) -> bool {
        self.inner.contains(app, id)
    }
    fn take_expired(
        &self,
        now_secs: u64,
    ) -> Vec<(String, String, ValueMap)> {
        self.inner.take_expired(now_secs)
    }
}

const APP: &str = r##"
component {
    this.name              = "session-nested-writeback-test";
    this.sessionManagement = true;
    this.sessionTimeout    = createTimeSpan(0, 1, 0, 0);

    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

/// Build an auth struct incrementally — assign `{}`, then write nested keys.
const PAGE: &str = r#"<cfscript>
    session.auth = {};
    session.auth.isLoggedIn = true;
    session.auth.user = "mat";
</cfscript>"#;

/// Run one request against a caller-supplied store + session id, executing the
/// given CFML page body.
fn run_request(store: Arc<dyn SessionStore>, sid: &str, page: &str) {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), APP.as_bytes().to_vec());
    files.insert("index.cfm".to_string(), page.as_bytes().to_vec());
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

    let mut server_state = ServerState::with_production(false);
    server_state.sessions = store;

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
    vm.session_id = Some(sid.to_string());

    let _ = vm.execute_with_lifecycle();
}

/// The `auth` sub-struct out of a persisted `SessionData`.
fn auth_struct(data: &SessionData) -> Option<ValueMap> {
    data.variables
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("auth"))
        .and_then(|(_, v)| v.as_struct())
}

fn ci_get(m: &ValueMap, key: &str) -> Option<CfmlValue> {
    m.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
}

#[test]
fn nested_session_writes_are_persisted() {
    let spy = Arc::new(SpyStore::new());
    let sid = "sid-nested";

    run_request(spy.clone() as Arc<dyn SessionStore>, sid, PAGE);

    let persisted = spy
        .last_set(sid)
        .expect("the request must persist the session via set()");
    let auth = auth_struct(&persisted)
        .expect("session.auth must persist as a struct");

    // The nested writes after `session.auth = {}` must be in the payload.
    assert!(
        ci_get(&auth, "user").is_some(),
        "nested write session.auth.user was dropped from the persisted payload — \
         auth persisted as {:?}",
        auth.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        ci_get(&auth, "user").and_then(|v| Some(v.as_string())),
        Some("mat".to_string()),
        "session.auth.user must persist its value"
    );
    assert!(
        ci_get(&auth, "isLoggedIn").is_some(),
        "nested write session.auth.isLoggedIn was dropped from the persisted payload — \
         auth persisted as {:?}",
        auth.keys().collect::<Vec<_>>()
    );
}

/// A nested write that auto-vivifies its intermediate AND is the first session
/// touch of the request. This is the path that genuinely needs the
/// `scope_aware_store("session", …)` commit (not just the load arm): the session
/// is lazy/unattached, so `scope_aware_load("session")` returns a COPY of the
/// (empty) persisted variables; store_runtime_path then auto-vivifies a brand-new
/// `auth` struct INTO that copy and inserts the leaf. Without the store-back arm,
/// the freshly-vivified `auth` lives only on the discarded copy and never reaches
/// the persisted SessionData. (Contrast PAGE, where the single-level
/// `session.auth = {}` first attaches the live scope, so the subsequent nested
/// writes mutate a shared handle in place.)
const FIRST_TOUCH_PAGE: &str = r#"<cfscript>
    session.auth.user = "mat";   // first session touch, intermediate auto-vivified
</cfscript>"#;

#[test]
fn nested_session_write_as_first_touch_is_persisted() {
    let spy = Arc::new(SpyStore::new());
    let sid = "sid-first-touch";

    run_request(spy.clone() as Arc<dyn SessionStore>, sid, FIRST_TOUCH_PAGE);

    let persisted = spy
        .last_set(sid)
        .expect("the request must persist the session via set()");
    let auth = auth_struct(&persisted)
        .expect("auto-vivified session.auth must persist as a struct");

    assert_eq!(
        ci_get(&auth, "user").map(|v| v.as_string()),
        Some("mat".to_string()),
        "first-touch nested write session.auth.user was dropped from the \
         persisted payload — auth persisted as {:?}",
        auth.keys().collect::<Vec<_>>()
    );
}

/// Coverage for the delete (null-assignment) shape: deleting a nested session
/// key must leave the sibling intact in the persisted payload. Note this is
/// behavioural coverage, not a regression guard for the v0.143.0 fix — nested
/// deletes mutate a reference-typed (Arc-backed) intermediate in place, so they
/// persist regardless; the defensive store-back in delete_scope_path only
/// matters if a future change makes an intermediate value-typed.
const DELETE_PAGE: &str = r#"<cfscript>
    session.auth = {};
    session.auth.isLoggedIn = true;
    session.auth.user = "mat";
    function voidFn() {}
    session.auth.user = voidFn();   // null assignment -> delete the nested key
</cfscript>"#;

#[test]
fn nested_session_delete_leaves_sibling() {
    let spy = Arc::new(SpyStore::new());
    let sid = "sid-nested-delete";

    run_request(spy.clone() as Arc<dyn SessionStore>, sid, DELETE_PAGE);

    let persisted = spy
        .last_set(sid)
        .expect("the request must persist the session via set()");
    let auth = auth_struct(&persisted)
        .expect("session.auth must persist as a struct");

    assert!(
        ci_get(&auth, "user").is_none(),
        "deleted nested key session.auth.user must be absent — \
         auth persisted as {:?}",
        auth.keys().collect::<Vec<_>>()
    );
    assert!(
        ci_get(&auth, "isLoggedIn").is_some(),
        "sibling key session.auth.isLoggedIn must survive the delete — \
         auth persisted as {:?}",
        auth.keys().collect::<Vec<_>>()
    );
}
