//! Interned struct/scope keys — the case-insensitive map key used by
//! [`crate::dynamic::ValueMap`].
//!
//! # Why
//!
//! CFML identifiers are case-insensitive but case-*preserving*: `structKeyList`
//! must return the casing the key was first written with, while `s.FOO`,
//! `s.foo` and `s["Foo"]` must all reach the same slot. The engine used to buy
//! that with a case-SENSITIVE `IndexMap<String, _>` plus a side `ci` index
//! mapping each folded key to its stored casing. A case-insensitive lookup
//! therefore cost, in the worst case, an ASCII-lowercase **heap allocation**,
//! a hash of the folded probe, a probe of `ci`, and then a *second* hash and
//! probe of the real map — and, on small structs (where maintaining the index
//! did not pay), a linear `eq_ignore_ascii_case` scan instead.
//!
//! Counted on a warm Preside homepage render, that was ~114,000 keyed lookups
//! per request: 43,428 linear scans costing 184,141 key comparisons, 16,949
//! indexed lookups, and 12,842 fold allocations.
//!
//! # What this does instead
//!
//! [`Key`] follows BoxLang's `Key` (`runtime/scopes/Key.java`): fold and hash
//! **once, at construction**, then carry the hash. Equality and hashing are
//! case-insensitive by construction, so the `ci` side index — and the whole
//! "indexed vs scan" decision with it — disappears, and a map keyed by `Key`
//! is *natively* a CFML struct.
//!
//! Two deliberate departures from BoxLang, both to suit a non-GC runtime:
//!
//! * **No global interner.** BoxLang interns every `Key` in a process-wide
//!   table, which is safe under a GC but would either leak or need a lock here
//!   — and CFML programs build keys from user data (query columns, form
//!   fields, session ids), so the key set is not bounded by the source text.
//!   A `Key` instead owns an `Arc<str>`, so *cloning* is an atomic increment
//!   with no allocation, which is the property the hot paths actually need.
//! * **The original casing is the only string stored.** Equality compares with
//!   `eq_ignore_ascii_case` against the pre-computed hash as a guard, so no
//!   second folded copy is needed.
//!
//! # Probing without a `Key`
//!
//! Call sites that hold only a `&str` use [`KeyRef`], which computes the same
//! hash on the fly (no allocation) and compares equivalent to a `Key`. That
//! path costs roughly what the old code cost, so migration is incremental:
//! nothing regresses, and every call site converted to a pre-built `Key`
//! (literal names built once at codegen, then cloned) drops to a bare probe
//! with no hashing at all.

use indexmap::Equivalent;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::Arc;

/// The Fx multiplier (`rustc_hash`'s, so key hashing keeps the distribution
/// the engine's maps were tuned for).
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
/// `b | 0x20` maps `A..=Z` onto `a..=z` and leaves `a..=z` alone.
const CASE_BIT: u64 = 0x2020_2020_2020_2020;

/// Hash a key case-insensitively, **word at a time and without allocating**.
///
/// The case fold is a single `| 0x20` per 8-byte word rather than a per-byte
/// `to_ascii_lowercase`. That is not a fold to a canonical *string* — it also
/// moves a handful of punctuation bytes (`_` → DEL, `@` → `` ` ``) — but this
/// value is only ever a hash. Two keys that differ only in ASCII case are
/// guaranteed to hash identically, which is the property the map needs;
/// anything that collides is settled by `eq_ignore_ascii_case`, so correctness
/// never depends on the fold being injective.
///
/// The byte-at-a-time version this replaced defeated the word-at-a-time hash
/// and measured **+1.7% on a warm Preside render** — the fold, not the hashing,
/// was the cost.
#[inline]
pub fn fold_hash(key: &str) -> u64 {
    let bytes = key.as_bytes();
    let mut h = bytes.len() as u64;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap()) | CASE_BIT;
        h = (h.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rest.len()].copy_from_slice(rest);
        // Fold only the bytes that are really there; the zero padding must not
        // pick up the case bit, or "ab" and "ab\0" would collide differently
        // depending on length. (Length is already mixed in above.)
        let mask = if rest.len() == 8 { u64::MAX } else { (1u64 << (rest.len() * 8)) - 1 };
        let w = u64::from_le_bytes(tail) | (CASE_BIT & mask);
        h = (h.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
    h
}

/// A case-insensitive, case-preserving struct/scope key carrying its own
/// pre-computed hash.
///
/// Cloning is one atomic increment — no allocation and no re-hash — which is
/// what makes per-frame scope seeding cheap. Construction (`Key::new`) is the
/// only place that allocates or hashes.
#[derive(Clone)]
pub struct Key {
    /// Hash of the ASCII-folded name. See [`fold_hash`].
    hash: u64,
    /// The name in its ORIGINAL casing — what `structKeyList`, `writeDump`,
    /// and serialization must report.
    name: Arc<str>,
}

/// The bounded key interner behind [`Key::new`] / [`Key::from_string`].
///
/// Every map that stores a given name used to hold its own `Arc<str>` of it:
/// 644,325 key insertions across one application's component metadata carried
/// only 800 distinct names, and the per-copy `Arc` allocations showed up as
/// tens of MB of a live Preside heap. Interning hands every constructor of the
/// same spelling the SAME `Arc<str>`, which also lets `Key::eq`'s pointer test
/// answer most comparisons.
///
/// Two properties the module doc's old "no global interner" stance demanded,
/// now provided instead of avoided:
///
/// * **Bounded.** CFML builds keys from user data (query columns, form fields,
///   session ids), so the name set is unbounded. The table stops interning at
///   a cap (`RUSTCFML_KEY_INTERN_CAP`, default 131072 names; `0` disables) and
///   later constructions simply allocate privately, exactly as before — the
///   cap bounds the table, never correctness. First-come wins: an application's
///   code-driven names are constructed early and stay shared.
/// * **Case-exact.** Keys are case-preserving per first write of each MAP, so
///   the interner must never substitute another casing: entries are matched by
///   EXACT bytes. Two casings of one name are two entries (both under the same
///   folded hash bucket).
///
/// 16 shards picked by the folded hash keep the lock uncontended; construction
/// is off the per-op hot path (lookups go through [`KeyRef`] / cloned keys).
mod intern {
    use super::fold_hash;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};

    const SHARDS: usize = 16;
    type Shard = parking_lot::Mutex<std::collections::HashMap<u64, Vec<Arc<str>>>>;

    static TABLE: OnceLock<Vec<Shard>> = OnceLock::new();
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    fn cap() -> usize {
        static CAP: OnceLock<usize> = OnceLock::new();
        *CAP.get_or_init(|| {
            std::env::var("RUSTCFML_KEY_INTERN_CAP")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(131_072)
        })
    }

    /// (names interned, approximate bytes they hold) — for the memory panel.
    pub fn stats() -> (usize, usize) {
        let n = COUNT.load(Ordering::Relaxed);
        let Some(table) = TABLE.get() else { return (0, 0) };
        let mut bytes = 0usize;
        for shard in table {
            let g = shard.lock();
            // Arc<str> header (2 counters) + text, plus the bucket slot.
            bytes += g
                .values()
                .map(|b| b.iter().map(|a| 16 + a.len() + 8).sum::<usize>() + 24)
                .sum::<usize>();
        }
        (n, bytes)
    }

    #[inline]
    pub fn intern(name: &str, hash: u64) -> Arc<str> {
        debug_assert_eq!(hash, fold_hash(name));
        let cap = cap();
        if cap == 0 || name.len() > 128 {
            return Arc::from(name);
        }
        let table = TABLE.get_or_init(|| (0..SHARDS).map(|_| Shard::default()).collect());
        let mut shard = table[(hash as usize) & (SHARDS - 1)].lock();
        let bucket = shard.entry(hash).or_default();
        // EXACT match only — see the module doc. Buckets are almost always one
        // entry (the fold hash already separates names), so this scan is short.
        for a in bucket.iter() {
            if **a == *name {
                return a.clone();
            }
        }
        if COUNT.load(Ordering::Relaxed) >= cap {
            return Arc::from(name);
        }
        let a: Arc<str> = Arc::from(name);
        bucket.push(a.clone());
        COUNT.fetch_add(1, Ordering::Relaxed);
        a
    }
}

/// How many distinct key names the interner holds, and roughly what they cost.
pub fn key_intern_stats() -> (usize, usize) {
    intern::stats()
}

impl Key {
    /// Fold, hash, and intern `name`. The one allocating entry point; prefer
    /// cloning an existing `Key` on hot paths. Every constructor of the same
    /// spelling (case-exact) shares one `Arc<str>` — see [`intern`].
    #[inline]
    pub fn new(name: impl AsRef<str>) -> Self {
        let name = name.as_ref();
        let hash = fold_hash(name);
        Key { hash, name: intern::intern(name, hash) }
    }

    /// Build from an already-owned `String`. The text is interned like
    /// [`Key::new`]'s, so the owned buffer is only kept when it is the first
    /// sighting of this spelling.
    #[inline]
    pub fn from_string(name: String) -> Self {
        let hash = fold_hash(&name);
        Key { hash, name: intern::intern(&name, hash) }
    }

    /// The key in its original casing.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// The name as a shared string (an `Arc` clone, no allocation).
    #[inline]
    pub fn as_arc(&self) -> Arc<str> {
        self.name.clone()
    }

    /// The pre-computed folded hash, for callers building their own indexes
    /// keyed the same way.
    #[inline]
    pub fn hash_value(&self) -> u64 {
        self.hash
    }

    /// A borrowing probe for this key (mostly useful in generic code).
    #[inline]
    pub fn as_ref(&self) -> KeyRef<'_> {
        KeyRef { hash: self.hash, name: &self.name }
    }

    /// True when the key is stored in the exact casing given — the check CFML
    /// needs when reporting keys, never for lookup.
    #[inline]
    pub fn is_exactly(&self, other: &str) -> bool {
        &*self.name == other
    }
}

impl PartialEq for Key {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Hash first: a u64 compare rejects nearly every non-match, and two
        // clones of the same key share an allocation so hit the pointer test.
        self.hash == other.hash
            && (Arc::ptr_eq(&self.name, &other.name)
                || self.name.eq_ignore_ascii_case(&other.name))
    }
}

impl Eq for Key {}

impl Hash for Key {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

/// Ordering folds case, so it agrees with [`Key`]'s case-insensitive `Eq`
/// (two keys that are `==` always compare `Equal`) — which is what `BTreeMap`,
/// `sort`/`dedup`, and binary search require to behave.
impl Ord for Key {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let (a, b) = (self.name.as_bytes(), other.name.as_bytes());
        for i in 0..a.len().min(b.len()) {
            let (x, y) = (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase());
            if x != y {
                return x.cmp(&y);
            }
        }
        a.len().cmp(&b.len())
    }
}

impl PartialOrd for Key {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Comparisons against plain strings — `key == "__variables"` and friends —
// fold case, like every other comparison of a CFML identifier in the engine.
impl PartialEq<str> for Key {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.name.eq_ignore_ascii_case(other)
    }
}

impl PartialEq<&str> for Key {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.name.eq_ignore_ascii_case(other)
    }
}

impl PartialEq<String> for Key {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.name.eq_ignore_ascii_case(other)
    }
}

impl PartialEq<Key> for str {
    #[inline]
    fn eq(&self, other: &Key) -> bool {
        other.name.eq_ignore_ascii_case(self)
    }
}

impl PartialEq<Key> for String {
    #[inline]
    fn eq(&self, other: &Key) -> bool {
        other.name.eq_ignore_ascii_case(self)
    }
}

impl std::ops::Deref for Key {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        &self.name
    }
}

impl AsRef<str> for Key {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.name
    }
}

impl std::borrow::Borrow<str> for Key {
    /// Only for `&str`-keyed *side* structures that a caller builds from
    /// `Key`s; it is NOT used for `ValueMap` lookups (which go through
    /// [`KeyRef`]), because `str`'s own `Hash` is case-sensitive and would not
    /// agree with [`Key::hash`].
    #[inline]
    fn borrow(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.name, f)
    }
}

impl From<&str> for Key {
    #[inline]
    fn from(s: &str) -> Self {
        Key::new(s)
    }
}

impl From<String> for Key {
    #[inline]
    fn from(s: String) -> Self {
        Key::from_string(s)
    }
}

impl From<&String> for Key {
    #[inline]
    fn from(s: &String) -> Self {
        Key::new(s.as_str())
    }
}

impl From<Key> for String {
    #[inline]
    fn from(k: Key) -> String {
        k.name.to_string()
    }
}

impl serde::Serialize for Key {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.name)
    }
}

impl<'de> serde::Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Key::from_string(String::deserialize(d)?))
    }
}

/// A borrowed probe equivalent to a [`Key`], for looking up by `&str` without
/// building (and allocating) a `Key`.
///
/// Costs one [`fold_hash`] per lookup — about what the old folded-probe path
/// cost — so it is the compatibility path, not the fast path. Prefer a cloned
/// `Key` where one is already to hand.
#[derive(Clone, Copy)]
pub struct KeyRef<'a> {
    hash: u64,
    name: &'a str,
}

impl<'a> KeyRef<'a> {
    #[inline]
    pub fn new(name: &'a str) -> Self {
        KeyRef { hash: fold_hash(name), name }
    }

    #[inline]
    pub fn as_str(&self) -> &'a str {
        self.name
    }

    /// Promote to an owned [`Key`], reusing the already-computed hash.
    #[inline]
    pub fn to_key(&self) -> Key {
        Key { hash: self.hash, name: Arc::from(self.name) }
    }
}

/// Shorthand for [`KeyRef::new`] — `map.get(k("foo"))`.
#[inline]
pub fn k(name: &str) -> KeyRef<'_> {
    KeyRef::new(name)
}

impl Hash for KeyRef<'_> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

impl PartialEq for KeyRef<'_> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.name.eq_ignore_ascii_case(other.name)
    }
}

impl Eq for KeyRef<'_> {}

/// Lets `IndexMap<Key, _>::get` accept a [`KeyRef`]. `indexmap` requires the
/// probe's `Hash` to agree with the stored key's — both write the same folded
/// `u64`, so it does.
impl Equivalent<Key> for KeyRef<'_> {
    #[inline]
    fn equivalent(&self, key: &Key) -> bool {
        self.hash == key.hash && self.name.eq_ignore_ascii_case(&key.name)
    }
}

/// Lets a `Key` probe a `KeyRef`-keyed map (rare; kept for symmetry).
impl Equivalent<KeyRef<'_>> for Key {
    #[inline]
    fn equivalent(&self, key: &KeyRef<'_>) -> bool {
        self.hash == key.hash && self.name.eq_ignore_ascii_case(key.name)
    }
}

/// Pass-through hasher for [`Key`]-keyed maps.
///
/// A `Key` already carries a well-mixed hash, so re-hashing it is pure waste —
/// this hasher just forwards the `u64`. **It is only valid for keys that hash
/// via `write_u64`**; anything that hashes as bytes (a bare `&str`, an integer
/// written with `write_u32`, …) is rejected in debug builds rather than
/// silently producing a hash that would never match.
#[derive(Default, Clone, Copy)]
pub struct IdentityHasher {
    hash: u64,
}

impl Hasher for IdentityHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.hash = n;
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Reaching here means something is being used as a Key-map key without
        // going through Key/KeyRef. Fold it so behaviour stays deterministic,
        // but fail loudly in debug: a silent mismatch here is a lookup that
        // misses forever.
        debug_assert!(false, "IdentityHasher used with a non-Key key type");
        for &b in bytes {
            self.hash = self.hash.rotate_left(8) ^ u64::from(b);
        }
    }
}

/// `BuildHasher` for [`Key`]-keyed maps. See [`IdentityHasher`].
pub type KeyBuildHasher = BuildHasherDefault<IdentityHasher>;

use std::fmt;

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    type M = IndexMap<Key, i32, KeyBuildHasher>;

    #[test]
    fn case_insensitive_equality_and_hash() {
        let a = Key::new("myVar");
        let b = Key::new("MYVAR");
        assert_eq!(a, b);
        assert_eq!(a.hash_value(), b.hash_value());
        // …but casing is preserved.
        assert_eq!(a.as_str(), "myVar");
        assert_eq!(b.as_str(), "MYVAR");
        assert!(a.is_exactly("myVar"));
        assert!(!a.is_exactly("MYVAR"));
    }

    #[test]
    fn distinct_names_differ() {
        assert_ne!(Key::new("foo"), Key::new("bar"));
        assert_ne!(Key::new("foo"), Key::new("foo2"));
        assert_ne!(Key::new(""), Key::new("a"));
    }

    #[test]
    fn map_lookup_is_case_insensitive_from_every_probe_form() {
        let mut m = M::default();
        m.insert(Key::new("FirstName"), 1);
        for probe in ["FirstName", "firstname", "FIRSTNAME", "fIrStNaMe"] {
            assert_eq!(m.get(&k(probe)), Some(&1), "probe {probe}");
            assert_eq!(m.get(&Key::new(probe)), Some(&1), "owned {probe}");
        }
        assert_eq!(m.get(&k("firstnam")), None);
    }

    #[test]
    fn insert_keeps_first_written_casing_and_replaces_value() {
        // CFML semantics: `s.Foo = 1; s.FOO = 2` leaves one key, cased `Foo`.
        let mut m = M::default();
        m.insert(Key::new("Foo"), 1);
        m.insert(Key::new("FOO"), 2);
        assert_eq!(m.len(), 1);
        assert_eq!(m.keys().next().unwrap().as_str(), "Foo");
        assert_eq!(m[&k("foo")], 2);
    }

    #[test]
    fn non_ascii_is_passed_through_not_case_folded() {
        // CFML case-insensitivity is ASCII-only; Turkish İ must not fold to i.
        let a = Key::new("İstanbul");
        let b = Key::new("istanbul");
        assert_ne!(a, b);
        // ASCII portions of a non-ASCII key still fold.
        assert_eq!(Key::new("café_X"), Key::new("café_x"));
    }

    #[test]
    fn long_keys_beyond_the_fold_chunk_still_match() {
        let long_lower = "k".repeat(199);
        let long_upper = long_lower.to_ascii_uppercase();
        assert_eq!(Key::new(&long_lower), Key::new(&long_upper));
        let mut m = M::default();
        m.insert(Key::new(&long_upper), 9);
        assert_eq!(m.get(&k(&long_lower)), Some(&9));
    }

    #[test]
    fn keyref_promotes_without_rehashing() {
        let r = k("SomeKey");
        let owned = r.to_key();
        assert_eq!(owned.hash_value(), fold_hash("somekey"));
        assert_eq!(owned, Key::new("SOMEKEY"));
    }

    #[test]
    fn clone_shares_the_allocation() {
        let a = Key::new("shared");
        let b = a.clone();
        assert!(Arc::ptr_eq(&a.name, &b.name));
    }

    #[test]
    fn hash_agrees_between_key_and_keyref() {
        use std::hash::BuildHasher;
        let bh = KeyBuildHasher::default();
        for name in ["x", "Some_Long_Key_Name", "MiXeD", ""] {
            assert_eq!(bh.hash_one(Key::new(name)), bh.hash_one(k(name)));
            assert_eq!(bh.hash_one(k(name)), bh.hash_one(k(&name.to_uppercase())));
        }
    }
}

/// Pre-built [`Key`]s for the engine's own reserved names.
///
/// These are the CFML equivalent of BoxLang's compile-time `Key` constants:
/// the VM probes `__variables` / `this` / the arguments scope on essentially
/// every frame, and those probes were re-hashing a string literal each time.
/// A `LazyLock` deref is an acquire load and a branch — no fold, no hash, no
/// allocation.
///
/// Add a constant here whenever a literal key shows up in the `probe-sites`
/// census; that census is what this list is derived from.
pub mod well_known {
    use super::Key;
    use std::sync::LazyLock;

    /// A CFC's private `variables` scope, as stored inside a frame's locals.
    pub static VARIABLES: LazyLock<Key> = LazyLock::new(|| Key::new("__variables"));
    /// The public scope / current instance.
    pub static THIS: LazyLock<Key> = LazyLock::new(|| Key::new("this"));
    /// The parent-component handle.
    pub static SUPER: LazyLock<Key> = LazyLock::new(|| Key::new("super"));
    /// A CFC method frame's shared per-class `static` scope.
    pub static STATIC: LazyLock<Key> = LazyLock::new(|| Key::new("__static"));
    /// The reserved slot holding a frame's `arguments` scope (the literal
    /// spelling a user variable could never collide with).
    pub static ARGUMENTS_SCOPE: LazyLock<Key> = LazyLock::new(|| Key::new("__arguments__"));
    /// The user-facing `arguments` name.
    pub static ARGUMENTS: LazyLock<Key> = LazyLock::new(|| Key::new("arguments"));
    /// Page/CFC `variables` scope under its user-facing name.
    pub static VARIABLES_SCOPE: LazyLock<Key> = LazyLock::new(|| Key::new("variables"));
    /// Frame-local scope.
    pub static LOCAL: LazyLock<Key> = LazyLock::new(|| Key::new("local"));
    /// A CFC's native (Rust) parent instance.
    pub static SUPER_NATIVE: LazyLock<Key> = LazyLock::new(|| Key::new("__super"));
    /// A CFC's declared `property` list.
    pub static PROPERTIES: LazyLock<Key> = LazyLock::new(|| Key::new("__properties"));
    /// A component's class name marker.
    pub static NAME_MARKER: LazyLock<Key> = LazyLock::new(|| Key::new("__name"));
    /// The .cfc a component instance was built from.
    pub static SOURCE_FILE: LazyLock<Key> = LazyLock::new(|| Key::new("__source_file"));
    /// A component instance's stable identity (GH #260).
    pub static INSTANCE_ID: LazyLock<Key> = LazyLock::new(|| Key::new("__instance_id"));
    /// Marks the struct standing in for a `super` reference.
    pub static IS_SUPER: LazyLock<Key> = LazyLock::new(|| Key::new("__is_super"));
    /// The env a LEXICAL closure frame resolves its captured names through —
    /// a `Function` carrier holding the `Arc<RwLock<ValueMap>>`, see
    /// `CfmlVirtualMachine::FRAME_CLOSURE_ENV_KEY`.
    pub static CLOSURE_FRAME_ENV: LazyLock<Key> =
        LazyLock::new(|| Key::new("__closure_frame_env__"));
    /// A closure env's link to its lexical parent env (`CfmlVirtualMachine::CLOSURE_PARENT_KEY`).
    pub static CLOSURE_PARENT: LazyLock<Key> =
        LazyLock::new(|| Key::new("__closure_parent_env__"));
    /// A closure env's own-key list (`CfmlVirtualMachine::CLOSURE_OWN_KEYS`).
    pub static CLOSURE_OWN_KEYS: LazyLock<Key> =
        LazyLock::new(|| Key::new("__closure_own_keys__"));
    /// Sentinel marking a struct AS the `arguments` scope.
    pub static ARGUMENTS_MARKER: LazyLock<Key> = LazyLock::new(|| Key::new("__arguments_scope"));
    /// Declared parameter names carried on the `arguments` scope, so
    /// `arguments[N]` can fall through to the positional param.
    pub static ARGUMENTS_PARAMS: LazyLock<Key> = LazyLock::new(|| Key::new("__arguments_params"));
}

#[cfg(test)]
mod intern_tests {
    //! The interner must share one Arc per exact spelling, keep distinct
    //! casings distinct (per-map case preservation), and never affect
    //! equality semantics.
    use super::*;

    #[test]
    fn same_spelling_shares_one_allocation() {
        let a = Key::new("internProbeAlpha");
        let b = Key::from_string("internProbeAlpha".to_string());
        assert!(Arc::ptr_eq(&a.as_arc(), &b.as_arc()), "same spelling must share the Arc");
    }

    #[test]
    fn different_casings_keep_their_own_text_but_compare_equal() {
        let a = Key::new("internProbeBeta");
        let b = Key::new("INTERNPROBEBETA");
        assert_eq!(a, b, "case-insensitive equality unchanged");
        assert_eq!(a.as_str(), "internProbeBeta");
        assert_eq!(b.as_str(), "INTERNPROBEBETA");
        assert!(!Arc::ptr_eq(&a.as_arc(), &b.as_arc()), "casings must not be merged");
    }

    #[test]
    fn stats_reports_interned_names() {
        let _ = Key::new("internProbeGamma");
        let (n, bytes) = key_intern_stats();
        assert!(n >= 1 && bytes > 0);
    }
}
