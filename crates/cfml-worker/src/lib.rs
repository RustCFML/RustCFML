//! Cloudflare Workers host glue for RustCFML.
//!
//! A Worker project depends on this crate, supplies its embedded CFML files
//! plus any KV/Hyperdrive bindings, and calls [`handle_fetch`] from its
//! `#[event(fetch)]` entry. The rest of the request lifecycle — routing,
//! cgi/url/form/cookie scope population, Application.cfc onRequestStart /
//! onRequest / onRequestEnd / onApplicationStart / onSessionStart, bytecode
//! caching, and response building — happens here.
//!
//! Native targets (`cfg(not(target_arch = "wasm32"))`) only get the public
//! [`WorkerConfig`] struct so the crate still participates in workspace
//! `cargo build` / `cargo check`. The fetch handler itself is wasm32-only.

#![allow(clippy::needless_lifetimes)]

pub mod embedded_vfs;
pub mod scopes;

#[cfg(target_arch = "wasm32")]
pub mod hyperdrive_driver;

/// `<cfhttp>` over the Workers `fetch` API. The native ureq client has no
/// wasm32 target, so the platform fetch is the transport here.
pub mod http_provider;
#[cfg(target_arch = "wasm32")]
pub mod handler;
#[cfg(target_arch = "wasm32")]
pub mod jspi;
#[cfg(target_arch = "wasm32")]
pub mod jspi_smoke;
#[cfg(target_arch = "wasm32")]
pub mod sync_runner;
#[cfg(target_arch = "wasm32")]
pub mod kv_stores;
#[cfg(target_arch = "wasm32")]
pub mod do_application_store;
#[cfg(target_arch = "wasm32")]
pub mod scheduled;
#[cfg(target_arch = "wasm32")]
pub use hyperdrive_driver::HyperdriveDriver;
#[cfg(target_arch = "wasm32")]
pub use do_application_store::DoApplicationStore;
#[cfg(target_arch = "wasm32")]
pub use handler::handle_fetch;
#[cfg(target_arch = "wasm32")]
pub use kv_stores::{KvBackedApplicationStore, KvBackedSessionStore};
#[cfg(target_arch = "wasm32")]
pub use scheduled::handle_scheduled;

#[cfg(target_arch = "wasm32")]
use worker::kv::KvStore;

/// Configuration passed to [`handle_fetch`] on every request.
///
/// Build it once at the top of your `fetch` handler from environment
/// bindings — `worker::Env::kv("FOO")`, `worker::Env::hyperdrive("BAR")`, etc.
pub struct WorkerConfig {
    /// Embedded CFML file table (path → bytes), typically produced by your
    /// `build.rs`.
    pub embedded_files: &'static [(&'static str, &'static [u8])],

    /// Virtual root the VM sees as its filesystem root. Anything under
    /// `<virtual_root>/...` is resolved through the embedded VFS.
    pub virtual_root: &'static str,

    /// Welcome files to try when the URL resolves to a directory. Defaults
    /// to `["index.cfm"]` if you leave it empty.
    pub welcome_files: Vec<String>,

    /// File extensions treated as CFML for path-info matching
    /// (`/foo.cfm/bar/baz`). Defaults to `["cfm", "cfc"]` when empty.
    pub cfml_extensions: Vec<String>,

    /// Optional KV namespace for session storage. When `None`, sessions live
    /// in the per-isolate `MemoryStore` (lost when the isolate recycles).
    #[cfg(target_arch = "wasm32")]
    pub kv_sessions: Option<KvStore>,

    /// Optional KV namespace for application scope. When `None`, application
    /// scope lives in the per-isolate `MemoryApplicationStore` (and
    /// onApplicationStart may fire on each new isolate). Ignored when
    /// `do_application_binding` is set.
    #[cfg(target_arch = "wasm32")]
    pub kv_application: Option<KvStore>,

    /// Optional Durable Object namespace handle that backs application
    /// scope. When set, takes priority over `kv_application` and provides
    /// strong consistency across isolates.
    ///
    /// The host project gets this via `env.durable_object("APP_DO")?`
    /// in its `#[event(fetch)]` and passes the resulting `ObjectNamespace`.
    /// The DO class must implement `GET /get` and `POST /put` — see
    /// `DoApplicationStore` docs for the wire shape.
    #[cfg(target_arch = "wasm32")]
    pub do_application: Option<worker::ObjectNamespace>,

    /// Named Hyperdrive datasources to register before each request. The
    /// string is the cfquery `datasource="..."` name; the JS shim looks the
    /// binding up by the same name on `env` at query time.
    ///
    /// `worker::Hyperdrive` is not `Clone`, so callers wrap their binding in
    /// an `Arc` once and pass the shared handle. The Rust side holds the
    /// binding alive for the duration of the request; the actual SQL is
    /// dispatched through the JSPI shim (postgres.js / mysql2).
    #[cfg(target_arch = "wasm32")]
    pub hyperdrive_datasources:
        Vec<(String, std::sync::Arc<worker::Hyperdrive>)>,

    /// Production mode toggles bytecode cache invalidation off (cache trusts
    /// mtime stamps, never re-checks). Default `true` for Workers since
    /// embedded files don't change mid-isolate.
    pub production_mode: bool,

    /// Names of CFML applications (Application.cfc `this.name`) that should
    /// be primed from `kv_application` at the start of each request. Without
    /// this list, application scope only persists within a single isolate.
    pub app_names: Vec<String>,

    /// Cookie name to read/write the session id from. Defaults to `"CFID"`.
    pub session_cookie_name: String,
}

impl WorkerConfig {
    /// Minimal config for the common case: just embedded files + virtual root.
    pub fn new(
        embedded_files: &'static [(&'static str, &'static [u8])],
        virtual_root: &'static str,
    ) -> Self {
        Self {
            embedded_files,
            virtual_root,
            welcome_files: vec!["index.cfm".into()],
            cfml_extensions: vec!["cfm".into(), "cfc".into()],
            #[cfg(target_arch = "wasm32")]
            kv_sessions: None,
            #[cfg(target_arch = "wasm32")]
            kv_application: None,
            #[cfg(target_arch = "wasm32")]
            do_application: None,
            #[cfg(target_arch = "wasm32")]
            hyperdrive_datasources: Vec::new(),
            production_mode: true,
            app_names: Vec::new(),
            session_cookie_name: "CFID".into(),
        }
    }
}
