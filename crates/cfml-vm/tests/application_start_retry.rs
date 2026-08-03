//! Regression coverage: a FAILED `onApplicationStart` must be retried on the
//! next request, never cached.
//!
//! `execute_with_lifecycle` flips `app.started = true` *before* invoking
//! `onApplicationStart`, so that concurrent requests don't all try to boot. The
//! error path returned without rolling that flag back, and the only other reset
//! is an explicit `applicationStop()`. So a single failed start — one slow or
//! black-holed database round-trip, one transient upstream blip — permanently
//! poisoned the process: every later request saw `started == true` and skipped
//! the start handler, serving an application whose scope and function table had
//! never been populated.
//!
//! That is the cached-failed-boot half of GitHub #302, and it is why the
//! reporter's symptom survived every engine version they bisected: the bug is
//! reached by *timing*, not by any particular release.
//!
//! Whether to keep retrying an application that cannot boot is the application's
//! decision, not the engine's — so the engine always retries.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

/// `onApplicationStart` throws the first time and succeeds afterwards. The
/// attempt counter lives in `server` scope specifically because it must survive
/// the failed boot — application scope is (correctly) discarded on that path.
const APPLICATION_CFC: &str = r#"component {
    this.name = "startRetryTest";

    function onApplicationStart() {
        if ( !structKeyExists( server, "bootAttempts" ) ) {
            server.bootAttempts = 0;
        }
        server.bootAttempts = server.bootAttempts + 1;

        if ( server.bootAttempts == 1 ) {
            throw( message="simulated first-boot failure" );
        }

        application.ready = true;
        return true;
    }

    function onRequestStart() { return true; }
}"#;

// `r##"…"##`: the CFML body contains `"#` (a quoted string ending an output
// interpolation), which would close an `r#"…"#` literal early.
const PAGE: &str = r##"<cfoutput>#structKeyExists( application, "ready" ) ? "ready" : "NOT-ready"#|#( structKeyExists( server, "bootAttempts" ) ? server.bootAttempts : 0 )#</cfoutput>"##;

fn fixtures() -> HashMap<String, Vec<u8>> {
    let mut f = HashMap::new();
    f.insert("Application.cfc".into(), APPLICATION_CFC.as_bytes().to_vec());
    f.insert("index.cfm".into(), PAGE.as_bytes().to_vec());
    f
}

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

/// Drive one request. Returns `Err` when the lifecycle itself failed (which the
/// first request here is expected to do).
fn run_request(
    server_state: &ServerState,
    vfs: Arc<dyn Vfs>,
    page: &str,
) -> Result<String, String> {
    let page_path = format!("{}/{}", VROOT, page);
    let program = compile_page(&vfs, &page_path);

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
    for s in ["url", "cgi", "form"] {
        vm.globals
            .entry(s.to_string())
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    }
    vm.server_state = Some(server_state.clone());
    match vm.execute_with_lifecycle() {
        Ok(_) => Ok(vm.output_buffer.trim().to_string()),
        Err(e) => Err(e.message),
    }
}

#[test]
fn failed_application_start_is_retried_on_the_next_request_not_cached() {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(fixtures(), VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    // Request 1 — onApplicationStart throws. The request itself must fail.
    let first = run_request(&server_state, vfs.clone(), "index.cfm");
    assert!(
        first.is_err(),
        "request 1 should surface the onApplicationStart failure, got {first:?}"
    );

    // Request 2 — the engine must RETRY the start rather than treat the
    // application as already-started. Before the fix this returned
    // "NOT-ready|1": the handler was skipped, so `application.ready` was never
    // set and the attempt counter never advanced past the failed first try.
    let second = run_request(&server_state, vfs.clone(), "index.cfm")
        .expect("request 2 should succeed once the start handler is retried");
    assert_eq!(
        "ready|2", second,
        "request 2 must re-run onApplicationStart (attempt 2) and populate application scope"
    );

    // Request 3 — and having now succeeded, it must NOT run again.
    let third = run_request(&server_state, vfs, "index.cfm")
        .expect("request 3 should succeed");
    assert_eq!(
        "ready|2", third,
        "a successful start must still be cached — attempt count stays at 2"
    );
}
