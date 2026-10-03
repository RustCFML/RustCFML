//! GH #456: the engine's admin endpoints (`/__rustcfml/metrics`,
//! `/__rustcfml/profiler`) obey `security.blockedPaths` and the urlrewrite
//! rules, so an app can keep them internal. A rule refuses them by setting an
//! error status (here: 404 when the request came through a proxy); a plain
//! catch-all forward onto a front controller does not hide them. Fixture
//! `tests/fixtures/admin_endpoint_app`, one server per config file.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admin_endpoint_app")
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

fn start_server(cfconfig: &str) -> Server {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .arg("--cfconfig")
        .arg(fixtures_dir().join(cfconfig))
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
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

fn http_get(port: u16, path: &str, extra_headers: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n{extra_headers}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read response");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

#[test]
fn metrics_served_directly_refused_through_a_proxy() {
    let server = start_server("metrics.cfconfig.json");
    // Called on the container directly: served, even though a catch-all rule
    // forwards every path to /index.cfm.
    let (status, body) = http_get(server.port, "/__rustcfml/metrics", "");
    assert_eq!(status, 200, "direct scrape should be served: {body}");
    assert!(!body.contains("front controller"), "metrics, not the front controller");

    // Through a proxy: the header rule sets 404 and the endpoint is refused.
    let (status, body) = http_get(server.port, "/__rustcfml/metrics", "X-Forwarded-For: 203.0.113.9\r\n");
    assert_eq!(status, 404, "proxied scrape should be refused: {body}");
    assert!(!body.contains("http_server"), "no metrics leaked through the proxy");
}

#[test]
fn set_status_applies_to_a_forwarded_page() {
    let server = start_server("metrics.cfconfig.json");
    let (status, body) = http_get(server.port, "/gone", "");
    assert_eq!(status, 410);
    assert!(body.contains("plain page"), "the forwarded page still renders: {body}");
    // The catch-all still forwards ordinary paths with a 200.
    let (status, body) = http_get(server.port, "/some/route", "");
    assert_eq!(status, 200);
    assert!(body.contains("front controller"), "{body}");
}

#[test]
fn blocked_paths_cover_the_admin_endpoints() {
    let server = start_server("blocked.cfconfig.json");
    let (status, _) = http_get(server.port, "/__rustcfml/metrics", "");
    assert_eq!(status, 404, "blockedPaths must refuse /__rustcfml/* too");
}
