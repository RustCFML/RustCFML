//! Pluggable session storage backend.

use cfml_common::dynamic::ValueMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::SessionData;

/// Trait for session storage backends.
///
/// Session identity is the composite **(application name, session id)** —
/// matching Lucee, where `Application.cfc`'s `this.name` selects the
/// application and one process can host several apps that each have an
/// isolated session scope. The same `CFID` presented under two different
/// `this.name` values must resolve to two independent sessions. Every keyed
/// method therefore takes the application name alongside the id, so a lookup
/// cannot be performed without an application context. Application names are
/// case-insensitive (CFML semantics); implementations normalise as needed.
///
/// Implementations must be `Send + Sync` so they can be shared across Tokio
/// worker threads.
pub trait SessionStore: Send + Sync + 'static {
    /// Retrieve a session by `(app, id)`. Returns `None` if it does not exist.
    fn get(&self, app: &str, id: &str) -> Option<SessionData>;
    /// Insert or overwrite a session under `(app, id)`.
    fn set(&self, app: &str, id: &str, data: SessionData);
    /// Remove a session under `(app, id)`.
    fn remove(&self, app: &str, id: &str);
    /// Atomically move session data from `(app, old_id)` to `(app, new_id)`.
    fn rotate(&self, app: &str, old_id: &str, new_id: &str);
    /// Returns `true` if the session exists under `(app, id)`.
    fn contains(&self, app: &str, id: &str) -> bool {
        self.get(app, id).is_some()
    }
    /// Drain all sessions whose `last_accessed_secs` age exceeds their
    /// `timeout_secs`, returning `(app_name, id, variables)` per drained
    /// session so callers can route `onSessionEnd` to the owning application.
    /// `app_name` is `""` for stores that do not record it (memcached / KV),
    /// which also never deliver `onSessionEnd`.
    fn take_expired(&self, now_secs: u64) -> Vec<(String, String, ValueMap)>;

    /// Earliest absolute expiry instant (unix epoch seconds) across all live
    /// sessions, if the store can compute it cheaply. Drives adaptive reaper
    /// scheduling: when `Some(t)`, the reaper may sleep until `t` (capped at
    /// the configured tick) instead of waking on a fixed interval. The default
    /// returns `None`, so a store opts out simply by not overriding it and the
    /// reaper falls back to its fixed tick.
    fn next_expiry(&self, _now_secs: u64) -> Option<u64> {
        None
    }

    /// Whether this store persists session data by SERIALIZING it (an external
    /// store: memcached, datasource, KV, cluster). Serializing stores can only
    /// hold plain data — a live CFC / closure / native object cannot survive the
    /// round trip — so the data-only session rule (issue #88) is enforced for
    /// them. The default in-process `MemoryStore` keeps live object references,
    /// so it returns `false`: CFCs/closures in `session` are allowed there,
    /// matching Lucee/ACF in-memory sessions (issue #236). Serializing stores
    /// override this to `true`.
    fn persists_by_serialization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────
// MemoryStore — in-process default
// ─────────────────────────────────────────────

/// In-process session store backed by a `Mutex<HashMap>`, keyed by the
/// composite `(application name, session id)` so two applications sharing a
/// CFID never collide. The map key is `"{app_lower}\u{1f}{id}"`; the unit
/// separator cannot appear in a UUID session id, so the bare id is always
/// recoverable for `take_expired`.
#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<String, SessionData>>>,
}

/// Build the composite map key. App names are case-insensitive in CFML, so
/// lowercase the app portion for consistent lookups.
fn composite_key(app: &str, id: &str) -> String {
    format!("{}\u{1f}{}", app.to_lowercase(), id)
}

/// Recover the bare session id from a composite key (everything after the
/// last unit separator). Falls back to the whole key if no separator is
/// present (defensive — never happens for keys we mint).
fn id_from_key(key: &str) -> &str {
    key.rsplit('\u{1f}').next().unwrap_or(key)
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore for MemoryStore {
    fn get(&self, app: &str, id: &str) -> Option<SessionData> {
        // Read-path exactness (G1): a session is invisible the instant it is
        // past `last_accessed + timeout`, independent of any sweep. Remove the
        // dead record opportunistically while we hold the lock.
        let now = crate::now_epoch_secs();
        let key = composite_key(app, id);
        let mut m = self.inner.lock().ok()?;
        match m.get(&key) {
            Some(s) if now.saturating_sub(s.last_accessed_secs) > s.timeout_secs => {
                m.remove(&key);
                None
            }
            Some(s) => Some(s.clone()),
            None => None,
        }
    }

    fn set(&self, app: &str, id: &str, data: SessionData) {
        if let Ok(mut m) = self.inner.lock() {
            m.insert(composite_key(app, id), data);
        }
    }

    fn remove(&self, app: &str, id: &str) {
        if let Ok(mut m) = self.inner.lock() {
            m.remove(&composite_key(app, id));
        }
    }

    fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
        if let Ok(mut m) = self.inner.lock() {
            if let Some(data) = m.remove(&composite_key(app, old_id)) {
                m.insert(composite_key(app, new_id), data);
            }
        }
    }

    fn contains(&self, app: &str, id: &str) -> bool {
        // Route through the expiry-aware get so an expired record reads absent.
        self.get(app, id).is_some()
    }

    fn take_expired(&self, now_secs: u64) -> Vec<(String, String, ValueMap)> {
        if let Ok(mut m) = self.inner.lock() {
            let expired: Vec<String> = m
                .iter()
                .filter(|(_, s)| now_secs.saturating_sub(s.last_accessed_secs) > s.timeout_secs)
                .map(|(k, _)| k.clone())
                .collect();
            expired
                .into_iter()
                .filter_map(|key| {
                    m.remove(&key)
                        .map(|s| (s.app_name.clone(), id_from_key(&key).to_string(), s.variables))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn next_expiry(&self, _now_secs: u64) -> Option<u64> {
        let m = self.inner.lock().ok()?;
        m.values()
            .map(|s| s.last_accessed_secs.saturating_add(s.timeout_secs))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::CfmlValue;
    use crate::now_epoch_secs;

    fn session(app: &str, last_accessed: u64, timeout: u64) -> SessionData {
        SessionData {
            variables: ValueMap::default(),
            created_secs: last_accessed,
            last_accessed_secs: last_accessed,
            auth_user: None,
            auth_roles: Vec::new(),
            timeout_secs: timeout,
            app_name: app.to_string(),
        }
    }

    #[test]
    fn get_hides_and_removes_expired_record() {
        let store = MemoryStore::new();
        let now = now_epoch_secs();
        // last accessed 10_000s ago, 5s timeout → already expired.
        store.set("appA", "dead", session("appA", now - 10_000, 5));
        assert!(store.get("appA", "dead").is_none(), "expired session must read as absent");
        assert!(!store.contains("appA", "dead"), "contains must agree with get");
        // It was opportunistically removed, so take_expired finds nothing left.
        assert!(store.take_expired(now).is_empty(), "get should have evicted it");
    }

    #[test]
    fn get_returns_live_record() {
        let store = MemoryStore::new();
        let now = now_epoch_secs();
        store.set("appA", "live", session("appA", now, 1800));
        assert!(store.get("appA", "live").is_some());
        assert!(store.contains("appA", "live"));
    }

    #[test]
    fn same_id_in_two_apps_is_two_sessions() {
        // The core namespacing guarantee: one CFID under two `this.name`
        // values resolves to independent sessions — neither leaks into the
        // other, and the app name is case-insensitive.
        let store = MemoryStore::new();
        let now = now_epoch_secs();
        let mut a = session("appA", now, 1800);
        a.variables.insert("x", CfmlValue::string("alpha"));
        let mut b = session("appB", now, 1800);
        b.variables.insert("x", CfmlValue::string("beta"));
        store.set("appA", "shared-cfid", a);
        store.set("appB", "shared-cfid", b);

        assert_eq!(
            store.get("appA", "shared-cfid").unwrap().variables.get("x").unwrap().as_string(),
            "alpha",
            "app A keeps its own value"
        );
        assert_eq!(
            store.get("AppB", "shared-cfid").unwrap().variables.get("x").unwrap().as_string(),
            "beta",
            "app B is independent and app name is case-insensitive"
        );
        // Removing A's session does not touch B's.
        store.remove("appA", "shared-cfid");
        assert!(store.get("appA", "shared-cfid").is_none());
        assert!(store.get("appB", "shared-cfid").is_some());
    }

    #[test]
    fn take_expired_reports_owning_app_name() {
        let store = MemoryStore::new();
        let now = now_epoch_secs();
        store.set("shop", "a", session("shop", now - 10_000, 5));
        store.set("blog", "b", session("blog", now, 1800));
        let mut drained = store.take_expired(now);
        assert_eq!(drained.len(), 1, "only the timed-out session is drained");
        let (app, id, _vars) = drained.pop().unwrap();
        assert_eq!(app, "shop");
        assert_eq!(id, "a");
        // The live session survives and its app is untouched.
        assert!(store.get("blog", "b").is_some());
    }

    #[test]
    fn next_expiry_is_earliest_absolute_instant() {
        let store = MemoryStore::new();
        let base = 1_000_000;
        store.set("x", "a", session("x", base, 100)); // expires at base+100
        store.set("x", "b", session("x", base, 30)); // expires at base+30  (earliest)
        store.set("x", "c", session("x", base, 900)); // expires at base+900
        assert_eq!(store.next_expiry(base), Some(base + 30));
    }
}
