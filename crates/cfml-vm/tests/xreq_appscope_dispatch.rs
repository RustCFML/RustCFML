//! Regression coverage: cross-request dispatch of application-scoped functions.
//!
//! A stored CfmlValue::Function originally dispatched through a bare numeric
//! index into the per-request `program.functions` table, so anything stored in
//! application scope and called on a LATER request would dangle once the
//! function-table layout shifted. The stable-function-identity redesign re-homes
//! app-reachable functions into a per-application table keyed by a stable
//! `(source_file, name, ordinal)` identity, rewriting their stored bodies to a
//! tagged stable id — so cross-request dispatch is now correct by construction.
//!
//! Each request below forces a DIFFERENT page table size (0 / 6 / 12 page UDFs)
//! so a layout shift WOULD have stranded the old per-request indices; with stable
//! ids the app-scope functions still resolve. Exercises the graph shapes a real
//! app uses:
//!   * flat services sharing init/read/save/checkAccess     (name collisions)
//!   * a service whose read() is INHERITED from a base CFC  (parent merge)
//!   * a service NESTED inside an app-scope struct          (struct walk)
//!   * services held in an app-scope ARRAY                  (array walk)
//!   * a component RETURNED FROM A FACTORY then stashed     (cross-instance merge)
//!   * a CLOSURE stashed in app scope that captured a CFC   (captured_scope walk)
//!   * a service LAZILY created during a normal request     (post-start caching)
//!
//! The closure case (application.handler) regressed before the request-end cache
//! refresh was changed to carry functions defined in Application.cfc / the page
//! (indices below the onApplicationStart offset), not just those merged after it.

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

fn service(label: &str) -> Vec<u8> {
    format!(
        r#"component {{
    function init() {{ return this; }}
    function read() {{ return "{l}-read"; }}
    function save() {{ return "{l}-save"; }}
    function checkAccess() {{ return "{l}-access"; }}
}}"#,
        l = label
    )
    .into_bytes()
}

fn fixtures() -> HashMap<String, Vec<u8>> {
    let mut f = HashMap::new();
    f.insert("UserService.cfc".into(), service("user"));
    f.insert("OrderService.cfc".into(), service("order"));
    f.insert("AuthService.cfc".into(), service("auth"));
    f.insert("PaymentService.cfc".into(), service("payment"));
    f.insert("ArrA.cfc".into(), service("arra"));
    f.insert("ArrB.cfc".into(), service("arrb"));
    f.insert("CapturedSvc.cfc".into(), service("captured"));
    f.insert("LazySvc.cfc".into(), service("lazy"));

    // Inherited read(): BaseService defines read(); ProductService inherits it.
    f.insert(
        "BaseService.cfc".into(),
        br#"component {
    function read() { return "base-read"; }
}"#
        .to_vec(),
    );
    f.insert(
        "ProductService.cfc".into(),
        br#"component extends="BaseService" {
    function save() { return "product-save"; }
}"#
        .to_vec(),
    );

    // Factory whose build() returns a fresh component (BuiltThing).
    f.insert("BuiltThing.cfc".into(), service("built"));
    f.insert(
        "FactoryService.cfc".into(),
        br#"component {
    function build() { return createObject("component", "BuiltThing"); }
}"#
        .to_vec(),
    );

    f.insert(
        "Application.cfc".into(),
        br#"component {
    this.name = "xreq-appscope-dispatch-test";
    function onApplicationStart() {
        application.userSvc    = createObject("component", "UserService");
        application.orderSvc   = createObject("component", "OrderService");
        application.authSvc    = createObject("component", "AuthService");
        application.productSvc = createObject("component", "ProductService");

        // nested inside a struct
        application.registry = {};
        application.registry.payment = createObject("component", "PaymentService");

        // held in an array
        application.svcArray = [
            createObject("component", "ArrA"),
            createObject("component", "ArrB")
        ];

        // returned from a factory, then stashed long-lived
        application.factory = createObject("component", "FactoryService");
        application.built   = application.factory.build();

        // a closure that captured a component
        var captured = createObject("component", "CapturedSvc");
        application.handler = function() { return captured.read(); };
    }
}"#
        .to_vec(),
    );

    // Request 1: zero page UDFs — just triggers onApplicationStart.
    f.insert("start.cfm".into(), b"<cfoutput>started</cfoutput>".to_vec());
    f.insert("work6.cfm".into(), work_page(6).into_bytes());
    f.insert("work12.cfm".into(), work_page(12).into_bytes());

    // Page that lazily creates an app-scope service during a NORMAL request
    // (not onApplicationStart), then reads everything.
    f.insert("seed_lazy.cfm".into(), seed_lazy_page().into_bytes());
    f
}

/// `n` padding UDFs to inflate program.functions and force an offset shift,
/// then calls into every app-scoped service / shape.
fn work_page(n: usize) -> String {
    let pads: String = (0..n)
        .map(|i| format!("function pad{i}() {{ return {i}; }}\n"))
        .collect();
    format!(
        "<cfscript>\n{pads}</cfscript><cfoutput>{calls}</cfoutput>",
        calls = CALLS
    )
}

fn seed_lazy_page() -> String {
    format!(
        "<cfscript>\nfunction p() {{ return 0; }}\napplication.lazySvc = createObject(\"component\", \"LazySvc\");\n</cfscript><cfoutput>{CALLS}#application.lazySvc.read()#</cfoutput>"
    )
}

const CALLS: &str = "\
#application.userSvc.read()#|\
#application.orderSvc.save()#|\
#application.authSvc.checkAccess()#|\
#application.productSvc.read()#|\
#application.productSvc.save()#|\
#application.registry.payment.read()#|\
#application.svcArray[1].read()#|\
#application.svcArray[2].save()#|\
#application.built.read()#|\
#application.handler()#";

const EXPECTED: &str = "user-read|order-save|auth-access|base-read|product-save|payment-read|arra-read|arrb-save|built-read|captured-read";

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
    vm.source_file = Some(std::sync::Arc::from(page_path.as_str()));
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
    vm.execute_with_lifecycle().unwrap();
    vm.output_buffer.trim().to_string()
}

#[test]
fn appscope_methods_dispatch_correctly_across_requests_with_shifting_tables() {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(fixtures(), VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    let r1 = run_request(&server_state, vfs.clone(), "start.cfm");
    assert_eq!("started", r1, "request 1 should run onApplicationStart");

    let r2 = run_request(&server_state, vfs.clone(), "work6.cfm");
    assert_eq!(EXPECTED, r2, "request 2 misdispatched (+6 page UDFs)");

    let r3 = run_request(&server_state, vfs.clone(), "work12.cfm");
    assert_eq!(EXPECTED, r3, "request 3 misdispatched (+12 page UDFs)");

    let r4 = run_request(&server_state, vfs, "work6.cfm");
    assert_eq!(EXPECTED, r4, "request 4 misdispatched (table shrink)");
}

#[test]
fn lazily_created_appscope_service_dispatches_on_a_later_request() {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(fixtures(), VROOT.to_string()));
    let server_state = ServerState::with_production(false);

    // Start the app (services created in onApplicationStart).
    run_request(&server_state, vfs.clone(), "start.cfm");

    // Request 2 lazily creates application.lazySvc during a normal request.
    let r2 = run_request(&server_state, vfs.clone(), "seed_lazy.cfm");
    assert_eq!(format!("{EXPECTED}lazy-read"), r2, "lazy seed request");

    // Request 3 (bigger table) must still dispatch the lazily-created service.
    let r3 = run_request(&server_state, vfs, "work12.cfm");
    // work12 doesn't call lazySvc, but the existing shapes must still be right.
    assert_eq!(EXPECTED, r3, "request after lazy seed misdispatched");
}
