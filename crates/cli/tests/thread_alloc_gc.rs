//! Allocations made on a `cfthread` are collectible garbage like any other.
//!
//! The cycle collector's allocation log is THREAD-LOCAL and armed per thread by
//! `cycle_gc::enable()`. Until v0.653.3 only the request path ever called it, so
//! nothing a spawned thread allocated was ever logged. That had two effects, and
//! the second is the one that mattered:
//!
//!  1. A cycle built on a thread could never be freed (it was in no survivor
//!     set), and
//!  2. every reference such an allocation held read to the collector as EXTERNAL
//!     ownership of its target — an invisible holder — so the target and its
//!     whole transitive closure were pinned live.
//!
//! Preside's `?fwreinit=true` spawns ~37 threads (task manager, heartbeats, log
//! listeners, module loaders) whose products land in the application graph. The
//! singletons they referenced came out with a small unexplained refcount surplus
//! (`strong = 1 + internal + 1..3`), and that dragged a complete ~111,000-node
//! generation along per reload: 1.0G → 1.9G over eight reloads, linear, never
//! reclaimed. With thread allocations logged and carried into the cross-request
//! set, the sweep reclaims a whole generation at a time and the footprint holds.
//!
//! Fixture: `tests/fixtures/thread_alloc_gc_app`. `?step=holder` makes a PLAIN
//! struct in application scope (displacement from a plain struct is invisible to
//! the relog mutation hook, so only the carried survivor set can find it);
//! `?step=build` has a cfthread allocate a cycle plus `n` nodes and store them
//! under it; `?step=drop` deletes them. The server runs with
//! `RUSTCFML_GC_PERSISTENT_ALWAYS=1` so the sweep fires at every request end,
//! and the assertion reads the collector's own `cross-request sweep reclaimed N`
//! line from stderr. Verified non-vacuous: with the `enable()` in
//! `spawn_cfthread` removed, the sweeps after the drop reclaim nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
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
    stderr: Arc<Mutex<Vec<String>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_server(extra_env: &[(&str, &str)]) -> Server {
    let port = free_port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustcfml"));
    cmd.arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .env("RUSTCFML_GC_DEBUG", "1");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
    // Drain stderr on a helper thread so the child can never block on a full
    // pipe, and keep every line for the assertion.
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let pipe = child.stderr.take().expect("stderr piped");
    let sink = Arc::clone(&stderr);
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(line);
        }
    });
    let mut server = Server { child, port, stderr };
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

/// Sum of `N` over every stderr line of the form `[cycle_gc] <kind> … reclaimed N …`
/// seen so far (`kind` is e.g. `cross-request sweep` or `displacement sweep #1`).
fn reclaimed_total_of(server: &Server, kind: &str) -> usize {
    server
        .stderr
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains(kind))
        .filter_map(|l| {
            let rest = l.rsplit("reclaimed ").next()?;
            rest.split_whitespace().next()?.parse::<usize>().ok()
        })
        .sum()
}

fn reclaimed_total(server: &Server) -> usize {
    reclaimed_total_of(server, "cross-request sweep")
}

#[test]
fn a_graph_allocated_on_a_cfthread_is_reclaimed_once_dropped() {
    const N: usize = 600;
    let server = start_server(&[("RUSTCFML_GC_PERSISTENT_ALWAYS", "1")]);

    let r = http_get(server.port, "/index.cfm?step=holder");
    assert!(r.contains("holder"), "holder step failed:\n{r}");

    let r = http_get(server.port, &format!("/index.cfm?step=build&n={N}"));
    assert!(
        r.contains("built status=COMPLETED") && r.contains("keys="),
        "build step failed:\n{r}"
    );
    // Everything so far is live (held from application scope), so nothing of
    // the graph may have been reclaimed yet — this pins down that the later
    // reclaim really is the drop, not churn from the build request itself.
    let before = reclaimed_total(&server);

    let r = http_get(server.port, "/index.cfm?step=drop");
    assert!(r.contains("dropped left=0"), "drop step failed:\n{r}");

    // The drop request's own end-of-request sweep is where the graph goes. Give
    // the stderr drain a moment, and allow one more request in case the sweep
    // landed while the builder thread's handle was still winding down.
    let mut gained = 0;
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(300));
        gained = reclaimed_total(&server).saturating_sub(before);
        if gained >= N {
            break;
        }
        http_get(server.port, "/index.cfm");
    }
    assert!(
        gained >= N,
        "a graph allocated on a cfthread and then dropped was not reclaimed \
         (reclaimed {gained} nodes after the drop, expected at least {N}): the \
         thread's allocations were never logged, so the cycle at its root is \
         invisible to the collector and everything it holds is pinned"
    );
}

/// The DISPLACEMENT SWEEP: dropping a generation-sized graph from a persistent
/// scope must trigger a sweep at that request's end, without
/// `RUSTCFML_GC_PERSISTENT_ALWAYS` and without waiting for the doubling budget.
///
/// Before this trigger existed, the cross-request sweep only ran once the tracked
/// set had DOUBLED since the last sweep, so on Preside a reload's dead generation
/// stayed resident for two or three more reloads (footprint 1.3G instead of
/// ~900M). The relog hook already knows the exact moment a generation is
/// displaced; this test pins down that the collector acts on it.
///
/// Two knobs are pinned so the test exercises the SWEEP and not the ordinary
/// request-end collect: the relog hook re-enters displaced nodes into the current
/// request's log, where `collect()` would free them at request end anyway, so the
/// relog budget is capped at 100 — everything past that is only in the carried
/// survivor set and can only be freed by the sweep — and the displacement
/// threshold is lowered to match. Verified non-vacuous: with
/// `RUSTCFML_GC_DISPLACE_SWEEP_MIN=0` (trigger disabled) no displacement sweep
/// runs and the graph stays tracked.
#[test]
fn displacing_a_generation_from_a_persistent_scope_sweeps_at_request_end() {
    const N: usize = 3_000;
    const RELOG_BUDGET: usize = 100;
    let server = start_server(&[
        ("RUSTCFML_RELOG_BUDGET", "100"),
        ("RUSTCFML_GC_DISPLACE_SWEEP_MIN", "50"),
    ]);

    let r = http_get(server.port, "/index.cfm?step=holder");
    assert!(r.contains("holder"), "holder step failed:\n{r}");
    let r = http_get(server.port, &format!("/index.cfm?step=build&n={N}"));
    assert!(r.contains("built status=COMPLETED"), "build step failed:\n{r}");
    let before = reclaimed_total_of(&server, "displacement sweep");

    let r = http_get(server.port, "/index.cfm?step=dropscope");
    assert!(r.contains("dropped-scope has=false"), "dropscope step failed:\n{r}");

    // The sweep belongs to the drop request itself (or the builder thread's
    // exit, moments later); allow the stderr drain to catch up.
    let want = N - RELOG_BUDGET;
    let mut gained = 0;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(300));
        gained = reclaimed_total_of(&server, "displacement sweep").saturating_sub(before);
        if gained >= want {
            break;
        }
    }
    assert!(
        gained >= want,
        "displacing a {N}-node graph from application scope did not trigger a \
         displacement sweep that reclaimed it (displacement sweeps reclaimed \
         {gained} nodes, expected at least {want}) — the relog hook is not arming \
         the sweep, or the sweep is still waiting on the doubling budget"
    );
}
