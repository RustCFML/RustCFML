//! MCP Streamable-HTTP transport integration tests.
//!
//! Spawns the real binary in serve mode against `tests/fixtures/mcp_app` and
//! drives `/mcp/demo` with raw HTTP, so the session rules, status codes and
//! headers the spec pins are checked on the wire rather than in a unit test.
//!
//! The HTTP client is hand-rolled (`Connection: close`, read to EOF) to keep
//! this suite dependency-free.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use serde_json::{json, Value};

/// Ask the OS for an unused port, then release it so the child can bind it.
/// Ports already handed out by this process are skipped — two tests drawing
/// the same port is the common failure when several bind in a burst.
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_app")
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

/// One HTTP response, parsed just enough for these assertions.
struct Res {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Res {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body was not JSON: {e}\nbody: {}", self.body))
    }
}

impl Server {
    /// Spawn a serve-mode server and return only once it is genuinely serving.
    /// A child that exits (lost port race, or a real failure) is retried on a
    /// fresh port rather than handed back dead.
    fn start() -> Self {
        const ATTEMPTS: usize = 5;
        let mut last = String::new();
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

            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = child.try_wait().expect("poll child") {
                    last = format!("attempt {attempt}: server exited ({status}) on port {port}");
                    break;
                }
                if serves_http(port) {
                    return Server { child, port };
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    last = format!("attempt {attempt}: never accepted on port {port}");
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        panic!("could not start `rustcfml --serve` in {ATTEMPTS} attempts. Last: {last}");
    }

    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Res {
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            self.port
        );
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if !body.is_empty() {
            req.push_str("Content-Type: application/json\r\n");
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        req.push_str(body);

        let mut sock = std::net::TcpStream::connect(("127.0.0.1", self.port))
            .expect("connect to the test server");
        sock.set_read_timeout(Some(Duration::from_secs(30))).expect("timeout");
        sock.write_all(req.as_bytes()).expect("write request");
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).expect("read response");
        parse_response(&String::from_utf8_lossy(&raw))
    }

    fn post(&self, headers: &[(&str, &str)], msg: Value) -> Res {
        self.request("POST", "/mcp/demo", headers, &msg.to_string())
    }

    /// Complete the handshake and return the session id the server minted.
    fn initialize(&self) -> String {
        let res = self.post(
            &[("Accept", "application/json, text/event-stream")],
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {} }
            }),
        );
        assert_eq!(res.status, 200, "body: {}", res.body);
        res.header("mcp-session-id")
            .unwrap_or_else(|| panic!("initialize must return an Mcp-Session-Id"))
            .to_string()
    }
}

/// Decode `Transfer-Encoding: chunked` framing. Streaming responses set no
/// Content-Length, so hyper chunks them and the raw bytes carry the frames.
fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else { break };
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or(""), 16);
        let Ok(size) = size else { break };
        if size == 0 || after.len() < size {
            out.push_str(&after[..size.min(after.len())]);
            break;
        }
        out.push_str(&after[..size]);
        rest = after[size..].strip_prefix("\r\n").unwrap_or("");
    }
    out
}

/// Split an SSE body into `(event id, payload)` pairs, ignoring `:` comments.
fn sse_events(body: &str) -> Vec<(String, Value)> {
    let mut events = Vec::new();
    for block in body.split("\n\n") {
        let mut id = String::new();
        let mut data = String::new();
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("id: ") {
                id = v.to_string();
            } else if let Some(v) = line.strip_prefix("data: ") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v);
            }
        }
        if data.is_empty() {
            continue;
        }
        let parsed = serde_json::from_str(&data)
            .unwrap_or_else(|e| panic!("SSE data was not JSON: {e}\ndata: {data}"));
        events.push((id, parsed));
    }
    events
}

fn parse_response(raw: &str) -> Res {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line in: {raw}"));
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked { dechunk(body) } else { body.to_string() };
    Res { status, headers, body }
}

/// Open the standalone GET stream and read until `want` events have arrived or
/// the deadline passes. A live SSE stream never reaches EOF, so a plain
/// read-to-end would simply block until the socket timeout.
fn read_stream(port: u16, session: &str, want: usize, timeout: Duration) -> Vec<(String, Value)> {
    read_stream_with(port, &format!("Mcp-Session-Id: {session}\r\n"), want, timeout)
}

fn read_stream_with(
    port: u16,
    extra_headers: &str,
    want: usize,
    timeout: Duration,
) -> Vec<(String, Value)> {
    let req = format!(
        "GET /mcp/demo HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Accept: text/event-stream\r\n{extra_headers}\r\n"
    );
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_millis(250))).expect("timeout");
    sock.write_all(req.as_bytes()).expect("write");

    let deadline = std::time::Instant::now() + timeout;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    while std::time::Instant::now() < deadline {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            // A read timeout just means nothing has arrived yet.
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => panic!("stream read failed: {e}"),
        }
        let text = String::from_utf8_lossy(&raw).to_string();
        let Some((_, body)) = text.split_once("\r\n\r\n") else { continue };
        let events = sse_events(&dechunk(body));
        if events.len() >= want {
            return events;
        }
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    match text.split_once("\r\n\r\n") {
        Some((_, body)) => sse_events(&dechunk(body)),
        None => Vec::new(),
    }
}

/// Like [`read_stream`], but resuming from a `Last-Event-ID` cursor.
fn read_stream_resuming(
    port: u16,
    session: &str,
    last_event_id: &str,
    want: usize,
    timeout: Duration,
) -> Vec<(String, Value)> {
    read_stream_with(
        port,
        &format!(
            "Mcp-Session-Id: {session}\r\nLast-Event-ID: {last_event_id}\r\n"
        ),
        want,
        timeout,
    )
}

/// POST a message and stream the SSE events back as they arrive.
///
/// A sampling round trip needs this: the tool's request appears on the stream
/// while the call is still running, and the answer must be POSTed on a
/// *different* connection before the stream will produce the final result.
fn post_streaming(
    port: u16,
    session: &str,
    body: Value,
) -> std::sync::mpsc::Receiver<(String, Value)> {
    let body = body.to_string();
    let req = format!(
        "POST /mcp/demo HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Accept: application/json, text/event-stream\r\nMcp-Session-Id: {session}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut sock =
            std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        sock.set_read_timeout(Some(Duration::from_millis(250))).expect("timeout");
        sock.write_all(req.as_bytes()).expect("write");

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut raw = Vec::new();
        let mut sent = 0usize;
        let mut buf = [0u8; 4096];
        while std::time::Instant::now() < deadline {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => panic!("stream read failed: {e}"),
            }
            let text = String::from_utf8_lossy(&raw).to_string();
            let Some((_, body)) = text.split_once("\r\n\r\n") else { continue };
            let events = sse_events(&dechunk(body));
            for event in events.iter().skip(sent) {
                if tx.send(event.clone()).is_err() {
                    return;
                }
            }
            sent = events.len();
        }
    });
    rx
}

fn serves_http(port: u16) -> bool {
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

#[test]
fn initialize_mints_a_session_and_later_calls_reuse_it() {
    let server = Server::start();
    let session = server.initialize();
    // The id travels in an HTTP header, so it must be visible ASCII only.
    assert!(
        session.bytes().all(|b| (0x21..=0x7E).contains(&b)),
        "session id is not header-safe: {session}"
    );

    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "echo", "arguments": { "text": "hi", "times": 2 } } }),
    );
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["result"]["content"][0]["text"], "hihi");
}

#[test]
fn the_session_gate_distinguishes_missing_from_expired() {
    let server = Server::start();
    server.initialize();

    // Missing id on a non-initialize request: the client sent a bad request.
    let res = server.post(&[], json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    assert_eq!(res.status, 400, "body: {}", res.body);

    // Unknown id: 404 is how a client is told to start a new session. Sending
    // 400 here would make it retry the same dead session forever.
    let res = server.post(
        &[("Mcp-Session-Id", "no-such-session")],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    assert_eq!(res.status, 404, "body: {}", res.body);
}

#[test]
fn deleting_a_session_ends_it() {
    let server = Server::start();
    let session = server.initialize();

    let res = server.request("DELETE", "/mcp/demo", &[("Mcp-Session-Id", session.as_str())], "");
    assert_eq!(res.status, 204);

    // The session is genuinely gone, not merely marked.
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
    );
    assert_eq!(res.status, 404);

    let res = server.request("DELETE", "/mcp/demo", &[], "");
    assert_eq!(res.status, 400, "a DELETE with no session id is a bad request");
}

#[test]
fn notifications_are_accepted_with_no_body() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    assert_eq!(res.status, 202, "the spec pins 202 Accepted with no body");
    assert!(res.body.trim().is_empty(), "got body: {}", res.body);
}

#[test]
fn a_cross_origin_request_is_refused() {
    let server = Server::start();
    // The DNS-rebinding defence the spec makes a MUST: a web page must not be
    // able to drive a local MCP server.
    let res = server.post(
        &[("Origin", "https://evil.example.com")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    assert_eq!(res.status, 403);

    // A local origin is fine, and so is no origin at all (non-browser clients).
    let res = server.post(
        &[("Origin", "http://localhost:1234")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {} } }),
    );
    assert_eq!(res.status, 200);
}

#[test]
fn an_unsupported_protocol_version_header_is_rejected() {
    let server = Server::start();
    let res = server.post(
        &[("MCP-Protocol-Version", "1999-01-01")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    assert_eq!(res.status, 400);
    assert_eq!(res.json()["error"]["code"], -32600);
}

#[test]
fn malformed_json_is_a_parse_error() {
    let server = Server::start();
    let res = server.request("POST", "/mcp/demo", &[], "not json at all");
    assert_eq!(res.status, 400);
    assert_eq!(res.json()["error"]["code"], -32700);
}

#[test]
fn the_mcp_route_does_not_shadow_the_applications_own_pages() {
    let server = Server::start();
    // `mcp/page.cfm` is a real template living under the same URL prefix. A
    // registered `/mcp/{name}` route must fall through to it rather than
    // answering 404 for everything that is not a server CFC.
    let res = server.request("GET", "/mcp/page.cfm", &[], "");
    assert_eq!(res.status, 200, "body: {}", res.body);
    assert!(res.body.contains("hello from a real page"), "body: {}", res.body);
}

#[test]
fn tools_list_over_http_matches_the_declared_surface() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let tools = res.json()["result"]["tools"].as_array().expect("tools").clone();
    let names: Vec<String> =
        tools.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect();
    assert!(names.contains(&"echo".to_string()), "got {names:?}");
    assert!(!names.contains(&"helper".to_string()), "private method exposed: {names:?}");
}

#[test]
fn a_progress_token_opens_an_sse_stream_carrying_notifications_then_the_response() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[
            ("Mcp-Session-Id", session.as_str()),
            ("Accept", "application/json, text/event-stream"),
        ],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "slow", "arguments": { "steps": 2 },
                            "_meta": { "progressToken": "tok-1" } } }),
    );
    assert_eq!(res.status, 200);
    assert_eq!(
        res.header("content-type").unwrap_or(""),
        "text/event-stream",
        "a progress token must be answered with a stream"
    );
    // Proxies buffer event streams by default, which would deliver every
    // notification at the very end — defeating the point.
    assert_eq!(res.header("x-accel-buffering"), Some("no"));
    assert!(res.header("content-length").is_none(), "a stream must not be length-framed");

    let events = sse_events(&res.body);
    let methods: Vec<&str> =
        events.iter().filter_map(|(_, v)| v["method"].as_str()).collect();
    assert_eq!(
        methods,
        vec![
            "notifications/progress",
            "notifications/message",
            "notifications/progress",
            "notifications/message"
        ],
        "got {methods:?}"
    );

    let first = &events[0].1;
    assert_eq!(first["params"]["progressToken"], "tok-1");
    assert_eq!(first["params"]["progress"], 1);
    assert_eq!(first["params"]["total"], 2);
    assert_eq!(first["params"]["message"], "step 1");

    // The response for the originating request is the LAST event on the stream.
    let (_, last) = events.last().expect("at least one event");
    assert_eq!(last["id"], 2);
    assert_eq!(last["result"]["content"][0]["text"], "finished 2 steps");
    assert_eq!(last["result"]["isError"], false);

    // Event ids are per-stream and monotonic, which is what Last-Event-ID
    // resumption keys on.
    let ids: Vec<&str> = events.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.first(), Some(&"0-0"), "got {ids:?}");
    assert!(ids.windows(2).all(|w| w[0] != w[1]), "ids must be distinct: {ids:?}");
}

#[test]
fn the_streaming_annotation_opens_a_stream_without_a_progress_token() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str()), ("Accept", "text/event-stream")],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "slow", "arguments": { "steps": 1 } } }),
    );
    assert_eq!(res.header("content-type"), Some("text/event-stream"));

    let events = sse_events(&res.body);
    // No progress token, so progress is suppressed (unsolicited progress is a
    // spec violation) but logging still flows, and the response still arrives.
    let methods: Vec<&str> =
        events.iter().filter_map(|(_, v)| v["method"].as_str()).collect();
    assert_eq!(methods, vec!["notifications/message"], "got {methods:?}");
    assert_eq!(events.last().unwrap().1["result"]["content"][0]["text"], "finished 1 steps");
}

#[test]
fn a_plain_tool_call_stays_on_a_json_response() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[
            ("Mcp-Session-Id", session.as_str()),
            ("Accept", "application/json, text/event-stream"),
        ],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "echo", "arguments": { "text": "x" } } }),
    );
    assert!(
        res.header("content-type").unwrap_or("").contains("application/json"),
        "a non-streaming tool should not pay for a stream"
    );
    assert_eq!(res.json()["result"]["content"][0]["text"], "x");
}

#[test]
fn a_client_that_will_not_accept_a_stream_gets_json_even_with_a_progress_token() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str()), ("Accept", "application/json")],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "slow", "arguments": { "steps": 1 },
                            "_meta": { "progressToken": 1 } } }),
    );
    assert_eq!(res.status, 200);
    assert!(res.header("content-type").unwrap_or("").contains("application/json"));
    assert_eq!(res.json()["result"]["content"][0]["text"], "finished 1 steps");
}

#[test]
fn the_standalone_get_stream_opens_for_a_client_that_accepts_it() {
    let server = Server::start();
    let session = server.initialize();

    // A client that cannot read an event stream gets the spec's 405, not a
    // 200 carrying something it will fail to parse.
    let res = server.request("GET", "/mcp/demo", &[("Mcp-Session-Id", session.as_str())], "");
    assert_eq!(res.status, 405);
    assert!(res.header("allow").is_some(), "405 must name the allowed methods");

    // A GET with no session is a bad request, not a stream.
    let res = server.request("GET", "/mcp/demo", &[("Accept", "text/event-stream")], "");
    assert_eq!(res.status, 400);
}

#[test]
fn mcp_notify_reaches_a_listener_on_the_standalone_stream() {
    let server = Server::start();
    let session = server.initialize();

    // Hold the standalone stream open on another thread, as a real client does.
    let port = server.port;
    let listening = session.clone();
    let reader = std::thread::spawn(move || {
        read_stream(port, &listening, 1, Duration::from_secs(20))
    });
    // Give the GET time to register its stream before anything is published.
    std::thread::sleep(Duration::from_millis(300));

    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "announce", "arguments": { "text": "ship it" } } }),
    );
    // mcpNotify returns how many sessions it reached.
    assert_eq!(res.json()["result"]["content"][0]["text"], "1", "body: {}", res.body);

    let events = reader.join().expect("reader thread");
    let announcement = events
        .iter()
        .find(|(_, v)| v["method"] == "notifications/announcement")
        .unwrap_or_else(|| panic!("no announcement on the stream; got {events:?}"));
    assert_eq!(announcement.1["params"]["text"], "ship it");
}

#[test]
fn the_call_context_is_visible_to_a_tool() {
    let server = Server::start();
    let session = server.initialize();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "where_am_i", "arguments": {} } }),
    );
    let structured = &res.json()["result"]["structuredContent"];
    assert_eq!(structured["server"], "/mcp/demo");
    assert_eq!(structured["streaming"], false, "this call was answered as JSON");
}

#[test]
fn last_event_id_replays_what_a_dropped_client_missed() {
    let server = Server::start();
    let session = server.initialize();

    // First connection: take one announcement, then drop the socket.
    let port = server.port;
    let listening = session.clone();
    let reader =
        std::thread::spawn(move || read_stream(port, &listening, 1, Duration::from_secs(20)));
    std::thread::sleep(Duration::from_millis(300));
    server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "announce", "arguments": { "text": "first" } } }),
    );
    let seen = reader.join().expect("reader");
    let (last_id, first) = seen.last().expect("one event").clone();
    assert_eq!(first["params"]["text"], "first");
    // The socket is closed now (the thread returned), so this next one is sent
    // while nobody is listening — exactly the case resumption exists for.
    server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "announce", "arguments": { "text": "missed" } } }),
    );

    // Reconnect with the cursor: the missed event is replayed.
    let replayed = read_stream_resuming(port, &session, &last_id, 1, Duration::from_secs(20));
    let texts: Vec<&str> =
        replayed.iter().filter_map(|(_, v)| v["params"]["text"].as_str()).collect();
    assert!(texts.contains(&"missed"), "expected the missed event, got {texts:?}");
    assert!(
        !texts.contains(&"first"),
        "only events strictly after the cursor may be replayed: {texts:?}"
    );
}

#[test]
fn a_tool_can_ask_the_clients_model_and_block_until_it_answers() {
    let server = Server::start();
    // The client must declare `sampling`, or the server refuses rather than
    // asking a client that cannot answer.
    let res = server.post(
        &[("Accept", "application/json, text/event-stream")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18",
                            "capabilities": { "sampling": {} } } }),
    );
    let session = res.header("mcp-session-id").expect("session").to_string();

    let events = post_streaming(
        server.port,
        &session,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "ask", "arguments": { "question": "why?" } } }),
    );

    // The tool's request to us arrives on the stream while the call is parked.
    let (_, request) = events
        .recv_timeout(Duration::from_secs(20))
        .expect("a sampling request should arrive");
    assert_eq!(request["method"], "sampling/createMessage");
    assert_eq!(request["params"]["messages"][0]["content"]["text"], "why?");
    assert_eq!(request["params"]["maxTokens"], 100);
    let request_id = request["id"].clone();
    assert!(!request_id.is_null(), "a server request needs an id to answer");

    // Answer on a different connection, as a real client does.
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": request_id,
                "result": { "role": "assistant", "model": "test-model",
                            "content": { "type": "text", "text": "because" } } }),
    );
    assert_eq!(res.status, 202, "a response carries no reply of its own");

    // The parked tool wakes, uses the answer, and its result closes the stream.
    let (_, result) = events
        .recv_timeout(Duration::from_secs(20))
        .expect("the tool should finish once answered");
    assert_eq!(result["id"], 2);
    assert_eq!(result["result"]["content"][0]["text"], "model said: because");
}

#[test]
fn sampling_from_a_non_streaming_tool_fails_with_advice_rather_than_hanging() {
    let server = Server::start();
    let res = server.post(
        &[("Accept", "application/json, text/event-stream")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18",
                            "capabilities": { "sampling": {} } } }),
    );
    let session = res.header("mcp-session-id").expect("session").to_string();

    let started = std::time::Instant::now();
    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "ask_unstreamed", "arguments": { "question": "why?" } } }),
    );
    // There is no stream to carry the request, so the call must fail at once
    // and say what to change — not park for the timeout.
    assert!(started.elapsed() < Duration::from_secs(20), "it parked instead of failing");
    let body = res.json();
    let text = body["result"]["content"][0]["text"].as_str().expect("message");
    assert!(text.contains("streaming=true"), "got {text:?}");
}

#[test]
fn sampling_is_refused_when_the_client_never_declared_it() {
    let server = Server::start();
    // Capabilities omitted at initialize.
    let session = server.initialize();
    let events = post_streaming(
        server.port,
        &session,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "ask", "arguments": { "question": "why?" } } }),
    );
    let (_, result) = events
        .recv_timeout(Duration::from_secs(20))
        .expect("it should fail rather than wait for an answer that cannot come");
    assert_eq!(result["id"], 2);
    let text = result["result"]["content"][0]["text"].as_str().expect("message");
    assert!(text.contains("does not support sampling"), "got {text:?}");
}

#[test]
fn setting_a_log_level_silences_messages_below_it() {
    let server = Server::start();
    let session = server.initialize();

    let res = server.post(
        &[("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "logging/setLevel",
                "params": { "level": "error" } }),
    );
    assert_eq!(res.status, 200);
    assert!(res.json().get("error").is_none(), "body: {}", res.body);

    // `slow` logs at info, which is now below the client's threshold, so the
    // stream carries the response and nothing else.
    let res = server.post(
        &[
            ("Mcp-Session-Id", session.as_str()),
            ("Accept", "application/json, text/event-stream"),
        ],
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "slow", "arguments": { "steps": 2 } } }),
    );
    let events = sse_events(&res.body);
    let methods: Vec<&str> =
        events.iter().filter_map(|(_, v)| v["method"].as_str()).collect();
    assert!(
        methods.is_empty(),
        "info-level logs must be filtered out once the client asks for error: {methods:?}"
    );
    assert_eq!(events.last().unwrap().1["id"], 3, "the response still arrives");
}
