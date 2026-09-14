//! Bridging MCP requests to the (synchronous) VM.
//!
//! Same recipe as the WebSocket driver: every call builds a fresh VM on a
//! blocking worker, instantiates the server CFC and invokes one method. A
//! fresh VM per call means a tool carries no state between invocations except
//! through `application`/`server` scope — identical to the contract a
//! WebSocket handler has, and the reason a tool is safe to run concurrently.

use std::sync::Arc;

use cfml_common::dynamic::{CfmlStruct, CfmlValue, ValueMap};
use cfml_vm::mcp::{CallContext, Entity, ServerManifest, UNAUTHORIZED_TYPE};
use cfml_vm::CfmlVirtualMachine;

use super::ServerInfo;

/// The slice of `AppState` a dispatch needs, detached so stdio mode — which
/// has no HTTP server and therefore no `AppState` — can build one too.
#[derive(Clone)]
pub(crate) struct McpRuntime {
    pub(crate) server_state: cfml_vm::ServerState,
    pub(crate) vfs: Arc<dyn cfml_common::vfs::Vfs>,
    pub(crate) sandbox: bool,
}

impl McpRuntime {
    fn vm(&self, cfc_path: &str) -> CfmlVirtualMachine {
        let empty = crate::CfmlCompiler::new().compile(
            crate::CfmlParser::new(String::new()).parse().expect("empty source parses"),
        );
        let mut vm = CfmlVirtualMachine::new(empty);
        vm.vfs = self.vfs.clone();
        vm.sandbox = self.sandbox;
        crate::register_vm_runtime(&mut vm);
        vm.apply_cfconfig(&self.server_state.cfconfig);
        vm.source_file = Some(Arc::from(cfc_path));
        vm.server_state = Some(self.server_state.clone());
        vm
    }
}

/// Reflect the server CFC's tools/resources/prompts.
///
/// Runs on a blocking worker because compiling and instantiating the component
/// is VM work. Cheap enough to do per `tools/list`; a (path, mtime)-keyed cache
/// is the obvious next step if a large server makes it show up in a profile.
pub(crate) async fn manifest(
    runtime: McpRuntime,
    info: ServerInfo,
) -> Result<ServerManifest, String> {
    tokio::task::spawn_blocking(move || {
        let mut vm = runtime.vm(&info.cfc_path);
        vm.mcp_manifest(&info.cfc_path, &info.name).map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(format!("MCP manifest task panicked: {e}")))
}

/// Why a handler call failed.
///
/// The message is the exception's own text and **never** its stack trace:
/// a trace names CFC paths and line numbers, and everything here is on its way
/// to a remote client.
pub(crate) struct CallFailure {
    pub(crate) message: String,
    /// True when the `secured` gate refused the call, which the engine reports
    /// as a JSON-RPC error rather than a tool result.
    pub(crate) denied: bool,
}

/// Invoke one tool/resource/prompt handler.
pub(crate) async fn call(
    runtime: McpRuntime,
    info: ServerInfo,
    entity: Entity,
    arguments: ValueMap,
    ctx: CallContext,
    identity: Option<CfmlValue>,
) -> Result<CfmlValue, CallFailure> {
    tokio::task::spawn_blocking(move || {
        let mut vm = runtime.vm(&info.cfc_path);
        vm.current_mcp_identity = identity;
        let args: CfmlStruct = match CfmlValue::strukt(arguments) {
            CfmlValue::Struct(s) => s,
            _ => unreachable!("strukt always yields a Struct"),
        };
        vm.dispatch_mcp(&info.cfc_path, &entity, args, ctx).map_err(|e| CallFailure {
            denied: matches!(
                &e.error_type,
                cfml_common::vm::CfmlErrorType::Custom(t) if t == UNAUTHORIZED_TYPE
            ),
            message: e.message.clone(),
        })
    })
    .await
    .unwrap_or_else(|e| {
        Err(CallFailure { message: format!("MCP dispatch task panicked: {e}"), denied: false })
    })
}
