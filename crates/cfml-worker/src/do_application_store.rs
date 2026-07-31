//! Durable Object–backed application store.
//!
//! Strong-consistency successor to [`crate::kv_stores::KvBackedApplicationStore`].
//! All persistence funnels through a single DO instance per application
//! name, so reads after writes are immediate across isolates and regions
//! (subject to DO's single-region home for the named instance).
//!
//! ## Architecture
//!
//! - The DO class is the host project's responsibility. It must expose
//!   a small JSON RPC over `fetch`:
//!
//!   - `GET /get` → `200 {variables}` if state exists, `404` if not.
//!   - `POST /put` body `{variables}` → `204` on success.
//!
//!   Multi-app support: the DO is addressed via
//!   `namespace.idFromName(<app_name>)`, so one DO instance per app name.
//!   The single DO instance serializes all requests, so `/put` is atomic
//!   at the DO level.
//!
//! - From the wasm side we keep the same memory-cache + dirty-tracking
//!   pattern as the KV store. Writes flush at the end of the request
//!   via `ctx.wait_until(...)` so they don't add to response latency.
//!
//! - The trait surface is sync, so every DO round-trip goes through JSPI
//!   ([`crate::jspi::do_fetch_sync`]).
//!
//! ## v1 trade-offs
//!
//! - **Race-on-write**: two isolates that `modify` concurrently both PUT
//!   their post-mutation state; the last writer wins. Same race window
//!   as the KV store. A v2 with a server-side merge endpoint (or
//!   optimistic-concurrency version field) closes this. Acceptable for
//!   typical low-write app-scope use (config flags, factory caches).
//! - **`onApplicationStart` exact-once**: requires the DO's `/put` of
//!   `started=true` to complete before another isolate primes. The DO's
//!   single-instance request serialization makes this true in practice
//!   provided the first isolate's `flush` lands before the second cold
//!   start's prime — typically true since flush runs immediately after
//!   `onApplicationStart`. A future tightening can move the
//!   `started=true` write into a sync `/claim` endpoint.

#![cfg(target_arch = "wasm32")]

use cfml_vm::application_store::{ApplicationStore, MemoryApplicationStore};
use cfml_vm::ApplicationState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use worker::{Method, ObjectNamespace, Request, RequestInit};

#[derive(Clone)]
pub struct DoApplicationStore {
    memory: Arc<MemoryApplicationStore>,
    /// Cloudflare Durable Object namespace binding. Used directly from
    /// async prime/flush — no JSPI needed because both are called from
    /// the async fetch handler, never from inside the sync VM.
    namespace: ObjectNamespace,
    dirty: Arc<Mutex<HashSet<String>>>,
}

impl DoApplicationStore {
    pub fn new(namespace: ObjectNamespace) -> Self {
        Self {
            memory: Arc::new(MemoryApplicationStore::new()),
            namespace,
            dirty: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Fetch existing state for `app_name` from the DO and warm the local
    /// cache. Called once per request before the VM runs.
    pub async fn prime(&self, app_name: &str) -> worker::Result<()> {
        if app_name.is_empty() || self.memory.contains(app_name) {
            return Ok(());
        }

        let stub = self.namespace.id_from_name(app_name)?.get_stub()?;
        let resp = stub.fetch_with_str("https://do.invalid/get").await?;
        if resp.status_code() == 404 {
            return Ok(());
        }
        if resp.status_code() >= 400 {
            return Ok(());
        }
        let mut resp = resp;
        let body = resp.text().await.unwrap_or_default();
        let Ok(snap) = serde_json::from_str::<PersistedApp>(&body) else {
            return Ok(());
        };
        self.memory.insert(
            app_name,
            ApplicationState {
                name: app_name.to_string(),
                variables: snap.variables,
                started: snap.started,
                config: cfml_common::dynamic::ValueMap::default(),
                app_function_table: Vec::new(),
                app_fn_prune_at: 0,
                session_storage: None,
                app_caches: indexmap::IndexMap::new(),
            },
        );
        Ok(())
    }

    /// Persist every dirty application back to the DO. Awaited inline by the
    /// handler after the VM returns — deliberately *not* deferred via
    /// `ctx.wait_until`, for the same reason as
    /// [`KvBackedSessionStore::flush`](crate::kv_stores::KvBackedSessionStore::flush):
    /// writes scheduled with `wait_until` after the JSPI promising activation
    /// are silently dropped once the request has performed an awaited
    /// read (the per-request `prime`), so application-scope mutations never
    /// reach the Durable Object. Awaiting here guarantees the write lands.
    pub async fn flush(&self) -> worker::Result<()> {
        let dirty: Vec<String> = {
            let mut g = self.dirty.lock().unwrap();
            g.drain().collect()
        };
        for name in dirty {
            let Some(state) = self.memory.get(&name) else { continue };
            let snap = PersistedApp {
                variables: state.variables.clone(),
                started: state.started,
            };
            let body = serde_json::to_string(&snap)
                .map_err(|e| worker::Error::RustError(format!("app {name} serialize: {e}")))?;
            let id = self.namespace.id_from_name(&name)?;
            let stub = id.get_stub()?;
            let mut init = RequestInit::new();
            init.with_method(Method::Post)
                .with_body(Some(wasm_bindgen::JsValue::from_str(&body)));
            let req = Request::new_with_init("https://do.invalid/put", &init)?;
            stub.fetch_with_request(req).await?;
        }
        Ok(())
    }
}

impl ApplicationStore for DoApplicationStore {
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

#[derive(Serialize, Deserialize)]
struct PersistedApp {
    variables: cfml_common::dynamic::ValueMap,
    #[serde(default)]
    started: bool,
}
