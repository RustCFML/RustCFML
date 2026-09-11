//! WebSocket connection driver (axum side, native-only).
//!
//! Bridges the long-lived async WebSocket to the synchronous VM, exactly as an
//! HTTP request does: each inbound frame is dispatched on a `spawn_blocking`
//! worker that builds a fresh VM, instantiates the channel CFC, and calls the
//! lifecycle method (`onConnect`/`onMessage`/`onDisconnect`/`onError`). Outbound
//! frames flow the other way through a bounded `mpsc` so VM code (and any
//! `wsPublish`/`io()` from another request) can push to this connection without
//! ever touching the socket directly.
//!
//! The registry (`cfml-vm/src/websocket.rs`) is transport-agnostic; this module
//! is the only place that knows about axum/tokio. See `docs/websocket-design.md`.

use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use cfml_common::dynamic::CfmlValue;
use cfml_vm::websocket::{FrameSink, SocketHandle, WebSocketRegistry, WireEnvelope};
use cfml_vm::CfmlVirtualMachine;
use futures_util::{SinkExt, StreamExt};

use crate::AppState;

/// Bounded outbound queue depth per connection. On overflow the connection is
/// closed rather than the server blocking (design principle P9 — the engine
/// owns backpressure; a slow client cannot stall a handler).
const OUTBOUND_QUEUE: usize = 256;

/// What the registry pushes toward a connection; drained by the per-connection
/// pump task and written to the socket sink.
enum SinkCmd {
    Frame(Box<WireEnvelope>),
    Close(u16, String),
}

/// The [`FrameSink`] handed to the registry for one connection. Non-blocking:
/// `try_send` drops to a close on overflow.
#[derive(Debug)]
struct ChannelSink {
    tx: tokio::sync::mpsc::Sender<SinkCmd>,
}

impl FrameSink for ChannelSink {
    fn send(&self, frame: WireEnvelope) {
        if self.tx.try_send(SinkCmd::Frame(Box::new(frame))).is_err() {
            // Queue full or pump gone → ask the pump to close (best-effort).
            let _ = self.tx.try_send(SinkCmd::Close(1011, "outbound overflow".into()));
        }
    }
    fn close(&self, code: u16, reason: String) {
        let _ = self.tx.try_send(SinkCmd::Close(code, reason));
    }
}

/// Per-channel resolution: the CFC file backing `/ws/<name>` and whether it
/// declared `encoding="json"`. Shared with the socket.io transport
/// (`crate::socketio`), hence `pub(crate)`.
#[derive(Clone)]
pub(crate) struct ChannelInfo {
    /// Wire/registry channel id, e.g. `/chat`.
    pub(crate) channel: String,
    /// Absolute path to the channel CFC.
    pub(crate) cfc_path: String,
    /// `encoding="json"` → inbound text is parsed to a struct before dispatch.
    pub(crate) json: bool,
    /// `history="N"` → retain the last N channel-wide frames for `lastEventId`
    /// resumability. 0 = disabled (no retention, no replay).
    pub(crate) history: usize,
    /// Named events declared via the `function … on="event"` annotation. The
    /// socket.io transport registers a `socket.on(<event>)` handler for each
    /// (it has no catch-all); the raw-WS transport ignores this (it routes any
    /// inbound `ev` dynamically). Always includes the conventional `"message"`.
    pub(crate) events: Vec<String>,
    /// Whisper / client-event names declared via the `clientEvents="a,b"`
    /// component attribute (Phase 4). The socket.io transport registers a
    /// relay-only `socket.on(client-<name>)` for each (it has no catch-all); the
    /// raw-WS transport ignores this (it relays any inbound `client-*` event
    /// dynamically). Each name is normalised to its full `client-…` wire form.
    pub(crate) client_events: Vec<String>,
}

/// Resolve `/ws/<name>` to a channel CFC under `<docroot>/websockets/`.
/// Convention discovery (Lucee style); the explicit `component socket="…"`
/// attribute is honoured for the wire channel id when present.
pub(crate) fn resolve_channel(state: &AppState, name: &str) -> Option<ChannelInfo> {
    // Strip any leading slash a client may send; channel id is normalised to
    // a single leading slash.
    let name = name.trim_start_matches('/');
    if name.is_empty() || name.contains("..") || name.contains('/') {
        return None;
    }
    let path = state
        .doc_root
        .join("websockets")
        .join(format!("{}.cfc", name));
    if !state.vfs.exists(&path.to_string_lossy()) {
        return None;
    }
    let cfc_path = path.to_string_lossy().to_string();
    // Cheap source scan for the channel attributes (avoids spinning a VM just
    // to read metadata at handshake). `socket="/foo"` overrides the wire id;
    // `encoding="json"` flips inbound parsing.
    let (channel, json, history, events, client_events) = match state.vfs.read(&cfc_path) {
        Ok(bytes) => {
            let src = String::from_utf8_lossy(&bytes);
            let lower = src.to_lowercase();
            let json = lower.contains("encoding=\"json\"") || lower.contains("encoding='json'");
            let channel = extract_attr(&src, "socket")
                .unwrap_or_else(|| format!("/{}", name.to_lowercase()));
            // `history="N"` opts the channel into resumability; non-numeric or
            // absent leaves it disabled.
            let history = extract_attr(&src, "history")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            (channel, json, history, extract_event_names(&src), extract_client_events(&src))
        }
        Err(_) => (format!("/{}", name.to_lowercase()), false, 0, Vec::new(), Vec::new()),
    };
    Some(ChannelInfo { channel, cfc_path, json, history, events, client_events })
}

/// Scan CFC source for the named events declared via the
/// `function … on="event"` handler annotation. The socket.io transport needs
/// the concrete set up front because socketioxide 0.16 has no catch-all event
/// handler — it registers one `socket.on(<event>)` per name. Always returns the
/// conventional `"message"` (mapped to `onMessage` with no event) so a stock
/// `socket.emit("message", …)` works. Lowercased + de-duplicated.
pub(crate) fn extract_event_names(src: &str) -> Vec<String> {
    let mut events = vec!["message".to_string()];
    // `\bon\s*=\s*["']…["']` — the `\b` keeps `encoding=`/`position=` etc. from
    // matching (the `on` inside them is not on a word boundary).
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?i)\bon\s*=\s*["']([^"']+)["']"#).expect("valid event-name regex")
    });
    for cap in re.captures_iter(src) {
        let ev = cap[1].trim().to_lowercase();
        if !ev.is_empty() && !events.contains(&ev) {
            events.push(ev);
        }
    }
    events
}

/// Scan CFC source for whisper / client-event names declared via the
/// `clientEvents="typing,cursor"` component attribute (Phase 4). socket.io has
/// no catch-all, so the engine must subscribe to the concrete relay events up
/// front. Names are comma-split, lowercased, and normalised to the full
/// `client-…` wire form (so both `"typing"` and `"client-typing"` work). The
/// raw-WS transport ignores this list — it relays any inbound `client-*` event.
pub(crate) fn extract_client_events(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(raw) = extract_attr(src, "clientEvents") else { return out };
    for name in raw.split(',') {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        let full =
            if name.starts_with("client-") { name } else { format!("client-{}", name) };
        if !out.contains(&full) {
            out.push(full);
        }
    }
    out
}

/// Pull the value of a `name="…"` component attribute out of CFC source. Used
/// at handshake time to read channel metadata (`socket=`, `history=`) without
/// spinning up a VM.
fn extract_attr(src: &str, name: &str) -> Option<String> {
    let needle = format!("{}=", name.to_lowercase());
    let lower = src.to_lowercase();
    let pos = lower.find(&needle)?;
    let rest = &src[pos + needle.len()..];
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)?;
    let val = &rest[1..1 + end];
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

/// Read the CFID session id out of the request cookies (handshake-time
/// identity, design principle P6).
pub(crate) fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get("cookie")?.to_str().ok()?;
    cookie.split(';').find_map(|c| {
        let c = c.trim();
        c.strip_prefix("CFID=").map(|v| v.to_string())
    })
}

/// axum handler for `GET /ws/{channel}`. Upgrades the connection and hands it
/// to the driver. A 404 is returned for an unknown channel (no CFC).
pub async fn ws_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let info = match resolve_channel(&state, &name) {
        Some(i) => i,
        None => return (axum::http::StatusCode::NOT_FOUND, "Unknown channel").into_response(),
    };
    let session_id = session_id_from_headers(&headers);
    let params = parse_query(query.as_deref());
    ws.on_upgrade(move |socket| driver(socket, state, info, session_id, params))
}

/// Parse a raw query string (`a=1&b=hi%20there`) into a CFML struct for
/// `socket.param(name)`. Minimal percent / `+` decoding — handshake params are
/// simple key/value pairs.
pub(crate) fn parse_query(query: Option<&str>) -> cfml_common::dynamic::ValueMap {
    let mut map = cfml_common::dynamic::ValueMap::default();
    let Some(q) = query else { return map };
    for pair in q.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(url_decode(k), CfmlValue::string(url_decode(v)));
    }
    map
}

fn url_decode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One tokio task per connection: registers it, runs the `onConnect` reject
/// gate, pumps inbound frames into blocking dispatches, and tears down (fire
/// `onDisconnect`, leave all rooms) on close — unconditionally (P10).
async fn driver(
    socket: WebSocket,
    state: Arc<AppState>,
    info: ChannelInfo,
    session_id: Option<String>,
    params: cfml_common::dynamic::ValueMap,
) {
    let registry = state.server_state.websocket.clone();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<SinkCmd>(OUTBOUND_QUEUE);
    let frame_sink: Arc<dyn FrameSink> = Arc::new(ChannelSink { tx });
    let conn_id = registry.register(&info.channel, frame_sink, session_id.clone(), params);
    // Opt the channel into resumability history (no-op when history=0).
    registry.set_history_cap(&info.channel, info.history);

    // Outbound pump: drain the queue, serialize, write to the socket.
    let pump = tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                SinkCmd::Frame(frame) => {
                    // A raw `socket.send(binaryValue)` (no event name, binary
                    // payload) goes out as a binary frame; everything else is a
                    // JSON text frame.
                    let send = if frame.ev.is_none() {
                        if let CfmlValue::Binary(bytes) = &frame.d {
                            sink.send(Message::Binary(bytes.clone().into())).await
                        } else {
                            sink.send(Message::Text(serialize_frame(&frame).into())).await
                        }
                    } else {
                        sink.send(Message::Text(serialize_frame(&frame).into())).await
                    };
                    if send.is_err() {
                        break;
                    }
                }
                SinkCmd::Close(code, reason) => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame { code, reason: reason.into() })))
                        .await;
                    break;
                }
            }
        }
    });

    // onConnect reject gate. A `false` return (or a throw) rejects the
    // handshake; an array return is the set of rooms to auto-join (P6).
    let connect =
        dispatch(&state, &info, &conn_id, session_id.clone(), "onConnect", None, None).await;
    let mut rejected = false;
    match connect {
        Ok(Some(v)) => {
            if is_reject(&v) {
                rejected = true;
            } else if let CfmlValue::Array(rooms) = &v {
                for r in rooms.iter() {
                    registry.join(&conn_id, &r.as_string());
                }
            }
        }
        Err(_) => rejected = true,
        Ok(None) => {}
    }
    if rejected {
        registry.close_conn(&conn_id, 1008, "connection rejected".into());
        registry.unregister(&conn_id);
        // Give the pump a moment to flush the close, then drop it.
        pump.abort();
        return;
    }

    // Resumability replay (P12): a reconnecting client sends `?lastEventId=…`;
    // queue the channel frames it missed ahead of live traffic. Done after the
    // onConnect accept + auto-join so the socket's rooms are re-established first.
    if info.history > 0 {
        if let Some(last) = registry.params_of(&conn_id).and_then(|p| {
            p.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("lastEventId"))
                .map(|(_, v)| v.as_string())
        }) {
            if !last.is_empty() {
                registry.replay_since(&info.channel, &last, &conn_id);
            }
        }
    }

    // Inbound loop. Ping/Pong are answered by axum and never reach CFML (P10).
    let mut close_reason = "client closed".to_string();
    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => {
                close_reason = "transport error".into();
                break;
            }
        };
        match msg {
            Message::Text(t) => {
                // For an `encoding="json"` channel an inbound object may name an
                // event (`{"ev":"say","d":{…}}`) → routed to an `on="say"`
                // handler with `d` as the payload; otherwise the whole parsed
                // value goes to `onMessage` (unchanged). `id` echoes back as the
                // ack's `ref` for client-side correlation.
                let parsed = if info.json {
                    Some(parse_json(t.as_str()))
                } else {
                    None
                };
                // Whisper (Phase 4): a `client-*` event is relayed to peers by
                // the hub with NO CFML running and no history — typing/cursor
                // chatter stays out of the request path. Only on a JSON channel
                // (the envelope is what carries the event name + optional room).
                if let Some(parsed) = &parsed {
                    if let Some((ev, data, room)) = as_client_event(parsed) {
                        relay_client_event(&registry, &info.channel, &conn_id, ev, data, room);
                        continue;
                    }
                }
                let (event, payload, reply_ref) = match parsed {
                    Some(p) => route_inbound(p),
                    None => (None, CfmlValue::string(t.as_str().to_string()), None),
                };
                let r = dispatch(
                    &state,
                    &info,
                    &conn_id,
                    session_id.clone(),
                    "onMessage",
                    event.as_deref(),
                    Some(payload),
                )
                .await;
                handle_message_result(&state, &info, &conn_id, &session_id, reply_ref, r).await;
            }
            Message::Binary(b) => {
                let payload = CfmlValue::Binary(b.to_vec());
                let r = dispatch(
                    &state,
                    &info,
                    &conn_id,
                    session_id.clone(),
                    "onMessage",
                    None,
                    Some(payload),
                )
                .await;
                handle_message_result(&state, &info, &conn_id, &session_id, None, r).await;
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    // Teardown — fire onDisconnect, then remove from the registry (and rooms).
    let _ = dispatch(
        &state,
        &info,
        &conn_id,
        session_id.clone(),
        "onDisconnect",
        None,
        Some(CfmlValue::string(close_reason)),
    )
    .await;
    registry.unregister(&conn_id);
    pump.abort();
}

/// Handle an `onMessage` dispatch result: ship the ack on success, or fire the
/// `onError(socket, err)` lifecycle method on failure (a throw inside a handler
/// is surfaced to the channel, never swallowed). If `onError` itself is absent
/// or also throws, the message is dropped — a bad message must not kill the
/// connection.
async fn handle_message_result(
    state: &Arc<AppState>,
    info: &ChannelInfo,
    conn_id: &str,
    session_id: &Option<String>,
    reply_ref: Option<String>,
    result: Result<Option<CfmlValue>, String>,
) {
    let registry = state.server_state.websocket.clone();
    match result {
        Ok(ack) => deliver_ack(&registry, conn_id, &info.channel, reply_ref, Ok(ack)),
        Err(msg) => {
            let mut err = cfml_common::dynamic::ValueMap::default();
            err.insert("message".to_string(), CfmlValue::string(msg));
            err.insert("type".to_string(), CfmlValue::string("Application".to_string()));
            let _ = dispatch(
                state,
                info,
                conn_id,
                session_id.clone(),
                "onError",
                None,
                Some(CfmlValue::strukt(err)),
            )
            .await;
        }
    }
}

/// Ship a non-null handler return value back to the sender as an `ack` frame
/// (design principle P5 — the handler's return value is the client's ack).
/// When the inbound frame carried an `id`, it rides back as the ack's `ref` so
/// the client can correlate the reply to its request.
fn deliver_ack(
    registry: &Arc<WebSocketRegistry>,
    conn_id: &str,
    channel: &str,
    reply_ref: Option<String>,
    result: Result<Option<CfmlValue>, String>,
) {
    if let Ok(Some(v)) = result {
        if !matches!(v, CfmlValue::Null) {
            let mut frame = registry.msg(channel, Some("ack".to_string()), v);
            frame.t = "ack".to_string();
            frame.ref_id = reply_ref;
            registry.emit_to(conn_id, frame);
        }
    }
}

/// Reject iff the value is a boolean-false (or false-ish scalar). `Null`,
/// arrays and structs are NOT rejections — only an explicit `return false`.
pub(crate) fn is_reject(v: &CfmlValue) -> bool {
    match v {
        CfmlValue::Bool(b) => !b,
        CfmlValue::Int(i) => *i == 0,
        CfmlValue::Double(d) => *d == 0.0,
        CfmlValue::String(s) => matches!(s.to_lowercase().as_str(), "false" | "no" | "0"),
        _ => false,
    }
}

/// Split an inbound JSON object into `(event, payload, ack-ref)`. An object
/// carrying a non-empty `ev` is an event frame: it routes to the matching
/// `on="<ev>"` handler with `d` as the payload, and its `id` becomes the ack's
/// `ref`. Anything else (a plain struct, an array, a scalar) keeps the legacy
/// behaviour — the whole value is handed to `onMessage` with no event.
fn route_inbound(parsed: CfmlValue) -> (Option<String>, CfmlValue, Option<String>) {
    if let CfmlValue::Struct(ref s) = parsed {
        let ev = s
            .get_ci("ev")
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty());
        if ev.is_some() {
            let payload = s.get_ci("d").unwrap_or(CfmlValue::Null);
            let id = s
                .get_ci("id")
                .map(|v| v.as_string())
                .filter(|s| !s.is_empty());
            return (ev, payload, id);
        }
    }
    (None, parsed, None)
}

/// Recognise an inbound whisper frame (Phase 4). A `client-`-prefixed event
/// name (Pusher convention, case-insensitive) marks a *client event* that the
/// hub relays straight to peers without running any CFML. Returns
/// `(event, payload, room)` — `room` (the optional `room` field) narrows the
/// relay to a single room the sender must belong to; absent → channel-wide.
/// Anything that is not a `client-*` event returns `None` (normal dispatch).
fn as_client_event(parsed: &CfmlValue) -> Option<(String, CfmlValue, Option<String>)> {
    let CfmlValue::Struct(s) = parsed else { return None };
    let ev = s.get_ci("ev").map(|v| v.as_string()).filter(|s| !s.is_empty())?;
    if !ev.to_ascii_lowercase().starts_with("client-") {
        return None;
    }
    let data = s.get_ci("d").unwrap_or(CfmlValue::Null);
    let room = s.get_ci("room").map(|v| v.as_string()).filter(|s| !s.is_empty());
    Some((ev, data, room))
}

/// Relay a whisper to peers (no CFML, no history). Room-targeted whispers are
/// membership-checked: a client may only whisper into a room it has joined
/// (otherwise the frame is silently dropped — a client can't inject into
/// arbitrary rooms). Shared with the socket.io transport.
pub(crate) fn relay_client_event(
    registry: &Arc<WebSocketRegistry>,
    channel: &str,
    sender: &str,
    event: String,
    data: CfmlValue,
    room: Option<String>,
) {
    let mut frame = registry.msg(channel, Some(event), data);
    frame.t = "client".to_string();
    match room {
        Some(r) => {
            if registry.rooms_of(sender).iter().any(|x| x.eq_ignore_ascii_case(&r)) {
                registry.relay(channel, Some(&r), frame, Some(sender));
            }
        }
        None => registry.relay(channel, None, frame, Some(sender)),
    }
}

/// Run one channel dispatch on a blocking worker (the VM is synchronous).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch(
    state: &Arc<AppState>,
    info: &ChannelInfo,
    conn_id: &str,
    session_id: Option<String>,
    method: &'static str,
    event: Option<&str>,
    payload: Option<CfmlValue>,
) -> Result<Option<CfmlValue>, String> {
    let server_state = state.server_state.clone();
    let vfs = state.vfs.clone();
    let sandbox = state.sandbox;
    let channel = info.channel.clone();
    let cfc_path = info.cfc_path.clone();
    let conn_id = conn_id.to_string();
    let event = event.map(|e| e.to_string());
    tokio::task::spawn_blocking(move || {
        run_dispatch(
            server_state,
            vfs,
            sandbox,
            channel,
            cfc_path,
            conn_id,
            session_id,
            method,
            event,
            payload,
        )
    })
    .await
    .unwrap_or_else(|e| Err(format!("dispatch task panicked: {e}")))
}

#[allow(clippy::too_many_arguments)]
fn run_dispatch(
    server_state: cfml_vm::ServerState,
    vfs: Arc<dyn cfml_common::vfs::Vfs>,
    sandbox: bool,
    channel: String,
    cfc_path: String,
    conn_id: String,
    session_id: Option<String>,
    method: &str,
    event: Option<String>,
    payload: Option<CfmlValue>,
) -> Result<Option<CfmlValue>, String> {
    // Fresh VM, wired like a request VM (same `register_vm_runtime` path).
    let empty = crate::CfmlCompiler::new()
        .compile(crate::CfmlParser::new(String::new()).parse().expect("empty source parses"));
    let mut vm = CfmlVirtualMachine::new(empty);
    vm.vfs = vfs;
    vm.sandbox = sandbox;
    crate::register_vm_runtime(&mut vm);
    vm.apply_cfconfig(&server_state.cfconfig);
    vm.session_id = session_id;
    vm.source_file = Some(std::sync::Arc::from(cfc_path.as_str()));
    let registry = server_state.websocket.clone();
    vm.server_state = Some(server_state);

    // The live `socket` NativeObject bound to this connection + the registry.
    let handle = SocketHandle::new(conn_id, channel.clone(), registry);
    let socket = CfmlValue::NativeObject(Arc::new(std::sync::RwLock::new(handle)));

    let mut args = vec![socket];
    if let Some(p) = payload {
        args.push(p);
    }

    vm.dispatch_ws_event(&channel, &cfc_path, method, event.as_deref(), args)
        .map_err(|e| format!("{e}"))
}

/// What an imperative socket.io-lucee dispatch should run.
pub(crate) enum SioDispatch {
    /// A namespace-level listener (`connect` / `disconnect` / `disconnecting`),
    /// invoked with a fresh socket facade.
    NsEvent(&'static str),
    /// A per-socket inbound event listener, invoked with the client payload;
    /// its return value rides back as the socket.io ack.
    SocketEvent { event: String, payload: CfmlValue },
}

/// Run one imperative socket.io-lucee dispatch on a blocking worker. Parallel to
/// [`dispatch`] but for the compat surface: it resolves the stored handler from
/// the process-wide `socketio_compat` store (not a convention CFC) and invokes
/// it on a fresh VM.
pub(crate) async fn dispatch_sio(
    state: &Arc<AppState>,
    ns: &str,
    conn_id: &str,
    session_id: Option<String>,
    kind: SioDispatch,
) -> Result<Option<CfmlValue>, String> {
    let server_state = state.server_state.clone();
    let vfs = state.vfs.clone();
    let sandbox = state.sandbox;
    let ns = ns.to_string();
    let conn_id = conn_id.to_string();
    tokio::task::spawn_blocking(move || {
        let empty = crate::CfmlCompiler::new()
            .compile(crate::CfmlParser::new(String::new()).parse().expect("empty source parses"));
        let mut vm = CfmlVirtualMachine::new(empty);
        vm.vfs = vfs;
        vm.sandbox = sandbox;
        crate::register_vm_runtime(&mut vm);
        vm.apply_cfconfig(&server_state.cfconfig);
        vm.session_id = session_id;
        vm.server_state = Some(server_state);
        let r = match kind {
            SioDispatch::NsEvent(event) => vm.dispatch_sio_ns(&conn_id, &ns, event),
            SioDispatch::SocketEvent { event, payload } => {
                vm.dispatch_sio_event(&conn_id, &ns, &event, payload)
            }
        };
        r.map_err(|e| format!("{e}"))
    })
    .await
    .unwrap_or_else(|e| Err(format!("socket.io dispatch task panicked: {e}")))
}

/// Serialize a wire frame to JSON text using the engine's own `serializeJSON`
/// (so `encoding="json"` payloads round-trip with CFML semantics).
fn serialize_frame(frame: &WireEnvelope) -> String {
    let value = CfmlValue::strukt(frame.to_value_map());
    match cfml_stdlib::builtins::fn_serialize_json(vec![value]) {
        Ok(v) => v.as_string(),
        Err(_) => "{}".to_string(),
    }
}

/// Parse inbound JSON text into a CfmlValue using the engine's
/// `deserializeJSON`. Malformed input degrades to the raw string (the handler
/// still runs; it can validate) rather than dropping the message.
fn parse_json(text: &str) -> CfmlValue {
    match cfml_stdlib::builtins::fn_deserialize_json(vec![CfmlValue::string(text.to_string())]) {
        Ok(v) => v,
        Err(_) => CfmlValue::string(text.to_string()),
    }
}
