//! Regression test: a component published into application scope by one request
//! MUST have its methods resolvable by another request that is running
//! CONCURRENTLY — including inherited methods reached without a receiver.
//!
//! Application-scope writes are normally buffered per request, but a write into
//! a nested container that already existed in the persisted snapshot shares that
//! container's inner storage, so it becomes visible to other in-flight requests
//! immediately. Function ids, however, were only published for cross-request use
//! at the *writing* request's end (`rehome_application_functions`). In the window
//! between, a concurrent reader saw the component but could not resolve its
//! method ids: `resolve_fn` returned `None`, dispatch fell back to by-name
//! lookup, and an inherited method — which `heal_stale_component_method` does not
//! cover — failed with "Variable is not a function or function 'X' is not
//! defined".
//!
//! Under ColdBox this surfaced as `'_targetAction' is not defined`, because
//! `EventHandler`'s inherited `_privateInvoker` pulls the action out as a VALUE
//! (`var _targetAction = variables[ method ]`), leaving the local alias as the
//! only name the fallback had. Preside admin ajax endpoints hit it constantly,
//! running concurrently with the page request that lazily builds WireBox
//! singletons. Fixed by `SHARED_FN_REGISTRY` (cfml-vm), a process-wide
//! `Weak`-backed fallback consulted on a per-request registry miss.
//!
//! Fixture: `tests/fixtures/concurrent_fn_registry`. `warmup.cfm` establishes the
//! shared nested container; `publish.cfm` writes a `Child extends Parent` into it
//! and then sleeps, staying in flight; `read.cfm` — issued concurrently — must
//! see it and dispatch both an inherited and an own method.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/concurrent_fn_registry")
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
    // an opaque connect panic in the first request.
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
fn concurrent_request_resolves_methods_of_a_component_published_mid_flight() {
    let server = start_server();
    let port = server.port;

    // Establish the shared nested container and let the request finish, so the
    // container's inner storage is part of the persisted snapshot.
    let warm = http_get(port, "/warmup.cfm");
    assert!(
        warm.contains("registry-established"),
        "warmup should establish the nested container; got:\n{warm}"
    );

    // Publisher stays in flight for ~4s after writing the component.
    let publisher = std::thread::spawn(move || http_get(port, "/publish.cfm"));

    // Give the publisher time to write, then read CONCURRENTLY — before its
    // request-end rehoming has run.
    std::thread::sleep(Duration::from_millis(1000));
    let reader = http_get(port, "/read.cfm");

    let published = publisher.join().expect("publisher thread");
    assert!(
        published.contains("published;"),
        "publisher should have written the component; got:\n{published}"
    );

    // The write must be visible mid-flight — otherwise this test is not
    // exercising the race at all and would pass vacuously.
    assert!(
        reader.contains("visible;"),
        "component should be visible to the concurrent request (test would be \
         vacuous otherwise); got:\n{reader}"
    );
    // The own method always worked (it heals against the receiver).
    assert!(
        reader.contains("own=[child-secret]"),
        "own method should dispatch; got:\n{reader}"
    );
    // The inherited method, invoked as a bare value, is the regression.
    assert!(
        reader.contains("inherited=[child-secret]"),
        "inherited method invoked without a receiver should resolve across the \
         concurrent request boundary; got:\n{reader}"
    );
    assert!(
        !reader.contains("is not defined"),
        "no function-id resolution failure expected; got:\n{reader}"
    );
}
