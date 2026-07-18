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

/// Shared handle to a flyweight instance. `CfmlValue::Instance` wraps one of
/// these; cloning it is an `Arc` refcount bump, and identity comparisons use
/// `Arc::ptr_eq` (revives the old `CfmlValue::Component` identity semantics).
#[cfg(feature = "component-instance")]
pub type InstanceRef = std::sync::Arc<parking_lot::RwLock<Instance>>;

/// One per CFC file. Immutable after build; `Arc`-shared across every instance
/// and request. Holds the class-invariant bulk (methods + metadata) that the
/// marker-struct representation currently copies into each instance.
#[cfg(feature = "component-instance")]
#[allow(dead_code)]
#[derive(Debug)]
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
/// `Arc<RwLock<Instance>>` with real `Arc::ptr_eq` identity.
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

// `CfmlStruct` has no `Debug` impl (the outer `CfmlValue` Debug is hand-rolled),
// so `Instance` can't derive it. Hand-roll a shallow form — class name + id,
// without recursing into the data maps — mirroring the concise style used for
// `NativeObject`.
#[cfg(feature = "component-instance")]
impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance")
            .field("class", &self.class.name)
            .field("instance_id", &self.instance_id)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "component-instance")]
#[allow(dead_code)]
impl Instance {
    /// The dotted type identifiers this instance satisfies (see the free
    /// [`type_identifiers`]). Prototype minimal: the blueprint only carries the
    /// class name so far (parent chain / interfaces are DEFERRED to full C.2),
    /// so leaf/concrete allowlisted classes are answered correctly and anything
    /// needing the chain is out of the prototype's scope.
    pub fn type_identifiers(&self) -> Vec<String> {
        vec![self.class.name.clone()]
    }
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
/// It abstracts over the two possible backings: the legacy marker struct
/// ([`CompRef::Marker`]) and — when the `component-instance` feature is on — the
/// flyweight [`Instance`] ([`CompRef::Instance`]). Consumers go through the
/// accessor methods so the representation can move underneath them (C.3/C.4)
/// without touching call sites. Kept `Copy` (both arms are shared references) so
/// call sites can pass it around freely without lifetime friction.
#[derive(Clone, Copy)]
pub enum CompRef<'a> {
    /// The current representation: a marker [`CfmlValue::Struct`].
    Marker(&'a CfmlStruct),
    /// The flyweight representation: a shared [`Instance`].
    #[cfg(feature = "component-instance")]
    Instance(&'a InstanceRef),
}

impl<'a> CompRef<'a> {
    /// Wrap a struct as a component view iff it is a component instance.
    #[inline]
    pub fn for_struct(s: &'a CfmlStruct) -> Option<CompRef<'a>> {
        if is_component_backing(s) {
            Some(CompRef::Marker(s))
        } else {
            None
        }
    }

    /// Wrap a flyweight instance handle as a component view.
    #[cfg(feature = "component-instance")]
    #[inline]
    pub fn for_instance(inst: &'a InstanceRef) -> CompRef<'a> {
        CompRef::Instance(inst)
    }

    /// The raw marker-struct backing, if this view is marker-backed. This is the
    /// C.1 escape hatch: as later slices add typed accessors (`get_public`,
    /// `get_var`, `lookup_method`, …) the direct-backing uses shrink until C.4
    /// can drop it entirely. Returns `None` for a flyweight-backed view — a
    /// boundary that still needs the old shape must bridge explicitly rather
    /// than assume a marker struct is always present.
    #[inline]
    pub fn backing(&self) -> Option<&'a CfmlStruct> {
        match self {
            CompRef::Marker(s) => Some(s),
            #[cfg(feature = "component-instance")]
            CompRef::Instance(_) => None,
        }
    }

    /// See the free function [`type_identifiers`].
    #[inline]
    pub fn type_identifiers(&self) -> Vec<String> {
        match self {
            CompRef::Marker(s) => type_identifiers(s),
            #[cfg(feature = "component-instance")]
            CompRef::Instance(inst) => inst.read().type_identifiers(),
        }
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
            #[cfg(feature = "component-instance")]
            CfmlValue::Instance(inst) => Some(CompRef::for_instance(inst)),
            _ => None,
        }
    }

    /// Convenience boolean form of [`CfmlValue::as_component`].
    #[inline]
    pub fn is_component(&self) -> bool {
        self.as_component().is_some()
    }
}
