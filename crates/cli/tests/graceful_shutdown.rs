//! `--serve` shuts down gracefully on SIGTERM as well as SIGINT.
//!
//! Why SIGTERM specifically. It is the stop signal `docker stop`, Kubernetes and
//! systemd all send, and until this was handled the engine did the wrong thing in
//! both of the ways an unhandled SIGTERM can go wrong: outside a container the
//! default action killed the process on the spot, cutting off in-flight requests
//! and skipping every `Drop`; as PID 1 in a container the kernel installs no
//! default disposition at all, so the signal was IGNORED and every stop became
//! "wait out the grace period, then SIGKILL". The reference Docker image carried
//! `STOPSIGNAL SIGINT` purely to work around it.
//!
//! Not asserted here (it needs a container): the PID 1 case. The fix is the same
//! code path — a handler exists, so PID 1 delivery works.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn docroot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graceful_shutdown_app")
}

fn start(port: u16) -> Child {
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(docroot())
        .arg("--port")
        .arg(port.to_string())
        .stderr(Stdio::inherit())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
    for _ in 0..600 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start on {port}");
}

fn get(port: u16, path: &str, timeout: Duration) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(timeout))?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;
    let mut out = String::new();
    stream.read_to_string(&mut out)?;
    Ok(out)
}

fn signal(child: &Child, sig: &str) {
    let ok = Command::new("kill")
        .arg(sig)
        .arg(child.id().to_string())
        .status()
        .expect("kill")
        .success();
    assert!(ok, "failed to send {sig}");
}

/// An idle server must exit promptly on SIGTERM — not sit there until something
/// SIGKILLs it.
///
/// Honest about its own power: this one passes with or without the handler when
/// run OUTSIDE a container, because the default action for an unhandled SIGTERM
/// kills the process anyway. It pins the contract, not the regression. The test
/// below is the one that fails without the fix (verified).
#[test]
fn sigterm_stops_an_idle_server_promptly() {
    let port = free_port();
    let mut child = start(port);
    let t0 = Instant::now();
    signal(&child, "-TERM");
    let mut exited = false;
    for _ in 0..100 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !exited {
        let _ = child.kill();
        panic!(
            "server was still running 10s after SIGTERM — an unhandled SIGTERM is \
             ignored as PID 1 in a container, which turns every `docker stop` into \
             a grace-period wait plus SIGKILL"
        );
    }
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "SIGTERM shutdown took {:?}",
        t0.elapsed()
    );
}

/// Graceful means graceful: a request already in flight when the signal arrives
/// must still get its response. This is what makes a rolling deploy lossless.
///
/// Verified non-vacuous: with the handler listening on a different signal, an
/// unhandled SIGTERM takes the default action and this test fails — the client
/// gets a truncated read instead of its response.
#[test]
fn a_request_in_flight_survives_sigterm() {
    let port = free_port();
    let mut child = start(port);

    let handle = std::thread::spawn(move || get(port, "/slow.cfm", Duration::from_secs(30)));
    // Let the request get inside the engine before signalling.
    std::thread::sleep(Duration::from_millis(1000));
    signal(&child, "-TERM");

    let body = handle
        .join()
        .expect("request thread")
        .expect("in-flight request must still receive its response across a SIGTERM");
    assert!(
        body.contains("slow request completed"),
        "the in-flight request was cut off by the shutdown:\n{body}"
    );

    for _ in 0..100 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    panic!("server did not exit after draining the in-flight request");
}
