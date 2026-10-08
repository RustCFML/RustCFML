//! socket.io transport (Phase 3, native-only).
//!
//! Mounts a [`socketioxide`] tower layer on the existing axum router so a stock
//! socket.io client (Engine.IO v4: namespaces, acks, binary, polling↔ws
//! fallback) can talk to the very same channel CFCs the raw-WS driver serves.
//! socketioxide is *only* the transport — every fan-out (`wsPublish`, `io()`,
//! presence, broadcast, the distributed `Broker`) still funnels through our own
//! [`WebSocketRegistry`]. One socket.io connection therefore:
//!
//!   * resolves its namespace (`/chat`) to a channel CFC (shared
//!     [`crate::websocket::resolve_channel`]),
//!   * registers a [`FrameSink`] that re-emits registry frames as socket.io
//!     events, so server-initiated pushes reach it like any other connection,
//!   * dispatches inbound events through the same `dispatch_ws_event` path as
//!     the raw driver — the handler's return value rides back as the **native
//!     socket.io ack** (design principle P5), and
//!   * fires `onConnect`/`onMessage`/`onError`/`onDisconnect` identically.
//!
//! The concrete set of `on="event"` names is scanned from the CFC at handshake
//! ([`crate::websocket::extract_event_names`]) and a `socket.on` registered for
//! each, plus the conventional `"message"` → `onMessage`. The imperative
//! surface additionally registers a catch-all (`on_fallback`), since a
//! socket.io-lucee app keeps its listener names in CFML.

use std::sync::Arc;

use cfml_common::dynamic::CfmlValue;
use cfml_vm::websocket::{FrameSink, WireEnvelope};
use socketioxide::extract::{AckSender, Data, Event, SocketRef};
use socketioxide::layer::SocketIoLayer;
use socketioxide::SocketIo;

use crate::websocket::{
    dispatch, dispatch_sio, is_reject, relay_client_event, resolve_channel, SioDispatch,
};
use crate::AppState;

/// The [`FrameSink`] handed to the registry for one socket.io connection. A
/// frame the registry routes to this connection is re-emitted as a socket.io
/// event named by the frame's `ev` (a raw `socket.send` → `"message"`; a
/// presence frame → `"presence_state"`/`"presence_diff"`; otherwise the frame
/// type). socket.io owns its own outbound buffering/backpressure, so `emit` is
/// non-blocking and we just drop on a closed socket.
#[derive(Debug)]
struct SocketIoSink {
    socket: SocketRef,
    /// When `Some`, outbound frames are buffered instead of emitted. The
    /// imperative compat path uses this to hold frames emitted *during* the
    /// connect listener (e.g. a `welcome` greeting) until the socketioxide
    /// `on(event)` handlers the listener registered are wired up — otherwise a
    /// client that emits in reaction to the greeting would race ahead of its
    /// (not-yet-registered) handler, and socketioxide (no catch-all) would drop
    /// the event. `None` for the fluent/convention path (emit immediately).
    buffer: std::sync::Mutex<Option<Vec<WireEnvelope>>>,
}

impl SocketIoSink {
    fn new(socket: SocketRef) -> Self {
        Self { socket, buffer: std::sync::Mutex::new(None) }
    }

    /// Start buffering outbound frames (idempotent).
    fn buffer_mode(&self) {
        let mut b = self.buffer.lock().unwrap();
        if b.is_none() {
            *b = Some(Vec::new());
        }
    }

    /// Stop buffering and emit everything captured so far, in order.
    fn flush(&self) {
        let drained = self.buffer.lock().unwrap().take();
        for frame in drained.into_iter().flatten() {
            self.emit_now(frame);
        }
    }

    fn emit_now(&self, frame: WireEnvelope) {
        let event = frame.ev.clone().unwrap_or_else(|| {
            if frame.t == "msg" {
                "message".to_string()
            } else {
                frame.t.clone()
            }
        });
        // `emit` errors only if the socket is already closed → nothing to do.
        let _ = self.socket.emit(event.as_str(), &frame.d);
    }
}

impl FrameSink for SocketIoSink {
    fn send(&self, frame: WireEnvelope) {
        {
            let mut b = self.buffer.lock().unwrap();
            if let Some(buf) = b.as_mut() {
                buf.push(frame);
                return;
            }
        }
        self.emit_now(frame);
    }
    fn close(&self, _code: u16, _reason: String) {
        // socket.io has no close-code channel like raw WS; just disconnect.
        let _ = self.socket.clone().disconnect();
    }
}

/// Build the socket.io tower layer, wiring a single dynamic namespace handler
/// that resolves any `/<channel>` namespace to a CFC under
/// `<docroot>/websockets/`. The returned layer is mounted on the axum router;
/// the `SocketIo` handle is consumed here (namespaces live on the shared client
/// the layer holds, so nothing else needs it — fan-out goes through our
/// registry, never socketioxide's own rooms).
pub(crate) fn build_layer(state: Arc<AppState>) -> SocketIoLayer {
    let (layer, io) = SocketIo::new_layer();
    io.dyn_ns("/{ns}", move |socket: SocketRef| {
        let state = state.clone();
        async move {
            on_connect(socket, state).await;
        }
    })
    .expect("register socket.io dynamic namespace");
    layer
}

/// One socket.io connection: resolve the channel, register with the registry,
/// run the `onConnect` reject gate (+ auto-join + resumability replay), then
/// wire up the per-event handlers and the disconnect teardown.
async fn on_connect(socket: SocketRef, state: Arc<AppState>) {
    let ns = socket.ns().to_string();
    // The imperative socket.io-lucee surface owns this namespace if a
    // `SocketIoServer` registered it (via `io.of(ns)`). Otherwise fall through
    // to the convention `websockets/<name>.cfc` discovery. The two surfaces
    // share this one transport + the one WebSocketRegistry.
    if cfml_vm::socketio_compat::compat().is_imperative_ns(&ns) {
        on_connect_imperative(socket, state, ns).await;
        return;
    }

    // Namespace `/chat` → channel CFC `chat`. An unknown channel is refused.
    let name = ns.trim_start_matches('/');
    let info = match resolve_channel(&state, name) {
        Some(i) => i,
        None => {
            let _ = socket.disconnect();
            return;
        }
    };

    // Handshake identity (CFID cookie) + query params (P6), read off the
    // upgrade request socketioxide preserved on the socket.
    let parts = socket.req_parts();
    let session_id = crate::websocket::session_id_from_headers(&parts.headers);
    let params = crate::websocket::parse_query(parts.uri.query());

    let registry = state.server_state.websocket.clone();
    let sink: Arc<dyn FrameSink> = Arc::new(SocketIoSink::new(socket.clone()));
    let conn_id = registry.register(&info.channel, sink, session_id.clone(), params);
    registry.set_handshake(
        &conn_id,
        crate::websocket::handshake_map(&parts.headers, &parts.uri.to_string(), parts.uri.query()),
    );
    registry.set_history_cap(&info.channel, info.history);

    // Register the per-event handlers *before* the onConnect gate. socketioxide
    // spawns this connect handler (so inbound packets are processed
    // concurrently), and it has no catch-all — registering up front means the
    // `socket.on` handlers exist the instant the socket is connected, closing
    // the window where a client that emits immediately after connecting would
    // have its first event silently dropped. (A message racing *ahead* of the
    // onConnect side effects is a sub-millisecond window; real clients await the
    // `connect` event first.) `on_disconnect` is registered only after accept,
    // below, so a rejected connection never fires `onDisconnect`.
    for event in &info.events {
        let event_name = event.clone();
        // `"message"` maps to `onMessage` with no event; everything else routes
        // to the matching `on="<event>"` handler (resolved VM-side).
        let routed_event = if event_name == "message" {
            None
        } else {
            Some(event_name.clone())
        };
        let state = state.clone();
        let info = info.clone();
        let conn_id = conn_id.clone();
        let session_id = session_id.clone();
        socket.on(
            event_name,
            move |_s: SocketRef, Data::<CfmlValue>(payload): Data<CfmlValue>, ack: AckSender| {
                let state = state.clone();
                let info = info.clone();
                let conn_id = conn_id.clone();
                let session_id = session_id.clone();
                let routed_event = routed_event.clone();
                async move {
                    let r = dispatch(
                        &state,
                        &info,
                        &conn_id,
                        session_id.clone(),
                        "onMessage",
                        routed_event.as_deref(),
                        Some(payload),
                    )
                    .await;
                    match r {
                        // Non-null return → the client's ack (P5). A null/void
                        // return sends no ack (matches the raw driver).
                        Ok(Some(v)) if !matches!(v, CfmlValue::Null) => {
                            let _ = ack.send(&v);
                        }
                        Ok(_) => {}
                        // A throw surfaces to onError, never killing the socket.
                        Err(msg) => {
                            let mut err = cfml_common::dynamic::ValueMap::default();
                            err.insert("message".to_string(), CfmlValue::string(msg));
                            err.insert(
                                "type".to_string(),
                                CfmlValue::string("Application".to_string()),
                            );
                            let _ = dispatch(
                                &state,
                                &info,
                                &conn_id,
                                session_id,
                                "onError",
                                None,
                                Some(CfmlValue::strukt(err)),
                            )
                            .await;
                        }
                    }
                }
            },
        );
    }

    // Whisper / client-events (Phase 4): a `client-*` event is relayed to peers
    // by the hub with NO CFML running. socketioxide has no catch-all, so the
    // relay events are declared via `clientEvents="…"` and subscribed up front
    // (channel-wide; room-targeting is a raw-WS refinement — the socket.io
    // payload is the data itself, with no envelope to carry a room).
    for ev in &info.client_events {
        let ev_name = ev.clone();
        let registry = registry.clone();
        let channel = info.channel.clone();
        let conn_id = conn_id.clone();
        socket.on(
            ev_name.clone(),
            move |_s: SocketRef, Data::<CfmlValue>(payload): Data<CfmlValue>| {
                let registry = registry.clone();
                let channel = channel.clone();
                let conn_id = conn_id.clone();
                let ev_name = ev_name.clone();
                async move {
                    relay_client_event(&registry, &channel, &conn_id, ev_name, payload, None);
                }
            },
        );
    }

    // onConnect reject gate (same semantics as the raw driver): `false` → reject;
    // an array return → the rooms to auto-join.
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
        registry.unregister(&conn_id);
        let _ = socket.disconnect();
        return;
    }

    // Resumability replay (P12): a reconnecting client passes `?lastEventId=…`
    // (socket.io `query` option); queue the missed channel frames ahead of live
    // traffic, after the auto-join so rooms are re-established first.
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

    // Teardown: fire onDisconnect then unregister (rooms + presence cleanup is
    // unconditional in the registry, P10).
    {
        let state = state.clone();
        let info = info.clone();
        let conn_id = conn_id.clone();
        let session_id = session_id.clone();
        socket.on_disconnect(move |_s: SocketRef| {
            let state = state.clone();
            let info = info.clone();
            let conn_id = conn_id.clone();
            let session_id = session_id.clone();
            async move {
                let _ = dispatch(
                    &state,
                    &info,
                    &conn_id,
                    session_id,
                    "onDisconnect",
                    None,
                    Some(CfmlValue::string("client closed".to_string())),
                )
                .await;
                state.server_state.websocket.unregister(&conn_id);
            }
        });
    }
}

/// One socket.io connection on an *imperative* (socket.io-lucee compat)
/// namespace: register with the shared registry, run the namespace `connect`
/// listener (which builds the socket facade and registers per-socket event
/// listeners engine-side), then wire socketioxide handlers for those events and
/// the disconnect teardown. Fan-out (`socket.emit`/`broadcast`, `namespace.emit`)
/// rides the same WebSocketRegistry as the fluent API.
async fn on_connect_imperative(socket: SocketRef, state: Arc<AppState>, ns: String) {
    let parts = socket.req_parts();
    let session_id = crate::websocket::session_id_from_headers(&parts.headers);
    let params = crate::websocket::parse_query(parts.uri.query());

    let registry = state.server_state.websocket.clone();
    // Buffer outbound frames emitted during connect (e.g. a `welcome` greeting)
    // until the per-event handlers are wired (see `SocketIoSink::buffer`).
    let sink = Arc::new(SocketIoSink::new(socket.clone()));
    sink.buffer_mode();
    // The namespace doubles as the registry channel, so `$sioBroadcast` /
    // `$sioSend` route by it just like the fluent `io(channel)`.
    let conn_id =
        registry.register(&ns, sink.clone() as Arc<dyn FrameSink>, session_id.clone(), params);
    registry.set_handshake(
        &conn_id,
        crate::websocket::handshake_map(&parts.headers, &parts.uri.to_string(), parts.uri.query()),
    );
    cfml_vm::socketio_compat::compat().register_conn(&conn_id, &ns);

    // Run the namespace `connect` listener (builds the socket facade; may call
    // `socket.on(event, ...)`, stored engine-side keyed by this connection).
    let _ = dispatch_sio(
        &state,
        &ns,
        &conn_id,
        session_id.clone(),
        SioDispatch::NsEvent("connect"),
    )
    .await;

    // socketioxide 0.16 has no catch-all, so subscribe to exactly the events the
    // connect listener registered. (A `socket.on` made *after* connect won't be
    // wired — register all handlers in the connect listener, the socket.io-lucee
    // convention.)
    let events = cfml_vm::socketio_compat::compat().socket_events(&conn_id);
    for event in events {
        let state = state.clone();
        let ns = ns.clone();
        let conn_id = conn_id.clone();
        let session_id = session_id.clone();
        let ev = event.clone();
        socket.on(
            event,
            move |_s: SocketRef, Data::<CfmlValue>(payload): Data<CfmlValue>, ack: AckSender| {
                let state = state.clone();
                let ns = ns.clone();
                let conn_id = conn_id.clone();
                let session_id = session_id.clone();
                let ev = ev.clone();
                async move {
                    let r = dispatch_sio(
                        &state,
                        &ns,
                        &conn_id,
                        session_id,
                        SioDispatch::SocketEvent { event: ev, payload },
                    )
                    .await;
                    // Non-null return → the client's ack (P5).
                    if let Ok(Some(v)) = r {
                        if !matches!(v, CfmlValue::Null) {
                            let _ = ack.send(&v);
                        }
                    }
                }
            },
        );
    }

    // Catch-all for every other inbound event. socket.io-lucee keeps its
    // per-socket listeners in CFML (`socket.on( event, cb )` stores a closure
    // and tells the server nothing), so when a namespace is driven through the
    // `SocketIoServerWrapper` shim there is no event list to subscribe to —
    // without this, a client's events were silently dropped. It also closes the
    // older gap where a `socket.on` registered AFTER connect was never wired.
    {
        let state = state.clone();
        let ns_fb = ns.clone();
        let conn_id_fb = conn_id.clone();
        let session_id_fb = session_id.clone();
        socket.on_fallback(
            move |Event(event): Event, Data::<CfmlValue>(payload): Data<CfmlValue>, ack: AckSender| {
                let state = state.clone();
                let ns = ns_fb.clone();
                let conn_id = conn_id_fb.clone();
                let session_id = session_id_fb.clone();
                async move {
                    let r = dispatch_sio(
                        &state,
                        &ns,
                        &conn_id,
                        session_id,
                        SioDispatch::SocketEvent { event, payload },
                    )
                    .await;
                    if let Ok(Some(v)) = r {
                        if !matches!(v, CfmlValue::Null) {
                            let _ = ack.send(&v);
                        }
                    }
                }
            },
        );
    }

    // Handlers are wired — release the frames buffered during connect (the
    // greeting now reaches the client, which can safely emit in response).
    sink.flush();

    // Teardown: `disconnecting` then `disconnect`, then forget the connection.
    {
        let state = state.clone();
        let ns = ns.clone();
        let conn_id = conn_id.clone();
        let session_id = session_id.clone();
        socket.on_disconnect(move |_s: SocketRef| {
            let state = state.clone();
            let ns = ns.clone();
            let conn_id = conn_id.clone();
            let session_id = session_id.clone();
            async move {
                let _ = dispatch_sio(
                    &state,
                    &ns,
                    &conn_id,
                    session_id.clone(),
                    SioDispatch::NsEvent("disconnecting"),
                )
                .await;
                let _ = dispatch_sio(
                    &state,
                    &ns,
                    &conn_id,
                    session_id,
                    SioDispatch::NsEvent("disconnect"),
                )
                .await;
                cfml_vm::socketio_compat::compat().drop_conn(&conn_id);
                state.server_state.websocket.unregister(&conn_id);
            }
        });
    }
}
