//! VM-intercepted MCP builtins.
//!
//! Two groups. `mcp()` / `mcpNotify()` / `mcpSessions()` need engine state —
//! the call context on the VM, or the session registry on `ServerState` — so
//! they cannot be plain stdlib functions. The `mcp*` content builders are
//! pure, but live here too so the whole MCP vocabulary is in one place.
//!
//! The name set below MUST stay in step with
//! `cfml_common::builtins_meta::VM_INTERCEPTED`; the source-scanning guard in
//! `tests/intercept_declaration_guard.rs` enforces it.

use super::*;
use crate::mcp;

/// Names this module handles.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "mcp" | "mcpnotify" | "mcpsessions" | "mcptext" | "mcpimage" | "mcpaudio"
            | "mcpresource" | "mcpresourcelink" | "mcpconnect" | "mcpclient"
    )
}

impl CfmlVirtualMachine {
    /// Dispatch an MCP builtin.
    pub(crate) fn dispatch_mcp_bif(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
    ) -> CfmlResult {
        match name_lower {
            "mcp" => self.bif_mcp(),
            "mcpnotify" => self.bif_mcp_notify(args),
            "mcpsessions" => self.bif_mcp_sessions(args),
            "mcpconnect" => self.bif_mcp_connect(args),
            "mcpclient" => self.bif_mcp_client(args),
            // Content builders: pure shaping, no engine state. Built in a
            // helper rather than inline because the declaration guard scans a
            // window after `name_lower` for string literals, and the block
            // type/field names here would read as dispatched builtin names.
            other => content_builder(other, args),
        }
    }

    /// `mcp()` — the live handle for the MCP call in flight.
    fn bif_mcp(&mut self) -> CfmlResult {
        let Some(ctx) = self.current_mcp_call.clone() else {
            return Err(self.wrap_error(CfmlError::runtime(
                "mcp() is only available inside an MCP handler".to_string(),
            )));
        };
        let Some(registry) = self.server_state.as_ref().map(|ss| ss.mcp.clone()) else {
            return Err(self.wrap_error(CfmlError::runtime(
                "mcp() needs a running server".to_string(),
            )));
        };
        let handle = mcp::McpHandle::new(ctx, registry);
        Ok(CfmlValue::NativeObject(std::sync::Arc::new(std::sync::RwLock::new(handle))))
    }

    /// `mcpNotify( server=, method=, params=, to= )` — push a notification from
    /// anywhere: an ordinary page, a `cfthread`, a scheduled job. The
    /// emit-from-anywhere ergonomic `wsPublish` provides for WebSockets.
    ///
    /// Always recorded into `mcp_test_log` (like `ws_test_log`) so the CFML
    /// suite can assert on it with no client attached, then delivered to every
    /// session of that server which has a stream open. Returns the number of
    /// sessions reached.
    fn bif_mcp_notify(&mut self, args: Vec<CfmlValue>) -> CfmlResult {
        let named = self.pending_ws_named.take();
        let server = self
            .ws_arg(&args, &named, "server", 0)
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .or_else(|| self.current_mcp_call.as_ref().map(|c| c.server.clone()))
            .unwrap_or_default();
        let method = self
            .ws_arg(&args, &named, "method", 1)
            .map(|v| v.as_string())
            .unwrap_or_default();
        if method.is_empty() {
            return Err(self.wrap_error(CfmlError::runtime(
                "mcpNotify() needs a method name".to_string(),
            )));
        }
        let params = self.ws_arg(&args, &named, "params", 2).unwrap_or(CfmlValue::Null);
        let to = self
            .ws_arg(&args, &named, "to", 3)
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty());

        let mut record = ValueMap::default();
        record.insert("server".to_string(), CfmlValue::string(server.clone()));
        record.insert("method".to_string(), CfmlValue::string(method.clone()));
        record.insert("params".to_string(), params.clone());
        record.insert(
            "to".to_string(),
            to.clone().map(CfmlValue::string).unwrap_or(CfmlValue::Null),
        );
        self.mcp_test_log.push(CfmlValue::strukt(record));

        let Some(registry) = self.server_state.as_ref().map(|ss| ss.mcp.clone()) else {
            return Ok(CfmlValue::Int(0));
        };
        let msg = mcp::Outgoing::Notification {
            method,
            params: mcp::content::to_json(&params),
        };
        let delivered = match to {
            Some(session) => usize::from(registry.notify_session(&session, &msg)),
            None => registry.notify_server(&server, &msg),
        };
        Ok(CfmlValue::Int(delivered as i64))
    }

    /// `mcpConnect( transport, options )` — connect to a remote MCP server.
    ///
    /// The handshake runs here, so a bad command or URL fails at the connect
    /// call where the developer is looking, rather than at the first `tools()`
    /// somewhere else entirely.
    #[cfg(feature = "mcp-client")]
    fn bif_mcp_connect(&mut self, args: Vec<CfmlValue>) -> CfmlResult {
        let named = self.pending_ws_named.take();
        let kind = self
            .ws_arg(&args, &named, "transport", 0)
            .map(|v| v.as_string())
            .unwrap_or_default();
        // Every named argument is an option, so `mcpConnect( "http", url=... )`
        // and `mcpConnect( transport="http", url=... )` both work.
        let mut options = ValueMap::default();
        if let Some(pairs) = &named {
            for (k, v) in pairs {
                if !k.eq_ignore_ascii_case("transport") {
                    options.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(CfmlValue::Struct(s)) = self.ws_arg(&args, &named, "options", 1) {
            for (k, v) in s.iter() {
                options.insert(k.as_str().to_string(), v.clone());
            }
        }
        let client = mcp::client::connect_from_options(&kind, &options)
            .map_err(|e| self.wrap_error(CfmlError::runtime(e)))?;
        Ok(CfmlValue::NativeObject(std::sync::Arc::new(std::sync::RwLock::new(client))))
    }

    /// `mcpClient( name )` — connect to a server declared in `.cfconfig.json`
    /// under `mcpServers`, in the same shape an editor's client config uses.
    #[cfg(feature = "mcp-client")]
    fn bif_mcp_client(&mut self, args: Vec<CfmlValue>) -> CfmlResult {
        let name = args.first().map(|v| v.as_string()).unwrap_or_default();
        if name.is_empty() {
            return Err(self.wrap_error(CfmlError::runtime(
                "mcpClient() needs the name of a server declared in mcpServers".to_string(),
            )));
        }
        // In serve mode the resolved config lives on ServerState; a CLI run
        // has none, and then there are no named servers to find.
        let Some(cfconfig) = self.server_state.as_ref().map(|ss| ss.cfconfig.clone()) else {
            return Err(self.wrap_error(CfmlError::runtime(format!(
                "mcpClient: no configuration is loaded, so [{name}] cannot be resolved — \
                 use mcpConnect() with an explicit command or url"
            ))));
        };
        let config = crate::cfconfig_to_cfml(cfconfig.as_ref());
        let Some((kind, options)) = mcp::client::options_for_named_server(&config, &name) else {
            return Err(self.wrap_error(CfmlError::runtime(format!(
                "mcpClient: no server named [{name}] under `mcpServers` in the configuration"
            ))));
        };
        let client = mcp::client::connect_from_options(&kind, &options)
            .map_err(|e| self.wrap_error(CfmlError::runtime(e)))?;
        Ok(CfmlValue::NativeObject(std::sync::Arc::new(std::sync::RwLock::new(client))))
    }

    /// Without the `mcp-client` feature there is no transport to use, so the
    /// BIFs say so rather than appearing to exist and failing obscurely.
    #[cfg(not(feature = "mcp-client"))]
    fn bif_mcp_connect(&mut self, _args: Vec<CfmlValue>) -> CfmlResult {
        Err(self.wrap_error(CfmlError::runtime(
            "mcpConnect() needs the `mcp-client` feature, which this build does not include"
                .to_string(),
        )))
    }

    #[cfg(not(feature = "mcp-client"))]
    fn bif_mcp_client(&mut self, _args: Vec<CfmlValue>) -> CfmlResult {
        Err(self.wrap_error(CfmlError::runtime(
            "mcpClient() needs the `mcp-client` feature, which this build does not include"
                .to_string(),
        )))
    }

    /// `mcpSessions( server )` — the live session ids for a server, so a page
    /// can see who is connected (and target one with `mcpNotify( to= )`).
    fn bif_mcp_sessions(&mut self, args: Vec<CfmlValue>) -> CfmlResult {
        let server = args
            .first()
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .or_else(|| self.current_mcp_call.as_ref().map(|c| c.server.clone()))
            .unwrap_or_default();
        let ids = match self.server_state.as_ref() {
            Some(ss) => ss.mcp.session_ids(&server),
            None => Vec::new(),
        };
        Ok(CfmlValue::array(ids.into_iter().map(CfmlValue::string).collect()))
    }
}

/// The `mcpText`/`mcpImage`/`mcpAudio`/`mcpResource`/`mcpResourceLink`
/// builders, which shape an explicit content block when a tool wants exact
/// control over what the client renders.
fn content_builder(bif: &str, args: Vec<CfmlValue>) -> CfmlResult {
    let block = match bif {
        "mcptext" => mcp::content::block("text", fields(&[("text", args.first())])),
        "mcpimage" => mcp::content::block(
            "image",
            fields(&[("data", args.first()), ("mimeType", args.get(1))]),
        ),
        "mcpaudio" => mcp::content::block(
            "audio",
            fields(&[("data", args.first()), ("mimeType", args.get(1))]),
        ),
        "mcpresource" => {
            mcp::content::block("resource", fields(&[("resource", args.first())]))
        }
        "mcpresourcelink" => mcp::content::block(
            "resource_link",
            fields(&[
                ("uri", args.first()),
                ("name", args.get(1)),
                ("description", args.get(2)),
                ("mimeType", args.get(3)),
            ]),
        ),
        _ => return Err(intercepts_common::unhandled()),
    };
    Ok(block)
}

/// Build a content-block field map from positional args, skipping the ones the
/// caller omitted so an absent `mimeType` does not become a null key.
fn fields(pairs: &[(&str, Option<&CfmlValue>)]) -> ValueMap {
    let mut map = ValueMap::default();
    for (key, value) in pairs {
        if let Some(v) = value {
            if !matches!(v, CfmlValue::Null) {
                map.insert((*key).to_string(), (*v).clone());
            }
        }
    }
    map
}
