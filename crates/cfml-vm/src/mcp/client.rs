//! The MCP **client** — CFML calling out to a remote MCP server.
//!
//! The mirror image of the rest of this module: instead of exposing CFML
//! functions as tools, this consumes somebody else's. Both transports the
//! ecosystem actually uses are supported — launching a server as a subprocess
//! (stdio) and talking to one over Streamable HTTP.
//!
//! Native-only, behind the `mcp-client` feature, because it needs subprocesses
//! and an HTTP client. `cfml-vm` must keep building for `wasm32`, where neither
//! exists — the same reason the `s3` surface is feature-gated.

use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::content;
use super::protocol::{self, methods, Incoming, Outgoing, RpcId, LATEST_PROTOCOL_VERSION};

/// How long to wait for a remote server to answer one request.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// One way of talking to a remote server. Blocking throughout — the VM is
/// synchronous, and a CFML author expects `client.call(...)` to return a value,
/// not a future.
pub trait Transport: Send + Sync + Debug {
    /// Send a request and wait for its response.
    fn request(&self, id: RpcId, method: &str, params: Value) -> Result<Value, String>;
    /// Send a notification, which by definition has no reply.
    fn notify(&self, method: &str, params: Value) -> Result<(), String>;
    /// Shut the connection down.
    fn close(&self);
    fn describe(&self) -> String;
}

// ── stdio ────────────────────────────────────────────────────────────────

/// A server we launched as a subprocess, spoken to over its pipes.
#[derive(Debug)]
pub struct StdioTransport {
    command: String,
    child: Mutex<Option<std::process::Child>>,
    stdin: Mutex<std::process::ChildStdin>,
    stdout: Mutex<BufReader<std::process::ChildStdout>>,
}

impl StdioTransport {
    pub fn spawn(command: &str, env: &[(String, String)]) -> Result<Self, String> {
        let parts = crate::cmdline::tokenize_arguments(command);
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| "mcpConnect: the stdio command is empty".to_string())?;

        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // The server's stderr is its log. Inheriting it puts a failing
            // server's own diagnostics in front of the developer instead of
            // swallowing them into a pipe nobody reads.
            .stderr(std::process::Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("mcpConnect: could not start [{command}]: {e}"))?;
        let stdin = child.stdin.take().ok_or("mcpConnect: no stdin on the child")?;
        let stdout = child.stdout.take().ok_or("mcpConnect: no stdout on the child")?;
        Ok(Self {
            command: command.to_string(),
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
        })
    }

    fn write_line(&self, msg: &Outgoing) -> Result<(), String> {
        let mut stdin = self.stdin.lock();
        writeln!(stdin, "{}", msg.to_line())
            .and_then(|_| stdin.flush())
            .map_err(|e| format!("MCP server closed its input: {e}"))
    }
}

impl Transport for StdioTransport {
    fn request(&self, id: RpcId, method: &str, params: Value) -> Result<Value, String> {
        self.write_line(&Outgoing::Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        })?;

        // Read until our own answer appears. Anything else on the pipe is the
        // server talking to us: notifications are logged and dropped, and a
        // request we cannot serve is declined immediately rather than ignored
        // — an unanswered request would leave the server parked until *its*
        // timeout for no reason.
        let mut reader = self.stdout.lock();
        loop {
            let mut line = String::new();
            let read = reader
                .read_line(&mut line)
                .map_err(|e| format!("MCP server read failed: {e}"))?;
            if read == 0 {
                return Err("MCP server closed the connection".to_string());
            }
            if line.trim().is_empty() {
                continue;
            }
            match protocol::decode(line.as_bytes()) {
                Ok(messages) => {
                    for msg in messages {
                        match msg {
                            Incoming::Response { id: got, result } if got == id => {
                                return Ok(result)
                            }
                            Incoming::Error { id: got, error } if got == id => {
                                return Err(rpc_error_text(&error))
                            }
                            Incoming::Request { id: theirs, method, .. } => {
                                let _ = self.write_line(&Outgoing::error(
                                    theirs,
                                    protocol::METHOD_NOT_FOUND,
                                    format!("This MCP client does not implement {method}"),
                                ));
                            }
                            Incoming::Notification { method, .. } => {
                                log::debug!("MCP client: ignoring notification {method}");
                            }
                            // A response to some other request — nothing is
                            // waiting on it, since requests are serialized.
                            _ => {}
                        }
                    }
                }
                Err((_, e)) => log::warn!("MCP client: undecodable line from server: {e}"),
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_line(&Outgoing::Notification { method: method.to_string(), params })
    }

    fn close(&self) {
        // Closing stdin is the polite shutdown the spec describes; the kill is
        // the backstop for a server that ignores it.
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn describe(&self) -> String {
        format!("stdio: {}", self.command)
    }
}

// ── Streamable HTTP ──────────────────────────────────────────────────────

/// A server reached over HTTP, per the Streamable HTTP transport.
#[derive(Debug)]
pub struct HttpTransport {
    url: String,
    headers: Vec<(String, String)>,
    session: Mutex<Option<String>>,
    timeout: std::time::Duration,
}

impl HttpTransport {
    pub fn new(url: &str, headers: Vec<(String, String)>, timeout_secs: u64) -> Self {
        Self {
            url: url.to_string(),
            headers,
            session: Mutex::new(None),
            timeout: std::time::Duration::from_secs(timeout_secs),
        }
    }

    fn post(&self, body: &Value) -> Result<(Option<String>, String, String), String> {
        let mut req = ureq::post(&self.url)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            // Both are required of a client by the spec; a server may answer
            // with either, and refusing one of them is a protocol error.
            .set("Accept", "application/json, text/event-stream")
            .set("MCP-Protocol-Version", LATEST_PROTOCOL_VERSION);
        if let Some(session) = self.session.lock().as_ref() {
            req = req.set("Mcp-Session-Id", session);
        }
        for (k, v) in &self.headers {
            req = req.set(k, v);
        }

        let resp = match req.send_string(&body.to_string()) {
            Ok(r) => r,
            // A 4xx/5xx still carries a JSON-RPC error body worth surfacing.
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(format!("MCP server returned HTTP {code}: {}", text.trim()));
            }
            Err(e) => return Err(format!("MCP server unreachable: {e}")),
        };
        let session = resp.header("Mcp-Session-Id").map(|s| s.to_string());
        let content_type = resp.header("Content-Type").unwrap_or("").to_string();
        let text = resp.into_string().map_err(|e| format!("MCP response unreadable: {e}"))?;
        Ok((session, content_type, text))
    }
}

/// Pull JSON-RPC payloads out of an SSE body.
///
/// A server MAY answer a request with an event stream, in which case the
/// response is among the events (usually last, after progress notifications).
fn sse_payloads(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for block in body.split("\n\n") {
        let mut data = String::new();
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.trim_start());
            }
        }
        if data.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&data) {
            out.push(v);
        }
    }
    out
}

impl Transport for HttpTransport {
    fn request(&self, id: RpcId, method: &str, params: Value) -> Result<Value, String> {
        let body = Outgoing::Request { id: id.clone(), method: method.to_string(), params }
            .to_json();
        let (session, content_type, text) = self.post(&body)?;
        if let Some(session) = session {
            *self.session.lock() = Some(session);
        }

        let candidates: Vec<Value> = if content_type.contains("text/event-stream") {
            sse_payloads(&text)
        } else {
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Array(items)) => items,
                Ok(one) => vec![one],
                Err(e) => return Err(format!("MCP response was not JSON: {e}")),
            }
        };

        for value in candidates {
            let Ok(messages) = protocol::decode(value.to_string().as_bytes()) else {
                continue;
            };
            for msg in messages {
                match msg {
                    Incoming::Response { id: got, result } if got == id => return Ok(result),
                    Incoming::Error { id: got, error } if got == id => {
                        return Err(rpc_error_text(&error))
                    }
                    _ => {}
                }
            }
        }
        Err(format!("MCP server sent no response to {method}"))
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let body = Outgoing::Notification { method: method.to_string(), params }.to_json();
        self.post(&body).map(|_| ())
    }

    fn close(&self) {
        // Best effort: the spec's explicit teardown. A server that refuses it
        // (405) is entitled to, and there is nothing to do about it.
        let session = self.session.lock().clone();
        if let Some(session) = session {
            let _ = ureq::delete(&self.url)
                .timeout(self.timeout)
                .set("Mcp-Session-Id", &session)
                .call();
        }
    }

    fn describe(&self) -> String {
        format!("http: {}", self.url)
    }
}

fn rpc_error_text(error: &Value) -> String {
    let message =
        error.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
    match error.get("code").and_then(|c| c.as_i64()) {
        Some(code) => format!("{message} (JSON-RPC {code})"),
        None => message.to_string(),
    }
}

// ── the CFML-facing object ───────────────────────────────────────────────

/// What `mcpConnect()` / `mcpClient()` hand back.
#[derive(Debug)]
pub struct McpClient {
    transport: Arc<dyn Transport>,
    seq: AtomicU64,
    /// The remote's `initialize` result: its info, capabilities and version.
    server_info: Mutex<Value>,
    closed: std::sync::atomic::AtomicBool,
}

impl McpClient {
    /// Connect and complete the MCP handshake.
    ///
    /// The handshake is done here rather than lazily so a bad endpoint fails at
    /// `mcpConnect()`, where the developer is looking, instead of at the first
    /// `tools()` call somewhere else entirely.
    pub fn connect(transport: Arc<dyn Transport>, client_name: &str) -> Result<Self, String> {
        let client = Self {
            transport,
            seq: AtomicU64::new(0),
            server_info: Mutex::new(Value::Null),
            closed: std::sync::atomic::AtomicBool::new(false),
        };
        let result = client.request(
            methods::INITIALIZE,
            json!({
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                // We declare no capabilities: this client cannot serve a
                // sampling or elicitation request, and claiming otherwise
                // would leave a server parked waiting for an answer.
                "capabilities": {},
                "clientInfo": { "name": client_name, "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        *client.server_info.lock() = result;
        // The spec requires this before any other request.
        client.transport.notify(methods::INITIALIZED, json!({}))?;
        Ok(client)
    }

    fn next_id(&self) -> RpcId {
        RpcId::Num(self.seq.fetch_add(1, Ordering::Relaxed) as i64 + 1)
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.closed.load(Ordering::Relaxed) {
            return Err("This MCP client has been closed".to_string());
        }
        self.transport.request(self.next_id(), method, params)
    }

    /// Run a request and hand the named key of its result back to CFML.
    fn list(&self, method: &str, key: &str) -> CfmlResult {
        let result = self.request(method, json!({})).map_err(CfmlError::runtime)?;
        let items = result.get(key).cloned().unwrap_or_else(|| json!([]));
        Ok(crate::json_value_to_cfml(items))
    }

    fn call_tool(&self, args: &[CfmlValue]) -> CfmlResult {
        let name = args.first().map(|v| v.as_string()).unwrap_or_default();
        if name.is_empty() {
            return Err(CfmlError::runtime("mcp client: call() needs a tool name".to_string()));
        }
        let arguments = args
            .get(1)
            .filter(|v| !matches!(v, CfmlValue::Null))
            .map(content::to_json)
            .unwrap_or_else(|| json!({}));
        let result = self
            .request(methods::TOOLS_CALL, json!({ "name": name, "arguments": arguments }))
            .map_err(CfmlError::runtime)?;

        // A tool that reported failure is an exception here: CFML's way of
        // saying "this did not work" is a throw, and silently returning the
        // error text as if it were the answer is how bad data spreads.
        if result.get("isError").and_then(|e| e.as_bool()).unwrap_or(false) {
            return Err(CfmlError::new(
                format!("MCP tool [{name}] failed: {}", first_text(&result)),
                cfml_common::vm::CfmlErrorType::Custom("MCPToolError".to_string()),
            ));
        }
        // Structured output when the tool provided it, else the text blocks.
        if let Some(structured) = result.get("structuredContent") {
            return Ok(crate::json_value_to_cfml(structured.clone()));
        }
        Ok(crate::json_value_to_cfml(result))
    }
}

/// The first text block of a result, for error messages.
fn first_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| blocks.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")))
        .and_then(|b| b.get("text").and_then(|t| t.as_str()))
        .unwrap_or("no detail given")
        .to_string()
}

impl CfmlNative for McpClient {
    fn class_name(&self) -> &str {
        "McpClient"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_lowercase().as_str() {
            "tools" => self.list(methods::TOOLS_LIST, "tools"),
            "resources" => self.list(methods::RESOURCES_LIST, "resources"),
            "resourcetemplates" => {
                self.list(methods::RESOURCES_TEMPLATES_LIST, "resourceTemplates")
            }
            "prompts" => self.list(methods::PROMPTS_LIST, "prompts"),
            "call" | "calltool" => self.call_tool(&args),
            "read" => {
                let uri = args.first().map(|v| v.as_string()).unwrap_or_default();
                if uri.is_empty() {
                    return Err(CfmlError::runtime(
                        "mcp client: read() needs a uri".to_string(),
                    ));
                }
                let result = self
                    .request(methods::RESOURCES_READ, json!({ "uri": uri }))
                    .map_err(CfmlError::runtime)?;
                Ok(crate::json_value_to_cfml(
                    result.get("contents").cloned().unwrap_or_else(|| json!([])),
                ))
            }
            "prompt" => {
                let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                let arguments = args
                    .get(1)
                    .filter(|v| !matches!(v, CfmlValue::Null))
                    .map(content::to_json)
                    .unwrap_or_else(|| json!({}));
                let result = self
                    .request(
                        methods::PROMPTS_GET,
                        json!({ "name": name, "arguments": arguments }),
                    )
                    .map_err(CfmlError::runtime)?;
                Ok(crate::json_value_to_cfml(result))
            }
            "ping" => {
                self.request(methods::PING, json!({})).map_err(CfmlError::runtime)?;
                Ok(CfmlValue::Bool(true))
            }
            // Escape hatch for anything this surface does not wrap.
            "request" => {
                let method = args.first().map(|v| v.as_string()).unwrap_or_default();
                if method.is_empty() {
                    return Err(CfmlError::runtime(
                        "mcp client: request() needs a method name".to_string(),
                    ));
                }
                let params = args
                    .get(1)
                    .filter(|v| !matches!(v, CfmlValue::Null))
                    .map(content::to_json)
                    .unwrap_or_else(|| json!({}));
                let result = self.request(&method, params).map_err(CfmlError::runtime)?;
                Ok(crate::json_value_to_cfml(result))
            }
            "info" | "serverinfo" => {
                Ok(crate::json_value_to_cfml(self.server_info.lock().clone()))
            }
            "close" => {
                if !self.closed.swap(true, Ordering::Relaxed) {
                    self.transport.close();
                }
                Ok(CfmlValue::Bool(true))
            }
            "isclosed" => Ok(CfmlValue::Bool(self.closed.load(Ordering::Relaxed))),
            "describe" => Ok(CfmlValue::string(self.transport.describe())),
            other => Err(CfmlError::runtime(format!(
                "Unknown method [{other}] on an MCP client"
            ))),
        }
    }

    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_lowercase().as_str() {
            "call" | "calltool" => &["name", "arguments"],
            "read" => &["uri"],
            "prompt" => &["name", "arguments"],
            "request" => &["method", "params"],
            _ => return None,
        })
    }

    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        match name.to_lowercase().as_str() {
            "info" => Some(crate::json_value_to_cfml(self.server_info.lock().clone())),
            _ => None,
        }
    }
}

impl Drop for McpClient {
    /// A client left open by a page that threw must not leave a subprocess
    /// running. CFML has no `using`, so the drop is the safety net.
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            self.transport.close();
        }
    }
}

/// Build a client from the `mcpConnect()` arguments.
pub fn connect_from_options(kind: &str, options: &ValueMap) -> Result<McpClient, String> {
    let get = |key: &str| {
        options
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    let timeout = get("timeout")
        .and_then(|v| v.as_string().parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .map(|s| s as u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    let transport: Arc<dyn Transport> = match kind.to_lowercase().as_str() {
        "stdio" | "" => {
            let command = get("command").map(|v| v.as_string()).unwrap_or_default();
            if command.is_empty() {
                return Err("mcpConnect: a stdio connection needs a `command`".to_string());
            }
            let env: Vec<(String, String)> = match get("env") {
                Some(CfmlValue::Struct(s)) => {
                    s.iter().map(|(k, v)| (k.as_str().to_string(), v.as_string())).collect()
                }
                _ => Vec::new(),
            };
            Arc::new(StdioTransport::spawn(&command, &env)?)
        }
        "http" | "https" | "streamable-http" => {
            let url = get("url").map(|v| v.as_string()).unwrap_or_default();
            if url.is_empty() {
                return Err("mcpConnect: an http connection needs a `url`".to_string());
            }
            let headers: Vec<(String, String)> = match get("headers") {
                Some(CfmlValue::Struct(s)) => {
                    s.iter().map(|(k, v)| (k.as_str().to_string(), v.as_string())).collect()
                }
                _ => Vec::new(),
            };
            Arc::new(HttpTransport::new(&url, headers, timeout))
        }
        other => {
            return Err(format!(
                "mcpConnect: unknown transport [{other}] — expected \"stdio\" or \"http\""
            ))
        }
    };
    let name = get("name").map(|v| v.as_string()).unwrap_or_else(|| "rustcfml".to_string());
    McpClient::connect(transport, &name)
}

/// Named servers from `.cfconfig.json`, in the same shape a client config file
/// uses (`{ "mcpServers": { "github": { "command": …, "url": … } } }`), so a
/// config can be pasted from an editor's settings without translation.
pub fn options_for_named_server(config: &CfmlValue, name: &str) -> Option<(String, ValueMap)> {
    let CfmlValue::Struct(config) = config else { return None };
    let CfmlValue::Struct(servers) = config.get_ci("mcpServers")? else { return None };
    let CfmlValue::Struct(entry) = servers.get_ci(name)? else { return None };
    let mut options = ValueMap::default();
    for (k, v) in entry.iter() {
        options.insert(k.as_str().to_string(), v.clone());
    }
    // `args` alongside `command` is the client-config convention; fold it in so
    // the transport sees one command line.
    if let (Some(command), Some(CfmlValue::Array(args))) =
        (options.get("command").cloned(), options.get("args").cloned())
    {
        let joined = std::iter::once(command.as_string())
            .chain(args.iter().map(|a| quote_arg(&a.as_string())))
            .collect::<Vec<_>>()
            .join(" ");
        options.insert("command".to_string(), CfmlValue::string(joined));
    }
    let kind = if options.contains_key("url") { "http" } else { "stdio" };
    Some((kind.to_string(), options))
}

/// Quote an argument that contains spaces, so re-joining a client config's
/// `args` array and re-tokenizing it is lossless.
fn quote_arg(arg: &str) -> String {
    if arg.contains(' ') && !arg.starts_with('"') {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_payloads_are_extracted_from_an_event_stream() {
        let body = "id: 0-0\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n\n\
                    id: 0-1\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let payloads = sse_payloads(body);
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[1]["id"], 1);
    }

    #[test]
    fn multi_line_sse_data_is_rejoined() {
        // A real server splits a multi-line payload with one `data:` per line
        // and no leading space on the continuation.
        let payloads = sse_payloads("data: {\"a\":\ndata: 1}\n\n");
        assert_eq!(payloads.len(), 1, "a payload split across data: lines must rejoin");
        assert_eq!(payloads[0]["a"], 1);
    }

    #[test]
    fn keepalive_comments_are_not_payloads() {
        assert!(sse_payloads(":ping\n\n").is_empty());
    }

    #[test]
    fn an_error_body_is_rendered_with_its_code() {
        let text = rpc_error_text(&json!({ "code": -32601, "message": "Method not found" }));
        assert_eq!(text, "Method not found (JSON-RPC -32601)");
        assert_eq!(rpc_error_text(&json!({})), "unknown error");
    }

    #[test]
    fn unknown_transports_and_missing_targets_are_refused() {
        assert!(connect_from_options("carrier-pigeon", &ValueMap::default())
            .unwrap_err()
            .contains("unknown transport"));
        assert!(connect_from_options("stdio", &ValueMap::default())
            .unwrap_err()
            .contains("needs a `command`"));
        assert!(connect_from_options("http", &ValueMap::default())
            .unwrap_err()
            .contains("needs a `url`"));
    }

    #[test]
    fn a_named_server_reads_a_client_config_shape_verbatim() {
        let mut entry = ValueMap::default();
        entry.insert("command".to_string(), CfmlValue::string("npx"));
        entry.insert(
            "args".to_string(),
            CfmlValue::array(vec![
                CfmlValue::string("-y"),
                CfmlValue::string("@modelcontextprotocol/server-filesystem"),
                CfmlValue::string("/tmp/my docs"),
            ]),
        );
        let mut servers = ValueMap::default();
        servers.insert("files".to_string(), CfmlValue::strukt(entry));
        let mut config = ValueMap::default();
        config.insert("mcpServers".to_string(), CfmlValue::strukt(servers));

        let (kind, options) =
            options_for_named_server(&CfmlValue::strukt(config), "files").expect("found");
        assert_eq!(kind, "stdio");
        assert_eq!(
            options.get("command").unwrap().as_string(),
            "npx -y @modelcontextprotocol/server-filesystem \"/tmp/my docs\"",
            "args are folded into the command line, quoting what needs it"
        );
    }

    #[test]
    fn a_named_server_with_a_url_is_an_http_connection() {
        let mut entry = ValueMap::default();
        entry.insert("url".to_string(), CfmlValue::string("https://example.com/mcp"));
        let mut servers = ValueMap::default();
        servers.insert("remote".to_string(), CfmlValue::strukt(entry));
        let mut config = ValueMap::default();
        config.insert("mcpServers".to_string(), CfmlValue::strukt(servers));

        let (kind, _) =
            options_for_named_server(&CfmlValue::strukt(config), "remote").expect("found");
        assert_eq!(kind, "http");
        assert!(options_for_named_server(&CfmlValue::strukt(ValueMap::default()), "x").is_none());
    }
}
