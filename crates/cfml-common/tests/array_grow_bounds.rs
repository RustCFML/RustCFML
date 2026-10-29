//! `CfmlArray::set_or_grow` at the top of the `usize` range. On 32-bit wasm a
//! CFML position reaches `usize::MAX`; the grow count wrapped to 0 there,
//! passed the growth cap, and `resize` aborted the process.

use cfml_common::dynamic::{CfmlArray, CfmlValue};

#[test]
fn a_position_at_usize_max_is_refused_not_aborted() {
    let a = CfmlArray::new(vec![]);
    assert!(a.set_or_grow(usize::MAX, CfmlValue::Int(1)).is_err());
    assert_eq!(a.len(), 0);
}

#[test]
fn an_in_range_write_still_grows() {
    let a = CfmlArray::new(vec![]);
    a.set_or_grow(2, CfmlValue::Int(1)).unwrap();
    assert_eq!(a.len(), 3);
}
