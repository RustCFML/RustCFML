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
    /// The method table as a shared `Arc<ValueMap>` of `name -> Function`, ready to
    /// hang off the instance data maps via [`CfmlStruct::set_method_table`]. This is
    /// the mechanism that lets unscoped sibling-method resolution and `this.method`
    /// dispatch find methods without copying them per instance: `get_ci` on a data
    /// map falls through to this table on a miss. Built once per class.
    pub method_values: std::sync::Arc<crate::dynamic::ValueMap>,
    /// Lazily-built full `getMetadata()` result (name/extends/functions/properties/
    /// implements), cached once per class (Phase C.3 Slice 5). Filled on the first
    /// `getMetadata(instance)` via the same builder the marker path uses, so the
    /// output matches exactly. `None` until first requested.
    pub metadata_cache: parking_lot::RwLock<Option<CfmlValue>>,
    /// The dotted type identifiers this class satisfies for `isInstanceOf` / typed
    /// argument validation: own name + resolved superclass chain + interface lists
    /// (Phase C.3 Slice 5). Precomputed once from the finished marker via the free
    /// [`type_identifiers`] so `Instance::type_identifiers` no longer under-reports.
    pub type_ids: Vec<String>,
    /// The shared per-class `static` scope (`__variables.__static`), if the class
    /// declares one (Phase C.3 Slice 5). Held as the original (Arc-backed) struct so
    /// every instance's methods see the SAME static store and mutations persist
    /// across instances. Injected into the method frame as `__static` at dispatch.
    pub static_scope: Option<CfmlValue>,
    /// The immediate parent's super-dispatch struct (marker `__super`: a
    /// `__is_super`-tagged struct carrying the parent's methods), captured once per
    /// class (Phase C.3 Slice 5). `super.method()` inside an instance method pushes
    /// this (or the level-specific entry from [`super_map`](Self::super_map)) and
    /// reuses the existing `__is_super` dispatch, which binds `this` to the live
    /// child instance from the frame.
    pub super_handle: Option<CfmlValue>,
    /// Per-defining-source parent structs (marker `__super_map`: source_file ->
    /// parent super struct) for multi-level `super` resolution — `super` resolves
    /// relative to the DEFINING class of the executing method (Phase C.3 Slice 5).
    pub super_map: Option<CfmlValue>,
    /// The marker `__source_names` map (source_file -> logical dotted name recorded
    /// at inheritance-merge time). Needed so an unqualified `new X()` executed
    /// inside an instance method resolves relative to the DEFINING class's
    /// mapping-qualified package (GH #229/#237 + the mapping-prefix specs). Absent
    /// for a class with no inheritance.
    pub source_names: Option<CfmlValue>,
    /// The marker `__properties` array (declared `property name=…` list, including
    /// inherited). Used by `serializeJSON` to include accessor-property values that
    /// live only in the private `variables` scope — default-only and inherited
    /// properties (GH #267). Absent for a component with no declared properties.
    pub properties: Option<CfmlValue>,
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
    /// Accessor-`property` names whose VALUES live in `this_members` but must stay
    /// HIDDEN from user introspection (`structKeyExists`/`structKeyList`/`for … in`)
    /// — Lucee keeps accessor values in the private `variables` scope; getX() and
    /// `serializeJSON` still read them. Lowercased. Interior-mutable because a
    /// runtime `setX()` marks the property after construction (see the
    /// `MarkAccessorPrivate` opcode). Mirrors the marker `__cfml_accessor_private__`.
    pub accessor_private: parking_lot::RwLock<std::collections::HashSet<String>>,
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
    /// The dotted type identifiers this instance satisfies (own name + superclass
    /// chain + interface lists), precomputed on the blueprint at produce time (see
    /// the free [`type_identifiers`]). Used by `isInstanceOf` / typed-argument
    /// validation.
    pub fn type_identifiers(&self) -> Vec<String> {
        self.class.type_ids.clone()
    }
}

// ---------------------------------------------------------------------------
// Phase C.3 — Slice 2: the `Instance` producer.
//
// Partition a *finished* marker instance (post-init, post-inheritance) into the
// flyweight form: the class-invariant bulk (methods + metadata) onto a shared
// `ClassBlueprint`, the per-instance user DATA into two data-only maps.
//
// The producer is the §5.1 "data-loss landmine" and the §5.2/C.2.3 self-reference
// audit in `C2planDoc.md`, so the routing rules are load-bearing:
//   * DATA vs bookkeeping is decided by the EXACT reserved set
//     (`is_reserved_component_key`), NEVER by a `starts_with("__")` prefix. `__`/
//     `___` are legal identifiers frameworks use for real data (FW/1 AOP's
//     `___doReverse`); a prefix test would DISCARD them — strictly worse than the
//     marker path's hiding.
//   * Methods move to the shared blueprint (not copied per instance).
//   * A data member that is the marker itself (the classic `variables[classname]
//     = this` self-reference) is retargeted to the live instance handle rather
//     than retained as a stale marker clone.
//
// Feature-gated OFF by default; wired at instantiation finalize in `cfml-vm`.
// The VM owns the per-`__source_file` blueprint cache and calls
// [`make_instance_value`].
// ---------------------------------------------------------------------------

#[cfg(feature = "component-instance")]
#[allow(dead_code)]
impl ClassBlueprint {
    /// Build the class-invariant blueprint from a finished marker instance.
    ///
    /// Collects the shared method table (public + private, deduped by name) plus
    /// the class name / source file / metadata blob. Everything read here is
    /// invariant across instances of the class, so the producer `Arc`-shares the
    /// result via its per-`__source_file` cache. Per-instance DATA is deliberately
    /// NOT read here — it belongs to [`Instance::from_marker`].
    pub fn from_marker(marker: &CfmlStruct) -> ClassBlueprint {
        use crate::dynamic::{CfmlFunction, CfmlAccess};
        let mut methods: indexmap::IndexMap<String, std::sync::Arc<CfmlFunction>> =
            indexmap::IndexMap::new();
        let mut method_access: rustc_hash::FxHashMap<String, CfmlAccess> =
            rustc_hash::FxHashMap::default();
        let mut name = String::new();
        let mut source_file = String::new();
        let mut metadata = CfmlValue::Null;

        // Helper: harvest Function values from a scope's DATA map AND its shared
        // method table (the marker-path per-class flyweight, v0.506/507, keeps the
        // methods in the table rather than the map — a map-only read would find
        // ZERO methods on a finished instance).
        let mut harvest = |scope: &CfmlStruct| {
            scope.with_read(|m| {
                for (k, v) in m.iter() {
                    if let CfmlValue::Function(f) = v {
                        method_access
                            .entry(k.to_ascii_lowercase())
                            .or_insert_with(|| f.access.clone());
                        methods.entry(k.clone()).or_insert_with(|| f.clone());
                    }
                }
            });
            if let Some(table) = scope.method_table() {
                for (k, v) in table.iter() {
                    if let CfmlValue::Function(f) = v {
                        method_access
                            .entry(k.to_ascii_lowercase())
                            .or_insert_with(|| f.access.clone());
                        methods.entry(k.clone()).or_insert_with(|| f.clone());
                    }
                }
            }
        };
        // Public (top-level) scope, then the private `__variables` scope. §core
        // surfaces the full method set in both scopes, so the public view is
        // normally complete; the private sweep is defensive (a private-only method).
        harvest(marker);
        if let Some(CfmlValue::Struct(vars)) = marker.get_ci("__variables") {
            harvest(&vars);
        }
        marker.with_read(|m| {
            if let Some(CfmlValue::String(n)) = m.get("__name") {
                name = n.to_string();
            }
            if let Some(CfmlValue::String(sf)) = m.get("__source_file") {
                source_file = sf.to_string();
            }
            if let Some(md) = m.get("__metadata") {
                metadata = md.clone();
            }
        });

        // Materialize the method table once as an Arc<ValueMap> of Function values,
        // ready to share onto every instance's data maps (set_method_table).
        let mut mv = crate::dynamic::ValueMap::default();
        for (k, f) in &methods {
            mv.insert(k.clone(), CfmlValue::Function(f.clone()));
        }

        let type_ids = type_identifiers(marker);

        // The shared per-class static scope lives at `__variables.__static`; capture
        // the original Arc-backed struct so every instance shares it.
        let static_scope = match marker.get_ci("__variables") {
            Some(CfmlValue::Struct(vars)) => vars.get_ci("__static"),
            _ => None,
        };

        // Super-dispatch handles (reserved keys, captured before they are dropped
        // from the data maps) — reused by the marker `__is_super` dispatch path.
        let super_handle = marker.get_ci("__super");
        let super_map = marker.get_ci("__super_map");
        let source_names = marker.get_ci("__source_names");
        let properties = marker.get_ci("__properties");

        ClassBlueprint {
            name,
            source_file,
            methods,
            method_access,
            metadata,
            method_values: std::sync::Arc::new(mv),
            metadata_cache: parking_lot::RwLock::new(None),
            type_ids,
            static_scope,
            super_handle,
            super_map,
            source_names,
            properties,
        }
    }
}

#[cfg(feature = "component-instance")]
#[allow(dead_code)]
impl Instance {
    /// Partition a finished marker instance into the flyweight two-map form.
    ///
    /// **§5.1 data-loss landmine:** the DATA/bookkeeping split is by the EXACT
    /// reserved set ([`is_reserved_component_key`]), NEVER by a `starts_with("__")`
    /// prefix. A user/framework `__`/`___` member is ordinary DATA and MUST land in
    /// the data maps. Methods (shared on the blueprint) and the `this`/`super`
    /// scope handles (re-derived, not data) are also excluded from the data maps.
    pub fn from_marker(
        marker: &CfmlStruct,
        class: std::sync::Arc<ClassBlueprint>,
        instance_id: u64,
    ) -> Instance {
        let this_members = partition_data_map(marker);
        let variables_members = match marker.get_ci("__variables") {
            Some(CfmlValue::Struct(vars)) => partition_data_map(&vars),
            _ => CfmlStruct::empty(),
        };
        // Hang the shared blueprint method table off BOTH data maps so `get_ci`
        // falls through to methods on a data miss — this is what makes `this.foo()`,
        // unscoped `foo()`, and `variables.foo()` dispatch resolve without copying
        // the method wrappers per instance. Data members still shadow (map wins in
        // `get_ci`). Enumeration (`iter()`) is map-only, so introspection stays
        // data-clean (Slice 4).
        this_members.set_method_table(class.method_values.clone());
        variables_members.set_method_table(class.method_values.clone());
        // Re-attach the shared per-class `static` scope under the private scope so
        // `find_static_scope` (`variables.__static`) resolves `static.X` reads/writes
        // inside methods. Shared Arc → mutations persist across instances. It is a
        // reserved key, so it never surfaces in public enumeration (which reads
        // `this_members`).
        if let Some(ref stat) = class.static_scope {
            variables_members.insert("__static".to_string(), stat.clone());
        }
        // Live `variables.this` alias (Lucee/ACF parity): the private scope carries
        // a WEAK back-edge to the public `this` scope, so `variables.this.x = v` and
        // `StructAppend(variables.this, fns)` reach the public members and
        // `StructKeyExists(variables, "this")` is true (Wheels Plugins mixin
        // injection). Weak ⇒ no Arc cycle ⇒ no per-request leak — the same mechanism
        // the marker path used, reused here rather than a strong self-reference.
        variables_members.set_this_alias_if_changed(&this_members);
        // Capture the construction-time accessor-private property set (the marker's
        // `__cfml_accessor_private__`, a case-insensitive name set) so introspection
        // hides those public-map values exactly as the marker path did.
        let mut accessor_private = std::collections::HashSet::new();
        if let Some(CfmlValue::Struct(m)) = marker.get_ci(crate::dynamic::ACCESSOR_PRIVATE_MARKER) {
            m.with_read(|mm| {
                for (k, _) in mm.iter() {
                    accessor_private.insert(k.to_ascii_lowercase());
                }
            });
        }
        Instance {
            class,
            this_members,
            variables_members,
            instance_id,
            accessor_private: parking_lot::RwLock::new(accessor_private),
        }
    }

    /// Read a member for property access (`obj.name` / `obj["name"]`): public data
    /// first, then private data, each `get_ci`-resolved so a method found via the
    /// shared table is returned as its `Function` value. `None` on a genuine miss
    /// (the caller decides Null vs throw). Mirrors the marker path's lenient
    /// public→variables fallthrough (RustCFML does not gate data-member access).
    pub fn get_member(&self, name: &str) -> Option<CfmlValue> {
        // Compat shim: `instance.__variables` exposes the private scope as a struct,
        // the way the marker representation did (a few RustCFML tests / helpers poke
        // it directly). Not a Lucee-visible member, but harmless and marker-parity.
        if name.eq_ignore_ascii_case("__variables") {
            return Some(CfmlValue::Struct(self.variables_members.clone()));
        }
        self.this_members
            .get_ci(name)
            .or_else(|| self.variables_members.get_ci(name))
    }

    /// Resolve a callable method: an injected/mixin data-member function (public
    /// then private) shadows the shared blueprint table, exactly as `get_ci`'s
    /// map-before-table order gives us for free.
    pub fn lookup_method(&self, name: &str) -> Option<CfmlValue> {
        self.this_members
            .get_ci(name)
            .filter(|v| matches!(v, CfmlValue::Function(_)))
            .or_else(|| {
                self.variables_members
                    .get_ci(name)
                    .filter(|v| matches!(v, CfmlValue::Function(_)))
            })
    }
}

/// Extract the pure user-DATA subset of a component scope map: drop methods
/// (shared on the blueprint), reserved bookkeeping keys (§3), and the `this`/
/// `super` scope handles. Everything surviving — INCLUDING `__`/`___` user keys —
/// is copied verbatim. See §5.1: partition by the reserved SET, never by prefix.
#[cfg(feature = "component-instance")]
fn partition_data_map(scope: &CfmlStruct) -> CfmlStruct {
    let mut data = crate::dynamic::ValueMap::default();
    scope.with_read(|m| {
        for (k, v) in m.iter() {
            if matches!(v, CfmlValue::Function(_)) {
                continue; // method → shared blueprint
            }
            if is_reserved_component_key(k) {
                continue; // engine bookkeeping → blueprint / typed Instance fields
            }
            if k.eq_ignore_ascii_case("this") || k.eq_ignore_ascii_case("super") {
                continue; // scope handle, re-derived on dispatch (not data)
            }
            data.insert(k.clone(), v.clone());
        }
    });
    CfmlStruct::new(data)
}

/// Build a [`CfmlValue::Instance`] from a finished marker instance and its
/// (VM-cached) blueprint.
///
/// Performs the §5.2/C.2.3 self-reference fixup: a top-level data member that IS
/// the marker itself (the classic `variables[classname] = this`) is retargeted to
/// the live instance handle. Left as a plain clone it would both retain a whole
/// stale marker AND read back as a marker struct instead of this instance.
#[cfg(feature = "component-instance")]
pub fn make_instance_value(
    marker: &CfmlStruct,
    class: std::sync::Arc<ClassBlueprint>,
    instance_id: u64,
) -> CfmlValue {
    let inst = Instance::from_marker(marker, class, instance_id);
    let handle: InstanceRef = std::sync::Arc::new(parking_lot::RwLock::new(inst));
    let marker_ptr = marker.backing_ptr();
    let self_val = CfmlValue::Instance(handle.clone());
    {
        let g = handle.read();
        fixup_self_ref(&g.this_members, marker_ptr, &self_val);
        fixup_self_ref(&g.variables_members, marker_ptr, &self_val);
    }
    CfmlValue::Instance(handle)
}

/// Retarget any top-level data member whose struct backing IS `marker_ptr` to the
/// live instance handle (§5.2 self-reference audit). Shallow by design: the
/// documented C.2.3 case is the direct `variables[classname] = this`; deeper
/// cycles are the cycle collector's concern, not the producer's.
#[cfg(feature = "component-instance")]
fn fixup_self_ref(scope: &CfmlStruct, marker_ptr: usize, self_val: &CfmlValue) {
    scope.with_write(|m| {
        for (_, v) in m.iter_mut() {
            if let CfmlValue::Struct(s) = v {
                if s.backing_ptr() == marker_ptr {
                    *v = self_val.clone();
                }
            }
        }
    });
}

/// The canonical component-instance marker predicate: a struct is a component
/// instance iff it carries the private `__variables` scope **and** either a
/// public-scope handle (`this`) or a class name (`__name`). Mid-construction
/// instances always carry `__variables`.
#[inline]
pub fn is_component_backing(s: &CfmlStruct) -> bool {
    s.contains_key_ci("__variables") && (s.contains_key_ci("this") || s.contains_key_ci("__name"))
}

/// True iff `a` and `b` are the SAME flyweight component instance (Arc identity).
/// Lucee/ACF compare CFC instances by reference, so this is the flyweight analog of
/// the marker path's `CfmlStruct::backing_ptr()` identity check — used by `===`,
/// `arrayContains`/`arrayFind`, and any component identity test. A feature-flag-free
/// facade so `cfml-stdlib` (which has no `component-instance` feature and cannot match
/// `CfmlValue::Instance`) can call it; always `false` in a default (marker-only) build.
#[inline]
pub fn same_component_instance(a: &CfmlValue, b: &CfmlValue) -> bool {
    #[cfg(feature = "component-instance")]
    {
        matches!((a, b), (CfmlValue::Instance(x), CfmlValue::Instance(y)) if std::sync::Arc::ptr_eq(x, y))
    }
    #[cfg(not(feature = "component-instance"))]
    {
        let _ = (a, b);
        false
    }
}

/// True iff `k` is an engine-reserved component-instance bookkeeping key that
/// must stay OUT of user-visible introspection (`structKeyExists`/`structKeyList`/
/// `structKeyArray`/`structCount`/`structEach`, `for … in`, `serializeJSON`,
/// `writeDump`, `getMetadata`).
///
/// This is the single source of truth for the reserved set (Phase C.3/C.4). It is
/// deliberately an EXACT set plus a few structured prefixes — **NOT** a blanket
/// `starts_with("__")` test. `__`/`___` are legal CFML identifiers and real
/// frameworks store data members under them (FW/1 DI/1 & AOP/1 use e.g.
/// `this["___doReverse"]`); a prefix test wrongly hides — or, in the C.3 producer
/// partition, wrongly DISCARDS — such user data. Lucee never had this problem
/// because its engine internals are Java object fields, never struct members.
///
/// Authoritative list: `C2planDoc.md` §3 / `COMPONENT_MODEL_PHASE_C0_CENSUS.md`.
/// Uses:
///  - C.3 producer partition (route ONLY these keys to the blueprint / typed
///    `Instance` fields; everything else is user data → `this_members`/
///    `variables_members`),
///  - C.4 deletion of the blanket `starts_with("__")` filters.
///
/// SCOPE: component-instance members only. The `__java_shim`-gated Java-facade
/// filters and the call-frame `__arguments__` scope key are SEPARATE concerns with
/// their own (unchanged) handling — see `C2planDoc.md` §5.4/§6. `this` and `super`
/// are separate scope handles, filtered by name elsewhere, not part of this set.
///
/// Case-insensitive on the exact set (CFML identifiers are case-insensitive; the
/// engine stores these lowercase). A fast reject on the `__` prefix keeps the
/// common case (every non-`__` key, including all single-`_` identifiers) at one
/// comparison.
#[inline]
pub fn is_reserved_component_key(k: &str) -> bool {
    // Fast path: only `__`-prefixed keys can be reserved. A single leading
    // underscore is NEVER reserved (Lucee shows `_foo`; so do we).
    if !k.as_bytes().starts_with(b"__") {
        return false;
    }
    // Structured prefix families: per-method annotation blobs and the synthetic
    // closure/arrow function-name prefixes.
    if k.starts_with("__funcmeta_") || k.starts_with("__closure_") || k.starts_with("__arrow_") {
        return true;
    }
    // Exact reserved set (C2planDoc.md §3). NOTE: reconcile against the C.0 census
    // when wiring the C.3 producer partition — a MISSING entry leaks an engine key
    // into user view; a SPURIOUS entry discards a user member. Keep this list and
    // the census in lockstep.
    const RESERVED: &[&str] = &[
        "__variables",
        "__name",
        "__source_file",
        "__source_names",
        "__metadata",
        "__properties",
        "__super",
        "__super_map",
        "__rust_extends",
        "__extends",
        "__extends_chain",
        "__implements",
        "__implements_chain",
        "__implements_fqns",
        "__implements_src",
        "__accessors",
        "__is_interface",
        "__is_super",
        "__class_name",
        "__instance_id",
        "__static",
        "__cfc_body__",
        "__cfc_static_init__",
        "__cfml_accessor_private__",
        "__java_shim",
        "__java_class",
        "__dynamic_proxy",
        "__proxy_method",
        "__proxy_target",
    ];
    RESERVED.iter().any(|r| k.eq_ignore_ascii_case(r))
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

    /// True iff this view is backed by the flyweight [`Instance`] (vs the legacy
    /// marker struct). Always callable: in a default build the `Instance` arm does
    /// not exist so this is a const `false`, which lets an introspection caller
    /// (`structKeyList`/`for`-in/`serializeJSON`/…) branch to the new data-direct
    /// path ONLY for flyweight instances and leave the marker path byte-for-byte
    /// unchanged — no `component-instance` feature flag needed at the call site.
    #[inline]
    pub fn is_instance_backed(&self) -> bool {
        match self {
            CompRef::Marker(_) => false,
            #[cfg(feature = "component-instance")]
            CompRef::Instance(_) => true,
        }
    }

    // ---- Phase C.3 — Slice 4: introspection bridges (flyweight instances) ----
    //
    // These read `this_members` / `variables_members` DIRECTLY, with **NO
    // `starts_with("__")` filter**: the flyweight data maps are already free of
    // engine reserved keys (the producer partitioned them onto the blueprint /
    // typed fields), and any `__`/`___` key that remains is genuine user data that
    // MUST enumerate — FW/1 AOP's `___doReverse` is the whole reason C.3 exists
    // (§5.2). Returns empty for a marker view; callers gate on `is_instance_backed`.

    /// Public-scope keys for user-facing enumeration (`structKeyList`/`structCount`/
    /// `structKeyArray`/`structKeyExists`, `for … in`): public DATA keys first,
    /// then public/remote method names not shadowed by a data key.
    pub fn instance_public_keys(&self) -> Vec<String> {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            let ap = g.accessor_private.read();
            let mut keys: Vec<String> = g
                .this_members
                .snapshot()
                .into_keys()
                .filter(|k| !ap.contains(&k.to_ascii_lowercase()))
                .collect();
            // Methods enumerate only while the shared table is still attached —
            // `structClear(instance)` drops it (MockBox `clearMethods`), after which
            // the object is method-less until re-mixed.
            if g.this_members.method_table().is_none() {
                return keys;
            }
            for name in g.class.methods.keys() {
                let is_public = matches!(
                    g.class.method_access.get(&name.to_ascii_lowercase()),
                    Some(crate::dynamic::CfmlAccess::Public)
                        | Some(crate::dynamic::CfmlAccess::Remote)
                );
                if is_public && !keys.iter().any(|k| k.eq_ignore_ascii_case(name)) {
                    keys.push(name.clone());
                }
            }
            return keys;
        }
        Vec::new()
    }

    /// Public members as `name -> value` (public DATA + public/remote methods) for
    /// `for … in` value binding and member-BIF fallbacks.
    pub fn instance_public_members(&self) -> crate::dynamic::ValueMap {
        // `mut` is used only in the feature-on arm below; a default build never
        // mutates it (the marker view returns the empty map).
        #[allow(unused_mut)]
        let mut out = crate::dynamic::ValueMap::default();
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            let ap = g.accessor_private.read();
            for (k, v) in g.this_members.snapshot() {
                if ap.contains(&k.to_ascii_lowercase()) {
                    continue; // accessor-private: hidden from for-in / member iteration
                }
                out.insert(k, v);
            }
            if g.this_members.method_table().is_none() {
                return out;
            }
            for (name, f) in g.class.methods.iter() {
                let is_public = matches!(
                    g.class.method_access.get(&name.to_ascii_lowercase()),
                    Some(crate::dynamic::CfmlAccess::Public)
                        | Some(crate::dynamic::CfmlAccess::Remote)
                );
                if is_public && !out.keys().any(|k| k.eq_ignore_ascii_case(name)) {
                    out.insert(name.clone(), CfmlValue::Function(f.clone()));
                }
            }
        }
        out
    }

    /// Public DATA only (`name -> value`, no methods) — for `serializeJSON` and
    /// any data-projection that must exclude callables.
    pub fn instance_public_data(&self) -> crate::dynamic::ValueMap {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            return inst.read().this_members.snapshot();
        }
        crate::dynamic::ValueMap::default()
    }

    /// Full `serializeJSON` DATA for a flyweight instance: public data members
    /// PLUS each declared `property` value that lives only in the private
    /// `variables` scope (default-only / inherited accessor properties — GH #267).
    /// Lucee serializes those too; reading `this_members` alone would drop them.
    /// Methods/closures are never included. Returns empty for a marker view.
    pub fn instance_serialize_data(&self) -> crate::dynamic::ValueMap {
        #[allow(unused_mut)]
        let mut out = self.instance_public_data();
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            if let Some(CfmlValue::Array(props)) = &g.class.properties {
                let vars = g.variables_members.snapshot();
                for prop in props.iter() {
                    let pname = match prop
                        .as_cfml_struct()
                        .and_then(|ps| ps.get_ci("name"))
                        .map(|n| n.as_string())
                    {
                        Some(n) => n,
                        None => continue,
                    };
                    if out.keys().any(|k| k.eq_ignore_ascii_case(&pname)) {
                        continue; // already emitted from the public scope
                    }
                    if let Some(v) = vars
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&pname))
                        .map(|(_, v)| v.clone())
                    {
                        if !matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_)) {
                            out.insert(pname, v);
                        }
                    }
                }
            }
        }
        out
    }

    /// Write a public DATA member (or inject a method as data) in place — the
    /// `structAppend(instance, …)` / mixin-injection path. No-op on a marker view.
    pub fn instance_set_public(&self, name: String, value: CfmlValue) {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            inst.read().this_members.insert(name, value);
        }
        #[cfg(not(feature = "component-instance"))]
        let _ = (name, value);
    }

    /// Empty the public scope in place — `structClear(instance)`. Clears the public
    /// data map AND drops the shared method table from both maps, so the object is
    /// method-less until re-mixed (MockBox `clearMethods=true`), matching the marker
    /// path. Identity + private data survive. No-op on a marker view.
    pub fn instance_clear_public(&self) {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            g.this_members.with_write(|m| m.clear());
            g.this_members.clear_method_table();
            g.variables_members.clear_method_table();
        }
    }

    /// True iff `name` is already a public member (data or public method) — for
    /// `structAppend`'s non-overwrite mode.
    pub fn instance_has_public(&self, name: &str) -> bool {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            return g.this_members.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
                || (g.this_members.method_table().is_some()
                    && g.class.methods.keys().any(|k| k.eq_ignore_ascii_case(name)));
        }
        #[cfg(not(feature = "component-instance"))]
        let _ = name;
        false
    }

    /// Remove a public DATA member in place — `structDelete(instance, key)` /
    /// `comp.key = null`. Removes from the public map and (covering a private-only
    /// member) the private map; returns true iff a key was removed. Methods live on
    /// the shared blueprint and are NOT deletable per-instance (a marker `structDelete`
    /// of a method is likewise a data-map no-op). No-op on a marker view.
    pub fn instance_delete_public(&self, name: &str) -> bool {
        #[cfg(feature = "component-instance")]
        if let CompRef::Instance(inst) = self {
            let g = inst.read();
            let removed_pub = g.this_members.remove_ci(name).is_some();
            let removed_priv = g.variables_members.remove_ci(name).is_some();
            return removed_pub || removed_priv;
        }
        #[cfg(not(feature = "component-instance"))]
        let _ = name;
        false
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

#[cfg(test)]
mod reserved_key_tests {
    use super::is_reserved_component_key;

    #[test]
    fn user_double_and_triple_underscore_keys_are_not_reserved() {
        // The regression this whole phase exists for: FW/1 AOP stashes the
        // original method under a `___`-prefixed key. These MUST be treated as
        // ordinary user data, never hidden/discarded.
        for k in [
            "___doReverse",
            "___orig",
            "__doReverse",
            "__myVar",
            "__foo",
            "___",
            "__",
            "__init", // user method-ish name, not a reserved bookkeeping key
        ] {
            assert!(!is_reserved_component_key(k), "{k} must NOT be reserved");
        }
    }

    #[test]
    fn single_underscore_and_plain_keys_are_not_reserved() {
        for k in ["_single", "_variables", "name", "foo", "this", "super", "static"] {
            assert!(!is_reserved_component_key(k), "{k} must NOT be reserved");
        }
    }

    #[test]
    fn engine_bookkeeping_keys_are_reserved() {
        for k in [
            "__variables",
            "__name",
            "__metadata",
            "__properties",
            "__super",
            "__super_map",
            "__extends_chain",
            "__implements_fqns",
            "__instance_id",
            "__static",
            "__source_file",
            "__accessors",
            "__is_super",
            "__cfc_body__",
            "__java_shim",
        ] {
            assert!(is_reserved_component_key(k), "{k} MUST be reserved");
        }
    }

    #[test]
    fn reserved_set_is_case_insensitive() {
        // CFML identifiers are case-insensitive; the engine stores these lowercase
        // but a member read/enumeration may present another casing.
        assert!(is_reserved_component_key("__VARIABLES"));
        assert!(is_reserved_component_key("__Name"));
        assert!(is_reserved_component_key("__Metadata"));
    }

    #[test]
    fn structured_prefix_families_are_reserved() {
        assert!(is_reserved_component_key("__funcmeta_doReverse"));
        assert!(is_reserved_component_key("__closure_1"));
        assert!(is_reserved_component_key("__arrow_42"));
        // ...but a user key that merely *starts like* a family prefix without the
        // underscore boundary is still evaluated by the exact rules above.
        assert!(!is_reserved_component_key("__funcmetadata")); // not `__funcmeta_`
    }
}

#[cfg(all(test, feature = "component-instance"))]
mod producer_tests {
    use super::*;
    use crate::dynamic::{
        CfmlAccess, CfmlClosureBody, CfmlFunction, CfmlStruct, CfmlValue, ValueMap,
    };
    use std::sync::Arc;

    fn method(name: &str, access: CfmlAccess) -> CfmlValue {
        CfmlValue::Function(Arc::new(CfmlFunction {
            name: name.to_string(),
            params: Vec::new(),
            body: CfmlClosureBody::Statements(Vec::new()),
            return_type: None,
            access,
            captured_scope: None,
        }))
    }

    /// Build a finished-shape marker: public (top-level) `this` scope carries data
    /// members, methods, and the reserved bookkeeping keys; `__variables` carries
    /// private data + a mirrored method.
    fn build_marker() -> CfmlStruct {
        let mut vars = ValueMap::default();
        vars.insert("privvar".to_string(), CfmlValue::string("PV"));
        vars.insert("___privstash".to_string(), CfmlValue::string("PS")); // user triple-underscore
        vars.insert("greet".to_string(), method("greet", CfmlAccess::Public));
        vars.insert("secret".to_string(), method("secret", CfmlAccess::Private));

        let mut top = ValueMap::default();
        top.insert("plain".to_string(), CfmlValue::string("P"));
        top.insert("___doreverse".to_string(), CfmlValue::string("STASHED")); // FW/1 AOP case
        top.insert("_single".to_string(), CfmlValue::string("S1")); // single underscore
        top.insert("greet".to_string(), method("greet", CfmlAccess::Public));
        top.insert("__name".to_string(), CfmlValue::string("oop.Foo"));
        top.insert("__source_file".to_string(), CfmlValue::string("/app/Foo.cfc"));
        top.insert("__instance_id".to_string(), CfmlValue::Int(7));
        top.insert(
            "__metadata".to_string(),
            CfmlValue::Struct(CfmlStruct::new(ValueMap::default())),
        );
        top.insert("__variables".to_string(), CfmlValue::Struct(CfmlStruct::new(vars)));

        CfmlStruct::new(top)
    }

    #[test]
    fn blueprint_captures_methods_and_metadata_only() {
        let marker = build_marker();
        let bp = ClassBlueprint::from_marker(&marker);
        assert_eq!(bp.name, "oop.Foo");
        assert_eq!(bp.source_file, "/app/Foo.cfc");
        // Public + private (mirrored/private-only) methods, deduped.
        assert!(bp.methods.contains_key("greet"));
        assert!(bp.methods.contains_key("secret"));
        assert_eq!(bp.method_access.get("greet"), Some(&CfmlAccess::Public));
        assert_eq!(bp.method_access.get("secret"), Some(&CfmlAccess::Private));
        // Data must NEVER be captured as a method.
        assert!(!bp.methods.contains_key("plain"));
        assert!(!bp.methods.contains_key("___doreverse"));
        assert!(matches!(bp.metadata, CfmlValue::Struct(_)));
    }

    #[test]
    fn data_maps_carry_user_data_including_double_underscore_keys() {
        let marker = build_marker();
        let bp = Arc::new(ClassBlueprint::from_marker(&marker));
        let inst = Instance::from_marker(&marker, bp, 7);

        // The raw data MAP (iter is map-only; methods live in the shared table).
        let map_has = |s: &CfmlStruct, k: &str| s.iter().any(|(mk, _)| mk.eq_ignore_ascii_case(k));

        // Public DATA: plain + the `__`/`___`/`_` user keys survive; methods and
        // reserved bookkeeping keys are NOT in the data map.
        let this = &inst.this_members;
        let sval = |s: &CfmlStruct, k: &str| s.get_ci(k).map(|v| v.as_string());
        assert_eq!(sval(this, "plain").as_deref(), Some("P"));
        // THE regression this whole phase exists for — must be present as DATA.
        assert_eq!(sval(this, "___doreverse").as_deref(), Some("STASHED"));
        assert_eq!(sval(this, "_single").as_deref(), Some("S1"));
        assert!(!map_has(this, "greet"), "method leaked into this_members map");
        assert!(!map_has(this, "__name"), "reserved key leaked into data map");
        assert!(!map_has(this, "__source_file"));
        assert!(!map_has(this, "__instance_id"));
        assert!(!map_has(this, "__metadata"));
        assert!(!map_has(this, "__variables"));
        // But the method IS resolvable through the shared table (dispatch path).
        assert!(matches!(this.get_ci("greet"), Some(CfmlValue::Function(_))));

        // Private DATA: user keys survive; the private method is not in the map.
        let vars = &inst.variables_members;
        assert_eq!(sval(vars, "privvar").as_deref(), Some("PV"));
        assert_eq!(sval(vars, "___privstash").as_deref(), Some("PS"));
        assert!(!map_has(vars, "greet"));
        assert!(!map_has(vars, "secret"));
    }

    #[test]
    fn duplicate_deep_copies_data_but_shares_blueprint() {
        let marker = build_marker();
        let value = make_instance_value(
            &marker,
            Arc::new(ClassBlueprint::from_marker(&marker)),
            7,
        );
        let dup = value.deep_copy();
        let (orig, copy) = match (&value, &dup) {
            (CfmlValue::Instance(a), CfmlValue::Instance(b)) => (a.clone(), b.clone()),
            _ => panic!("duplicate did not yield an Instance"),
        };
        // Distinct instance handles (value semantics) but the SAME shared blueprint.
        assert!(!Arc::ptr_eq(&orig, &copy), "duplicate shared the instance handle");
        assert!(
            Arc::ptr_eq(&orig.read().class, &copy.read().class),
            "duplicate should share the class blueprint"
        );
        // Mutating the copy's public data does not affect the original.
        copy.read().this_members.insert("plain".to_string(), CfmlValue::string("CHANGED"));
        assert_eq!(
            orig.read().this_members.get_ci("plain").map(|v| v.as_string()).as_deref(),
            Some("P")
        );
        assert_eq!(
            copy.read().this_members.get_ci("plain").map(|v| v.as_string()).as_deref(),
            Some("CHANGED")
        );
        // The copy still dispatches its methods (shared table survived).
        assert!(matches!(copy.read().this_members.get_ci("greet"), Some(CfmlValue::Function(_))));
    }

    #[test]
    fn self_reference_is_retargeted_to_the_instance_handle() {
        // The classic C.2.3 self-reference: `variables[classname] = this`.
        let marker = build_marker();
        if let Some(CfmlValue::Struct(vars)) = marker.get_ci("__variables") {
            vars.with_write(|m| {
                m.insert("foo".to_string(), CfmlValue::Struct(marker.clone()));
            });
        } else {
            panic!("no __variables");
        }

        let bp = Arc::new(ClassBlueprint::from_marker(&marker));
        let value = make_instance_value(&marker, bp, 7);
        let handle = match &value {
            CfmlValue::Instance(h) => h.clone(),
            _ => panic!("producer did not yield an Instance"),
        };
        let g = handle.read();
        match g.variables_members.get_ci("foo") {
            // Retargeted to the live instance, NOT a retained marker clone.
            Some(CfmlValue::Instance(inner)) => {
                assert!(Arc::ptr_eq(&inner, &handle), "self-ref points at a different instance");
            }
            other => panic!("self-reference not retargeted: {other:?}"),
        }
    }
}
