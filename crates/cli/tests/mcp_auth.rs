//! MCP authentication, address rules and tool filtering.
//!
//! Driven over real HTTP against `tests/fixtures/mcp_auth_app`, whose
//! `.cfconfig.json` declares three tokens with different roles and filters.
//! These are the tests that prove `secured` handlers are reachable at all —
//! before authentication existed they could only ever be refused.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_auth_app")
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
        for attempt in 1..=5 {
            let port = free_port();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
                .arg("--serve")
                .arg(fixtures_dir())
                .arg("--port")
                .arg(port.to_string())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
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
            let _ = attempt;
        }
        panic!("could not start `rustcfml --serve`");
    }

    fn request(&self, method: &str, headers: &[(&str, &str)], body: &str) -> Res {
        let mut req = format!(
            "{method} /mcp/secure HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
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

        let mut sock =
            std::net::TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
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
        self.request("POST", headers, &msg.to_string())
    }

    /// Handshake with a token and return the session id.
    fn initialize(&self, token: &str) -> String {
        let auth = format!("Bearer {token}");
        let res = self.post(
            &[("Authorization", auth.as_str())],
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2025-06-18", "capabilities": {} } }),
        );
        assert_eq!(res.status, 200, "body: {}", res.body);
        res.header("mcp-session-id").expect("session id").to_string()
    }

    /// Call a tool as the holder of `token`.
    fn call(&self, token: &str, session: &str, tool: &str) -> Value {
        let auth = format!("Bearer {token}");
        self.post(
            &[("Authorization", auth.as_str()), ("Mcp-Session-Id", session)],
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": tool, "arguments": {} } }),
        )
        .json()
    }
}

fn serves_http(port: u16) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
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
fn a_request_without_a_token_is_challenged() {
    let server = Server::start();
    let res = server.post(
        &[],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {} } }),
    );
    assert_eq!(res.status, 401, "body: {}", res.body);
    assert_eq!(
        res.header("www-authenticate"),
        Some("Bearer"),
        "a 401 must tell the client how to authenticate"
    );
}

#[test]
fn a_wrong_token_is_refused_and_says_nothing_useful() {
    let server = Server::start();
    let res = server.post(
        &[("Authorization", "Bearer not-the-token")],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    assert_eq!(res.status, 401);
    // A probe must not be able to tell a wrong token from a blocked address.
    let body = res.json();
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert_eq!(message, "Not authorized", "got {message:?}");
}

#[test]
fn a_secured_handler_is_reachable_with_the_right_role() {
    // The point of the whole phase: before authentication existed, every
    // `secured` handler could only ever be refused.
    let server = Server::start();

    let session = server.initialize("admin-token");
    let result = server.call("admin-token", &session, "admin_tool");
    assert_eq!(result["result"]["content"][0]["text"], "admin ok", "got {result}");
    assert_eq!(result["result"]["isError"], false);

    // A token with no roles is authenticated, so a bare `secured` passes…
    let session = server.initialize("plain-token");
    let result = server.call("plain-token", &session, "login_tool");
    assert_eq!(result["result"]["content"][0]["text"], "login ok", "got {result}");

    // …but a role it does not hold is still refused.
    let result = server.call("plain-token", &session, "admin_tool");
    assert_eq!(result["error"]["code"], -32600, "got {result}");
    assert!(result["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("Not authorized"));
}

#[test]
fn an_unsecured_tool_is_callable_by_any_valid_token() {
    let server = Server::start();
    let session = server.initialize("plain-token");
    let result = server.call("plain-token", &session, "open_tool");
    assert_eq!(result["result"]["content"][0]["text"], "open");
}

#[test]
fn excluded_tools_are_neither_listed_nor_callable() {
    let server = Server::start();
    let session = server.initialize("admin-token");

    let listed = server.post(
        &[("Authorization", "Bearer admin-token"), ("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let names: Vec<String> = listed.json()["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    assert!(names.contains(&"open_tool".to_string()), "got {names:?}");
    assert!(
        !names.contains(&"delete_everything".to_string()),
        "an excluded tool must not be advertised: {names:?}"
    );

    // And calling it directly is refused — as "unknown", so a caller cannot
    // discover that a tool it may not use exists.
    let result = server.call("admin-token", &session, "delete_everything");
    assert_eq!(result["error"]["code"], -32602, "got {result}");
    assert!(result["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("Unknown tool"));
}

#[test]
fn a_tokens_own_filter_narrows_what_it_can_see() {
    let server = Server::start();
    let session = server.initialize("readonly-token");

    let listed = server.post(
        &[("Authorization", "Bearer readonly-token"), ("Mcp-Session-Id", session.as_str())],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let names: Vec<String> = listed.json()["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    assert_eq!(names, vec!["open_tool".to_string()], "the token allows only open_*");

    let result = server.call("readonly-token", &session, "admin_tool");
    assert_eq!(result["error"]["code"], -32602, "got {result}");
}

#[test]
fn the_token_also_gates_the_other_methods() {
    let server = Server::start();
    let session = server.initialize("admin-token");

    // GET and DELETE go through the same guard as POST.
    let res = server.request("GET", &[("Mcp-Session-Id", session.as_str())], "");
    assert_eq!(res.status, 401, "an unauthenticated GET must not open a stream");

    let res = server.request("DELETE", &[("Mcp-Session-Id", session.as_str())], "");
    assert_eq!(res.status, 401);

    let auth = "Bearer admin-token";
    let res = server.request(
        "DELETE",
        &[("Authorization", auth), ("Mcp-Session-Id", session.as_str())],
        "",
    );
    assert_eq!(res.status, 204);
}
