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

// ---------------------------------------------------------------------------
// Phase C.2 prototype: flyweight backing (feature-gated, OFF by default).
//
// `ClassBlueprint` is built once per CFC and shared (Arc) across ALL instances;
// `Instance` is the thin per-instance value. The current marker-struct
// representation duplicates, per instance, two full scope maps each carrying an
// entry for every method plus the class metadata — those move onto the shared
// blueprint here. NOT yet wired to a `CfmlValue::Instance` variant or a producer
// (see COMPONENT_MODEL_PHASE_C2_PROTOTYPE.md, steps C.2.1/C.2.2); these types
// exist so the scaffolding compiles under the flag before that surgery.
// Minimal on purpose: parent chain / properties / static scope / rust_extends
// are deferred to the full C.2.
// ---------------------------------------------------------------------------

/// One per CFC file. Immutable after build; `Arc`-shared across every instance
/// and request. Holds the class-invariant bulk (methods + metadata) that the
/// marker-struct representation currently copies into each instance.
#[cfg(feature = "component-instance")]
#[allow(dead_code)]
pub struct ClassBlueprint {
    /// Dotted component name (`__name`).
    pub name: String,
    /// Source file the class was loaded from (super keying / heal rehoming).
    pub source_file: String,
    /// Shared method table (public + private), stored ONCE per class rather than
    /// duplicated into each instance's `this`/`variables` maps.
    pub methods: indexmap::IndexMap<String, std::sync::Arc<crate::dynamic::CfmlFunction>>,
    /// Per-method access modifier, for gating EXTERNAL calls (the Lucee rule:
    /// all methods visible in both scopes, but only public/remote callable from
    /// outside `this`/`super`).
    pub method_access: rustc_hash::FxHashMap<String, crate::dynamic::CfmlAccess>,
    /// The class-invariant metadata blob (reuses the existing per-class shape).
    pub metadata: CfmlValue,
}

/// Thin, per-instance value. Revives `CfmlValue::Component` conceptually as an
/// `Arc<RwLock<Instance>>` with real `Arc::ptr_eq` identity (added in C.2.1).
#[cfg(feature = "component-instance")]
#[allow(dead_code)]
pub struct Instance {
    /// Shared blueprint — zero per-instance copy.
    pub class: std::sync::Arc<ClassBlueprint>,
    /// Public DATA members only (Lucee `_data`); no methods, no `__*` metadata.
    pub this_members: CfmlStruct,
    /// Private DATA members only (Lucee `shadow`); independent map (see §core).
    pub variables_members: CfmlStruct,
    /// Logical identity for `duplicate()` disambiguation / fluent-chain guards.
    pub instance_id: u64,
}

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

    /// See the free function [`type_identifiers`].
    #[inline]
    pub fn type_identifiers(&self) -> Vec<String> {
        type_identifiers(self.backing)
    }
}

/// The dotted type identifiers a component instance satisfies for `isInstanceOf`
/// / type checks: its own class name (`__name`), its resolved superclass chain
/// (`__extends_chain`), and its interface lists (`__implements`,
/// `__implements_chain`, `__implements_fqns`). Names are returned as stored
/// (original case); callers compare case-insensitively.
///
/// This is the canonical read of the introspection keys — the single place that
/// knows their names, so the representation can move under it (C.3/C.4) without
/// touching consumers.
///
/// CAUTION: this deliberately flattens every key with NO per-key distinction, so
/// it is only correct for callers that apply *uniform* matching. A caller that
/// needs different matching per key — notably `isInstanceOf`
/// (`crate`-external `fn_is_instance_of`), which requires path-EXACT matching for
/// `__implements_fqns` (issue #206) while allowing last-segment matches for the
/// others, and also special-cases the base `"component"` type and `__java_class`
/// — must NOT use this and keeps its own walk.
pub fn type_identifiers(s: &CfmlStruct) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(CfmlValue::String(name)) = s.get_ci("__name") {
        ids.push(name.to_string());
    }
    for key in [
        "__extends_chain",
        "__implements",
        "__implements_chain",
        "__implements_fqns",
    ] {
        if let Some(CfmlValue::Array(arr)) = s.get_ci(key) {
            for item in arr.iter() {
                ids.push(item.as_string());
            }
        }
    }
    ids
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
