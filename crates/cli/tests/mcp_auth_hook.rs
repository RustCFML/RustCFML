//! MCP pluggable authentication (GH #436): the `authenticate` hook, the
//! identity reaching the handler through `mcp().identity()`, scope→tool
//! mapping, and the OAuth 2.1 resource-server seams.
//!
//! Driven over real HTTP (and stdio) against `tests/fixtures/mcp_hook_app`,
//! whose `auth/McpAuth.cfc` vouches for `user-*`, `reader` and refuses the rest.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

fn free_port() -> u16 {
    use std::sync::Mutex;
    static HANDED_OUT: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    for _ in 0..50 {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind ephemeral port")
            .local_addr()
            .unwrap()
            .port();
        let mut seen = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
        if !seen.contains(&port) {
            seen.push(port);
            return port;
        }
    }
    panic!("could not find an unused port in 50 attempts");
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_hook_app")
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

struct Res {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Res {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body was not JSON: {e}\nbody: {}", self.body))
    }
}

impl Server {
    fn start() -> Self {
        for _ in 1..=5 {
            let port = free_port();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
                .arg("--serve")
                .arg(fixtures_dir())
                .arg("--port")
                .arg(port.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn rustcfml --serve");
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if child.try_wait().expect("poll child").is_some() {
                    break;
                }
                if serves_http(port) {
                    return Server { child, port };
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        panic!("could not start `rustcfml --serve`");
    }

    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Res {
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            self.port
        );
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if !body.is_empty() {
            req.push_str("Content-Type: application/json\r\n");
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        req.push_str(body);

        let mut sock = std::net::TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        sock.set_read_timeout(Some(Duration::from_secs(30))).expect("timeout");
        sock.write_all(req.as_bytes()).expect("write");
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).expect("read");
        let raw = String::from_utf8_lossy(&raw);
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or_else(|| panic!("no status line in: {raw}"));
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        Res { status, headers, body: body.to_string() }
    }

    fn post(&self, headers: &[(&str, &str)], msg: Value) -> Res {
        self.request("POST", "/mcp/app", headers, &msg.to_string())
    }

    fn initialize_res(&self, token: &str) -> Res {
        let auth = format!("Bearer {token}");
        self.post(
            &[("Authorization", auth.as_str())],
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2025-06-18", "capabilities": {} } }),
        )
    }

    fn initialize(&self, token: &str) -> String {
        let res = self.initialize_res(token);
        assert_eq!(res.status, 200, "body: {}", res.body);
        res.header("mcp-session-id").expect("session id").to_string()
    }

    fn call(&self, token: &str, session: &str, tool: &str, extra: &[(&str, &str)]) -> Value {
        let auth = format!("Bearer {token}");
        let mut headers = vec![("Authorization", auth.as_str()), ("Mcp-Session-Id", session)];
        headers.extend_from_slice(extra);
        self.post(
            &headers,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": tool, "arguments": {} } }),
        )
        .json()
    }

    fn tool_names(&self, token: &str, session: &str) -> Vec<String> {
        let auth = format!("Bearer {token}");
        let listed = self.post(
            &[("Authorization", auth.as_str()), ("Mcp-Session-Id", session)],
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        );
        listed.json()["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect()
    }
}

fn serves_http(port: u16) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    // A TCP self-connect on Linux can "succeed" before the server binds.
    if s.local_addr().ok() == s.peer_addr().ok() {
        return false;
    }
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    if s.write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 12];
    let mut got = 0;
    while got < buf.len() {
        match s.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return false,
        }
    }
    buf[..got].starts_with(b"HTTP/")
}

#[test]
fn the_hook_authenticates_and_the_identity_reaches_the_handler() {
    let server = Server::start();
    let session = server.initialize("user-42");
    let result = server.call("user-42", &session, "whoami", &[("X-Probe", "hello")]);
    let who = &result["result"]["structuredContent"];
    assert_eq!(who["principalId"], "42", "got {result}");
    assert_eq!(who["tenantId"], "t1", "extra keys travel with the identity: {result}");
    assert_eq!(who["roles"], json!(["member"]));
    assert_eq!(who["via"], "http", "the hook is told the transport");
    assert_eq!(who["sawHeader"], "hello", "the hook sees the request headers");
    assert_eq!(who["transport"], "http");
    assert_eq!(who["authorization"], "Bearer user-42", "the raw header is exposed");

    // Its roles drive `secured` exactly as a static token's do.
    let result = server.call("user-42", &session, "members_only", &[]);
    assert_eq!(result["result"]["content"][0]["text"], "members ok", "got {result}");
}

#[test]
fn static_tokens_still_work_alongside_the_hook() {
    let server = Server::start();
    let session = server.initialize("static-token");
    let result = server.call("static-token", &session, "whoami", &[]);
    let who = &result["result"]["structuredContent"];
    assert_eq!(who["roles"], json!(["admin"]), "got {result}");
    assert_eq!(who["principalId"], "", "a static token carries no app identity");
}

#[test]
fn a_refusing_or_throwing_hook_is_a_terse_401() {
    let server = Server::start();
    for token in ["nobody", "throws", "disabled"] {
        let res = server.initialize_res(token);
        assert_eq!(res.status, 401, "{token}: {}", res.body);
        let message = res.json()["error"]["message"].as_str().unwrap_or_default().to_string();
        assert_eq!(message, "Not authorized", "{token} must not leak anything: {message:?}");
        assert!(!res.body.contains("secret internal detail"));
    }
    // No token at all is challenged before the hook is ever asked.
    let res = server.post(&[], json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
    assert_eq!(res.status, 401);
}

#[test]
fn a_scope_maps_to_the_tools_it_may_use() {
    let server = Server::start();
    let session = server.initialize("reader");
    let mut names = server.tool_names("reader", &session);
    names.sort();
    assert_eq!(names, vec!["get_status".to_string(), "whoami".to_string()]);

    let result = server.call("reader", &session, "set_status", &[]);
    assert!(
        result["error"]["message"].as_str().unwrap_or_default().contains("Unknown tool"),
        "a tool outside the scope is unknown to this caller: {result}"
    );
    // A member token has no scopes, so it sees everything.
    let session = server.initialize("user-1");
    assert_eq!(server.tool_names("user-1", &session).len(), 4);
}

#[test]
fn a_401_points_the_client_at_the_protected_resource_metadata() {
    let server = Server::start();
    let res = server.initialize_res("nobody");
    let challenge = res.header("www-authenticate").expect("challenge");
    let expected = format!(
        "Bearer resource_metadata=\"http://127.0.0.1:{}/.well-known/oauth-protected-resource/mcp/app\"",
        server.port
    );
    assert_eq!(challenge, expected);
}

#[test]
fn the_protected_resource_metadata_is_served() {
    let server = Server::start();
    let res = server.request("GET", "/.well-known/oauth-protected-resource/mcp/app", &[], "");
    assert_eq!(res.status, 200, "body: {}", res.body);
    let doc = res.json();
    assert_eq!(doc["resource"], format!("http://127.0.0.1:{}/mcp/app", server.port));
    assert_eq!(doc["authorization_servers"], json!(["https://auth.example.com"]));
    assert_eq!(doc["scopes_supported"], json!(["read", "write"]));
    assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
    assert_eq!(doc["resource_name"], "Hook fixture", "extra metadata passes through");

    let res = server.request("GET", "/.well-known/oauth-protected-resource", &[], "");
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["resource"], format!("http://127.0.0.1:{}", server.port));
}

#[test]
fn the_authorization_server_paths_stay_with_the_application() {
    let server = Server::start();
    let res = server.request("GET", "/.well-known/oauth-authorization-server", &[], "");
    assert_eq!(res.status, 200, "body: {}", res.body);
    assert!(res.body.contains("app-owned"), "the app's own file is served: {}", res.body);
}

/// stdio: the credential comes from the environment variable the config names.
fn stdio_whoami(token: Option<&str>) -> (Option<Value>, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustcfml"));
    cmd.arg("mcp").arg("app").arg(fixtures_dir());
    match token {
        Some(t) => cmd.env("MCP_HOOK_TEST_TOKEN", t),
        None => cmd.env_remove("MCP_HOOK_TEST_TOKEN"),
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rustcfml mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": { "protocolVersion": "2025-06-18", "capabilities": {} } });
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                       "params": { "name": "whoami", "arguments": {} } });
    let _ = writeln!(stdin, "{init}");
    let _ = writeln!(stdin, "{call}");
    let _ = stdin.flush();
    drop(stdin);
    let mut last = None;
    let mut line = String::new();
    while stdout.read_line(&mut line).unwrap_or(0) > 0 {
        last = serde_json::from_str::<Value>(&line).ok();
        line.clear();
    }
    let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
    (last, code)
}

#[test]
fn stdio_resolves_its_credential_through_the_hook() {
    let (reply, code) = stdio_whoami(Some("user-7"));
    assert_eq!(code, 0);
    let who = &reply.expect("reply")["result"]["structuredContent"];
    assert_eq!(who["principalId"], "7", "got {who}");
    assert_eq!(who["via"], "stdio");
    assert_eq!(who["transport"], "stdio");
}

#[test]
fn stdio_with_a_refused_credential_exits_rather_than_serving_someone_else() {
    let (reply, code) = stdio_whoami(Some("nobody"));
    assert!(reply.is_none(), "nothing may be served: {reply:?}");
    assert_eq!(code, 1);
}

#[test]
fn stdio_without_the_variable_is_the_launcher_as_before() {
    let (reply, code) = stdio_whoami(None);
    assert_eq!(code, 0);
    let who = &reply.expect("reply")["result"]["structuredContent"];
    assert_eq!(who["roles"], json!([]), "got {who}");
    assert_eq!(who["transport"], "stdio");
}
