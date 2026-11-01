//! End-to-end test for `this.lazySessionCreation`.
//!
//! Wires up a tiny in-memory VFS containing an Application.cfc + a
//! page, runs `execute_with_lifecycle` directly, and asserts on the
//! shared `MemoryStore`'s contents.
//!
//! Default (eager) mode: a request always materialises a session
//! record + fires `onSessionStart` once. Lazy mode: only writes
//! trigger creation, reads against a non-existent session are silent.
//!
//! Session fixation: a `CFID` the client supplies is only honoured when
//! it names a session the store already holds. An unknown id is treated
//! exactly like no cookie at all — the engine mints its own — so an
//! attacker cannot plant an id in a victim's browser and later read the
//! session the victim establishes under it. `run_request` therefore takes
//! the supplied id separately from whether the store should already KNOW
//! it (`known`), and returns the id the VM ended the request with.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionData, SessionStore};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const VROOT: &str = "/app";

/// Run one request. `sid` is the id the request carries (its `CFID`
/// cookie); `known` pre-seeds the store with an unexpired record under it,
/// i.e. the cookie names a session this server issued earlier. Returns the
/// store and the id the VM holds at the end of the request — what the
/// embedder would put in `Set-Cookie`.
fn run_request(
    app_cfc: &str,
    page_cfm: &str,
    lazy: bool,
    sid: Option<&str>,
    known: bool,
) -> (Arc<MemoryStore>, Option<String>) {
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
    if let (true, Some(sid)) = (known, sid) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        store.set(
            APP_NAME,
            sid,
            SessionData {
                variables: ValueMap::default(),
                created_secs: now,
                last_accessed_secs: now,
                auth_user: None,
                auth_roles: Vec::new(),
                timeout_secs: 3600,
                app_name: APP_NAME.to_string(),
                live: None,
            },
        );
    }
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
    let final_sid = vm.session_id.clone();
    (store, final_sid)
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
    let (store, sid) = run_request(APP_BASE, "<cfoutput>hello</cfoutput>", false, None, false);
    let sid = sid.expect("eager mode must mint an id for a cookie-less request");
    assert!(store.contains(APP_NAME, &sid), "eager mode must create the record");
}

#[test]
fn eager_mode_keeps_a_known_cookie_id() {
    // The cookie names a session this server issued: it is honoured as-is.
    let (store, sid) = run_request(APP_BASE, "<cfoutput>hello</cfoutput>", false, Some("sid-A"), true);
    assert_eq!(sid.as_deref(), Some("sid-A"), "a known CFID must be kept");
    assert!(store.contains(APP_NAME, "sid-A"));
}

#[test]
fn eager_mode_does_not_adopt_an_unknown_cookie_id() {
    // Session fixation: the cookie carries an id the store has never seen.
    // It must NOT become the session's id — a fresh one is minted, exactly
    // as if no cookie had been sent, and the attacker's id stays empty.
    let (store, sid) =
        run_request(APP_BASE, "<cfoutput>hello</cfoutput>", false, Some("attacker-fixed-id"), false);
    let sid = sid.expect("a fresh id must be minted in place of the unknown one");
    assert_ne!(sid, "attacker-fixed-id", "an unknown CFID must not be adopted");
    assert!(store.contains(APP_NAME, &sid), "the record lives under the minted id");
    assert!(
        !store.contains(APP_NAME, "attacker-fixed-id"),
        "nothing may be stored under the attacker-chosen id"
    );
}

#[test]
fn lazy_mode_skips_record_when_page_never_writes_session() {
    let (store, _) = run_request(
        APP_BASE,
        "<cfoutput>hello — session never touched</cfoutput>",
        true,
        Some("sid-B"),
        true,
    );
    // The known record is still there (touched), untouched otherwise, and
    // no second one appeared.
    assert!(store.contains(APP_NAME, "sid-B"));
    assert_eq!(store.session_count(), Some(1));
}

#[test]
fn lazy_mode_unknown_cookie_id_without_a_write_mints_nothing() {
    // An unknown id under lazy creation: not adopted, and since the page
    // never writes session there is nothing to mint either — the request
    // ends with no id at all, so the embedder sends no cookie.
    let (store, sid) = run_request(
        APP_BASE,
        "<cfoutput>hello — session never touched</cfoutput>",
        true,
        Some("attacker-fixed-id"),
        false,
    );
    assert_eq!(sid, None, "an unknown CFID must be dropped, not carried");
    assert_eq!(store.session_count(), Some(0), "lazy mode must NOT create a record");
}

#[test]
fn lazy_mode_unknown_cookie_id_is_replaced_on_first_write() {
    let (store, sid) = run_request(
        APP_BASE,
        r#"<cfscript> session.cart = []; </cfscript>"#,
        true,
        Some("attacker-fixed-id"),
        false,
    );
    let sid = sid.expect("the first write must mint an id");
    assert_ne!(sid, "attacker-fixed-id", "an unknown CFID must not be adopted");
    assert!(store.contains(APP_NAME, &sid));
    assert!(!store.contains(APP_NAME, "attacker-fixed-id"));
}

#[test]
fn lazy_mode_creates_record_on_first_session_write() {
    let (store, sid) = run_request(
        APP_BASE,
        r#"<cfscript> session.cart = []; </cfscript>"#,
        true,
        Some("sid-C"),
        true,
    );
    assert_eq!(sid.as_deref(), Some("sid-C"), "a known CFID must be kept across the write");
    assert!(
        store.contains(APP_NAME, "sid-C"),
        "lazy mode must create the record on first session write"
    );
    let data = store.get(APP_NAME, "sid-C").unwrap();
    assert!(
        data.variables_snapshot().keys().any(|k| k.eq_ignore_ascii_case("cart")),
        "the user write should be persisted alongside the lazy-created record"
    );
}

#[test]
fn lazy_mode_does_not_create_record_on_session_read_only() {
    // Read-only access to non-existent session keys returns defaults
    // (or Null with nullSupport off → empty string) and must NOT
    // materialise a record. Matches the Preside-CMS pattern.
    let (store, sid) = run_request(
        APP_BASE,
        r#"<cfscript>
            // Defensive read — should not trigger session creation.
            var exists = structKeyExists(session, "user");
            writeOutput("exists=" & exists);
        </cfscript>"#,
        true,
        None,
        false,
    );
    assert_eq!(sid, None, "a read-only request must not mint an id");
    assert_eq!(
        store.session_count(),
        Some(0),
        "lazy mode must NOT create a record on read-only session access"
    );
}
