//! `requestTimeout` is enforced — docs/known-issues.md §3.
//!
//! `server.requestTimeout`, `<cfsetting requestTimeout=N>` and
//! `getPageContext().setRequestTimeout()` all stored a value that nothing ever
//! compared elapsed time against, so no request was ever aborted: a page that
//! raised its own timeout expecting protection had none.
//!
//! Enforcement is deliberately **Lucee-faithful**, which means it fires at
//! blocking points rather than from the bytecode dispatch loop. Verified against
//! Lucee 7.0.4: a `sleep()` that overruns IS interrupted, while a tight CFML
//! `while` loop spinning for 8s under a 1-second timeout runs to completion.
//! So this suite drives the deadline with `sleep()`, the point both engines
//! agree on.
//!
//! A Rust integration test rather than a CFML suite because the trigger is a
//! SERVER-level cfconfig key — `server.*` keys are server-level by design (§5),
//! so an app-level `.cfconfig.json` cannot set it and `tests/runner.cfm` has no
//! way to ask for one.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn webroot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/request_timeout_app")
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

/// The fixture's `.cfconfig.json` sets `server.requestTimeout: 2` and turns
/// debugging on so the error page carries the message we assert against.
fn start_server() -> Server {
    let port = free_port();
    let root = webroot();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--cfconfig")
        .arg(root.join(".cfconfig.json"))
        .spawn()
        .expect("spawn rustcfml --serve");
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, port }
}

fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
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
fn request_timeout_is_enforced_at_blocking_points() {
    let server = start_server();

    // --- a request inside its budget is untouched -------------------------
    let fast = http_get(server.port, "/fast.cfm");
    assert!(
        fast.contains("FAST-OK"),
        "a request well inside requestTimeout must complete; got:\n{fast}"
    );
    assert!(
        !fast.contains("run into a timeout"),
        "a fast request must not be reported as timed out; got:\n{fast}"
    );

    // --- an overrunning request is stopped, and stopped EARLY -------------
    // The 10s sleep must die near the 2s limit, not run to completion; timing
    // is what proves the sleep is actually interrupted rather than merely
    // reported as over once it finished.
    let began = Instant::now();
    let slow = http_get(server.port, "/slow.cfm");
    let took = began.elapsed();
    assert!(
        slow.contains("run into a timeout (timeout: 2 seconds) and has been stopped"),
        "an overrunning request must abort with Lucee's wording; got:\n{slow}"
    );
    assert!(
        took < Duration::from_secs(7),
        "the sleep should be interrupted near the 2s deadline, but the request \
         took {took:?} — it looks like it ran to completion"
    );
    assert!(
        !slow.contains("SLOW-COMPLETED"),
        "the body after the overrunning sleep must not run; got:\n{slow}"
    );
    // The headline property: catch(any) must NOT see it, or a framework's
    // catch-all would swallow the timeout and carry on.
    assert!(
        !slow.contains("SLOW-CAUGHT"),
        "the request timeout must not be catchable by catch(any); got:\n{slow}"
    );

    // --- cfsetting can RAISE the limit above the configured one -----------
    let raised = http_get(server.port, "/raised.cfm");
    assert!(
        raised.contains("RAISED-OK"),
        "<cfsetting requestTimeout> must be able to raise the limit past the \
         configured 2s; got:\n{raised}"
    );

    // --- and LOWER it below the configured one ----------------------------
    let began = Instant::now();
    let lowered = http_get(server.port, "/lowered.cfm");
    let took = began.elapsed();
    assert!(
        lowered.contains("run into a timeout (timeout: 1 seconds) and has been stopped"),
        "<cfsetting requestTimeout=1> must lower the limit and be reported as \
         1 second; got:\n{lowered}"
    );
    assert!(
        !lowered.contains("LOWERED-COMPLETED"),
        "the lowered-timeout request must not complete; got:\n{lowered}"
    );
    assert!(
        took < Duration::from_secs(5),
        "the lowered deadline should fire near 1s; took {took:?}"
    );

    // --- the server is still healthy afterwards --------------------------
    // A timeout must abort ONE request, not wedge the worker.
    let after = http_get(server.port, "/fast.cfm");
    assert!(
        after.contains("FAST-OK"),
        "the server must still serve normally after a timeout; got:\n{after}"
    );
}
