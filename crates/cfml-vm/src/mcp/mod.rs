//! Model Context Protocol support.
//!
//! One CFC under `<docroot>/mcp/` is one MCP server, exactly as one CFC under
//! `<docroot>/websockets/` is one realtime channel. Its public methods become
//! tools, resources and prompts purely by annotation:
//!
//! ```cfml
//! component mcp="docs" version="1.0.0" description="Documentation tools" {
//!     function searchDocs( required string query, numeric limit = 10 )
//!         tool = "search_docs" description = "Search the documentation"
//!     {
//!         return results;   // coerced to MCP content by `content.rs`
//!     }
//! }
//! ```
//!
//! The layering mirrors WebSockets: everything here is transport-agnostic and
//! `wasm32`-safe (no tokio, no axum, no clock). The three transports —
//! Streamable HTTP, the deprecated HTTP+SSE pair, and stdio — live in
//! `crates/cli/src/mcp/` and share this vocabulary, so a rule fixed once is
//! fixed for all three.

#[cfg(feature = "mcp-client")]
pub mod client;
pub mod content;
pub mod handle;
pub mod protocol;
pub mod registry;
pub mod schema;

use cfml_common::dynamic::{CfmlAccess, CfmlStruct, CfmlValue};
use serde_json::{json, Map, Value};

pub use handle::McpHandle;
pub use protocol::{Incoming, Outgoing, RpcId};
pub use registry::{ClientCapabilities, McpRegistry, McpSink, StreamKind};

/// `CfmlErrorType::Custom` marker for an authorization denial, so a transport
/// can turn it into a JSON-RPC error instead of a tool result. A denial is not
/// a tool failure: the model should not be handed "try again", it should be
/// told the call was refused.
pub const UNAUTHORIZED_TYPE: &str = "MCPUnauthorized";

/// What kind of MCP entity a method is exposed as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Tool,
    Resource,
    Prompt,
}

impl EntityKind {
    /// The function annotation that declares it (`tool="x"`, `resource="…"`,
    /// `prompt="x"`).
    pub fn annotation(self) -> &'static str {
        match self {
            EntityKind::Tool => "tool",
            EntityKind::Resource => "resource",
            EntityKind::Prompt => "prompt",
        }
    }
}

/// One exposed method: the wire-facing descriptor plus the CFML function name
/// to dispatch to.
#[derive(Clone, Debug)]
pub struct Entity {
    pub kind: EntityKind,
    /// The MCP name (tool/prompt) or URI (resource) from the annotation.
    pub name: String,
    /// The CFML method to invoke.
    pub function: String,
    /// `secured` annotation, if any — same semantics as the WebSocket gate.
    pub secured: Option<String>,
    /// The JSON descriptor served by `tools/list` / `resources/list` /
    /// `prompts/list`.
    pub descriptor: Value,
    /// Declared parameters, used to bind incoming arguments by name.
    pub params: Vec<String>,
    /// `streaming` annotation: this handler wants an SSE stream even when the
    /// client attached no progress token, because it logs or (phase 4) asks the
    /// client questions mid-call. The transport must decide the response mode
    /// **before** the handler runs, so this cannot be inferred at runtime.
    pub streaming: bool,
}

impl Entity {
    /// True when the resource name is a URI *template* (`doc://pages/{slug}`),
    /// which `resources/templates/list` serves and `resources/read` matches
    /// against rather than comparing literally.
    pub fn is_template(&self) -> bool {
        self.kind == EntityKind::Resource && self.name.contains('{')
    }
}

/// Everything a transport needs to answer a request without touching the VM
/// again: the server's identity, its capabilities, and its entities.
#[derive(Clone, Debug, Default)]
pub struct ServerManifest {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
    pub instructions: Option<String>,
    pub entities: Vec<Entity>,
}

impl ServerManifest {
    pub fn find(&self, kind: EntityKind, name: &str) -> Option<&Entity> {
        self.entities
            .iter()
            .find(|e| e.kind == kind && e.name.eq_ignore_ascii_case(name))
    }

    pub fn of_kind(&self, kind: EntityKind) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter(move |e| e.kind == kind)
    }

    /// The `capabilities` block for `initialize`. A capability is declared only
    /// when the server actually has entities of that kind — advertising
    /// `resources` on a server with none makes clients issue pointless calls.
    pub fn capabilities(&self) -> Value {
        let mut caps = Map::new();
        if self.of_kind(EntityKind::Tool).next().is_some() {
            caps.insert("tools".into(), json!({ "listChanged": true }));
        }
        if self.of_kind(EntityKind::Resource).next().is_some() {
            caps.insert("resources".into(), json!({ "listChanged": true, "subscribe": false }));
        }
        if self.of_kind(EntityKind::Prompt).next().is_some() {
            caps.insert("prompts".into(), json!({ "listChanged": true }));
        }
        caps.insert("logging".into(), json!({}));
        Value::Object(caps)
    }

    /// The `serverInfo` block for `initialize`.
    pub fn server_info(&self) -> Value {
        let mut info = Map::new();
        info.insert("name".into(), json!(self.name));
        if let Some(t) = &self.title {
            info.insert("title".into(), json!(t));
        }
        info.insert("version".into(), json!(self.version));
        Value::Object(info)
    }
}

/// The live context of one MCP handler invocation. Carried on the VM for the
/// duration of a dispatch so `mcp()` can reach the session and — once the
/// streaming transport is in — write progress and log notifications onto the
/// very stream the caller's request opened.
#[derive(Clone, Debug)]
pub struct CallContext {
    /// Server id (`/mcp/<name>`), the key `mcpNotify` fans out on.
    pub server: String,
    pub session: String,
    /// The request-scoped stream, when the transport opened one. `None` for a
    /// plain JSON response and for stdio, where there is nothing to stream on.
    pub stream: Option<registry::StreamOrd>,
    /// The client's `_meta.progressToken`, if it sent one. Progress
    /// notifications are only legal when it did.
    pub progress_token: Option<Value>,
    /// What the client said it can do — gates sampling/elicitation.
    pub capabilities: ClientCapabilities,
}

/// Read an annotation off a `__funcmeta_<name>` struct, case-insensitively.
fn funcmeta_value(meta: &CfmlStruct, key: &str) -> Option<String> {
    meta.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_string())
}

/// Build a server manifest from a component's public view.
///
/// Reads the same compiler-emitted `__funcmeta_<name>` structs the WebSocket
/// `on=` router uses, so annotation handling behaves identically across the two
/// features. Only `public` and `remote` methods are eligible: a `private`
/// helper annotated `tool=` is a mistake, and silently exposing it would be a
/// security bug.
pub fn manifest_from_component(
    view: &CfmlStruct,
    fallback_name: &str,
    component_meta: &CfmlStruct,
) -> ServerManifest {
    let mut manifest = ServerManifest {
        name: component_meta
            .get_ci("mcp")
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback_name.to_string()),
        title: component_meta.get_ci("title").map(|v| v.as_string()).filter(|s| !s.is_empty()),
        version: component_meta
            .get_ci("version")
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "1.0.0".to_string()),
        instructions: component_meta
            .get_ci("instructions")
            .or_else(|| component_meta.get_ci("description"))
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty()),
        entities: Vec::new(),
    };

    for (key, value) in view.iter() {
        let Some(fname) = key.strip_prefix("__funcmeta_") else { continue };
        let CfmlValue::Struct(ref meta) = value else { continue };
        let Some(CfmlValue::Function(func)) = view.get_ci(fname) else { continue };
        if !matches!(func.access, CfmlAccess::Public | CfmlAccess::Remote) {
            continue;
        }
        for kind in [EntityKind::Tool, EntityKind::Resource, EntityKind::Prompt] {
            let Some(name) = funcmeta_value(meta, kind.annotation()).filter(|s| !s.is_empty())
            else {
                continue;
            };
            let descriptor = descriptor_for(kind, &name, meta, &func);
            manifest.entities.push(Entity {
                kind,
                name,
                function: func.name.clone(),
                secured: secured_annotation(meta),
                streaming: funcmeta_value(meta, "streaming")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1"),
                descriptor,
                params: func.params.iter().map(|p| p.name.clone()).collect(),
            });
        }
    }

    // Deterministic order: clients cache tool lists and a churning order shows
    // up as spurious `list_changed` churn in their UIs.
    manifest.entities.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    manifest
}

/// `secured` / `secured="a,b"` on the handler. `secured="false"` opts out.
/// Same contract as the WebSocket gate so authors learn it once.
fn secured_annotation(meta: &CfmlStruct) -> Option<String> {
    let val = funcmeta_value(meta, "secured")?;
    if val.eq_ignore_ascii_case("false") {
        return None;
    }
    Some(if val.eq_ignore_ascii_case("true") { String::new() } else { val })
}

fn descriptor_for(
    kind: EntityKind,
    name: &str,
    meta: &CfmlStruct,
    func: &cfml_common::dynamic::CfmlFunction,
) -> Value {
    let description = funcmeta_value(meta, "description")
        .or_else(|| funcmeta_value(meta, "hint"))
        .filter(|s| !s.is_empty());
    let title = funcmeta_value(meta, "title").filter(|s| !s.is_empty());
    let mut d = Map::new();

    match kind {
        EntityKind::Resource => {
            d.insert(if name.contains('{') { "uriTemplate" } else { "uri" }.into(), json!(name));
            d.insert("name".into(), json!(title.clone().unwrap_or_else(|| func.name.clone())));
            if let Some(mime) = funcmeta_value(meta, "mimeType").filter(|s| !s.is_empty()) {
                d.insert("mimeType".into(), json!(mime));
            }
        }
        EntityKind::Prompt => {
            d.insert("name".into(), json!(name));
            // Prompt arguments are a flat list, not a JSON Schema.
            let args: Vec<Value> = func
                .params
                .iter()
                .map(|p| {
                    let mut a = Map::new();
                    a.insert("name".into(), json!(p.name));
                    if let Some((_, v)) = p.annotations.iter().find(|(k, _)| {
                        k.eq_ignore_ascii_case("description") || k.eq_ignore_ascii_case("hint")
                    }) {
                        a.insert("description".into(), json!(v));
                    }
                    a.insert("required".into(), json!(p.required && p.default.is_none()));
                    Value::Object(a)
                })
                .collect();
            d.insert("arguments".into(), Value::Array(args));
        }
        EntityKind::Tool => {
            d.insert("name".into(), json!(name));
            let input = funcmeta_value(meta, "inputSchema")
                .and_then(|raw| schema::explicit_schema(&raw))
                .unwrap_or_else(|| schema::input_schema(&func.params));
            d.insert("inputSchema".into(), input);
            if let Some(out) = funcmeta_value(meta, "outputSchema")
                .and_then(|raw| schema::explicit_schema(&raw))
            {
                d.insert("outputSchema".into(), out);
            }
            let hints = tool_annotations(meta);
            if !hints.is_empty() {
                d.insert("annotations".into(), Value::Object(hints));
            }
        }
    }

    if let Some(t) = title {
        d.insert("title".into(), json!(t));
    }
    if let Some(desc) = description {
        d.insert("description".into(), json!(desc));
    }
    Value::Object(d)
}

/// The optional behaviour hints (`readOnly`, `destructive`, `idempotent`,
/// `openWorld`) a client uses to decide how much friction to put in front of a
/// call. Written in CFML as plain function annotations: `readOnly = true`.
fn tool_annotations(meta: &CfmlStruct) -> Map<String, Value> {
    let mut out = Map::new();
    for (annotation, wire) in [
        ("readOnly", "readOnlyHint"),
        ("destructive", "destructiveHint"),
        ("idempotent", "idempotentHint"),
        ("openWorld", "openWorldHint"),
    ] {
        if let Some(v) = funcmeta_value(meta, annotation) {
            if !v.is_empty() {
                out.insert(wire.into(), json!(v.eq_ignore_ascii_case("true") || v == "1"));
            }
        }
    }
    out
}

/// Match a `resources/read` URI against a URI template, returning the captured
/// variables. `doc://pages/{slug}` + `doc://pages/intro` → `{ slug: "intro" }`.
/// A template segment never matches across `/`, so one template cannot swallow
/// a deeper path it was not meant to serve.
pub fn match_uri_template(template: &str, uri: &str) -> Option<Vec<(String, String)>> {
    let mut vars = Vec::new();
    let mut t = template;
    let mut u = uri;
    while let Some(open) = t.find('{') {
        let (literal, rest) = t.split_at(open);
        if !u.starts_with(literal) {
            return None;
        }
        u = &u[literal.len()..];
        let close = rest.find('}')?;
        let var = &rest[1..close];
        t = &rest[close + 1..];
        // The variable runs until the next literal character of the template,
        // and never past a path separator.
        let next_literal = t.chars().next();
        let end = match next_literal {
            Some(c) => u.find(c).unwrap_or(u.len()),
            None => u.len(),
        };
        let end = u[..end].find('/').unwrap_or(end);
        if end == 0 {
            return None;
        }
        vars.push((var.to_string(), u[..end].to_string()));
        u = &u[end..];
    }
    (t == u).then_some(vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::{CfmlClosureBody, CfmlFunction, CfmlParam, ValueMap};
    use std::sync::Arc;

    fn strukt(pairs: Vec<(&str, CfmlValue)>) -> CfmlStruct {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert(k.to_string(), v);
        }
        match CfmlValue::strukt(m) {
            CfmlValue::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    fn func(name: &str, access: CfmlAccess, params: Vec<CfmlParam>) -> CfmlValue {
        CfmlValue::Function(Arc::new(CfmlFunction {
            name: name.to_string(),
            params,
            body: CfmlClosureBody::Statements(Vec::new()),
            return_type: None,
            access,
            captured_scope: None,
        }))
    }

    fn param(name: &str, ty: &str, required: bool) -> CfmlParam {
        CfmlParam {
            name: name.to_string(),
            param_type: Some(ty.to_string()),
            default: None,
            required,
            annotations: Vec::new(),
        }
    }

    fn view_with(fname: &str, access: CfmlAccess, meta: Vec<(&str, CfmlValue)>) -> CfmlStruct {
        strukt(vec![
            (fname, func(fname, access, vec![param("query", "string", true)])),
            (
                &format!("__funcmeta_{fname}"),
                CfmlValue::Struct(strukt(meta)),
            ),
        ])
    }

    #[test]
    fn a_tool_annotation_produces_a_descriptor_with_a_derived_schema() {
        let view = view_with(
            "searchDocs",
            CfmlAccess::Public,
            vec![
                ("tool", CfmlValue::string("search_docs")),
                ("description", CfmlValue::string("Search the docs")),
                ("readOnly", CfmlValue::string("true")),
            ],
        );
        let m = manifest_from_component(&view, "docs", &strukt(vec![]));
        let tool = m.find(EntityKind::Tool, "search_docs").expect("tool exposed");
        assert_eq!(tool.function, "searchDocs");
        assert_eq!(tool.descriptor["description"], "Search the docs");
        assert_eq!(tool.descriptor["inputSchema"]["properties"]["query"]["type"], "string");
        assert_eq!(tool.descriptor["annotations"]["readOnlyHint"], true);
        assert_eq!(m.capabilities()["tools"]["listChanged"], true);
        assert!(m.capabilities().get("resources").is_none(), "no resources declared");
    }

    #[test]
    fn private_methods_are_never_exposed() {
        let view = view_with(
            "secretHelper",
            CfmlAccess::Private,
            vec![("tool", CfmlValue::string("secret"))],
        );
        let m = manifest_from_component(&view, "docs", &strukt(vec![]));
        assert!(m.entities.is_empty(), "a private method annotated tool= must stay unreachable");
    }

    #[test]
    fn component_attributes_name_and_version_the_server() {
        let meta = strukt(vec![
            ("mcp", CfmlValue::string("docs")),
            ("version", CfmlValue::string("2.1.0")),
            ("description", CfmlValue::string("Docs tools")),
        ]);
        let m = manifest_from_component(&strukt(vec![]), "fallback", &meta);
        assert_eq!(m.server_info()["name"], "docs");
        assert_eq!(m.server_info()["version"], "2.1.0");
        assert_eq!(m.instructions.as_deref(), Some("Docs tools"));

        let bare = manifest_from_component(&strukt(vec![]), "fallback", &strukt(vec![]));
        assert_eq!(bare.server_info()["name"], "fallback");
        assert_eq!(bare.server_info()["version"], "1.0.0");
    }

    #[test]
    fn secured_follows_the_websocket_contract() {
        let bare = view_with(
            "a",
            CfmlAccess::Public,
            vec![("tool", CfmlValue::string("a")), ("secured", CfmlValue::string("true"))],
        );
        assert_eq!(
            manifest_from_component(&bare, "d", &strukt(vec![])).entities[0].secured.as_deref(),
            Some("")
        );
        let roles = view_with(
            "b",
            CfmlAccess::Public,
            vec![("tool", CfmlValue::string("b")), ("secured", CfmlValue::string("admin,ops"))],
        );
        assert_eq!(
            manifest_from_component(&roles, "d", &strukt(vec![])).entities[0].secured.as_deref(),
            Some("admin,ops")
        );
        let opted_out = view_with(
            "c",
            CfmlAccess::Public,
            vec![("tool", CfmlValue::string("c")), ("secured", CfmlValue::string("false"))],
        );
        assert!(manifest_from_component(&opted_out, "d", &strukt(vec![])).entities[0].secured.is_none());
    }

    #[test]
    fn resources_distinguish_uri_from_uri_template() {
        let plain = view_with(
            "page",
            CfmlAccess::Public,
            vec![("resource", CfmlValue::string("doc://index"))],
        );
        let e = &manifest_from_component(&plain, "d", &strukt(vec![])).entities[0];
        assert!(!e.is_template());
        assert_eq!(e.descriptor["uri"], "doc://index");

        let tmpl = view_with(
            "page",
            CfmlAccess::Public,
            vec![("resource", CfmlValue::string("doc://pages/{slug}"))],
        );
        let e = &manifest_from_component(&tmpl, "d", &strukt(vec![])).entities[0];
        assert!(e.is_template());
        assert_eq!(e.descriptor["uriTemplate"], "doc://pages/{slug}");
    }

    #[test]
    fn uri_templates_match_one_segment_only() {
        assert_eq!(
            match_uri_template("doc://pages/{slug}", "doc://pages/intro"),
            Some(vec![("slug".into(), "intro".into())])
        );
        assert_eq!(
            match_uri_template("doc://{a}/{b}", "doc://x/y"),
            Some(vec![("a".into(), "x".into()), ("b".into(), "y".into())])
        );
        assert!(
            match_uri_template("doc://pages/{slug}", "doc://pages/deep/path").is_none(),
            "a template must not swallow a deeper path"
        );
        assert!(match_uri_template("doc://pages/{slug}", "doc://pages/").is_none());
        assert!(match_uri_template("doc://index", "doc://index").is_some());
        assert!(match_uri_template("doc://index", "doc://other").is_none());
    }

    #[test]
    fn entities_are_listed_in_a_stable_order() {
        let mut view = view_with(
            "zeta",
            CfmlAccess::Public,
            vec![("tool", CfmlValue::string("zeta"))],
        );
        view.insert("alpha", func("alpha", CfmlAccess::Public, vec![]));
        view.insert("__funcmeta_alpha",
            CfmlValue::Struct(strukt(vec![("tool", CfmlValue::string("alpha"))])),
        );
        let m = manifest_from_component(&view, "d", &strukt(vec![]));
        let names: Vec<&str> = m.entities.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }
}
