//! Phase 3b — socket.io-lucee compat layer integration tests.
//!
//! Drives the built `rustcfml` binary in serve mode against the *imperative*
//! surface (`new SocketIoServer()` / `io.of(ns).on("connect", fn)` /
//! `socket.on/emit/broadcast/joinRoom`) bootstrapped in the fixture's
//! `Application.cfc::onApplicationStart`. The same hand-rolled Engine.IO v4 /
//! Socket.IO v5 client as `websocket_socketio.rs` (CFML can't be a socket.io
//! client). The imperative namespaces share the `/socket.io/` transport and the
//! one WebSocketRegistry with the fluent/convention surface.
//!
//! Acceptance (docs/websocket-implementation-plan.md, Phase 3b): a
//! preside-ext-socket-io-style handler runs unchanged — the connect listener
//! runs against a live socket facade, a per-socket `socket.on` listener fires
//! with a native ack, a broadcast reaches a second client, and room-scoped
//! `namespace.emit(rooms=[…])` is honoured.

use std::io::{Read, Write};
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sio_app")
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

/// Fire a plain HTTP GET so `Application.cfc::onApplicationStart` runs and
/// registers the imperative socket.io namespaces *before* a socket connects
/// (the app owns its bootstrap, exactly as socket.io-lucee requires).
fn warmup_get(port: u16) {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect for warmup");
    stream
        .write_all(
            format!("GET /index.cfm HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write warmup request");
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
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
    let mut ready = false;
    for _ in 0..600 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            panic!("rustcfml --serve exited before accepting connections: {status}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !ready {
        panic!("rustcfml --serve not accepting connections on port {port} after 30s");
    }
    warmup_get(port);
    server
}

#[derive(Debug)]
enum Packet {
    Connect,
    Disconnect,
    Event(String, Value),
    Ack(u64, Value),
    Other,
}

async fn next_packet(ws: &mut Ws) -> Packet {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let s = t.as_str();
                    if s == "2" {
                        let _ = ws.send(Message::Text("3".to_string().into())).await;
                        continue;
                    }
                    let Some(sio) = s.strip_prefix('4') else {
                        return Packet::Other;
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

fn parse_sio(sio: &str) -> Packet {
    let mut chars = sio.char_indices();
    let (_, ty) = chars.next().expect("sio type");
    let mut rest = &sio[ty.len_utf8()..];
    if rest.starts_with('/') {
        if let Some(comma) = rest.find(',') {
            rest = &rest[comma + 1..];
        }
    }
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
            let arr: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
            let ev = arr.get(0).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let data = arr.get(1).cloned().unwrap_or(Value::Null);
            Packet::Event(ev, data)
        }
        '3' => {
            let arr: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
            let data = arr.get(0).cloned().unwrap_or(Value::Null);
            Packet::Ack(ack_id.unwrap_or(0), data)
        }
        _ => Packet::Other,
    }
}

async fn sio_connect(port: u16, ns: &str, query: &str) -> Ws {
    let q = if query.is_empty() { String::new() } else { format!("&{query}") };
    let url = format!("ws://127.0.0.1:{port}/socket.io/?EIO=4&transport=websocket{q}");
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");
    match ws.next().await {
        Some(Ok(Message::Text(t))) => assert!(t.starts_with('0'), "expected EIO open, got {t}"),
        other => panic!("expected EIO open frame, got {other:?}"),
    }
    ws.send(Message::Text(format!("40{ns},").into())).await.expect("send connect");
    loop {
        match next_packet(&mut ws).await {
            Packet::Connect => break,
            Packet::Disconnect => panic!("namespace connect rejected"),
            _ => continue,
        }
    }
    ws
}

async fn sio_emit(ws: &mut Ws, ns: &str, event: &str, data: Value, ack_id: Option<u64>) {
    let payload = json!([event, data]).to_string();
    let frame = match ack_id {
        Some(id) => format!("42{ns},{id}{payload}"),
        None => format!("42{ns},{payload}"),
    };
    ws.send(Message::Text(frame.into())).await.expect("emit");
}

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
async fn sio_compat_connect_emit_ack_and_broadcast() {
    let server = start_server().await;

    // Client A connects to the imperative /im namespace.
    let mut a = sio_connect(server.port, "/im", "").await;
    // The connect listener emits "welcome" with the socket id.
    let welcome = expect_event(&mut a, "welcome").await;
    assert!(welcome["id"].is_string(), "welcome carries the socket id");

    // Emit the per-socket `on("say")` listener, requesting an ack (id 1).
    sio_emit(&mut a, "/im", "say", json!({ "text": "hi" }), Some(1)).await;
    let mut got_echo = false;
    let mut got_ack = false;
    for _ in 0..20 {
        match next_packet(&mut a).await {
            Packet::Event(ev, data) if ev == "sayEcho" => {
                assert_eq!(data["routed"], json!("say"));
                assert_eq!(data["text"], json!("hi"));
                got_echo = true;
            }
            Packet::Ack(1, data) => {
                assert_eq!(data["routed"], json!("say"), "handler return is the ack");
                assert_eq!(data["text"], json!("hi"));
                got_ack = true;
            }
            _ => {}
        }
        if got_echo && got_ack {
            break;
        }
    }
    assert!(got_echo, "received the sayEcho event");
    assert!(got_ack, "received the native socket.io ack (handler return)");

    // Client B connects; A's `socket.broadcast("said", …)` should reach it.
    let mut b = sio_connect(server.port, "/im", "").await;
    let _ = expect_event(&mut b, "welcome").await;

    sio_emit(&mut a, "/im", "say", json!({ "text": "hello" }), None).await;
    let said = expect_event(&mut b, "said").await;
    assert_eq!(said["text"], json!("hello"), "broadcast reached the other client");
}

#[tokio::test]
async fn sio_compat_rooms() {
    let server = start_server().await;
    let mut a = sio_connect(server.port, "/im", "").await;
    let _ = expect_event(&mut a, "welcome").await;

    // Join a room; the listener namespace-broadcasts "roomNews" to that room,
    // which this socket is now a member of, so it should receive it.
    sio_emit(&mut a, "/im", "joinRoom", json!({ "room": "lobby" }), None).await;
    let news = expect_event(&mut a, "roomNews").await;
    assert_eq!(news["room"], json!("lobby"), "room-scoped namespace.emit reached the member");
}
