//! Dynamic value types for CFML runtime

use crate::vm::{CfmlError, CfmlResult};
use indexmap::IndexMap;
use parking_lot::RwLock as PlRwLock;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock, Weak};

/// Build-hasher for all per-call scope maps, struct maps, and query-row maps.
///
/// CFML scope/struct keys are short ASCII identifiers and case-insensitivity is
/// handled by callers (`get_ci`, `eq_ignore_ascii_case` scans), NOT the hasher —
/// so SipHash's DoS-resistance buys nothing here. `FxHasher` is ~3-5x faster on
/// short keys; hashing was the #1 self-time bucket in the v0.192 `/posts` profile.
pub type ValueBuildHasher = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;

/// `ValueMap` with the fast [`ValueBuildHasher`]. The ordered
/// key-value map underpinning CFML structs, scopes, and query rows. Construct with
/// `ValueMap::default()` (the `ValueMap::default()` ctor only exists for `RandomState`)
/// and pre-size with `ValueMap::with_capacity_and_hasher(n, Default::default())`.
pub type ValueMap = IndexMap<String, CfmlValue, ValueBuildHasher>;

/// A minimal interface-metadata stub: `{ name, fullname, type:"interface" }`.
/// Used as the value for each entry of the `implements` / interface-`extends`
/// metadata structs. Lucee/ACF store the interface's full metadata here, but
/// every consumer that matters (and the Wheels interface specs) only reads the
/// key (the interface FQN) and `name`, so a stub is sufficient and avoids a
/// recursive template resolve.
pub fn interface_meta_stub(fqn: &str) -> CfmlValue {
    let mut m = ValueMap::default();
    m.insert("name".to_string(), CfmlValue::string(fqn.to_string()));
    m.insert("fullname".to_string(), CfmlValue::string(fqn.to_string()));
    m.insert("type".to_string(), CfmlValue::string("interface".to_string()));
    CfmlValue::strukt(m)
}

/// Build the `implements` metadata struct for a component: a struct keyed by
/// each implemented interface's declared FQN, value = [`interface_meta_stub`].
/// Sources the transitive `__implements_chain` (so an interface's own `extends`
/// ancestors appear) unioned with the directly-declared `__implements` list,
/// dedup'd case-insensitively (first-seen casing wins, matching the declared
/// case). Returns `None` when the component implements nothing. Shared by
/// `getMetadata()` and `getComponentMetaData()` so both forms agree.
pub fn build_implements_meta(s: &ValueMap) -> Option<CfmlValue> {
    let mut seen = std::collections::HashSet::new();
    let mut out = ValueMap::default();
    // Read the directly-declared list first so its original-case FQNs win; the
    // transitive chain (built during inheritance merge, lowercased) then adds
    // only purely-inherited interface ancestors.
    for key in ["__implements", "__implements_chain"] {
        if let Some(CfmlValue::Array(arr)) = s.get(key) {
            for v in arr.iter() {
                let fqn = v.as_string();
                if fqn.is_empty() || !seen.insert(fqn.to_lowercase()) {
                    continue;
                }
                out.insert(fqn.clone(), interface_meta_stub(&fqn));
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(CfmlValue::strukt(out))
    }
}

/// Marker key that tags a struct as a Lucee-style "magic" scope (currently the
/// `cgi` scope): reading ANY missing key returns an empty string `""` rather
/// than throwing / yielding null, while `structKeyExists` still reports the
/// unset key as absent. The marker is engine-internal and must never surface
/// in struct introspection (`structKeyList`, `structCount`, for-in, JSON, …).
pub const EMPTY_DEFAULT_SCOPE_MARKER: &str = "__cfml_empty_default_scope__";

/// Reserved key on a component instance struct holding the set (a `Struct` used
/// as a set: key = lowercased property name, value ignored) of accessor
/// properties whose VALUE was written by the engine's accessor path — the
/// implicit accessor constructor or a generated `setX()` setter. Lucee stores
/// such values in the PRIVATE `variables` scope, so they are invisible to
/// `structKeyList`/`structCount`/`structKeyExists`/for-in (only `getX()` and
/// `serializeJSON` surface them). This engine materialises them at the struct
/// top level (shared with the public `this` scope), so introspection must
/// consult this marker to hide them and match Lucee. An explicit `this.x = …`
/// write does NOT enter this set — it is a genuine public member (kept visible).
/// `__`-prefixed, so it is itself already hidden from introspection and JSON.
pub const ACCESSOR_PRIVATE_MARKER: &str = "__cfml_accessor_private__";

/// Shared, interior-mutable backing for a CFML array — the basis of Lucee-style
/// **reference semantics**. Cloning a `CfmlArray` bumps the `Arc` (it does NOT
/// copy the elements), so `b = a` makes `a` and `b` two handles onto the *same*
/// `Vec`; a mutation through either is visible through both. Contrast the old
/// `Arc<Vec>` + copy-on-write model, which diverged aliases on first write.
///
/// All locking lives behind this type's methods so callers (especially
/// `cfml-stdlib`, which doesn't depend on `parking_lot`) never hold a raw guard.
/// Lock discipline: methods take a guard, do one thing, and drop it before
/// returning — never call back into VM/user code while a guard is held, and
/// never lock the same array twice on one thread (parking_lot locks are not
/// reentrant). Anything that needs to iterate-then-call (higher-order fns,
/// equality) must `snapshot()` first to release the lock.
#[derive(Clone)]
pub struct CfmlArray(Arc<PlRwLock<Vec<CfmlValue>>>);

impl CfmlArray {
    #[inline]
    pub fn new(v: Vec<CfmlValue>) -> Self {
        let arc = Arc::new(PlRwLock::new(v));
        crate::cycle_gc::log_array(&arc);
        CfmlArray(arc)
    }

    #[inline]
    pub fn empty() -> Self {
        CfmlArray::new(Vec::new())
    }

    /// Two handles onto the same backing store (reference identity).
    #[inline]
    pub fn ptr_eq(&self, other: &CfmlArray) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity of the shared backing store, for cycle detection in
    /// recursive walks (reference-typed arrays can alias / form cycles).
    #[inline]
    pub fn backing_ptr(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.read().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.read().is_empty()
    }

    /// Clone the element at a 0-based index, or `None` if out of range.
    #[inline]
    pub fn get(&self, idx: usize) -> Option<CfmlValue> {
        self.0.read().get(idx).cloned()
    }

    #[inline]
    pub fn first(&self) -> Option<CfmlValue> {
        self.0.read().first().cloned()
    }

    #[inline]
    pub fn last(&self) -> Option<CfmlValue> {
        self.0.read().last().cloned()
    }

    /// Overwrite an existing 0-based index in place. Returns false if out of
    /// range (no auto-grow — see `set_or_grow`).
    #[inline]
    pub fn set(&self, idx: usize, value: CfmlValue) -> bool {
        let mut g = self.0.write();
        if idx < g.len() {
            g[idx] = value;
            true
        } else {
            false
        }
    }

    /// Set a 0-based index, growing the array (filling gaps with `Null`, Lucee
    /// semantics) when `idx` is past the end.
    pub fn set_or_grow(&self, idx: usize, value: CfmlValue) {
        let mut g = self.0.write();
        if idx < g.len() {
            g[idx] = value;
        } else {
            g.resize(idx, CfmlValue::Null);
            g.push(value);
        }
    }

    #[inline]
    pub fn push(&self, value: CfmlValue) {
        self.0.write().push(value);
    }

    /// A point-in-time copy of the contents. Use this before iterating when the
    /// loop body may call back into code that touches the same array (closures,
    /// equality, dump) — it releases the lock so re-entrancy can't deadlock.
    #[inline]
    pub fn snapshot(&self) -> Vec<CfmlValue> {
        self.0.read().clone()
    }

    /// Iterate a point-in-time **snapshot** of the elements (yields owned
    /// `CfmlValue`s, not borrows). Iterating a snapshot — rather than holding
    /// the lock across the loop — is what makes reference-typed arrays safe to
    /// walk while the body may mutate the same array (and can't deadlock). This
    /// is the reference-semantics analogue of `Vec::iter()`; it snapshots, so
    /// avoid it on hot paths where `len()`/`get()` suffice.
    #[inline]
    pub fn iter(&self) -> std::vec::IntoIter<CfmlValue> {
        self.snapshot().into_iter()
    }

    /// Alias for `snapshot()` — owned copy of the elements.
    #[inline]
    pub fn to_vec(&self) -> Vec<CfmlValue> {
        self.snapshot()
    }

    /// Run a closure with exclusive (write) access to the backing `Vec`. The
    /// closure MUST NOT touch this same array again (would deadlock).
    #[inline]
    pub fn with_write<R>(&self, f: impl FnOnce(&mut Vec<CfmlValue>) -> R) -> R {
        f(&mut self.0.write())
    }

    /// Run a closure with shared (read) access. Same re-entrancy caveat.
    #[inline]
    pub fn with_read<R>(&self, f: impl FnOnce(&Vec<CfmlValue>) -> R) -> R {
        f(&self.0.read())
    }
}

impl FromIterator<CfmlValue> for CfmlArray {
    fn from_iter<I: IntoIterator<Item = CfmlValue>>(iter: I) -> Self {
        CfmlArray::new(iter.into_iter().collect())
    }
}

/// Shared, interior-mutable backing for a CFML struct — the struct analogue of
/// [`CfmlArray`], giving structs Lucee-style **reference semantics**. Cloning a
/// `CfmlStruct` bumps the `Arc` (it does NOT copy the entries), so `b = a` makes
/// `a` and `b` two handles onto the *same* `IndexMap`; a mutation through either
/// (and through any CFC instance that shares the handle) is visible through both.
///
/// All locking lives behind this type's methods so callers (especially
/// `cfml-stdlib`, which doesn't depend on `parking_lot`) never hold a raw guard.
/// Lock discipline (critical — parking_lot is NOT reentrant): a method takes a
/// guard, does one thing, drops it. Never call back into VM/user code while a
/// guard is held, and never lock the same struct twice on one thread. Anything
/// iterate-then-call (higher-order struct fns, equality, dump, CFC method
/// dispatch) must `snapshot()` / `iter()` first to release the lock.
/// v0.99.4 — inner struct payload. `shape_id` is bumped on every
/// **structural** change (new key inserted, key removed, clear when
/// non-empty, or any `with_write` access). Value-only updates do NOT
/// bump shape — the same `(name → index)` mapping holds, and JIT inline
/// caches over `GetProperty(name)` stay valid. `with_write` exposes the
/// inner `IndexMap` directly, so it must bump unconditionally (the
/// closure could do anything). Shape IDs are allocated from a process-
/// wide atomic counter; `0` is reserved (never used) so an
/// uninitialised IC slot is always a miss.
pub struct StructInner {
    pub map: ValueMap,
    /// v0.442 (issue #262) — case-insensitive lookup index: maps each key's
    /// ASCII-lowercased form to the ORIGINAL-cased key currently stored in
    /// `map`. This turns every ci lookup / existence check / insert-dedup from
    /// the old O(n) `keys().any(eq_ignore_ascii_case)` scan into an O(1) hash
    /// probe (large per-request caches, session/URL scopes, and the pixl8/
    /// sticker sort that motivated the fix all built and probed huge structs,
    /// degrading to O(n²)–O(n⁴)). Invariant: for every key `K` in `map`,
    /// `ci[fold(K)] == K` (first-written casing wins, matching `insert`). It is
    /// rebuilt wholesale after any raw [`CfmlStruct::with_write`], whose closure
    /// can restructure `map` arbitrarily.
    pub ci: HashMap<String, String, ValueBuildHasher>,
    pub shape_id: u64,
    /// Live `variables.this` alias (Lucee/ACF semantics). When set on a CFC's
    /// private `__variables` struct, a read of the `this` key resolves to the
    /// upgraded handle — the component's live public scope — rather than a
    /// stored value. Held as a `Weak` so it never forms a strong Arc cycle
    /// (`instance -> __variables -> this -> instance`), which would leak the
    /// instance forever (the v0.185.0 per-request serve-mode leak). `None` on
    /// every non-component struct, so unrelated structs pay nothing.
    pub this_alias: Option<Weak<PlRwLock<StructInner>>>,
}

static STRUCT_SHAPE_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[inline]
fn next_shape_id() -> u64 {
    STRUCT_SHAPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Fold a key to its case-insensitive canonical form for the `ci` index.
/// CFML case-insensitivity is ASCII-only (the whole codebase compares with
/// `eq_ignore_ascii_case`), so we lower only ASCII — and, crucially, borrow
/// the key unchanged when it is already lowercase (the overwhelmingly common
/// case: `key_1`, `columnList`-style names, etc.), so a hot lookup loop pays
/// zero allocations.
#[inline]
fn fold_key(key: &str) -> std::borrow::Cow<'_, str> {
    if key.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(key.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(key)
    }
}

/// Structs at or below this size skip the case-insensitive `ci` index
/// entirely: a linear `eq_ignore_ascii_case` scan over so few keys is cheaper
/// than allocating, populating, and maintaining a hash index — and the
/// overwhelmingly common structs (every function call's `arguments` scope,
/// small option/config structs, per-row query structs) are all this small.
/// Building that index eagerly in `CfmlStruct::new` for these tiny structs was
/// ~25% of serve-mode CPU on the pixl8/sticker sort (millions of per-call
/// arguments scopes). The index is built lazily only once a struct grows past
/// this threshold — which is where O(1) ci ops actually matter (issue #262: big
/// per-request caches, session/URL scopes, the sticker `comparisonCache`).
///
/// Invariant: `ci` is complete (`ci[fold(K)] == K` for every key `K`) iff
/// `map.len() > CI_THRESHOLD`; otherwise `ci` is empty and ci resolution scans.
const CI_THRESHOLD: usize = 16;

impl StructInner {
    /// Rebuild the `ci` index from scratch to re-establish the
    /// `ci[fold(K)] == K` invariant after `map` may have been mutated through
    /// a channel that doesn't maintain it incrementally (raw `with_write`).
    /// O(n); only called off the hot path. When two keys fold to the same form
    /// (a pre-existing raw map with case-variant duplicates — normal inserts
    /// dedup, so this is an edge), the LAST one iterated wins the ci slot,
    /// matching `map`'s own last-writer-wins for equal keys.
    fn ci_rebuild(&mut self) {
        self.ci.clear();
        self.ci.reserve(self.map.len());
        for k in self.map.keys() {
            self.ci.insert(k.to_ascii_lowercase(), k.clone());
        }
    }

    /// Resolve `key` case-insensitively to the ORIGINAL-cased key stored in
    /// `map`, or `None`. O(1) via the `ci` index when the struct is large
    /// enough to maintain one; otherwise a linear `eq_ignore_ascii_case` scan
    /// (cheap at `<= CI_THRESHOLD` keys — see `CI_THRESHOLD`). This is the one
    /// place the "indexed vs scan" decision lives; every ci read routes here.
    #[inline]
    fn resolve_ci_key(&self, key: &str) -> Option<&String> {
        if self.map.len() > CI_THRESHOLD {
            self.ci.get(fold_key(key).as_ref())
        } else {
            self.map.keys().find(|k| k.eq_ignore_ascii_case(key))
        }
    }

    /// Maintain the `ci` index after a genuinely-new key was appended to `map`.
    /// A no-op while the struct is small (index inactive); rebuilds wholesale on
    /// the insert that crosses `CI_THRESHOLD` (or when a prior shrink left the
    /// index cleared) and incrementally otherwise. `folded`/`key` are the new
    /// key's fold form and original casing.
    #[inline]
    fn ci_note_new_key(&mut self, folded: String, key: String) {
        if self.map.len() > CI_THRESHOLD {
            if self.ci.len() == self.map.len() - 1 {
                self.ci.insert(folded, key); // index was complete → incremental
            } else {
                self.ci_rebuild(); // crossing the threshold, or stale post-shrink
            }
        }
    }
}

/// Format an f64 the way Lucee/ACF stringify numbers, rather than Rust's
/// shortest-round-trip `f64::to_string` (which leaks IEEE noise like
/// `1756.8000000000002`). The CFML rule, verified against Lucee 7:
///   * integer-valued doubles print as a whole number (no `.0`, no scientific);
///   * otherwise, start from the shortest round-trip decimal and, only if it
///     carries more than 12 fractional digits, round to 12 decimal places;
///     then strip trailing zeros (and a bare trailing `.`).
/// Working from the shortest round-trip (not the raw f64 expansion) is what
/// keeps genuine precision on large magnitudes — `99999999999.9999` and
/// `1234567890123.456` already have ≤12 fractional digits so survive intact —
/// while still collapsing noise: `1/3` → `0.333333333333`,
/// `3.14159265358979` → `3.14159265359`, `0.1+0.2` → `0.3`, `1e-13` → `0`.
pub fn format_double(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_string();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    // Integer-valued: print as a whole number with no decimals or exponent.
    if d.fract() == 0.0 {
        // Below 2^53 every integer is exactly representable; use i64 for speed.
        if d.abs() < 1e15 {
            return (d as i64).to_string();
        }
        return format!("{:.0}", d);
    }
    // Rust's Display gives the shortest round-trip in plain (non-scientific)
    // form for normal magnitudes. If that already fits in ≤12 fractional
    // digits, it is exactly what Lucee prints.
    let short = d.to_string();
    if !short.contains(['e', 'E']) {
        if let Some(dot) = short.find('.') {
            if short.len() - dot - 1 <= 12 {
                return short;
            }
        }
    }
    let mut s = format!("{:.12}", d);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    // `-0.0000000000004` rounds to "-0"; normalise to "0" like Lucee.
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

/// If `s` is a Java-object shim (`createObject("java", …)`, represented
/// internally as a struct tagged with `__java_shim`), return its Java
/// `toString()` string. Lucee coerces Java objects to their `toString()` in
/// every string context — `"" & obj`, `replace(obj, …)`, `<cfoutput>#obj#` —
/// rather than dumping the object or throwing, so RustCFML must do the same for
/// its shim representation. Mirrors the per-class `toString` handlers in
/// cfml-vm's `java_shims.rs` for the classes whose string form real apps rely
/// on (UUID, StringBuilder/Buffer, Locale, URL, InetAddress, …); any other
/// shim falls back to its Java class name — never a struct dump, never a throw.
///
/// Returns `None` for a plain CFML struct so the caller keeps normal
/// struct-dump / throw behaviour.
fn java_shim_string(s: &CfmlStruct) -> Option<String> {
    if !s.get("__java_shim").map(|v| v.is_true()).unwrap_or(false) {
        return None;
    }
    // java.util.UUID -> canonical 8-4-4-4-12 form (matches UUID.toString()).
    if let Some(u) = s.get("__uuid") {
        let uuid = u.as_string();
        if uuid.len() >= 32 {
            return Some(format!(
                "{}-{}-{}-{}-{}",
                &uuid[0..8],
                &uuid[8..12],
                &uuid[12..16],
                &uuid[16..20],
                &uuid[20..32]
            ));
        }
        return Some(uuid);
    }
    // java.lang.StringBuilder / StringBuffer -> buffered contents.
    if let Some(b) = s.get("__buffer") {
        return Some(b.as_string());
    }
    // java.util.Locale -> its id (`en`, `en_US`), matching Locale.toString().
    if let Some(id) = s.get("__locale_id") {
        return Some(id.as_string());
    }
    // java.net.URL -> its spec (URL.toString() == toExternalForm()).
    if let Some(spec) = s.get("__spec") {
        return Some(spec.as_string());
    }
    // java.net.InetAddress -> its hostname.
    if let Some(h) = s.get("__hostname") {
        return Some(h.as_string());
    }
    // Generic single-value wrapper shims store the scalar under `__value`.
    if let Some(v) = s.get("__value") {
        return Some(v.as_string());
    }
    // Any other Java object: fall back to its class name (never a struct dump,
    // never a coercion throw).
    s.get("__java_class").map(|c| c.as_string())
}

#[derive(Clone)]
pub struct CfmlStruct(Arc<PlRwLock<StructInner>>);

impl CfmlStruct {
    #[inline]
    pub fn new(m: ValueMap) -> Self {
        // Build the case-insensitive index eagerly ONLY for large structs; small
        // structs (the common per-call case) skip it and scan on the rare ci
        // read. See `CI_THRESHOLD`. `HashMap::with_hasher` allocates nothing.
        let ci = if m.len() > CI_THRESHOLD {
            let mut ci =
                HashMap::with_capacity_and_hasher(m.len(), ValueBuildHasher::default());
            for k in m.keys() {
                ci.insert(k.to_ascii_lowercase(), k.clone());
            }
            ci
        } else {
            HashMap::with_hasher(ValueBuildHasher::default())
        };
        let arc = Arc::new(PlRwLock::new(StructInner {
            map: m,
            ci,
            shape_id: next_shape_id(),
            this_alias: None,
        }));
        crate::cycle_gc::log_struct(&arc);
        CfmlStruct(arc)
    }

    #[inline]
    pub fn empty() -> Self {
        CfmlStruct::new(ValueMap::default())
    }

    /// Two handles onto the same backing store (reference identity).
    #[inline]
    pub fn ptr_eq(&self, other: &CfmlStruct) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity of the shared backing store, for cycle detection in
    /// recursive struct walks (reference-typed structs can alias / form cycles).
    #[inline]
    pub fn backing_ptr(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }

    /// v0.99.4 — current shape generation. Bumped on every structural
    /// change. JIT IC fast path: load this, compare with cached
    /// `shape_id`; on match the cached `(name → index)` is still valid
    /// so the IC can index directly into `map.get_index(cached_idx)`.
    /// On miss the slow path re-resolves the key and updates the IC.
    #[inline]
    pub fn shape_id(&self) -> u64 {
        self.0.read().shape_id
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.read().map.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.read().map.is_empty()
    }

    /// Clone the value for `key` (case-sensitive), or `None`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<CfmlValue> {
        if let Some(v) = self.0.read().map.get(key).cloned() {
            return Some(v);
        }
        // Live `variables.this` alias (Lucee/ACF): a CFC `__variables` with no
        // stored `this` key resolves it to the live public scope via a Weak
        // back-edge. Only consulted on a miss, and only for the `this` key.
        if key.eq_ignore_ascii_case("this") {
            return self.this_alias_struct().map(CfmlValue::Struct);
        }
        None
    }

    /// Set (or refresh) the live `variables.this` alias to `target`'s backing
    /// store, but only when it differs from what is already stored — avoids a
    /// write lock on the hot `variables` read path once the alias is stamped.
    /// Held as a `Weak`, so this never extends `target`'s lifetime. Does NOT
    /// bump `shape_id`: the key set is unchanged (the alias is resolved lazily
    /// on read, never materialized into the map), so JIT inline caches stay
    /// valid.
    pub fn set_this_alias_if_changed(&self, target: &CfmlStruct) {
        // Fast path: already aliased to this exact backing store.
        {
            let g = self.0.read();
            if let Some(w) = &g.this_alias {
                if let Some(cur) = w.upgrade() {
                    if Arc::ptr_eq(&cur, &target.0) {
                        return;
                    }
                }
            }
        }
        self.0.write().this_alias = Some(Arc::downgrade(&target.0));
    }

    /// Upgrade the live `variables.this` alias to a strong handle, if set and
    /// still alive.
    #[inline]
    pub fn this_alias_struct(&self) -> Option<CfmlStruct> {
        self.0.read().this_alias.as_ref().and_then(|w| w.upgrade()).map(CfmlStruct)
    }

    /// Clone the value for `key`, matching keys case-insensitively (CFML keys
    /// are case-insensitive). Returns the first matching entry's value.
    pub fn get_ci(&self, key: &str) -> Option<CfmlValue> {
        {
            let g = self.0.read();
            if let Some(v) = g.map.get(key) {
                return Some(v.clone()); // exact-case fast path (no fold)
            }
            // ci resolution: O(1) index for large structs, linear scan for small.
            if let Some(orig) = g.resolve_ci_key(key) {
                if let Some(v) = g.map.get(orig) {
                    return Some(v.clone());
                }
            }
        }
        // Live `variables.this` alias on a miss (see `get`).
        if key.eq_ignore_ascii_case("this") {
            return self.this_alias_struct().map(CfmlValue::Struct);
        }
        None
    }

    /// v0.99.5 — case-insensitive lookup that also returns the IndexMap
    /// entry index. Used by the JIT member-access inline cache:
    /// `(name → idx)` is stable while `shape_id` doesn't change, so the
    /// IC can hit `map.get_index(cached_idx)` on the fast path. Walks the
    /// map twice in the cold case (exact, then ci-scan) — same shape as
    /// `get_ci` but threaded with `.enumerate()`.
    pub fn get_ci_indexed(&self, key: &str) -> Option<(usize, CfmlValue)> {
        let g = self.0.read();
        if let Some((i, _, v)) = g.map.get_full(key) {
            return Some((i, v.clone()));
        }
        // Resolve original casing (O(1) index for large, scan for small), then
        // one `get_full`.
        let orig = g.resolve_ci_key(key)?;
        g.map.get_full(orig).map(|(i, _, v)| (i, v.clone()))
    }

    /// v0.99.5 — read the value at a specific IndexMap entry index. Used
    /// by the JIT IC's fast path after the cached shape matched. Returns
    /// `None` if the index is out of range (shouldn't happen when shape
    /// matched, but defensive).
    #[inline]
    pub fn get_at_index(&self, idx: usize) -> Option<CfmlValue> {
        self.0.read().map.get_index(idx).map(|(_, v)| v.clone())
    }

    /// v0.100.0 — write a value at a specific IndexMap entry index. Used by
    /// the JIT member-write IC's fast path: when a cached `(shape, idx)` hit
    /// confirms the key is at the position we recorded, replace the value
    /// in place. Does NOT bump `shape_id` — the key set is unchanged, only
    /// the value at that slot. Returns the previous value, or `None` if the
    /// index is out of range (defensive — shape match implies in-range).
    #[inline]
    pub fn set_at_index(&self, idx: usize, value: CfmlValue) -> Option<CfmlValue> {
        let mut g = self.0.write();
        g.map
            .get_index_mut(idx)
            .map(|(_, slot)| std::mem::replace(slot, value))
    }

    /// v0.442 — resolve `key` case-insensitively to the ORIGINAL-cased key as
    /// stored in the map, in O(1) via the ci index. Returns `None` if no
    /// case-variant is present. Used by `structKeyExists`/`structFindKey`-style
    /// callers that need the real stored key, not just presence.
    #[inline]
    pub fn key_ci(&self, key: &str) -> Option<String> {
        let g = self.0.read();
        if g.map.contains_key(key) {
            return Some(key.to_string()); // exact-case fast path
        }
        g.resolve_ci_key(key).cloned()
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.read().map.contains_key(key)
            || (key.eq_ignore_ascii_case("this") && self.this_alias_struct().is_some())
    }

    /// Case-insensitive key presence check.
    pub fn contains_key_ci(&self, key: &str) -> bool {
        let g = self.0.read();
        // exact hit, else ci resolution (O(1) index for large, scan for small).
        if g.map.contains_key(key) || g.resolve_ci_key(key).is_some() {
            return true;
        }
        drop(g);
        // `StructKeyExists(variables, "this")` must see the live alias (Lucee
        // parity — Wheels Plugins.cfc gates the public mixin append on it).
        key.eq_ignore_ascii_case("this") && self.this_alias_struct().is_some()
    }

    /// Insert (interior mutability — visible to all aliases). Returns the
    /// previous value if the key already existed. v0.99.4 — shape_id is
    /// bumped iff the key is genuinely new (no prior value); value-only
    /// updates leave shape alone so JIT ICs stay warm.
    ///
    /// v0.116.0 — case-insensitive on write to match Lucee/ACF: when a key
    /// already exists under a different casing, update its value in place and
    /// preserve the FIRST-WRITTEN casing in the key list (`StructKeyList`,
    /// iteration order, etc.). Writes that hit an exact case match are
    /// unchanged. Prior behavior forked the key — `s={a:1}; s["A"]=2` left
    /// two physical entries, poisoning set-one-case / read-another-case flows
    /// (URL/form params, query columnList lookups, option-struct merges).
    pub fn insert(&self, key: String, value: CfmlValue) -> Option<CfmlValue> {
        let mut g = self.0.write();
        if let Some(slot) = g.map.get_mut(&key) {
            return Some(std::mem::replace(slot, value)); // exact hit, value-only
        }
        // Case-variant dedup: O(1) via the ci index for large structs, a linear
        // scan for small ones (see `CI_THRESHOLD`). Either way, an existing key
        // under a different casing is updated in place (first-casing wins).
        if let Some(orig) = g.resolve_ci_key(&key).cloned() {
            if let Some(slot) = g.map.get_mut(&orig) {
                return Some(std::mem::replace(slot, value));
            }
        }
        // Genuinely new key: append to `map`, bump shape, and maintain the ci
        // index iff the struct is (or just became) large enough to keep one.
        let folded = key.to_ascii_lowercase();
        let prev = g.map.insert(key.clone(), value);
        if prev.is_none() {
            g.shape_id = next_shape_id();
        }
        g.ci_note_new_key(folded, key);
        prev
    }

    /// Merge every entry of `other` into `self` (insert-or-overwrite, with the
    /// same case-insensitive overwrite semantics as [`insert`]).
    ///
    /// **Reference-identity fast path:** when `other` is the *same* backing
    /// store as `self` (`ptr_eq`), this is a no-op — the entries are literally
    /// already present, so there is nothing to copy. This is the common case
    /// for CFC method `variables`-scope write-back: the method mutates the
    /// instance's `__variables` through a shared `Arc`, so by the time we go to
    /// "write it back" the data already landed. Avoids cloning the whole map on
    /// every method return.
    pub fn merge_from(&self, other: &CfmlStruct) {
        if self.ptr_eq(other) {
            return;
        }
        for (k, v) in other.snapshot() {
            self.insert(k, v);
        }
    }

    /// Remove a key (case-sensitive), returning its value if present. Uses
    /// `shift_remove` to preserve insertion order of the remaining entries.
    /// v0.99.4 — shape_id bumps iff a key was actually removed.
    #[inline]
    pub fn remove(&self, key: &str) -> Option<CfmlValue> {
        let mut g = self.0.write();
        let prev = g.map.shift_remove(key);
        if prev.is_some() {
            g.shape_id = next_shape_id();
            // Maintain the ci index only while one is active. If the removal
            // drops the struct to/below the threshold, retire the index (clear)
            // so later reads fall back to the scan and a regrow rebuilds cleanly.
            if !g.ci.is_empty() {
                if g.map.len() > CI_THRESHOLD {
                    let f = fold_key(key).into_owned();
                    if g.ci.get(&f).map(|o| o == key).unwrap_or(false) {
                        g.ci.remove(&f);
                    }
                } else {
                    g.ci.clear();
                }
            }
        }
        prev
    }

    /// Remove a key case-insensitively, returning its value if present.
    /// v0.99.4 — shape_id bumps iff a key was actually removed.
    pub fn remove_ci(&self, key: &str) -> Option<CfmlValue> {
        let mut g = self.0.write();
        let f = fold_key(key).into_owned();
        // Resolve the stored original casing (O(1) index for large, scan for
        // small), then remove it.
        let orig = if g.map.contains_key(key) {
            Some(key.to_string())
        } else {
            g.resolve_ci_key(key).cloned()
        };
        let prev = orig.and_then(|k| g.map.shift_remove(&k));
        if prev.is_some() {
            g.shape_id = next_shape_id();
            if !g.ci.is_empty() {
                if g.map.len() > CI_THRESHOLD {
                    g.ci.remove(&f);
                } else {
                    g.ci.clear();
                }
            }
        }
        prev
    }

    /// v0.99.4 — shape_id bumps iff the map was non-empty before clear.
    #[inline]
    pub fn clear(&self) {
        let mut g = self.0.write();
        if !g.map.is_empty() {
            g.map.clear();
            g.ci.clear();
            g.shape_id = next_shape_id();
        }
    }

    #[inline]
    pub fn keys(&self) -> Vec<String> {
        self.0.read().map.keys().cloned().collect()
    }

    /// A point-in-time copy of the contents. Use this before iterating when the
    /// loop body may call back into code that touches the same struct — it
    /// releases the lock so re-entrancy can't deadlock.
    #[inline]
    pub fn snapshot(&self) -> ValueMap {
        self.0.read().map.clone()
    }

    /// Iterate a point-in-time **snapshot** of the entries (yields owned
    /// `(String, CfmlValue)` pairs, not borrows). Iterating a snapshot — rather
    /// than holding the lock across the loop — is what makes reference-typed
    /// structs safe to walk while the body may mutate the same struct (and
    /// can't deadlock). Snapshots, so avoid on hot paths where `get()`/`len()`
    /// suffice.
    #[inline]
    pub fn iter(&self) -> indexmap::map::IntoIter<String, CfmlValue> {
        self.snapshot().into_iter()
    }

    /// Alias for `snapshot()` — owned copy of the entries.
    #[inline]
    pub fn to_indexmap(&self) -> ValueMap {
        self.snapshot()
    }

    /// Run a closure with exclusive (write) access to the backing map. The
    /// closure MUST NOT touch this same struct again (would deadlock).
    /// v0.99.4 — bumps shape_id unconditionally on entry because the
    /// closure can do anything (insert / remove / restructure); we can't
    /// see whether the operation was structural. Conservative: every
    /// `with_write` invalidates all ICs on this struct.
    #[inline]
    pub fn with_write<R>(&self, f: impl FnOnce(&mut ValueMap) -> R) -> R {
        let mut g = self.0.write();
        g.shape_id = next_shape_id();
        let r = f(&mut g.map);
        // The closure may have inserted / removed / renamed keys arbitrarily.
        // Restore the invariant: rebuild the index if the struct is large,
        // otherwise retire it (small structs scan). Off the hot path.
        if g.map.len() > CI_THRESHOLD {
            g.ci_rebuild();
        } else {
            g.ci.clear();
        }
        r
    }

    /// Run a closure with shared (read) access. Same re-entrancy caveat.
    #[inline]
    pub fn with_read<R>(&self, f: impl FnOnce(&ValueMap) -> R) -> R {
        f(&self.0.read().map)
    }

    /// Get the value at `key` as a shared struct handle, inserting a fresh
    /// empty struct if the key is absent (or holds a non-struct). Returns the
    /// handle so the caller can mutate it (visible to all aliases). Holds the
    /// write guard only for the get-or-insert — never calls user code — so it
    /// can't deadlock. The replacement template for the old
    /// `entry(..).or_insert_with(..)` + `as_struct_mut()` idiom.
    /// v0.99.4 — shape_id bumps iff the key was absent OR held a non-struct
    /// (in either case the entry is overwritten / created).
    pub fn get_or_insert_struct(&self, key: &str) -> CfmlStruct {
        let mut g = self.0.write();
        // Case-insensitive locate, matching `insert`'s write semantics: an
        // existing key under a different casing (`assetManager` vs
        // `assetmanager`) must be navigated into, NOT forked into a second
        // physical entry. Forking here was the root of the Preside boot bug —
        // a nested dotted assignment `settings.assetmanager.x = v` created a
        // parallel lowercase key, and a later `structAppend` then merged both
        // (the partial fork last-writer-wins), dropping most keys.
        // Locate case-insensitively (O(1) index for large, scan for small).
        let existing_idx = if g.map.contains_key(key) {
            g.map.get_index_of(key)
        } else if let Some(orig) = g.resolve_ci_key(key).cloned() {
            g.map.get_index_of(&orig)
        } else {
            None
        };
        if let Some(idx) = existing_idx {
            let (_, entry) = g.map.get_index_mut(idx).expect("existing_idx in range");
            if let CfmlValue::Struct(s) = entry {
                return s.clone();
            }
            // Present but not a struct — overwrite in place (preserves the
            // original key casing/order), bumping the shape.
            let s = CfmlStruct::empty();
            *entry = CfmlValue::Struct(s.clone());
            g.shape_id = next_shape_id();
            return s;
        }
        // Brand-new key.
        let s = CfmlStruct::empty();
        g.map.insert(key.to_string(), CfmlValue::Struct(s.clone()));
        g.shape_id = next_shape_id();
        g.ci_note_new_key(fold_key(key).into_owned(), key.to_string());
        s
    }
}

impl FromIterator<(String, CfmlValue)> for CfmlStruct {
    fn from_iter<I: IntoIterator<Item = (String, CfmlValue)>>(iter: I) -> Self {
        CfmlStruct::new(iter.into_iter().collect())
    }
}

/// Trait implemented by Rust types that want to be addressable as CFML
/// objects (`new rust:MyClass()` / member-call dispatch).
///
/// Implementers must be `Send + Sync` because instances can be shared across
/// cfthread boundaries via the surrounding `Arc<RwLock<…>>`. `Debug` is
/// required so the runtime can stringify native objects in dump output
/// without an extra trait.
///
/// `call_method` is the single dispatch entry point: the runtime looks up
/// `name` on the object and forwards `args`. Method names are matched
/// case-insensitively at the call site, so implementers can choose either
/// style — the convention is camelCase to match the rest of the CFML
/// surface.
pub trait CfmlNative: Send + Sync + fmt::Debug {
    /// Logical class name (e.g. "Counter"). Used for `type_name`,
    /// `getMetadata`, and dump output.
    fn class_name(&self) -> &str;

    /// Invoke a method on the underlying Rust value. Return
    /// `Err(CfmlError::…)` for unknown methods or argument mismatches.
    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult;

    /// Optional property read. Used when a CFC declares
    /// `extends="rust:Name"` and host code reads `this.X` (or `inst.X`)
    /// for a key the CFC struct doesn't define. Default returns `None` —
    /// the runtime falls back to the standard CFC property lookup.
    /// Implementers expose Rust-side state to the CFC half by returning
    /// `Some(value)` for the names they recognise.
    fn get_property(&self, _name: &str) -> Option<CfmlValue> {
        None
    }

    /// Optional property write. Mirrors `get_property`: return `None` to
    /// let the CFC struct take the assignment, or `Some(Ok(()))` /
    /// `Some(Err(…))` to indicate the native side handled (or rejected)
    /// the write. Default returns `None`.
    fn set_property(&mut self, _name: &str, _value: CfmlValue) -> Option<Result<(), crate::vm::CfmlError>> {
        None
    }
}

#[derive(Clone)]
pub enum CfmlValue {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    /// A CFML timespan (the value produced by `createTimeSpan`/`createTimespan`).
    /// Numerically it IS a `Double` — the count of fractional days (Lucee/ACF
    /// semantics: `createTimeSpan(1,0,0,0)` == 1.0), and it behaves exactly like
    /// `Double` in every arithmetic, comparison, coercion and stringification
    /// context. It is a distinct variant ONLY so the engine can answer the two
    /// type-introspection questions Lucee answers via its dedicated `TimeSpan`
    /// class: `x.getClass().getName()` (→ a name containing "timespan") and the
    /// `timespan` argument-type / `isValid("timespan", x)` check. Without a
    /// distinct type a timespan is indistinguishable from a plain number, which
    /// broke Preside's `AdHocTaskManagerService._isTimespan()` (a `getClass()`
    /// class-name sniff) and `timespan`-typed params. Treat it as `Double`
    /// everywhere except those introspection sites.
    TimeSpan(f64),
    /// CFML string value. Wrapped in `Arc<String>` (v0.87.0) so cloning a
    /// `CfmlValue::String` is an `Arc::clone` (refcount bump) instead of a
    /// heap allocation + copy. Mutating string ops (rare in CFML — strings
    /// are usually returned as new values from `uCase`/`trim`/...) should
    /// use `Arc::make_mut` for copy-on-write. The prerequisite for Option-γ
    /// tag-pointer polymorphic values inside the JIT (`JIT_POLY_DESIGN.md`).
    String(Arc<String>),
    /// Reference-typed array (Lucee semantics): a shared, interior-mutable
    /// handle. Aliases see each other's mutations. See `CfmlArray`.
    Array(CfmlArray),
    /// Lucee-style query column proxy: behaves as Array for iteration/indexing/length,
    /// but stringifies to the query's current-row value (so `q.col & "x"` works) and
    /// reports `type_name()` as "Array" so `isArray()` is true. Produced by
    /// `query.colname` member-access on a Query. The first payload is the column's row
    /// values; the second is the 0-based row the proxy stands in for in scalar contexts
    /// — snapshotted from the query's cursor at access time, so it reflects the current
    /// row inside a `<cfloop query>`/`<cfoutput query>` (0 = first row, the default).
    QueryColumn(Arc<Vec<CfmlValue>>, usize),
    /// Reference-typed struct (Lucee semantics): a shared, interior-mutable
    /// handle. Aliases (and CFC instances sharing it) see each other's
    /// mutations. See `CfmlStruct`.
    Struct(CfmlStruct),
    Closure(Box<CfmlClosure>),
    Component(Box<CfmlComponent>),
    // `Arc`-handle (was `Box<CfmlFunction>`): a `CfmlFunction` carries a `name`
    // String, a `params` Vec<CfmlParam>, and a body — so a `Box` clone deep-copied
    // all of it plus a fresh allocation. Profiling stock Wheels (`/posts`, 100-row
    // ORM + view render) showed ~50% of request CPU was `CfmlFunction` clone+drop:
    // scopes are IndexMaps full of CFC-method `Function` values, and every scope
    // clone (per call / per CFC-method dispatch) deep-cloned every method. Sharing
    // the function behind an `Arc` makes a `CfmlValue::Function` clone a refcount
    // bump (no alloc, no copy) — the same handle pattern already used for String/
    // Array/Struct/Query. Still an 8 B pointer, so `CfmlValue` stays 32 B. Arc
    // deref-coerces, so field/method reads are unchanged; in-place field writes
    // (only `captured_scope`) go through `Arc::make_mut` (copy-on-write).
    Function(Arc<CfmlFunction>),
    /// Reference-typed query (Lucee/BoxLang semantics): a shared, interior-
    /// mutable handle. `b = a` aliases (a mutation through either is visible
    /// through both); `duplicate(a)` deep-copies. The `Arc` is the indirection,
    /// so no `Box` is needed. See `CfmlQuery`.
    Query(CfmlQuery),
    Binary(Vec<u8>),
    /// Instance of a Rust-backed class registered via
    /// `CfmlVirtualMachine::register_native_class`. Method dispatch goes
    /// through the `CfmlNative` trait.
    NativeObject(Arc<RwLock<dyn CfmlNative>>),
}

thread_local! {
    /// Backing-Arc pointers of the containers currently being Debug-formatted.
    /// Reference-typed arrays/structs can alias and form cycles (e.g. a TestBox
    /// mock holds `this.mockBox`, whose generator holds the mock back); without
    /// this guard `{:?}` — used by writeDump and logging — recurses until the
    /// native stack overflows and aborts the whole process (uncatchable SIGABRT).
    static DEBUG_VISITED: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Hand-rolled Debug elides the Arc<_> wrapper on Array/Struct so log diffs
/// and test output remain byte-identical to the pre-Arc-flip representation.
impl fmt::Debug for CfmlValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CfmlValue::Null => f.write_str("Null"),
            CfmlValue::Bool(b) => f.debug_tuple("Bool").field(b).finish(),
            CfmlValue::Int(i) => f.debug_tuple("Int").field(i).finish(),
            CfmlValue::Double(d) => f.debug_tuple("Double").field(d).finish(),
            CfmlValue::TimeSpan(d) => f.debug_tuple("TimeSpan").field(d).finish(),
            CfmlValue::String(s) => f.debug_tuple("String").field(s).finish(),
            CfmlValue::Array(a) => {
                let ptr = a.backing_ptr();
                if DEBUG_VISITED.with(|v| v.borrow().contains(&ptr)) {
                    return f.write_str("Array(<recursive>)");
                }
                DEBUG_VISITED.with(|v| v.borrow_mut().push(ptr));
                let r = f.debug_tuple("Array").field(&a.snapshot()).finish();
                DEBUG_VISITED.with(|v| { v.borrow_mut().pop(); });
                r
            }
            CfmlValue::QueryColumn(a, row) => f.debug_tuple("QueryColumn").field(&**a).field(row).finish(),
            CfmlValue::Struct(s) => {
                let ptr = s.backing_ptr();
                if DEBUG_VISITED.with(|v| v.borrow().contains(&ptr)) {
                    return f.write_str("Struct(<recursive>)");
                }
                DEBUG_VISITED.with(|v| v.borrow_mut().push(ptr));
                let r = f.debug_tuple("Struct").field(&s.snapshot()).finish();
                DEBUG_VISITED.with(|v| { v.borrow_mut().pop(); });
                r
            }
            CfmlValue::Closure(c) => f.debug_tuple("Closure").field(c).finish(),
            CfmlValue::Component(c) => f.debug_tuple("Component").field(c).finish(),
            CfmlValue::Function(fun) => f.debug_tuple("Function").field(fun).finish(),
            CfmlValue::Query(q) => f.debug_tuple("Query").field(q).finish(),
            CfmlValue::Binary(b) => f.debug_tuple("Binary").field(b).finish(),
            CfmlValue::NativeObject(obj) => match obj.read() {
                Ok(g) => f
                    .debug_tuple("NativeObject")
                    .field(&g.class_name().to_string())
                    .finish(),
                Err(_) => f.debug_tuple("NativeObject").field(&"<poisoned>").finish(),
            },
        }
    }
}

impl CfmlValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            CfmlValue::Null => "Null",
            CfmlValue::Bool(_) => "Boolean",
            CfmlValue::Int(_) => "Integer",
            CfmlValue::Double(_) => "Double",
            // A timespan is numerically a Double; report it as such so any
            // type-name-based numeric handling treats it identically. Its
            // distinct identity is surfaced only via getClass()/the timespan
            // type-check, which match the variant directly.
            CfmlValue::TimeSpan(_) => "Double",
            CfmlValue::String(_) => "String",
            CfmlValue::Array(_) => "Array",
            // Lucee@7: `isArray(q.col)` is false — QueryColumn is a string proxy
            // with bracket-indexing for rows, not an array. Distinct type_name
            // means isArray/isStruct/etc. all report false.
            CfmlValue::QueryColumn(..) => "QueryColumn",
            CfmlValue::Struct(_) => "Struct",
            CfmlValue::Closure(_) => "Closure",
            CfmlValue::Component(_) => "Component",
            CfmlValue::Function(_) => "Function",
            CfmlValue::Query(_) => "Query",
            CfmlValue::Binary(_) => "Binary",
            CfmlValue::NativeObject(_) => "NativeObject",
        }
    }

    pub fn is_true(&self) -> bool {
        match self {
            CfmlValue::Null => false,
            CfmlValue::Bool(b) => *b,
            CfmlValue::Int(i) => *i != 0,
            CfmlValue::Double(d) => *d != 0.0,
            CfmlValue::TimeSpan(d) => *d != 0.0,
            CfmlValue::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return false;
                }
                match trimmed.to_lowercase().as_str() {
                    "false" | "no" | "0" => false,
                    _ => true,
                }
            }
            CfmlValue::Array(a) => !a.is_empty(),
            // (CfmlArray::is_empty locks briefly.)
            // QueryColumn truthiness: the current row's truthiness (Lucee proxies
            // to the query's cursor row; falls back to the first row).
            CfmlValue::QueryColumn(a, row) => {
                a.get(*row).or_else(|| a.first()).map(|v| v.is_true()).unwrap_or(false)
            }
            CfmlValue::Struct(s) => !s.is_empty(),
            CfmlValue::Closure(_) => true,
            CfmlValue::Component(_) => true,
            CfmlValue::Function(_) => true,
            CfmlValue::Query(q) => !q.is_empty(),
            CfmlValue::Binary(b) => !b.is_empty(),
            CfmlValue::NativeObject(_) => true,
        }
    }

    pub fn as_string(&self) -> String {
        let mut path: Vec<usize> = Vec::new();
        let mut memo: HashMap<usize, String> = HashMap::new();
        self.as_string_memo(&mut path, &mut memo).0
    }

    /// Lucee-parity strict string coercion for the contexts Lucee *rejects* for
    /// complex values — the `&` concat operator, output (`<cfoutput>#x#</cfoutput>`
    /// / `writeOutput`), and `toString()`. Lucee throws
    /// `Can't cast Complex Object Type [Struct] to String` (type `expression`)
    /// rather than dumping a `{k: v}` representation; RustCFML historically
    /// produced the dump, which — on a densely cross-linked object graph like
    /// WireBox's injector↔binder↔builder — expanded to an O(2^depth) string and
    /// hung the process (ColdBox boot). Matching Lucee both fixes that and
    /// surfaces the real coercion site to the CFML author.
    ///
    /// Scalars, dates, binary, XML, Java `NativeObject`s and `QueryColumn`
    /// proxies coerce normally (Lucee casts those); only the genuinely-complex
    /// types throw.
    pub fn to_string_strict(&self) -> Result<String, CfmlError> {
        match self {
            // A Java-object shim (`createObject("java", …)`) is represented
            // internally as a tagged struct, but Lucee coerces Java objects to
            // their `toString()` in string contexts (concat, output) rather than
            // throwing — e.g. `"" & java.util.UUID.randomUUID()` yields the UUID
            // string. Route those through the shim stringifier; only genuine
            // CFML structs throw.
            CfmlValue::Struct(s) if java_shim_string(s).is_some() => {
                Ok(java_shim_string(s).unwrap())
            }
            CfmlValue::Struct(_) => Err(CfmlError::expression(
                "Can't cast Complex Object Type [Struct] to String".to_string(),
            )),
            CfmlValue::Array(_) => Err(CfmlError::expression(
                "Can't cast Complex Object Type [Array] to String".to_string(),
            )),
            CfmlValue::Query(_) => Err(CfmlError::expression(
                "Can't cast Complex Object Type [Query] to String".to_string(),
            )),
            CfmlValue::Component(c) => Err(CfmlError::expression(format!(
                "Can't cast Component [{}] to String",
                c.name
            ))),
            CfmlValue::Function(f) => Err(CfmlError::expression(format!(
                "Can't cast Object type [user defined function ({})] to a value of type [string]",
                f.name
            ))),
            CfmlValue::Closure(_) => Err(CfmlError::expression(
                "Can't cast Object type [user defined function (closure)] to a value of type [string]"
                    .to_string(),
            )),
            _ => Ok(self.as_string()),
        }
    }

    /// Content-deterministic stringification: identical to `as_string` except a
    /// `Struct`'s keys are emitted in case-insensitive sorted order rather than
    /// insertion order, recursively.
    ///
    /// Lucee/ACF back a plain `{}` struct with a Java `HashMap`, so its
    /// `toString()` is hash-bucket order — neither insertion nor alphabetical,
    /// but DETERMINISTIC FOR A GIVEN CONTENT (the same keys always stringify the
    /// same way regardless of how the struct was built). RustCFML's `IndexMap`
    /// is insertion-ordered, so two structs with identical content but different
    /// build order stringify differently. That bites any code that hashes a
    /// stringified struct as an identity key — notably TestBox/MockBox's
    /// `normalizeArguments()`, which `$args( {...} )` then matches against the
    /// struct the system-under-test builds (e.g. Preside's
    /// `AdHocTaskManagerService.createTask` lists `next_attempt_date` /
    /// `retry_interval` in a different order than the spec's `$args` literal).
    /// On RustCFML the hashes diverged, the mock fell through to a null result,
    /// and `var x = mock(...)` deleted `x` → "Variable X undefined". Sorting the
    /// keys makes our `toString()` content-deterministic like Lucee's, so the
    /// setup and call hashes match. See docs/known-issues.md §15.
    pub fn to_string_sorted(&self) -> String {
        let mut path: Vec<usize> = Vec::new();
        let mut memo: HashMap<usize, String> = HashMap::new();
        self.to_string_sorted_memo(&mut path, &mut memo).0
    }

    /// Sorted-key counterpart of [`as_string_memo`]. Same memoization contract:
    /// `path` guards cycles on the current chain, `memo` caches the rendered
    /// string of every *clean* (cycle-free) container so a shared sub-graph is
    /// rendered once, not once per path to it.
    fn to_string_sorted_memo(
        &self,
        path: &mut Vec<usize>,
        memo: &mut HashMap<usize, String>,
    ) -> (String, bool) {
        match self {
            CfmlValue::Array(a) => {
                let ptr = a.backing_ptr();
                if path.contains(&ptr) {
                    return ("[...]".to_string(), false);
                }
                if let Some(cached) = memo.get(&ptr) {
                    return (cached.clone(), true);
                }
                path.push(ptr);
                let mut clean = true;
                let items: Vec<String> = a
                    .snapshot()
                    .iter()
                    .map(|v| {
                        let (s, c) = v.to_string_sorted_memo(path, memo);
                        clean &= c;
                        s
                    })
                    .collect();
                path.pop();
                let out = format!("[{}]", items.join(", "));
                if clean {
                    memo.insert(ptr, out.clone());
                }
                (out, clean)
            }
            CfmlValue::Struct(s) => {
                if let Some(js) = java_shim_string(s) {
                    return (js, true);
                }
                let ptr = s.backing_ptr();
                if path.contains(&ptr) {
                    return ("{...}".to_string(), false);
                }
                if let Some(cached) = memo.get(&ptr) {
                    return (cached.clone(), true);
                }
                path.push(ptr);
                let mut entries: Vec<(String, CfmlValue)> = s.iter().collect();
                entries.sort_by(|a, b| {
                    a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.0.cmp(&b.0))
                });
                let mut clean = true;
                let items: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let (s, c) = v.to_string_sorted_memo(path, memo);
                        clean &= c;
                        format!("{}: {}", k, s)
                    })
                    .collect();
                path.pop();
                let out = format!("{{{}}}", items.join(", "));
                if clean {
                    memo.insert(ptr, out.clone());
                }
                (out, clean)
            }
            // Everything else stringifies identically to as_string.
            _ => self.as_string_memo(path, memo),
        }
    }

    /// Cycle- *and* sharing-guarded stringification. Structs/arrays are reference
    /// types, so an object graph can contain both cycles (WireBox's injector ↔
    /// binder ↔ builder) and *shared* sub-graphs reachable by many paths (a
    /// densely cross-linked config/metadata tree). The old per-path `visited`
    /// guard stopped cycles from overflowing the stack, but a shared child was
    /// still re-rendered once per path to it — O(2^depth) time and intermediate
    /// string allocation. On ColdBox boot that hung the process at ~14 GB RSS.
    ///
    /// Two-part guard:
    /// - `path` is the set of container pointers on the *current* recursion
    ///   chain; revisiting one is a genuine cycle → emit `{...}`/`[...]`.
    /// - `memo` caches the finished string of every container whose whole
    ///   sub-graph rendered *without* hitting a cycle placeholder ("clean"). A
    ///   shared clean sub-graph is then rendered once and reused, collapsing the
    ///   exponential blow-up to O(nodes). The output is byte-identical to the old
    ///   full re-rendering — only faster.
    ///
    /// A node is memoized only when clean, because a string that embedded a
    /// `{...}` placeholder is context-dependent (the placeholder fired only
    /// because an ancestor was mid-render) and must not be reused on another path.
    /// Returns `(rendered, clean)`.
    fn as_string_memo(
        &self,
        path: &mut Vec<usize>,
        memo: &mut HashMap<usize, String>,
    ) -> (String, bool) {
        match self {
            CfmlValue::Null => (String::new(), true),
            CfmlValue::Bool(b) => (b.to_string(), true),
            CfmlValue::Int(i) => (i.to_string(), true),
            CfmlValue::Double(d) => (format_double(*d), true),
            // Stringifies exactly like its fractional-day Double value, so string
            // concatenation and number-via-string coercion are unchanged.
            CfmlValue::TimeSpan(d) => (format_double(*d), true),
            CfmlValue::String(s) => ((**s).clone(), true),
            CfmlValue::Array(a) => {
                let ptr = a.backing_ptr();
                if path.contains(&ptr) {
                    return ("[...]".to_string(), false);
                }
                if let Some(cached) = memo.get(&ptr) {
                    return (cached.clone(), true);
                }
                path.push(ptr);
                let mut clean = true;
                let items: Vec<String> = a
                    .snapshot()
                    .iter()
                    .map(|v| {
                        let (s, c) = v.as_string_memo(path, memo);
                        clean &= c;
                        s
                    })
                    .collect();
                path.pop();
                let out = format!("[{}]", items.join(", "));
                if clean {
                    memo.insert(ptr, out.clone());
                }
                (out, clean)
            }
            // QueryColumn stringifies to the current-row value, matching Lucee's
            // proxy behavior so `q.col & "x"` concatenates the query's cursor row
            // (falls back to the first row).
            CfmlValue::QueryColumn(a, row) => (
                a.get(*row).or_else(|| a.first()).map(|v| v.as_string()).unwrap_or_default(),
                true,
            ),
            CfmlValue::Struct(s) => {
                // A java.util.Locale shim stringifies to its Java-style id
                // (`en`, `en_US`) — matching Locale.toString() — so cbi18n's
                // `arrayToList( Locale.getAvailableLocales() )` yields the ids
                // it validates against (rather than a struct dump).
                if let Some(js) = java_shim_string(s) {
                    return (js, true);
                }
                let ptr = s.backing_ptr();
                if path.contains(&ptr) {
                    return ("{...}".to_string(), false);
                }
                if let Some(cached) = memo.get(&ptr) {
                    return (cached.clone(), true);
                }
                path.push(ptr);
                let mut clean = true;
                let items: Vec<String> = s
                    .iter()
                    .map(|(k, v)| {
                        let (sv, c) = v.as_string_memo(path, memo);
                        clean &= c;
                        format!("{}: {}", k, sv)
                    })
                    .collect();
                path.pop();
                let out = format!("{{{}}}", items.join(", "));
                if clean {
                    memo.insert(ptr, out.clone());
                }
                (out, clean)
            }
            CfmlValue::Closure(_) => ("<Closure>".to_string(), true),
            CfmlValue::Component(_) => ("<Component>".to_string(), true),
            CfmlValue::Function(f) => (f.name.clone(), true),
            CfmlValue::Query(_) => ("<Query>".to_string(), true),
            CfmlValue::Binary(_) => ("<Binary>".to_string(), true),
            CfmlValue::NativeObject(obj) => match obj.read() {
                Ok(g) => (format!("<NativeObject:{}>", g.class_name()), true),
                Err(_) => ("<NativeObject:poisoned>".to_string(), true),
            },
        }
    }

    /// For a `QueryColumn` proxy, the scalar value it stands in for — its first
    /// row (Lucee treats `q.col` as a proxy that behaves like the first row in
    /// scalar contexts: numeric coercion, comparison). For anything else,
    /// returns `self` unchanged.
    ///
    /// A NULL first cell (or an empty column) resolves to the empty string,
    /// not `Null`: with full-null support off — the engine default — Lucee/ACF
    /// read a NULL query cell as `""`, so `q.col EQ ""`, `isSimpleValue(q.col)`,
    /// and `Len(q.col)` all behave as for an empty string. (Without this, an
    /// aggregate over zero matching rows — `SELECT MAX(x) … WHERE id=0`, one
    /// row, NULL cell — compared `!=` to `""` and reported as non-simple.)
    pub fn query_column_scalar(&self) -> &CfmlValue {
        static EMPTY: std::sync::LazyLock<CfmlValue> =
            std::sync::LazyLock::new(|| CfmlValue::String(Arc::new(String::new())));
        match self {
            CfmlValue::QueryColumn(a, row) => match a.get(*row).or_else(|| a.first()) {
                Some(CfmlValue::Null) | None => &*EMPTY,
                Some(v) => v,
            },
            _ => self,
        }
    }

    pub fn get(&self, key: &str) -> Option<CfmlValue> {
        match self {
            CfmlValue::Struct(s) => s.get(key),
            CfmlValue::Array(a) => key.parse::<usize>().ok().and_then(|idx| a.get(idx)),
            CfmlValue::QueryColumn(a, _) => {
                if let Ok(idx) = key.parse::<usize>() {
                    a.get(idx).cloned()
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Case-insensitive struct-key lookup (CFML keys are case-insensitive).
    /// Mirrors `get` but resolves struct members regardless of casing — e.g.
    /// `this.MockBox` reaching a stored `mockbox`. Arrays/query columns are
    /// numeric-indexed, so casing does not apply; they defer to `get`.
    pub fn get_ci(&self, key: &str) -> Option<CfmlValue> {
        match self {
            CfmlValue::Struct(s) => s.get_ci(key),
            other => other.get(key),
        }
    }

    pub fn set(&mut self, key: String, value: CfmlValue) {
        match self {
            CfmlValue::Struct(s) => {
                s.insert(key, value);
            }
            CfmlValue::Array(a) => {
                if let Ok(idx) = key.parse::<usize>() {
                    // Interior mutability: no `&mut`/make_mut needed; the shared
                    // backing is updated so aliases observe the write.
                    a.set(idx, value);
                }
            }
            CfmlValue::Query(q) => {
                // Dot-form column write-back `q.col = arrayOrColumn` (the outer
                // step of `q.col[row] = v`). Replace the column in place on the
                // shared query so all aliases observe it.
                let new_values: Vec<CfmlValue> = match value {
                    CfmlValue::QueryColumn(a, _) => a.as_ref().clone(),
                    CfmlValue::Array(a) => a.snapshot(),
                    other => vec![other],
                };
                q.set_column(&key, new_values);
            }
            _ => {}
        }
    }

    /// Construct a `CfmlValue::String` from anything `Into<String>`. Wraps
    /// the owned `String` in an `Arc` so cloning a `CfmlValue::String` is a
    /// refcount bump instead of a heap allocation. Use this helper at every
    /// new construction site; pattern matches stay unchanged thanks to
    /// `Arc`'s `Deref<Target = String>`.
    #[inline]
    pub fn string(s: impl Into<String>) -> Self {
        CfmlValue::String(Arc::new(s.into()))
    }

    /// Construct a `CfmlValue::Array` from an owned `Vec`, wrapping in the
    /// shared Arc layer. `#[inline]` because this is called from every
    /// Array-producing builtin across crate boundaries.
    #[inline]
    pub fn array(v: Vec<CfmlValue>) -> Self {
        CfmlValue::Array(CfmlArray::new(v))
    }

    /// Construct a `CfmlValue::Struct` from an owned `IndexMap`, wrapping in
    /// the shared Arc layer. Named `strukt` because `struct` is a keyword.
    #[inline]
    pub fn strukt(m: ValueMap) -> Self {
        CfmlValue::Struct(CfmlStruct::new(m))
    }

    /// Borrow the shared array handle (no copy). Mutating through it is visible
    /// to all aliases. Returns `None` for non-arrays (QueryColumn excluded).
    pub fn as_cfml_array(&self) -> Option<&CfmlArray> {
        match self {
            CfmlValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// A point-in-time copy of the array's elements. Returns `None` for
    /// non-arrays. (A snapshot, not a borrow — the backing is behind a lock.)
    pub fn as_array(&self) -> Option<Vec<CfmlValue>> {
        match self {
            CfmlValue::Array(a) => Some(a.snapshot()),
            _ => None,
        }
    }

    /// Like `as_array` but also returns the row view when called on a
    /// `QueryColumn`. Use for narrow opt-in cases (e.g. `valueList(q.col)`
    /// which canonically iterates rows on Lucee). Most array consumers
    /// should stay on `as_array` so that `arrayLen(q.col)` etc. cleanly
    /// reject the value, matching Lucee@7.
    pub fn as_array_or_query_column(&self) -> Option<Vec<CfmlValue>> {
        match self {
            CfmlValue::Array(a) => Some(a.snapshot()),
            CfmlValue::QueryColumn(a, _) => Some((**a).clone()),
            _ => None,
        }
    }

    /// Borrow the shared struct handle (no copy). Mutating through it is visible
    /// to all aliases. Returns `None` for non-structs.
    pub fn as_cfml_struct(&self) -> Option<&CfmlStruct> {
        match self {
            CfmlValue::Struct(s) => Some(s),
            _ => None,
        }
    }

    /// A point-in-time copy of the struct's entries. Returns `None` for
    /// non-structs. (A snapshot, not a borrow — the backing is behind a lock.)
    pub fn as_struct(&self) -> Option<ValueMap> {
        match self {
            CfmlValue::Struct(s) => Some(s.snapshot()),
            _ => None,
        }
    }

    /// Recursively copy a value, breaking all shared references. Arrays and
    /// structs get fresh backing stores with deep-copied elements, so the
    /// result is fully independent of the source (this is what `duplicate()`
    /// must do now that arrays/structs are reference-typed — a plain `clone()`
    /// only shares the handle). Scalars/immutable variants fall back to
    /// `clone()`. Internal aliasing is PRESERVED: a struct/array/query reachable
    /// from more than one place in the source graph (a DAG, or a cycle where it
    /// is reachable from itself) maps to a single shared copy in the result —
    /// matching Lucee's `duplicate()`, which keeps shared references shared and
    /// terminates on circular references. The `seen` map records, per source
    /// backing-store pointer, the new copy already created for it; revisiting a
    /// pointer returns that same copy rather than splitting it into an
    /// independent duplicate. (This is also what makes component instantiation
    /// correct: a single object stored in both `this.x` and `variables.x` stays
    /// one shared reference after the instance template is deep-copied.)
    pub fn deep_copy(&self) -> CfmlValue {
        let mut seen: HashMap<usize, CfmlValue> = HashMap::new();
        self.deep_copy_guarded(&mut seen)
    }

    /// Deep-copy sharing a caller-supplied `seen` map, so that a series of
    /// deep-copies preserves aliasing ACROSS calls: an object already copied in
    /// an earlier `deep_copy_with` (recorded in `seen`) resolves to that same
    /// copy here. Component instantiation relies on this — the instance's `this`
    /// scope is deep-copied first, then its `variables` scope is deep-copied
    /// through the same map, so an object the pseudo-constructor stored in both
    /// `this.x` and `variables.x` stays one shared reference in the instance.
    pub fn deep_copy_with(&self, seen: &mut HashMap<usize, CfmlValue>) -> CfmlValue {
        self.deep_copy_guarded(seen)
    }

    fn deep_copy_guarded(&self, seen: &mut HashMap<usize, CfmlValue>) -> CfmlValue {
        match self {
            CfmlValue::Array(a) => {
                let ptr = a.backing_ptr();
                if let Some(existing) = seen.get(&ptr) {
                    return existing.clone();
                }
                // Register the (empty) destination BEFORE recursing so a cycle
                // or a second reference to this same array resolves to this one
                // copy instead of recursing forever / splitting into two.
                let dest = CfmlArray::empty();
                seen.insert(ptr, CfmlValue::Array(dest.clone()));
                let items: Vec<CfmlValue> =
                    a.snapshot().iter().map(|v| v.deep_copy_guarded(seen)).collect();
                dest.with_write(|w| *w = items);
                CfmlValue::Array(dest)
            }
            CfmlValue::Struct(s) => {
                let ptr = s.backing_ptr();
                if let Some(existing) = seen.get(&ptr) {
                    return existing.clone();
                }
                let dest = CfmlStruct::empty();
                seen.insert(ptr, CfmlValue::Struct(dest.clone()));
                let entries: ValueMap =
                    s.iter().map(|(k, v)| (k, v.deep_copy_guarded(seen))).collect();
                dest.with_write(|w| *w = entries);
                CfmlValue::Struct(dest)
            }
            // Queries are reference-typed, so `duplicate()` must break the
            // shared handle: snapshot the data (releases the lock), deep-copy
            // every cell, and wrap in a fresh backing store.
            CfmlValue::Query(q) => {
                let ptr = q.backing_ptr();
                if let Some(existing) = seen.get(&ptr) {
                    return existing.clone();
                }
                // Pre-register the original handle as a cycle-breaker (a query
                // reachable from itself terminates); overwrite with the real
                // copy once built so later DAG revisits share the duplicate.
                seen.insert(ptr, self.clone());
                let (columns, data, sql) =
                    q.with_read(|d| (d.columns.clone(), d.data.clone(), d.sql.clone()));
                // Genuinely deep-copy each column so the duplicate shares NO
                // storage with the original. Arc::clone alone wouldn't suffice —
                // a later mutation through `duplicate(q)` would CoW the column
                // but the per-cell nested arrays/structs would still alias.
                let data: Vec<Arc<Vec<CfmlValue>>> = data
                    .into_iter()
                    .map(|col| {
                        Arc::new(
                            col.iter().map(|v| v.deep_copy_guarded(seen)).collect(),
                        )
                    })
                    .collect();
                let copy = CfmlValue::Query(CfmlQuery::from_data(CfmlQueryData { columns, data, sql, execution_time: None, current_row: 1 }));
                seen.insert(ptr, copy.clone());
                copy
            }
            other => other.clone(),
        }
    }

    pub fn eq(&self, other: &CfmlValue) -> bool {
        // A timespan compares as its fractional-day Double value. Rewrite either
        // operand to Double up-front so all the numeric arms below apply without
        // duplicating every Int/Double combination for TimeSpan.
        if let CfmlValue::TimeSpan(d) = self {
            return CfmlValue::Double(*d).eq(other);
        }
        if let CfmlValue::TimeSpan(d) = other {
            return self.eq(&CfmlValue::Double(*d));
        }
        match (self, other) {
            (CfmlValue::Null, CfmlValue::Null) => true,
            // NativeObjects compare by identity: two CFML references that
            // point at the same underlying Rust object are equal. A second
            // `createObject("rust", "Name")` returns a fresh Arc and so is
            // NOT equal even if the Rust state matches.
            (CfmlValue::NativeObject(a), CfmlValue::NativeObject(b)) => Arc::ptr_eq(a, b),
            (CfmlValue::Bool(a), CfmlValue::Bool(b)) => a == b,
            (CfmlValue::Int(a), CfmlValue::Int(b)) => a == b,
            (CfmlValue::Double(a), CfmlValue::Double(b)) => a == b,
            (CfmlValue::String(a), CfmlValue::String(b)) => a.to_lowercase() == b.to_lowercase(),
            (CfmlValue::Int(a), CfmlValue::Double(b)) => *a as f64 == *b,
            (CfmlValue::Double(a), CfmlValue::Int(b)) => *a == *b as f64,
            (CfmlValue::Array(a), CfmlValue::Array(b)) => {
                // Identity short-circuit avoids locking the same array twice
                // (and terminates self-referential structures).
                if a.ptr_eq(b) {
                    return true;
                }
                // Snapshot to release the locks before the (possibly recursive)
                // element comparison — prevents re-entrant lock deadlocks.
                let (a, b) = (a.snapshot(), b.snapshot());
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq(y))
            }
            (
                CfmlValue::Array(a),
                CfmlValue::QueryColumn(b, _),
            ) => {
                let a = a.snapshot();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq(y))
            }
            (
                CfmlValue::QueryColumn(a, _),
                CfmlValue::Array(b),
            ) => {
                let b = b.snapshot();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq(y))
            }
            (CfmlValue::QueryColumn(a, _), CfmlValue::QueryColumn(b, _)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq(y))
            }
            (CfmlValue::Struct(a), CfmlValue::Struct(b)) => {
                // Identity short-circuit avoids locking the same struct twice
                // (and terminates self-referential structures).
                if a.ptr_eq(b) {
                    return true;
                }
                // Snapshot both sides to release the locks before the (possibly
                // recursive) value comparison — prevents re-entrant deadlocks.
                let (a, b) = (a.snapshot(), b.snapshot());
                if a.len() != b.len() {
                    return false;
                }
                a.iter()
                    .all(|(k, v)| b.get(k).map(|bv| v.eq(bv)).unwrap_or(false))
            }
            // Queries compare by reference identity (Lucee errors on query
            // comparison; pointer-equality is the safe, useful answer — two
            // handles onto the same data are equal, distinct queries are not).
            (CfmlValue::Query(a), CfmlValue::Query(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

impl Default for CfmlValue {
    fn default() -> Self {
        CfmlValue::Null
    }
}

impl fmt::Display for CfmlValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_string())
    }
}

#[derive(Debug, Clone)]
pub struct CfmlClosure {
    pub params: Vec<String>,
    pub body: Box<CfmlClosureBody>,
    pub captured_vars: ValueMap,
}

#[derive(Debug, Clone)]
pub enum CfmlClosureBody {
    Expression(Box<CfmlValue>),
    Statements(Vec<CfmlStatement>),
}

#[derive(Debug, Clone)]
pub enum CfmlStatement {
    Expression(CfmlValue),
    Return(Option<CfmlValue>),
    Assignment(String, CfmlValue),
}

#[derive(Debug, Clone)]
pub struct CfmlComponent {
    pub name: String,
    pub properties: ValueMap,
    pub methods: HashMap<String, CfmlFunction>,
    pub extends: Option<String>,
    pub implements: Vec<String>,
}

impl CfmlComponent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            properties: ValueMap::default(),
            methods: HashMap::new(),
            extends: None,
            implements: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CfmlFunction {
    pub name: String,
    pub params: Vec<CfmlParam>,
    pub body: CfmlClosureBody,
    pub return_type: Option<String>,
    pub access: CfmlAccess,
    /// Captured scope for closures — shared mutable environment so multiple
    /// invocations (and sibling closures) see each other's mutations.
    pub captured_scope: Option<Arc<RwLock<ValueMap>>>,
}

#[derive(Debug, Clone)]
pub struct CfmlParam {
    pub name: String,
    pub param_type: Option<String>,
    pub default: Option<CfmlValue>,
    pub required: bool,
    /// Javadoc-style annotations attached to this parameter, e.g.
    /// `@configuredFeatures.inject coldbox:setting:features` → `("inject",
    /// "coldbox:setting:features")`. Surfaced in getMetadata()/
    /// getComponentMetadata() so WireBox-style DI can read `param.inject`.
    pub annotations: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CfmlAccess {
    Public,
    Private,
    Package,
    Remote,
}

/// Column-major backing data for a CFML query — the store behind the shared
/// [`CfmlQuery`] handle. Held directly (no lock) by the QoQ engine while it
/// builds a result; wrapped in a `CfmlQuery` handle at the value boundary.
///
/// `data[col_idx]` is one column's values in row order. All inner `Vec`s have
/// the same length (= [`row_count`](Self::row_count)). The outer `Vec` is
/// parallel to `columns`. Use [`row_at`](Self::row_at) /
/// [`synthesise_rows`](Self::synthesise_rows) to get a row-shaped view for
/// CFML callers that want struct rows.
#[derive(Debug, Clone, Default)]
pub struct CfmlQueryData {
    pub columns: Vec<String>,
    /// Column-major data. Each column is wrapped in `Arc<Vec<_>>` so that
    /// `CfmlQueryData::clone()` is O(columns) Arc bumps instead of deep-cloning
    /// every cell. Mutations go through `Arc::make_mut` — free when the column
    /// Arc is unique (the common case for in-place builders), copy-on-write
    /// otherwise.
    pub data: Vec<Arc<Vec<CfmlValue>>>,
    pub sql: Option<String>,
    /// Wall-clock execution time in milliseconds, recorded when the query was
    /// run via `queryExecute`/`cfquery`. `None` for queries built in memory
    /// (queryNew, QoQ before timing). Surfaced in `writeDump`'s query metadata.
    pub execution_time: Option<i64>,
    /// 1-based cursor row — the "current row" of the recordset. Advanced by
    /// `<cfloop query>`/`<cfoutput query>` so that `q.col` reads the current
    /// row's value and `q.currentRow` reports the position, matching Lucee/ACF
    /// (where the cursor lives on the query object). `0` is treated as row 1 —
    /// see [`current_row`](Self::current_row) — so `#[derive(Default)]` and the
    /// pre-cursor struct literals keep working.
    pub current_row: usize,
}

impl CfmlQueryData {
    /// Empty data block with the given columns.
    pub fn new(columns: Vec<String>) -> Self {
        let n = columns.len();
        Self { columns, data: (0..n).map(|_| Arc::new(Vec::new())).collect(), sql: None, execution_time: None, current_row: 1 }
    }

    /// The 1-based cursor row, normalising the `0` default to row 1.
    #[inline]
    pub fn current_row(&self) -> usize {
        if self.current_row == 0 { 1 } else { self.current_row }
    }

    /// Build from columns + already-row-shaped rows (the legacy IndexMap shape).
    /// Rows are unpacked into column-major storage; unknown columns in rows
    /// extend the column list (matching Lucee/ACF row-then-column behaviour).
    pub fn from_named_rows(
        columns: Vec<String>,
        rows: Vec<ValueMap>,
    ) -> Self {
        let mut q = Self::new(columns);
        for row in rows {
            q.push_row_named(row);
        }
        q
    }

    #[inline]
    pub fn column_count(&self) -> usize { self.columns.len() }

    #[inline]
    pub fn row_count(&self) -> usize { self.data.first().map_or(0, |c| c.len()) }

    #[inline]
    pub fn is_empty(&self) -> bool { self.row_count() == 0 }

    /// Case-insensitive column lookup.
    #[inline]
    pub fn column_index_ci(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.eq_ignore_ascii_case(name))
    }

    /// Borrow a cell by (row, col_idx).
    #[inline]
    pub fn cell(&self, row: usize, col_idx: usize) -> Option<&CfmlValue> {
        self.data.get(col_idx).and_then(|c| c.get(row))
    }

    #[inline]
    pub fn cell_mut(&mut self, row: usize, col_idx: usize) -> Option<&mut CfmlValue> {
        self.data.get_mut(col_idx).and_then(|c| Arc::make_mut(c).get_mut(row))
    }

    /// Set a cell by column name (CI). Unknown columns are added (pre-existing
    /// rows in that new column are Null). Returns false if `row` is out of range.
    pub fn set_cell_named(&mut self, row: usize, name: &str, val: CfmlValue) -> bool {
        if row >= self.row_count() {
            return false;
        }
        if let Some(ci) = self.column_index_ci(name) {
            Arc::make_mut(&mut self.data[ci])[row] = val;
        } else {
            self.columns.push(name.to_string());
            let rows = self.row_count();
            let mut col = vec![CfmlValue::Null; rows];
            col[row] = val;
            self.data.push(Arc::new(col));
        }
        true
    }

    /// Borrow one column's values by index.
    #[inline]
    pub fn column_data(&self, col_idx: usize) -> Option<&Vec<CfmlValue>> {
        self.data.get(col_idx).map(|a| a.as_ref())
    }

    /// Borrow one column's values by name (CI). Zero-copy.
    #[inline]
    pub fn column_data_ci(&self, name: &str) -> Option<&Vec<CfmlValue>> {
        self.column_index_ci(name).and_then(|i| self.data.get(i)).map(|a| a.as_ref())
    }

    /// Borrow one column's Arc directly — lets callers cheaply `Arc::clone` and
    /// share the column without re-cloning. Used by `column_values_ci` to hand
    /// the same Arc straight to `CfmlValue::QueryColumn`.
    #[inline]
    pub fn column_arc_ci(&self, name: &str) -> Option<&Arc<Vec<CfmlValue>>> {
        self.column_index_ci(name).and_then(|i| self.data.get(i))
    }

    /// Synthesise a single row as an `IndexMap` keyed by canonical column names.
    pub fn row_at(&self, row: usize) -> Option<ValueMap> {
        if row >= self.row_count() {
            return None;
        }
        let mut m = ValueMap::with_capacity_and_hasher(self.columns.len(), Default::default());
        for (ci, col) in self.columns.iter().enumerate() {
            // A SQL-NULL cell surfaces as an empty string, not `Null`. This is
            // the CFML default (`nullSupport = false`): every column of a query
            // row is a PRESENT key whose NULL value reads as "". A `Null` here
            // would make the column vanish from the row struct — `structKeyExists`
            // / `structKeyList` / `cfparam` treat a Null-valued key as absent —
            // so `for row in q { row.nullCol }` and a `param name="args.nullCol"
            // type="string"` (Preside sitetree `_node.cfm`) would wrongly see the
            // column as missing. Lucee/ACF include it as "".
            let cell = match &self.data[ci][row] {
                CfmlValue::Null => CfmlValue::string(String::new()),
                other => other.clone(),
            };
            m.insert(col.clone(), cell);
        }
        Some(m)
    }

    /// Synthesise every row as an `IndexMap` (used by Debug, serde, snapshot).
    pub fn synthesise_rows(&self) -> Vec<ValueMap> {
        (0..self.row_count()).map(|r| self.row_at(r).unwrap()).collect()
    }

    /// Fast path for `queryAddRow([positional])`. Extra values are dropped;
    /// missing cells filled with Null.
    pub fn push_row_positional(&mut self, mut vals: Vec<CfmlValue>) {
        let n = self.columns.len();
        vals.resize_with(n, || CfmlValue::Null);
        for (ci, v) in vals.into_iter().enumerate() {
            Arc::make_mut(&mut self.data[ci]).push(v);
        }
    }

    /// Append a row keyed by column name (CI). Any column in `row` that is not
    /// already known extends `columns` (and back-fills prior rows with Null).
    /// Missing columns get Null. Keeps the column-major invariant.
    pub fn push_row_named(&mut self, row: ValueMap) {
        // Extend columns with any new keys (rare in practice — most rows have
        // the same shape).
        for k in row.keys() {
            if self.column_index_ci(k).is_none() {
                self.columns.push(k.clone());
                let prev = self.row_count();
                self.data.push(Arc::new(vec![CfmlValue::Null; prev]));
            }
        }
        // Lowercase the row keys once for the lookup loop (case-insensitive
        // match against canonical columns).
        for ci in 0..self.columns.len() {
            let col_name = self.columns[ci].as_str();
            let val = row
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col_name))
                .map(|(_, v)| v.clone())
                .unwrap_or(CfmlValue::Null);
            Arc::make_mut(&mut self.data[ci]).push(val);
        }
    }

    pub fn insert_row_positional(&mut self, at: usize, mut vals: Vec<CfmlValue>) {
        let n = self.columns.len();
        vals.resize_with(n, || CfmlValue::Null);
        for (ci, v) in vals.into_iter().enumerate() {
            Arc::make_mut(&mut self.data[ci]).insert(at, v);
        }
    }

    pub fn insert_row_named(&mut self, at: usize, row: ValueMap) {
        for k in row.keys() {
            if self.column_index_ci(k).is_none() {
                self.columns.push(k.clone());
                let prev = self.row_count();
                self.data.push(Arc::new(vec![CfmlValue::Null; prev]));
            }
        }
        for ci in 0..self.columns.len() {
            let col_name = self.columns[ci].as_str();
            let val = row
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col_name))
                .map(|(_, v)| v.clone())
                .unwrap_or(CfmlValue::Null);
            Arc::make_mut(&mut self.data[ci]).insert(at, val);
        }
    }

    /// Remove a row and return its synthesised `IndexMap`, or None if oob.
    pub fn remove_row(&mut self, row: usize) -> Option<ValueMap> {
        if row >= self.row_count() {
            return None;
        }
        let m = self.row_at(row);
        for col in &mut self.data {
            Arc::make_mut(col).remove(row);
        }
        m
    }

    pub fn swap_rows(&mut self, r1: usize, r2: usize) {
        for col in &mut self.data {
            Arc::make_mut(col).swap(r1, r2);
        }
    }

    pub fn reverse_rows(&mut self) {
        for col in &mut self.data {
            Arc::make_mut(col).reverse();
        }
    }

    /// Add a column, truncating/padding `values` to `row_count`.
    pub fn add_column(&mut self, name: String, values: Vec<CfmlValue>) {
        let r = self.row_count();
        let mut col = values;
        if col.len() > r {
            // Lucee: adding a column with MORE values than the current row count
            // EXTENDS the query — existing columns get Null-padded up to the new
            // length so recordcount grows to fit the longest column.
            let new_len = col.len();
            for c in self.data.iter_mut() {
                Arc::make_mut(c).resize_with(new_len, || CfmlValue::Null);
            }
        } else if col.len() < r {
            col.resize_with(r, || CfmlValue::Null);
        }
        self.columns.push(name);
        self.data.push(Arc::new(col));
    }

    /// Remove a column by case-insensitive name. Returns true if it existed.
    pub fn remove_column_by_name(&mut self, name: &str) -> bool {
        if let Some(idx) = self.column_index_ci(name) {
            self.columns.remove(idx);
            self.data.remove(idx);
            true
        } else {
            false
        }
    }

    /// Append the rows of `other`, adding any missing columns and filling with
    /// Null where columns don't overlap.
    pub fn append_query(&mut self, other: &CfmlQueryData) {
        for col in &other.columns {
            if self.column_index_ci(col).is_none() {
                self.columns.push(col.clone());
                let r = self.row_count();
                self.data.push(Arc::new(vec![CfmlValue::Null; r]));
            }
        }
        let or = other.row_count();
        for ci in 0..self.columns.len() {
            let col_name = self.columns[ci].as_str();
            match other.column_index_ci(col_name) {
                Some(oci) => {
                    let extra = other.data[oci].iter().cloned();
                    Arc::make_mut(&mut self.data[ci]).extend(extra);
                }
                None => {
                    let new_len = self.data[ci].len() + or;
                    Arc::make_mut(&mut self.data[ci]).resize_with(new_len, || CfmlValue::Null);
                }
            }
        }
    }

    /// Prepend the rows of `other`. Columns merge as with `append_query`.
    pub fn prepend_query(&mut self, other: &CfmlQueryData) {
        for col in &other.columns {
            if self.column_index_ci(col).is_none() {
                self.columns.push(col.clone());
                let r = self.row_count();
                self.data.push(Arc::new(vec![CfmlValue::Null; r]));
            }
        }
        let or = other.row_count();
        for ci in 0..self.columns.len() {
            let col_name = self.columns[ci].as_str();
            let mut prefix: Vec<CfmlValue> = match other.column_index_ci(col_name) {
                Some(oci) => (*other.data[oci]).clone(),
                None => vec![CfmlValue::Null; or],
            };
            let owned = Arc::make_mut(&mut self.data[ci]);
            prefix.append(owned);
            *owned = prefix;
        }
    }
}

/// Shared, interior-mutable backing for a CFML query — the query analogue of
/// [`CfmlArray`]/[`CfmlStruct`], giving queries Lucee/BoxLang-style **reference
/// semantics**. Cloning a `CfmlQuery` bumps the `Arc` (it does NOT copy the
/// rows), so `b = a` makes `a` and `b` two handles onto the *same* data; a
/// mutation through either (e.g. `queryAddRow`) is visible through both, and
/// passing a query to a function lets the callee mutate the caller's query.
/// `duplicate(q)` makes an independent copy (see `CfmlValue::deep_copy`).
///
/// Crucially this also makes `q.addRow(...)` an **O(1)** in-place push instead
/// of the old value-typed clone-the-whole-query-per-row (which made building an
/// N-row query O(n²)).
///
/// All locking lives behind this type's methods so callers (especially
/// `cfml-stdlib`, which doesn't depend on `parking_lot`) never hold a raw guard.
/// Lock discipline (parking_lot is NOT reentrant): a method takes a guard, does
/// one thing, drops it. Never call back into VM/user code while a guard is held.
/// Anything iterate-then-call must `rows()`/`columns()` (snapshot) first.
#[derive(Clone)]
pub struct CfmlQuery(Arc<PlRwLock<CfmlQueryData>>);

impl CfmlQuery {
    /// A query with the given columns and no rows.
    pub fn new(columns: Vec<String>) -> Self {
        let arc = Arc::new(PlRwLock::new(CfmlQueryData::new(columns)));
        crate::cycle_gc::log_query(&arc);
        CfmlQuery(arc)
    }

    /// Wrap an already-built data block (e.g. a QoQ result) into a handle.
    #[inline]
    pub fn from_data(data: CfmlQueryData) -> Self {
        let arc = Arc::new(PlRwLock::new(data));
        crate::cycle_gc::log_query(&arc);
        CfmlQuery(arc)
    }

    /// Build from columns + row-shaped data (sql = None). Rows are unpacked
    /// into column-major storage.
    pub fn from_parts(columns: Vec<String>, rows: Vec<ValueMap>) -> Self {
        CfmlQuery::from_data(CfmlQueryData::from_named_rows(columns, rows))
    }

    /// Build from columns + row-shaped data + originating SQL.
    pub fn from_parts_sql(
        columns: Vec<String>,
        rows: Vec<ValueMap>,
        sql: Option<String>,
    ) -> Self {
        let mut d = CfmlQueryData::from_named_rows(columns, rows);
        d.sql = sql;
        CfmlQuery::from_data(d)
    }

    /// Clone the raw column-major backing arc so QoQ can hold a read guard
    /// across `run_statement` and borrow column slices zero-copy. Internal.
    #[inline]
    pub fn backing(&self) -> Arc<PlRwLock<CfmlQueryData>> {
        Arc::clone(&self.0)
    }

    /// Two handles onto the same backing store (reference identity).
    #[inline]
    pub fn ptr_eq(&self, other: &CfmlQuery) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity of the shared backing store, for cycle detection.
    #[inline]
    pub fn backing_ptr(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }

    /// Snapshot of the column names, in order.
    #[inline]
    pub fn columns(&self) -> Vec<String> {
        self.0.read().columns.clone()
    }

    #[inline]
    pub fn column_count(&self) -> usize {
        self.0.read().column_count()
    }

    #[inline]
    pub fn row_count(&self) -> usize {
        self.0.read().row_count()
    }

    /// 1-based cursor row (the recordset's "current row"). Defaults to 1.
    #[inline]
    pub fn current_row(&self) -> usize {
        self.0.read().current_row()
    }

    /// Move the 1-based cursor row (used by `<cfloop query>`/`<cfoutput query>`).
    /// Shared through the backing Arc, so all handles onto the same recordset —
    /// and any `QueryColumn` proxies created afterwards — observe the new row.
    #[inline]
    pub fn set_current_row(&self, row: usize) {
        self.0.write().current_row = row.max(1);
    }

    /// True when the query has no rows.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.read().is_empty()
    }

    /// Case-insensitive column presence check.
    pub fn has_column_ci(&self, name: &str) -> bool {
        self.0.read().columns.iter().any(|c| c.eq_ignore_ascii_case(name))
    }

    /// Uppercased, comma-joined column list (Lucee/ACF `columnList` convention).
    pub fn column_list(&self) -> String {
        self.0
            .read()
            .columns
            .iter()
            .map(|c| c.to_uppercase())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// A point-in-time snapshot of the rows as `IndexMap`s. Synthesised from
    /// column-major storage on demand.
    #[inline]
    pub fn rows(&self) -> Vec<ValueMap> {
        self.0.read().synthesise_rows()
    }

    /// Snapshot of a single 0-based row, or `None` if out of range.
    pub fn get_row(&self, row0: usize) -> Option<ValueMap> {
        self.0.read().row_at(row0)
    }

    /// All values for a column (case-insensitive), one per row, in row order.
    /// `None` if the column doesn't exist. Used to build a `QueryColumn` proxy.
    /// Returns the column's Arc directly — sharing storage with the underlying
    /// query (zero copy). Mutations through the query will CoW the column.
    pub fn column_values_ci(&self, name: &str) -> Option<Arc<Vec<CfmlValue>>> {
        self.0.read().column_arc_ci(name).cloned()
    }

    /// Append a row in place (interior mutability — visible to all aliases).
    /// This is the **O(1)** push that fixes the old O(n²) query build.
    #[inline]
    pub fn add_row(&self, row: ValueMap) {
        self.0.write().push_row_named(row);
    }

    /// Append a row from positional cell values (fast path — no IndexMap alloc
    /// per row). Extra values are dropped; missing cells are Null.
    #[inline]
    pub fn add_row_positional(&self, vals: Vec<CfmlValue>) {
        self.0.write().push_row_positional(vals);
    }

    /// Set a cell at 0-based `row0` for `column` (in place). Returns false if
    /// the row is out of range.
    pub fn set_cell(&self, row0: usize, column: String, value: CfmlValue) -> bool {
        self.0.write().set_cell_named(row0, &column, value)
    }

    /// Replace an entire column's values by name (case-insensitive), in place on
    /// the shared backing so all aliases observe it. If the column doesn't
    /// exist it is appended. The supplied vec is normalised to the query's
    /// current row count (Null-padded or truncated). Used by indexed query-cell
    /// write-back (`q[col][row] = v`), where the modified (CoW-detached) column
    /// is written back wholesale, and by whole-column assignment (`q.col = arr`).
    pub fn set_column(&self, name: &str, mut values: Vec<CfmlValue>) {
        let mut g = self.0.write();
        let rows = g.row_count();
        if values.len() < rows {
            values.resize(rows, CfmlValue::Null);
        } else if values.len() > rows && rows > 0 {
            values.truncate(rows);
        }
        if let Some(ci) = g.column_index_ci(name) {
            g.data[ci] = Arc::new(values);
        } else {
            g.columns.push(name.to_string());
            g.data.push(Arc::new(values));
        }
    }

    pub fn sql(&self) -> Option<String> {
        self.0.read().sql.clone()
    }

    pub fn set_sql(&self, sql: Option<String>) {
        self.0.write().sql = sql;
    }

    pub fn execution_time(&self) -> Option<i64> {
        self.0.read().execution_time
    }

    pub fn set_execution_time(&self, ms: Option<i64>) {
        self.0.write().execution_time = ms;
    }

    /// Run a closure with shared (read) access to the backing data. MUST NOT
    /// touch this same query again, and MUST NOT call back into VM/user code.
    #[inline]
    pub fn with_read<R>(&self, f: impl FnOnce(&CfmlQueryData) -> R) -> R {
        f(&self.0.read())
    }

    /// Run a closure with exclusive (write) access. Same re-entrancy caveat.
    #[inline]
    pub fn with_write<R>(&self, f: impl FnOnce(&mut CfmlQueryData) -> R) -> R {
        f(&mut self.0.write())
    }
}

/// Debug delegates to the backing data so output matches the pre-handle
/// representation (`CfmlQuery { columns, rows, sql }`).
impl fmt::Debug for CfmlQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.0.read();
        f.debug_struct("CfmlQuery")
            .field("columns", &d.columns)
            .field("rows", &d.synthesise_rows())
            .field("sql", &d.sql)
            .finish()
    }
}

// ─────────────────────────────────────────────
// CfmlValue serde support (for session serialization)
// ─────────────────────────────────────────────

impl serde::Serialize for CfmlValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            CfmlValue::Null => s.serialize_none(),
            CfmlValue::Bool(b) => s.serialize_bool(*b),
            CfmlValue::Int(i) => s.serialize_i64(*i),
            CfmlValue::Double(d) => s.serialize_f64(*d),
            // serializeJSON emits a timespan as its numeric (fractional-day) value.
            CfmlValue::TimeSpan(d) => s.serialize_f64(*d),
            CfmlValue::String(st) => s.serialize_str(st),
            CfmlValue::Array(a) => {
                let snap = a.snapshot();
                let mut seq = s.serialize_seq(Some(snap.len()))?;
                for v in snap.iter() {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            CfmlValue::QueryColumn(a, _) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a.iter() {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            CfmlValue::Struct(m) => {
                let snap = m.snapshot();
                let mut map = s.serialize_map(Some(snap.len()))?;
                for (k, v) in snap.iter() {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            CfmlValue::Binary(b) => {
                let hex: String = b.iter().map(|byte| format!("{:02x}", byte)).collect();
                let mut map = s.serialize_map(Some(2))?;
                map.serialize_entry("_cftype", "binary")?;
                map.serialize_entry("data", &hex)?;
                map.end()
            }
            CfmlValue::Query(q) => {
                let d = q.0.read();
                let mut map = s.serialize_map(Some(3))?;
                map.serialize_entry("_cftype", "query")?;
                map.serialize_entry("columns", &d.columns)?;
                let synth = d.synthesise_rows();
                let rows: Vec<std::collections::HashMap<&str, &CfmlValue>> = synth
                    .iter()
                    .map(|row| row.iter().map(|(k, v)| (k.as_str(), v)).collect())
                    .collect();
                map.serialize_entry("rows", &rows)?;
                map.end()
            }
            CfmlValue::Closure(_) | CfmlValue::Function(_) | CfmlValue::Component(_) | CfmlValue::NativeObject(_) => {
                log::debug!("serializing non-serializable CfmlValue variant as null");
                s.serialize_none()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for CfmlValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(CfmlValueVisitor)
    }
}

struct CfmlValueVisitor;

impl<'de> serde::de::Visitor<'de> for CfmlValueVisitor {
    type Value = CfmlValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a CFML value (null, bool, number, string, array, or object)")
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<CfmlValue, E> {
        Ok(CfmlValue::Null)
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<CfmlValue, E> {
        Ok(CfmlValue::Null)
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<CfmlValue, D::Error> {
        serde::Deserialize::deserialize(d)
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<CfmlValue, E> {
        Ok(CfmlValue::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<CfmlValue, E> {
        Ok(CfmlValue::Int(v))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<CfmlValue, E> {
        Ok(CfmlValue::Int(v as i64))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<CfmlValue, E> {
        if v.fract() == 0.0 && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
            Ok(CfmlValue::Int(v as i64))
        } else {
            Ok(CfmlValue::Double(v))
        }
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<CfmlValue, E> {
        Ok(CfmlValue::String(Arc::new(v.to_string())))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<CfmlValue, E> {
        Ok(CfmlValue::String(Arc::new(v)))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<CfmlValue, A::Error> {
        let mut vec = Vec::new();
        while let Some(v) = a.next_element::<CfmlValue>()? {
            vec.push(v);
        }
        Ok(CfmlValue::array(vec))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<CfmlValue, A::Error> {
        let mut map: ValueMap = ValueMap::default();
        while let Some((k, v)) = a.next_entry::<String, CfmlValue>()? {
            map.insert(k, v);
        }
        // Detect tagged special types
        if let Some(CfmlValue::String(t)) = map.get("_cftype") {
            match t.as_str() {
                "binary" => {
                    if let Some(CfmlValue::String(hex)) = map.get("data") {
                        let bytes: Vec<u8> = (0..hex.len())
                            .step_by(2)
                            .filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                            .collect();
                        return Ok(CfmlValue::Binary(bytes));
                    }
                }
                "query" => {
                    if let Some(CfmlValue::Array(cols)) = map.get("columns") {
                        let columns: Vec<String> =
                            cols.snapshot().iter().map(|v| v.as_string()).collect();
                        let mut rows: Vec<ValueMap> = Vec::new();
                        if let Some(CfmlValue::Array(row_arr)) = map.get("rows") {
                            for row_val in row_arr.snapshot() {
                                if let CfmlValue::Struct(row_map) = row_val {
                                    rows.push(row_map.snapshot());
                                }
                            }
                        }
                        return Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)));
                    }
                }
                _ => {}
            }
        }
        Ok(CfmlValue::strukt(map))
    }
}

#[cfg(test)]
mod size_probe {
    //! PR-0 size probes (RustCFML performance plan). These print the live size
    //! of the core value/runtime types and assert a non-regression *ceiling*.
    //!
    //! Run with: `cargo test -p cfml-common size_probe -- --nocapture`
    //!
    //! When an intentional shrink lands (e.g. boxing `Function`/`Query`,
    //! `String(Arc<str>)`), tighten the ceiling here so the win is recorded and
    //! protected against future regressions.
    use super::*;
    use std::mem::size_of;

    #[test]
    fn report_sizes() {
        let cfml_value = size_of::<CfmlValue>();
        eprintln!("size_of::<CfmlValue>()      = {cfml_value} B");
        eprintln!("size_of::<CfmlFunction>()   = {} B", size_of::<CfmlFunction>());
        eprintln!("size_of::<CfmlQuery>()      = {} B (Arc handle)", size_of::<CfmlQuery>());
        eprintln!("size_of::<CfmlQueryData>()  = {} B", size_of::<CfmlQueryData>());
        eprintln!("size_of::<CfmlComponent>()  = {} B", size_of::<CfmlComponent>());
        eprintln!("size_of::<CfmlClosure>()    = {} B", size_of::<CfmlClosure>());

        // Ceiling, not an exact match: catches accidental growth, tolerates
        // shrinks. Lower this number whenever a planned shrink lands.
        //
        // Baseline as of PR-0 (2026-05-30): 112 B. PR-A (T1.1) boxed the two
        // large variants — `Function(CfmlFunction)` (112 B inline) and
        // `Query(CfmlQuery)` (72 B) — dropping the enum to 32 B, now floored
        // by `String(String)` (24 B) + discriminant. The next planned shrink
        // (interning idents / `String(Arc<str>)`, PR-B) could take it to ~24 B.
        assert!(
            cfml_value <= 32,
            "CfmlValue grew to {cfml_value} B (ceiling 32 B) — a perf regression. \
             If intentional, justify and raise the ceiling."
        );
    }
}
