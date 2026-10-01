//! Regression test: concurrent FIRST requests to an application must share one
//! application scope.
//!
//! Creating the application was `contains` then `insert`, so two cold requests
//! could each create it, and the second insert orphaned the scope the first was
//! already writing to. A guard-once boot (Preside's reload lock plus re-check
//! inside it) then ran twice: the second boot cleared the application while
//! requests were using the first, which left ColdBox handlers cached without
//! their framework methods, so a Kubernetes /alive/ probe 500'd forever.
//!
//! Fixture: `tests/fixtures/cold_app_create_app`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cold_app_create_app")
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

fn start_server() -> Server {
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
    // an opaque connect panic in the first http_get.
    let mut server = Server { child, port };
    for _ in 0..600 {
        // A socket whose local and peer addresses match connected to ITSELF (Linux
        // TCP simultaneous open on an ephemeral-range port), not to a server.
        if TcpStream::connect(("127.0.0.1", port))
            .is_ok_and(|s| s.local_addr().ok() != s.peer_addr().ok())
        {
            return server;
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            panic!("rustcfml --serve exited before accepting connections: {status}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("rustcfml --serve not accepting connections on port {port} after 30s");
}

/// Minimal HTTP/1.0 GET returning the full response (headers + body).
fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read response");
    out
}

#[test]
fn concurrent_cold_requests_boot_the_application_once() {
    // The race needs requests to arrive together on a cold server, so try a few.
    for trial in 1..=4 {
        let server = start_server();
        let port = server.port;
        let handles: Vec<_> = (0..12)
            .map(|_| std::thread::spawn(move || http_get(port, "/index.cfm")))
            .collect();
        for h in handles {
            h.join().expect("request thread");
        }
        let r = http_get(port, "/index.cfm");
        assert!(
            r.contains("boots=1"),
            "trial {trial}: the application should boot once; got:\n{r}"
        );
    }
}
