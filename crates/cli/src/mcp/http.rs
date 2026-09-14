//! Streamable HTTP transport (MCP spec 2025-06-18).
//!
//! One endpoint — `/mcp/{name}` — answering `POST` (client→server messages),
//! `GET` (the server→client stream) and `DELETE` (session teardown).
//!
//! The awkward part of this transport is that the response mode for a `POST`
//! must be chosen **before** the handler runs, while whether a handler streams
//! is a property of the handler. It is therefore decided from what the client
//! asked for (`_meta.progressToken`) and what the tool declared (`streaming`),
//! both of which are known statically. See [`wants_stream`].

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use cfml_vm::mcp::protocol::{self, Incoming, Outgoing, RpcId};
use cfml_vm::mcp::{ClientCapabilities, ServerManifest, StreamKind};
use serde_json::Value;

use super::dispatch::McpRuntime;
use super::engine::{self, Ctx, Handled};
use super::sse;
use super::ServerInfo;
use crate::AppState;

/// Header carrying the session id, both ways.
const SESSION_HEADER: &str = "mcp-session-id";
/// Header carrying the negotiated protocol revision on every later request.
const VERSION_HEADER: &str = "mcp-protocol-version";

/// A session is reaped after this long without a request. Long enough that an
/// idle editor keeps its session, short enough that abandoned ones do not
/// accumulate.
const SESSION_IDLE_MS: u64 = 5 * 60 * 1000;

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Drop sessions nobody has touched, closing their streams and failing
/// anything parked on them. Modelled on the HTTP session reaper.
pub(crate) fn spawn_reaper(server_state: cfml_vm::ServerState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let reaped = server_state.mcp.reap(now_ms(), SESSION_IDLE_MS);
            if !reaped.is_empty() {
                log::debug!("MCP: reaped {} idle session(s)", reaped.len());
            }
        }
    });
}

/// `POST /mcp/{name}` — the client sending us JSON-RPC.
pub(crate) async fn post(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    addr: ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
) -> Response {
    let Some(info) = super::resolve_server(&state.doc_root, &state.vfs, &name) else {
        // No such MCP server: this path belongs to the application, not to us.
        return crate::handle_request_for_mcp(State(state), addr, req).await;
    };
    let (parts, body) = req.into_parts();
    let headers = parts.headers;

    if let Some(deny) = guard(&headers) {
        return deny;
    }
    let body = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                protocol::PARSE_ERROR,
                format!("Could not read request body: {e}"),
            )
        }
    };
    handle_post(state, info, headers, body).await
}

async fn handle_post(
    state: Arc<AppState>,
    info: ServerInfo,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let messages = match protocol::decode(&body) {
        Ok(m) => m,
        Err((code, message)) => return rpc_error(StatusCode::BAD_REQUEST, code, message),
    };
    let registry = state.server_state.mcp.clone();
    let session_header = header_str(&headers, SESSION_HEADER);
    let is_initialize = messages.iter().any(|m| m.is_initialize());

    // Session gate. `initialize` is the one request allowed in without a
    // session; everything else needs a live one. The 400/404 split matters:
    // clients read 404 as "re-initialize", and 400 as "you sent a bad request".
    if !is_initialize {
        match &session_header {
            None => {
                return rpc_error(
                    StatusCode::BAD_REQUEST,
                    protocol::INVALID_REQUEST,
                    "Missing Mcp-Session-Id header",
                )
            }
            Some(id) if !registry.touch(id, now_ms()) => {
                return rpc_error(
                    StatusCode::NOT_FOUND,
                    protocol::INVALID_REQUEST,
                    "Unknown or expired MCP session",
                )
            }
            Some(_) => {}
        }
    }

    // A client's answer to a server→client request is routed to the thread
    // parked on it, not to the engine.
    if let Some(session) = &session_header {
        for msg in &messages {
            match msg {
                Incoming::Response { id, result } => {
                    registry.resolve_pending(session, id, Ok(result.clone()));
                }
                Incoming::Error { id, error } => {
                    let text = error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("client reported an error")
                        .to_string();
                    registry.resolve_pending(session, id, Err(text));
                }
                _ => {}
            }
        }
    }

    let runtime = McpRuntime {
        server_state: state.server_state.clone(),
        vfs: state.vfs.clone(),
        sandbox: state.sandbox,
    };

    // The manifest is loaded once here when the batch calls a tool, both to
    // choose the response mode and to hand to the engine — otherwise the same
    // component would be compiled twice for one request.
    let manifest = if messages.iter().any(is_tool_call) {
        super::dispatch::manifest(runtime.clone(), info.clone()).await.ok()
    } else {
        None
    };

    let accepts_sse = header_str(&headers, "accept")
        .map(|a| a.contains("text/event-stream"))
        .unwrap_or(false);
    if accepts_sse && wants_stream(&messages, manifest.as_ref()) {
        return stream_response(state, info, runtime, manifest, session_header, messages);
    }

    let capabilities = session_capabilities(&registry, session_header.as_deref());
    let ctx = Ctx {
        runtime,
        info: info.clone(),
        session: session_header.clone(),
        identity: None,
        capabilities,
        stream: None,
        manifest,
    };

    let mut replies: Vec<Value> = Vec::new();
    let mut new_session: Option<String> = None;
    for msg in messages {
        let client_caps = initialize_capabilities(&msg);
        match engine::handle(&ctx, msg).await {
            Handled::Accepted => {}
            Handled::Reply(out) => replies.push(out.to_json()),
            Handled::Initialized { reply, negotiated } => {
                new_session = Some(registry.create_session(
                    &info.id,
                    &negotiated,
                    client_info_of(&reply),
                    client_caps.unwrap_or_default(),
                    now_ms(),
                ));
                replies.push(reply.to_json());
            }
        }
    }

    if replies.is_empty() {
        // Notifications and responses only: the spec pins 202 with no body.
        return StatusCode::ACCEPTED.into_response();
    }
    let payload =
        if replies.len() == 1 { replies.remove(0) } else { Value::Array(replies) };
    let mut response = (StatusCode::OK, axum::Json(payload)).into_response();
    if let Some(id) = new_session {
        if let Ok(v) = id.parse::<axum::http::HeaderValue>() {
            response.headers_mut().insert(SESSION_HEADER, v);
        }
    }
    response
}

/// Answer a POST with an SSE stream: the handler runs in the background and
/// its notifications *and* its eventual response travel on the stream.
fn stream_response(
    state: Arc<AppState>,
    info: ServerInfo,
    runtime: McpRuntime,
    manifest: Option<ServerManifest>,
    session: Option<String>,
    messages: Vec<Incoming>,
) -> Response {
    let registry = state.server_state.mcp.clone();
    let Some(session_id) = session else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            protocol::INVALID_REQUEST,
            "Missing Mcp-Session-Id header",
        );
    };
    let request_id = messages
        .iter()
        .find_map(|m| match m {
            Incoming::Request { id, .. } => Some(id.clone()),
            _ => None,
        })
        .unwrap_or(RpcId::Null);

    let stream = sse::Stream::new();
    let Some(ord) = registry.attach_stream(
        &session_id,
        StreamKind::RequestScoped { request: request_id },
        stream.sink.clone(),
        sse::REPLAY_CAP,
    ) else {
        return rpc_error(
            StatusCode::NOT_FOUND,
            protocol::INVALID_REQUEST,
            "Unknown or expired MCP session",
        );
    };

    let capabilities = session_capabilities(&registry, Some(&session_id));
    let ctx = Ctx {
        runtime,
        info,
        session: Some(session_id.clone()),
        identity: None,
        capabilities,
        stream: Some(ord),
        manifest,
    };

    tokio::spawn(async move {
        for msg in messages {
            match engine::handle(&ctx, msg).await {
                Handled::Accepted => {}
                Handled::Reply(out) | Handled::Initialized { reply: out, .. } => {
                    registry.send_on(&session_id, ord, &out);
                }
            }
        }
        // The spec: close the stream once the response for the originating
        // request has been sent.
        registry.close_stream(&session_id, ord);
    });

    stream.into_response(None, Vec::new())
}

/// Should this POST be answered with a stream?
///
/// Two triggers, both statically knowable: the client attached a
/// `_meta.progressToken` (so it is expecting progress), or the tool declared
/// `streaming`. A tool that calls `mcp().progress()` without either gets
/// `false` back from that call rather than a notification vanishing — there is
/// genuinely no stream to write to once a JSON response has been chosen.
fn wants_stream(messages: &[Incoming], manifest: Option<&ServerManifest>) -> bool {
    messages.iter().any(|m| {
        let Incoming::Request { method, params, .. } = m else { return false };
        if method != protocol::methods::TOOLS_CALL {
            return false;
        }
        if params.get("_meta").and_then(|meta| meta.get("progressToken")).is_some() {
            return true;
        }
        let Some(manifest) = manifest else { return false };
        params
            .get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| manifest.find(cfml_vm::mcp::EntityKind::Tool, n))
            .is_some_and(|e| e.streaming)
    })
}

fn is_tool_call(msg: &Incoming) -> bool {
    matches!(msg, Incoming::Request { method, .. } if method == protocol::methods::TOOLS_CALL)
}

fn session_capabilities(
    registry: &Arc<cfml_vm::mcp::McpRegistry>,
    session: Option<&str>,
) -> ClientCapabilities {
    session
        .and_then(|id| registry.get(id))
        .map(|s| s.lock().capabilities.clone())
        .unwrap_or_default()
}

/// `GET /mcp/{name}` — the standalone server→client stream.
pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    addr: ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
) -> Response {
    if super::resolve_server(&state.doc_root, &state.vfs, &name).is_none() {
        return crate::handle_request_for_mcp(State(state), addr, req).await;
    }
    let headers = req.headers().clone();
    if let Some(deny) = guard(&headers) {
        return deny;
    }
    // A client that will not accept an event stream cannot be given one; 405 is
    // the spec's answer, and is what makes this endpoint safe to probe.
    if !header_str(&headers, "accept")
        .map(|a| a.contains("text/event-stream"))
        .unwrap_or(false)
    {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST, DELETE")],
            "This endpoint streams only to clients that accept text/event-stream",
        )
            .into_response();
    }

    let registry = state.server_state.mcp.clone();
    let Some(session_id) = header_str(&headers, SESSION_HEADER) else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            protocol::INVALID_REQUEST,
            "Missing Mcp-Session-Id header",
        );
    };
    if !registry.touch(&session_id, now_ms()) {
        return rpc_error(
            StatusCode::NOT_FOUND,
            protocol::INVALID_REQUEST,
            "Unknown or expired MCP session",
        );
    }

    // Resumption: replay what this client missed on *its own* stream. Replaying
    // another stream's events is an explicit spec violation, so an id naming a
    // stream we no longer hold starts fresh instead.
    let replay = header_str(&headers, "last-event-id")
        .and_then(|last| registry.replay_since(&session_id, &last))
        .map(|(_, events)| events)
        .unwrap_or_default();

    let stream = sse::Stream::new();
    if registry
        .attach_stream(&session_id, StreamKind::Standalone, stream.sink.clone(), sse::REPLAY_CAP)
        .is_none()
    {
        return rpc_error(
            StatusCode::NOT_FOUND,
            protocol::INVALID_REQUEST,
            "Unknown or expired MCP session",
        );
    }
    stream.into_response(None, replay)
}

/// `DELETE /mcp/{name}` — explicit session teardown.
pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    addr: ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
) -> Response {
    if super::resolve_server(&state.doc_root, &state.vfs, &name).is_none() {
        return crate::handle_request_for_mcp(State(state), addr, req).await;
    }
    if let Some(deny) = guard(req.headers()) {
        return deny;
    }
    match header_str(req.headers(), SESSION_HEADER) {
        Some(id) if state.server_state.mcp.end_session(&id) => {
            StatusCode::NO_CONTENT.into_response()
        }
        Some(_) => rpc_error(
            StatusCode::NOT_FOUND,
            protocol::INVALID_REQUEST,
            "Unknown or expired MCP session",
        ),
        None => rpc_error(
            StatusCode::BAD_REQUEST,
            protocol::INVALID_REQUEST,
            "Missing Mcp-Session-Id header",
        ),
    }
}

/// The checks every method shares: origin, then protocol version.
fn guard(headers: &HeaderMap) -> Option<Response> {
    origin_rejected(headers).or_else(|| version_rejected(headers))
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(|s| s.to_string())
}

/// Reject a cross-origin browser request.
///
/// The spec makes this a MUST: without it a web page can drive a local MCP
/// server through DNS rebinding. Non-browser clients send no `Origin` at all,
/// which is not a cross-origin request and is allowed through.
fn origin_rejected(headers: &HeaderMap) -> Option<Response> {
    let origin = header_str(headers, "origin")?;
    if origin_is_local(&origin) {
        return None;
    }
    Some((StatusCode::FORBIDDEN, format!("Origin not allowed for MCP: {origin}")).into_response())
}

fn origin_is_local(origin: &str) -> bool {
    let authority = origin.split("//").nth(1).unwrap_or(origin);
    let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// A client that names a protocol revision we cannot speak gets a 400, as the
/// spec requires. An absent header is *not* an error: it means the client
/// predates the header, and the spec pins the assumed version.
fn version_rejected(headers: &HeaderMap) -> Option<Response> {
    let version = header_str(headers, VERSION_HEADER)?;
    if protocol::version_supported(&version) {
        return None;
    }
    Some(rpc_error(
        StatusCode::BAD_REQUEST,
        protocol::INVALID_REQUEST,
        format!("Unsupported MCP-Protocol-Version: {version}"),
    ))
}

fn rpc_error(status: StatusCode, code: i64, message: impl Into<String>) -> Response {
    let body = Outgoing::Error { id: RpcId::Null, code, message: message.into(), data: None };
    (status, axum::Json(body.to_json())).into_response()
}

/// Pull the client's declared capabilities out of an `initialize` request.
fn initialize_capabilities(msg: &Incoming) -> Option<ClientCapabilities> {
    match msg {
        Incoming::Request { method, params, .. } if method == protocol::methods::INITIALIZE => {
            Some(ClientCapabilities::from_json(
                params.get("capabilities").unwrap_or(&Value::Null),
            ))
        }
        _ => None,
    }
}

/// The `clientInfo` recorded on the session, for `mcp().client()`.
fn client_info_of(reply: &Outgoing) -> Value {
    match reply {
        Outgoing::Result { result, .. } => {
            result.get("serverInfo").cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_call(params: Value) -> Incoming {
        Incoming::Request {
            id: RpcId::Num(1),
            method: protocol::methods::TOOLS_CALL.to_string(),
            params,
        }
    }

    #[test]
    fn local_origins_are_allowed_and_others_are_not() {
        assert!(origin_is_local("http://localhost:8500"));
        assert!(origin_is_local("http://127.0.0.1:8500"));
        assert!(origin_is_local("https://localhost"));
        assert!(!origin_is_local("https://evil.example.com"));
        assert!(!origin_is_local("http://attacker.localhost.evil.com"));
    }

    #[test]
    fn an_absent_origin_is_not_a_cross_origin_request() {
        assert!(origin_rejected(&HeaderMap::new()).is_none());
    }

    #[test]
    fn an_unsupported_protocol_version_is_rejected_but_an_absent_one_is_not() {
        assert!(version_rejected(&HeaderMap::new()).is_none());
        let mut headers = HeaderMap::new();
        headers.insert(VERSION_HEADER, "2025-06-18".parse().unwrap());
        assert!(version_rejected(&headers).is_none());
        headers.insert(VERSION_HEADER, "1999-01-01".parse().unwrap());
        assert!(version_rejected(&headers).is_some());
    }

    #[test]
    fn a_progress_token_asks_for_a_stream() {
        let with_token = tool_call(json!({ "name": "x", "_meta": { "progressToken": 7 } }));
        assert!(wants_stream(&[with_token], None));

        let without = tool_call(json!({ "name": "x" }));
        assert!(!wants_stream(&[without], None), "a plain call stays on JSON");
    }

    #[test]
    fn a_non_tool_request_never_opens_a_stream() {
        let ping = Incoming::Request {
            id: RpcId::Num(1),
            method: protocol::methods::PING.to_string(),
            params: json!({ "_meta": { "progressToken": 1 } }),
        };
        assert!(!wants_stream(&[ping], None));
        assert!(!wants_stream(&[], None));
    }
}
