//! Regression test for GH #301: under `--serve` + production mode, an
//! `Application.cfc` whose `extends` target is only reachable through its own
//! `this.mappings` must still resolve its parent — on the first request and
//! every request after.
//!
//! v0.545.0 broke this: `load_application_cfc` probes the extends parent
//! BEFORE `this.mappings` is extracted (so the probe fails), and the failed
//! resolution was written into the cross-request production
//! `component_path_cache` under a key that — after the GH #298 source-file →
//! source-directory key collapse — collided with the later, post-mappings
//! resolution. The poisoned entry handed back a nonexistent path, the parent
//! silently failed to load, and every `super.onApplicationStart()` /
//! `super.OnRequestStart()` died with "cannot call method [...] on a null
//! value". Dev mode was unaffected (the production cache layer doesn't run
//! there), which is exactly why this needs a production-mode serve test.
//!
//! Fixed by (a) never caching resolutions whose path doesn't exist and
//! (b) folding a fingerprint of the live mappings table into the cache key so
//! resolutions made under different mapping states can never answer each other.
//!
//! Fixture: `tests/fixtures/production_super_mapping_app` — `www/` is the
//! served webroot; the parent CFC lives in the sibling `moopa/` directory,
//! reachable only via `this.mappings = { "/moopa": "../moopa" }`. The parent's
//! hooks set markers that `index.cfm` prints, so the assertions prove the
//! super chain actually executed rather than just "no error".

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn webroot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/production_super_mapping_app/www")
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

fn start_server(production: bool) -> Server {
    let port = free_port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustcfml"));
    cmd.arg("--serve")
        .arg(webroot())
        .arg("--port")
        .arg(port.to_string());
    if production {
        cmd.env("RUSTCFML_PRODUCTION", "1");
    }
    let child = cmd.spawn().expect("spawn rustcfml --serve");
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

fn assert_super_chain_ran(server: &Server, mode: &str) {
    // The FIRST request is the critical one — GH #301 failed from request 1
    // (the poison was planted and consumed within a single request). Later
    // requests then exercise the warm cross-request cache path.
    for i in 1..=3 {
        let r = http_get(server.port, "/index.cfm");
        assert!(
            r.contains("boot=[parent-boot] req=[parent-request]"),
            "{mode} request {i}: expected the mapped extends parent's \
             onApplicationStart/OnRequestStart to have run via super; got:\n{r}"
        );
    }
}

#[test]
fn production_mode_super_via_this_mappings_extends_resolves() {
    let server = start_server(true);
    assert_super_chain_ran(&server, "production");
}

#[test]
fn dev_mode_super_via_this_mappings_extends_resolves() {
    let server = start_server(false);
    assert_super_chain_ran(&server, "dev");
}
