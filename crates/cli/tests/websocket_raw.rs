//! Phase 1 raw-WebSocket integration tests.
//!
//! CFML can't act as a WS client, so these are the source of truth for the
//! realtime engine (the design's verification strategy). Each test spawns the
//! built `rustcfml` binary in serve mode against the `tests/fixtures/ws_app`
//! channel CFCs and drives it with a real `tokio-tungstenite` client.
//!
//! Acceptance (per docs/websocket-implementation-plan.md, Phase 1):
//!   * connect, send, receive an echo and a broadcast to a second client;
//!   * `onConnect` rejection closes the handshake;
//!   * disconnect auto-removes from rooms (no panic / clean teardown);
//!   * a `.cfm` page calling `wsPublish` reaches a connected client.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Ask the OS for an unused port, then release it so the child can bind it.
///
/// The release is unavoidable — the child binds the port itself — so there is
/// always a window in which another process can take it. With 11 tests starting
/// servers at once that window loses often enough to matter, so callers MUST
/// handle the child failing to bind; `start_server` retries on a fresh port.
///
/// Ports already handed out by this process are skipped: two tests drawing the
/// same port from the OS is the common case when many bind/close in a burst.
fn free_port() -> u16 {
    use std::sync::Mutex;
    static HANDED_OUT: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    for _ in 0..50 {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind ephemeral port")
            .local_addr()
            .unwrap()
            .port();
        let mut seen = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
        if !seen.contains(&port) {
            seen.push(port);
            return port;
        }
    }
    panic!("could not find an unused port in 50 attempts");
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ws_app")
}

/// A spawned server that is killed on drop.
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

/// Spawn a serve-mode server and return only once it is genuinely accepting.
///
/// This used to poll for up to 5s and then return the `Server` REGARDLESS of
/// whether anything was listening. When it lost the `free_port` race the child
/// died with "Address already in use (os error 48)" and exit code 1, but the
/// harness ignored that and handed back a dead server — so the test failed far
/// away at `connect_async` with a misleading `Connection refused`. Running the
/// 11 tests in parallel, that cost roughly one spurious failure per run, on a
/// different test each time.
///
/// Now: a child that exits is detected and retried on a fresh port, and a
/// server that never accepts panics with the child's own output instead of
/// being passed off as healthy.
async fn start_server() -> Server {
    const ATTEMPTS: usize = 5;
    const READY_TIMEOUT: Duration = Duration::from_secs(30);

    let mut last_failure = String::new();
    for attempt in 1..=ATTEMPTS {
        let port = free_port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
            .arg("--serve")
            .arg(fixtures_dir())
            .arg("--port")
            .arg(port.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn rustcfml --serve");

        let deadline = std::time::Instant::now() + READY_TIMEOUT;
        loop {
            match child.try_wait().expect("poll server child") {
                // Exited before accepting — lost the port race, or a real
                // startup failure. Either way its own stderr says which.
                Some(status) => {
                    last_failure = format!(
                        "attempt {attempt}: server exited ({status}) before accepting on port \
                         {port}{}",
                        child_output(&mut child)
                    );
                    break;
                }
                None => {}
            }
            if serves_http(port) {
                return Server { child, port };
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                last_failure = format!(
                    "attempt {attempt}: server never accepted on port {port} within \
                     {READY_TIMEOUT:?}{}",
                    child_output(&mut child)
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    panic!("could not start `rustcfml --serve` in {ATTEMPTS} attempts. Last: {last_failure}");
}

/// Is the server actually SERVING, not merely bound?
///
/// A bare `TcpStream::connect` only proves the listener exists. The bind happens
/// before the CFML application has booted and registered its WS channels, so a
/// connect-only probe let tests fire an upgrade into that window — where it is
/// answered with a 404/failure and the test dies on `connect_async(..).expect()`
/// somewhere far from the cause. One full HTTP round-trip forces the app through
/// boot, so by the time this returns true the channels are registered.
fn serves_http(port: u16) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    if s.write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 12];
    let mut got = 0;
    while got < buf.len() {
        match s.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return false,
        }
    }
    buf[..got].starts_with(b"HTTP/")
}

/// Drain whatever the child managed to say, for panic messages.
fn child_output(child: &mut Child) -> String {
    use std::io::Read;
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let mut buf = String::new();
        let _ = s.read_to_string(&mut buf);
        out.push_str(&buf);
    }
    if let Some(mut s) = child.stderr.take() {
        let mut buf = String::new();
        let _ = s.read_to_string(&mut buf);
        out.push_str(&buf);
    }
    if out.trim().is_empty() {
        String::new()
    } else {
        format!("\n--- server output ---\n{}", out.trim())
    }
}

/// Receive the next text frame, parsed as JSON, within a timeout.
async fn next_json<S>(stream: &mut S) -> serde_json::Value
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let fut = async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(t))) => {
                    return serde_json::from_str::<serde_json::Value>(&t).unwrap();
                }
                Some(Ok(_)) => continue,
                other => panic!("expected text frame, got {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("timed out waiting for a frame")
}

#[tokio::test]
async fn connect_echo_broadcast_and_publish() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/echo", server.port);

    // Client A connects → onConnect emits a "welcome".
    let (mut a, _) = connect_async(&base).await.expect("client A connects");
    let welcome = next_json(&mut a).await;
    assert_eq!(welcome["ev"], "welcome", "first frame is the welcome event");
    assert!(welcome["d"]["id"].is_string(), "welcome carries the socket id");

    // Client B connects too.
    let (mut b, _) = connect_async(&base).await.expect("client B connects");
    let _b_welcome = next_json(&mut b).await;

    // A sends a JSON message → A gets an "echo" + an "ack"; B gets "said".
    a.send(Message::Text(r#"{"text":"hello"}"#.into())).await.unwrap();

    // A's two frames (echo + ack), order not guaranteed across the two sends.
    let f1 = next_json(&mut a).await;
    let f2 = next_json(&mut a).await;
    let events: Vec<&str> = [&f1, &f2].iter().map(|f| f["ev"].as_str().unwrap()).collect();
    assert!(events.contains(&"echo"), "sender receives an echo: {events:?}");
    assert!(events.contains(&"ack"), "non-null return delivered as ack: {events:?}");
    let echo = if f1["ev"] == "echo" { &f1 } else { &f2 };
    assert_eq!(echo["d"]["text"], "hello", "echo carries the parsed payload");

    // B receives the broadcast (and NOT its own).
    let said = next_json(&mut b).await;
    assert_eq!(said["ev"], "said");
    assert_eq!(said["d"]["text"], "hello");

    // Emit-from-anywhere: an HTTP page hitting wsPublish reaches client A.
    let http = format!("http://127.0.0.1:{}/publish.cfm?msg=ping", server.port);
    let body = reqwest_get(&http).await;
    assert_eq!(body.trim(), "published");
    let announce = next_json(&mut a).await;
    assert_eq!(announce["ev"], "announcement");
    assert_eq!(announce["d"]["text"], "ping");

    drop(server);
}

#[tokio::test]
async fn handshake_param_and_onerror() {
    let server = start_server().await;

    // Handshake query param is exposed via socket.param().
    let url = format!("ws://127.0.0.1:{}/ws/echo?user=alice", server.port);
    let (mut a, _) = connect_async(&url).await.expect("connect with query param");
    let welcome = next_json(&mut a).await;
    assert_eq!(welcome["ev"], "welcome");
    assert_eq!(welcome["d"]["user"], "alice", "socket.param('user') round-trips");

    // A handler throw fires onError, which emits an "errored" event.
    a.send(Message::Text(r#"{"boom":true}"#.into())).await.unwrap();
    let errored = next_json(&mut a).await;
    assert_eq!(errored["ev"], "errored", "onError fired and emitted");
    assert!(
        errored["d"]["message"].as_str().unwrap().contains("boom"),
        "onError received the thrown message: {errored:?}"
    );
    drop(server);
}

#[tokio::test]
async fn event_routing_and_ack_ref() {
    let server = start_server().await;
    let url = format!("ws://127.0.0.1:{}/ws/echo", server.port);
    let (mut a, _) = connect_async(&url).await.expect("connect");
    let _welcome = next_json(&mut a).await;

    // An inbound frame naming an event routes to the `on="say"` handler (not
    // onMessage), and its `id` rides back on the ack's `ref`.
    a.send(Message::Text(r#"{"ev":"say","d":{"text":"hi"},"id":"req-1"}"#.into()))
        .await
        .unwrap();

    let f1 = next_json(&mut a).await;
    let f2 = next_json(&mut a).await;
    let by_ev = |ev: &str| -> &serde_json::Value {
        if f1["ev"] == ev {
            &f1
        } else {
            &f2
        }
    };

    let say_echo = by_ev("sayEcho");
    assert_eq!(say_echo["ev"], "sayEcho", "on=\"say\" handler ran: {f1:?} {f2:?}");
    assert_eq!(say_echo["d"]["routed"], "say", "routed via the on= annotation");
    assert_eq!(say_echo["d"]["text"], "hi", "payload came from the `d` field");

    let ack = if f1["t"] == "ack" { &f1 } else { &f2 };
    assert_eq!(ack["t"], "ack", "non-null return delivered as ack");
    assert_eq!(ack["ref"], "req-1", "ack correlates to the inbound id");
    assert_eq!(ack["d"]["routed"], "say");

    // A frame with no `ev` still falls through to onMessage (unchanged).
    a.send(Message::Text(r#"{"text":"plain"}"#.into())).await.unwrap();
    let g1 = next_json(&mut a).await;
    let g2 = next_json(&mut a).await;
    let events: Vec<&str> =
        [&g1, &g2].iter().map(|f| f["ev"].as_str().unwrap_or("")).collect();
    assert!(events.contains(&"echo"), "no-ev frame still hits onMessage: {events:?}");

    drop(server);
}

#[tokio::test]
async fn presence_state_diffs_and_roster() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/presence", server.port);

    // Collect every `user` meta out of a presence map (`{key:{metas:[{user}]}}`).
    fn users(map: &serde_json::Value) -> Vec<String> {
        let mut out = vec![];
        if let Some(obj) = map.as_object() {
            for v in obj.values() {
                if let Some(metas) = v["metas"].as_array() {
                    for m in metas {
                        if let Some(u) = m["user"].as_str() {
                            out.push(u.to_string());
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }

    // A connects → onConnect tracks → A gets a presence_state snapshot with itself.
    let (mut a, _) = connect_async(format!("{base}?user=alice")).await.expect("A connects");
    let a_state = next_json(&mut a).await;
    assert_eq!(a_state["ev"], "presence_state", "tracker gets a state snapshot");
    assert_eq!(users(&a_state["d"]), vec!["alice"], "snapshot has the tracker");

    // B connects → B gets the full snapshot; A gets a join diff for bob.
    let (mut b, _) = connect_async(format!("{base}?user=bob")).await.expect("B connects");
    let b_state = next_json(&mut b).await;
    assert_eq!(b_state["ev"], "presence_state");
    assert_eq!(users(&b_state["d"]), vec!["alice", "bob"], "B sees both");

    let a_join = next_json(&mut a).await;
    assert_eq!(a_join["ev"], "presence_diff");
    assert_eq!(users(&a_join["d"]["joins"]), vec!["bob"], "A learns bob joined");
    assert_eq!(users(&a_join["d"]["leaves"]), Vec::<String>::new());

    // io().presence() roster via an on="roster" handler → returned as an ack.
    b.send(Message::Text(r#"{"ev":"roster"}"#.into())).await.unwrap();
    let roster = next_json(&mut b).await;
    assert_eq!(roster["t"], "ack", "roster returned as an ack");
    assert_eq!(users(&roster["d"]), vec!["alice", "bob"], "roster lists everyone");

    // B disconnects → A gets a leave diff for bob (auto-untrack on teardown).
    drop(b);
    let a_leave = next_json(&mut a).await;
    assert_eq!(a_leave["ev"], "presence_diff");
    assert_eq!(users(&a_leave["d"]["leaves"]), vec!["bob"], "A learns bob left");
    assert_eq!(users(&a_leave["d"]["joins"]), Vec::<String>::new());

    drop(server);
}

#[tokio::test]
async fn secured_annotation_gates_handlers() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/auth", server.port);

    // An admin can call the admin-only handler.
    let (mut admin, _) = connect_async(format!("{base}?role=admin")).await.expect("admin connects");
    assert_eq!(next_json(&mut admin).await["ev"], "ready");
    admin.send(Message::Text(r#"{"ev":"admin"}"#.into())).await.unwrap();
    let f1 = next_json(&mut admin).await;
    let f2 = next_json(&mut admin).await;
    let evs: Vec<&str> = [&f1, &f2].iter().map(|f| f["ev"].as_str().unwrap_or("")).collect();
    assert!(evs.contains(&"adminOk"), "admin reached the secured handler: {evs:?}");

    // A guest is denied the admin-only handler (secured=\"admin\").
    let (mut guest, _) = connect_async(format!("{base}?role=guest")).await.expect("guest connects");
    assert_eq!(next_json(&mut guest).await["ev"], "ready");
    guest.send(Message::Text(r#"{"ev":"admin"}"#.into())).await.unwrap();
    let denied = next_json(&mut guest).await;
    assert_eq!(denied["ev"], "denied", "guest blocked from admin handler");
    assert!(
        denied["d"]["message"].as_str().unwrap().contains("Not authorized"),
        "onError saw the authorization failure: {denied:?}"
    );

    // A guest *is* authenticated, so a bare-`secured` handler lets them through.
    guest.send(Message::Text(r#"{"ev":"member"}"#.into())).await.unwrap();
    let m1 = next_json(&mut guest).await;
    let m2 = next_json(&mut guest).await;
    let mevs: Vec<&str> = [&m1, &m2].iter().map(|f| f["ev"].as_str().unwrap_or("")).collect();
    assert!(mevs.contains(&"memberOk"), "authenticated guest passed bare secured: {mevs:?}");

    drop(server);
}

#[tokio::test]
async fn canjoin_gates_room_joins() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/auth", server.port);
    let (mut ws, _) = connect_async(format!("{base}?role=admin")).await.expect("connect");
    assert_eq!(next_json(&mut ws).await["ev"], "ready");

    // An allowed room joins fine.
    ws.send(Message::Text(r#"{"ev":"join","d":{"room":"public-1"}}"#.into())).await.unwrap();
    let f1 = next_json(&mut ws).await;
    let f2 = next_json(&mut ws).await;
    let evs: Vec<&str> = [&f1, &f2].iter().map(|f| f["ev"].as_str().unwrap_or("")).collect();
    assert!(evs.contains(&"joined"), "allowed room joined: {evs:?}");

    // A disallowed room is rejected by canJoin → the handler throws → onError.
    ws.send(Message::Text(r#"{"ev":"join","d":{"room":"private-x"}}"#.into())).await.unwrap();
    let denied = next_json(&mut ws).await;
    assert_eq!(denied["ev"], "denied");
    assert!(
        denied["d"]["message"].as_str().unwrap().contains("canJoin"),
        "canJoin rejection surfaced: {denied:?}"
    );

    drop(server);
}

#[tokio::test]
async fn history_replays_missed_messages_on_reconnect() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/history", server.port);

    // A and B connect to the `history="50"` channel.
    let (mut a, _) = connect_async(&base).await.expect("A connects");
    let (mut b, _) = connect_async(&base).await.expect("B connects");

    // A emits three `say` events → each is broadcast channel-wide as `said`
    // (io().emit reaches A and B). Capture the wire ids B sees — those are the
    // resumability cursors.
    let mut ids = Vec::new();
    for n in 1..=3 {
        a.send(Message::Text(format!(r#"{{"ev":"say","d":{{"n":{n}}}}}"#).into()))
            .await
            .unwrap();
        let _a_said = next_json(&mut a).await; // drain A's own copy
        let said = next_json(&mut b).await;
        assert_eq!(said["ev"], "said");
        assert_eq!(said["d"]["n"], n);
        ids.push(said["id"].as_str().unwrap().to_string());
    }

    // B drops and reconnects with a cursor at the 2nd message.
    drop(b);
    let url = format!("{base}?lastEventId={}", ids[1]);
    let (mut b2, _) = connect_async(&url).await.expect("B reconnects");

    // Replay: B2 receives exactly the 3rd message (newer than the cursor),
    // keeping its original id so the client can keep advancing.
    let replayed = next_json(&mut b2).await;
    assert_eq!(replayed["ev"], "said");
    assert_eq!(replayed["d"]["n"], 3, "only the missed (3rd) message replays");
    assert_eq!(replayed["id"], ids[2], "replayed frame keeps its original id");

    // Live traffic resumes after replay.
    a.send(Message::Text(r#"{"ev":"say","d":{"n":4}}"#.into())).await.unwrap();
    let _a_said4 = next_json(&mut a).await;
    let live = next_json(&mut b2).await;
    assert_eq!(live["d"]["n"], 4, "live traffic resumes after replay");

    drop(server);
}

#[tokio::test]
async fn binary_frames_round_trip() {
    let server = start_server().await;
    let url = format!("ws://127.0.0.1:{}/ws/raw", server.port);
    let (mut ws, _) = connect_async(&url).await.expect("connect raw channel");

    let payload = vec![0u8, 1, 2, 250, 255];
    ws.send(Message::Binary(payload.clone().into())).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => return b.to_vec(),
                Some(Ok(_)) => continue,
                other => panic!("expected a binary frame, got {other:?}"),
            }
        }
    })
    .await
    .expect("timed out waiting for binary echo");
    assert_eq!(got, payload, "binary payload echoes back as binary");
    drop(server);
}

#[tokio::test]
async fn onconnect_rejection_closes_handshake() {
    let server = start_server().await;
    let url = format!("ws://127.0.0.1:{}/ws/guarded", server.port);
    // The upgrade itself succeeds (HTTP 101), but onConnect returns false so
    // the server immediately closes. The client sees the stream end with a
    // close, not a welcome.
    // The close races the handshake: usually the upgrade completes (HTTP 101)
    // and the close frame lands just after, but the server is entitled to drop
    // the connection before the client has read the 101, in which case
    // `connect_async` itself errors. Both are the same rejection, so accept
    // either — insisting on the first ordering made this test fail sporadically.
    // What must NOT happen is the connection being fed data, and that still fails.
    let closed = match connect_async(&url).await {
        Err(_) => true,
        Ok((mut ws, _)) => tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Close(_))) | None => return true,
                    Some(Ok(_)) => return false, // any data frame = not rejected
                    Some(Err(_)) => return true,
                }
            }
        })
        .await
        .expect("timed out"),
    };
    assert!(closed, "rejected connection should be closed, not fed data");
    drop(server);
}

#[tokio::test]
async fn unknown_channel_is_404() {
    let server = start_server().await;
    let url = format!("ws://127.0.0.1:{}/ws/nope", server.port);
    let res = connect_async(&url).await;
    assert!(res.is_err(), "unknown channel should fail the upgrade (404)");
    drop(server);
}

#[tokio::test]
async fn whisper_relays_to_peers_without_running_cfml() {
    let server = start_server().await;
    let base = format!("ws://127.0.0.1:{}/ws/whisper", server.port);

    // A and B both connect and auto-join "lobby" in onConnect.
    let (mut a, _) = connect_async(&base).await.expect("A connects");
    assert_eq!(next_json(&mut a).await["ev"], "ready");
    let (mut b, _) = connect_async(&base).await.expect("B connects");
    assert_eq!(next_json(&mut b).await["ev"], "ready");

    // 1. Room-targeted whisper to "lobby" (A is a member) → reaches B.
    a.send(Message::Text(r#"{"ev":"client-typing","d":{"phase":"room"},"room":"lobby"}"#.into()))
        .await
        .unwrap();
    // 2. Room-targeted whisper to a room A is NOT in → silently dropped.
    a.send(Message::Text(r#"{"ev":"client-cursor","d":{"x":1},"room":"nope"}"#.into()))
        .await
        .unwrap();
    // 3. Channel-wide whisper → reaches B.
    a.send(Message::Text(r#"{"ev":"client-typing","d":{"phase":"all"}}"#.into()))
        .await
        .unwrap();

    // B sees the room whisper, then the channel-wide one — never the dropped
    // cursor (proving the membership check), and each is a `t:"client"` frame.
    let first = next_json(&mut b).await;
    assert_eq!(first["t"], "client", "relayed as a client whisper frame");
    assert_eq!(first["ev"], "client-typing");
    assert_eq!(first["d"]["phase"], "room", "room-targeted whisper delivered");
    let second = next_json(&mut b).await;
    assert_eq!(second["d"]["phase"], "all", "cursor→nope was dropped, not relayed");

    // The hub ran no CFML for the whispers: a normal message still triggers the
    // onMessage broadcast, proving the connection is intact and whispers bypass
    // the handler entirely.
    a.send(Message::Text(r#"{"text":"hi"}"#.into())).await.unwrap();
    let said = next_json(&mut b).await;
    assert_eq!(said["ev"], "said");
    assert_eq!(said["d"]["text"], "hi");

    drop(server);
}

// ── Phase 2: distributed Broker (real 2-process gossip cluster) ───────────
//
// End-to-end verification that WebSocket fan-out crosses nodes over the shared
// `memberlist` gossip cluster: a broadcast on node A reaches a client connected
// to node B. Spawns two clustered `rustcfml --serve` processes that gossip on
// loopback.
//
// `#[ignore]` by default — it is timing-sensitive (waits on gossip convergence)
// and spins up two full server processes, so it is unsuitable for the routine
// parallel test run. The deterministic cross-node semantics are covered without
// this by `cfml-vm`'s `two_node_cluster_fan_out_and_presence` /
// `broker_msg_round_trips_through_json` unit tests; this exercises the *real
// transport + cli wiring*. Run it explicitly:
//   cargo test -p rustcfml-cli --features cluster --test websocket_raw -- --ignored
#[cfg(feature = "cluster")]
fn start_cluster_node(http_port: u16, gossip_port: u16, peer_gossip: u16, node: &str) -> Child {
    use std::io::Write;
    let cfg_path = std::env::temp_dir().join(format!("rustcfml_ws_cluster_{node}_{gossip_port}.json"));
    let cfg = format!(
        r#"{{"sessionStorage":"clusterCache","caches":{{"clusterCache":{{"provider":"cluster","storage":true,"properties":{{"listenAddr":"127.0.0.1:{gossip_port}","nodeName":"{node}","seeds":["127.0.0.1:{peer_gossip}"]}}}}}}}}"#
    );
    std::fs::File::create(&cfg_path).unwrap().write_all(cfg.as_bytes()).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(http_port.to_string())
        .arg("--cfconfig")
        .arg(&cfg_path)
        .spawn()
        .expect("spawn clustered rustcfml --serve");
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", http_port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child
}

#[tokio::test]
#[ignore = "real 2-process gossip cluster; timing-sensitive — run with --features cluster -- --ignored"]
#[cfg(feature = "cluster")]
async fn cluster_broadcast_reaches_client_on_other_node() {
    let (ga, gb) = (free_port(), free_port());
    let (ha, hb) = (free_port(), free_port());
    let mut node_a = start_cluster_node(ha, ga, gb, "nodeA");
    let mut node_b = start_cluster_node(hb, gb, ga, "nodeB");
    // Give the gossip cluster time to converge (LAN profile is sub-second, but
    // both processes also need their channel CFCs ready).
    tokio::time::sleep(Duration::from_secs(2)).await;

    let (mut a, _) = connect_async(format!("ws://127.0.0.1:{ha}/ws/echo"))
        .await
        .expect("client A connects to node A");
    let (mut b, _) = connect_async(format!("ws://127.0.0.1:{hb}/ws/echo"))
        .await
        .expect("client B connects to node B");

    // A sends a message → echo.onMessage broadcasts "said" channel-wide → it must
    // cross the cluster and arrive at B (connected to the other node).
    a.send(Message::Text("{\"hello\":\"world\"}".into())).await.unwrap();

    // B's stream begins with its own onConnect "welcome"; wait for the "said".
    let said = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let f = next_json(&mut b).await;
            if f["ev"] == "said" {
                return f;
            }
        }
    })
    .await
    .expect("cross-node broadcast did not reach the client on node B");
    assert_eq!(said["d"]["hello"], "world", "payload survives the cross-node hop");

    let _ = node_a.kill();
    let _ = node_b.kill();
}

/// Minimal HTTP GET without pulling in a full client crate — just enough to
/// fetch a tiny CFML page body.
async fn reqwest_get(url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let url = url.strip_prefix("http://").unwrap();
    let (host_port, path) = url.split_once('/').unwrap();
    let mut stream = tokio::net::TcpStream::connect(host_port).await.unwrap();
    let req = format!(
        "GET /{path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    // Body is after the blank line.
    text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
}
