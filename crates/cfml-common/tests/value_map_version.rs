//! `ValueMap::version()` must change on every `&mut` path. The VM's
//! `FrameScopeCache` and the closure write-back skip treat an unchanged
//! version as "nothing was written", so a path that mutates without bumping
//! it is a silently lost write.

use cfml_common::dynamic::{CfmlValue, ValueMap};

fn seeded() -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("a", CfmlValue::Int(1));
    m
}

#[test]
fn extend_bumps_the_version() {
    let mut m = seeded();
    let before = m.version();
    m.extend([("b", CfmlValue::Int(2))]);
    assert_ne!(m.version(), before);
}

#[test]
fn mutable_iteration_bumps_the_version() {
    let mut m = seeded();
    let before = m.version();
    for (_, v) in &mut m {
        *v = CfmlValue::Int(9);
    }
    assert_ne!(m.version(), before);
}

#[test]
fn reads_leave_the_version_alone() {
    let m = seeded();
    let before = m.version();
    for _ in &m {}
    let _ = m.get("a");
    assert_eq!(m.version(), before);
}
