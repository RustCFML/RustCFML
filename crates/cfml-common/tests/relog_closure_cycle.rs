//! A cyclic graph displaced from a persistent scope is reclaimed even when the
//! cycle runs through a CLOSURE'S CAPTURED SCOPE.
//!
//! The relog hook (`CfmlValue::relog_cycle_nodes`) re-enters a displaced
//! application/session/server-scope value into the current request's collection
//! set, because the collector is otherwise bounded to containers THIS request
//! allocated. It originally walked Structs, Arrays and Instances only — but a
//! closure's `captured_scope` is its own collectible node type
//! (`TrackedAlloc::Scope`), and entering the struct while leaving the scope out
//! is worse than entering neither: the un-entered scope's reference to the
//! struct reads as EXTERNAL ownership, so the collector pins the whole graph
//! live. That is the exact shape a DI container has (WireBox wires providers as
//! closures over the injector), and it left ~2.75 MB per request stranded on a
//! synthetic rebuild-the-registry loop that is now flat.
//!
//! The assertion is on OBSERVED RECLAMATION, not on a byte count: hold a `Weak`
//! to the closure scope, drop every strong handle the test owns, run the pass,
//! and require the cycle to be gone.

use cfml_common::cycle_gc;
use cfml_common::dynamic::{
    CfmlAccess, CfmlClosureBody, CfmlFunction, CfmlStruct, CfmlValue, ValueMap,
};
use std::sync::Arc;

/// `registry -> service -> closure -> captured scope -> registry`.
/// Returns the app scope holding it plus a `Weak` to the closure scope, which is
/// the node that only survives if the cycle was NOT collected.
fn build_cycle() -> (CfmlStruct, std::sync::Weak<std::sync::RwLock<ValueMap>>) {
    let app = CfmlStruct::new(ValueMap::with_capacity(0));
    app.mark_persistent_scope();

    let registry = CfmlStruct::new(ValueMap::with_capacity(0));
    let scope = cycle_gc::tracked_scope(ValueMap::with_capacity(0));
    // The closure captures the registry — the back edge that closes the cycle.
    scope
        .write()
        .unwrap()
        .insert("registry", CfmlValue::Struct(registry.clone()));

    let closure = CfmlValue::Function(Arc::new(CfmlFunction {
        name: "provider".to_string(),
        params: vec![],
        body: CfmlClosureBody::Expression(Box::new(CfmlValue::Null)),
        return_type: None,
        access: CfmlAccess::Public,
        captured_scope: Some(Arc::clone(&scope)),
    }));

    let service = CfmlStruct::new(ValueMap::with_capacity(0));
    service.insert("provider", closure);
    registry.insert("svc1", CfmlValue::Struct(service));
    app.insert("registry", CfmlValue::Struct(registry));

    (app, Arc::downgrade(&scope))
}

#[test]
fn displaced_closure_cycle_is_reclaimed() {
    cycle_gc::arm();

    // --- Request 1: the graph is built and escapes into the persistent scope.
    cycle_gc::enable();
    let (app, weak_scope) = build_cycle();
    cycle_gc::collect();
    assert!(
        weak_scope.upgrade().is_some(),
        "the graph is owned by the app scope and must survive request 1"
    );

    // --- Request 2: the registry is replaced, exactly as a framework reload
    // does. Nothing in this graph is in request 2's survivor set, so only the
    // relog hook on the persistent scope's `insert` can enter it.
    cycle_gc::enable();
    app.insert("registry", CfmlValue::String("rebuilt".to_string().into()));
    cycle_gc::collect();

    assert!(
        weak_scope.upgrade().is_none(),
        "the displaced closure cycle was not reclaimed — relog_cycle_nodes is \
         not entering Function captured scopes, so the scope's reference back \
         into the graph reads as external and pins it"
    );
    cycle_gc::disarm();
}
