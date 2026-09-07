//! `--max-memory`: the soft tier (503 back-pressure) and the hard tier (abort
//! the largest in-flight allocator).
//!
//! Over 85% of the limit the server refuses NEW requests with 503 + Retry-After
//! while in-flight ones finish, sheds (collector sweep + `mi_collect`), and
//! reopens admission once the footprint is back under. Fixture
//! `tests/fixtures/thread_alloc_gc_app`, `?step=hog&mb=N&holdms=M`: holds ~N MB
//! of plain strings in a local for M ms — pure footprint, nothing for the cycle
//! collector — then drops it at request end.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/thread_alloc_gc_app")
}

struct Server {
    child: Child,
    port: u16,
    /// The server's stderr. The client only ever sees a plain 500 for an aborted
    /// request — error detail is deliberately not leaked to it — so the watchdog's
    /// decision (which request, and why) is asserted here.
    stderr: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn log(&self) -> String {
        self.stderr.lock().unwrap().join("\n")
    }
    /// Wait up to `secs` for a line matching `needle`.
    fn wait_for_log(&self, needle: &str, secs: u64) -> bool {
        for _ in 0..(secs * 10) {
            if self.log().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_server(max_memory: &str) -> Server {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .arg("--max-memory")
        .arg(max_memory)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
    let mut child = child;
    // Drained on a helper thread so the child can never block on a full pipe.
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let pipe = child.stderr.take().expect("stderr piped");
    let sink = Arc::clone(&stderr);
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            eprintln!("[server] {line}");
            sink.lock().unwrap().push(line);
        }
    });
    let mut server = Server { child, port, stderr };
    for _ in 0..600 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            panic!("rustcfml --serve exited before accepting connections: {status}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("rustcfml --serve not accepting connections on port {port} after 30s");
}

/// GET returning (status, headers+body).
fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read response");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, out)
}

#[test]
fn over_the_soft_limit_new_requests_get_503_and_admission_reopens_after_shedding() {
    // A debug-build server idles around 150-250M of physical footprint; a 600M
    // hog pushes it far past 85% of a 500M limit while the request is in flight.
    let server = start_server("500M");

    let (st, body) = http_get(server.port, "/index.cfm");
    assert_eq!(st, 200, "baseline request should be admitted:\n{body}");

    // Hold ~400MB for 4s on a helper thread; meanwhile a new request must be
    // refused with 503 + Retry-After.
    let port = server.port;
    let hog = std::thread::spawn(move || http_get(port, "/index.cfm?step=hog&mb=600&holdms=4000"));
    std::thread::sleep(Duration::from_millis(1500));
    let (st, resp) = http_get(server.port, "/index.cfm");
    assert_eq!(
        st, 503,
        "a request arriving while the process is over the soft limit must be refused:\n{resp}"
    );
    assert!(
        resp.to_ascii_lowercase().contains("retry-after:"),
        "the 503 must carry Retry-After:\n{resp}"
    );
    assert!(resp.contains("--max-memory"), "the body should name the knob:\n{resp}");

    // The in-flight hog is NOT aborted by the soft tier.
    let (st, body) = hog.join().unwrap();
    assert_eq!(st, 200, "in-flight request must finish normally:\n{body}");
    assert!(body.contains("hogged 600MB"), "{body}");

    // Its data is gone at request end and the end-of-request hook shed; the
    // footprint must come back under and admission reopen.
    let mut reopened = false;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        let (st, _) = http_get(server.port, "/index.cfm");
        if st == 200 {
            reopened = true;
            break;
        }
    }
    assert!(reopened, "admission did not reopen within 10s of the hog finishing — shedding is not returning memory");
}

/// THE HARD TIER. A single request that allocates without bound is the case the
/// soft tier cannot touch: nothing new arrives to be refused, and the request
/// already inside runs until the OOM killer takes the process — and every other
/// request — down with it. The watchdog must stop that request, and only that
/// request.
///
/// `?step=runaway` allocates tracked containers through a user-function call and
/// keeps them, so the only thing that can end it is the abort. Verified
/// non-vacuous: with `HARD_FRACTION` raised so the tier never fires, this test
/// fails — the request reached 6.9 GB and was still running at the client read
/// timeout 125s later. (The loop is bounded at 10M iterations purely so a broken
/// build fails here rather than hanging forever.)
#[test]
fn the_hard_tier_aborts_the_runaway_request_and_the_server_survives() {
    let server = start_server("700M");

    let (st, body) = http_get(server.port, "/index.cfm");
    assert_eq!(st, 200, "baseline request should be admitted:\n{body}");

    let (st, body) = http_get(server.port, "/index.cfm?step=runaway");
    assert_ne!(
        st, 200,
        "a request allocating without bound must be aborted, not completed:\n{body}"
    );
    assert!(
        !body.contains("runaway completed unaborted"),
        "the loop ran to its bound, so the hard tier never fired:\n{body}"
    );
    // The client gets a plain 500 — error detail is not leaked to it, and 500
    // (not 503) is deliberate: a runaway request must not be retried elsewhere.
    assert_eq!(st, 500, "an aborted request should surface as a 500:\n{body}");
    // The operator gets the reason, and it must name the request it chose and the
    // evidence it chose on.
    assert!(
        server.wait_for_log("aborting request", 5),
        "the watchdog must log which request it aborted:\n{}",
        server.log()
    );
    let log = server.log();
    assert!(
        log.contains("--max-memory") || log.contains("max-memory"),
        "the log must name the knob:\n{log}"
    );
    assert!(
        log.contains("largest allocator") && log.contains("Other requests continue"),
        "the log must say why this request was chosen and that others survive:\n{log}"
    );

    // The point of aborting one request rather than letting the process die: the
    // server is still here and serving. (The abort dropped the runaway's data, so
    // admission reopens once the footprint is back under the soft line.)
    let mut served = false;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        if http_get(server.port, "/index.cfm").0 == 200 {
            served = true;
            break;
        }
    }
    assert!(
        served,
        "the server did not serve a normal request within 10s of the abort"
    );
}

/// The safety property that makes the hard tier usable in production: a request
/// that did NOT build the heap is never chosen. The `hog` step holds ~600MB of
/// STRINGS in a couple of containers — it takes the process over the hard line
/// while allocating far too few tracked containers to be responsible for it — so
/// the watchdog must report that no request is responsible and shed instead,
/// leaving the request to finish normally.
#[test]
fn a_request_that_did_not_build_the_heap_is_not_aborted() {
    let server = start_server("500M");
    let (st, body) = http_get(server.port, "/index.cfm?step=hog&mb=600&holdms=3000");
    assert_eq!(
        st, 200,
        "a request holding memory it did not allocate as containers must finish:\n{body}"
    );
    assert!(body.contains("hogged 600MB"), "{body}");
    assert!(
        server.log().contains("has allocated enough to be responsible"),
        "the watchdog must report that no request is responsible, rather than \
         picking one:\n{}",
        server.log()
    );
}
