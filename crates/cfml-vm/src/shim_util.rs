//! Helpers every Java-class shim needs. Each `*_shim.rs` used to carry its own
//! byte-identical copy of these.

use cfml_common::dynamic::{CfmlValue, ValueMap};

/// A bare shim marker map for `class`: what `createObject("java", class)`
/// returns before the shim adds its own state.
pub(crate) fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

/// A field of a shim instance (a struct), or `None` for anything else.
pub(crate) fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}
