//! Server-level security settings, exercised against a real `--serve`.
//!
//! These are `.cfconfig.json` keys an application cannot set for itself, so
//! `tests/runner.cfm` cannot ask for them; each test writes its own webroot and
//! baseline config into a temp directory and starts a server on `--port 0`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server {
    child: Child,
    port: u16,
    root: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml-secpol-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Start a server over `files` (relative path → content) with `cfconfig` as
/// its baseline configuration. `extra` are additional CLI flags.
fn start(tag: &str, cfconfig: &str, files: &[(&str, &str)], extra: &[&str]) -> Server {
    let root = temp_root(tag);
    let web = root.join("web");
    std::fs::create_dir_all(&web).unwrap();
    for (rel, body) in files {
        let p = web.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    let cfg = root.join("baseline.cfconfig.json");
    std::fs::write(&cfg, cfconfig).unwrap();
    spawn(root, &web, &cfg, extra)
}

fn spawn(root: PathBuf, web: &Path, cfg: &Path, extra: &[&str]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(web)
        .args(["--port", "0", "--cfconfig"])
        .arg(cfg)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rustcfml --serve");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|line| {
            let addr = line.split("running on http://").nth(1)?;
            addr.split_whitespace().next()?.rsplit(':').next()?.parse::<u16>().ok()
        })
        .expect("server exited before printing its ready banner");
    std::thread::spawn(move || lines.for_each(drop));
    Server { child, port, root }
}

fn request(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn get(port: u16, path: &str) -> String {
    get_with(port, path, "")
}

/// GET with extra raw header lines (each ending in `\r\n`).
fn get_with(port: u16, path: &str, headers: &str) -> String {
    request(
        port,
        &format!(
            "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n{headers}Connection: close\r\n\r\n"
        ),
    )
}

const PROBE_HELPER: &str = r#"<cfscript>
function probe(label, fn) {
  try { r = fn(); writeOutput(label & "=ALLOWED" & chr(10)); }
  catch (any e) { writeOutput(label & "=refused" & chr(10)); }
}
</cfscript>"#;

/// `disallowedFunctions` used to be checked only at the top of the generic
/// call path. Compile-time-bound builtins, member calls, `evaluate()` and the
/// `<cfexecute>` tag (lowered to `__cfexecute`) all walked past it, and the
/// documented spelling `"cfexecute"` never matched the lowered name.
#[test]
fn disallowed_functions_cover_every_call_form() {
    let page = format!(
        r#"{PROBE_HELPER}<cfscript>
function myProbe() {{ return 1; }}
probe("udf", function(){{ return myProbe(); }});
probe("direct", function(){{ return ucase("a"); }});
probe("member", function(){{ return "a".ucase(); }});
probe("evaluate", function(){{ return evaluate("ucase('a')"); }});
probe("arrayLen", function(){{ return arrayLen([1]); }});
probe("arrayLenMember", function(){{ return [1].len(); }});
probe("scriptExecute", function(){{ cfexecute(name="/bin/echo", arguments="x", variable="v", timeout=5); return 1; }});
probe("notBanned", function(){{ return trim(" a "); }});
</cfscript><cftry><cfexecute name="/bin/echo" arguments="x" variable="v2" timeout="5"><cfoutput>tag=ALLOWED</cfoutput><cfcatch><cfoutput>tag=refused</cfoutput></cfcatch></cftry>"#
    );
    let server = start(
        "disallowed",
        r#"{"security":{"disallowedFunctions":["ucase","arrayLen","cfexecute","myProbe"]}}"#,
        &[("t.cfm", &page)],
        &[],
    );
    let body = get(server.port, "/t.cfm");
    for label in [
        "udf",
        "direct",
        "member",
        "evaluate",
        "arrayLen",
        "arrayLenMember",
        "scriptExecute",
        "tag",
    ] {
        assert!(body.contains(&format!("{label}=refused")), "{label} not refused:\n{body}");
    }
    assert!(body.contains("notBanned=ALLOWED"), "unbanned builtin broke:\n{body}");
}

/// `disallowedImports` was applied to `component` and `rust` only, so a
/// pattern naming a Java class did nothing.
#[test]
fn disallowed_imports_cover_java_classes() {
    let page = format!(
        r#"{PROBE_HELPER}<cfscript>
probe("pb", function(){{ return createObject("java", "java.lang.ProcessBuilder"); }});
probe("other", function(){{ return createObject("java", "java.lang.StringBuilder"); }});
</cfscript>"#
    );
    let server = start(
        "imports",
        r#"{"security":{"disallowedImports":["java\\.lang\\.ProcessBuilder"]}}"#,
        &[("t.cfm", &page)],
        &[],
    );
    let body = get(server.port, "/t.cfm");
    assert!(body.contains("pb=refused"), "ProcessBuilder not refused:\n{body}");
    assert!(body.contains("other=ALLOWED"), "unrelated class broke:\n{body}");
}

/// `?debug=true` used to open the footer for ANY client — gate 2 is "IP
/// allowed OR URL trigger", and the trigger shipped enabled with a public
/// value. And the 500 page printed the full message and stack trace (absolute
/// paths) to every client whenever `debugging.enabled`.
#[test]
fn debug_output_stays_with_whitelisted_viewers() {
    let server = start(
        "debug",
        r#"{"debugging":{"enabled":true,"showFromIPs":["10.0.0.5"]}}"#,
        &[
            ("index.cfm", "<html><body><cfoutput>hi #now()#</cfoutput></body></html>"),
            ("err.cfm", "<cfscript>x = 1;\nthrow(message=\"boom-secret-detail\", detail=\"inner-detail\");</cfscript>"),
        ],
        &["--production"],
    );
    let page = get(server.port, "/index.cfm?debug=true");
    assert!(page.contains("hi "), "page did not render:\n{page}");
    assert!(
        !page.contains("rustcfml-debug"),
        "?debug=true opened the footer for a non-whitelisted client"
    );
    let err = get(server.port, "/err.cfm");
    assert!(err.contains("Internal Server Error"), "no error page:\n{err}");
    assert!(
        !err.contains("boom-secret-detail") && !err.contains("err.cfm"),
        "error detail leaked to a non-whitelisted client:\n{err}"
    );
}

/// The whitelisted viewer still gets both.
#[test]
fn debug_output_reaches_whitelisted_viewer() {
    let server = start(
        "debug-ok",
        r#"{"debugging":{"enabled":true,"showFromIPs":["127.0.0.1","::1"]}}"#,
        &[
            ("index.cfm", "<html><body><cfoutput>hi #now()#</cfoutput></body></html>"),
            ("err.cfm", "<cfscript>throw(message=\"boom-secret-detail\");</cfscript>"),
        ],
        &[],
    );
    let page = get(server.port, "/index.cfm");
    assert!(page.contains("rustcfml-debug"), "whitelisted viewer lost the footer");
    let err = get(server.port, "/err.cfm");
    assert!(err.contains("boom-secret-detail"), "whitelisted viewer lost error detail:\n{err}");
}
