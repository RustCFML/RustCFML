//! Sync wasm activation for CFML VM execution.
//!
//! Why this exists: V8's JSPI (`WebAssembly.promising` / `Suspending`)
//! requires a *contiguous wasm stack* from the promising wrapper entry to
//! the Suspending import call. `#[event(fetch)]` from `worker-macros` is
//! async — `wasm-bindgen-futures` drives it via a poll loop, with the
//! original wasm fetch call returning a Promise handle to JS long before
//! the request work is done. Each await point on the Rust side is a
//! separate wasm activation. Suspending imports invoked from that context
//! have no promising wrapper above them on the wasm stack and the request
//! hangs.
//!
//! The workaround: keep `handle_fetch` async (so KV/DO/worker SDK calls
//! still work), but stage the VM execution into a *separate* sync wasm
//! export and invoke it from JS through `WebAssembly.promising`. That call
//! site is a fresh, contiguous wasm activation — JSPI can suspend it. The
//! `<cfquery>` driver inside the VM then works as the design intended.
//!
//! The end-to-end flow looks like:
//!
//!   1. async `handle_fetch` builds the `RunContext` (vfs, server state,
//!      globals, etc.) and stashes it in [`stash_context`].
//!   2. It invokes the JS-side `__cfml_invoke_run_sync`, which calls a
//!      `WebAssembly.promising(wasm.cfml_worker_run_sync)` wrapper installed
//!      by the post-build patch.
//!   3. Sync wasm pops the context, runs the VM. The VM hits `<cfquery>`,
//!      goes through the dynamic-driver registry to the `HyperdriveDriver`,
//!      which calls `jspi::hyperdrive_query_sync` → Suspending import.
//!      JSPI suspends the wasm stack, JS event loop drives postgres.js or
//!      mysql2 over the Hyperdrive socket, wasm resumes.
//!   4. The VM finishes, [`cfml_worker_run_sync`] writes the result into a
//!      thread-local.
//!   5. The JS Promise resolves, control returns to async `handle_fetch`,
//!      which pulls the result via [`take_result`] and builds the
//!      `worker::Response`.

#![cfg(target_arch = "wasm32")]

use crate::handler::{run_cfml, ResponseData};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::Vfs;
use cfml_vm::ServerState;
use std::cell::RefCell;
use std::sync::Arc;

pub struct RunContext {
    pub file_path: String,
    pub vfs: Arc<dyn Vfs>,
    pub extra_globals: ValueMap,
    pub http_request_data: CfmlValue,
    pub upload_registry: cfml_vm::web::UploadRegistry,
    pub server_state: ServerState,
    pub session_id: Option<String>,
}

thread_local! {
    static CTX: RefCell<Option<RunContext>> = const { RefCell::new(None) };
    static RESULT: RefCell<Option<Result<ResponseData, String>>> = const { RefCell::new(None) };
}

pub fn stash_context(ctx: RunContext) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

pub(crate) fn take_result() -> Option<Result<ResponseData, String>> {
    RESULT.with(|r| r.borrow_mut().take())
}

/// Sync wasm function. The post-build patch wraps the corresponding wasm
/// export in `WebAssembly.promising`. The async handler invokes that
/// wrapper from JS — only that invocation path gives JSPI a contiguous
/// wasm stack to suspend on.
///
/// Panics if no context has been staged. This is a programmer error; the
/// async caller must always [`stash_context`] before invoking.
pub fn cfml_worker_run_sync() {
    let ctx = CTX
        .with(|c| c.borrow_mut().take())
        .expect("cfml_worker_run_sync called without staged RunContext");

    let result = run_cfml(
        &ctx.file_path,
        ctx.vfs,
        ctx.extra_globals,
        ctx.http_request_data,
        ctx.upload_registry,
        &ctx.server_state,
        ctx.session_id,
    );

    RESULT.with(|r| *r.borrow_mut() = Some(result));
}
