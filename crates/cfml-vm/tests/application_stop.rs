//! End-to-end coverage for `applicationStop()`.
//!
//! The function has to mutate the VM's shared application store, so this test
//! drives real lifecycle execution instead of testing the stdlib shim.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";
const APP_NAME: &str = "application-stop-test";

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

fn run_request(server_state: &ServerState, vfs: Arc<dyn Vfs>, page: &str, endlog: &str) -> String {
    let page_path = format!("{}/{}", VROOT, page);
    let program = compile_page(&vfs, &page_path);

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
    // Hand onApplicationEnd a path to record that it fired.
    let mut url = ValueMap::default();
    url.insert("endlog".to_string(), CfmlValue::string(endlog.to_string()));
    vm.globals.insert("url".to_string(), CfmlValue::strukt(url));
    vm.globals
        .entry("cgi".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("form".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    vm.server_state = Some(server_state.clone());

    vm.execute_with_lifecycle().unwrap();
    vm.output_buffer.trim().to_string()
}

fn seed_of(server_state: &ServerState) -> String {
    let app = server_state.applications.get(APP_NAME).unwrap();
    let (_, value) = app
        .variables
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("seed"))
        .expect("started application must hold a seed");
    match value {
        CfmlValue::String(s) => s.to_string(),
        other => panic!("seed should be a string, got {:?}", other),
    }
}

#[test]
fn application_stop_clears_shared_application_state() {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert(
        "Application.cfc".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/Application.cfc")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "index.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/index.cfm")
            .as_bytes()
            .to_vec(),
    );
    files.insert(
        "stop.cfm".to_string(),
        include_str!("../../../tests/lifecycle/application_stop/stop.cfm")
            .as_bytes()
            .to_vec(),
    );

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    // onApplicationEnd records here so we can prove it fired (Lucee parity).
    let endlog = std::env::temp_dir().join("rustcfml-application-stop-end.log");
    let endlog_str = endlog.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&endlog);

    // First request starts the application. The rendered seed must match the
    // value stored in shared scope by onApplicationStart.
    let first_seed_output = run_request(&server_state, vfs.clone(), "index.cfm", &endlog_str);
    let started_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(started_app.started);
    let first_seed = seed_of(&server_state);
    assert_eq!(
        first_seed, first_seed_output,
        "index.cfm must render the seed onApplicationStart stored"
    );
    assert!(
        !endlog.exists(),
        "onApplicationEnd must not fire on an ordinary request"
    );

    // The stop request runs, sets application.stopMarker, then calls
    // applicationStop(). The page still completes and renders its output.
    let stop_output = run_request(&server_state, vfs.clone(), "stop.cfm", &endlog_str);
    assert_eq!("stopped", stop_output, "stop.cfm must complete normally");

    // applicationStop() must fire onApplicationEnd exactly once, synchronously,
    // with the still-live application scope (so it sees the original seed).
    let end_log = std::fs::read_to_string(&endlog).expect("onApplicationEnd must have run");
    let end_lines: Vec<&str> = end_log.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        vec![format!("END:{}", first_seed)],
        end_lines,
        "onApplicationEnd must fire once with the pre-stop application scope"
    );

    let stopped_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(
        !stopped_app.started,
        "applicationStop() must mark the application unstarted"
    );
    assert!(
        stopped_app.variables.is_empty(),
        "applicationStop() must clear application scope variables (including the \
         stopMarker set in the same request before the call)"
    );
    assert!(
        stopped_app.app_function_table.is_empty(),
        "applicationStop() must discard the carried function table"
    );

    // The next request must re-fire onApplicationStart, producing a brand-new
    // seed — proving the lifecycle genuinely restarted rather than reusing a
    // lingering value.
    let restart_seed_output = run_request(&server_state, vfs, "index.cfm", &endlog_str);
    let restarted_app = server_state.applications.get(APP_NAME).unwrap();
    assert!(restarted_app.started);
    let restart_seed = seed_of(&server_state);
    assert_eq!(
        restart_seed, restart_seed_output,
        "restarted index.cfm must render the freshly generated seed"
    );
    assert_ne!(
        first_seed, restart_seed,
        "restart must run onApplicationStart again and generate a new seed"
    );

    // The restart's onApplicationStart must NOT have logged another END line:
    // onApplicationEnd belongs to applicationStop(), not to startup.
    let end_log = std::fs::read_to_string(&endlog).unwrap();
    assert_eq!(
        1,
        end_log.lines().filter(|l| !l.is_empty()).count(),
        "onApplicationEnd must fire only for applicationStop(), not on restart"
    );

    let _ = std::fs::remove_file(&endlog);
}
