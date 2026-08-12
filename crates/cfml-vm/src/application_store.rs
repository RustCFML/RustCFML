//! Pluggable application-scope storage backend.
//!
//! Mirrors [`crate::session_store::SessionStore`] in spirit. The default
//! in-process implementation is `MemoryApplicationStore`; embedders (e.g.
//! cfml-worker) supply alternatives backed by Cloudflare KV or other
//! key-value services.
//!
//! All operations are sync because all current call-sites in the VM already
//! run on a sync thread. KV-backed implementations should use a write-through
//! cache + `ctx.wait_until()` for async persistence.

use crate::ApplicationState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Backend trait for application-scope storage.
pub trait ApplicationStore: Send + Sync + 'static {
    /// Returns a cloned snapshot of the application state, or `None` if the
    /// named application is not yet present.
    fn get(&self, name: &str) -> Option<ApplicationState>;

    /// Insert or overwrite an entire application state.
    fn insert(&self, name: &str, state: ApplicationState);

    /// Returns `true` if the named application exists.
    fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Apply a mutator to the state and persist the result. No-op if the
    /// application is not present.
    ///
    /// Default impl is get → modify → insert. In-memory implementations may
    /// override for in-place mutation to avoid cloning.
    fn modify(&self, name: &str, f: &mut dyn FnMut(&mut ApplicationState)) {
        if let Some(mut state) = self.get(name) {
            f(&mut state);
            self.insert(name, state);
        }
    }

    /// Whether this backend needs the application scope written through at the end
    /// of every request.
    ///
    /// `false` for in-process stores: `ApplicationState::variables` is a live
    /// `CfmlStruct` (`Arc<RwLock<…>>`), so the scope a request mutates already *is*
    /// the stored object and republishing it would reintroduce last-writer-wins.
    ///
    /// `true` for backends that SERIALISE state (KV, Durable Object): they cannot
    /// hold a live shared scope, so they must be handed a snapshot.
    fn needs_variable_publish(&self) -> bool {
        false
    }

    /// Write the application scope through to a serialising backend. Only called
    /// when [`Self::needs_variable_publish`] is `true`.
    ///
    /// ⚠️ Such backends necessarily give a WEAKER guarantee than the in-process
    /// store: writes become visible to other nodes/isolates only at request end,
    /// so a guard-once idiom (`if ( !StructKeyExists( application, "x" ) ) { … }`)
    /// can still run more than once across nodes. That is a documented divergence,
    /// not something the trait can paper over.
    fn publish_variables(&self, name: &str, variables: &crate::ValueMap) {
        let _ = (name, variables);
    }
}

// ─────────────────────────────────────────────
// MemoryApplicationStore — in-process default
// ─────────────────────────────────────────────

/// In-process application store backed by `Mutex<HashMap>`. Matches the
/// pre-trait behaviour byte-for-byte for serve mode.
#[derive(Clone)]
pub struct MemoryApplicationStore {
    inner: Arc<Mutex<HashMap<String, ApplicationState>>>,
}

impl MemoryApplicationStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MemoryApplicationStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationStore for MemoryApplicationStore {
    fn get(&self, name: &str) -> Option<ApplicationState> {
        self.inner.lock().ok()?.get(name).cloned()
    }

    fn insert(&self, name: &str, state: ApplicationState) {
        if let Ok(mut m) = self.inner.lock() {
            m.insert(name.to_string(), state);
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.inner
            .lock()
            .ok()
            .map_or(false, |m| m.contains_key(name))
    }

    fn modify(&self, name: &str, f: &mut dyn FnMut(&mut ApplicationState)) {
        if let Ok(mut m) = self.inner.lock() {
            if let Some(state) = m.get_mut(name) {
                f(state);
            }
        }
    }
}
