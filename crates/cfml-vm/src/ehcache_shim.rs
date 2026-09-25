//! EhCache 3, as the `cbehcache` CacheBox provider drives it through
//! `org.pixl8.cbehcache.CbEhCacheService` — implemented natively.
//!
//! `preside-ext-cluster-helpers` points EVERY CacheBox cache at cbehcache, so a
//! Preside site that installs it cannot boot without this class. The provider
//! CFC runs unchanged; these are the Java objects it talks to:
//!
//! | class | methods used |
//! |---|---|
//! | `CbEhCacheService` | `init(storageDir)`, `init()`, `close()`, `getStatus()`, `createCache(name, config)`, `getStats(name)` |
//! | `org.ehcache.Cache` | `get`, `put`, `containsKey`, `remove`, `clear`, `iterator()` |
//! | cache statistics | `getCacheHits/Misses/Evictions()`, `getCacheHitPercentage()`, `getKnownStatistics()`, `clear()` |
//!
//! Semantics follow the jar's `CbEhCacheService` (read from its bytecode):
//! - Expiry, in MINUTES: none when `objectDefaultTimeout + objectDefaultLastAccessTimeout <= 0`;
//!   with `useLastAccessTimeouts` time-to-idle (the last-access timeout, else the
//!   object timeout); otherwise time-to-live of `objectDefaultTimeout`.
//! - `storage = "heap"`: `maxObjects > 0` caps the entry count, else `maxSizeInMb`
//!   caps the (approximate) size. Values are held BY REFERENCE, like EhCache's
//!   on-heap store.
//! - `"offheap"` / `"disk"`: `maxSizeInMb` caps the bytes; values are stored
//!   SERIALISED (copy semantics), so only data values are accepted. `"disk"`
//!   writes one file per entry under the manager's directory; with
//!   `persistent = true` the entries survive a restart.
//! - `valueClass` `struct` / `array` / `query` / `java.lang.String` is enforced
//!   on `put`, as EhCache's typed cache does; `java.lang.Object` accepts anything.
//! - Eviction is least-recently-used.

use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const SERVICE_CLASS: &str = "org.pixl8.cbehcache.cbehcacheservice";
const CACHE_CLASS: &str = "org.ehcache.cache";
const STATS_CLASS: &str = "org.ehcache.core.statistics.cachestatistics";
const STAT_CLASS: &str = "org.ehcache.core.statistics.tierstatistics.statistic";
const ITERATOR_CLASS: &str = "org.ehcache.cache.iterator";
const ENTRY_CLASS: &str = "org.ehcache.cache.entry";

pub(crate) fn constructs(class_lower: &str) -> bool {
    class_lower == SERVICE_CLASS
}

pub(crate) fn handles(class_lower: &str) -> bool {
    matches!(
        class_lower,
        SERVICE_CLASS | CACHE_CLASS | STATS_CLASS | STAT_CLASS | ITERATOR_CLASS | ENTRY_CLASS
    )
}

// ── model ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Expiry {
    Never,
    Ttl(u64),
    Tti(u64),
}

#[derive(Clone, Copy, PartialEq)]
enum ValueClass {
    Any,
    Struct,
    Array,
    Query,
    Str,
}

enum Tier {
    Heap { max_entries: u64, max_bytes: u64 },
    OffHeap { max_bytes: u64 },
    Disk { max_bytes: u64, dir: PathBuf, persistent: bool },
}

impl Tier {
    fn mapping_stat(&self) -> &'static str {
        match self {
            Tier::Heap { .. } => "OnHeap:MappingCount",
            Tier::OffHeap { .. } => "OffHeap:MappingCount",
            Tier::Disk { .. } => "Disk:MappingCount",
        }
    }
}

enum Stored {
    Live(CfmlValue),
    Bytes(Vec<u8>),
    File(PathBuf),
}

struct Entry {
    stored: Stored,
    size: u64,
    created_ms: u64,
    accessed_ms: u64,
}

struct Cache {
    tier: Tier,
    expiry: Expiry,
    value_class: ValueClass,
    /// Insertion order is recency order: an access moves the entry to the end,
    /// so eviction takes from the front (least recently used).
    entries: Mutex<indexmap::IndexMap<String, Entry>>,
    bytes: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    closed: AtomicBool,
}

struct Manager {
    dir: PathBuf,
    available: AtomicBool,
    caches: Mutex<HashMap<String, Arc<Cache>>>,
}

fn managers() -> &'static Mutex<HashMap<u64, Arc<Manager>>> {
    static M: OnceLock<Mutex<HashMap<u64, Arc<Manager>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn err(msg: impl Into<String>) -> CfmlError {
    CfmlError::runtime(msg.into())
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn int_field(object: &CfmlValue, key: &str) -> Option<u64> {
    match field(object, key)? {
        CfmlValue::Int(i) => Some(i as u64),
        _ => None,
    }
}

fn manager_of(object: &CfmlValue) -> Result<Arc<Manager>, CfmlError> {
    let id = int_field(object, "__ehc_mgr").ok_or_else(|| err("CbEhCacheService used before init()"))?;
    managers()
        .lock()
        .ok()
        .and_then(|m| m.get(&id).cloned())
        .ok_or_else(|| err("CbEhCacheService: this manager no longer exists"))
}

fn cache_of(object: &CfmlValue) -> Result<Arc<Cache>, CfmlError> {
    let mgr = manager_of(object)?;
    let name = field(object, "__ehc_cache").map(|v| v.as_string()).unwrap_or_default();
    let cache = mgr
        .caches
        .lock()
        .ok()
        .and_then(|c| c.get(&name).cloned())
        .ok_or_else(|| err(format!("State is UNINITIALIZED: cache '{}' is closed", name)))?;
    if cache.closed.load(Ordering::Relaxed) {
        return Err(err(format!("State is UNINITIALIZED: cache '{}' is closed", name)));
    }
    Ok(cache)
}

/// Values the serialising tiers can hold: data only. A component, closure or
/// native object would round-trip as null — refuse it instead.
fn check_serialisable(v: &CfmlValue) -> Result<(), CfmlError> {
    #[cfg(feature = "component-instance")]
    if matches!(v, CfmlValue::Instance(_)) {
        return Err(err("the value is a component; a serialising cache tier holds data values only"));
    }
    crate::validate_session_value("value", v).map_err(|p| {
        err(format!("{}; a serialising cache tier (offheap/disk) holds data values only", p))
    })
}

fn class_ok(vc: ValueClass, v: &CfmlValue) -> bool {
    match vc {
        ValueClass::Any => true,
        ValueClass::Struct => matches!(v, CfmlValue::Struct(s) if !cfml_common::component::is_component_backing(s)),
        ValueClass::Array => matches!(v, CfmlValue::Array(_)),
        ValueClass::Query => matches!(v, CfmlValue::Query(_)),
        ValueClass::Str => matches!(v, CfmlValue::String(_)),
    }
}

fn value_class_name(vc: ValueClass) -> &'static str {
    match vc {
        ValueClass::Any => "java.lang.Object",
        ValueClass::Struct => "lucee.runtime.type.Struct",
        ValueClass::Array => "lucee.runtime.type.Array",
        ValueClass::Query => "lucee.runtime.type.Query",
        ValueClass::Str => "java.lang.String",
    }
}

// ── disk files ───────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct DiskEntry {
    k: String,
    c: u64,
    v: CfmlValue,
}

fn cache_dir(root: &Path, name: &str) -> PathBuf {
    // Cache names are CacheBox ids (letters, digits, `_`); keep anything else
    // out of the path.
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    root.join(safe)
}

fn read_disk(path: &Path) -> Option<DiskEntry> {
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

// ── cache operations ─────────────────────────────────────────────────────

impl Cache {
    fn expired(&self, e: &Entry, now: u64) -> bool {
        match self.expiry {
            Expiry::Never => false,
            Expiry::Ttl(mins) => now.saturating_sub(e.created_ms) > mins * 60_000,
            Expiry::Tti(mins) => now.saturating_sub(e.accessed_ms) > mins * 60_000,
        }
    }

    fn drop_stored(&self, e: &Entry) {
        self.bytes.fetch_sub(e.size.min(self.bytes.load(Ordering::Relaxed)), Ordering::Relaxed);
        if let Stored::File(p) = &e.stored {
            let _ = std::fs::remove_file(p);
        }
    }

    fn load(&self, e: &Entry) -> Option<CfmlValue> {
        match &e.stored {
            Stored::Live(v) => Some(v.clone()),
            Stored::Bytes(b) => serde_json::from_slice::<CfmlValue>(b).ok(),
            Stored::File(p) => read_disk(p).map(|d| d.v),
        }
    }

    fn get(&self, key: &str) -> Option<CfmlValue> {
        let now = now_ms();
        let mut m = self.entries.lock().ok()?;
        let Some(idx) = m.get_index_of(key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        if self.expired(&m[idx], now) {
            if let Some(e) = m.shift_remove(key) {
                self.drop_stored(&e);
            }
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        // Most recently used goes to the end.
        let last = m.len() - 1;
        m.move_index(idx, last);
        let e = &mut m[last];
        e.accessed_ms = now;
        match self.load(e) {
            Some(v) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(v)
            }
            None => {
                // An unreadable disk file: treat as absent.
                if let Some(e) = m.shift_remove(key) {
                    self.drop_stored(&e);
                }
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    fn contains(&self, key: &str) -> bool {
        let now = now_ms();
        let Ok(mut m) = self.entries.lock() else { return false };
        match m.get(key) {
            Some(e) if self.expired(e, now) => {
                if let Some(e) = m.shift_remove(key) {
                    self.drop_stored(&e);
                }
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    fn put(&self, key: String, value: CfmlValue) -> Result<(), CfmlError> {
        if !class_ok(self.value_class, &value) {
            return Err(CfmlError::new(
                format!(
                    "Invalid value type, expected : {}",
                    value_class_name(self.value_class)
                ),
                CfmlErrorType::Custom("java.lang.ClassCastException".to_string()),
            ));
        }
        let now = now_ms();
        let (stored, size) = match &self.tier {
            Tier::Heap { .. } => {
                let size = value.approx_heap_bytes(&mut std::collections::HashSet::new()) as u64;
                (Stored::Live(value), size)
            }
            Tier::OffHeap { .. } => {
                check_serialisable(&value)?;
                let b = serde_json::to_vec(&value).map_err(|e| err(format!("cannot serialise the value: {e}")))?;
                let n = b.len() as u64;
                (Stored::Bytes(b), n)
            }
            Tier::Disk { dir, .. } => {
                check_serialisable(&value)?;
                let body = serde_json::to_vec(&DiskEntry { k: key.clone(), c: now, v: value })
                    .map_err(|e| err(format!("cannot serialise the value: {e}")))?;
                std::fs::create_dir_all(dir).map_err(|e| err(format!("cache directory {}: {e}", dir.display())))?;
                let path = dir.join(format!("{:016x}.entry", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, &body)
                    .and_then(|_| std::fs::rename(&tmp, &path))
                    .map_err(|e| err(format!("cache write {}: {e}", path.display())))?;
                (Stored::File(path), body.len() as u64)
            }
        };
        let mut m = self.entries.lock().map_err(|_| err("cache poisoned"))?;
        if let Some(old) = m.shift_remove(&key) {
            self.drop_stored(&old);
        }
        m.insert(key, Entry { stored, size, created_ms: now, accessed_ms: now });
        self.bytes.fetch_add(size, Ordering::Relaxed);
        self.enforce_capacity(&mut m);
        Ok(())
    }

    fn enforce_capacity(&self, m: &mut indexmap::IndexMap<String, Entry>) {
        let (max_entries, max_bytes) = match &self.tier {
            Tier::Heap { max_entries, max_bytes } => (*max_entries, *max_bytes),
            Tier::OffHeap { max_bytes } | Tier::Disk { max_bytes, .. } => (0, *max_bytes),
        };
        loop {
            let over_count = max_entries > 0 && m.len() as u64 > max_entries;
            let over_bytes = max_bytes > 0 && self.bytes.load(Ordering::Relaxed) > max_bytes && m.len() > 1;
            if !(over_count || over_bytes) {
                break;
            }
            match m.shift_remove_index(0) {
                Some((_, e)) => {
                    self.drop_stored(&e);
                    self.evictions.fetch_add(1, Ordering::Relaxed);
                }
                None => break,
            }
        }
    }

    fn remove(&self, key: &str) {
        if let Ok(mut m) = self.entries.lock() {
            if let Some(e) = m.shift_remove(key) {
                self.drop_stored(&e);
            }
        }
    }

    fn clear(&self) {
        if let Ok(mut m) = self.entries.lock() {
            for (_, e) in m.drain(..) {
                self.drop_stored(&e);
            }
        }
        self.bytes.store(0, Ordering::Relaxed);
    }

    fn live_keys(&self) -> Vec<String> {
        let now = now_ms();
        let Ok(mut m) = self.entries.lock() else { return Vec::new() };
        let dead: Vec<String> = m
            .iter()
            .filter(|(_, e)| self.expired(e, now))
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            if let Some(e) = m.shift_remove(&k) {
                self.drop_stored(&e);
            }
        }
        m.keys().cloned().collect()
    }

    /// A persistent disk cache picks its entries back up from its files.
    fn reload_from_disk(&self) {
        let Tier::Disk { dir, .. } = &self.tier else { return };
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut found: Vec<(u64, String, Entry)> = Vec::new();
        for f in rd.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("entry") {
                continue;
            }
            let Some(d) = read_disk(&p) else {
                let _ = std::fs::remove_file(&p);
                continue;
            };
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            found.push((d.c, d.k, Entry { stored: Stored::File(p), size, created_ms: d.c, accessed_ms: d.c }));
        }
        found.sort_by_key(|(c, _, _)| *c);
        if let Ok(mut m) = self.entries.lock() {
            for (_, k, e) in found {
                self.bytes.fetch_add(e.size, Ordering::Relaxed);
                if let Some(old) = m.insert(k, e) {
                    self.drop_stored(&old);
                }
            }
            self.enforce_capacity(&mut m);
        }
    }
}

// ── configuration (CbEhCacheService._buildConfig) ─────────────────────────

fn cfg_num(cfg: &CfmlStruct, key: &str) -> u64 {
    cfg.get_ci(key)
        .map(|v| v.as_string().trim().parse::<f64>().unwrap_or(0.0))
        .filter(|n| *n > 0.0)
        .map(|n| n as u64)
        .unwrap_or(0)
}

fn cfg_bool(cfg: &CfmlStruct, key: &str) -> bool {
    cfg.get_ci(key).map(|v| v.is_true()).unwrap_or(false)
}

fn build_cache(root: &Path, name: &str, cfg: &CfmlStruct) -> Result<Cache, CfmlError> {
    let value_class = match cfg.get_ci("valueClass").map(|v| v.as_string().to_lowercase()).as_deref() {
        None | Some("") | Some("java.lang.object") => ValueClass::Any,
        Some("struct") | Some("lucee.runtime.type.struct") => ValueClass::Struct,
        Some("array") | Some("lucee.runtime.type.array") => ValueClass::Array,
        Some("query") | Some("lucee.runtime.type.query") => ValueClass::Query,
        Some("java.lang.string") | Some("string") => ValueClass::Str,
        Some(other) => {
            return Err(CfmlError::new(
                other.to_string(),
                CfmlErrorType::Custom("java.lang.ClassNotFoundException".to_string()),
            ))
        }
    };
    let mb = cfg_num(cfg, "maxSizeInMb") * 1024 * 1024;
    let storage = cfg.get_ci("storage").map(|v| v.as_string().to_lowercase()).unwrap_or_default();
    let tier = match storage.as_str() {
        "offheap" => Tier::OffHeap { max_bytes: mb },
        "disk" => Tier::Disk { max_bytes: mb, dir: cache_dir(root, name), persistent: cfg_bool(cfg, "persistent") },
        _ => Tier::Heap { max_entries: cfg_num(cfg, "maxObjects"), max_bytes: mb },
    };
    let last_access = cfg_num(cfg, "objectDefaultLastAccessTimeout");
    let timeout = cfg_num(cfg, "objectDefaultTimeout");
    let expiry = if last_access + timeout == 0 {
        Expiry::Never
    } else if cfg_bool(cfg, "useLastAccessTimeouts") {
        Expiry::Tti(if last_access > 0 { last_access } else { timeout })
    } else {
        Expiry::Ttl(timeout)
    };
    Ok(Cache {
        tier,
        expiry,
        value_class,
        entries: Mutex::new(indexmap::IndexMap::new()),
        bytes: AtomicU64::new(0),
        hits: AtomicU64::new(0),
        misses: AtomicU64::new(0),
        evictions: AtomicU64::new(0),
        closed: AtomicBool::new(false),
    })
}

// ── dispatch ─────────────────────────────────────────────────────────────

impl CfmlVirtualMachine {
    pub(crate) fn construct_ehcache_shim(&self) -> CfmlResult {
        Ok(CfmlValue::strukt(shim("org.pixl8.cbehcache.CbEhCacheService")))
    }

    pub(crate) fn dispatch_ehcache_shim(
        &mut self,
        class_lower: &str,
        method: &str,
        args: Vec<CfmlValue>,
        object: &CfmlValue,
    ) -> CfmlResult {
        let arg = |i: usize| args.get(i).cloned().unwrap_or(CfmlValue::Null);
        match (class_lower, method) {
            // new CbEhCacheService(storageDir) — Lucee's createObject().init(...)
            (SERVICE_CLASS, "init") if int_field(object, "__ehc_mgr").is_none() => {
                let dir = PathBuf::from(arg(0).as_string());
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let mgr = Arc::new(Manager { dir, available: AtomicBool::new(true), caches: Mutex::new(HashMap::new()) });
                if let Ok(mut m) = managers().lock() {
                    m.insert(id, mgr);
                }
                if let CfmlValue::Struct(s) = object {
                    s.insert("__ehc_mgr".to_string(), CfmlValue::Int(id as i64));
                }
                Ok(object.clone())
            }
            (SERVICE_CLASS, "init") => {
                let mgr = manager_of(object)?;
                if mgr.available.swap(true, Ordering::Relaxed) {
                    return Err(err("State is AVAILABLE: the cache manager is already initialised"));
                }
                Ok(CfmlValue::Null)
            }
            (SERVICE_CLASS, "close") => {
                let mgr = manager_of(object)?;
                mgr.available.store(false, Ordering::Relaxed);
                let caches: Vec<Arc<Cache>> = mgr
                    .caches
                    .lock()
                    .map(|mut c| c.drain().map(|(_, v)| v).collect())
                    .unwrap_or_default();
                for c in caches {
                    c.closed.store(true, Ordering::Relaxed);
                    // A non-persistent disk cache leaves nothing behind.
                    if let Tier::Disk { dir, persistent: false, .. } = &c.tier {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                Ok(CfmlValue::Null)
            }
            (SERVICE_CLASS, "getstatus") => {
                let mgr = manager_of(object)?;
                Ok(CfmlValue::string(
                    if mgr.available.load(Ordering::Relaxed) { "AVAILABLE" } else { "UNINITIALIZED" }.to_string(),
                ))
            }
            (SERVICE_CLASS, "createcache") => {
                let mgr = manager_of(object)?;
                if !mgr.available.load(Ordering::Relaxed) {
                    return Err(err("State is UNINITIALIZED"));
                }
                let name = arg(0).as_string();
                let cfg = match arg(1) {
                    CfmlValue::Struct(s) => s,
                    _ => CfmlStruct::new(ValueMap::default()),
                };
                if mgr.caches.lock().map(|c| c.contains_key(&name)).unwrap_or(false) {
                    return Err(err(format!("Cache '{}' already exists", name)));
                }
                let cache = build_cache(&mgr.dir, &name, &cfg)?;
                if let Tier::Disk { dir, persistent, .. } = &cache.tier {
                    if *persistent {
                        cache.reload_from_disk();
                    } else {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                if let Ok(mut c) = mgr.caches.lock() {
                    c.insert(name.clone(), Arc::new(cache));
                }
                let mut m = shim("org.ehcache.Cache");
                m.insert("__ehc_mgr".to_string(), field(object, "__ehc_mgr").unwrap_or(CfmlValue::Null));
                m.insert("__ehc_cache".to_string(), CfmlValue::string(name));
                Ok(CfmlValue::strukt(m))
            }
            (SERVICE_CLASS, "getstats") => {
                let mgr = manager_of(object)?;
                let name = arg(0).as_string();
                if !mgr.caches.lock().map(|c| c.contains_key(&name)).unwrap_or(false) {
                    return Err(err(format!("Unknown cache: {}", name)));
                }
                let mut m = shim("org.ehcache.core.statistics.CacheStatistics");
                m.insert("__ehc_mgr".to_string(), field(object, "__ehc_mgr").unwrap_or(CfmlValue::Null));
                m.insert("__ehc_cache".to_string(), CfmlValue::string(name));
                Ok(CfmlValue::strukt(m))
            }

            (CACHE_CLASS, "get") => Ok(cache_of(object)?.get(&arg(0).as_string()).unwrap_or(CfmlValue::Null)),
            (CACHE_CLASS, "put") => {
                cache_of(object)?.put(arg(0).as_string(), arg(1))?;
                Ok(CfmlValue::Null)
            }
            (CACHE_CLASS, "containskey") => Ok(CfmlValue::Bool(cache_of(object)?.contains(&arg(0).as_string()))),
            (CACHE_CLASS, "remove") => {
                cache_of(object)?.remove(&arg(0).as_string());
                Ok(CfmlValue::Null)
            }
            (CACHE_CLASS, "clear") => {
                cache_of(object)?.clear();
                Ok(CfmlValue::Null)
            }
            (CACHE_CLASS, "iterator") => {
                let cache = cache_of(object)?;
                let keys = cache.live_keys().into_iter().map(CfmlValue::string).collect();
                let mut m = shim("org.ehcache.Cache.Iterator");
                m.insert("__ehc_mgr".to_string(), field(object, "__ehc_mgr").unwrap_or(CfmlValue::Null));
                m.insert("__ehc_cache".to_string(), field(object, "__ehc_cache").unwrap_or(CfmlValue::Null));
                m.insert("__keys".to_string(), CfmlValue::array(keys));
                m.insert("__pos".to_string(), CfmlValue::Int(0));
                Ok(CfmlValue::strukt(m))
            }
            (ITERATOR_CLASS, "hasnext") | (ITERATOR_CLASS, "next") => {
                let keys = match field(object, "__keys") {
                    Some(CfmlValue::Array(a)) => a.snapshot(),
                    _ => Vec::new(),
                };
                let pos = int_field(object, "__pos").unwrap_or(0) as usize;
                if method == "hasnext" {
                    return Ok(CfmlValue::Bool(pos < keys.len()));
                }
                let Some(key) = keys.get(pos).cloned() else {
                    return Err(CfmlError::new(
                        "no more elements".to_string(),
                        CfmlErrorType::Custom("java.util.NoSuchElementException".to_string()),
                    ));
                };
                if let CfmlValue::Struct(s) = object {
                    s.insert("__pos".to_string(), CfmlValue::Int(pos as i64 + 1));
                }
                let mut m = shim("org.ehcache.Cache.Entry");
                m.insert("__key".to_string(), key);
                m.insert("__ehc_mgr".to_string(), field(object, "__ehc_mgr").unwrap_or(CfmlValue::Null));
                m.insert("__ehc_cache".to_string(), field(object, "__ehc_cache").unwrap_or(CfmlValue::Null));
                Ok(CfmlValue::strukt(m))
            }
            (ENTRY_CLASS, "getkey") => Ok(field(object, "__key").unwrap_or(CfmlValue::Null)),
            (ENTRY_CLASS, "getvalue") => {
                let key = field(object, "__key").map(|v| v.as_string()).unwrap_or_default();
                Ok(cache_of(object)?.get(&key).unwrap_or(CfmlValue::Null))
            }

            (STATS_CLASS, m) => {
                let cache = cache_of(object)?;
                let hits = cache.hits.load(Ordering::Relaxed);
                let misses = cache.misses.load(Ordering::Relaxed);
                match m {
                    "getcachehits" => Ok(CfmlValue::Int(hits as i64)),
                    "getcachemisses" => Ok(CfmlValue::Int(misses as i64)),
                    "getcacheevictions" => Ok(CfmlValue::Int(cache.evictions.load(Ordering::Relaxed) as i64)),
                    "getcachegets" => Ok(CfmlValue::Int((hits + misses) as i64)),
                    "getcachehitpercentage" => Ok(CfmlValue::Double(if hits + misses == 0 {
                        0.0
                    } else {
                        hits as f64 * 100.0 / (hits + misses) as f64
                    })),
                    "getcachemisspercentage" => Ok(CfmlValue::Double(if hits + misses == 0 {
                        0.0
                    } else {
                        misses as f64 * 100.0 / (hits + misses) as f64
                    })),
                    "clear" => {
                        cache.hits.store(0, Ordering::Relaxed);
                        cache.misses.store(0, Ordering::Relaxed);
                        cache.evictions.store(0, Ordering::Relaxed);
                        Ok(CfmlValue::Null)
                    }
                    "getknownstatistics" => {
                        let mapped = cache.live_keys().len() as i64;
                        let stat = |v: i64| {
                            let mut s = shim("org.ehcache.core.statistics.TierStatistics.Statistic");
                            s.insert("__value".to_string(), CfmlValue::Int(v));
                            CfmlValue::strukt(s)
                        };
                        let mut out = ValueMap::default();
                        out.insert("Cache:HitCount".to_string(), stat(hits as i64));
                        out.insert("Cache:MissCount".to_string(), stat(misses as i64));
                        out.insert(
                            "Cache:EvictionCount".to_string(),
                            stat(cache.evictions.load(Ordering::Relaxed) as i64),
                        );
                        out.insert(cache.tier.mapping_stat().to_string(), stat(mapped));
                        Ok(CfmlValue::strukt(out))
                    }
                    other => Err(err(format!("CacheStatistics.{}() is not supported by RustCFML", other))),
                }
            }
            (STAT_CLASS, "value") => Ok(field(object, "__value").unwrap_or(CfmlValue::Int(0))),
            (class, other) => Err(err(format!("{}.{}() is not supported by RustCFML", class, other))),
        }
    }
}
