//! `--max-memory`: the soft tier.
//!
//! Over 85% of the limit the server refuses NEW requests with 503 + Retry-After
//! while in-flight ones finish, sheds (collector sweep + `mi_collect`), and
//! reopens admission once the footprint is back under. Fixture
//! `tests/fixtures/thread_alloc_gc_app`, `?step=hog&mb=N&holdms=M`: holds ~N MB
//! of plain strings in a local for M ms — pure footprint, nothing for the cycle
//! collector — then drops it at request end.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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
        .stderr(Stdio::inherit())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
    let mut server = Server { child, port };
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
