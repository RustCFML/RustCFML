//! MCP stdio-transport integration tests.
//!
//! These drive the real binary the way a client does — `rustcfml mcp demo`
//! with newline-delimited JSON-RPC on the pipes — against the fixture server
//! in `tests/fixtures/mcp_app/mcp/demo.cfc`.
//!
//! CFML cannot act as an MCP client, so, exactly as with WebSockets, these
//! Rust tests are the source of truth for the protocol surface.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_app")
}

/// A live `rustcfml mcp` subprocess, killed on drop.
struct Server {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start() -> Self {
        Server::start_named("demo")
    }

    fn start_named(name: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
            .arg("mcp")
            .arg(name)
            .arg(fixtures_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn `rustcfml mcp {name}`: {e}"));
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        Self { child, stdin, stdout }
    }

    /// Send one message and read the single reply it produces.
    fn call(&mut self, msg: Value) -> Value {
        self.send(msg);
        self.read()
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").expect("write to server stdin");
        self.stdin.flush().expect("flush");
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read from server stdout");
        assert!(n > 0, "server closed stdout without replying");
        serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("server wrote non-JSON to stdout: {e}\nline: {line}"))
    }

    fn initialize(&mut self) -> Value {
        self.call(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "integration-test", "version": "1" }
            }
        }))
    }
}

#[test]
fn initialize_negotiates_and_advertises_only_declared_capabilities() {
    let mut s = Server::start();
    let reply = s.initialize();

    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply["result"]["serverInfo"]["name"], "demo");
    assert_eq!(reply["result"]["serverInfo"]["version"], "1.2.3");
    assert_eq!(reply["result"]["capabilities"]["tools"]["listChanged"], true);
}

#[test]
fn a_server_never_advertises_a_capability_it_has_no_entities_for() {
    // `toolsonly.cfc` declares tools and nothing else. Advertising `resources`
    // there would have clients issue calls that cannot succeed.
    let mut s = Server::start_named("toolsonly");
    let caps = s.initialize()["result"]["capabilities"].clone();
    assert_eq!(caps["tools"]["listChanged"], true);
    assert!(caps.get("resources").is_none(), "got {caps}");
    assert!(caps.get("prompts").is_none(), "got {caps}");
}

#[test]
fn an_unknown_protocol_version_negotiates_down_rather_than_failing() {
    let mut s = Server::start();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "1999-01-01", "capabilities": {} }
    }));
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");

    let mut s = Server::start();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));
    assert_eq!(reply["result"]["protocolVersion"], "2024-11-05", "older clients are honoured");
}

#[test]
fn tools_list_derives_schemas_from_the_cfml_signature() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));

    let tools = reply["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"echo"), "got {names:?}");
    assert!(names.contains(&"stats"));
    // A `private` method annotated `tool=` must never be exposed.
    assert!(!names.contains(&"helper"), "private method leaked into tools/list: {names:?}");

    let echo = tools.iter().find(|t| t["name"] == "echo").expect("echo tool");
    assert_eq!(echo["description"], "Echo the given text");
    assert_eq!(echo["annotations"]["readOnlyHint"], true);
    let schema = &echo["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["text"]["type"], "string");
    // The `@text.description` doc annotation becomes the parameter description.
    assert_eq!(schema["properties"]["text"]["description"], "The text to echo back");
    assert_eq!(schema["properties"]["times"]["type"], "number");
    assert_eq!(schema["properties"]["times"]["default"], 1);
    // `times` has a default, so it is optional even though `text` is not.
    assert_eq!(schema["required"], json!(["text"]));
}

#[test]
fn tools_call_binds_named_arguments_and_honours_defaults() {
    let mut s = Server::start();
    s.initialize();

    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "echo", "arguments": { "text": "ab", "times": 3 } }
    }));
    assert_eq!(reply["result"]["content"][0]["type"], "text");
    assert_eq!(reply["result"]["content"][0]["text"], "ababab");
    assert_eq!(reply["result"]["isError"], false);

    // `times` omitted → the CFML default of 1 applies, not null.
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "echo", "arguments": { "text": "solo" } }
    }));
    assert_eq!(reply["result"]["content"][0]["text"], "solo");
}

#[test]
fn a_struct_return_carries_structured_content_and_serialized_text() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "stats", "arguments": { "label": "hits" } }
    }));
    let result = &reply["result"];
    assert_eq!(result["structuredContent"]["label"], "hits");
    assert_eq!(result["structuredContent"]["total"], 42);
    assert_eq!(result["structuredContent"]["ok"], true);
    // The spec's backwards-compatibility rule: the same JSON also rides as text.
    let text = result["content"][0]["text"].as_str().expect("text block");
    let reparsed: Value = serde_json::from_str(text).expect("text block is the serialized JSON");
    assert_eq!(reparsed, result["structuredContent"]);
}

#[test]
fn a_query_becomes_an_array_of_row_objects() {
    // Lucee's `{COLUMNS, DATA}` envelope means nothing to an MCP client, so a
    // query is serialized the way any other consumer would expect.
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "rows", "arguments": {} }
    }));
    let rows = &reply["result"]["structuredContent"];
    assert_eq!(rows, &json!([
        { "id": 1, "title": "first" },
        { "id": 2, "title": "second" }
    ]), "got {rows}");
}

#[test]
fn a_thrown_exception_is_a_tool_error_not_a_protocol_error() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "explode", "arguments": {} }
    }));
    assert!(reply.get("error").is_none(), "a failing tool is a result, not a JSON-RPC error");
    assert_eq!(reply["result"]["isError"], true);
    let text = reply["result"]["content"][0]["text"].as_str().expect("message");
    assert!(text.contains("tool blew up"), "got {text:?}");
    // A stack trace names CFC paths and line numbers; it must not reach a client.
    assert!(!text.contains("Stack trace"), "leaked a stack trace: {text}");
    assert!(!text.contains(".cfc"), "leaked a source path: {text}");
}

#[test]
fn a_secured_tool_is_refused_at_the_protocol_level() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "hidden", "arguments": {} }
    }));
    // A refusal is a JSON-RPC error, so the model is told "you may not", not
    // "that failed, try again".
    assert_eq!(reply["error"]["code"], -32600);
    let message = reply["error"]["message"].as_str().expect("message");
    assert!(message.contains("Not authorized"), "got {message:?}");
    assert!(!message.contains(".cfc"), "leaked a source path: {message}");
}

#[test]
fn unknown_tools_and_methods_get_the_right_json_rpc_codes() {
    let mut s = Server::start();
    s.initialize();

    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "no_such_tool", "arguments": {} }
    }));
    assert_eq!(reply["error"]["code"], -32602, "unknown tool is an invalid param");

    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 3, "method": "no/such/method" }));
    assert_eq!(reply["error"]["code"], -32601);

    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "arguments": {} }
    }));
    assert_eq!(reply["error"]["code"], -32602, "tools/call needs a name");
}

#[test]
fn ping_answers_and_notifications_stay_silent() {
    let mut s = Server::start();
    s.initialize();

    // A notification must produce no reply at all; the ping that follows it
    // proves the stream is still aligned rather than off by one message.
    s.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }));
    assert_eq!(reply["id"], 2, "a notification must not consume an id");
    assert_eq!(reply["result"], json!({}));
}

#[test]
fn malformed_input_is_a_parse_error_and_does_not_kill_the_server() {
    let mut s = Server::start();
    s.initialize();

    writeln!(s.stdin, "this is not json").expect("write");
    s.stdin.flush().expect("flush");
    let reply = s.read();
    assert_eq!(reply["error"]["code"], -32700);

    // Still alive and still framed correctly.
    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 9, "method": "ping" }));
    assert_eq!(reply["id"], 9);
}

#[test]
fn stdout_carries_nothing_but_protocol_messages() {
    // The fixture's `noisy` tool calls writeOutput, which in an ordinary
    // request would go to the response body. On stdio it must not reach
    // stdout: a single stray byte desynchronises every client.
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "noisy", "arguments": {} }
    }));
    assert_eq!(reply["id"], 2, "writeOutput must not appear on stdout");
    assert_eq!(reply["result"]["isError"], false);

    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }));
    assert_eq!(reply["id"], 3, "the stream is still aligned after a noisy tool");
}

#[test]
fn an_unknown_server_name_exits_nonzero_with_a_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("mcp")
        .arg("no_such_server")
        .arg(fixtures_dir())
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "errors belong on stderr, never on stdout");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no MCP server named"));
}

#[test]
fn capabilities_grow_to_match_what_the_server_declares() {
    let mut s = Server::start();
    let caps = s.initialize()["result"]["capabilities"].clone();
    // The fixture now declares resources and prompts as well as tools, so all
    // three must be advertised — a client will not call what is not offered.
    assert_eq!(caps["tools"]["listChanged"], true);
    assert_eq!(caps["resources"]["listChanged"], true);
    assert_eq!(caps["prompts"]["listChanged"], true);
}

#[test]
fn resources_list_omits_templates_which_have_their_own_endpoint() {
    let mut s = Server::start();
    s.initialize();

    let listed = s.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }));
    let uris: Vec<&str> = listed["result"]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .filter_map(|r| r["uri"].as_str())
        .collect();
    assert!(uris.contains(&"doc://readme"), "got {uris:?}");
    // A template is not a readable URI; listing it here makes clients try to
    // read `doc://pages/{slug}` literally.
    assert!(!uris.iter().any(|u| u.contains('{')), "a template leaked into resources/list");

    let templates =
        s.call(json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/templates/list" }));
    let t = &templates["result"]["resourceTemplates"][0];
    assert_eq!(t["uriTemplate"], "doc://pages/{slug}");
    assert_eq!(t["mimeType"], "text/plain");
    assert!(t.get("uri").is_none(), "a template has no concrete uri");
}

#[test]
fn reading_a_literal_resource_attaches_its_uri_and_mime_type() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "doc://readme" }
    }));
    let entry = &reply["result"]["contents"][0];
    assert_eq!(entry["uri"], "doc://readme");
    assert_eq!(entry["mimeType"], "text/markdown");
    assert!(entry["text"].as_str().expect("text").contains("Body text"));
}

#[test]
fn a_uri_template_binds_its_variables_as_arguments() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "doc://pages/intro" }
    }));
    let text = reply["result"]["contents"][0]["text"].as_str().expect("text");
    // `{slug}` is bound from the URI, and a declared `uri` parameter sees the
    // whole thing.
    assert_eq!(text, "page=intro uri=doc://pages/intro");

    // A template must not swallow a deeper path it was not written for.
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "resources/read",
        "params": { "uri": "doc://pages/deep/path" }
    }));
    assert_eq!(reply["error"]["code"], -32602);
}

#[test]
fn a_binary_resource_is_delivered_as_a_blob() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "doc://blob" }
    }));
    let entry = &reply["result"]["contents"][0];
    assert_eq!(entry["blob"], "Ynl0ZXM=", "base64 of \"bytes\"");
    assert!(entry.get("text").is_none(), "binary must not be delivered as text");
}

#[test]
fn an_unknown_resource_and_a_secured_one_are_both_refused() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "doc://nope" }
    }));
    assert_eq!(reply["error"]["code"], -32602);

    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "resources/read",
        "params": { "uri": "doc://secret" }
    }));
    assert_eq!(reply["error"]["code"], -32600, "the secured gate covers resources too");
    assert_eq!(reply["id"], 3, "an error must stay correlatable to its request");
    assert!(!reply["error"]["message"].as_str().unwrap().contains(".cfc"));
}

#[test]
fn prompts_list_describes_arguments_flatly_not_as_json_schema() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "prompts/list" }));
    let prompts = reply["result"]["prompts"].as_array().expect("prompts");
    let review = prompts.iter().find(|p| p["name"] == "code_review").expect("code_review");
    assert_eq!(review["description"], "Ask for a code review");
    // Prompt arguments are a flat list, NOT an inputSchema.
    assert!(review.get("inputSchema").is_none());
    let arg = &review["arguments"][0];
    assert_eq!(arg["name"], "language");
    assert_eq!(arg["description"], "The language being reviewed");
    assert_eq!(arg["required"], false, "it has a default");
}

#[test]
fn getting_a_prompt_binds_arguments_and_shapes_messages() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "prompts/get",
        "params": { "name": "code_review", "arguments": { "language": "Rust" } }
    }));
    let result = &reply["result"];
    assert_eq!(result["description"], "Ask for a code review");
    assert_eq!(result["messages"][0]["role"], "assistant");
    assert_eq!(result["messages"][0]["content"]["type"], "text");
    assert_eq!(result["messages"][0]["content"]["text"], "I review Rust code.");
    assert_eq!(result["messages"][1]["role"], "user");

    // The default applies when the client sends no arguments at all.
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "prompts/get",
        "params": { "name": "code_review" }
    }));
    assert_eq!(reply["result"]["messages"][0]["content"]["text"], "I review CFML code.");
}

#[test]
fn a_prompt_may_return_a_bare_string() {
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "prompts/get",
        "params": { "name": "one_liner" }
    }));
    let messages = reply["result"]["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"]["text"], "Summarise the repository");

    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "prompts/get", "params": { "name": "nope" }
    }));
    assert_eq!(reply["error"]["code"], -32602);
}

#[test]
fn completion_answers_with_no_suggestions_rather_than_an_error() {
    // A client's completion UI breaks on an error but handles an empty list,
    // and "no suggestions" is the honest answer until per-argument completion
    // is authorable.
    let mut s = Server::start();
    s.initialize();
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "completion/complete",
        "params": { "ref": { "type": "ref/prompt", "name": "code_review" },
                    "argument": { "name": "language", "value": "R" } }
    }));
    assert_eq!(reply["result"]["completion"]["values"], json!([]));
    assert_eq!(reply["result"]["completion"]["hasMore"], false);
}

#[test]
fn every_advertised_capability_has_its_required_methods() {
    // Found by the reference MCP Inspector, which calls `logging/setLevel`
    // immediately after the handshake because we advertise `logging`. A server
    // that declares a capability MUST serve the methods that come with it —
    // advertising one we did not implement broke the connection on its very
    // first exchange, before any tool could be called.
    let mut s = Server::start();
    let caps = s.initialize()["result"]["capabilities"].clone();

    if caps.get("logging").is_some() {
        let reply = s.call(json!({
            "jsonrpc": "2.0", "id": 2, "method": "logging/setLevel",
            "params": { "level": "warning" }
        }));
        assert!(
            reply.get("error").is_none(),
            "advertised `logging` but logging/setLevel failed: {reply}"
        );
    }
    // The level is required, and a malformed call is a parameter error rather
    // than a silently ignored one.
    let reply = s.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "logging/setLevel", "params": {}
    }));
    assert_eq!(reply["error"]["code"], -32602);

    // Still usable afterwards.
    let reply = s.call(json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }));
    assert_eq!(reply["id"], 4);
}
