//! KV-backed session + application stores.
//!
//! Workers KV is async; the [`cfml_vm::SessionStore`] /
//! [`cfml_vm::ApplicationStore`] traits the VM consumes are sync. Bridging
//! the two cleanly without changing the VM contract: prime an in-isolate
//! [`MemoryStore`] / [`MemoryApplicationStore`] from KV at request start,
//! mirror writes back via `ctx.wait_until(...)` after the VM finishes.
//!
//! Trade-offs:
//! - **Eventual consistency**: two isolates touching the same session or
//!   application name race the way any KV-backed store does. Sessions
//!   tolerate this; if your app needs strong consistency for application
//!   scope, run a Durable Object instead.
//! - **`onSessionEnd` doesn't fire**: KV's TTL silently evicts sessions
//!   without giving the host a hook. Acceptable for v1 — document and
//!   revisit alongside a scheduled-event handler.

#![cfg(target_arch = "wasm32")]

use cfml_vm::application_store::{ApplicationStore, MemoryApplicationStore};
use cfml_vm::session_store::{MemoryStore, SessionStore};
use cfml_vm::{ApplicationState, SessionData};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use worker::kv::KvStore;

// ─────────────────────────────────────────────
// Sessions
// ─────────────────────────────────────────────

/// Fixed application partition for the in-isolate `MemoryStore`. The worker
/// keys sessions by bare id (see the impl note below); using one constant
/// partition keeps every internal access — `prime`, `flush`, `sweep_expired`,
/// and the `SessionStore` methods — consistent regardless of the per-request
/// app name the VM supplies.
const MEM_APP: &str = "";

#[derive(Clone)]
pub struct KvBackedSessionStore {
    memory: Arc<MemoryStore>,
    kv: KvStore,
    dirty: Arc<Mutex<HashSet<String>>>,
}

impl KvBackedSessionStore {
    pub fn new(kv: KvStore) -> Self {
        Self {
            memory: Arc::new(MemoryStore::new()),
            kv,
            dirty: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Pull `session_id` from KV into the in-isolate cache if not already
    /// present. Called once per request before the VM runs.
    pub async fn prime(&self, session_id: &str) -> worker::Result<()> {
        if session_id.is_empty() || self.memory.contains(MEM_APP, session_id) {
            return Ok(());
        }
        if let Some(bytes) = self.kv.get(session_id).bytes().await? {
            if let Ok(data) = serde_json::from_slice::<SessionData>(&bytes) {
                self.memory.set(MEM_APP, session_id, data);
            }
        }
        Ok(())
    }

    /// Sweep KV for expired sessions, delete them, and return the
    /// `(id, SessionData)` pairs so the caller can fire `onSessionEnd`
    /// for each. Called from the scheduled (cron) handler — the inline
    /// request path keeps `take_expired` returning empty because KV's
    /// TTL handles eviction.
    ///
    /// Race safety: each expired key is deleted from KV *before* being
    /// returned, so two overlapping cron firings will see at most one
    /// successful delete per session.
    pub async fn sweep_expired(&self, now_secs: u64) -> worker::Result<Vec<(String, SessionData)>> {
        let mut expired = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut builder = self.kv.list();
            if let Some(c) = cursor.as_ref() {
                builder = builder.cursor(c.clone());
            }
            let resp = builder.execute().await.map_err(|e| worker::Error::RustError(format!("kv list: {e:?}")))?;
            for key in resp.keys {
                let bytes = match self.kv.get(&key.name).bytes().await? {
                    Some(b) => b,
                    None => continue,
                };
                let Ok(data) = serde_json::from_slice::<SessionData>(&bytes) else {
                    continue;
                };
                let age = now_secs.saturating_sub(data.last_accessed_secs);
                if age > data.timeout_secs {
                    // Delete first so concurrent sweeps don't double-fire.
                    if self.kv.delete(&key.name).await.is_ok() {
                        self.memory.remove(MEM_APP, &key.name);
                        expired.push((key.name, data));
                    }
                }
            }
            if resp.list_complete {
                break;
            }
            cursor = resp.cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(expired)
    }

    /// Persist every dirty session back to KV. Awaited inline by the handler
    /// after the VM returns — deliberately *not* deferred via
    /// `ctx.wait_until`.
    ///
    /// Why awaited: writes scheduled with `wait_until` after the JSPI
    /// promising sync activation were observed to be silently dropped on any
    /// request that had already performed an awaited KV read (the per-request
    /// [`prime`](Self::prime)). The symptom was that an *existing* session's
    /// mutations — user `session.X` writes and the framework's own
    /// `last_accessed_secs` bump (which drives timeout sliding) — never
    /// reached KV, so the session appeared frozen at creation. Awaiting here
    /// runs in the same async context as `prime`, where KV operations are
    /// proven to work, and guarantees the write lands before the response is
    /// returned. Errors are propagated instead of swallowed.
    pub async fn flush(&self) -> worker::Result<()> {
        let dirty: Vec<String> = {
            let mut g = self.dirty.lock().unwrap();
            g.drain().collect()
        };
        for id in dirty {
            let Some(data) = self.memory.get(MEM_APP, &id) else { continue };
            let bytes = serde_json::to_vec(&data)
                .map_err(|e| worker::Error::RustError(format!("session {id} serialize: {e}")))?;
            // KV rejects an `expiration_ttl` below 60s. Defend against a
            // stale/misconfigured `timeout_secs` (e.g. a 0 written by an
            // older build) so the write can't silently fail again.
            let ttl = data.timeout_secs.max(60);
            self.kv
                .put_bytes(&id, &bytes)
                .map_err(|e| worker::Error::RustError(format!("session {id} put: {e:?}")))?
                .expiration_ttl(ttl)
                .execute()
                .await
                .map_err(|e| worker::Error::RustError(format!("session {id} flush: {e:?}")))?;
        }
        Ok(())
    }
}

// NOTE on application namespacing: the `SessionStore` trait keys sessions by
// `(app_name, id)` so a multi-application process isolates sessions per app.
// A Workers isolate, however, runs a single application per script and `prime`
// pulls a session from KV by bare id *before* `Application.cfc` resolves the
// app name — so the app name is not yet known at the KV key boundary. This
// store therefore keys by bare id (app-agnostic), matching its prime/flush/KV
// layout. The trait's `app` argument is accepted and ignored; for the one-app-
// per-worker deployment model there is nothing to disambiguate. The native
// multi-app stores (memory/datasource/cluster/memcached) do the real
// namespacing.
impl SessionStore for KvBackedSessionStore {
    // Serializes session data to Cloudflare KV — data values only (issue #236).
    fn persists_by_serialization(&self) -> bool {
        true
    }

    fn get(&self, _app: &str, id: &str) -> Option<SessionData> {
        self.memory.get(MEM_APP, id)
    }

    fn set(&self, _app: &str, id: &str, data: SessionData) {
        self.memory.set(MEM_APP, id, data);
        self.dirty.lock().unwrap().insert(id.to_string());
    }

    fn remove(&self, _app: &str, id: &str) {
        self.memory.remove(MEM_APP, id);
        // Schedule the KV delete on the next flush; cheaper to track as a
        // separate set, but for v1 piggyback on `dirty` and let flush detect
        // missing memory entries → delete. Actually simpler: do nothing here
        // and let TTL clean up. Document the trade-off.
        let _ = id;
    }

    fn rotate(&self, _app: &str, old_id: &str, new_id: &str) {
        self.memory.rotate(MEM_APP, old_id, new_id);
        self.dirty.lock().unwrap().insert(new_id.to_string());
    }

    fn contains(&self, _app: &str, id: &str) -> bool {
        self.memory.contains(MEM_APP, id)
    }

    fn take_expired(&self, now_secs: u64) -> Vec<(String, String, cfml_common::dynamic::ValueMap)> {
        // Worker isolates don't run a background sweeper — expiry is handled
        // by KV TTL. Always return empty so the VM doesn't try to fire
        // onSessionEnd for sessions it can't observe.
        let _ = now_secs;
        Vec::new()
    }
}

// ─────────────────────────────────────────────
// Application scope
// ─────────────────────────────────────────────

#[derive(Clone)]
pub struct KvBackedApplicationStore {
    memory: Arc<MemoryApplicationStore>,
    kv: KvStore,
    dirty: Arc<Mutex<HashSet<String>>>,
}

impl KvBackedApplicationStore {
    pub fn new(kv: KvStore) -> Self {
        Self {
            memory: Arc::new(MemoryApplicationStore::new()),
            kv,
            dirty: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn prime(&self, app_name: &str) -> worker::Result<()> {
        if app_name.is_empty() || self.memory.contains(app_name) {
            return Ok(());
        }
        if let Some(bytes) = self.kv.get(app_name).bytes().await? {
            if let Ok(snap) = serde_json::from_slice::<PersistedApp>(&bytes) {
                self.memory.insert(
                    app_name,
                    ApplicationState {
                        name: app_name.to_string(),
                        variables: snap.variables,
                        // `started` is intentionally false on cold isolates
                        // so onApplicationStart can re-fire if the user
                        // relies on it for per-isolate priming (function
                        // tables, caches, etc.).
                        started: false,
                        config: cfml_common::dynamic::ValueMap::default(),
                        app_function_table: Vec::new(),
                        app_fn_prune_at: 0,
                        session_storage: None,
                        app_caches: indexmap::IndexMap::new(),
                    },
                );
            }
        }
        Ok(())
    }

    /// Persist every dirty application back to KV. Awaited inline (not
    /// `ctx.wait_until`) for the same reason as
    /// [`KvBackedSessionStore::flush`]: deferred writes after the JSPI
    /// promising activation are silently dropped once the request has done an
    /// awaited KV read (`prime`).
    pub async fn flush(&self) -> worker::Result<()> {
        let dirty: Vec<String> = {
            let mut g = self.dirty.lock().unwrap();
            g.drain().collect()
        };
        for name in dirty {
            let Some(state) = self.memory.get(&name) else { continue };
            let snap = PersistedApp { variables: state.variables.clone() };
            let bytes = serde_json::to_vec(&snap)
                .map_err(|e| worker::Error::RustError(format!("app {name} serialize: {e}")))?;
            self.kv
                .put_bytes(&name, &bytes)
                .map_err(|e| worker::Error::RustError(format!("app {name} put: {e:?}")))?
                .execute()
                .await
                .map_err(|e| worker::Error::RustError(format!("app {name} flush: {e:?}")))?;
        }
        Ok(())
    }
}

impl ApplicationStore for KvBackedApplicationStore {
    fn get(&self, name: &str) -> Option<ApplicationState> {
        self.memory.get(name)
    }

    fn insert(&self, name: &str, state: ApplicationState) {
        self.memory.insert(name, state);
        self.dirty.lock().unwrap().insert(name.to_string());
    }

    fn contains(&self, name: &str) -> bool {
        self.memory.contains(name)
    }

    fn modify(&self, name: &str, f: &mut dyn FnMut(&mut ApplicationState)) {
        self.memory.modify(name, f);
        self.dirty.lock().unwrap().insert(name.to_string());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedApp {
    variables: cfml_common::dynamic::ValueMap,
}
