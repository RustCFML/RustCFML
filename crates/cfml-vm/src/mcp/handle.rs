//! The live `mcp()` handle handed to a running tool.
//!
//! Mirrors the WebSocket `io()` emitter: a `CfmlValue::NativeObject` that only
//! ever pushes onto the registry, so it needs no `&mut VM` (which a
//! `CfmlNative` method cannot have — see `async_kernel.rs`). Everything a tool
//! can say back to its client mid-call goes through here.

use std::sync::Arc;

use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use serde_json::json;

use super::content;
use super::protocol::{methods, Outgoing};
use super::registry::McpRegistry;
use super::CallContext;

/// How long a server→client request waits before giving up.
///
/// There MUST be a bound. A parked request holds a thread from the blocking
/// pool that every ordinary CFML request also draws on, so a client that never
/// answers would otherwise degrade the whole server, not just this call.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// How many server→client requests one session may have outstanding. Same
/// reasoning: this is the cap that stops a misbehaving client from pinning the
/// blocking pool by opening calls and never responding.
const MAX_PENDING_PER_SESSION: usize = 4;

/// The object `mcp()` returns.
#[derive(Debug)]
pub struct McpHandle {
    ctx: CallContext,
    registry: Arc<McpRegistry>,
}

impl McpHandle {
    pub fn new(ctx: CallContext, registry: Arc<McpRegistry>) -> Self {
        Self { ctx, registry }
    }

    /// Send one server-initiated message on this call's own stream, falling
    /// back to the session's standalone stream. Returns whether it was
    /// delivered — a call with no open stream has nobody to tell, which is a
    /// fact the caller may want rather than a silent nothing.
    fn emit(&self, msg: Outgoing) -> bool {
        match self.ctx.stream {
            Some(ord) => self.registry.send_on(&self.ctx.session, ord, &msg),
            None => self.registry.notify_session(&self.ctx.session, &msg),
        }
    }

    /// `notifications/progress`.
    ///
    /// Only legal when the client attached a `_meta.progressToken` to the
    /// request: the spec forbids unsolicited progress, so without a token this
    /// reports `false` rather than putting an illegal frame on the wire.
    fn progress(&self, args: &[CfmlValue]) -> CfmlResult {
        let Some(token) = self.ctx.progress_token.clone() else {
            return Ok(CfmlValue::Bool(false));
        };
        let mut params = json!({
            "progressToken": token,
            "progress": arg_json(args, 0),
        });
        if let Some(total) = args.get(1).filter(|v| !matches!(v, CfmlValue::Null)) {
            params["total"] = content::to_json(total);
        }
        if let Some(message) = args.get(2).filter(|v| !matches!(v, CfmlValue::Null)) {
            params["message"] = json!(message.as_string());
        }
        Ok(CfmlValue::Bool(self.emit(Outgoing::Notification {
            method: methods::PROGRESS.to_string(),
            params,
        })))
    }

    /// `notifications/message` — structured logging to the client's console.
    fn log(&self, args: &[CfmlValue]) -> CfmlResult {
        let level = args
            .first()
            .map(|v| v.as_string().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "info".to_string());
        let mut params = json!({
            "level": level,
            "data": args.get(1).map(content::to_json).unwrap_or(serde_json::Value::Null),
        });
        if let Some(logger) = args.get(2).filter(|v| !matches!(v, CfmlValue::Null)) {
            params["logger"] = json!(logger.as_string());
        }
        Ok(CfmlValue::Bool(self.emit(Outgoing::Notification {
            method: methods::LOG_MESSAGE.to_string(),
            params,
        })))
    }
}

impl McpHandle {
    /// Ask the client something and block until it answers.
    ///
    /// Three guards, in order, because each one turns a hang into an error:
    /// the client must have declared the capability; there must be a stream to
    /// write the request onto; and the wait is always bounded.
    fn request(
        &self,
        method: &str,
        capable: bool,
        capability: &str,
        params: serde_json::Value,
        timeout_ms: u64,
    ) -> CfmlResult {
        if !capable {
            return Err(CfmlError::runtime(format!(
                "The MCP client does not support {capability}, so mcp().{} cannot be used",
                short_name(method)
            )));
        }
        let Some(ord) = self.ctx.stream else {
            // Without a stream the request has nowhere to go and the answer
            // nowhere to come back to. Failing loudly here is far better than
            // parking for the timeout: the fix is an annotation on the handler.
            return Err(CfmlError::runtime(format!(
                "mcp().{} needs a streaming call — annotate the handler `streaming=true`",
                short_name(method)
            )));
        };

        let id = self.registry.new_request_id();
        let slot = self
            .registry
            .register_pending(&self.ctx.session, id.clone(), MAX_PENDING_PER_SESSION)
            .map_err(CfmlError::runtime)?;

        let sent = self.registry.send_on(
            &self.ctx.session,
            ord,
            &Outgoing::Request { id: id.clone(), method: method.to_string(), params },
        );
        if !sent {
            self.registry.forget_pending(&self.ctx.session, &id);
            return Err(CfmlError::runtime(
                "The MCP client disconnected before the request could be sent".to_string(),
            ));
        }

        match slot.wait(timeout_ms) {
            Ok(value) => Ok(crate::json_value_to_cfml(value)),
            Err(e) => {
                self.registry.forget_pending(&self.ctx.session, &id);
                Err(CfmlError::runtime(e))
            }
        }
    }

    /// `sampling/createMessage` — ask the client's model to generate something.
    fn sample(&self, args: &[CfmlValue]) -> CfmlResult {
        let messages: Vec<serde_json::Value> = match args.first() {
            Some(CfmlValue::Array(arr)) => {
                arr.iter().map(|m| content::prompt_message(&m)).collect()
            }
            Some(CfmlValue::Null) | None => {
                return Err(CfmlError::runtime(
                    "mcp().sample() needs at least one message".to_string(),
                ))
            }
            Some(single) => vec![content::prompt_message(single)],
        };
        let options = args.get(1).and_then(|v| v.as_cfml_struct());
        let opt = |key: &str| options.as_ref().and_then(|o| o.get_ci(key));

        let mut params = json!({
            "messages": messages,
            // The spec requires maxTokens; defaulting it keeps the common call
            // to one argument instead of making every author remember it.
            "maxTokens": opt("maxTokens").map(|v| content::to_json(&v)).unwrap_or(json!(1000)),
        });
        for (cfml_key, wire_key) in [
            ("systemPrompt", "systemPrompt"),
            ("temperature", "temperature"),
            ("stopSequences", "stopSequences"),
            ("modelPreferences", "modelPreferences"),
            ("includeContext", "includeContext"),
        ] {
            if let Some(v) = opt(cfml_key).filter(|v| !matches!(v, CfmlValue::Null)) {
                params[wire_key] = content::to_json(&v);
            }
        }
        self.request(
            methods::SAMPLING_CREATE_MESSAGE,
            self.ctx.capabilities.sampling,
            "sampling",
            params,
            timeout_from(opt("timeout")),
        )
    }

    /// `elicitation/create` — ask the *user*, through the client, for input.
    fn elicit(&self, args: &[CfmlValue]) -> CfmlResult {
        let message = args.first().map(|v| v.as_string()).unwrap_or_default();
        if message.is_empty() {
            return Err(CfmlError::runtime(
                "mcp().elicit() needs a message to show the user".to_string(),
            ));
        }
        // The schema describes the shape being asked for; an absent one means
        // "any object", which is a valid schema rather than a missing key.
        let schema = args
            .get(1)
            .filter(|v| !matches!(v, CfmlValue::Null))
            .map(content::to_json)
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        self.request(
            methods::ELICITATION_CREATE,
            self.ctx.capabilities.elicitation,
            "elicitation",
            json!({ "message": message, "requestedSchema": schema }),
            timeout_from(args.get(2).cloned()),
        )
    }

    /// `roots/list` — what the client has exposed to this server.
    fn roots(&self, args: &[CfmlValue]) -> CfmlResult {
        self.request(
            methods::ROOTS_LIST,
            self.ctx.capabilities.roots,
            "roots",
            json!({}),
            timeout_from(args.first().cloned()),
        )
    }
}

/// A timeout given in seconds (CFML's unit for these), clamped to something
/// sane: zero or negative would mean "give up immediately".
fn timeout_from(value: Option<CfmlValue>) -> u64 {
    match value {
        Some(v) if !matches!(v, CfmlValue::Null) => {
            let secs = v.as_string().parse::<f64>().unwrap_or(0.0);
            if secs > 0.0 {
                (secs * 1000.0) as u64
            } else {
                DEFAULT_TIMEOUT_MS
            }
        }
        _ => DEFAULT_TIMEOUT_MS,
    }
}

/// `sampling/createMessage` → `sample`, for error messages that name the CFML
/// method rather than the wire method.
fn short_name(method: &str) -> &'static str {
    match method {
        methods::SAMPLING_CREATE_MESSAGE => "sample()",
        methods::ELICITATION_CREATE => "elicit()",
        methods::ROOTS_LIST => "roots()",
        _ => "request()",
    }
}

fn arg_json(args: &[CfmlValue], idx: usize) -> serde_json::Value {
    args.get(idx).map(content::to_json).unwrap_or(serde_json::Value::Null)
}

impl CfmlNative for McpHandle {
    fn class_name(&self) -> &str {
        "McpHandle"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_lowercase().as_str() {
            "progress" => self.progress(&args),
            "log" => self.log(&args),
            "sample" => self.sample(&args),
            "elicit" => self.elicit(&args),
            "roots" => self.roots(&args),
            "session" => Ok(CfmlValue::string(self.ctx.session.clone())),
            "server" => Ok(CfmlValue::string(self.ctx.server.clone())),
            "client" => Ok(client_struct(&self.ctx, &self.registry)),
            "streaming" => Ok(CfmlValue::Bool(self.ctx.stream.is_some())),
            // `notifications/tools/list_changed` — tell the client to re-read
            // the tool list, e.g. after an app reload changed what is exposed.
            "toolschanged" => Ok(CfmlValue::Bool(self.emit(Outgoing::Notification {
                method: methods::TOOLS_LIST_CHANGED.to_string(),
                params: json!({}),
            }))),
            "notify" => {
                let method = args.first().map(|v| v.as_string()).unwrap_or_default();
                if method.is_empty() {
                    return Err(CfmlError::runtime(
                        "mcp().notify() needs a method name".to_string(),
                    ));
                }
                Ok(CfmlValue::Bool(self.emit(Outgoing::Notification {
                    method,
                    params: arg_json(&args, 1),
                })))
            }
            other => Err(CfmlError::runtime(format!(
                "Unknown method [{other}] on the mcp() handle"
            ))),
        }
    }

    /// Declared so these can be called with named arguments. Without it the
    /// host refuses a named call rather than binding values positionally and
    /// corrupting them silently.
    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_lowercase().as_str() {
            "progress" => &["progress", "total", "message"],
            "log" => &["level", "data", "logger"],
            "sample" => &["messages", "options"],
            "elicit" => &["message", "schema", "timeout"],
            "roots" => &["timeout"],
            "notify" => &["method", "params"],
            _ => return None,
        })
    }

    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        match name.to_lowercase().as_str() {
            "session" => Some(CfmlValue::string(self.ctx.session.clone())),
            "server" => Some(CfmlValue::string(self.ctx.server.clone())),
            _ => None,
        }
    }
}

/// What the client told us about itself at `initialize`, plus the negotiated
/// protocol version — enough for a tool to adapt its output to the caller.
fn client_struct(ctx: &CallContext, registry: &Arc<McpRegistry>) -> CfmlValue {
    let mut map = ValueMap::default();
    if let Some(session) = registry.get(&ctx.session) {
        let session = session.lock();
        map.insert(
            "protocolVersion".to_string(),
            CfmlValue::string(session.protocol_version.clone()),
        );
        map.insert("info".to_string(), crate::json_value_to_cfml(session.client_info.clone()));
    }
    let mut caps = ValueMap::default();
    caps.insert("sampling".to_string(), CfmlValue::Bool(ctx.capabilities.sampling));
    caps.insert("elicitation".to_string(), CfmlValue::Bool(ctx.capabilities.elicitation));
    caps.insert("roots".to_string(), CfmlValue::Bool(ctx.capabilities.roots));
    map.insert("capabilities".to_string(), CfmlValue::strukt(caps));
    CfmlValue::strukt(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::{ClientCapabilities, McpSink, StreamKind};
    use parking_lot::Mutex;

    #[derive(Debug, Default)]
    struct Capture(Mutex<Vec<String>>);

    impl McpSink for Capture {
        fn send(&self, _event_id: &str, payload: &str) -> bool {
            self.0.lock().push(payload.to_string());
            true
        }
        fn close(&self) {}
    }

    fn setup(progress_token: Option<serde_json::Value>) -> (McpHandle, Arc<Capture>) {
        let registry = Arc::new(McpRegistry::new("node1"));
        let session = registry.create_session(
            "/mcp/demo",
            "2025-06-18",
            json!({ "name": "test-client" }),
            ClientCapabilities { sampling: true, ..Default::default() },
            0,
        );
        let sink = Arc::new(Capture::default());
        let ord = registry
            .attach_stream(&session, StreamKind::Standalone, sink.clone(), 16)
            .expect("stream");
        let ctx = CallContext {
            server: "/mcp/demo".into(),
            session,
            stream: Some(ord),
            progress_token,
            capabilities: ClientCapabilities { sampling: true, ..Default::default() },
        };
        (McpHandle::new(ctx, registry), sink)
    }

    #[test]
    fn progress_requires_a_token_from_the_client() {
        // Unsolicited progress notifications are a spec violation, so without a
        // token nothing goes on the wire and the caller is told so.
        let (mut handle, sink) = setup(None);
        let sent = handle.call_method("progress", vec![CfmlValue::Double(0.5)]).unwrap();
        assert!(matches!(sent, CfmlValue::Bool(false)));
        assert!(sink.0.lock().is_empty());
    }

    #[test]
    fn progress_carries_the_token_total_and_message() {
        let (mut handle, sink) = setup(Some(json!("tok-1")));
        let sent = handle
            .call_method(
                "progress",
                vec![
                    CfmlValue::Double(0.5),
                    CfmlValue::Int(10),
                    CfmlValue::string("halfway"),
                ],
            )
            .unwrap();
        assert!(matches!(sent, CfmlValue::Bool(true)));
        let frames = sink.0.lock();
        let v: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(v["method"], "notifications/progress");
        assert_eq!(v["params"]["progressToken"], "tok-1");
        assert_eq!(v["params"]["progress"], 0.5);
        assert_eq!(v["params"]["total"], 10);
        assert_eq!(v["params"]["message"], "halfway");
    }

    #[test]
    fn log_defaults_to_info_and_carries_structured_data() {
        let (mut handle, sink) = setup(None);
        handle.call_method("log", vec![]).unwrap();
        handle
            .call_method(
                "log",
                vec![CfmlValue::string("warning"), CfmlValue::string("careful")],
            )
            .unwrap();
        let frames = sink.0.lock();
        let first: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(first["method"], "notifications/message");
        assert_eq!(first["params"]["level"], "info");
        let second: serde_json::Value = serde_json::from_str(&frames[1]).unwrap();
        assert_eq!(second["params"]["level"], "warning");
        assert_eq!(second["params"]["data"], "careful");
    }

    #[test]
    fn the_handle_exposes_session_server_and_client_details() {
        let (mut handle, _) = setup(None);
        assert_eq!(handle.call_method("server", vec![]).unwrap().as_string(), "/mcp/demo");
        assert!(!handle.call_method("session", vec![]).unwrap().as_string().is_empty());
        assert!(matches!(handle.call_method("streaming", vec![]).unwrap(), CfmlValue::Bool(true)));

        let client = handle.call_method("client", vec![]).unwrap();
        let CfmlValue::Struct(s) = client else { panic!("expected a struct") };
        assert_eq!(s.get_ci("protocolVersion").unwrap().as_string(), "2025-06-18");
        let caps = s.get_ci("capabilities").expect("capabilities");
        let caps = caps.as_cfml_struct().expect("capabilities struct");
        assert!(matches!(caps.get_ci("sampling"), Some(CfmlValue::Bool(true))));
    }

    /// A handle whose client declared every capability, plus the registry and
    /// session so a test can answer the request it issues.
    fn sampling_setup() -> (McpHandle, Arc<McpRegistry>, String, u64, Arc<Capture>) {
        let registry = Arc::new(McpRegistry::new("node1"));
        let caps = ClientCapabilities { sampling: true, elicitation: true, roots: true };
        let session =
            registry.create_session("/mcp/demo", "2025-06-18", json!({}), caps.clone(), 0);
        let sink = Arc::new(Capture::default());
        let ord = registry
            .attach_stream(&session, StreamKind::Standalone, sink.clone(), 16)
            .expect("stream");
        let ctx = CallContext {
            server: "/mcp/demo".into(),
            session: session.clone(),
            stream: Some(ord),
            progress_token: None,
            capabilities: caps,
        };
        (McpHandle::new(ctx, registry.clone()), registry, session, ord, sink)
    }

    fn messages() -> CfmlValue {
        let mut m = ValueMap::default();
        m.insert("role".to_string(), CfmlValue::string("user"));
        m.insert("content".to_string(), CfmlValue::string("hello"));
        CfmlValue::array(vec![CfmlValue::strukt(m)])
    }

    #[test]
    fn sampling_writes_a_request_and_returns_the_clients_answer() {
        let (mut handle, registry, session, _, sink) = sampling_setup();

        // Answer from another thread, as the transport does when the client's
        // response arrives on a separate POST.
        let answering = registry.clone();
        let sid = session.clone();
        let responder = std::thread::spawn(move || {
            for _ in 0..200 {
                let frames = { sink.0.lock().clone() };
                if let Some(frame) = frames.first() {
                    let v: serde_json::Value = serde_json::from_str(frame).unwrap();
                    assert_eq!(v["method"], "sampling/createMessage");
                    assert_eq!(v["params"]["messages"][0]["content"]["text"], "hello");
                    assert_eq!(v["params"]["maxTokens"], 500);
                    let id: crate::mcp::RpcId =
                        serde_json::from_value(v["id"].clone()).unwrap();
                    answering.resolve_pending(
                        &sid,
                        &id,
                        Ok(json!({ "role": "assistant", "content": { "type": "text", "text": "hi back" } })),
                    );
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("no sampling request was written");
        });

        let mut options = ValueMap::default();
        options.insert("maxTokens".to_string(), CfmlValue::Int(500));
        let result = handle
            .call_method("sample", vec![messages(), CfmlValue::strukt(options)])
            .expect("sampling result");
        responder.join().expect("responder");

        let s = result.as_cfml_struct().expect("struct result");
        assert_eq!(s.get_ci("role").unwrap().as_string(), "assistant");
    }

    #[test]
    fn a_client_without_the_capability_fails_fast_instead_of_hanging() {
        // A client that declared no capabilities at all. Asking it to sample
        // must be an immediate error: parking for the full timeout would hold
        // a blocking-pool thread for a minute over something knowable up front.
        let registry = Arc::new(McpRegistry::new("node1"));
        let session = registry.create_session(
            "/mcp/demo",
            "2025-06-18",
            json!({}),
            ClientCapabilities::default(),
            0,
        );
        let ord = registry
            .attach_stream(&session, StreamKind::Standalone, Arc::new(Capture::default()), 16)
            .expect("stream");
        let ctx = CallContext {
            server: "/mcp/demo".into(),
            session,
            stream: Some(ord),
            progress_token: None,
            capabilities: ClientCapabilities::default(),
        };
        let mut handle = McpHandle::new(ctx, registry);

        let started = std::time::Instant::now();
        let err = handle.call_method("sample", vec![messages()]).unwrap_err();
        assert!(err.message.contains("does not support sampling"), "got {}", err.message);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "must not park at all");

        assert!(handle
            .call_method("elicit", vec![CfmlValue::string("hi")])
            .unwrap_err()
            .message
            .contains("does not support elicitation"));
        assert!(handle
            .call_method("roots", vec![])
            .unwrap_err()
            .message
            .contains("does not support roots"));
    }

    #[test]
    fn a_non_streaming_call_cannot_sample_and_says_why() {
        let registry = Arc::new(McpRegistry::new("node1"));
        let caps = ClientCapabilities { sampling: true, ..Default::default() };
        let session =
            registry.create_session("/mcp/demo", "2025-06-18", json!({}), caps.clone(), 0);
        let ctx = CallContext {
            server: "/mcp/demo".into(),
            session,
            stream: None, // answered as plain JSON — nowhere to put a request
            progress_token: None,
            capabilities: caps,
        };
        let mut handle = McpHandle::new(ctx, registry);
        let err = handle.call_method("sample", vec![messages()]).unwrap_err();
        assert!(err.message.contains("streaming=true"), "got {}", err.message);
    }

    #[test]
    fn a_client_that_never_answers_times_out_rather_than_pinning_the_thread() {
        let (mut handle, _, _, _, _) = sampling_setup();
        let mut options = ValueMap::default();
        // 0.05s — the wait is always bounded; a parked request holds a thread
        // from the pool every ordinary request also draws on.
        options.insert("timeout".to_string(), CfmlValue::Double(0.05));
        let started = std::time::Instant::now();
        let err = handle
            .call_method("sample", vec![messages(), CfmlValue::strukt(options)])
            .unwrap_err();
        assert!(err.message.contains("Timed out"), "got {}", err.message);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn outstanding_requests_are_capped_per_session() {
        let (handle, registry, session, _, _) = sampling_setup();
        for _ in 0..MAX_PENDING_PER_SESSION {
            registry
                .register_pending(&session, registry.new_request_id(), MAX_PENDING_PER_SESSION)
                .expect("under the cap");
        }
        // At the cap, the next request must be refused immediately rather than
        // parking another blocking-pool thread.
        let mut handle = handle;
        let mut options = ValueMap::default();
        options.insert("timeout".to_string(), CfmlValue::Double(5.0));
        let err = handle
            .call_method("sample", vec![messages(), CfmlValue::strukt(options)])
            .unwrap_err();
        assert!(err.message.contains("Too many outstanding"), "got {}", err.message);
    }

    #[test]
    fn elicitation_needs_a_message_and_defaults_its_schema() {
        let (mut handle, _, _, _, sink) = sampling_setup();
        assert!(handle.call_method("elicit", vec![]).is_err(), "no message to show");

        let mut options = ValueMap::default();
        let _ = options;
        std::thread::spawn({
            let sink = sink.clone();
            move || {
                for _ in 0..50 {
                    if !sink.0.lock().is_empty() {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });
        // Times out (nothing answers), but the request shape is what matters.
        let _ = handle.call_method(
            "elicit",
            vec![CfmlValue::string("Which environment?"), CfmlValue::Null, CfmlValue::Double(0.05)],
        );
        let frames = sink.0.lock();
        let v: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(v["method"], "elicitation/create");
        assert_eq!(v["params"]["message"], "Which environment?");
        assert_eq!(v["params"]["requestedSchema"]["type"], "object");
    }

    #[test]
    fn notify_needs_a_method_and_an_unknown_method_is_an_error() {
        let (mut handle, _) = setup(None);
        assert!(handle.call_method("notify", vec![]).is_err());
        assert!(handle.call_method("nonsense", vec![]).is_err());
        assert!(matches!(
            handle.call_method("notify", vec![CfmlValue::string("notifications/custom")]).unwrap(),
            CfmlValue::Bool(true)
        ));
    }
}
