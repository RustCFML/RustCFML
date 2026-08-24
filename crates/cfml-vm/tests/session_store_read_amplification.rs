//! Regression (GH #361): a request must not re-read the session record from
//! the store over and over.
//!
//! On the out-of-process stores (datasource, memcached) every
//! `SessionStore::get` is a network round trip, and the request path used to
//! ask for the same record repeatedly — the start-of-request touch (twice,
//! because `contains()` is `get().is_some()`), the live-scope attach, every
//! `isUserLoggedIn`/`isUserInRole`/`getAuthUser` call in user code, and the
//! end-of-request persist. On a remote Postgres that was ~185 ms of pure
//! latency before any CFML ran.
//!
//! The VM now memoises the record per request for stores that declare
//! `reads_are_cheap() == false`. Stores whose reads are free (the default
//! in-process `MemoryStore`) deliberately keep re-reading, so their
//! concurrency behaviour is untouched — both halves are asserted here.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionData, SessionStore};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const VROOT: &str = "/app";
const APP_NAME: &str = "session-read-amplification-test";

/// A `MemoryStore` that counts `get()` calls and can pose as either a cheap
/// in-process store or an expensive remote one.
struct CountingStore {
    inner: MemoryStore,
    cheap: bool,
    gets: AtomicUsize,
}

impl CountingStore {
    fn new(cheap: bool) -> Self {
        Self {
            inner: MemoryStore::new(),
            cheap,
            gets: AtomicUsize::new(0),
        }
    }
    fn gets(&self) -> usize {
        self.gets.load(Ordering::Relaxed)
    }
    fn reset(&self) {
        self.gets.store(0, Ordering::Relaxed);
    }
}

impl SessionStore for CountingStore {
    fn get(&self, app: &str, id: &str) -> Option<SessionData> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get(app, id)
    }
    fn set(&self, app: &str, id: &str, data: SessionData) {
        self.inner.set(app, id, data);
    }
    fn remove(&self, app: &str, id: &str) {
        self.inner.remove(app, id);
    }
    fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
        self.inner.rotate(app, old_id, new_id);
    }
    fn reads_are_cheap(&self) -> bool {
        self.cheap
    }
    fn take_expired(&self, now_secs: u64) -> Vec<(String, String, ValueMap)> {
        self.inner.take_expired(now_secs)
    }
}

const APP: &str = r##"
component {
    this.name              = "session-read-amplification-test";
    this.sessionManagement = true;
    this.sessionTimeout    = createTimeSpan(0, 1, 0, 0);

    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

/// Three auth predicates plus a session write — the shape of a framework
/// request (Preside calls `isUserLoggedIn()` many times per page).
const PAGE: &str = r#"<cfscript>
    a = isUserLoggedIn();
    b = isUserLoggedIn();
    c = isUserInRole("admin");
    d = getAuthUser();
    session.visits = ( session.visits ?: 0 ) + 1;
</cfscript>"#;

fn run_request(store: Arc<dyn SessionStore>, sid: &str) {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), APP.as_bytes().to_vec());
    files.insert("index.cfm".to_string(), PAGE.as_bytes().to_vec());
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
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
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

    vm.server_state = Some(server_state);
    vm.session_id = Some(sid.to_string());

    let _ = vm.execute_with_lifecycle();
}

fn visits(data: &SessionData) -> i64 {
    data.variables
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("visits"))
        .and_then(|(_, v)| v.as_string().trim().parse::<i64>().ok())
        .unwrap_or(-1)
}

#[test]
fn a_remote_store_is_read_once_per_request() {
    let store = Arc::new(CountingStore::new(false));
    let sid = "sid-remote";

    // Request 1 creates the record.
    run_request(store.clone() as Arc<dyn SessionStore>, sid);
    store.reset();

    // Request 2 finds it. One read is the whole budget: the touch refreshes
    // the memo, and the attach, the four auth/session BIFs and the
    // end-of-request persist all serve from it.
    run_request(store.clone() as Arc<dyn SessionStore>, sid);
    assert_eq!(
        store.gets(),
        1,
        "an established session must cost exactly one store read per request"
    );

    // ...and the request still did its job.
    let rec = store.inner.get(APP_NAME, sid).expect("record present");
    assert_eq!(visits(&rec), 2, "the memo must not swallow the writeback");
}

#[test]
fn a_cheap_store_keeps_re_reading() {
    // The default in-process store is deliberately NOT memoised — re-reading
    // costs a mutex and narrows the window in which a concurrent request's
    // `cflogin` could be clobbered. If this ever starts reading once, the
    // `reads_are_cheap` split has been lost.
    let store = Arc::new(CountingStore::new(true));
    let sid = "sid-cheap";

    run_request(store.clone() as Arc<dyn SessionStore>, sid);
    store.reset();
    run_request(store.clone() as Arc<dyn SessionStore>, sid);

    assert!(
        store.gets() > 1,
        "a cheap store should still re-read per use, got {} read(s)",
        store.gets()
    );
}
