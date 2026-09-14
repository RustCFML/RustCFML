//! JSON-RPC 2.0 framing and the MCP method vocabulary.
//!
//! Pure data: no tokio, no axum, no VM. Every transport (Streamable HTTP, the
//! deprecated HTTP+SSE pair, and stdio) decodes bytes into [`Incoming`] and
//! encodes an [`Outgoing`] back, so the wire rules live in exactly one place
//! and the three transports cannot drift.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The newest spec revision this engine implements.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// Revisions we will negotiate down to. Ordered newest-first; the first entry
/// the client also names wins.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-06-18", "2025-03-26", "2024-11-05"];

/// What to assume when an HTTP client sends no `MCP-Protocol-Version` header.
/// The spec pins this exact value for backwards compatibility — it is NOT
/// "the latest".
pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

// ── JSON-RPC error codes ──────────────────────────────────────────────────
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// A JSON-RPC id. `null` is legal on the wire for an error response to an
/// unparseable request, so it is a variant rather than an `Option`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    Num(i64),
    Str(String),
    Null,
}

impl RpcId {
    fn from_json(v: Option<&Value>) -> Self {
        match v {
            Some(Value::Number(n)) => n.as_i64().map(RpcId::Num).unwrap_or(RpcId::Null),
            Some(Value::String(s)) => RpcId::Str(s.clone()),
            _ => RpcId::Null,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            RpcId::Num(n) => json!(n),
            RpcId::Str(s) => json!(s),
            RpcId::Null => Value::Null,
        }
    }
}

/// One decoded inbound message. A request expects a response; a notification
/// never does; a response/error is the client answering a request *we* sent
/// (sampling, elicitation, roots).
#[derive(Clone, Debug)]
pub enum Incoming {
    Request { id: RpcId, method: String, params: Value },
    Notification { method: String, params: Value },
    Response { id: RpcId, result: Value },
    Error { id: RpcId, error: Value },
}

impl Incoming {
    pub fn is_request(&self) -> bool {
        matches!(self, Incoming::Request { .. })
    }

    /// True for the one method allowed to arrive without a session id.
    pub fn is_initialize(&self) -> bool {
        matches!(self, Incoming::Request { method, .. } if method == methods::INITIALIZE)
    }
}

/// One message we are about to write.
#[derive(Clone, Debug)]
pub enum Outgoing {
    Result { id: RpcId, result: Value },
    Error { id: RpcId, code: i64, message: String, data: Option<Value> },
    Notification { method: String, params: Value },
    Request { id: RpcId, method: String, params: Value },
}

impl Outgoing {
    pub fn error(id: RpcId, code: i64, message: impl Into<String>) -> Self {
        Outgoing::Error { id, code, message: message.into(), data: None }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Outgoing::Result { id, result } => {
                json!({ "jsonrpc": "2.0", "id": id.to_json(), "result": result })
            }
            Outgoing::Error { id, code, message, data } => {
                let mut err = Map::new();
                err.insert("code".into(), json!(code));
                err.insert("message".into(), json!(message));
                if let Some(d) = data {
                    err.insert("data".into(), d.clone());
                }
                json!({ "jsonrpc": "2.0", "id": id.to_json(), "error": Value::Object(err) })
            }
            Outgoing::Notification { method, params } => {
                json!({ "jsonrpc": "2.0", "method": method, "params": params })
            }
            Outgoing::Request { id, method, params } => {
                json!({ "jsonrpc": "2.0", "id": id.to_json(), "method": method, "params": params })
            }
        }
    }

    /// Compact single-line JSON. stdio framing REQUIRES no embedded newlines,
    /// and the SSE encoder relies on it too, so this is the only serializer
    /// any transport should use.
    pub fn to_line(&self) -> String {
        serde_json::to_string(&self.to_json()).unwrap_or_else(|_| {
            String::from(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialization failed"}}"#)
        })
    }
}

/// Decode a POST body / stdio line. A JSON-RPC *batch* (a top-level array) is
/// accepted for pre-2025-06-18 clients and flattened; an empty batch is a
/// protocol error, not an empty success.
pub fn decode(bytes: &[u8]) -> Result<Vec<Incoming>, (i64, String)> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| (PARSE_ERROR, format!("Parse error: {e}")))?;
    let items = match value {
        Value::Array(a) if a.is_empty() => {
            return Err((INVALID_REQUEST, "Empty JSON-RPC batch".into()))
        }
        Value::Array(a) => a,
        other => vec![other],
    };
    items.into_iter().map(decode_one).collect()
}

fn decode_one(value: Value) -> Result<Incoming, (i64, String)> {
    let Value::Object(obj) = value else {
        return Err((INVALID_REQUEST, "JSON-RPC message must be an object".into()));
    };
    // `jsonrpc: "2.0"` is mandatory. Being strict here is cheap and catches a
    // whole class of "why is nothing happening" client bugs early.
    match obj.get("jsonrpc").and_then(|v| v.as_str()) {
        Some("2.0") => {}
        _ => return Err((INVALID_REQUEST, "Missing or invalid \"jsonrpc\": \"2.0\"".into())),
    }
    let id_present = obj.get("id").is_some_and(|v| !v.is_null());
    let id = RpcId::from_json(obj.get("id"));
    let params = obj.get("params").cloned().unwrap_or(Value::Null);

    if let Some(method) = obj.get("method").and_then(|v| v.as_str()) {
        let method = method.to_string();
        return Ok(if id_present {
            Incoming::Request { id, method, params }
        } else {
            Incoming::Notification { method, params }
        });
    }
    if let Some(err) = obj.get("error") {
        return Ok(Incoming::Error { id, error: err.clone() });
    }
    if let Some(result) = obj.get("result") {
        return Ok(Incoming::Response { id, result: result.clone() });
    }
    Err((INVALID_REQUEST, "Message has neither \"method\", \"result\" nor \"error\"".into()))
}

/// Negotiate a protocol version against what the client asked for. An
/// unknown version is not fatal: the spec says the server answers with a
/// version it *does* support and the client decides whether to continue.
pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    match requested {
        Some(req) => SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == req)
            .copied()
            .unwrap_or(LATEST_PROTOCOL_VERSION),
        None => LATEST_PROTOCOL_VERSION,
    }
}

/// Is this a version we can speak at all? Used by the HTTP layer to answer
/// 400 on an unsupported `MCP-Protocol-Version` header.
pub fn version_supported(v: &str) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&v)
}

/// MCP method names. String constants rather than an enum so an unknown
/// method is data (answered with -32601) instead of a parse failure.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const INITIALIZED: &str = "notifications/initialized";
    pub const PING: &str = "ping";
    pub const CANCELLED: &str = "notifications/cancelled";

    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";

    pub const RESOURCES_LIST: &str = "resources/list";
    pub const RESOURCES_READ: &str = "resources/read";
    pub const RESOURCES_TEMPLATES_LIST: &str = "resources/templates/list";
    pub const RESOURCES_SUBSCRIBE: &str = "resources/subscribe";
    pub const RESOURCES_UNSUBSCRIBE: &str = "resources/unsubscribe";
    pub const RESOURCES_UPDATED: &str = "notifications/resources/updated";
    pub const RESOURCES_LIST_CHANGED: &str = "notifications/resources/list_changed";

    pub const PROMPTS_LIST: &str = "prompts/list";
    pub const PROMPTS_GET: &str = "prompts/get";
    pub const PROMPTS_LIST_CHANGED: &str = "notifications/prompts/list_changed";

    pub const COMPLETION_COMPLETE: &str = "completion/complete";
    pub const LOGGING_SET_LEVEL: &str = "logging/setLevel";
    pub const LOG_MESSAGE: &str = "notifications/message";
    pub const PROGRESS: &str = "notifications/progress";

    pub const SAMPLING_CREATE_MESSAGE: &str = "sampling/createMessage";
    pub const ELICITATION_CREATE: &str = "elicitation/create";
    pub const ROOTS_LIST: &str = "roots/list";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_request_notification_and_response() {
        let msgs = decode(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert!(matches!(msgs[0], Incoming::Request { .. }));

        let msgs = decode(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(matches!(msgs[0], Incoming::Notification { .. }));

        let msgs = decode(br#"{"jsonrpc":"2.0","id":"a","result":{}}"#).unwrap();
        assert!(matches!(msgs[0], Incoming::Response { .. }));
    }

    #[test]
    fn a_null_id_is_a_notification_not_a_request() {
        // A request with `"id": null` must not be answered — treating it as a
        // request would put an `id: null` response on the wire.
        let msgs = decode(br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
        assert!(matches!(msgs[0], Incoming::Notification { .. }));
    }

    #[test]
    fn batches_flatten_and_empty_batch_is_invalid() {
        let msgs =
            decode(br#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"x"}]"#)
                .unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(decode(b"[]").unwrap_err().0, INVALID_REQUEST);
    }

    #[test]
    fn rejects_missing_jsonrpc_version() {
        assert_eq!(decode(br#"{"id":1,"method":"ping"}"#).unwrap_err().0, INVALID_REQUEST);
        assert_eq!(decode(b"not json").unwrap_err().0, PARSE_ERROR);
    }

    #[test]
    fn outgoing_is_always_one_line() {
        let out = Outgoing::Result {
            id: RpcId::Num(1),
            result: json!({ "text": "a\nb" }),
        };
        assert!(!out.to_line().contains('\n'), "stdio framing requires single-line JSON");
    }

    #[test]
    fn version_negotiation_falls_back_to_latest() {
        assert_eq!(negotiate_version(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate_version(Some("1999-01-01")), LATEST_PROTOCOL_VERSION);
        assert_eq!(negotiate_version(None), LATEST_PROTOCOL_VERSION);
        assert!(version_supported(DEFAULT_PROTOCOL_VERSION));
    }
}
