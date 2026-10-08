//! socket.io-lucee compatibility layer — imperative server-side store
//! (Phase 3b).
//!
//! Where [`crate::websocket`] backs the *fluent* RustCFML API (one CFC =
//! one channel, convention lifecycle), this module backs the *imperative*
//! socket.io-lucee surface so a `preside-ext-socket-io`-style handler runs
//! unchanged:
//!
//! ```cfml
//! application.io = new SocketIoServer();
//! application.io.of( "/chat" ).on( "connect", function( socket ){
//!     socket.on( "say", function( msg ){ socket.broadcast( "said", msg ); } );
//! } );
//! ```
//!
//! The CFML CFCs (`SocketIoServer` / `SocketIoNamespace` / `SocketIoSocket`)
//! are a faithful port of socket.io-lucee, with its embedded-Java server
//! replaced by flat, VM-intercepted `$sio*` BIFs that talk to *this* store and
//! to the shared [`crate::websocket::WebSocketRegistry`]. Both surfaces ride
//! the one `/socket.io/` transport and the one registry — a given namespace is
//! owned by whichever surface registered it (the convention `websockets/*.cfc`
//! discovery is untouched).
//!
//! ## Why a store, and why it holds `Arc<BytecodeFunction>`
//!
//! The imperative model stores **closures** (the `connect` handler, and the
//! per-socket `on="event"` handlers it registers) and invokes them *later*,
//! from a fresh dispatch VM on the async transport thread. A stored
//! `CfmlValue::Function` carries its captured scope (an `Arc`) and a stable
//! `global_id`, but the actual [`cfml_codegen::compiler::BytecodeFunction`]
//! lives in a VM's `fn_registry`. So that a later, unrelated VM can resolve and
//! run the closure, we capture the set of reachable `BytecodeFunction` `Arc`s
//! at registration time (when a VM — the one running the `$sio*` BIF — has them
//! in scope) and re-register them into the dispatch VM before invoking. This is
//! exactly the mechanism serve mode already uses for application-scope closures
//! (`rehome_application_functions`).
//!
//! This module is **axum/tokio-free** (like [`crate::websocket`]) so the
//! `wasm32` builds stay green; it is simply inert there (nothing constructs a
//! `SocketIoServer`).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use cfml_codegen::compiler::BytecodeFunction;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use parking_lot::RwLock;

use crate::websocket::ConnId;

/// A stored CFML callback plus the `BytecodeFunction` `Arc`s reachable from it,
/// captured at registration time so a later dispatch VM can resolve it.
#[derive(Clone)]
pub struct Handler {
    /// The event name as the CFML registered it (original case). Lookups are
    /// case-insensitive, but socket.io event names on the wire are
    /// case-sensitive — so the transport subscribes with this exact string.
    pub event: String,
    pub callback: CfmlValue,
    pub fns: Vec<Arc<BytecodeFunction>>,
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handler")
            .field("fns", &self.fns.len())
            .finish()
    }
}

/// Namespace-level state: the `connect`/`disconnect`/`disconnecting` listeners
/// registered via `io.of(ns).on(event, cb)`.
#[derive(Debug, Default)]
struct NsState {
    handlers: HashMap<String, Handler>,
}

/// Per-connection state: which namespace it belongs to, the per-socket event
/// listeners registered via `socket.on(event, cb)` inside the connect handler,
/// and the `socketData` struct (kept here so it survives across the fresh
/// dispatch VMs that each inbound event spins up).
#[derive(Debug, Default)]
struct ConnState {
    ns: String,
    handlers: HashMap<String, Handler>,
    data: ValueMap,
}

/// The process-wide imperative socket.io store. One per server (there is one
/// transport and one [`crate::websocket::WebSocketRegistry`]), reached from the
/// `$sio*` BIFs (CFML side) and the socket.io transport (`crates/cli`).
#[derive(Debug, Default)]
pub struct SocketIoCompat {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Namespace key (lowercased, e.g. `/chat`) → its registered listeners.
    namespaces: HashMap<String, NsState>,
    /// Connection id → its per-socket listeners + data.
    conns: HashMap<ConnId, ConnState>,
    /// The socket.io-lucee **wrapper** CFC, when an app drives this store
    /// through `com.pixl8.socketiolucee.SocketIoServerWrapper` (Preside's
    /// socket-io extension) rather than through the engine's own
    /// `SocketIoServer`. That library keeps the whole server/namespace/socket
    /// model in CFML and expects a Java object underneath to (a) carry its
    /// outbound calls and (b) call four listener methods back on it. The
    /// handler CFC is that callback target; see `java_shims_socketio`.
    wrapper: Option<Handler>,
    /// What the wrapper's `getServerState()` reports.
    wrapper_state: Option<String>,
}

fn ns_key(ns: &str) -> String {
    let ns = ns.trim();
    let ns = if ns.is_empty() { "/" } else { ns };
    ns.to_lowercase()
}

impl SocketIoCompat {
    // ── socket.io-lucee wrapper (Java-shim surface) ───────────────────────

    /// Adopt the wrapper's handler CFC — the object the Java library would call
    /// `onConnect` / `onDisconnecting` / `onDisconnect` / `onSocketEvent` on.
    pub fn set_wrapper(&self, handler: Handler) {
        let mut inner = self.inner.write();
        inner.wrapper = Some(handler);
        inner.wrapper_state.get_or_insert_with(|| "NOTINITIALIZED".to_string());
    }

    pub fn wrapper(&self) -> Option<Handler> {
        self.inner.read().wrapper.clone()
    }

    pub fn set_wrapper_state(&self, state: &str) {
        self.inner.write().wrapper_state = Some(state.to_string());
    }

    pub fn wrapper_state(&self) -> String {
        self.inner
            .read()
            .wrapper_state
            .clone()
            .unwrap_or_else(|| "NOTINITIALIZED".to_string())
    }

    // ── namespaces ────────────────────────────────────────────────────────

    /// Mark a namespace as imperative-handled (idempotent). Called the moment
    /// `io.of(ns)` / `io.namespace(ns)` runs, so the transport can tell which
    /// surface owns an incoming connection.
    pub fn register_namespace(&self, ns: &str) {
        self.inner.write().namespaces.entry(ns_key(ns)).or_default();
    }

    /// Whether the namespace is owned by the imperative surface.
    pub fn is_imperative_ns(&self, ns: &str) -> bool {
        self.inner.read().namespaces.contains_key(&ns_key(ns))
    }

    /// Names of every registered namespace (original-case not preserved — keys
    /// are lowercased; socket.io namespaces are conventionally lowercase).
    pub fn registered_namespaces(&self) -> Vec<String> {
        self.inner.read().namespaces.keys().cloned().collect()
    }

    /// Store a namespace-level listener (`connect` / `disconnect` /
    /// `disconnecting`). Keyed case-insensitively by `handler.event`.
    pub fn set_ns_handler(&self, ns: &str, handler: Handler) {
        let mut inner = self.inner.write();
        let key = handler.event.to_lowercase();
        inner.namespaces.entry(ns_key(ns)).or_default().handlers.insert(key, handler);
    }

    /// Look up a namespace-level listener.
    pub fn ns_handler(&self, ns: &str, event: &str) -> Option<Handler> {
        self.inner
            .read()
            .namespaces
            .get(&ns_key(ns))
            .and_then(|n| n.handlers.get(&event.to_lowercase()).cloned())
    }

    // ── connections ───────────────────────────────────────────────────────

    /// Begin tracking a connection under its namespace.
    pub fn register_conn(&self, conn_id: &str, ns: &str) {
        self.inner.write().conns.insert(
            conn_id.to_string(),
            ConnState { ns: ns_key(ns), handlers: HashMap::new(), data: ValueMap::default() },
        );
    }

    /// Store a per-socket event listener (`socket.on(event, cb)`). Keyed
    /// case-insensitively by `handler.event`.
    pub fn set_socket_handler(&self, conn_id: &str, handler: Handler) {
        if let Some(c) = self.inner.write().conns.get_mut(conn_id) {
            c.handlers.insert(handler.event.to_lowercase(), handler);
        }
    }

    /// Look up a per-socket event listener.
    pub fn socket_handler(&self, conn_id: &str, event: &str) -> Option<Handler> {
        self.inner
            .read()
            .conns
            .get(conn_id)
            .and_then(|c| c.handlers.get(&event.to_lowercase()).cloned())
    }

    /// The event names a connection has registered listeners for, in their
    /// original (registered) case. The transport reads this *after* the connect
    /// handler runs to know which socket.io events to subscribe to (socketioxide
    /// 0.16 has no catch-all), and socket.io event names are case-sensitive.
    pub fn socket_events(&self, conn_id: &str) -> Vec<String> {
        self.inner
            .read()
            .conns
            .get(conn_id)
            .map(|c| c.handlers.values().map(|h| h.event.clone()).collect())
            .unwrap_or_default()
    }

    /// The namespace a connection belongs to.
    pub fn conn_ns(&self, conn_id: &str) -> Option<String> {
        self.inner.read().conns.get(conn_id).map(|c| c.ns.clone())
    }

    /// Forget a connection (its per-socket listeners + data). Namespace-level
    /// listeners persist for the next connection.
    pub fn drop_conn(&self, conn_id: &str) {
        self.inner.write().conns.remove(conn_id);
    }

    // ── socketData ────────────────────────────────────────────────────────

    /// The connection's `socketData` struct (empty if unset / unknown conn).
    pub fn get_data(&self, conn_id: &str) -> ValueMap {
        self.inner
            .read()
            .conns
            .get(conn_id)
            .map(|c| c.data.clone())
            .unwrap_or_default()
    }

    /// Replace the connection's `socketData` struct.
    pub fn set_data(&self, conn_id: &str, data: ValueMap) {
        if let Some(c) = self.inner.write().conns.get_mut(conn_id) {
            c.data = data;
        }
    }
}

/// The process-wide store. There is exactly one transport + registry per
/// server process, so a single shared instance is the natural home; the bare
/// `fn`-pointer native constructors and the async transport both reach it here.
static COMPAT: OnceLock<Arc<SocketIoCompat>> = OnceLock::new();

/// Access the process-wide imperative socket.io store, creating it on first
/// use. Inert (just an empty store) until the CFML side registers a namespace.
pub fn compat() -> Arc<SocketIoCompat> {
    COMPAT.get_or_init(|| Arc::new(SocketIoCompat::default())).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(event: &str) -> Handler {
        Handler { event: event.to_string(), callback: CfmlValue::Null, fns: Vec::new() }
    }

    #[test]
    fn namespace_registration_is_case_insensitive_and_idempotent() {
        let c = SocketIoCompat::default();
        assert!(!c.is_imperative_ns("/Chat"));
        c.register_namespace("/Chat");
        assert!(c.is_imperative_ns("/chat"));
        assert!(c.is_imperative_ns("/CHAT"));
        c.register_namespace("/chat"); // idempotent
        assert_eq!(c.registered_namespaces().len(), 1);
    }

    #[test]
    fn ns_and_socket_handlers_round_trip() {
        let c = SocketIoCompat::default();
        c.set_ns_handler("/chat", handler("connect"));
        assert!(c.ns_handler("/chat", "connect").is_some());
        assert!(c.ns_handler("/chat", "disconnect").is_none());

        c.register_conn("n1:abc", "/chat");
        // Registered camelCase, looked up case-insensitively, surfaced original.
        c.set_socket_handler("n1:abc", handler("joinRoom"));
        assert!(c.socket_handler("n1:abc", "joinroom").is_some());
        assert_eq!(c.socket_events("n1:abc"), vec!["joinRoom".to_string()]);
        assert_eq!(c.conn_ns("n1:abc").as_deref(), Some("/chat"));
    }

    #[test]
    fn data_persists_until_conn_dropped() {
        let c = SocketIoCompat::default();
        c.register_conn("n1:abc", "/chat");
        let mut d = ValueMap::default();
        d.insert("userId".to_string(), CfmlValue::Int(42));
        c.set_data("n1:abc", d);
        assert!(matches!(c.get_data("n1:abc").get("userId"), Some(CfmlValue::Int(42))));
        c.drop_conn("n1:abc");
        assert!(c.get_data("n1:abc").is_empty());
        assert!(c.socket_handler("n1:abc", "say").is_none());
    }
}
