//! Data-only session enforcement (issues #88, #236).
//!
//! A SERIALIZING session store (memcached / datasource / KV / cluster) persists
//! data values only: writing a closure, function, component, or native object
//! must fail loudly instead of silently serialising to null. The default
//! in-process store keeps live object references, so it accepts CFCs/closures in
//! `session`, matching Lucee/ACF in-memory sessions (issue #236).

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionData, SessionStore};
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

/// A session store that reports it persists by SERIALIZATION (so the data-only
/// rule applies), delegating storage to an in-process map. Mirrors what a real
/// memcached/datasource store does for the purpose of this test.
#[derive(Default)]
struct SerializingStore {
    inner: MemoryStore,
}

impl SessionStore for SerializingStore {
    fn persists_by_serialization(&self) -> bool {
        true
    }
    fn get(&self, app: &str, id: &str) -> Option<SessionData> {
        self.inner.get(app, id)
    }
    fn set(&self, app: &str, id: &str, data: SessionData) {
        self.inner.set(app, id, data)
    }
    fn remove(&self, app: &str, id: &str) {
        self.inner.remove(app, id)
    }
    fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
        self.inner.rotate(app, old_id, new_id)
    }
    fn take_expired(&self, now_secs: u64) -> Vec<(String, String, ValueMap)> {
        self.inner.take_expired(now_secs)
    }
}

/// Run a request and return the lifecycle result. `serializing` selects a
/// serializing store (data-only enforced) vs the default in-memory store.
fn run_with(page_cfm: &str, serializing: bool) -> Result<CfmlValue, cfml_common::vm::CfmlError> {
    let app_cfc = r##"
component {
    this.name              = "data-only-test";
    this.sessionManagement = true;
    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), app_cfc.as_bytes().to_vec());
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

    let mut server_state = ServerState::with_production(false);
    server_state.sessions = if serializing {
        Arc::new(SerializingStore::default()) as Arc<dyn SessionStore>
    } else {
        Arc::new(MemoryStore::new()) as Arc<dyn SessionStore>
    };

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
    vm.session_id = Some("sid-data-only".to_string());

    vm.execute_with_lifecycle()
}

#[test]
fn plain_data_is_allowed_on_memory() {
    let r = run_with(r#"<cfscript> session.cart = ["a", "b"]; session.count = 2; </cfscript>"#, false);
    assert!(r.is_ok(), "plain data values must persist without error: {:?}", r.err());
}

#[test]
fn plain_data_is_allowed_on_serializing() {
    let r = run_with(r#"<cfscript> session.cart = ["a", "b"]; session.count = 2; </cfscript>"#, true);
    assert!(r.is_ok(), "plain data values must persist without error: {:?}", r.err());
}

#[test]
fn closure_in_memory_session_is_allowed() {
    // In-memory keeps live references, so a closure in session is fine (Lucee
    // parity, issue #236) — no serialization to null.
    let r = run_with(r#"<cfscript> session.handler = function() { return 1; }; </cfscript>"#, false);
    assert!(r.is_ok(), "an in-memory session may hold a closure: {:?}", r.err());
}

#[test]
fn closure_in_serializing_session_is_rejected() {
    let r = run_with(r#"<cfscript> session.handler = function() { return 1; }; </cfscript>"#, true);
    let err = r.expect_err("a closure in a serializing session must be rejected");
    assert!(
        err.message.to_lowercase().contains("session.handler")
            && err.message.to_lowercase().contains("data values"),
        "error should name the offending key path: {}",
        err.message
    );
}

#[test]
fn nested_closure_in_serializing_session_is_rejected_with_path() {
    let r = run_with(r#"<cfscript> session.cfg = { cb: function(){ return 1; } }; </cfscript>"#, true);
    let err = r.expect_err("a nested closure in a serializing session must be rejected");
    assert!(
        err.message.contains("session.cfg.cb"),
        "error should name the nested key path: {}",
        err.message
    );
}

#[test]
fn reference_smuggled_closure_is_caught_at_persist_on_serializing() {
    // The shallow assignment check can't see this — the persist-time deep walk
    // is the airtight gate for serializing stores.
    let r = run_with(
        r#"<cfscript>
            local.holder = {};
            session.box = local.holder;   // plain struct at write time
            local.holder.fn = function(){ return 1; };  // mutated through the alias
        </cfscript>"#,
        true,
    );
    let err = r.expect_err("a reference-smuggled closure must be caught at persist");
    assert!(
        err.message.to_lowercase().contains("data values"),
        "persist gate should reject smuggled non-data value: {}",
        err.message
    );
}
