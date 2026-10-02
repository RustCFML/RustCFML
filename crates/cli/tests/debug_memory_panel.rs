//! Per-request memory: the debug footer's memory panel and the memory metrics.
//!
//! Allocation accounting is on only when debugging is enabled. With it, the
//! footer shows what the request allocated, its objects by type (component
//! instances by class), the process breakdown and a per-file allocation
//! column, and the Prometheus endpoint carries per-request memory histograms
//! and the live heap. Without it, none of that appears. Fixture
//! `tests/fixtures/debug_memory_app`, one server per config file.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/debug_memory_app")
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

fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
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

/// The value of an unlabelled metric line, or of the first line starting with
/// `prefix` when it carries labels.
fn metric(text: &str, prefix: &str) -> Option<f64> {
    text.lines()
        .find(|l| l.starts_with(prefix) && !l.starts_with('#'))
        .and_then(|l| l.rsplit(' ').next())
        .and_then(|v| v.parse().ok())
}

#[test]
fn with_debugging_the_footer_and_the_metrics_report_request_memory() {
    let server = start_server("debug.cfconfig.json");
    let (st, page) = http_get(server.port, "/index.cfm");
    assert_eq!(st, 200, "{page}");
    assert!(page.contains("made 25"), "{page}");

    // The panel: this request, by type, by class, and the process.
    for needle in [
        "Allocated by this request",
        "Peak in use",
        "Still in use as the page ends",
        "Objects created (",
        "Component instances",
        "&#8627; Widget",
        "Request Memory",
        ">Runtime</h4>",
        "Live heap",
        "Application scopes",
        "data-rcfml-sort=\"5\" title=\"sort by alloc KB\"",
    ] {
        assert!(page.contains(needle), "footer is missing {needle:?}");
    }
    // Runtime and its Flags and Memory blocks all start collapsed.
    for block in [
        "<div class=\"rcfml-runtime\" style=\"display:none",
        "class=\"rcfml-flags\" border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"display:none",
        "<div class=\"rcfml-memproc\" style=\"display:none\"",
    ] {
        assert!(page.contains(block), "not collapsed by default: {block:?}");
    }
    // Runtime holds, in order, CFConfig, Environment, Flags and Memory.
    let at = |needle: &str| page.find(needle).unwrap_or_else(|| panic!("missing {needle}"));
    let order = [
        at("<div class=\"rcfml-runtime\""),
        at("data-rcfml-tog=\"rcfml-cfg\""),
        at("data-rcfml-tog=\"rcfml-env\""),
        at("data-rcfml-tog=\"rcfml-flags\""),
        at("data-rcfml-tog=\"rcfml-memproc\""),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    // 25 widgets were created and are still held by the page.
    let widget_row = page
        .split("<tr")
        .find(|r| r.contains("&#8627; Widget"))
        .expect("Widget row");
    assert!(
        widget_row.contains(">25<"),
        "the Widget row should count 25 instances: {widget_row}"
    );

    let (_, m) = http_get(server.port, "/__rustcfml/metrics");
    let count = metric(&m, "rustcfml_request_memory_allocated_bytes_count{route=\"/index.cfm\"}")
        .expect("allocated-bytes histogram for the route");
    assert!(count >= 1.0, "{m}");
    let allocated = metric(&m, "rustcfml_request_memory_allocated_bytes_sum{route=\"/index.cfm\"}").unwrap();
    assert!(
        allocated > 100_000.0,
        "a page that builds 2,000 structs allocated more than 100 KB, got {allocated}"
    );
    let retained = metric(&m, "rustcfml_request_memory_retained_bytes_sum{route=\"/index.cfm\"}").unwrap();
    assert!(
        retained > 0.0 && retained < allocated,
        "the request keeps application.held, but not everything it allocated: {retained} of {allocated}"
    );
    for name in [
        "rustcfml_memory_footprint_bytes",
        "rustcfml_memory_heap_live_bytes",
        "rustcfml_gc_sweeps_total",
        "rustcfml_gc_reclaimed_bytes_total",
    ] {
        assert!(metric(&m, name).is_some(), "metrics are missing {name}:\n{m}");
    }
    // Not unless asked for.
    assert!(!m.contains("java_lang_Memory_HeapMemoryUsage_used"));
}

#[test]
fn without_debugging_there_is_no_request_memory() {
    let server = start_server("nodebug.cfconfig.json");
    let (st, page) = http_get(server.port, "/index.cfm");
    assert_eq!(st, 200, "{page}");
    assert!(!page.contains("Allocated by this request"), "no footer without debugging");

    let (_, m) = http_get(server.port, "/__rustcfml/metrics");
    assert!(
        metric(&m, "rustcfml_request_memory_allocated_bytes_count").is_none(),
        "no request is metered without debugging:\n{m}"
    );
    assert!(metric(&m, "rustcfml_memory_heap_live_bytes").is_none(), "{m}");
    // The process and collector figures don't need the accounting.
    assert!(metric(&m, "rustcfml_memory_footprint_bytes").is_some(), "{m}");
    assert!(metric(&m, "rustcfml_gc_sweeps_total").is_some(), "{m}");
}
