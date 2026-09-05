//! A generation of components displaced from application scope must be
//! reclaimed — including when each component keeps its LIVE `variables` scope.
//!
//! A component that defines a closure captures its `variables` scope, so the
//! Instance keeps that TRACKED scope struct rather than the untracked
//! partitioned copy. `NodeHandle::Instance::for_each_child_node` walked the data
//! maps' VALUES but never emitted the maps themselves, so the Instance's own
//! reference to that scope was an uncounted external edge:
//!
//!     external = strong(3) - 1 probe - 1 internal = 1   =>  pinned root
//!
//! One pinned root per instance, and marking its transitive closure live
//! stranded the entire generation. On a Preside `?fwreinit=true` that was
//! ~111,000 nodes per reload, with the cross-request sweep reclaiming ~862.
//!
//! The assertion is on the collector's own accounting rather than on RSS, so it
//! is deterministic: rebuild the generation N times and require the tracked
//! survivor set to stay flat instead of growing by a generation each round.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState};
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

fn fixtures() -> HashMap<String, Vec<u8>> {
    let mut f = HashMap::new();
    f.insert(
        "Application.cfc".into(),
        b"component { this.name = \"gcScopePin\"; }".to_vec(),
    );
    // Defining closures in init() is what makes the instance hold its live,
    // TRACKED variables scope.
    f.insert(
        "Svc.cfc".into(),
        br#"component {
    function init( id ) {
        variables.id = arguments.id;
        variables.reader = function() { return variables.id; };
        return this;
    }
    function wire( peers ) { variables.peers = arguments.peers; }
    function read() { return variables.reader(); }
}"#
        .to_vec(),
    );
    // Each request mints a whole new generation and displaces the previous one.
    f.insert(
        "gen.cfm".into(),
        br#"<cfscript>
reg = {};
for ( i = 1; i <= 25; i++ ) { reg[ "svc" & i ] = new Svc( "svc" & i ); }
for ( k in reg ) { reg[ k ].wire( reg ); }
application.generation = reg;
writeOutput( reg.svc1.read() );
</cfscript>"#
            .to_vec(),
    );
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

fn run_request(server_state: &ServerState, vfs: Arc<dyn Vfs>, page: &str) -> String {
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

    cfml_common::cycle_gc::arm();
    cfml_common::cycle_gc::enable();
    vm.execute_with_lifecycle().unwrap();
    let out = vm.output_buffer.trim().to_string();
    drop(vm); // transient roots gone before collecting, as serve mode does
    cfml_common::cycle_gc::collect();
    out
}

#[test]
fn a_displaced_generation_of_components_is_reclaimed() {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(fixtures(), VROOT.to_string()));
    let server_state = ServerState::new();

    let mut tracked = Vec::new();
    for _ in 0..6 {
        assert_eq!(run_request(&server_state, Arc::clone(&vfs), "gen.cfm"), "svc1");
        cfml_common::cycle_gc::sweep_persistent();
        tracked.push(cfml_common::cycle_gc::persistent_tracked());
    }

    // Requests 1-2 settle (the application scope and its first generation become
    // tracked); from there the set must not grow by a generation per request.
    let settled = tracked[2];
    let last = *tracked.last().unwrap();
    assert!(
        last <= settled + settled / 4,
        "the tracked survivor set grew per request — a displaced generation is \
         being pinned, not reclaimed. Counts across requests: {:?}",
        tracked
    );
}
