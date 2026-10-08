//! socket.io-lucee's **Java wrapper** surface
//! (`com.pixl8.socketiolucee.SocketIoServerWrapper`) over the engine's socket.io
//! transport.
//!
//! Where `websocket_sio_compat.rs` drives the engine's own `SocketIoServer.cfc`,
//! this drives the shape Preside's `preside-ext-socket-io` uses: the whole
//! server model stays in CFML and a Java object carries the wire, calling
//! `onConnect` / `onSocketEvent` back on the handler CFC. Without the shim that
//! `createObject` threw and took the application's boot with it.
//!
//! The fixture's Application.cfc is a cut-down stand-in for the library's
//! SocketIoServer.cfc — it makes the same calls in the same order.
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sio_wrapper_app")
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

/// Held from picking a port until the server is accepting on it: free_port()
/// binds :0 and releases it, so two tests starting at once can be handed the
/// same port. One server then fails to bind and exits, both readiness probes
/// still succeed against the survivor, and the loser's requests are refused
/// once the other test stops its server (an intermittent CI failure).
static START: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn start_server() -> Server {
    let _serialised = START.lock().unwrap_or_else(|e| e.into_inner());
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
        // A socket whose local and peer addresses match connected to ITSELF (Linux
        // TCP simultaneous open on an ephemeral-range port), not to a server.
        if std::net::TcpStream::connect(("127.0.0.1", port))
            .is_ok_and(|s| s.local_addr().ok() != s.peer_addr().ok())
        {
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


/// Connect carrying a cookie + query string, so the fixture can prove the HTTP
/// handshake survived the upgrade (Preside resolves a socket's session from its
/// `PSID` cookie, so dropping it would be a silent authentication hole).
async fn sio_connect_with_cookie(port: u16, ns: &str) -> Ws {
    let url = format!(
        "ws://127.0.0.1:{port}/socket.io/?EIO=4&transport=websocket&tenant=acme"
    );
    let req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&*url)
        .expect("build ws request");
    let mut req = req;
    req.headers_mut().insert(
        "cookie",
        "PSID=sess-123; CFID=cf-9".parse().expect("cookie header"),
    );
    let (mut ws, _) = connect_async(req).await.expect("ws connect");
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

#[tokio::test]
async fn sio_wrapper_connect_and_event_round_trip() {
    let server = start_server().await;

    // onConnect fired on the handler CFC, and its socketSend reached the client.
    let mut a = sio_connect_with_cookie(server.port, "/im").await;
    let welcome = expect_event(&mut a, "welcome").await;
    assert!(welcome["id"].is_string(), "the socket id reached the handler CFC");
    assert_eq!(
        welcome["psid"],
        json!("sess-123"),
        "the handshake cookie survived the upgrade (Preside reads PSID for the session)"
    );
    assert!(
        welcome["query"].as_str().unwrap_or_default().contains("tenant=acme"),
        "the handshake query string reached the handler CFC, got {:?}",
        welcome["query"]
    );

    // An inbound client event reaches onSocketEvent, whose reply goes back out
    // through the wrapper's socketSend.
    sio_emit(&mut a, "/im", "say", json!({ "text": "hi" }), None).await;
    let echo = expect_event(&mut a, "sayEcho").await;
    assert_eq!(echo["routed"], json!("say"));
    assert_eq!(echo["text"], json!("hi"), "the event payload reached CFML intact");
}
