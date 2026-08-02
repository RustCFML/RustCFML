//! Regression test: cfconfig `server.host` MUST decide the listener's bind
//! address.
//!
//! The key was deserialized and stored but never read — the listener hardcoded
//! `TcpListener::bind(("0.0.0.0", port))`. So a deployment that set
//! `"host": "127.0.0.1"` intending to keep the server off the network was
//! silently reachable from every interface, and the schema's own documented
//! default said `127.0.0.1` while the engine bound `0.0.0.0`. Nothing failed;
//! the setting simply did nothing.
//!
//! Proving "it binds the address you asked for" without depending on this
//! machine's network interfaces: point `host` at 192.0.2.1 (TEST-NET-1,
//! RFC 5737 — reserved for documentation and assigned to no local interface).
//! Binding it must FAIL. Before the fix the key was ignored, the server bound
//! 0.0.0.0 and started happily, which is precisely the silent no-op.
//!
//! Note both the config file and the port are per-test: `server.*` is a
//! server-level section (never overlaid from a per-app `.cfconfig.json`), so
//! each case passes its own file via `--cfconfig` rather than sharing one
//! fixture file that the parallel test runner would race on.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_host_bind")
}

struct Server {
    child: Child,
    cfconfig: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.cfconfig);
    }
}

/// Start `--serve` with a server-level cfconfig pinning `server.host`.
fn spawn(host: &str, port: u16, tag: &str) -> Server {
    let cfconfig = std::env::temp_dir().join(format!("rustcfml_host_bind_{tag}.json"));
    std::fs::write(
        &cfconfig,
        format!("{{\n  \"server\": {{\n    \"host\": \"{host}\"\n  }}\n}}\n"),
    )
    .expect("write cfconfig");

    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .arg("--cfconfig")
        .arg(&cfconfig)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rustcfml --serve");
    Server { child, cfconfig }
}

#[test]
fn server_host_loopback_binds_and_serves() {
    let port = free_port();
    let mut server = spawn("127.0.0.1", port, "loopback");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut connected = false;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        connected,
        "server with host=127.0.0.1 never accepted a loopback connection on port {port}"
    );

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream
        .write_all(
            format!("GET /index.cfm HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
    stream.flush().unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).expect("read response");
    let body = String::from_utf8_lossy(&out);
    assert!(
        body.contains("host-bind-ok"),
        "loopback-bound server did not serve the fixture; got:\n{body}"
    );

    let _ = server.child.kill();
}

#[test]
fn unbindable_server_host_fails_instead_of_silently_binding_everything() {
    // RFC 5737 TEST-NET-1: reserved for documentation, present on no interface.
    let port = free_port();
    let mut server = spawn("192.0.2.1", port, "testnet");

    // The process must exit non-zero rather than fall back to 0.0.0.0.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut status = None;
    while Instant::now() < deadline {
        match server.child.try_wait().expect("try_wait") {
            Some(s) => {
                status = Some(s);
                break;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }

    let status = status.expect(
        "server did not exit: an unbindable server.host was ignored and the listener \
         came up anyway (the pre-fix behaviour — it bound 0.0.0.0 regardless)",
    );
    assert!(
        !status.success(),
        "server exited successfully despite an unbindable server.host"
    );

    // Nothing should be listening on that port, on any interface.
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "port {port} is accepting connections even though server.host was unbindable — \
         the bind address was ignored"
    );
}
