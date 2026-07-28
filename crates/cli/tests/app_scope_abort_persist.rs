//! Regression test: application-scope state written during a request that
//! unwinds out of `onRequestStart` via `cfabort` MUST survive into later
//! requests, with its component methods (including super-chain dispatch)
//! resolvable.
//!
//! Historically the abort/error exit paths of `execute_with_lifecycle`
//! returned before the request-end persistence step, which
//!   (a) dropped every top-level `application.*` write made during the
//!       request (a freshly-booted Preside app vanished when its first
//!       request ended 401/500 and re-booted on every request), and
//!   (b) left components cached into already-shared NESTED containers
//!       (e.g. ColdBox's handler cache) visible to later requests but with
//!       un-rehomed function ids: their own methods healed, but
//!       `super.method()` failed with "function 'X' is not defined"
//!       (Preside admin 500'd permanently after one 401 access-denied hit).
//!
//! Fixture: `tests/fixtures/abort_persist_app` — `?cacheAndAbort=1` caches a
//! `Child extends Parent` instance at a top-level key AND inside a nested
//! struct during `onRequestStart`, then aborts. Subsequent plain requests
//! call `greet()` (which delegates to `super.greet()`) on both.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/abort_persist_app")
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
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, port }
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
fn app_scope_writes_survive_abort_and_super_dispatch_stays_resolvable() {
    let server = start_server();

    // Request 1: cache components (top-level + nested) in onRequestStart, abort.
    let r1 = http_get(server.port, "/call.cfm?cacheAndAbort=1");
    assert!(
        r1.contains("primed:child->parent-greet"),
        "abort request should run and cache the components; got:\n{r1}"
    );

    // Requests 2+3: both instances must still exist and dispatch through the
    // full inheritance chain. Before the fix, request 2 failed with
    // "Variable 'topLevel' is undefined" (top-level write dropped) or
    // "function 'greet' is not defined" (stale super chain on the nested one).
    for i in 2..=3 {
        let r = http_get(server.port, "/call.cfm");
        assert!(
            r.contains("nested=[child->parent-greet] top=[child->parent-greet]"),
            "request {i}: cached components should survive the aborted request; got:\n{r}"
        );
        assert!(
            !r.contains("CAUGHT"),
            "request {i}: no error expected; got:\n{r}"
        );
    }
}
