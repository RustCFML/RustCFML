//! CFML values ⇄ MCP content blocks.
//!
//! A tool handler returns whatever is natural in CFML — a string, a struct, an
//! array of structs, a query — and this module turns it into the `content` /
//! `structuredContent` pair the protocol wants, without the author having to
//! know the wire shape. Explicit control is still available through the
//! `mcpText()` / `mcpImage()` / `mcpAudio()` / `mcpResource()` /
//! `mcpResourceLink()` BIFs, which build already-shaped blocks that pass
//! through here untouched.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use serde_json::{json, Map, Value};

/// Content-block `type` values we recognise as "already shaped" on the way out.
const BLOCK_TYPES: &[&str] =
    &["text", "image", "audio", "resource", "resource_link"];

/// Convert a CFML value to JSON for the wire.
///
/// Deliberately independent of `serializeJSON`: MCP payloads are schema-shaped
/// JSON, so a query becomes an array of row objects (not Lucee's
/// `{COLUMNS,DATA}` envelope, which no MCP client understands), and binary
/// becomes base64 rather than an array of byte integers.
pub fn to_json(value: &CfmlValue) -> Value {
    match value {
        CfmlValue::Null => Value::Null,
        CfmlValue::Bool(b) => json!(b),
        CfmlValue::Int(i) => json!(i),
        CfmlValue::Double(d) | CfmlValue::TimeSpan(d) => {
            // JSON has no NaN/Infinity; degrade to null rather than emitting
            // invalid JSON that fails at the client's parser.
            if d.is_finite() {
                json!(d)
            } else {
                Value::Null
            }
        }
        CfmlValue::String(s) => json!(s.as_str()),
        CfmlValue::Binary(bytes) => json!(base64(bytes)),
        CfmlValue::Array(arr) => Value::Array(arr.iter().map(|v| to_json(&v)).collect()),
        CfmlValue::QueryColumn(vals, _) => Value::Array(vals.iter().map(to_json).collect()),
        CfmlValue::Struct(s) => {
            let mut map = Map::new();
            for (k, v) in s.iter() {
                // `__variables`, `__funcmeta_*` and friends are engine
                // bookkeeping — never protocol payload.
                if k.starts_with("__") {
                    continue;
                }
                map.insert(k.as_str().to_string(), to_json(&v));
            }
            Value::Object(map)
        }
        CfmlValue::Query(q) => {
            let data = q.backing();
            let data = data.read();
            let cols: Vec<String> = data.columns.clone();
            let mut rows = Vec::with_capacity(data.row_count());
            for r in 0..data.row_count() {
                let mut obj = Map::new();
                for (c, name) in cols.iter().enumerate() {
                    let cell = data.data.get(c).and_then(|col| col.get(r)).cloned();
                    obj.insert(name.clone(), to_json(&cell.unwrap_or(CfmlValue::Null)));
                }
                rows.push(Value::Object(obj));
            }
            Value::Array(rows)
        }
        // Functions, components and native objects have no wire form. Their
        // string rendering is more useful to a model than a hard error.
        other => json!(other.as_string()),
    }
}

/// A tool's return value rendered as a `CallToolResult`.
///
/// - a string (or number/boolean/date) → one `text` block;
/// - an already-shaped content block, or an array of them → passed through;
/// - a full result struct (`{ content: [...] }`) → passed through, so an author
///   who wants total control can build one;
/// - anything else (struct, array, query) → `structuredContent`, **plus** the
///   serialized JSON as a `text` block, which the spec requires for clients
///   that do not read structured content.
pub fn tool_result(value: &CfmlValue) -> Value {
    // Author built the whole result themselves.
    if let CfmlValue::Struct(s) = value {
        if s.get_ci("content").is_some_and(|c| matches!(c, CfmlValue::Array(_))) {
            return to_json(value);
        }
    }
    if let Some(blocks) = as_blocks(value) {
        return json!({ "content": blocks, "isError": false });
    }
    match value {
        CfmlValue::Null => json!({ "content": [], "isError": false }),
        CfmlValue::String(s) => json!({ "content": [text_block(s)], "isError": false }),
        CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::TimeSpan(_) => {
            json!({ "content": [text_block(&value.as_string())], "isError": false })
        }
        CfmlValue::Binary(bytes) => json!({
            "content": [ { "type": "resource", "resource": {
                "uri": "data:application/octet-stream;base64",
                "mimeType": "application/octet-stream",
                "blob": base64(bytes),
            } } ],
            "isError": false,
        }),
        other => {
            let structured = to_json(other);
            let text = serde_json::to_string(&structured).unwrap_or_default();
            json!({
                "content": [ text_block(&text) ],
                "structuredContent": structured,
                "isError": false,
            })
        }
    }
}

/// Render a thrown CFML exception as a tool *execution* error — `isError: true`
/// inside a successful JSON-RPC result, not a JSON-RPC error. The distinction
/// matters: a protocol error tells the model the call was malformed, while
/// `isError` hands it the failure text so it can adapt and retry.
pub fn tool_error(message: &str) -> Value {
    json!({ "content": [ text_block(message) ], "isError": true })
}

/// A resource handler's return value rendered as a `ReadResourceResult`.
///
/// The handler is spared the envelope: return the text and the engine attaches
/// the URI and mime type it already knows from the annotation. Binary becomes
/// a base64 `blob` rather than `text`, which is the distinction the protocol
/// draws and a client will reject if you get it wrong.
pub fn resource_result(uri: &str, mime_type: Option<&str>, value: &CfmlValue) -> Value {
    // An author who wants several contents (or exact control) can return them.
    if let CfmlValue::Struct(s) = value {
        if s.get_ci("contents").is_some_and(|c| matches!(c, CfmlValue::Array(_))) {
            return to_json(value);
        }
    }
    let mut entry = Map::new();
    entry.insert("uri".into(), json!(uri));
    match value {
        CfmlValue::Binary(bytes) => {
            entry.insert("blob".into(), json!(base64(bytes)));
            entry.insert(
                "mimeType".into(),
                json!(mime_type.unwrap_or("application/octet-stream")),
            );
        }
        CfmlValue::String(text) => {
            entry.insert("text".into(), json!(text.as_str()));
            entry.insert("mimeType".into(), json!(mime_type.unwrap_or("text/plain")));
        }
        // Structured data is serialized, and defaults to JSON rather than
        // "text/plain" so a client knows it can parse it.
        other => {
            let structured = to_json(other);
            entry.insert(
                "text".into(),
                json!(serde_json::to_string(&structured).unwrap_or_default()),
            );
            entry.insert("mimeType".into(), json!(mime_type.unwrap_or("application/json")));
        }
    }
    json!({ "contents": [ Value::Object(entry) ] })
}

/// A prompt handler's return value rendered as a `GetPromptResult`.
///
/// Accepts the three shapes an author naturally reaches for: a bare string
/// (one user message), an array of `{ role, content }` structs, or a full
/// result struct with its own `messages` key.
pub fn prompt_result(description: Option<&str>, value: &CfmlValue) -> Value {
    if let CfmlValue::Struct(s) = value {
        if s.get_ci("messages").is_some_and(|m| matches!(m, CfmlValue::Array(_))) {
            return to_json(value);
        }
    }
    let messages: Vec<Value> = match value {
        CfmlValue::Array(arr) => arr.iter().map(|m| prompt_message(&m)).collect(),
        CfmlValue::Null => Vec::new(),
        single => vec![prompt_message(single)],
    };
    let mut out = Map::new();
    if let Some(d) = description {
        out.insert("description".into(), json!(d));
    }
    out.insert("messages".into(), Value::Array(messages));
    Value::Object(out)
}

/// One conversation message (`{ role, content }`), used by both prompts and
/// sampling. `content` may be a plain string — the common case — and is
/// wrapped into a text block, because the protocol requires a content object
/// and an author should not have to remember that.
pub fn prompt_message(value: &CfmlValue) -> Value {
    let CfmlValue::Struct(s) = value else {
        return json!({ "role": "user", "content": text_block(&value.as_string()) });
    };
    let role = s
        .get_ci("role")
        .map(|r| r.as_string().to_lowercase())
        .filter(|r| r == "assistant")
        .unwrap_or_else(|| "user".to_string());
    let content = match s.get_ci("content") {
        Some(c) if is_block(&c) => to_json(&c),
        Some(CfmlValue::Null) | None => text_block(""),
        Some(other) => text_block(&other.as_string()),
    };
    json!({ "role": role, "content": content })
}

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

/// Recognise an already-shaped content block, or an array of them. Returns
/// `None` for anything that is ordinary data, so the caller falls back to the
/// structured-content path.
fn as_blocks(value: &CfmlValue) -> Option<Vec<Value>> {
    match value {
        CfmlValue::Struct(_) => is_block(value).then(|| vec![to_json(value)]),
        CfmlValue::Array(arr) => {
            // Empty arrays are data (an empty result set), not an empty block
            // list — otherwise a tool returning no rows looks like it returned
            // no content at all.
            if arr.is_empty() {
                return None;
            }
            let items: Vec<CfmlValue> = arr.iter().collect();
            items
                .iter()
                .all(is_block)
                .then(|| items.iter().map(to_json).collect())
        }
        _ => None,
    }
}

fn is_block(value: &CfmlValue) -> bool {
    let CfmlValue::Struct(s) = value else { return false };
    s.get_ci("type")
        .map(|t| {
            let t = t.as_string().to_lowercase();
            BLOCK_TYPES.contains(&t.as_str())
        })
        .unwrap_or(false)
}

/// Build a content block from the `mcpText`/`mcpImage`/… BIFs.
pub fn block(kind: &str, fields: ValueMap) -> CfmlValue {
    let mut map = ValueMap::default();
    map.insert("type".to_string(), CfmlValue::string(kind.to_string()));
    for (k, v) in fields.iter() {
        map.insert(k.clone(), v.clone());
    }
    CfmlValue::strukt(map)
}

/// Minimal standard base64 (RFC 4648, padded). Vendored rather than pulling a
/// dependency into `cfml-vm`, which must keep building for `wasm32`.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strukt(pairs: &[(&str, CfmlValue)]) -> CfmlValue {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        CfmlValue::strukt(m)
    }

    #[test]
    fn string_becomes_one_text_block() {
        let r = tool_result(&CfmlValue::string("hi"));
        assert_eq!(r["content"][0]["type"], "text");
        assert_eq!(r["content"][0]["text"], "hi");
        assert!(r.get("structuredContent").is_none());
    }

    #[test]
    fn struct_becomes_structured_plus_serialized_text() {
        let r = tool_result(&strukt(&[("ok", CfmlValue::Bool(true))]));
        assert_eq!(r["structuredContent"]["ok"], true);
        // The spec's backwards-compat rule: the JSON also rides as text.
        assert_eq!(r["content"][0]["text"], r#"{"ok":true}"#);
    }

    #[test]
    fn shaped_blocks_pass_through() {
        let b = strukt(&[("type", CfmlValue::string("text")),
                         ("text", CfmlValue::string("x"))]);
        let r = tool_result(&b);
        assert_eq!(r["content"][0]["text"], "x");
        assert!(r.get("structuredContent").is_none(), "a block is not structured data");
    }

    #[test]
    fn empty_array_is_data_not_an_empty_block_list() {
        let r = tool_result(&CfmlValue::array(vec![]));
        assert_eq!(r["structuredContent"], json!([]));
        assert_eq!(r["content"][0]["text"], "[]");
    }

    #[test]
    fn engine_bookkeeping_keys_never_reach_the_wire() {
        let v = strukt(&[
            ("__variables", CfmlValue::string("secret")),
            ("name", CfmlValue::string("ok")),
        ]);
        let j = to_json(&v);
        assert!(j.get("__variables").is_none());
        assert_eq!(j["name"], "ok");
    }

    #[test]
    fn non_finite_doubles_degrade_to_null() {
        assert_eq!(to_json(&CfmlValue::Double(f64::NAN)), Value::Null);
        assert_eq!(to_json(&CfmlValue::Double(1.5)), json!(1.5));
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_resource_string_gets_its_uri_and_mime_type_attached() {
        let r = resource_result("doc://intro", Some("text/markdown"), &CfmlValue::string("# Hi"));
        assert_eq!(r["contents"][0]["uri"], "doc://intro");
        assert_eq!(r["contents"][0]["text"], "# Hi");
        assert_eq!(r["contents"][0]["mimeType"], "text/markdown");
    }

    #[test]
    fn binary_resources_use_blob_not_text() {
        // A client rejects base64 delivered in `text`, and vice versa.
        let r = resource_result("doc://x", None, &CfmlValue::Binary(b"foo".to_vec()));
        assert_eq!(r["contents"][0]["blob"], "Zm9v");
        assert!(r["contents"][0].get("text").is_none());
        assert_eq!(r["contents"][0]["mimeType"], "application/octet-stream");
    }

    #[test]
    fn a_structured_resource_is_serialized_and_typed_as_json() {
        let r = resource_result("doc://x", None, &strukt(&[("a", CfmlValue::Int(1))]));
        assert_eq!(r["contents"][0]["text"], r#"{"a":1}"#);
        assert_eq!(r["contents"][0]["mimeType"], "application/json");
    }

    #[test]
    fn a_prompt_string_becomes_one_user_message() {
        let r = prompt_result(Some("Review"), &CfmlValue::string("Look at this"));
        assert_eq!(r["description"], "Review");
        assert_eq!(r["messages"][0]["role"], "user");
        assert_eq!(r["messages"][0]["content"]["type"], "text");
        assert_eq!(r["messages"][0]["content"]["text"], "Look at this");
    }

    #[test]
    fn prompt_messages_accept_plain_string_content_and_default_to_user() {
        let msgs = CfmlValue::array(vec![
            strukt(&[("role", CfmlValue::string("assistant")),
                     ("content", CfmlValue::string("I will help"))]),
            strukt(&[("content", CfmlValue::string("no role given"))]),
        ]);
        let r = prompt_result(None, &msgs);
        assert_eq!(r["messages"][0]["role"], "assistant");
        assert_eq!(r["messages"][0]["content"]["text"], "I will help");
        assert_eq!(r["messages"][1]["role"], "user", "an unknown role falls back to user");
        assert!(r.get("description").is_none());
    }

    #[test]
    fn an_author_can_return_the_whole_envelope() {
        let full = strukt(&[("messages", CfmlValue::array(vec![]))]);
        assert_eq!(prompt_result(None, &full)["messages"], json!([]));
        let full = strukt(&[("contents", CfmlValue::array(vec![]))]);
        assert_eq!(resource_result("doc://x", None, &full)["contents"], json!([]));
    }

    #[test]
    fn errors_are_results_not_protocol_errors() {
        let r = tool_error("boom");
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["text"], "boom");
    }
}
