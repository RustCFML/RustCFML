//! Phase 3 socket.io transport integration tests.
//!
//! Drives the built `rustcfml` binary in serve mode against the SAME channel
//! CFCs the raw-WS suite uses (`tests/fixtures/ws_app/websockets/`), but over
//! the socket.io transport (socketioxide layer on `/socket.io/`). CFML can't be
//! a socket.io client, so we hand-roll a minimal Engine.IO v4 / Socket.IO v5
//! client over `tokio-tungstenite` (the same dep the raw suite uses) — no heavy
//! socket.io client crate.
//!
//! Acceptance (docs/websocket-implementation-plan.md, Phase 3): a stock
//! socket.io client connects to a namespace, emits an event, receives a
//! server-pushed event + a broadcast to a second client, and gets a native ack
//! (the handler's return value). Plus the onConnect reject gate closes the
//! socket.
//!
//! Wire format (after the Engine.IO `4` message prefix), per the socket.io v5
//! parser: `<sio-type>[<namespace>,][<ack-id>]<json>` — e.g.
//! `42/echo,7["say",{…}]` = EVENT on `/echo`, ack id 7, args `["say",{…}]`.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ws_app")
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start_server() -> Server {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .spawn()
        .expect("spawn rustcfml --serve");
    // A debug-build server under a fully parallel `cargo test --workspace` can
    // take well over 5s to bind; time out loudly instead of falling through to
    // an opaque connect panic in the first request.
    let mut server = Server { child, port };
    for _ in 0..600 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            panic!("rustcfml --serve exited before accepting connections: {status}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("rustcfml --serve not accepting connections on port {port} after 30s");
}

/// A decoded inbound socket.io packet (only the kinds the tests care about).
#[derive(Debug)]
enum Packet {
    /// Namespace connect acknowledgement (`40/ns,{sid}`).
    Connect,
    /// Namespace disconnect (`41/ns,`).
    Disconnect,
    /// An event: `(event_name, first_arg)`.
    Event(String, Value),
    /// An ack to one of our emits: `(ack_id, first_arg)`.
    Ack(u64, Value),
    /// Anything else we don't model (engine.io open, etc.).
    Other,
}

/// Read the next socket.io packet, auto-responding to Engine.IO pings. Times
/// out so a hung test fails loudly rather than blocking forever.
async fn next_packet(ws: &mut Ws) -> Packet {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let s = t.as_str();
                    // Engine.IO ping (`2`) → pong (`3`).
                    if s == "2" {
                        let _ = ws.send(Message::Text("3".to_string().into())).await;
                        continue;
                    }
                    // Engine.IO message packets are prefixed with `4`.
                    let Some(sio) = s.strip_prefix('4') else {
                        return Packet::Other; // engine.io open `0{…}`, etc.
                    };
                    return parse_sio(sio);
                }
                Some(Ok(Message::Close(_))) | None => return Packet::Disconnect,
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("ws error: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("timed out waiting for a socket.io packet")
}

/// Parse one Socket.IO packet body (the part after the Engine.IO `4`):
/// `<type>[<namespace>,][<ack-id>]<json>`.
fn parse_sio(sio: &str) -> Packet {
    let mut chars = sio.char_indices();
    let (_, ty) = chars.next().expect("sio type");
    let mut rest = &sio[ty.len_utf8()..];
    // Optional namespace, terminated by ',' (only present when it starts '/').
    if rest.starts_with('/') {
        if let Some(comma) = rest.find(',') {
            rest = &rest[comma + 1..];
        }
    }
    // Optional leading ack-id digits (before the `[`/`{` of the JSON).
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let ack_id: Option<u64> = if digits.is_empty() {
        None
    } else {
        rest = &rest[digits.len()..];
        digits.parse().ok()
    };
    match ty {
        '0' => Packet::Connect,
        '1' => Packet::Disconnect,
        '2' => {
            // ["event", arg]
            let arr: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
            let ev = arr.get(0).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let data = arr.get(1).cloned().unwrap_or(Value::Null);
            Packet::Event(ev, data)
        }
        '3' => {
            // [arg]
            let arr: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
            let data = arr.get(0).cloned().unwrap_or(Value::Null);
            Packet::Ack(ack_id.unwrap_or(0), data)
        }
        _ => Packet::Other,
    }
}

/// Open a websocket, do the Engine.IO open + Socket.IO namespace connect, and
/// return the live stream once the namespace connect is acknowledged.
async fn sio_connect(port: u16, ns: &str, query: &str) -> Ws {
    let q = if query.is_empty() { String::new() } else { format!("&{query}") };
    let url = format!("ws://127.0.0.1:{port}/socket.io/?EIO=4&transport=websocket{q}");
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");
    // First frame is the Engine.IO open packet (`0{…}`).
    match ws.next().await {
        Some(Ok(Message::Text(t))) => assert!(t.starts_with('0'), "expected EIO open, got {t}"),
        other => panic!("expected EIO open frame, got {other:?}"),
    }
    // Connect to the namespace: EIO message `4` + SIO connect `0` + `/ns,`.
    ws.send(Message::Text(format!("40{ns},").into())).await.expect("send connect");
    // Wait for the namespace connect ack.
    loop {
        match next_packet(&mut ws).await {
            Packet::Connect => break,
            Packet::Disconnect => panic!("namespace connect rejected"),
            _ => continue,
        }
    }
    ws
}

/// Emit `event` with `data` on `ns`, optionally requesting an ack (`ack_id`).
async fn sio_emit(ws: &mut Ws, ns: &str, event: &str, data: Value, ack_id: Option<u64>) {
    let payload = json!([event, data]).to_string();
    let frame = match ack_id {
        Some(id) => format!("42{ns},{id}{payload}"),
        None => format!("42{ns},{payload}"),
    };
    ws.send(Message::Text(frame.into())).await.expect("emit");
}

/// Read packets until an event with the given name arrives; returns its data.
async fn expect_event(ws: &mut Ws, name: &str) -> Value {
    for _ in 0..20 {
        if let Packet::Event(ev, data) = next_packet(ws).await {
            if ev == name {
                return data;
            }
        }
    }
    panic!("never received event {name:?}");
}

#[tokio::test]
async fn socketio_connect_emit_ack_event_and_broadcast() {
    let server = start_server().await;

    // Client A connects to the /echo namespace with a handshake query param.
    let mut a = sio_connect(server.port, "/echo", "user=alice").await;
    // onConnect emits "welcome" with the param echoed back.
    let welcome = expect_event(&mut a, "welcome").await;
    assert_eq!(welcome["user"], json!("alice"), "socket.param() in welcome");
    assert!(welcome["id"].is_string(), "welcome carries the socket id");

    // Emit the annotated `on="say"` event, requesting an ack (id 1).
    sio_emit(&mut a, "/echo", "say", json!({ "text": "hi" }), Some(1)).await;
    // handleSay emits "sayEcho" and returns {routed:"say"} as the native ack.
    let mut got_say_echo = false;
    let mut got_ack = false;
    for _ in 0..20 {
        match next_packet(&mut a).await {
            Packet::Event(ev, data) if ev == "sayecho" || ev == "sayEcho" => {
                assert_eq!(data["routed"], json!("say"));
                assert_eq!(data["text"], json!("hi"));
                got_say_echo = true;
            }
            Packet::Ack(id, data) => {
                assert_eq!(id, 1, "ack correlates to our emit id");
                assert_eq!(data["routed"], json!("say"), "handler return is the ack");
                got_ack = true;
            }
            _ => {}
        }
        if got_say_echo && got_ack {
            break;
        }
    }
    assert!(got_say_echo, "received the sayEcho event");
    assert!(got_ack, "received the native socket.io ack");

    // Client B connects; A's onMessage broadcast should reach it.
    let mut b = sio_connect(server.port, "/echo", "user=bob").await;
    let _ = expect_event(&mut b, "welcome").await;

    // Emit a plain "message" (→ onMessage): echoes to A, broadcasts "said" to B,
    // returns {ok:true} as A's ack (id 2).
    sio_emit(&mut a, "/echo", "message", json!({ "text": "hello" }), Some(2)).await;

    // B receives the broadcast.
    let said = expect_event(&mut b, "said").await;
    assert_eq!(said["text"], json!("hello"), "broadcast reached the other client");

    // A receives its echo + ack.
    let mut got_echo = false;
    let mut got_ok_ack = false;
    for _ in 0..20 {
        match next_packet(&mut a).await {
            Packet::Event(ev, data) if ev == "echo" => {
                assert_eq!(data["text"], json!("hello"));
                got_echo = true;
            }
            Packet::Ack(2, data) => {
                assert_eq!(data["ok"], json!(true), "onMessage return is the ack");
                got_ok_ack = true;
            }
            _ => {}
        }
        if got_echo && got_ok_ack {
            break;
        }
    }
    assert!(got_echo, "sender received its echo");
    assert!(got_ok_ack, "sender received the onMessage ack");
}

#[tokio::test]
async fn socketio_whisper_relays_to_peers() {
    let server = start_server().await;

    // Two clients on the /whisper namespace (both auto-join "lobby" in onConnect).
    let mut a = sio_connect(server.port, "/whisper", "").await;
    let _ = expect_event(&mut a, "ready").await;
    let mut b = sio_connect(server.port, "/whisper", "").await;
    let _ = expect_event(&mut b, "ready").await;

    // A whispers `client-typing` (declared via `clientEvents="typing"`). The hub
    // relays it to B with NO CFML running — socketioxide has no catch-all, so the
    // event was subscribed up front from the declaration.
    sio_emit(&mut a, "/whisper", "client-typing", json!({ "on": true }), None).await;
    let typing = expect_event(&mut b, "client-typing").await;
    assert_eq!(typing["on"], json!(true), "whisper relayed to the peer");

    // Connection intact + the whisper bypassed CFML: a normal "message" still
    // runs onMessage, broadcasting "said" to B.
    sio_emit(&mut a, "/whisper", "message", json!({ "text": "hi" }), None).await;
    let said = expect_event(&mut b, "said").await;
    assert_eq!(said["text"], json!("hi"), "normal handler still runs after a whisper");
}

#[tokio::test]
async fn socketio_onconnect_reject_closes_socket() {
    let server = start_server().await;
    let q = "&".to_string();
    let url = format!(
        "ws://127.0.0.1:{}/socket.io/?EIO=4&transport=websocket{}",
        server.port,
        q.trim_end_matches('&')
    );
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");
    // EIO open.
    let _ = ws.next().await;
    // Connect to the rejecting namespace.
    ws.send(Message::Text("40/guarded,".to_string().into())).await.expect("send connect");
    // socketioxide sends the connect ack before our handler runs; onConnect then
    // returns false, so the socket is disconnected. We must observe a disconnect.
    let mut disconnected = false;
    for _ in 0..20 {
        match next_packet(&mut ws).await {
            Packet::Disconnect => {
                disconnected = true;
                break;
            }
            _ => {}
        }
    }
    assert!(disconnected, "rejected onConnect disconnects the socket");
}
