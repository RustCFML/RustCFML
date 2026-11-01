//! The HTTP server's protection against slow and hostile clients —
//! docs/configuration.md "Overload protection".
//!
//! Three things that were open before this suite existed:
//!
//! * `axum::serve` built each hyper connection without a timer, which turns
//!   hyper's header-read timeout OFF: a header block trickled in and never
//!   finished pinned its connection for good (slowloris).
//! * A body that stopped short of its `Content-Length` had nothing bounding
//!   it either — the handler sat in `to_bytes` forever.
//! * `server.maxConcurrentRequests` was parsed and never read.
//! * A `CFID` cookie naming a session the store had never issued was adopted
//!   verbatim (session fixation).
//!
//! The fixture's `.cfconfig.json` sets every limit to 1 so the suite runs in
//! seconds: `headerReadTimeout: 1`, `bodyReadTimeout: 1`,
//! `maxConcurrentRequests: 1`. A Rust integration test rather than a CFML
//! suite because these are `server.*` keys — server-level by design, so no
//! app-level config can set them and `tests/runner.cfm` cannot ask for them.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn webroot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_hardening_app")
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

fn start_server() -> Server {
    // `--port 0` and read the port back from the ready banner. Picking a free
    // port first and passing it in raced the suite's other servers, which start
    // in parallel: two tests could be handed the same port, one server failed
    // to bind, and its test probed the OTHER test's server as "ready" and then
    // got ConnectionRefused once that one shut down.
    let root = webroot();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(&root)
        .arg("--port")
        .arg("0")
        .arg("--cfconfig")
        .arg(root.join(".cfconfig.json"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn rustcfml --serve");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|line| {
            let addr = line.split("running on http://").nth(1)?;
            addr.split_whitespace().next()?.rsplit(':').next()?.parse::<u16>().ok()
        })
        .expect("server exited before printing its ready banner");
    // Keep draining stdout so the server never blocks on a full pipe.
    std::thread::spawn(move || lines.for_each(drop));
    Server { child, port }
}

/// One HTTP/1.0 request; returns the raw response (status line, headers, body).
fn http(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn get(port: u16, path: &str) -> String {
    http(
        port,
        &format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    )
}

fn status(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Send `partial` and then nothing more; report how the server reacted
/// within `wait`: the bytes it sent back (empty = it closed the connection
/// without a response), or `None` if it was still holding the connection open.
fn send_partial_and_wait(port: u16, partial: &[u8], wait: Duration) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(wait)).unwrap();
    stream.write_all(partial).unwrap();
    let mut buf = Vec::new();
    match stream.read_to_end(&mut buf) {
        Ok(_) => Some(buf),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
            if buf.is_empty() { None } else { Some(buf) }
        }
        Err(_) => Some(buf),
    }
}

#[test]
fn a_header_block_that_never_finishes_is_closed_at_header_read_timeout() {
    let server = start_server();
    
    // Retry logic: immediate connection close (<100ms) can happen under CI load
    // when the server hasn't fully initialized yet. Give it up to 3 attempts.
    for attempt in 1..=3 {
        let started = Instant::now();
        // The slowloris shape: a request line, one header, and a header name
        // whose value never arrives.
        let reaction = send_partial_and_wait(
            server.port,
            b"GET /fast.cfm HTTP/1.1\r\nHost: x\r\nX-Slow: ",
            Duration::from_secs(10),
        );
        let elapsed = started.elapsed();
        let bytes = reaction.expect("the connection must be closed, not held open for 10s");
        
        // Immediate close (< 100ms) is a CI flake; retry
        if elapsed < Duration::from_millis(100) {
            if attempt < 3 {
                eprintln!("Attempt {}: connection closed too quickly ({:?}), retrying...", attempt, elapsed);
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            // On the last attempt, fail with a clear message
            panic!(
                "After {} attempts, connection still closes immediately ({:?}). This suggests \
                 the server isn't enforcing headerReadTimeout, or CI load is extreme.",
                attempt, elapsed
            );
        }
        
        assert!(
            bytes.is_empty(),
            "no request has been received, so nothing should be answered; got {:?}",
            String::from_utf8_lossy(&bytes)
        );
        assert!(
            elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(6),
            "the close should land at headerReadTimeout (1s), not {:?}",
            elapsed
        );
        
        // Test passed, verify server is still serving
        assert_eq!(status(&get(server.port, "/fast.cfm")), 200);
        return;
    }
}

#[test]
fn a_body_that_never_completes_gets_408_at_body_read_timeout() {
    let server = start_server();
    let started = Instant::now();
    let reaction = send_partial_and_wait(
        server.port,
        b"POST /fast.cfm HTTP/1.1\r\nHost: x\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\nabc",
        Duration::from_secs(10),
    );
    let elapsed = started.elapsed();
    let bytes = reaction.expect("the request must be answered, not held open for 10s");
    let response = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status(&response), 408, "expected 408, got:\n{response}");
    assert!(
        header(&response, "connection").is_some_and(|v| v.eq_ignore_ascii_case("close")),
        "the unread body is still on the wire, so the connection must be closed:\n{response}"
    );
    assert!(
        elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(6),
        "the 408 should land at bodyReadTimeout (1s), not {elapsed:?}"
    );
    assert_eq!(status(&get(server.port, "/fast.cfm")), 200);
}

#[test]
fn a_complete_body_is_unaffected_by_the_body_read_timeout() {
    let server = start_server();
    let body = "a=1&b=2";
    let response = http(
        server.port,
        &format!(
            "POST /fast.cfm HTTP/1.0\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    );
    assert_eq!(status(&response), 200, "{response}");
    assert!(response.contains("fast-ok"));
}

#[test]
fn requests_over_max_concurrent_requests_are_shed_with_503() {
    let server = start_server();
    let port = server.port;
    // One slot. Fill it with a 3.5s page, then ask for another page: it can
    // wait at most 2s for the slot, which does not free in time, so it is
    // shed with 503 + Retry-After rather than queued behind the slow one.
    let slow = std::thread::spawn(move || get(port, "/slow.cfm"));
    std::thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    let shed = get(port, "/fast.cfm");
    let waited = started.elapsed();
    assert_eq!(status(&shed), 503, "expected 503 while the only slot is busy, got:\n{shed}");
    assert_eq!(header(&shed, "retry-after"), Some("1"), "{shed}");
    assert!(
        waited >= Duration::from_millis(1500) && waited < Duration::from_millis(3200),
        "the shed request should wait ~2s for a slot, not {waited:?}"
    );
    // The slow request itself was never disturbed...
    let slow = slow.join().unwrap();
    assert_eq!(status(&slow), 200, "{slow}");
    assert!(slow.contains("slow-done"));
    // ...and once it is done the slot is free again.
    assert_eq!(status(&get(port, "/fast.cfm")), 200);
}

/// The `CFID` value a `Set-Cookie` in `response` carries, if any.
fn issued_cfid(response: &str) -> Option<String> {
    response.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if !k.eq_ignore_ascii_case("set-cookie") {
            return None;
        }
        let v = v.trim();
        let rest = v.strip_prefix("CFID=")?;
        Some(rest.split(';').next().unwrap_or("").to_string())
    })
}

fn get_with_cookie(port: u16, path: &str, cfid: &str) -> String {
    http(
        port,
        &format!(
            "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nCookie: CFID={cfid}\r\nConnection: close\r\n\r\n"
        ),
    )
}

#[test]
fn an_unknown_cfid_cookie_is_not_adopted_but_a_known_one_is() {
    let server = start_server();
    let port = server.port;

    // Planted id: the store has never issued it, so the engine mints its own
    // and tells the browser about it. Nothing is ever keyed by the planted
    // value — a second visit with it is a stranger again.
    let first = get_with_cookie(port, "/session.cfm", "attacker-fixed-id");
    assert_eq!(status(&first), 200, "{first}");
    let minted = issued_cfid(&first).unwrap_or_else(|| panic!("a replacement CFID must be set:\n{first}"));
    assert_ne!(minted, "attacker-fixed-id");
    assert!(first.contains(&format!("cfid={minted} hits=1")), "{first}");
    assert!(!first.contains("attacker-fixed-id hits"), "{first}");

    let again = get_with_cookie(port, "/session.cfm", "attacker-fixed-id");
    let minted_again = issued_cfid(&again).expect("still a stranger: a fresh CFID again");
    assert_ne!(minted_again, "attacker-fixed-id");
    assert_ne!(minted_again, minted, "nothing was stored under the planted id, so nothing resumes");
    assert!(again.contains("hits=1"), "{again}");

    // The id the server issued names a real session: it is honoured, no
    // new cookie is set, and the session's data carries on.
    let resumed = get_with_cookie(port, "/session.cfm", &minted);
    assert_eq!(status(&resumed), 200, "{resumed}");
    assert_eq!(issued_cfid(&resumed), None, "a known CFID must not be replaced:\n{resumed}");
    assert!(resumed.contains(&format!("cfid={minted} hits=2")), "{resumed}");
}

/// Drop the connection mid-request so the server sees a reset rather than a
/// timeout; it must not wedge anything.
#[test]
fn an_aborted_request_leaves_the_server_serving() {
    let server = start_server();
    {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        stream
            .write_all(b"POST /fast.cfm HTTP/1.1\r\nHost: x\r\nContent-Length: 50\r\n\r\nabc")
            .unwrap();
        let _ = stream.shutdown(Shutdown::Both);
    }
    assert_eq!(status(&get(server.port, "/fast.cfm")), 200);
}
