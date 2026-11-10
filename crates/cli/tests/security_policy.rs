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

/// The `CFID` cookie is client input: a value that is not an id this engine
/// could have minted (here, one carrying a space — a memcached text-protocol
/// injection) must be ignored and a fresh id issued.
#[test]
fn a_malformed_cfid_cookie_is_not_adopted() {
    let server = start(
        "cfid",
        "{}",
        &[
            ("Application.cfc", "component { this.name = \"cfidprobe\"; this.sessionManagement = true; }"),
            ("s.cfm", "<cfset session.touched = true><cfoutput>sid=#session.sessionid#</cfoutput>"),
        ],
        &[],
    );
    let planted = "evil value";
    let resp = get_with(server.port, "/s.cfm", &format!("Cookie: CFID={planted}\r\n"));
    assert!(!resp.contains(planted), "planted CFID was adopted:\n{resp}");
    let set_cookie = resp
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie: cfid="))
        .unwrap_or_else(|| panic!("no fresh CFID issued:\n{resp}"));
    let fresh = set_cookie["set-cookie: cfid=".len()..].split(';').next().unwrap();
    assert_eq!(fresh.len(), 36, "fresh id is not a UUID: {fresh}");
}

/// A client's own `X-Original-URL` must never reach the application — it is
/// the header Preside's router trusts first for the requested path.
#[test]
fn client_x_original_url_is_dropped() {
    let server = start(
        "xorig",
        "{}",
        &[(
            "hdr.cfm",
            "<cfoutput>cgi=[#cgi.http_x_original_url#] hdr=[#getHttpRequestData().headers[\"X-Original-URL\"] ?: \"\"#] rw=[#cgi.http_x_rewrite_url#]</cfoutput>",
        )],
        &[],
    );
    let resp = get_with(
        server.port,
        "/hdr.cfm",
        "X-Original-URL: /admin/\r\nX-Rewrite-URL: /admin/\r\n",
    );
    assert!(resp.contains("cgi=[]"), "cgi saw the client header:\n{resp}");
    assert!(resp.contains("hdr=[]"), "getHttpRequestData saw it:\n{resp}");
    assert!(resp.contains("rw=[]"), "X-Rewrite-URL passed through:\n{resp}");
}

/// Cross-site WebSocket hijacking: a foreign page's upgrade (it carries the
/// visitor's cookie) is refused before any channel is resolved; the server's
/// own origin, localhost and a client with no `Origin` are not.
#[test]
fn websocket_upgrade_from_a_foreign_origin_is_refused() {
    let server = start(
        "wsorigin",
        r#"{"server":{"websocketAllowedOrigins":["https://*.partner.example"]}}"#,
        &[("index.cfm", "ok")],
        &[],
    );
    // Read only the status line: the connection is kept alive, so reading to
    // the end would wait for the server's idle timeout.
    let status_line = |raw: &str| {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        line
    };
    let upgrade = |origin: Option<&str>| {
        let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
        status_line(
            &format!(
                "GET /ws/nosuchchannel HTTP/1.1\r\nHost: app.example\r\n{origin}Connection: Upgrade\r\n\
                 Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
            ),
        )
    };
    let status = |r: String| r.split_whitespace().nth(1).unwrap_or("").to_string();
    assert_eq!(status(upgrade(Some("https://evil.example"))), "403");
    // Allowed origins get past the guard and reach channel resolution (404 here).
    assert_eq!(status(upgrade(Some("https://app.example"))), "404");
    assert_eq!(status(upgrade(Some("http://localhost:3000"))), "404");
    assert_eq!(status(upgrade(Some("https://chat.partner.example"))), "404");
    assert_eq!(status(upgrade(None)), "404");
}

/// `server.cfmlExtensions` adds to the CFML set; it cannot remove `.cfc`. A
/// list without `cfc` used to serve component source as a static file.
#[test]
fn cfc_source_is_never_served_as_static() {
    let server = start(
        "cfcsrc",
        r#"{"server":{"cfmlExtensions":["cfm"]}}"#,
        &[("Secret.cfc", "component { /* SOURCE-MARKER */ remote string function hi() { return \"x\"; } }")],
        &[],
    );
    let resp = get(server.port, "/Secret.cfc");
    assert!(!resp.contains("SOURCE-MARKER"), "component source served:\n{resp}");
}

/// Static files carry `X-Content-Type-Options: nosniff`.
#[test]
fn static_files_are_not_content_sniffed() {
    let server = start("nosniff", "{}", &[("u.txt", "<script>alert(1)</script>")], &[]);
    let resp = get(server.port, "/u.txt");
    assert!(
        resp.lines().any(|l| l.eq_ignore_ascii_case("x-content-type-options: nosniff")),
        "no nosniff header:\n{resp}"
    );
}

/// The engine's admin endpoints answer only `server.adminAllowedIPs`
/// (loopback by default) and never a proxied request.
#[test]
fn admin_endpoints_are_limited_to_admin_addresses() {
    let server = start(
        "adminips",
        r#"{"observability":{"profiler":{"enabled":true}}}"#,
        &[("index.cfm", "ok")],
        &[],
    );
    let status = |r: String| r.split_whitespace().nth(1).unwrap_or("").to_string();
    // From loopback, directly: not refused by the address gate.
    assert_ne!(status(get(server.port, "/__rustcfml/profiler")), "403");
    let proxied = get_with(server.port, "/__rustcfml/profiler", "X-Forwarded-For: 203.0.113.9\r\n");
    assert_eq!(status(proxied), "404");

    let locked = start(
        "adminips2",
        r#"{"server":{"adminAllowedIPs":["10.9.9.9"]},"observability":{"profiler":{"enabled":true}}}"#,
        &[("index.cfm", "ok")],
        &[],
    );
    let r = get(locked.port, "/__rustcfml/profiler");
    assert_eq!(status(r.clone()), "404", "{r}");
    assert!(!r.contains("\"requests\""), "profiler data served to a non-admin address:\n{r}");
}

/// `X-Forwarded-Proto` is believed only from `server.trustedProxies`, and
/// `server.allowedHosts` refuses a forged `Host`.
#[test]
fn forwarded_proto_and_host_are_not_client_controlled() {
    let page = "<cfoutput>https=[#cgi.https#] host=[#cgi.server_name#]</cfoutput>";
    // The test client is 127.0.0.1, so trust nobody to play an outside client.
    let untrusted = start("xfp", r#"{"server":{"trustedProxies":[]}}"#, &[("p.cfm", page)], &[]);
    let r = get_with(untrusted.port, "/p.cfm", "X-Forwarded-Proto: https\r\n");
    assert!(!r.contains("https=[on]") && !r.contains("https=[true]"), "client claimed https:\n{r}");

    let trusted = start("xfp2", "{}", &[("p.cfm", page)], &[]);
    let r = get_with(trusted.port, "/p.cfm", "X-Forwarded-Proto: https\r\n");
    assert!(r.contains("https=[on]") || r.contains("https=[true]"), "trusted proxy ignored:\n{r}");

    let hosts = start(
        "hosts",
        r#"{"server":{"allowedHosts":["127.0.0.1","*.example.com"]}}"#,
        &[("p.cfm", page)],
        &[],
    );
    let status = |r: String| r.split_whitespace().nth(1).unwrap_or("").to_string();
    assert_eq!(status(get(hosts.port, "/p.cfm")), "200");
    let forged = request(
        hosts.port,
        "GET /p.cfm HTTP/1.0\r\nHost: evil.test\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status(forged), "400");
    let sub = request(
        hosts.port,
        "GET /p.cfm HTTP/1.0\r\nHost: app.example.com:8080\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status(sub), "200");
}
