//! Interned identifier names for bytecode operands (perf plan Phase 3.1).
//!
//! Every name operand in `BytecodeOp` used to be an owned `String`: each op
//! carried its own 24-byte String, every VM-side `name.clone()` was a heap
//! copy, and hundreds of dispatch-loop sites re-derived case information
//! (`to_lowercase()` / `eq_ignore_ascii_case`) from the raw bytes on every
//! execution. A [`Name`] is an `Arc` to an interned record that carries the
//! original spelling AND its precomputed lowercase form, so:
//!
//! - cloning is a refcount bump,
//! - `lower()` is a free borrow (no per-dispatch case folding),
//! - the op operand shrinks from 24 B to 8 B (better icache density),
//! - identical spellings share one allocation process-wide.
//!
//! CFML identifiers are case-insensitive but case-preserving, which is why
//! both spellings are kept: `orig` surfaces in error messages and metadata,
//! `lower` feeds scope probes and builtin lookups.
//!
//! Interning is keyed by the ORIGINAL spelling (case-sensitive): `Foo` and
//! `foo` intern to two distinct `Name`s that each know the same lowercase
//! form. The interner only ever grows, bounded by the number of distinct
//! identifier spellings across all compiled code — the same strings the old
//! representation kept one owned copy of *per op*.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
struct NameInner {
    /// The map key for this identifier, built ONCE here at intern time — i.e.
    /// at compile time, since `Name`s are created by codegen. This is the
    /// BoxLang trick (`Key` emitted as a compile-time constant): a scope or
    /// struct lookup through a `Name` needs no fold and no hash, and seeding a
    /// frame with one allocates nothing (cloning a `Key` is a refcount bump).
    key: crate::key::Key,
    orig: Box<str>,
    /// `None` when `orig` is already all-lowercase (the common case for CFML
    /// code in practice) — `lower()` then borrows `orig` directly.
    lower: Option<Box<str>>,
    /// The identifier names a scope, an engine-managed frame key, or is an
    /// engine `__` key — anything the frame ops route somewhere other than
    /// the plain variable store. Computed once here so a hot store/load can
    /// skip the whole chain of scope-name comparisons with one bool test.
    reserved: bool,
    /// The identifier is a compiled-in builtin function name (`BUILTIN_NAMES`),
    /// answered at intern time so the bare-call resolver need not hash the
    /// lowercase spelling into the builtin index on every call.
    builtin: bool,
    /// The compiled-in builtin this name dispatches to, resolved by the VM on
    /// the first call and reused by every later call of the same spelling —
    /// the per-call hash of the name into the builtin index was ~10% of a
    /// small string BIF. Only ever set for a `builtin` name whose VM has no
    /// native override for it (the VM checks that before consulting this).
    builtin_fn: std::sync::OnceLock<BuiltinFnPtr>,
}

/// A compiled-in builtin's entry point, as cached on a [`Name`].
pub type BuiltinFnPtr = fn(Vec<crate::dynamic::CfmlValue>) -> crate::vm::CfmlResult;

/// An interned, case-aware identifier. Cheap to clone (`Arc`), derefs to the
/// original spelling, and exposes the precomputed lowercase via [`Name::lower`].
#[derive(Clone)]
pub struct Name(Arc<NameInner>);

static INTERNER: RwLock<Option<HashMap<Box<str>, Name>>> = RwLock::new(None);

/// Identifiers a frame's load/store ops give special routing (scope names,
/// the reserved `arguments`/`cfcatch` frame keys, custom-tag bridge keys). See
/// [`Name::is_reserved_word`].
const RESERVED_WORDS: &[&str] = &[
    "local", "variables", "arguments", "this", "super", "static", "request",
    "application", "session", "server", "client", "thread", "url", "form", "cgi",
    "cookie", "cfcatch", "cfthread", "attributes", "caller", "thistag",
];

impl Name {
    /// Intern `s`, returning the shared `Name` for this exact spelling.
    pub fn intern(s: &str) -> Name {
        if let Ok(guard) = INTERNER.read() {
            if let Some(hit) = guard.as_ref().and_then(|m| m.get(s)) {
                return hit.clone();
            }
        }
        let mut guard = match INTERNER.write() {
            Ok(g) => g,
            // Poisoned (a panic mid-insert): degrade to an unshared Name —
            // everything still works, we just lose dedup for this call.
            Err(_) => return Self::new_unshared(s),
        };
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some(hit) = map.get(s) {
            return hit.clone();
        }
        let name = Self::new_unshared(s);
        map.insert(Box::from(s), name.clone());
        name
    }

    /// Number of distinct spellings currently interned. Diagnostic only
    /// (`RUSTCFML_CACHE_CENSUS`): the interner is process-lifetime and never
    /// pruned, so a count that climbs across identical workloads means something
    /// is interning DYNAMIC strings, which this table can never give back.
    pub fn interned_count() -> usize {
        INTERNER
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|m| m.len()))
            .unwrap_or(0)
    }

    fn new_unshared(s: &str) -> Name {
        let lower = if s.bytes().any(|b| b.is_ascii_uppercase()) || s.chars().any(|c| !c.is_ascii())
        {
            let folded = s.to_lowercase();
            // Non-ASCII spellings can lowercase to themselves; only store a
            // second copy when folding actually changed something.
            if folded == s {
                None
            } else {
                Some(folded.into_boxed_str())
            }
        } else {
            None
        };
        let reserved = s.starts_with("__")
            || RESERVED_WORDS.iter().any(|w| s.eq_ignore_ascii_case(w));
        let builtin = {
            let l: &str = lower.as_deref().unwrap_or(s);
            crate::builtins_meta::BUILTIN_NAMES.binary_search(&l).is_ok()
        };
        Name(Arc::new(NameInner {
            key: crate::key::Key::new(s),
            orig: Box::from(s),
            lower,
            reserved,
            builtin,
            builtin_fn: std::sync::OnceLock::new(),
        }))
    }

    /// The original spelling.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0.orig
    }

    /// The precomputed lowercase spelling (== `as_str()` when the original is
    /// already lowercase). This is the whole point of the type: hot VM paths
    /// read this instead of calling `to_lowercase()` per dispatch.
    #[inline]
    pub fn lower(&self) -> &str {
        match &self.0.lower {
            Some(l) => l,
            None => &self.0.orig,
        }
    }

    /// Was the original spelling already all-lowercase? (Free — the answer was
    /// computed at intern time.) Mirrors the ASCII-uppercase probe several VM
    /// ops used to run per dispatch.
    #[inline]
    pub fn is_lowercase(&self) -> bool {
        self.0.lower.is_none()
    }

    /// Case-insensitive equality with an arbitrary string.
    #[inline]
    pub fn eq_ci(&self, other: &str) -> bool {
        self.lower().eq_ignore_ascii_case(other)
    }

    /// Does this identifier need special routing in a frame's load/store ops
    /// (a scope name, a reserved frame key, or an engine `__` key)? Free —
    /// computed at intern time. A `false` answer means the name is a plain
    /// variable the fast paths may store straight into the frame's scope.
    #[inline]
    pub fn is_reserved_word(&self) -> bool {
        self.0.reserved
    }

    /// Is this a compiled-in builtin function name? Free — computed at intern
    /// time against `BUILTIN_NAMES`. Extension-provided and natively
    /// registered builtins are NOT covered; callers that must see those fall
    /// back to the VM's dynamic index when this answers `false`.
    #[inline]
    pub fn is_builtin_static(&self) -> bool {
        self.0.builtin
    }

    /// The cached builtin entry point for this name (see `NameInner::builtin_fn`).
    #[inline]
    pub fn builtin_fn_cache(&self) -> &std::sync::OnceLock<BuiltinFnPtr> {
        &self.0.builtin_fn
    }

    /// This identifier as a struct/scope [`Key`] — free, precomputed at intern
    /// time. Use for INSERTS (`scope.insert(name.key(), v)`): it clones an
    /// existing `Key`, so no string is allocated and nothing is hashed.
    #[inline]
    pub fn key(&self) -> &crate::key::Key {
        &self.0.key
    }
}

/// Probe a [`ValueMap`](crate::dynamic::ValueMap) with a `Name` directly —
/// the hash comes from the interned `Key`, so the lookup does no hashing at
/// all. This is the fast path the whole `Key` migration exists to enable.
impl crate::dynamic::ProbeKey for Name {
    #[inline]
    fn probe(&self) -> crate::key::KeyRef<'_> {
        #[cfg(feature = "probe-sites")]
        crate::perf_counters::bump(&crate::perf_counters::PROBE_PRECOMPUTED);
        self.0.key.as_ref()
    }
}

impl crate::dynamic::IntoKey for &Name {
    #[inline]
    fn into_key(self) -> crate::key::Key {
        self.0.key.clone()
    }
}

impl Deref for Name {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        &self.0.orig
    }
}

impl AsRef<str> for Name {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0.orig
    }
}

impl Borrow<str> for Name {
    #[inline]
    fn borrow(&self) -> &str {
        &self.0.orig
    }
}

impl PartialEq for Name {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.orig == other.0.orig
    }
}
impl Eq for Name {}

impl std::hash::Hash for Name {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash as the underlying str so `Borrow<str>` map lookups agree.
        self.0.orig.hash(state)
    }
}

impl PartialEq<str> for Name {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &*self.0.orig == other
    }
}
impl PartialEq<&str> for Name {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        &*self.0.orig == *other
    }
}
impl PartialEq<String> for Name {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        &*self.0.orig == other.as_str()
    }
}
impl PartialEq<Name> for str {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        self == &*other.0.orig
    }
}
impl PartialEq<Name> for &str {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        *self == &*other.0.orig
    }
}
impl PartialEq<Name> for String {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        self.as_str() == &*other.0.orig
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.orig)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0.orig, f)
    }
}

impl From<&str> for Name {
    #[inline]
    fn from(s: &str) -> Name {
        Name::intern(s)
    }
}
impl From<&String> for Name {
    #[inline]
    fn from(s: &String) -> Name {
        Name::intern(s)
    }
}
impl From<String> for Name {
    #[inline]
    fn from(s: String) -> Name {
        Name::intern(&s)
    }
}
// Mechanically-rewritten emit sites produce `Name::from(&x)` where `x` is
// already a `&String`/`&str`; accept the double reference rather than making
// hundreds of call sites re-derive the right number of `&`s.
impl From<&&String> for Name {
    #[inline]
    fn from(s: &&String) -> Name {
        Name::intern(s)
    }
}
impl From<&&str> for Name {
    #[inline]
    fn from(s: &&str) -> Name {
        Name::intern(s)
    }
}
impl From<&Name> for String {
    #[inline]
    fn from(n: &Name) -> String {
        n.as_str().to_string()
    }
}
impl From<Name> for String {
    #[inline]
    fn from(n: Name) -> String {
        n.as_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_dedupes_exact_spelling() {
        let a = Name::intern("myVar");
        let b = Name::intern("myVar");
        assert!(Arc::ptr_eq(&a.0, &b.0));
        // Different casing = different Name, same lowercase.
        let c = Name::intern("MYVAR");
        assert!(!Arc::ptr_eq(&a.0, &c.0));
        assert_eq!(a.lower(), c.lower());
    }

    #[test]
    fn lower_is_precomputed_and_borrowed_when_already_lower() {
        let lc = Name::intern("already_lower");
        assert!(lc.is_lowercase());
        assert!(std::ptr::eq(lc.lower().as_ptr(), lc.as_str().as_ptr()));
        let mixed = Name::intern("CamelCase");
        assert!(!mixed.is_lowercase());
        assert_eq!(mixed.lower(), "camelcase");
        assert_eq!(mixed.as_str(), "CamelCase");
    }

    #[test]
    fn str_interop() {
        let n = Name::intern("Foo");
        assert_eq!(n, "Foo");
        assert_eq!("Foo", n);
        assert!(n.eq_ci("FOO"));
        assert_eq!(n.len(), 3); // Deref<str>
        let mut map: HashMap<Name, i32> = HashMap::new();
        map.insert(n.clone(), 1);
        assert_eq!(map.get("Foo"), Some(&1)); // Borrow<str>
    }

    #[test]
    fn unicode_folding() {
        let n = Name::intern("ÜBER");
        assert_eq!(n.lower(), "über");
        // Non-ASCII already-lowercase folds to itself → borrowed, no copy.
        let l = Name::intern("über");
        assert!(l.is_lowercase() || l.lower() == "über");
    }
}
