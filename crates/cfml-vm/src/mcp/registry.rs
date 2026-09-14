//! The MCP session registry — the cross-request home for everything a live
//! MCP conversation needs: sessions, their open streams, per-stream event ids
//! and replay buffers, and the table of server→client requests waiting on an
//! answer.
//!
//! Like `websocket.rs`, this module is deliberately **axum/tokio-free**: the
//! only contact with the async world is the [`McpSink`] trait, which the
//! transport in `crates/cli` implements over a bounded channel. It is also
//! **clock-free** — every method that needs "now" takes it as a millisecond
//! argument from the caller, so `cfml-vm` keeps building for `wasm32`, where
//! `std::time::Instant` is unavailable.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::{Condvar, Mutex, RwLock};
use serde_json::Value;

use super::protocol::{Outgoing, RpcId};

/// An opaque, per-conversation session id. Visible ASCII only (the spec pins
/// 0x21–0x7E) because it travels in the `Mcp-Session-Id` HTTP header.
pub type SessionId = String;

/// Ordinal of a stream within its session. Event ids are rendered
/// `"{stream}-{n}"` so a `Last-Event-ID` can be resolved back to the stream
/// that issued it — resumption is per-stream, and replaying another stream's
/// events is an explicit spec violation.
pub type StreamOrd = u64;

/// The single outbound primitive. Implementations must be non-blocking: the
/// registry is called from synchronous VM code on a blocking worker.
pub trait McpSink: Send + Sync + std::fmt::Debug {
    /// Enqueue one SSE event. Returns `false` when the peer is gone, which
    /// tells the registry to mark the stream closed and stop writing.
    fn send(&self, event_id: &str, payload: &str) -> bool;
    /// Ask the transport to end the stream.
    fn close(&self);
}

/// Why a stream exists. The distinction drives two spec rules: a
/// request-scoped stream carries the response to its originating request and
/// then closes, whereas the standalone stream carries only server-initiated
/// traffic and must never carry a response except on resumption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamKind {
    /// Opened by `GET` — server→client notifications and requests.
    Standalone,
    /// Opened by a `POST` carrying a request, and closed after its response.
    RequestScoped { request: RpcId },
}

/// One open (or recently-closed-but-resumable) SSE stream.
#[derive(Debug)]
pub struct StreamState {
    pub ord: StreamOrd,
    pub kind: StreamKind,
    pub sink: Arc<dyn McpSink>,
    next_event: u64,
    /// Ring of `(event id, serialized message)` retained for `Last-Event-ID`
    /// resumption. Bounded — a disconnected client must not be able to pin
    /// memory indefinitely.
    replay: VecDeque<(u64, String)>,
    replay_cap: usize,
    pub closed: bool,
}

impl StreamState {
    fn event_id(&self, n: u64) -> String {
        format!("{}-{}", self.ord, n)
    }
}

/// Rank a syslog-style MCP log level. An unrecognised name sorts as `info`,
/// so a typo in a handler never silently drops every message.
pub fn severity(level: &str) -> u8 {
    match level.to_lowercase().as_str() {
        "debug" => 0,
        "info" => 1,
        "notice" => 2,
        "warning" => 3,
        "error" => 4,
        "critical" => 5,
        "alert" => 6,
        "emergency" => 7,
        _ => 1,
    }
}

/// Parse an event id back into `(stream ordinal, sequence)`.
pub fn parse_event_id(raw: &str) -> Option<(StreamOrd, u64)> {
    let (s, n) = raw.split_once('-')?;
    Some((s.parse().ok()?, n.parse().ok()?))
}

/// A server→client request parked waiting for the client's response, which
/// arrives on a *different* HTTP POST. The CFML thread that issued it is
/// blocked on the condvar inside.
#[derive(Debug, Default)]
pub struct ResponseSlot {
    state: Mutex<Option<Result<Value, String>>>,
    ready: Condvar,
}

impl ResponseSlot {
    /// Park until the client answers or `timeout_ms` elapses. Returns `Err`
    /// on a client-reported error, a timeout, or session teardown — never
    /// hangs forever, because a parked slot holds a blocking-pool thread that
    /// ordinary HTTP requests also need.
    pub fn wait(&self, timeout_ms: u64) -> Result<Value, String> {
        let mut guard = self.state.lock();
        if guard.is_none() {
            let timed_out = self
                .ready
                .wait_for(&mut guard, std::time::Duration::from_millis(timeout_ms))
                .timed_out();
            if timed_out && guard.is_none() {
                return Err("Timed out waiting for the MCP client to respond".to_string());
            }
        }
        guard.take().unwrap_or_else(|| Err("Request was abandoned".to_string()))
    }

    /// Deliver the client's answer and wake the parked thread.
    pub fn fulfil(&self, outcome: Result<Value, String>) {
        *self.state.lock() = Some(outcome);
        self.ready.notify_all();
    }
}

/// What the client told us it can do, from `initialize`. Gates server→client
/// requests: asking a client without `sampling` to sample must fail fast
/// rather than park a thread until timeout.
#[derive(Clone, Debug, Default)]
pub struct ClientCapabilities {
    pub sampling: bool,
    pub elicitation: bool,
    pub roots: bool,
}

impl ClientCapabilities {
    pub fn from_json(v: &Value) -> Self {
        Self {
            sampling: v.get("sampling").is_some(),
            elicitation: v.get("elicitation").is_some(),
            roots: v.get("roots").is_some(),
        }
    }
}

/// One MCP conversation.
#[derive(Debug)]
pub struct McpSession {
    pub id: SessionId,
    /// Which server CFC this session is talking to (`/mcp/<name>`).
    pub server: String,
    pub protocol_version: String,
    pub client_info: Value,
    pub capabilities: ClientCapabilities,
    /// Set by `notifications/initialized`. Requests other than `ping` before
    /// this are tolerated but noted — some clients are sloppy about ordering.
    pub initialized: bool,
    pub last_seen_ms: u64,
    /// Minimum severity the client wants, from `logging/setLevel`. `None`
    /// until it asks, which means "send everything" — a client that has not
    /// expressed a preference should not silently lose messages.
    pub log_level: Option<String>,
    streams: HashMap<StreamOrd, StreamState>,
    next_stream: StreamOrd,
    pending: HashMap<RpcId, Arc<ResponseSlot>>,
}

impl McpSession {
    pub fn stream_count(&self) -> usize {
        self.streams.values().filter(|s| !s.closed).count()
    }

    pub fn has_standalone(&self) -> bool {
        self.streams
            .values()
            .any(|s| !s.closed && s.kind == StreamKind::Standalone)
    }
}

/// Cross-request MCP state. Lives on `ServerState`.
#[derive(Debug)]
pub struct McpRegistry {
    node_id: String,
    sessions: RwLock<HashMap<SessionId, Arc<Mutex<McpSession>>>>,
    seq: AtomicU64,
}

impl McpRegistry {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            sessions: RwLock::new(HashMap::new()),
            seq: AtomicU64::new(0),
        }
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Mint a session id. Node-qualified like connection ids, so a future
    /// clustered deployment can route a session back to its owning node
    /// without a wire change.
    pub fn new_session_id(&self) -> SessionId {
        format!("{}.{}", self.node_id.replace('-', ""), uuid::Uuid::new_v4().simple())
    }

    pub fn create_session(
        &self,
        server: &str,
        protocol_version: &str,
        client_info: Value,
        capabilities: ClientCapabilities,
        now_ms: u64,
    ) -> SessionId {
        let id = self.new_session_id();
        let session = McpSession {
            id: id.clone(),
            server: server.to_string(),
            protocol_version: protocol_version.to_string(),
            client_info,
            capabilities,
            initialized: false,
            last_seen_ms: now_ms,
            log_level: None,
            streams: HashMap::new(),
            next_stream: 0,
            pending: HashMap::new(),
        };
        self.sessions.write().insert(id.clone(), Arc::new(Mutex::new(session)));
        id
    }

    /// Record the client's `logging/setLevel` choice. Returns false for an
    /// unknown session.
    pub fn set_log_level(&self, session_id: &str, level: &str) -> bool {
        match self.get(session_id) {
            Some(s) => {
                s.lock().log_level = Some(level.to_lowercase());
                true
            }
            None => false,
        }
    }

    /// Should a message of this severity be sent to the session?
    pub fn logs_at(&self, session_id: &str, level: &str) -> bool {
        let Some(session) = self.get(session_id) else { return true };
        let minimum = session.lock().log_level.clone();
        match minimum {
            Some(minimum) => severity(level) >= severity(&minimum),
            None => true,
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<Mutex<McpSession>>> {
        self.sessions.read().get(id).cloned()
    }

    /// Mark a session alive. Returns false when the id is unknown — the HTTP
    /// layer turns that into a 404, which tells the client to re-initialize.
    pub fn touch(&self, id: &str, now_ms: u64) -> bool {
        match self.get(id) {
            Some(s) => {
                s.lock().last_seen_ms = now_ms;
                true
            }
            None => false,
        }
    }

    /// End a session: close every stream and fail every parked request so no
    /// blocking thread is left orphaned. Waking the pending slots *before*
    /// dropping the session is the whole point of doing this in one place.
    pub fn end_session(&self, id: &str) -> bool {
        let Some(session) = self.sessions.write().remove(id) else {
            return false;
        };
        let mut session = session.lock();
        for slot in session.pending.values() {
            slot.fulfil(Err("MCP session ended".to_string()));
        }
        session.pending.clear();
        for stream in session.streams.values_mut() {
            stream.closed = true;
            stream.sink.close();
        }
        true
    }

    pub fn session_ids(&self, server: &str) -> Vec<SessionId> {
        self.sessions
            .read()
            .values()
            .filter_map(|s| {
                let s = s.lock();
                (s.server == server).then(|| s.id.clone())
            })
            .collect()
    }

    /// Register a new stream on a session and return its ordinal.
    pub fn attach_stream(
        &self,
        session_id: &str,
        kind: StreamKind,
        sink: Arc<dyn McpSink>,
        replay_cap: usize,
    ) -> Option<StreamOrd> {
        let session = self.get(session_id)?;
        let mut session = session.lock();
        let ord = session.next_stream;
        session.next_stream += 1;
        session.streams.insert(
            ord,
            StreamState {
                ord,
                kind,
                sink,
                next_event: 0,
                replay: VecDeque::new(),
                replay_cap,
                closed: false,
            },
        );
        Some(ord)
    }

    /// Close one stream, keeping its replay ring so a client that reconnects
    /// with `Last-Event-ID` can still catch up.
    pub fn close_stream(&self, session_id: &str, ord: StreamOrd) {
        if let Some(session) = self.get(session_id) {
            if let Some(stream) = session.lock().streams.get_mut(&ord) {
                stream.closed = true;
                stream.sink.close();
            }
        }
    }

    /// Write one message to a specific stream. Returns false when the stream
    /// is gone or the peer has disconnected.
    pub fn send_on(&self, session_id: &str, ord: StreamOrd, msg: &Outgoing) -> bool {
        let Some(session) = self.get(session_id) else { return false };
        let mut session = session.lock();
        let Some(stream) = session.streams.get_mut(&ord) else { return false };
        if stream.closed {
            return false;
        }
        let n = stream.next_event;
        stream.next_event += 1;
        let event_id = stream.event_id(n);
        let payload = msg.to_line();
        stream.replay.push_back((n, payload.clone()));
        while stream.replay.len() > stream.replay_cap {
            stream.replay.pop_front();
        }
        if !stream.sink.send(&event_id, &payload) {
            stream.closed = true;
            return false;
        }
        true
    }

    /// Deliver a server-initiated notification to a session's standalone
    /// stream. This is the `mcpNotify()` / `mcp().toolsChanged()` path, and it
    /// is legitimately a no-delivery when nobody is listening on a GET stream.
    pub fn notify_session(&self, session_id: &str, msg: &Outgoing) -> bool {
        let Some(session) = self.get(session_id) else { return false };
        let ord = {
            let session = session.lock();
            session
                .streams
                .values()
                .find(|s| !s.closed && s.kind == StreamKind::Standalone)
                .map(|s| s.ord)
        };
        match ord {
            Some(ord) => self.send_on(session_id, ord, msg),
            None => false,
        }
    }

    /// Fan a notification out to every session of one server. Returns how many
    /// sessions actually received it.
    pub fn notify_server(&self, server: &str, msg: &Outgoing) -> usize {
        self.session_ids(server)
            .iter()
            .filter(|id| self.notify_session(id, msg))
            .count()
    }

    /// Events strictly newer than `last_event_id`, for resumption. Returns
    /// `None` when the id names a stream we no longer hold; the caller then
    /// starts a fresh stream rather than silently delivering nothing.
    pub fn replay_since(
        &self,
        session_id: &str,
        last_event_id: &str,
    ) -> Option<(StreamOrd, Vec<(String, String)>)> {
        let (ord, after) = parse_event_id(last_event_id)?;
        let session = self.get(session_id)?;
        let session = session.lock();
        let stream = session.streams.get(&ord)?;
        let events = stream
            .replay
            .iter()
            .filter(|(n, _)| *n > after)
            .map(|(n, payload)| (stream.event_id(*n), payload.clone()))
            .collect();
        Some((ord, events))
    }

    // ── server→client requests (sampling / elicitation / roots) ───────────

    /// Mint the id for a server-initiated request. Node-qualified and
    /// monotonic, so ids never collide across sessions or nodes.
    pub fn new_request_id(&self) -> RpcId {
        RpcId::Str(format!("{}:{}", self.node_id, self.next_seq()))
    }

    /// Park a slot for a server→client request. `max_pending` caps how many
    /// threads a single session can hold blocked — without it a client that
    /// never answers can exhaust the shared blocking pool and stall every
    /// ordinary HTTP request on the server.
    pub fn register_pending(
        &self,
        session_id: &str,
        id: RpcId,
        max_pending: usize,
    ) -> Result<Arc<ResponseSlot>, String> {
        let session = self.get(session_id).ok_or("Unknown MCP session")?;
        let mut session = session.lock();
        if session.pending.len() >= max_pending {
            return Err(format!(
                "Too many outstanding MCP requests for this session (limit {max_pending})"
            ));
        }
        let slot = Arc::new(ResponseSlot::default());
        session.pending.insert(id, slot.clone());
        Ok(slot)
    }

    /// Route a client's response back to the thread waiting on it. Returns
    /// false when no such request is outstanding (a late or duplicate answer).
    pub fn resolve_pending(
        &self,
        session_id: &str,
        id: &RpcId,
        outcome: Result<Value, String>,
    ) -> bool {
        let Some(session) = self.get(session_id) else { return false };
        let slot = session.lock().pending.remove(id);
        match slot {
            Some(slot) => {
                slot.fulfil(outcome);
                true
            }
            None => false,
        }
    }

    pub fn forget_pending(&self, session_id: &str, id: &RpcId) {
        if let Some(session) = self.get(session_id) {
            session.lock().pending.remove(id);
        }
    }

    /// Drop sessions idle for longer than `idle_ms`, waking anything parked on
    /// them. Returns the ids reaped so the caller can fire `onSessionEnd`.
    pub fn reap(&self, now_ms: u64, idle_ms: u64) -> Vec<SessionId> {
        let stale: Vec<SessionId> = self
            .sessions
            .read()
            .values()
            .filter_map(|s| {
                let s = s.lock();
                now_ms.saturating_sub(s.last_seen_ms).ge(&idle_ms).then(|| s.id.clone())
            })
            .collect();
        for id in &stale {
            self.end_session(id);
        }
        stale
    }

    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new(uuid::Uuid::new_v4().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Test sink that records what was written, and can pretend the peer went
    /// away — the MCP analogue of the WebSocket suite's `CapturingSink`.
    #[derive(Debug, Default)]
    struct CapturingSink {
        events: Mutex<Vec<(String, String)>>,
        dead: std::sync::atomic::AtomicBool,
        closed: std::sync::atomic::AtomicBool,
    }

    impl McpSink for CapturingSink {
        fn send(&self, event_id: &str, payload: &str) -> bool {
            if self.dead.load(Ordering::Relaxed) {
                return false;
            }
            self.events.lock().push((event_id.to_string(), payload.to_string()));
            true
        }
        fn close(&self) {
            self.closed.store(true, Ordering::Relaxed);
        }
    }

    fn registry() -> McpRegistry {
        McpRegistry::new("node1")
    }

    fn session(reg: &McpRegistry) -> SessionId {
        reg.create_session("/mcp/demo", "2025-06-18", json!({}), ClientCapabilities::default(), 0)
    }

    fn note(n: &str) -> Outgoing {
        Outgoing::Notification { method: n.to_string(), params: json!({}) }
    }

    #[test]
    fn session_ids_are_visible_ascii() {
        let reg = registry();
        let id = session(&reg);
        assert!(id.bytes().all(|b| (0x21..=0x7E).contains(&b)), "header-safe: {id}");
    }

    #[test]
    fn events_carry_per_stream_ids_and_replay_from_a_cursor() {
        let reg = registry();
        let id = session(&reg);
        let sink = Arc::new(CapturingSink::default());
        let ord = reg.attach_stream(&id, StreamKind::Standalone, sink.clone(), 10).unwrap();

        for _ in 0..3 {
            assert!(reg.send_on(&id, ord, &note("notifications/message")));
        }
        let seen: Vec<String> = sink.events.lock().iter().map(|(e, _)| e.clone()).collect();
        assert_eq!(seen, vec!["0-0", "0-1", "0-2"]);

        let (replay_ord, events) = reg.replay_since(&id, "0-0").unwrap();
        assert_eq!(replay_ord, ord);
        assert_eq!(events.len(), 2, "only events strictly after the cursor");
        assert_eq!(events[0].0, "0-1");
    }

    #[test]
    fn replay_never_crosses_streams() {
        let reg = registry();
        let id = session(&reg);
        let a = Arc::new(CapturingSink::default());
        let b = Arc::new(CapturingSink::default());
        let ord_a = reg.attach_stream(&id, StreamKind::Standalone, a, 10).unwrap();
        let ord_b = reg
            .attach_stream(&id, StreamKind::RequestScoped { request: RpcId::Num(1) }, b, 10)
            .unwrap();
        assert_ne!(ord_a, ord_b);
        reg.send_on(&id, ord_a, &note("a"));
        reg.send_on(&id, ord_b, &note("b"));

        let (_, events) = reg.replay_since(&id, &format!("{ord_b}-0")).unwrap();
        assert!(events.is_empty(), "stream b had nothing after its own event 0");
        assert!(reg.replay_since(&id, "99-0").is_none(), "unknown stream → no replay");
        assert!(reg.replay_since(&id, "garbage").is_none());
    }

    #[test]
    fn the_replay_ring_is_bounded() {
        let reg = registry();
        let id = session(&reg);
        let ord = reg
            .attach_stream(&id, StreamKind::Standalone, Arc::new(CapturingSink::default()), 2)
            .unwrap();
        for _ in 0..5 {
            reg.send_on(&id, ord, &note("x"));
        }
        let (_, events) = reg.replay_since(&id, "0-0").unwrap();
        assert_eq!(events.len(), 2, "a disconnected client cannot pin memory");
    }

    #[test]
    fn a_dead_peer_marks_the_stream_closed() {
        let reg = registry();
        let id = session(&reg);
        let sink = Arc::new(CapturingSink::default());
        let ord = reg.attach_stream(&id, StreamKind::Standalone, sink.clone(), 10).unwrap();
        sink.dead.store(true, Ordering::Relaxed);
        assert!(!reg.send_on(&id, ord, &note("x")));
        assert!(!reg.send_on(&id, ord, &note("x")), "stays closed");
    }

    #[test]
    fn notify_only_reaches_a_standalone_stream() {
        let reg = registry();
        let id = session(&reg);
        let sink = Arc::new(CapturingSink::default());
        reg.attach_stream(&id, StreamKind::RequestScoped { request: RpcId::Num(1) }, sink, 10);
        assert!(!reg.notify_session(&id, &note("x")), "a request stream is not for broadcasts");

        let standalone = Arc::new(CapturingSink::default());
        reg.attach_stream(&id, StreamKind::Standalone, standalone.clone(), 10);
        assert!(reg.notify_session(&id, &note("x")));
        assert_eq!(standalone.events.lock().len(), 1);
        assert_eq!(reg.notify_server("/mcp/demo", &note("x")), 1);
        assert_eq!(reg.notify_server("/mcp/other", &note("x")), 0);
    }

    #[test]
    fn pending_requests_resolve_and_are_capped() {
        let reg = registry();
        let id = session(&reg);
        let rid = reg.new_request_id();
        let slot = reg.register_pending(&id, rid.clone(), 2).unwrap();

        let reg2 = Arc::new(reg);
        let inner = reg2.clone();
        let sid = id.clone();
        let rid2 = rid.clone();
        std::thread::spawn(move || {
            inner.resolve_pending(&sid, &rid2, Ok(json!({ "ok": true })));
        });
        assert_eq!(slot.wait(5_000).unwrap()["ok"], true);

        reg2.register_pending(&id, reg2.new_request_id(), 2).unwrap();
        reg2.register_pending(&id, reg2.new_request_id(), 2).unwrap();
        assert!(
            reg2.register_pending(&id, reg2.new_request_id(), 2).is_err(),
            "cap protects the blocking pool"
        );
    }

    #[test]
    fn a_parked_request_times_out_rather_than_hanging() {
        let reg = registry();
        let id = session(&reg);
        let slot = reg.register_pending(&id, reg.new_request_id(), 4).unwrap();
        assert!(slot.wait(10).unwrap_err().contains("Timed out"));
    }

    #[test]
    fn ending_a_session_wakes_everything_parked_on_it() {
        let reg = Arc::new(registry());
        let id = session(&reg);
        let slot = reg.register_pending(&id, reg.new_request_id(), 4).unwrap();
        let sink = Arc::new(CapturingSink::default());
        reg.attach_stream(&id, StreamKind::Standalone, sink.clone(), 10);

        let inner = reg.clone();
        let sid = id.clone();
        std::thread::spawn(move || inner.end_session(&sid));
        // Would hang forever if teardown did not fail the slot.
        assert!(slot.wait(5_000).is_err());
        assert!(sink.closed.load(Ordering::Relaxed));
        assert!(reg.get(&id).is_none());
        assert!(!reg.touch(&id, 1), "an unknown session is a 404, not a revival");
    }

    #[test]
    fn reaping_drops_only_idle_sessions() {
        let reg = registry();
        let stale = reg.create_session("/mcp/demo", "2025-06-18", json!({}), Default::default(), 0);
        let fresh = reg.create_session("/mcp/demo", "2025-06-18", json!({}), Default::default(), 0);
        reg.touch(&fresh, 10_000);

        let reaped = reg.reap(10_000, 5_000);
        assert_eq!(reaped, vec![stale]);
        assert_eq!(reg.session_count(), 1);
        assert!(reg.get(&fresh).is_some());
    }

    #[test]
    fn log_level_filtering_follows_the_clients_choice() {
        let reg = registry();
        let id = session(&reg);
        // No preference expressed yet: everything is sent, rather than a
        // default silently swallowing a handler's debug output.
        assert!(reg.logs_at(&id, "debug"));

        assert!(reg.set_log_level(&id, "warning"));
        assert!(!reg.logs_at(&id, "debug"));
        assert!(!reg.logs_at(&id, "info"));
        assert!(reg.logs_at(&id, "warning"));
        assert!(reg.logs_at(&id, "error"));
        assert!(reg.logs_at(&id, "emergency"));
        // An unrecognised level is treated as `info` rather than dropped.
        assert!(!reg.logs_at(&id, "chatty"));
        assert!(!reg.set_log_level("no-such-session", "debug"));
    }

    #[test]
    fn client_capabilities_come_from_initialize() {
        let caps = ClientCapabilities::from_json(&json!({ "sampling": {}, "roots": { "listChanged": true } }));
        assert!(caps.sampling && caps.roots && !caps.elicitation);
    }
}
