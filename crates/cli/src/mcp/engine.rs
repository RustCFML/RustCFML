//! The transport-independent MCP request engine.
//!
//! Every transport decodes bytes into a [`Incoming`] and hands it here; the
//! JSON-RPC method vocabulary, argument validation and result shaping live in
//! exactly one place, so HTTP and stdio cannot drift apart. Anything genuinely
//! transport-specific — session headers, SSE framing, `Origin` checks — stays
//! in the transport.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_vm::mcp::{
    content, match_uri_template, protocol::methods, CallContext, Entity, EntityKind, Incoming,
    Outgoing, RpcId, ServerManifest,
};
use cfml_vm::mcp::protocol::{
    negotiate_version, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND,
};
use serde_json::{json, Value};

use super::dispatch::{self, McpRuntime};
use super::ServerInfo;

/// What a transport learns from handling one message.
pub(crate) enum Handled {
    /// Write this back to the client.
    Reply(Box<Outgoing>),
    /// A notification or response — nothing to send (HTTP answers 202).
    Accepted,
    /// An `initialize` that succeeded: the transport must mint/attach a session
    /// and return this result.
    Initialized { reply: Box<Outgoing>, negotiated: String },
}

/// The per-message context a transport supplies.
pub(crate) struct Ctx {
    pub(crate) runtime: McpRuntime,
    pub(crate) info: ServerInfo,
    pub(crate) session: Option<String>,
    /// Who is calling, and what they may use. Carries the identity the
    /// `secured` gate checks.
    pub(crate) caller: super::auth::Caller,
    pub(crate) capabilities: cfml_vm::mcp::ClientCapabilities,
    /// The request-scoped stream, when the transport opened one.
    pub(crate) stream: Option<u64>,
    /// Manifest already loaded by the transport (it needs one to choose the
    /// response mode). Reused rather than recompiling the component.
    pub(crate) manifest: Option<ServerManifest>,
}

/// Handle one decoded inbound message.
pub(crate) async fn handle(ctx: &Ctx, msg: Incoming) -> Handled {
    match msg {
        // A notification never gets an answer, and an unknown one must be
        // ignored rather than errored — clients send notifications we have not
        // implemented and are entitled to expect silence.
        Incoming::Notification { .. } => Handled::Accepted,
        // Responses to server→client requests are routed by the transport
        // before they reach here (they need the session's pending table).
        Incoming::Response { .. } | Incoming::Error { .. } => Handled::Accepted,
        Incoming::Request { id, method, params } => {
            request(ctx, id, &method, params).await
        }
    }
}

async fn request(ctx: &Ctx, id: RpcId, method: &str, params: Value) -> Handled {
    match method {
        methods::PING => reply(id, json!({})),
        methods::INITIALIZE => initialize(ctx, id, params).await,
        methods::TOOLS_LIST => list(ctx, id, EntityKind::Tool, "tools").await,
        methods::TOOLS_CALL => call_tool(ctx, id, params).await,
        methods::RESOURCES_LIST => list(ctx, id, EntityKind::Resource, "resources").await,
        methods::RESOURCES_TEMPLATES_LIST => templates(ctx, id).await,
        methods::RESOURCES_READ => read_resource(ctx, id, params).await,
        methods::PROMPTS_LIST => list(ctx, id, EntityKind::Prompt, "prompts").await,
        methods::PROMPTS_GET => get_prompt(ctx, id, params).await,
        methods::COMPLETION_COMPLETE => complete(ctx, id, params).await,
        methods::LOGGING_SET_LEVEL => set_log_level(ctx, id, params),
        _ => Handled::Reply(Box::new(Outgoing::error(
            id,
            METHOD_NOT_FOUND,
            format!("Method not supported: {method}"),
        ))),
    }
}

fn reply(id: RpcId, result: Value) -> Handled {
    Handled::Reply(Box::new(Outgoing::Result { id, result }))
}

fn fail(id: RpcId, code: i64, message: impl Into<String>) -> Handled {
    Handled::Reply(Box::new(Outgoing::error(id, code, message)))
}

async fn manifest_or_fail(ctx: &Ctx, id: &RpcId) -> Result<ServerManifest, Handled> {
    if let Some(manifest) = &ctx.manifest {
        return Ok(manifest.clone());
    }
    dispatch::manifest(ctx.runtime.clone(), ctx.info.clone())
        .await
        .map_err(|e| fail(id.clone(), INTERNAL_ERROR, format!("MCP server failed to load: {e}")))
}

async fn initialize(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };
    let requested = params.get("protocolVersion").and_then(|v| v.as_str());
    let negotiated = negotiate_version(requested);

    let mut result = json!({
        "protocolVersion": negotiated,
        "capabilities": manifest.capabilities(),
        "serverInfo": manifest.server_info(),
    });
    if let Some(instructions) = &manifest.instructions {
        result["instructions"] = json!(instructions);
    }
    Handled::Initialized {
        reply: Box::new(Outgoing::Result { id, result }),
        negotiated: negotiated.to_string(),
    }
}

async fn list(ctx: &Ctx, id: RpcId, kind: EntityKind, key: &str) -> Handled {
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };
    // Tools the caller may not use are not listed. Advertising a tool that
    // will be refused wastes a model's turn and leaks the shape of what it is
    // not allowed to reach.
    let items: Vec<Value> = manifest
        .of_kind(kind)
        .filter(|e| !e.is_template())
        .filter(|e| kind != EntityKind::Tool || ctx.caller.may_call(&e.name))
        .map(|e| e.descriptor.clone())
        .collect();
    // No `nextCursor`: the whole list is served in one page. Clients treat an
    // absent cursor as "that is everything", which it is.
    reply(id, json!({ key: items }))
}

async fn call_tool(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return fail(id, INVALID_PARAMS, "tools/call requires a \"name\"");
    };
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };
    let Some(entity) = manifest.find(EntityKind::Tool, name) else {
        return fail(id, INVALID_PARAMS, format!("Unknown tool: {name}"));
    };
    if !ctx.caller.may_call(name) {
        // The same answer an unknown tool gets: a caller who may not use it
        // should not be able to discover that it exists.
        return fail(id, INVALID_PARAMS, format!("Unknown tool: {name}"));
    }

    let arguments = match params.get("arguments") {
        Some(Value::Object(_)) | None => json_args(params.get("arguments")),
        Some(_) => {
            return fail(id, INVALID_PARAMS, "tools/call \"arguments\" must be an object")
        }
    };

    let call_ctx = CallContext {
        server: ctx.info.id.clone(),
        session: ctx.session.clone().unwrap_or_default(),
        stream: ctx.stream,
        progress_token: params
            .get("_meta")
            .and_then(|m| m.get("progressToken"))
            .cloned(),
        capabilities: ctx.capabilities.clone(),
    };

    match dispatch::call(
        ctx.runtime.clone(),
        ctx.info.clone(),
        entity.clone(),
        arguments,
        call_ctx,
        ctx.caller.identity.clone(),
    )
    .await
    {
        Ok(value) => reply(id, content::tool_result(&value)),
        // A refusal is a protocol error: the caller is not allowed to make this
        // call at all, and dressing that up as a tool result would invite the
        // model to retry it forever.
        Err(f) if f.denied => fail(id, INVALID_REQUEST, f.message),
        // A handler that threw is a *tool* failure, not a protocol failure:
        // the model gets the message back and can adapt, which is the whole
        // point of `isError`.
        Err(f) => reply(id, content::tool_error(&f.message)),
    }
}

/// `resources/templates/list` — the parameterised resources, which
/// `resources/list` deliberately omits: a template is not a readable URI, and
/// listing it as one makes clients try to read `doc://pages/{slug}` literally.
async fn templates(ctx: &Ctx, id: RpcId) -> Handled {
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };
    let items: Vec<Value> = manifest
        .of_kind(EntityKind::Resource)
        .filter(|e| e.is_template())
        .map(|e| e.descriptor.clone())
        .collect();
    reply(id, json!({ "resourceTemplates": items }))
}

/// `resources/read`.
///
/// A literal URI matches exactly (URIs are case-sensitive), and only if nothing
/// matches literally are templates tried — otherwise a template could shadow a
/// concrete resource it happens to cover.
async fn read_resource(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let Some(uri) = params.get("uri").and_then(|v| v.as_str()) else {
        return fail(id, INVALID_PARAMS, "resources/read requires a \"uri\"");
    };
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };

    let mut arguments = ValueMap::default();
    let entity = manifest
        .of_kind(EntityKind::Resource)
        .find(|e| !e.is_template() && e.name == uri)
        .cloned()
        .or_else(|| {
            manifest.of_kind(EntityKind::Resource).filter(|e| e.is_template()).find_map(|e| {
                match_uri_template(&e.name, uri).map(|vars| {
                    for (k, v) in vars {
                        arguments.insert(k, CfmlValue::string(v));
                    }
                    e.clone()
                })
            })
        });
    let Some(entity) = entity else {
        return fail(id, INVALID_PARAMS, format!("Unknown resource: {uri}"));
    };
    // A handler may also declare a `uri` parameter to see what was asked for.
    arguments.insert("uri".to_string(), CfmlValue::string(uri.to_string()));

    let mime = entity.descriptor.get("mimeType").and_then(|m| m.as_str()).map(String::from);
    match invoke(ctx, &entity, arguments, &params, &id).await {
        Ok(value) => reply(id, content::resource_result(uri, mime.as_deref(), &value)),
        Err(h) => h,
    }
}

/// `prompts/get`.
async fn get_prompt(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return fail(id, INVALID_PARAMS, "prompts/get requires a \"name\"");
    };
    let manifest = match manifest_or_fail(ctx, &id).await {
        Ok(m) => m,
        Err(h) => return h,
    };
    let Some(entity) = manifest.find(EntityKind::Prompt, name).cloned() else {
        return fail(id, INVALID_PARAMS, format!("Unknown prompt: {name}"));
    };
    let description =
        entity.descriptor.get("description").and_then(|d| d.as_str()).map(String::from);
    let arguments = json_args(params.get("arguments"));
    match invoke(ctx, &entity, arguments, &params, &id).await {
        Ok(value) => reply(id, content::prompt_result(description.as_deref(), &value)),
        Err(h) => h,
    }
}

/// `completion/complete` — argument autocompletion.
///
/// Nothing to complete from is not an error: the spec's shape for "no
/// suggestions" is an empty values array, and answering that keeps a client's
/// completion UI working instead of showing it an error.
async fn complete(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let _ = (ctx, &params);
    reply(
        id,
        json!({ "completion": { "values": [], "total": 0, "hasMore": false } }),
    )
}

/// `logging/setLevel`.
///
/// Required of any server that advertises the `logging` capability — which we
/// do, because `mcp().log()` exists. Advertising it without serving this is
/// what the reference MCP Inspector trips over on its very first handshake.
fn set_log_level(ctx: &Ctx, id: RpcId, params: Value) -> Handled {
    let Some(level) = params.get("level").and_then(|v| v.as_str()) else {
        return fail(id, INVALID_PARAMS, "logging/setLevel requires a \"level\"");
    };
    if let Some(session) = &ctx.session {
        ctx.runtime.server_state.mcp.set_log_level(session, level);
    }
    reply(id, json!({}))
}

/// Shared dispatch for a resource or prompt handler: the same `secured` gate
/// and error mapping `tools/call` uses, except that a handler which throws is
/// a protocol error here — unlike a tool, there is no `isError` channel to
/// hand the model.
async fn invoke(
    ctx: &Ctx,
    entity: &Entity,
    arguments: ValueMap,
    params: &Value,
    // Carried through so a failure is correlatable: an error answered with
    // `id: null` is one the client cannot match to its request.
    id: &RpcId,
) -> Result<CfmlValue, Handled> {
    let call_ctx = CallContext {
        server: ctx.info.id.clone(),
        session: ctx.session.clone().unwrap_or_default(),
        stream: ctx.stream,
        progress_token: params.get("_meta").and_then(|m| m.get("progressToken")).cloned(),
        capabilities: ctx.capabilities.clone(),
    };
    dispatch::call(
        ctx.runtime.clone(),
        ctx.info.clone(),
        entity.clone(),
        arguments,
        call_ctx,
        ctx.caller.identity.clone(),
    )
    .await
    .map_err(|f| {
        let code = if f.denied { INVALID_REQUEST } else { INTERNAL_ERROR };
        fail(id.clone(), code, f.message)
    })
}

/// Turn the JSON `arguments` object into CFML values for binding.
fn json_args(value: Option<&Value>) -> ValueMap {
    let mut map = ValueMap::default();
    if let Some(Value::Object(obj)) = value {
        for (k, v) in obj {
            map.insert(k.clone(), cfml_vm::json_value_to_cfml(v.clone()));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_convert_to_cfml_values() {
        let args = json_args(Some(&json!({ "q": "hi", "n": 3, "flag": true })));
        assert_eq!(args.get("q").map(|v| v.as_string()).as_deref(), Some("hi"));
        assert!(matches!(args.get("n"), Some(CfmlValue::Int(3))));
        assert!(matches!(args.get("flag"), Some(CfmlValue::Bool(true))));
    }

    #[test]
    fn absent_arguments_bind_to_an_empty_map() {
        assert_eq!(json_args(None).len(), 0);
        assert_eq!(json_args(Some(&json!(null))).len(), 0);
    }
}
