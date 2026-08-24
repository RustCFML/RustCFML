//! Memcached-backed session store.
//!
//! Enabled by the `memcached` Cargo feature. Each session is serialised to
//! JSON and stored under the key `{key_prefix}{app_name}\u{1f}{session_id}`
//! with a TTL equal to the session's `timeout_secs`. The application name is
//! part of the key so two applications sharing a CFID never collide.
//!
//! **Migration note:** keys gained the `{app_name}\u{1f}` segment, so session
//! keys written by an earlier build (`{key_prefix}{session_id}`) are no longer
//! found — existing memcached sessions are effectively invalidated on upgrade
//! (users re-authenticate / start fresh sessions). Memcached enforces TTL
//! natively, so the stale keys expire on their own.

#[cfg(feature = "memcached")]
mod inner {
    use cfml_common::dynamic::ValueMap;
    use cfml_vm::{SessionData, session_store::SessionStore};
    
    use memcache::Client;

    pub struct MemcachedStore {
        client: Client,
        key_prefix: String,
    }

    impl MemcachedStore {
        /// Connect to a Memcached cluster.
        ///
        /// `servers` should be bare `host:port` strings; the `memcache://`
        /// scheme is added automatically.
        pub fn new(servers: &[String], key_prefix: &str) -> Result<Self, memcache::MemcacheError> {
            let urls: Vec<String> = servers
                .iter()
                .map(|s| {
                    if s.starts_with("memcache://") {
                        s.clone()
                    } else {
                        format!("memcache://{}", s)
                    }
                })
                .collect();
            let client = memcache::connect(urls)?;
            Ok(Self {
                client,
                key_prefix: key_prefix.to_string(),
            })
        }

        /// Namespace the storage key by application: `{prefix}{app}\u{1f}{id}`.
        /// App names are case-insensitive in CFML, so lowercase the app
        /// segment for consistent lookups across `this.name` casings.
        fn key(&self, app: &str, id: &str) -> String {
            format!("{}{}\u{1f}{}", self.key_prefix, app.to_lowercase(), id)
        }
    }

    impl SessionStore for MemcachedStore {
        // Serializes session data to memcached — data values only (issue #236).
        fn persists_by_serialization(&self) -> bool {
            true
        }

        // Every `get` is a network round trip — the VM must memo the record
        // per request rather than re-read it (issue #361).
        fn reads_are_cheap(&self) -> bool {
            false
        }

        fn get(&self, app: &str, id: &str) -> Option<SessionData> {
            let raw: Option<String> = self.client.get(&self.key(app, id)).ok().flatten();
            raw.and_then(|s| serde_json::from_str(&s).ok())
        }

        fn set(&self, app: &str, id: &str, data: SessionData) {
            if let Ok(json) = serde_json::to_string(&data) {
                let ttl = data.timeout_secs as u32;
                let _ = self.client.set(&self.key(app, id), json, ttl);
            }
        }

        fn remove(&self, app: &str, id: &str) {
            let _ = self.client.delete(&self.key(app, id));
        }

        fn rotate(&self, app: &str, old_id: &str, new_id: &str) {
            if let Some(data) = self.get(app, old_id) {
                self.set(app, new_id, data);
                self.remove(app, old_id);
            }
        }

        fn take_expired(
            &self,
            _now_secs: u64,
        ) -> Vec<(String, String, ValueMap)> {
            // Memcached enforces TTL natively — nothing to drain here, and
            // onSessionEnd is consequently never delivered for this store.
            Vec::new()
        }
    }
}

#[cfg(feature = "memcached")]
pub use inner::MemcachedStore;
