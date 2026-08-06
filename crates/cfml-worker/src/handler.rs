//! Cloudflare Workers fetch handler.

#![cfg(target_arch = "wasm32")]

use crate::do_application_store::DoApplicationStore;
use crate::embedded_vfs::EmbeddedVfs;
use crate::kv_stores::{KvBackedApplicationStore, KvBackedSessionStore};
use crate::scopes;
use crate::WorkerConfig;
use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::Vfs;
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::web::resolve_file;
use cfml_vm::{ApplicationStore, CfmlVirtualMachine, MemoryApplicationStore, MemoryStore, ServerState, SessionStore};
use std::path::PathBuf;
use std::sync::Arc;
use worker::*;

/// Cloudflare Workers fetch handler entry point.
///
/// Expected to be invoked from the host project's `#[event(fetch)]` function:
///
/// ```ignore
/// #[event(fetch)]
/// pub async fn main(req: Request, env: Env, ctx: Context) -> Result<Response> {
///     let config = cfml_worker::WorkerConfig::new(CFML_FILES, "/app");
///     cfml_worker::handle_fetch(req, env, ctx, &config).await
/// }
/// ```
pub async fn handle_fetch(
    mut req: Request,
    _env: Env,
    _ctx: Context,
    config: &WorkerConfig,
) -> Result<Response> {
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedVfs::new(
        config.embedded_files,
        config.virtual_root.to_string(),
    ));

    let path = req.path();
    let doc_root = PathBuf::from(config.virtual_root);
    let welcome = if config.welcome_files.is_empty() {
        vec!["index.cfm".to_string()]
    } else {
        config.welcome_files.clone()
    };
    let cfml_exts = if config.cfml_extensions.is_empty() {
        vec!["cfm".to_string(), "cfc".to_string()]
    } else {
        config.cfml_extensions.clone()
    };

    let resolved = match resolve_file(&doc_root, &path, vfs.as_ref(), &welcome, &cfml_exts) {
        Some(r) => r,
        None => return Response::error("Not Found", 404),
    };

    let script_name = resolved.script_name.clone();
    let path_info = resolved.path_info.clone();
    let file_path = resolved.file_path.to_string_lossy().to_string();

    // Build CGI/URL/Form/Cookie scopes
    let (globals, http_request_data) =
        match scopes::build_from_request(&mut req, &script_name, &path_info).await {
            Ok(p) => p,
            Err(e) => return Response::error(format!("scope build error: {e:?}"), 500),
        };

    // Discover session id from the cookie scope. We DON'T mint a fresh
    // one here — that's deferred until after the VM runs, and only
    // happens if the VM actually created a session record. With
    // `this.lazySessionCreation = true`, a request that never touches
    // session scope produces no record and no `Set-Cookie`. With
    // default (eager) sessionManagement, the VM itself mints + creates
    // and we read it back from `vm.session_id` post-run.
    let cookie_struct = globals.get("cookie").cloned();
    let cookie_session_id = extract_session_id(&cookie_struct, &config.session_cookie_name);

    // Register Hyperdrive datasources for this request. The dynamic registry
    // is process-global; re-registering by name each request lets the JS
    // shim resolve the datasource name → binding on the current request's
    // env.
    for (name, binding) in &config.hyperdrive_datasources {
        cfml_stdlib::register_dynamic_datasource(
            name,
            Arc::new(crate::hyperdrive_driver::HyperdriveDriver::new(
                name.clone(),
                Arc::clone(binding),
            )),
        );
    }

    // Build ServerState with KV-backed stores when bindings are present.
    // Sessions get primed from KV before the VM runs; applications get
    // primed by name from `config.app_names`.
    let kv_session_store = config
        .kv_sessions
        .as_ref()
        .map(|kv| Arc::new(KvBackedSessionStore::new(kv.clone())));

    // Application scope: DO binding wins over KV, KV wins over memory.
    let do_app_store = config
        .do_application
        .as_ref()
        .map(|ns| Arc::new(DoApplicationStore::new(ns.clone())));
    let kv_app_store = if do_app_store.is_some() {
        None
    } else {
        config
            .kv_application
            .as_ref()
            .map(|kv| Arc::new(KvBackedApplicationStore::new(kv.clone())))
    };

    if let (Some(store), Some(sid)) = (kv_session_store.as_ref(), cookie_session_id.as_ref()) {
        let _ = store.prime(sid).await;
    }
    if let Some(store) = do_app_store.as_ref() {
        for name in &config.app_names {
            let _ = store.prime(name).await;
        }
    } else if let Some(store) = kv_app_store.as_ref() {
        for name in &config.app_names {
            let _ = store.prime(name).await;
        }
    }

    let mut server_state = ServerState::with_production(config.production_mode);
    server_state.sessions = match &kv_session_store {
        Some(s) => s.clone() as Arc<dyn SessionStore>,
        None => Arc::new(MemoryStore::new()),
    };
    server_state.applications = if let Some(a) = &do_app_store {
        a.clone() as Arc<dyn ApplicationStore>
    } else if let Some(a) = &kv_app_store {
        a.clone() as Arc<dyn ApplicationStore>
    } else {
        Arc::new(MemoryApplicationStore::new())
    };

    // Execute the VM inside a separate sync wasm activation wrapped in
    // `WebAssembly.promising`. The async outer fetch can't host the JSPI
    // suspends directly (wasm-bindgen-futures breaks contiguity); the sync
    // runner gives JSPI a clean stack to suspend on for `<cfquery>`.
    crate::sync_runner::stash_context(crate::sync_runner::RunContext {
        file_path: file_path.clone(),
        vfs,
        extra_globals: globals,
        http_request_data,
        server_state: server_state.clone(),
        session_id: cookie_session_id.clone(),
    });
    if let Err(msg) = crate::jspi::invoke_run_sync().await {
        let headers = Headers::new();
        let _ = headers.set("Content-Type", "text/plain; charset=utf-8");
        return Ok(Response::ok(format!("CFML dispatch error:\n\n{msg}"))?
            .with_status(500)
            .with_headers(headers));
    }
    let response_data = match crate::sync_runner::take_result() {
        Some(Ok(r)) => r,
        Some(Err(msg)) => {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "text/plain; charset=utf-8");
            return Ok(Response::ok(format!("CFML error:\n\n{msg}"))?
                .with_status(500)
                .with_headers(headers));
        }
        None => {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "text/plain; charset=utf-8");
            return Ok(Response::ok("CFML error: sync runner produced no result")?
                .with_status(500)
                .with_headers(headers));
        }
    };

    // Map response
    if let Some(redirect) = response_data.redirect_url {
        return Response::redirect(redirect.parse().map_err(|_| Error::BadEncoding)?);
    }

    let status = response_data.status.unwrap_or(200);
    // Resolve the singleton Content-Type and de-duplicate other singleton
    // headers so a cfheader(name="Content-Type") (or Content-Length/Location)
    // replaces rather than appends to the engine default (issue #148).
    let (content_type, emit_headers) = cfml_vm::web::resolve_response_headers(
        response_data.content_type.as_deref(),
        &response_data.headers,
    );

    let headers = Headers::new();
    headers.set("Content-Type", &content_type)?;
    for (k, v) in emit_headers {
        headers.append(&k, &v)?;
    }

    // Emit Set-Cookie only if the VM actually created a session record
    // AND the incoming request had no cookie for us to echo. In lazy
    // mode this means pages that never touched session pay no cookie
    // round-trip overhead.
    if response_data.session_record_created && cookie_session_id.is_none() {
        if let Some(sid) = &response_data.session_id {
            // Workers are HTTPS end-to-end, so the connection is always secure:
            // the auto-`Secure` default resolves to on unless the app set
            // `this.sessioncookie.secure = false` explicitly.
            let cookie = response_data.session_cookie_policy.render(
                &config.session_cookie_name,
                sid,
                true,
            );
            headers.append("Set-Cookie", &cookie)?;
        }
    }

    // Flush dirty state back to KV/DO after the response is built. These are
    // awaited inline (not `ctx.wait_until`): deferred writes scheduled after
    // the JSPI promising sync activation were silently dropped on requests
    // that had already done an awaited KV read (`prime`), so existing-session
    // and application-scope mutations never persisted. See
    // `KvBackedSessionStore::flush`.
    if let Some(store) = kv_session_store.as_ref() {
        if let Err(e) = store.flush().await {
            worker::console_error!("session store flush failed: {e:?}");
        }
    }
    if let Some(store) = do_app_store.as_ref() {
        if let Err(e) = store.flush().await {
            worker::console_error!("application (DO) store flush failed: {e:?}");
        }
    } else if let Some(store) = kv_app_store.as_ref() {
        if let Err(e) = store.flush().await {
            worker::console_error!("application (KV) store flush failed: {e:?}");
        }
    }

    Ok(Response::ok(response_data.output)?
        .with_status(status)
        .with_headers(headers))
}

fn extract_session_id(
    cookie_struct: &Option<CfmlValue>,
    cookie_name: &str,
) -> Option<String> {
    let CfmlValue::Struct(map) = cookie_struct.as_ref()? else {
        return None;
    };
    for (k, v) in map.iter() {
        if k.eq_ignore_ascii_case(cookie_name) {
            let s = v.as_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

pub(crate) struct ResponseData {
    pub(crate) output: String,
    pub(crate) status: Option<u16>,
    pub(crate) content_type: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) redirect_url: Option<String>,
    /// The session id the VM ended up using — either the cookie value
    /// the request came in with, or one minted by the VM during the
    /// session-lifecycle phase (eager mode) or first session write
    /// (lazy mode).
    pub(crate) session_id: Option<String>,
    /// Set if the VM actually inserted a session record this request.
    /// The handler uses this to decide whether to emit `Set-Cookie`.
    pub(crate) session_record_created: bool,
    /// Resolved `this.sessioncookie` attributes for rendering the session
    /// `Set-Cookie` header.
    pub(crate) session_cookie_policy: cfml_common::session_cookie::SessionCookiePolicy,
}

pub(crate) fn run_cfml(
    file_path: &str,
    vfs: Arc<dyn Vfs>,
    extra_globals: ValueMap,
    http_request_data: CfmlValue,
    server_state: &ServerState,
    session_id: Option<String>,
) -> std::result::Result<ResponseData, String> {
    // Read source (compile path uses VFS via compile_file_cached, but we
    // also want the source for the first-compile path when the file isn't
    // yet cached).
    let source = vfs
        .read_to_string(file_path)
        .map_err(|e| format!("read {file_path}: {e}"))?;

    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };

    let ast = Parser::new(processed)
        .parse()
        .map_err(|e| format!("parse error [{}:{}] {}", e.line, e.column, e.message))?;

    let program = CfmlCompiler::new().compile(ast);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.base_template_path = Some(file_path.to_string());
    vm.source_file = Some(file_path.to_string());

    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.refresh_builtin_index();

    // Wire queryExecute to the dynamic-driver-only variant. The worker
    // build of cfml-stdlib intentionally compiles without any per-engine
    // DB feature (sqlite/mysql/etc); all queries go through the
    // dynamic-driver registry that the Hyperdrive driver registered above.
    vm.query_execute_fn = Some(cfml_stdlib::builtins::fn_query_execute_dynamic);

    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("cgi".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals
        .entry("form".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    for (name, value) in extra_globals {
        vm.globals.insert(name, value);
    }

    vm.apply_cfconfig(&server_state.cfconfig);
    vm.server_state = Some(server_state.clone());
    vm.http_request_data = Some(http_request_data);
    vm.session_id = session_id;

    let result = vm.execute_with_lifecycle();
    let result = match result {
        Err(e) if e.message == "__cflocation_redirect" || e.message == "__cfabort" => {
            Ok(CfmlValue::Null)
        }
        other => other,
    };

    // Flush buffered <cfhtmlhead>/<cfhtmlbody> content into the response body.
    vm.finalize_html_injections();

    let output = vm.output_buffer.clone();
    let status = vm.response_status.map(|(c, _)| c);
    let content_type = vm.response_content_type.clone();
    let headers = vm.response_headers.clone();
    let redirect_url = vm.redirect_url.clone();
    let session_id = vm.session_id.clone();
    let session_record_created = vm.session_record_created;
    let session_cookie_policy = vm.session_cookie_policy.clone();

    match result {
        Ok(_) => Ok(ResponseData {
            output,
            status,
            content_type,
            headers,
            redirect_url,
            session_id,
            session_record_created,
            session_cookie_policy,
        }),
        Err(e) => Err(format!("{}\n\nOutput so far:\n{}", e, output)),
    }
}
