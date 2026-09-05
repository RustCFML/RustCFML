//! A cycle that becomes garbage AFTER the request that allocated it is still
//! reclaimed.
//!
//! `collect()` used to drain the request's allocation log and drop it, so every
//! survivor — everything that escaped into application/session/server scope, and
//! everything those graphs reach — stopped being tracked the instant the request
//! ended (~206,000 nodes per request on a live Preside page). Nothing ever
//! looked at them again, and refcounting cannot free a cycle, so any of them
//! that later became garbage leaked for the life of the process.
//!
//! The `relog_cycle_nodes` hook only covers a value displaced from a struct
//! explicitly flagged as a persistent scope. This test deliberately uses NO
//! flagged scope at all: the graph is dropped one level down, from a plain
//! struct, which is the case a mutation hook structurally cannot see.

use cfml_common::cycle_gc;
use cfml_common::dynamic::{CfmlStruct, CfmlValue, ValueMap};

/// `holder -> a <-> b`, returning the holder and a `Weak` to the cycle.
fn build(holder: &CfmlStruct) -> std::sync::Weak<parking_lot::RwLock<cfml_common::dynamic::StructInner>> {
    let a = CfmlStruct::new(ValueMap::with_capacity(0));
    let b = CfmlStruct::new(ValueMap::with_capacity(0));
    a.insert("b", CfmlValue::Struct(b.clone()));
    b.insert("a", CfmlValue::Struct(a.clone()));
    holder.insert("cycle", CfmlValue::Struct(a.clone()));
    a.weak_backing()
}

#[test]
fn a_cycle_orphaned_after_its_request_is_still_reclaimed() {
    cycle_gc::arm();

    // Request 1 — allocate the cycle and let it escape. It is live at request
    // end, so the old code stopped tracking it here, permanently.
    cycle_gc::enable();
    let holder = CfmlStruct::new(ValueMap::with_capacity(0));
    let weak = build(&holder);
    cycle_gc::collect();
    assert!(weak.upgrade().is_some(), "still owned by the holder");

    // Request 2 — orphan it from a PLAIN struct. No persistent-scope flag, so no
    // relog hook fires; only the carried-forward survivor set can find this.
    cycle_gc::enable();
    holder.remove("cycle");
    cycle_gc::collect();

    // The cross-request sweep runs on the doubling rule, so force one rather
    // than allocating 50k nodes to trip the budget naturally.
    cycle_gc::sweep_persistent();

    assert!(
        weak.upgrade().is_none(),
        "a cycle orphaned after the request that allocated it was never \
         re-examined — collect() is dropping its survivors instead of carrying \
         them forward"
    );
    cycle_gc::disarm();
}
