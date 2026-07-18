//! Component-instance facade (Phase C.1).
//!
//! A CFML component instance is currently represented as a marker
//! [`CfmlValue::Struct`] carrying the private `__variables` scope plus a
//! public-scope handle (`this`) or a class name (`__name`). Historically the
//! marker predicate was open-coded in several places
//! (`is_component_struct`/`is_component_backing`/inline `contains_key_ci`
//! checks). This module is the single source of truth for "is this value a
//! component?" and the entry point ([`CfmlValue::as_component`]) that later
//! phases grow into a full read/write facade over — eventually — a dedicated
//! `Instance` backing (Phase C.2+). During C.1 it is implemented purely over
//! the existing marker struct, so it is behaviour-preserving by construction.
//!
//! NOTE: the marker predicate here (`__variables` + (`this` | `__name`)) is the
//! *component* predicate only. Deliberately NOT folded in are the *different*
//! predicates that share the `__*` idiom but mean something else — a bare
//! `__variables`-only check, the `+ __is_super` super-marker, the `+ __java_shim`
//! / `+ __java_class` Java-shim family, and the `+ __properties` class-template
//! checks. Folding those would change behaviour; they are left to their own
//! (later, separate) treatment.

use crate::dynamic::{CfmlStruct, CfmlValue};

/// The canonical component-instance marker predicate: a struct is a component
/// instance iff it carries the private `__variables` scope **and** either a
/// public-scope handle (`this`) or a class name (`__name`). Mid-construction
/// instances always carry `__variables`.
#[inline]
pub fn is_component_backing(s: &CfmlStruct) -> bool {
    s.contains_key_ci("__variables") && (s.contains_key_ci("this") || s.contains_key_ci("__name"))
}

/// Lightweight, borrowed read view over a component instance.
///
/// Phase C.1 backs this with the marker struct; Phase C.2 will add an
/// `Instance` backing and this type becomes the dispatch point. Kept `Copy` so
/// call sites can pass it around freely without lifetime friction.
#[derive(Clone, Copy)]
pub struct CompRef<'a> {
    backing: &'a CfmlStruct,
}

impl<'a> CompRef<'a> {
    /// Wrap a struct as a component view iff it is a component instance.
    #[inline]
    pub fn for_struct(s: &'a CfmlStruct) -> Option<CompRef<'a>> {
        if is_component_backing(s) {
            Some(CompRef { backing: s })
        } else {
            None
        }
    }

    /// The raw backing struct. This is the C.1 escape hatch: as later slices
    /// add typed accessors (`get_public`, `get_var`, `lookup_method`, …) the
    /// direct-backing uses shrink until C.4 can drop it entirely.
    #[inline]
    pub fn backing(&self) -> &'a CfmlStruct {
        self.backing
    }
}

impl CfmlValue {
    /// Component-instance facade entry point. Returns `Some` iff this value is a
    /// component instance. All new code should detect components via this method
    /// rather than open-coding the marker keys.
    #[inline]
    pub fn as_component(&self) -> Option<CompRef<'_>> {
        match self {
            CfmlValue::Struct(s) => CompRef::for_struct(s),
            _ => None,
        }
    }

    /// Convenience boolean form of [`CfmlValue::as_component`].
    #[inline]
    pub fn is_component(&self) -> bool {
        self.as_component().is_some()
    }
}
