//! CFML Virtual Machine - Bytecode execution engine

use cfml_codegen::{BytecodeFunction, BytecodeOp, BytecodeProgram, CmpOp};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{RealFs, Vfs};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

pub mod application_store;
mod java_shims;
#[cfg(feature = "s3")]
mod s3_vfs;
pub mod session_store;
pub mod web;
pub use application_store::{ApplicationStore, MemoryApplicationStore};
pub use session_store::{MemoryStore, SessionStore};
use java_shims::{
    handle_java_collections, handle_java_concurrenthashmap, handle_java_concurrentlinkedqueue,
    handle_java_file, handle_java_inetaddress, handle_java_linkedhashmap,
    handle_java_messagedigest, handle_java_paths, handle_java_stringbuilder, handle_java_system,
    handle_java_thread, handle_java_treemap, handle_java_uuid,
};

pub type BuiltinFunction = fn(Vec<CfmlValue>) -> CfmlResult;

/// Persistent application state, keyed by app name.
#[derive(Clone)]
pub struct ApplicationState {
    pub name: String,
    pub variables: IndexMap<String, CfmlValue>,
    pub started: bool,
    pub config: IndexMap<String, CfmlValue>,
    /// Bytecode functions added during onApplicationStart (factory, resources, etc.).
    /// Only the delta (functions added after load_application_cfc) is cached.
    pub cached_functions: Vec<std::sync::Arc<cfml_codegen::compiler::BytecodeFunction>>,
    /// The program.functions.len() at which cached_functions were originally inserted.
    /// Used to compute index offsets when restoring on subsequent requests.
    pub cached_functions_original_offset: usize,
    /// Value of `this.sessionStorage` from Application.cfc — name of the cache to use
    /// for session storage. Overrides the server-wide `.cfconfig.json` setting.
    pub session_storage: Option<String>,
    /// Named cache definitions from `this.cache` in Application.cfc.
    /// These merge with (and override) the server-wide `caches` from `.cfconfig.json`.
    pub app_caches: indexmap::IndexMap<String, cfml_config::CacheCfg>,
}

/// A CFML component mapping: virtual prefix → physical directory.
#[derive(Debug, Clone)]
pub struct CfmlMapping {
    pub name: String, // Normalized: leading+trailing "/" e.g. "/taffy/"
    pub path: String, // Absolute filesystem directory
}

/// Session data for a single user session.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionData {
    pub variables: IndexMap<String, CfmlValue>,
    /// Unix epoch seconds when the session was created.
    pub created_secs: u64,
    /// Unix epoch seconds of the most recent access.
    pub last_accessed_secs: u64,
    pub auth_user: Option<String>,
    pub auth_roles: Vec<String>,
    /// Session timeout in seconds (default 1800 = 30 minutes)
    pub timeout_secs: u64,
}

/// Returns current Unix epoch seconds.
#[inline]
fn now_epoch_secs() -> u64 {
    cfml_common::clock::now_unix_secs()
}

/// A cached compiled bytecode program with its source file modification time.
pub struct CachedProgram {
    pub program: BytecodeProgram,
    pub mtime: SystemTime,
}

/// Thread-safe bytecode cache keyed by file path.
/// Skips recompilation when a file's mtime is unchanged.
#[derive(Clone)]
pub struct BytecodeCache {
    entries: Arc<parking_lot::RwLock<HashMap<String, CachedProgram>>>,
    /// When true, skip the per-hit `vfs.modified()` stat and always trust
    /// cached entries. Set from the `--production` flag.
    trusted: bool,
}

impl BytecodeCache {
    pub fn new() -> Self {
        Self::with_trust(false)
    }

    pub fn with_trust(trusted: bool) -> Self {
        Self {
            entries: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            trusted,
        }
    }

    /// Return a cached program if present. In trusted (production) mode the
    /// file's mtime is not re-checked; otherwise the entry is only returned
    /// when the on-disk mtime still matches.
    pub fn get(&self, path: &str, vfs: &dyn Vfs) -> Option<BytecodeProgram> {
        if self.trusted {
            let entries = self.entries.read();
            return entries.get(path).map(|e| e.program.clone());
        }
        let mtime = vfs.modified(path).ok()?;
        let entries = self.entries.read();
        let entry = entries.get(path)?;
        if entry.mtime == mtime {
            Some(entry.program.clone())
        } else {
            None
        }
    }

    /// Insert a freshly compiled program into the cache.
    pub fn insert(&self, path: String, program: BytecodeProgram, mtime: SystemTime) {
        self.entries
            .write()
            .insert(path, CachedProgram { program, mtime });
    }
}

/// Build the `server` scope struct. Populates Lucee-compatible keys so that
/// migration code reading `server.system.environment.FOO`,
/// `server.system.properties["os.name"]`, `server.separator.file`, etc. works
/// out of the box. Snapshotted on each access; cheap because env vars and
/// args are small.
/// Convert a parsed `.cfconfig.json` into a CFML-visible read-only struct.
/// Goes via `serde_json::Value` so we get a single converter for all the
/// schema's nested types. Returns an empty struct on serialise failure
/// (should never happen — every field has Serialize).
pub(crate) fn cfconfig_to_cfml(cfg: &cfml_config::RustCfmlConfig) -> CfmlValue {
    match serde_json::to_value(cfg) {
        Ok(v) => json_value_to_cfml(v),
        Err(_) => CfmlValue::strukt(IndexMap::new()),
    }
}

fn json_value_to_cfml(value: serde_json::Value) -> CfmlValue {
    match value {
        serde_json::Value::Null => CfmlValue::Null,
        serde_json::Value::Bool(b) => CfmlValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CfmlValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                CfmlValue::Double(f)
            } else {
                CfmlValue::Int(0)
            }
        }
        serde_json::Value::String(s) => CfmlValue::String(s),
        serde_json::Value::Array(arr) => {
            CfmlValue::array(arr.into_iter().map(json_value_to_cfml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = IndexMap::new();
            for (k, v) in obj {
                map.insert(k, json_value_to_cfml(v));
            }
            CfmlValue::strukt(map)
        }
    }
}

fn build_server_scope() -> IndexMap<String, CfmlValue> {
    let mut info = IndexMap::new();

    let mut cf = IndexMap::new();
    cf.insert(
        "productname".to_string(),
        CfmlValue::String("RustCFML".to_string()),
    );
    cf.insert(
        "productversion".to_string(),
        CfmlValue::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    cf.insert(
        "productlevel".to_string(),
        CfmlValue::String("Final".to_string()),
    );
    info.insert("coldfusion".to_string(), CfmlValue::strukt(cf));

    let mut os = IndexMap::new();
    os.insert(
        "name".to_string(),
        CfmlValue::String(std::env::consts::OS.to_string()),
    );
    os.insert(
        "arch".to_string(),
        CfmlValue::String(std::env::consts::ARCH.to_string()),
    );
    if let Ok(host) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
        os.insert("hostname".to_string(), CfmlValue::String(host));
    }
    info.insert("os".to_string(), CfmlValue::strukt(os));

    let file_sep = std::path::MAIN_SEPARATOR.to_string();
    let path_sep = if cfg!(windows) { ";" } else { ":" }.to_string();
    let line_sep = if cfg!(windows) { "\r\n" } else { "\n" }.to_string();

    let mut sep = IndexMap::new();
    sep.insert("file".to_string(), CfmlValue::String(file_sep.clone()));
    sep.insert("path".to_string(), CfmlValue::String(path_sep.clone()));
    sep.insert("line".to_string(), CfmlValue::String(line_sep.clone()));
    info.insert("separator".to_string(), CfmlValue::strukt(sep));

    let mut java = IndexMap::new();
    java.insert(
        "version".to_string(),
        CfmlValue::String(String::new()),
    );
    java.insert(
        "vendor".to_string(),
        CfmlValue::String("RustCFML (no JVM)".to_string()),
    );
    java.insert(
        "archModel".to_string(),
        CfmlValue::String(
            if cfg!(target_pointer_width = "64") { "64" } else { "32" }.to_string(),
        ),
    );
    info.insert("java".to_string(), CfmlValue::strukt(java));

    let mut system = IndexMap::new();

    let mut env = IndexMap::new();
    for (k, v) in std::env::vars() {
        env.insert(k, CfmlValue::String(v));
    }
    system.insert("environment".to_string(), CfmlValue::strukt(env));

    let mut props = IndexMap::new();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let tmp = std::env::temp_dir().to_string_lossy().to_string();
    props.insert(
        "os.name".to_string(),
        CfmlValue::String(std::env::consts::OS.to_string()),
    );
    props.insert(
        "os.arch".to_string(),
        CfmlValue::String(std::env::consts::ARCH.to_string()),
    );
    props.insert("user.dir".to_string(), CfmlValue::String(cwd));
    props.insert("user.home".to_string(), CfmlValue::String(home));
    props.insert("user.name".to_string(), CfmlValue::String(user));
    props.insert("java.io.tmpdir".to_string(), CfmlValue::String(tmp));
    props.insert("file.separator".to_string(), CfmlValue::String(file_sep));
    props.insert("path.separator".to_string(), CfmlValue::String(path_sep));
    props.insert("line.separator".to_string(), CfmlValue::String(line_sep));
    props.insert(
        "file.encoding".to_string(),
        CfmlValue::String("UTF-8".to_string()),
    );
    system.insert("properties".to_string(), CfmlValue::strukt(props));

    let args: Vec<CfmlValue> = std::env::args()
        .skip(1)
        .map(CfmlValue::String)
        .collect();
    system.insert("arguments".to_string(), CfmlValue::Array(args.into()));

    info.insert("system".to_string(), CfmlValue::strukt(system));

    info
}

/// Lexically normalise a path: collapse `.` and `..` segments without
/// touching the filesystem. Used by `<cfinclude>` so that
/// `examples/api_empty/../dashboard/dashboard.cfm` resolves to
/// `dashboard/dashboard.cfm`. Unlike `std::fs::canonicalize` this works on
/// non-existent paths and does not resolve symlinks.
fn normalize_path(path: &str) -> String {
    use std::path::{Component, Path};
    let p = Path::new(path);
    let mut out: Vec<String> = Vec::new();
    let mut leading_root: Option<String> = None;
    let mut leading_prefix: Option<String> = None;
    for comp in p.components() {
        match comp {
            Component::Prefix(pref) => {
                leading_prefix = Some(pref.as_os_str().to_string_lossy().to_string());
            }
            Component::RootDir => {
                leading_root = Some(std::path::MAIN_SEPARATOR.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last().map(String::as_str), Some(s) if s != "..") {
                    out.pop();
                } else if leading_root.is_none() {
                    out.push("..".to_string());
                }
            }
            Component::Normal(seg) => {
                out.push(seg.to_string_lossy().to_string());
            }
        }
    }
    let mut result = String::new();
    if let Some(prefix) = leading_prefix {
        result.push_str(&prefix);
    }
    if let Some(root) = leading_root {
        result.push_str(&root);
    }
    result.push_str(&out.join(&std::path::MAIN_SEPARATOR.to_string()));
    if result.is_empty() {
        ".".to_string()
    } else {
        result
    }
}

/// Compile a CFML file to bytecode, using the cache when available.
/// When `cache` is None (CLI mode), always compiles fresh.
/// Reads source from the provided VFS (real filesystem or embedded).
pub fn compile_file_cached(
    path: &str,
    cache: Option<&BytecodeCache>,
    vfs: &dyn Vfs,
) -> Result<BytecodeProgram, CfmlError> {
    // Check cache first
    if let Some(c) = cache {
        if let Some(program) = c.get(path, vfs) {
            return Ok(program);
        }
    }

    // Read source via VFS
    let source_code = vfs
        .read_to_string(path)
        .map_err(|e| CfmlError::runtime(format!("Cannot read '{}': {}", path, e)))?;

    // Tag preprocessing. Run when the file looks like a template — either it
    // contains CFML tags, or its extension marks it as static markup that
    // cfinclude is expected to splice in verbatim (.css, .html, .htm, .txt).
    // Without this, e.g. `<cfinclude template="dashboard.css">` parses raw CSS
    // (`body { background-color: #f3f4f6; }`) as CFML script and explodes on
    // the first hex color. Skip preprocessing for .cfc files that don't have
    // CFML tags — those are script-only components (`component { ... }`).
    let lower_path = path.to_lowercase();
    let is_template_ext = lower_path.ends_with(".cfm")
        || lower_path.ends_with(".css")
        || lower_path.ends_with(".html")
        || lower_path.ends_with(".htm")
        || lower_path.ends_with(".txt");
    let needs_tag_parse =
        cfml_compiler::tag_parser::has_cfml_tags(&source_code) || is_template_ext;
    let source_code = if needs_tag_parse {
        let converted = cfml_compiler::tag_parser::tags_to_script(&source_code);
        if std::env::var("RUSTCFML_DUMP_TAGS").is_ok() {
            eprintln!(
                "=== TAG CONVERTED: {} ===\n{}\n=== END ===",
                path, converted
            );
        }
        converted
    } else {
        source_code
    };

    // Parse
    let ast = cfml_compiler::parser::Parser::new(source_code)
        .parse()
        .map_err(|e| {
            CfmlError::runtime(format!(
                "Parse error in '{}' [line {}, col {}]: {}",
                path, e.line, e.column, e.message
            ))
        })?;

    // Compile
    let compiler = cfml_codegen::compiler::CfmlCompiler::new();
    let program = compiler.compile(ast);

    // Cache the result
    if let Some(c) = cache {
        if let Ok(mtime) = vfs.modified(path) {
            c.insert(path.to_string(), program.clone(), mtime);
        }
    }

    Ok(program)
}

/// Server-level state, persists across requests in --serve mode.
#[derive(Clone)]
pub struct ServerState {
    pub applications: Arc<dyn ApplicationStore>,
    pub sessions: Arc<dyn SessionStore>,
    /// Named locks for cflock: name → RwLock (exclusive = write, readonly = read)
    pub named_locks: Arc<Mutex<HashMap<String, Arc<RwLock<()>>>>>,
    /// Bytecode cache — skips recompilation when file mtime is unchanged
    pub bytecode_cache: BytecodeCache,
    /// Document root for `--serve` mode. Used as a fallback search path
    /// when resolving dotted CFC paths (e.g. `taffy.core.api`) for files
    /// that live outside the Application.cfc directory.
    pub webroot: Option<std::path::PathBuf>,
    /// When true, in-memory caches (bytecode, Application.cfc path, resolved
    /// URLs) are never invalidated until server restart. Set by `--production`.
    pub production_mode: bool,
    /// Cache of Application.cfc path resolution keyed by the page's parent
    /// directory. Only populated when `production_mode` is true.
    pub app_cfc_path_cache: Arc<parking_lot::RwLock<HashMap<std::path::PathBuf, Option<String>>>>,
    /// Resolved `.cfconfig.json` (or defaults if no file). Wraps in `Arc` so
    /// every cloned ServerState shares the same struct without re-parsing.
    pub cfconfig: Arc<cfml_config::RustCfmlConfig>,
}

impl ServerState {
    pub fn new() -> Self {
        Self::with_production(false)
    }

    pub fn with_production(production_mode: bool) -> Self {
        Self::with_config(production_mode, Arc::new(cfml_config::RustCfmlConfig::default()))
    }

    pub fn with_config(
        production_mode: bool,
        cfconfig: Arc<cfml_config::RustCfmlConfig>,
    ) -> Self {
        Self {
            applications: Arc::new(MemoryApplicationStore::new()),
            sessions: Arc::new(MemoryStore::new()),
            named_locks: Arc::new(Mutex::new(HashMap::new())),
            bytecode_cache: BytecodeCache::with_trust(production_mode),
            webroot: None,
            production_mode,
            app_cfc_path_cache: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            cfconfig,
        }
    }
}

/// A held lock guard for cflock (keeps the lock alive during the block).
/// The guard fields are never read directly — they are held for their Drop behavior.
#[allow(dead_code)]
enum HeldLock {
    Write(std::sync::RwLockWriteGuard<'static, ()>),
    Read(std::sync::RwLockReadGuard<'static, ()>),
}

pub struct CfmlVirtualMachine {
    pub program: BytecodeProgram,
    pub globals: IndexMap<String, CfmlValue>,
    pub builtins: HashMap<String, BuiltinFunction>,
    pub output_buffer: String,
    /// Virtual filesystem for source file I/O (real disk or embedded archive)
    pub vfs: Arc<dyn Vfs>,
    /// User-defined functions (name -> function definition)
    /// Held as `Arc<BytecodeFunction>` so that cloning (very hot on every call)
    /// is a refcount bump rather than a deep clone of the whole bytecode body.
    pub user_functions: HashMap<String, Arc<BytecodeFunction>>,
    /// Source file path (for include resolution)
    pub source_file: Option<String>,
    /// Call stack for tracking execution
    call_stack: Vec<CallFrame>,
    /// Try-catch handler stack
    try_stack: Vec<TryHandler>,
    /// Current exception (if any)
    #[allow(dead_code)]
    current_exception: Option<CfmlValue>,
    /// Last thrown exception (for rethrow support)
    last_exception: Option<CfmlValue>,
    /// Current source line being executed (updated by LineInfo op)
    current_line: usize,
    /// Current source column
    current_column: usize,
    /// After a component method executes, holds the modified `this` for write-back
    /// to the caller's object variable. Set by execute_function_with_args.
    method_this_writeback: Option<CfmlValue>,
    /// After a component method executes, holds modified variables scope entries for
    /// write-back to the component's __variables. Enables `variables.x = y` to persist.
    method_variables_writeback: Option<IndexMap<String, CfmlValue>>,
    /// After a closure executes, holds modified parent-scope variables for write-back
    /// to the caller's locals. Enables closures to mutate parent scope.
    closure_parent_writeback: Option<IndexMap<String, CfmlValue>>,
    /// Request scope — lives for the duration of one request
    pub request_scope: IndexMap<String, CfmlValue>,
    /// Application scope — shared across requests (Arc<Mutex> for thread safety)
    pub application_scope: Option<Arc<Mutex<IndexMap<String, CfmlValue>>>>,
    /// Server-level state — persists across requests in --serve mode
    pub server_state: Option<ServerState>,
    /// HTTP response headers set by cfheader
    pub response_headers: Vec<(String, String)>,
    /// HTTP response status code set by cfheader
    pub response_status: Option<(u16, String)>,
    /// Content type set by cfcontent
    pub response_content_type: Option<String>,
    /// Body override set by cfcontent (variable/file)
    pub response_body: Option<CfmlValue>,
    /// Redirect URL set by cflocation
    pub redirect_url: Option<String>,
    /// HTTP request data for getHTTPRequestData()
    pub http_request_data: Option<CfmlValue>,
    /// Stack of saved output buffers for cfsavecontent
    pub saved_output_buffers: Vec<String>,
    /// Base template path (original .cfm being served)
    pub base_template_path: Option<String>,
    /// Component mappings: virtual prefix → physical directory (sorted longest-first)
    pub mappings: Vec<CfmlMapping>,
    /// Captured locals from most recent execute_function_with_args call
    /// Used to capture component body variables (variables scope) after component loading
    captured_locals: Option<IndexMap<String, CfmlValue>>,
    /// Active transaction connection (held during cftransaction block, type-erased)
    pub transaction_conn: Option<Box<dyn std::any::Any>>,
    /// Datasource URL of the active transaction
    pub transaction_datasource: Option<String>,
    /// Function pointer: begin transaction (datasource) -> conn
    pub txn_begin: Option<fn(&str) -> Result<Box<dyn std::any::Any>, CfmlError>>,
    /// Function pointer: commit transaction (conn)
    pub txn_commit: Option<fn(&mut Box<dyn std::any::Any>) -> Result<(), CfmlError>>,
    /// Function pointer: rollback transaction (conn)
    pub txn_rollback: Option<fn(&mut Box<dyn std::any::Any>) -> Result<(), CfmlError>>,
    /// Function pointer: execute query with transaction conn (conn, sql, params, return_type) -> result
    pub txn_execute: Option<fn(&mut Box<dyn std::any::Any>, &str, &CfmlValue, &str) -> CfmlResult>,
    /// Function pointer: execute query normally (args) -> result
    pub query_execute_fn: Option<fn(Vec<CfmlValue>) -> CfmlResult>,
    /// Session ID for current request
    pub session_id: Option<String>,
    /// Per-request flag: Application.cfc set `this.lazySessionCreation =
    /// true`. When true, no session record is created at request start;
    /// instead the record + `onSessionStart` fire on the first write to
    /// the `session` scope.
    pub lazy_session_creation: bool,
    /// Set after the lifecycle's session phase if lazy mode is on AND
    /// no existing record was hydrated. The next session-scope write
    /// triggers [`lazy_init_session_if_pending`] which clears this.
    pub session_lazy_pending: bool,
    /// True after `lazy_init_session_if_pending` actually inserted a
    /// new session record (i.e. the request touched the session scope).
    /// Embedders (the Workers fetch handler, the `--serve` HTTP layer)
    /// read this to decide whether to emit `Set-Cookie`.
    pub session_record_created: bool,
    /// Re-entry guard for `lazy_init_session_if_pending` so that
    /// `onSessionStart`'s own session-scope writes don't recurse.
    session_lazy_initializing: bool,
    /// Application.cfc component struct, stashed by
    /// `execute_with_lifecycle` so that
    /// `lazy_init_session_if_pending` can fire `onSessionStart` from
    /// inside a bytecode-op handler without re-loading the .cfc.
    pub app_cfc_template: Option<CfmlValue>,
    /// Stack of held cflock guards (name, guard)
    held_locks: Vec<(String, HeldLock)>,
    /// Custom tag paths from this.customTagPaths in Application.cfc
    pub custom_tag_paths: Vec<String>,
    /// Application-wide default for Lucee `localMode`. Set from
    /// `this.localMode` in Application.cfc; functions that don't declare
    /// their own `localMode` attribute inherit this. Default: classic
    /// (false) for back-compat.
    pub app_local_mode_modern: bool,
    /// Stack for nested body-mode custom tags
    custom_tag_stack: Vec<CustomTagState>,
    /// In-memory cache: key -> (value, optional expiry instant)
    pub cache: HashMap<String, (CfmlValue, Option<cfml_common::clock::Monotonic>)>,
    /// cfsetting enableCFOutputOnly counter (>0 means only cfoutput content is emitted)
    pub enable_cfoutput_only: i32,
    /// Sandbox mode: blocks host filesystem access, routes reads through VFS
    pub sandbox: bool,
    /// After a function call, holds modified complex-type argument values for
    /// pass-by-reference writeback. Maps param name → final value.
    arg_ref_writeback: Option<Vec<(String, CfmlValue)>>,
    /// Registry of Rust-backed classes, keyed by lowercased class name.
    /// Populated via `register_native_class`. The function is invoked when
    /// CFML calls `createObject("rust", "Name", ...)` / `new rust:Name(...)`
    /// to produce a fresh `CfmlValue::NativeObject`.
    pub native_classes: HashMap<String, NativeConstructor>,

    // ── Runtime knobs sourced from `.cfconfig.json`. Seeded by the CLI from
    // ServerState.cfconfig at VM construction time; Application.cfc `this.*`
    // can override at app scope (handled in extract_app_config). Builtins
    // that care about these values read them from the VM directly so they
    // don't have to chase through server_state every call.

    /// `runtime.nullSupport`. When true, unset variables return null instead
    /// of empty string. Default `false` (Lucee/ACF classic).
    pub null_support: bool,
    /// `runtime.dotNotationUpperCase`. When true, dot-notation struct key
    /// assignment forces upper-case (classic CF behaviour). Default `true`.
    pub dot_notation_upper: bool,
    /// `runtime.locale` — IETF BCP 47 (e.g. `en-GB`). Empty = system locale.
    /// Consumed by lsXxx() formatters as the default when none is supplied.
    pub locale: String,
    /// `runtime.timezone` — IANA tz name (e.g. `Europe/London`). Empty =
    /// system timezone. Consumed by now() and date formatters.
    pub timezone: String,
    /// `runtime.whitespaceCompressionEnabled` — global equivalent of
    /// `cfsetting enableCFOutputOnly=true`. Defaults `false`.
    pub whitespace_compression: bool,
    /// Resolved session timeout in seconds (default 1800). `this.sessionTimeout`
    /// in Application.cfc overrides this at app scope.
    pub session_timeout_secs: u64,
    /// Resolved application timeout in seconds (default 86400).
    pub application_timeout_secs: u64,
    /// Resolved client timeout in seconds (default 604800).
    pub client_timeout_secs: u64,

    /// `security.disallowedFunctions` — lower-cased BIF names that are
    /// refused before dispatch. A call to a banned name raises a runtime
    /// error rather than executing the implementation.
    pub disallowed_functions: std::collections::HashSet<String>,
    /// `security.disallowedImports` — regex patterns blocking the path arg
    /// of `createObject("component", path)` and class arg of
    /// `createObject("rust", name)`. Match = refusal.
    pub disallowed_imports: Vec<regex::Regex>,
}

/// Constructor signature for a Rust-backed class registered via
/// `register_native_class`. Receives the constructor arguments and must
/// return a `CfmlValue::NativeObject` wrapping the new instance.
pub type NativeConstructor = fn(Vec<CfmlValue>) -> CfmlResult;

#[derive(Debug, Clone)]
struct CallFrame {
    function_name: String,
    template: String,
    /// Current line within this function (updated by LineInfo)
    line: usize,
    /// Line in the caller where this function was invoked
    caller_line: usize,
}

#[derive(Debug, Clone)]
struct TryHandler {
    catch_ip: usize,
    stack_depth: usize,
}

/// State for a body-mode custom tag execution
#[derive(Debug, Clone)]
struct CustomTagState {
    template_path: String,
    attributes: CfmlValue,
}

impl CfmlVirtualMachine {
    pub fn new(program: BytecodeProgram) -> Self {
        Self {
            program,
            globals: IndexMap::new(),
            builtins: HashMap::new(),
            output_buffer: String::new(),
            vfs: Arc::new(RealFs),
            user_functions: HashMap::new(),
            source_file: None,
            call_stack: Vec::new(),
            try_stack: Vec::new(),
            current_exception: None,
            last_exception: None,
            current_line: 0,
            current_column: 0,
            method_this_writeback: None,
            method_variables_writeback: None,
            closure_parent_writeback: None,
            request_scope: IndexMap::new(),
            application_scope: None,
            server_state: None,
            response_headers: Vec::new(),
            response_status: None,
            response_content_type: None,
            response_body: None,
            redirect_url: None,
            http_request_data: None,
            saved_output_buffers: Vec::new(),
            base_template_path: None,
            mappings: Vec::new(),
            captured_locals: None,
            transaction_conn: None,
            transaction_datasource: None,
            txn_begin: None,
            txn_commit: None,
            txn_rollback: None,
            txn_execute: None,
            session_id: None,
            lazy_session_creation: false,
            session_lazy_pending: false,
            session_record_created: false,
            session_lazy_initializing: false,
            app_cfc_template: None,
            query_execute_fn: None,
            held_locks: Vec::new(),
            custom_tag_paths: Vec::new(),
            app_local_mode_modern: false,
            custom_tag_stack: Vec::new(),
            cache: HashMap::new(),
            enable_cfoutput_only: 0,
            sandbox: false,
            arg_ref_writeback: None,
            native_classes: HashMap::new(),
            // Compiled-in runtime defaults. `apply_cfconfig` overlays the
            // user's `.cfconfig.json` values when a VM is constructed via
            // the serve / CLI path that has a ServerState.
            null_support: false,
            dot_notation_upper: true,
            locale: String::new(),
            timezone: String::new(),
            whitespace_compression: false,
            session_timeout_secs: 1800,
            application_timeout_secs: 86_400,
            client_timeout_secs: 604_800,
            disallowed_functions: std::collections::HashSet::new(),
            disallowed_imports: Vec::new(),
        }
    }

    /// Overlay `.cfconfig.json` runtime knobs onto a freshly-constructed VM.
    /// Call this immediately after `new()` (and before stdlib registration)
    /// so timeouts, locale, and timezone defaults reflect the config file.
    pub fn apply_cfconfig(&mut self, cfg: &cfml_config::RustCfmlConfig) {
        let r = &cfg.runtime;
        self.null_support = r.null_support;
        self.dot_notation_upper = r.dot_notation_upper_case;
        if !r.locale.is_empty() {
            self.locale = r.locale.clone();
        }
        if !r.timezone.is_empty() {
            self.timezone = r.timezone.clone();
        }
        self.whitespace_compression = r.whitespace_compression_enabled;
        if let Some(secs) = cfml_config::RuntimeCfg::parse_timeout_seconds(&r.session_timeout) {
            self.session_timeout_secs = secs;
        }
        if let Some(secs) =
            cfml_config::RuntimeCfg::parse_timeout_seconds(&r.application_timeout)
        {
            self.application_timeout_secs = secs;
        }
        if let Some(secs) = cfml_config::RuntimeCfg::parse_timeout_seconds(&r.client_timeout) {
            self.client_timeout_secs = secs;
        }
        // security.sandbox sets the default when no CLI flag overrode it.
        // The CLI path sets `vm.sandbox` explicitly afterwards if it needs to.
        if cfg.security.sandbox {
            self.sandbox = true;
        }
        // security.disallowedFunctions: lower-case all names once for cheap
        // case-insensitive matching at call time.
        self.disallowed_functions = cfg
            .security
            .disallowed_functions
            .iter()
            .map(|n| n.to_lowercase())
            .collect();
        // security.disallowedImports: compile each regex. Invalid patterns
        // are logged and skipped — a typo shouldn't take down the VM.
        self.disallowed_imports = cfg
            .security
            .disallowed_imports
            .iter()
            .filter_map(|p| match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    log::warn!(
                        "cfconfig security.disallowedImports: invalid regex '{}': {}",
                        p,
                        e
                    );
                    None
                }
            })
            .collect();
    }

    /// Register a Rust-backed class so that CFML code can construct
    /// instances via `createObject("rust", "Name")` / `new rust:Name()`.
    ///
    /// `name` is matched case-insensitively (lowercased at registration and
    /// at the call site). `constructor` receives the args passed to `new`
    /// and must return a `CfmlValue::NativeObject` wrapping the new
    /// instance — typically by allocating the underlying struct, wrapping
    /// it in `Arc::new(RwLock::new(_))`, then `CfmlValue::NativeObject(arc)`.
    pub fn register_native_class(&mut self, name: &str, constructor: NativeConstructor) {
        self.native_classes
            .insert(name.to_lowercase(), constructor);
    }

    /// Register a Rust function as a callable CFML built-in.
    ///
    /// Intended for use by `--build`-produced binaries (and tests) that need to
    /// expose extra functions written in Rust without touching `cfml-stdlib`.
    /// The function is reachable from CFML two ways: by direct call (`name(...)`)
    /// and as a first-class value once `name` is looked up in `globals` — mirroring
    /// the existing stdlib registration pattern in `cfml_stdlib::get_builtin_functions`
    /// + `get_builtins`.
    ///
    /// `name` is stored as-provided; CFML's call dispatcher already does a
    /// case-insensitive fallback on builtin lookup.
    pub fn register_native_fn(&mut self, name: &str, f: BuiltinFunction) {
        self.builtins.insert(name.to_string(), f);
        self.globals.insert(
            name.to_string(),
            CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                name: name.to_string(),
                params: Vec::new(),
                body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                    CfmlValue::Null,
                )),
                return_type: None,
                access: cfml_common::dynamic::CfmlAccess::Public,
                captured_scope: None,
            }),
        );
    }

    fn build_stack_trace(&self) -> Vec<cfml_common::vm::StackFrame> {
        use cfml_common::vm::StackFrame;
        let mut frames = Vec::new();
        let template = self.source_file.clone().unwrap_or_default();

        if self.call_stack.is_empty() {
            // Error in __main__ — single frame
            frames.push(StackFrame {
                function: "__main__".to_string(),
                template,
                line: self.current_line,
            });
        } else {
            // Innermost frame: the function currently executing, at the current line
            frames.push(StackFrame {
                function: self.call_stack.last().unwrap().function_name.clone(),
                template: template.clone(),
                line: self.current_line,
            });
            // Intermediate frames in reverse (skip the last/current)
            for frame in self.call_stack.iter().rev().skip(1) {
                frames.push(StackFrame {
                    function: frame.function_name.clone(),
                    template: frame.template.clone(),
                    line: frame.line,
                });
            }
            // Root frame: __main__ at the line where the outermost function was called
            frames.push(StackFrame {
                function: "__main__".to_string(),
                template,
                line: self.call_stack.first().unwrap().caller_line,
            });
        }
        frames
    }

    fn build_tag_context(&self) -> CfmlValue {
        let frames = self.build_stack_trace();
        let context: Vec<CfmlValue> = frames
            .iter()
            .map(|f| {
                let mut entry = IndexMap::new();
                entry.insert(
                    "template".to_string(),
                    CfmlValue::String(f.template.clone()),
                );
                entry.insert("line".to_string(), CfmlValue::Int(f.line as i64));
                entry.insert("id".to_string(), CfmlValue::String("CFML".to_string()));
                entry.insert(
                    "raw_trace".to_string(),
                    CfmlValue::String(format!("at {}({}:{})", f.function, f.template, f.line)),
                );
                entry.insert("column".to_string(), CfmlValue::Int(0));
                CfmlValue::strukt(entry)
            })
            .collect();
        CfmlValue::array(context)
    }

    fn build_error_struct(e: &CfmlError, tag_context: CfmlValue) -> CfmlValue {
        let mut err_struct = IndexMap::new();
        err_struct.insert("message".to_string(), CfmlValue::String(e.message.clone()));
        err_struct.insert(
            "type".to_string(),
            CfmlValue::String(format!("{}", e.error_type)),
        );
        err_struct.insert("detail".to_string(), CfmlValue::String(String::new()));
        err_struct.insert("tagcontext".to_string(), tag_context);
        CfmlValue::strukt(err_struct)
    }

    // If `last_exception` already holds a struct whose `message` matches
    // `e.message`, reuse it (inner throw preserved detail); otherwise build a
    // fresh error struct. Avoids cloning the whole exception just to compare
    // a message string.
    fn resolve_catch_error_val(&mut self, e: &CfmlError) -> CfmlValue {
        let matched = matches!(
            self.last_exception.as_ref(),
            Some(CfmlValue::Struct(s))
                if matches!(s.get("message"), Some(CfmlValue::String(msg)) if msg == &e.message)
        );
        if matched {
            // last_exception already holds the right value; clone once for the stack
            self.last_exception.as_ref().unwrap().clone()
        } else {
            let v = Self::build_error_struct(e, self.build_tag_context());
            self.last_exception = Some(v.clone());
            v
        }
    }

    /// Control-flow sentinels that look like exceptions in the VM but must
    /// NOT be caught by user-level `try { ... } catch (any e) { ... }` blocks.
    /// `cfabort` and `cflocation` use these to unwind the call stack; if
    /// user code intercepts them, frameworks like Taffy break (Taffy's
    /// `throwError` calls `abort` to short-circuit, then the user-level
    /// outer `try` swallows it).
    #[inline]
    fn is_control_flow_error(e: &CfmlError) -> bool {
        e.message == "__cfabort" || e.message == "__cflocation_redirect"
    }

    fn wrap_error(&self, mut err: CfmlError) -> CfmlError {
        if err.stack_trace.is_empty() {
            err.stack_trace = self.build_stack_trace();
        }
        err
    }

    /// Extract `obj.name` semantics — identical to the BytecodeOp::GetProperty
    /// logic but operates on a borrowed CfmlValue so the caller avoids a
    /// stack push/pop round-trip. Used by LoadLocalProperty.
    fn lookup_property(obj: &CfmlValue, name: &str) -> CfmlValue {
        match obj {
            CfmlValue::Struct(s) => {
                let val = s
                    .get(name)
                    .or_else(|| s.get(&name.to_uppercase()))
                    .or_else(|| s.get(&name.to_lowercase()))
                    .or_else(|| {
                        let name_lower = name.to_lowercase();
                        s.iter()
                            .find(|(k, _)| k.to_lowercase() == name_lower)
                            .map(|(_, v)| v)
                    })
                    .or_else(|| {
                        if let Some(CfmlValue::Struct(vars)) = s.get("__variables") {
                            let name_lower = name.to_lowercase();
                            vars.get(name)
                                .or_else(|| vars.get(&name_lower))
                                .or_else(|| {
                                    vars.iter()
                                        .find(|(k, _)| k.to_lowercase() == name_lower)
                                        .map(|(_, v)| v)
                                })
                        } else {
                            None
                        }
                    })
                    .cloned();
                if let Some(v) = val {
                    return v;
                }
                // Fall through to a Rust-backed parent if one is attached.
                if let Some(CfmlValue::NativeObject(parent)) = s.get("__super") {
                    if let Ok(guard) = parent.read() {
                        if let Some(v) = guard.get_property(name) {
                            return v;
                        }
                    }
                }
                CfmlValue::Null
            }
            CfmlValue::Array(arr) => {
                if name.eq_ignore_ascii_case("len") || name.eq_ignore_ascii_case("length") {
                    CfmlValue::Int(arr.len() as i64)
                } else {
                    CfmlValue::Null
                }
            }
            CfmlValue::String(s) => {
                if name.eq_ignore_ascii_case("len") || name.eq_ignore_ascii_case("length") {
                    CfmlValue::Int(s.len() as i64)
                } else {
                    CfmlValue::Null
                }
            }
            CfmlValue::Query(q) => {
                if name.eq_ignore_ascii_case("recordcount") {
                    CfmlValue::Int(q.rows.len() as i64)
                } else if name.eq_ignore_ascii_case("columnlist") {
                    CfmlValue::String(q.columns.join(","))
                } else {
                    let is_col = q.columns.iter().any(|c| c.eq_ignore_ascii_case(name));
                    if is_col {
                        let col_data: Vec<CfmlValue> = q
                            .rows
                            .iter()
                            .map(|row| {
                                row.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or(CfmlValue::Null)
                            })
                            .collect();
                        // QueryColumn proxy: acts as Array for indexing/iteration
                        // but stringifies to first row (Lucee parity).
                        CfmlValue::QueryColumn(std::sync::Arc::new(col_data))
                    } else {
                        CfmlValue::Null
                    }
                }
            }
            _ => obj.get(name).unwrap_or(CfmlValue::Null),
        }
    }

    pub fn execute(&mut self) -> CfmlResult {
        let main_idx = self
            .program
            .functions
            .iter()
            .position(|f| f.name == "__main__")
            .ok_or_else(|| CfmlError::runtime("No main function found".to_string()))?;

        self.execute_function_by_index(main_idx, Vec::new())
            .map_err(|e| self.wrap_error(e))
    }

    fn execute_function_by_index(&mut self, func_idx: usize, args: Vec<CfmlValue>) -> CfmlResult {
        let func = self.program.functions[func_idx].clone();
        self.execute_function_with_args(&func, args, None)
    }

    fn execute_function_with_args(
        &mut self,
        func: &BytecodeFunction,
        args: Vec<CfmlValue>,
        parent_scope: Option<&IndexMap<String, CfmlValue>>,
    ) -> CfmlResult {
        // Guard against runaway recursion — checked before allocating locals
        // to avoid blowing the native Rust stack.
        //
        // Strategy:
        //  1. Hard ceiling at 2500 (matches CFML engine defaults)
        //  2. Early infinite-recursion detection: once depth > 64, check if
        //     the last 32 frames show a repeating cycle (covers both direct
        //     self-recursion and mutual recursion like A→B→A→B)
        let depth = self.call_stack.len();
        if depth > 2500 {
            return Err(self.wrap_error(CfmlError::runtime(format!(
                "Call stack overflow (depth {})",
                depth
            ))));
        }
        if depth > 64 && depth % 256 == 0 {
            // Throttled cycle detection: only check every 256 calls to avoid
            // scanning function names on every call in deep recursion.
            let window = 32.min(depth);
            let recent: Vec<&str> = self.call_stack[depth - window..]
                .iter()
                .map(|f| f.function_name.as_str())
                .collect();
            'cycle: for cycle_len in 1..=4 {
                if window < cycle_len * 4 {
                    continue;
                }
                let pattern = &recent[recent.len() - cycle_len..];
                let check_count = window / cycle_len;
                for i in 0..check_count {
                    let offset = recent.len() - cycle_len * (i + 1);
                    let chunk = &recent[offset..offset + cycle_len];
                    if chunk != pattern {
                        continue 'cycle;
                    }
                }
                let cycle_desc = pattern.join(" -> ");
                return Err(self.wrap_error(CfmlError::runtime(format!(
                    "Likely infinite recursion detected: {} (depth {})",
                    cycle_desc, depth
                ))));
            }
        }

        let mut locals: IndexMap<String, CfmlValue> = IndexMap::new();
        let mut stack: Vec<CfmlValue> = Vec::new();
        let mut ip = 0;
        // Track variables declared with `var` (function-local, not written back to parent)
        let mut declared_locals: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Shared closure environment: all closures defined within this function
        // invocation share one Rc<RefCell<HashMap>>. Lazily created on first DefineFunction.
        let mut closure_env: Option<Arc<RwLock<IndexMap<String, CfmlValue>>>> = None;

        // Effective Lucee localMode for this frame: function's declared attribute
        // wins; otherwise inherits the application default (`this.localMode` in
        // Application.cfc); otherwise classic.
        let effective_local_mode_modern: bool =
            func.declared_local_mode.unwrap_or(self.app_local_mode_modern);

        // Copy parent scope variables (closures and nested functions see parent vars).
        // Skip Function values — they're immutable and already available via
        // user_functions, so cloning them (and their captured scopes) is pure waste.
        if let Some(parent) = parent_scope {
            for (k, v) in parent {
                if !matches!(v, CfmlValue::Function(_)) {
                    locals.insert(k.clone(), v.clone());
                }
            }
        }

        // Build CFML arguments scope
        let mut arguments_map: IndexMap<String, CfmlValue> = IndexMap::new();
        for (i, param_name) in func.params.iter().enumerate() {
            let value = args.get(i).cloned().unwrap_or(CfmlValue::Null);
            locals.insert(param_name.clone(), value.clone());
            arguments_map.insert(param_name.clone(), value.clone());
            // Also make args accessible by position (1-based)
            arguments_map.insert((i + 1).to_string(), value);
        }
        // Also add any extra positional args beyond declared params
        for i in func.params.len()..args.len() {
            arguments_map.insert((i + 1).to_string(), args[i].clone());
        }
        // Check required parameters
        for (i, param_name) in func.params.iter().enumerate() {
            if func.required_params.get(i).copied().unwrap_or(false) && args.get(i).is_none() {
                return Err(self.wrap_error(CfmlError::runtime(format!(
                    "The parameter [{}] to function [{}] is required but was not passed in.",
                    param_name, func.name
                ))));
            }
        }
        locals.insert("arguments".to_string(), CfmlValue::strukt(arguments_map));

        // Push call frame for stack trace tracking (skip __main__ — it's the root)
        if func.name != "__main__" {
            self.call_stack.push(CallFrame {
                function_name: func.name.clone(),
                template: func
                    .source_file
                    .clone()
                    .or_else(|| self.source_file.clone())
                    .unwrap_or_default(),
                line: 0,
                caller_line: self.current_line,
            });
        }

        // Invariant for the duration of this function's execution —
        // hoisted out of the per-op dispatch loop.
        let is_inside_function = func.name != "__main__";

        loop {
            if ip >= func.instructions.len() {
                break;
            }

            let op = &func.instructions[ip];
            ip += 1;

            match op {
                BytecodeOp::Null => stack.push(CfmlValue::Null),
                BytecodeOp::True => stack.push(CfmlValue::Bool(true)),
                BytecodeOp::False => stack.push(CfmlValue::Bool(false)),
                BytecodeOp::Integer(n) => stack.push(CfmlValue::Int(*n)),
                BytecodeOp::Double(d) => stack.push(CfmlValue::Double(*d)),
                BytecodeOp::String(s) => stack.push(CfmlValue::String(s.clone())),

                BytecodeOp::LoadLocal(name) => {
                    // Handle CFML scope references
                    // Avoid allocating a lowercase String when the identifier is
                    // already all-lowercase ASCII (the common case). Unicode
                    // identifiers still get full case-folding.
                    let name_lower_owned: String;
                    let name_lower: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
                        name_lower_owned = name.to_lowercase();
                        &name_lower_owned
                    } else {
                        name.as_str()
                    };
                    let val = if name_lower == "local" {
                        // `local` is the function-local scope inside functions; at
                        // template scope CFML engines (Lucee/ACF/BoxLang) treat it
                        // as an auto-vivifying struct on first subscript write.
                        // Return the locals struct view either way — StoreLocal
                        // merges the result back.
                        let mut scope = locals.clone();
                        scope.shift_remove("__variables");
                        CfmlValue::strukt(scope)
                    } else if name_lower == "variables"
                    {
                        // Return a struct representing the variables scope
                        if !is_inside_function {
                            let mut merged = self.globals.clone();
                            for (k, v) in &locals {
                                merged.insert(k.clone(), v.clone());
                            }
                            CfmlValue::strukt(merged)
                        } else if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                            // CFC method: variables scope IS the __variables struct.
                            CfmlValue::Struct(vars.clone())
                        } else {
                            CfmlValue::strukt(locals.clone())
                        }
                    } else if name_lower == "request" {
                        CfmlValue::strukt(self.request_scope.clone())
                    } else if name_lower == "application" {
                        if let Some(ref app_scope) = self.application_scope {
                            if let Ok(scope) = app_scope.lock() {
                                CfmlValue::strukt(scope.clone())
                            } else {
                                CfmlValue::strukt(IndexMap::new())
                            }
                        } else {
                            CfmlValue::strukt(IndexMap::new())
                        }
                    } else if name_lower == "session" {
                        self.get_session_scope()
                    } else if name_lower == "cookie" {
                        self.globals
                            .get("cookie")
                            .cloned()
                            .unwrap_or(CfmlValue::strukt(IndexMap::new()))
                    } else if name_lower == "server" {
                        let mut scope = build_server_scope();
                        if let Some(ref ss) = self.server_state {
                            scope.insert(
                                "cfconfig".to_string(),
                                cfconfig_to_cfml(&ss.cfconfig),
                            );
                        }
                        CfmlValue::strukt(scope)
                    } else if let Some(val) =
                        self.lookup_name_in_scopes(name.as_str(), name_lower, &locals)
                    {
                        val
                    } else if let Some(bc_func) = self
                        .user_functions
                        .get(name.as_str())
                        .or_else(|| {
                            self.user_functions
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(name_lower))
                                .map(|(_, v)| v)
                        })
                        .cloned()
                    {
                        // User-defined function referenced as a value (first-class function)
                        // Like Lucee/BoxLang: functions are in variables scope.
                        // Capture the current scope so the function retains access to its
                        // defining scope's variables when stored in a struct and called later.
                        // Filter out Function values to avoid recursive reference chains.
                        let func_idx = self
                            .program
                            .functions
                            .iter()
                            .position(|f| f.name == bc_func.name)
                            .unwrap_or(0);
                        let filtered: IndexMap<String, CfmlValue> = locals
                            .iter()
                            .filter(|(_, v)| !matches!(v, CfmlValue::Function(_)))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        let scope = Arc::new(RwLock::new(filtered));
                        CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                            name: bc_func.name.clone(),
                            params: bc_func
                                .params
                                .iter()
                                .enumerate()
                                .map(|(i, pname)| cfml_common::dynamic::CfmlParam {
                                    name: pname.clone(),
                                    param_type: None,
                                    default: None,
                                    required: bc_func
                                        .required_params
                                        .get(i)
                                        .copied()
                                        .unwrap_or(false),
                                })
                                .collect(),
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                CfmlValue::Int(func_idx as i64),
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: Some(scope),
                        })
                    } else {
                        // Variable not found — check try_stack for error handler
                        if let Some(handler) = self.try_stack.pop() {
                            let mut exception = IndexMap::new();
                            exception.insert(
                                "message".to_string(),
                                CfmlValue::String(format!("Variable '{}' is undefined", name)),
                            );
                            exception.insert(
                                "type".to_string(),
                                CfmlValue::String("expression".to_string()),
                            );
                            exception
                                .insert("detail".to_string(), CfmlValue::String(String::new()));
                            stack.truncate(handler.stack_depth);
                            let exc = CfmlValue::strukt(exception);
                            self.last_exception = Some(exc.clone());
                            locals.insert("cfcatch".to_string(), exc);
                            ip = handler.catch_ip;
                            continue;
                        }
                        return Err(self.wrap_error(CfmlError::runtime(format!(
                            "Variable '{}' is undefined",
                            name
                        ))));
                    };
                    stack.push(val);
                }
                BytecodeOp::TryLoadLocal(name) => {
                    // Safe load: returns Null for undefined vars (used by Elvis, null-safe, isNull)
                    let name_lower = name.to_lowercase();
                    let val = if name_lower == "variables" || name_lower == "local" {
                        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                            CfmlValue::Struct(vars.clone())
                        } else {
                            CfmlValue::strukt(locals.clone())
                        }
                    } else if name_lower == "request" {
                        CfmlValue::strukt(self.request_scope.clone())
                    } else if name_lower == "application" {
                        if let Some(ref app_scope) = self.application_scope {
                            if let Ok(scope) = app_scope.lock() {
                                CfmlValue::strukt(scope.clone())
                            } else {
                                CfmlValue::Null
                            }
                        } else {
                            CfmlValue::Null
                        }
                    } else if name_lower == "server" {
                        CfmlValue::Null // server scope handled by LoadLocal
                    } else {
                        self.lookup_name_in_scopes(name.as_str(), &name_lower, &locals)
                            .unwrap_or(CfmlValue::Null)
                    };
                    stack.push(val);
                }
                BytecodeOp::DeclareLocal(name) => {
                    // Mark this variable as function-local (var keyword)
                    declared_locals.insert(name.clone());
                }
                BytecodeOp::StoreLocal(name) => {
                    if let Some(val) = stack.pop() {
                        let name_lower = name.to_lowercase();
                        if name_lower == "local" {
                            // `local.X = Y` — write back into the function's locals
                            // (or the template-scope locals when called outside a function),
                            // NOT __variables (which is the component scope in CFC methods).
                            if let CfmlValue::Struct(s) = val {
                                // Preserve __variables if present; merge everything else.
                                let saved_vars = locals.get("__variables").cloned();
                                for (k, v) in s.iter() {
                                    locals.insert(k.clone(), v.clone());
                                }
                                if let Some(v) = saved_vars {
                                    locals.insert("__variables".to_string(), v);
                                }
                            }
                        } else if name_lower == "variables" {
                            if let CfmlValue::Struct(s) = val {
                                if locals.contains_key("__variables") {
                                    // CFC method: write back to the __variables scope
                                    locals.insert("__variables".to_string(), CfmlValue::Struct(s));
                                } else {
                                    // Non-CFC: merge keys back into locals
                                    for (k, v) in s.iter() {
                                        locals.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                        } else if name_lower == "request" {
                            if let CfmlValue::Struct(s) = &val {
                                self.request_scope = (**s).clone();
                            }
                        } else if name_lower == "application" {
                            if let CfmlValue::Struct(s) = &val {
                                if let Some(ref app_scope) = self.application_scope {
                                    if let Ok(mut scope) = app_scope.lock() {
                                        *scope = (**s).clone();
                                    }
                                }
                            }
                        } else if name_lower == "session" {
                            if let CfmlValue::Struct(s) = &val {
                                self.set_session_scope((**s).clone());
                            }
                        } else if name_lower == "thread" && self.globals.contains_key("thread") {
                            self.globals.insert("thread".to_string(), val);
                        } else if name_lower == "arguments" && is_inside_function {
                            // When the arguments scope is stored, sync complex-type
                            // params back to their named locals so that modifications
                            // via `arguments.param.prop = val` are visible to the
                            // pass-by-reference writeback mechanism.
                            if let CfmlValue::Struct(ref args) = val {
                                for (k, v) in args.iter() {
                                    if matches!(
                                        v,
                                        CfmlValue::Struct(_)
                                            | CfmlValue::Array(_)
                                            | CfmlValue::Query(_)
                                            | CfmlValue::Component(_)
                                    ) {
                                        locals.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                            locals.insert(name.clone(), val);
                        } else if locals.contains_key("__variables")
                            && !declared_locals.contains(name)
                            && !declared_locals.contains(&name_lower)
                            && !locals.contains_key(name.as_str())
                            && name_lower != "arguments"
                            && name_lower != "cfcatch"
                            && !effective_local_mode_modern
                        {
                            // CFC method, classic localmode (default): unscoped,
                            // non-local variables go to __variables (the component scope).
                            // In modern localmode this branch is skipped and the write
                            // falls through to the locals-insert branch below.
                            if let Some(vars) =
                                locals.get_mut("__variables").and_then(|v| v.as_struct_mut())
                            {
                                vars.insert(name.clone(), val);
                            }
                        } else {
                            locals.insert(name.clone(), val.clone());
                            // Bidirectional sync: when a function param is stored,
                            // also update arguments[param] so `arguments.x` sees
                            // the latest value (and vice versa, handled above for arguments)
                            if is_inside_function
                                && func.params.iter().any(|p| p.eq_ignore_ascii_case(name))
                                && matches!(
                                    val,
                                    CfmlValue::Struct(_)
                                        | CfmlValue::Array(_)
                                        | CfmlValue::Query(_)
                                        | CfmlValue::Component(_)
                                )
                            {
                                if let Some(args) =
                                    locals.get_mut("arguments").and_then(|v| v.as_struct_mut())
                                {
                                    args.insert(name.clone(), val.clone());
                                }
                            }
                            // Sync to shared closure env so closures see updated value
                            // Only update vars already in the env (don't pollute with new vars)
                            if let Some(ref env) = closure_env {
                                let mut m = env.write().unwrap();
                                if m.contains_key(name.as_str()) {
                                    m.insert(name.clone(), val);
                                }
                            }
                        }
                    }
                }
                BytecodeOp::LoadGlobal(name) => {
                    let name_lower = name.to_lowercase();
                    // 1. Check locals (exact, then CI)
                    if let Some(val) = locals.get(name.as_str()) {
                        stack.push(val.clone());
                    } else if let Some(val) = locals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == name_lower)
                        .map(|(_, v)| v.clone())
                    {
                        stack.push(val);
                    // 1b. Check __variables scope for CFC methods
                    } else if let Some(val) = locals.get("__variables").and_then(|v| {
                        if let CfmlValue::Struct(vars) = v {
                            vars.get(name.as_str()).cloned().or_else(|| {
                                vars.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
                                    .map(|(_, v)| v.clone())
                            })
                        } else {
                            None
                        }
                    }) {
                        stack.push(val);
                    // 2. Check globals (exact, then CI)
                    } else if let Some(val) = self.globals.get(name.as_str()) {
                        stack.push(val.clone());
                    } else if let Some(val) = self
                        .globals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == name_lower)
                        .map(|(_, v)| v.clone())
                    {
                        stack.push(val);
                    // 3. Check builtins/user_functions (exact, then CI)
                    } else if self.builtins.contains_key(name.as_str())
                        || self.user_functions.contains_key(name.as_str())
                    {
                        let params = self
                            .user_functions
                            .get(name.as_str())
                            .map(|uf| {
                                uf.params
                                    .iter()
                                    .enumerate()
                                    .map(|(i, p)| cfml_common::dynamic::CfmlParam {
                                        name: p.clone(),
                                        param_type: None,
                                        default: None,
                                        required: uf
                                            .required_params
                                            .get(i)
                                            .copied()
                                            .unwrap_or(false),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // For user functions, find the bytecode index and capture the
                        // current scope so the function retains access to its defining
                        // scope's variables when stored in a struct and called later.
                        let (body_val, scope) = if self.user_functions.contains_key(name.as_str()) {
                            let func_idx =
                                self.program.functions.iter().position(|f| f.name == *name);
                            match func_idx {
                                Some(idx) => (CfmlValue::Int(idx as i64), None),
                                None => (CfmlValue::Null, None),
                            }
                        } else {
                            (CfmlValue::Null, None)
                        };
                        stack.push(CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                            name: name.clone(),
                            params,
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                body_val,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: scope,
                        }));
                    } else if self.builtins.keys().any(|k| k.to_lowercase() == name_lower)
                        || self
                            .user_functions
                            .keys()
                            .any(|k| k.to_lowercase() == name_lower)
                    {
                        let canonical = self
                            .builtins
                            .keys()
                            .find(|k| k.to_lowercase() == name_lower)
                            .or_else(|| {
                                self.user_functions
                                    .keys()
                                    .find(|k| k.to_lowercase() == name_lower)
                            })
                            .cloned()
                            .unwrap_or(name.clone());
                        let params = self
                            .user_functions
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == name_lower)
                            .map(|(_, uf)| {
                                uf.params
                                    .iter()
                                    .enumerate()
                                    .map(|(i, p)| cfml_common::dynamic::CfmlParam {
                                        name: p.clone(),
                                        param_type: None,
                                        default: None,
                                        required: uf
                                            .required_params
                                            .get(i)
                                            .copied()
                                            .unwrap_or(false),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // For user functions (CI match), find bytecode index and capture scope
                        let (body_val, scope, resolved_name) = if let Some(uf_name) = self
                            .user_functions
                            .keys()
                            .find(|k| k.to_lowercase() == name_lower)
                            .cloned()
                        {
                            let func_idx = self
                                .program
                                .functions
                                .iter()
                                .position(|f| f.name.to_lowercase() == name_lower);
                            match func_idx {
                                Some(idx) => (CfmlValue::Int(idx as i64), None, uf_name),
                                None => (CfmlValue::Null, None, uf_name),
                            }
                        } else {
                            (CfmlValue::Null, None, canonical)
                        };
                        stack.push(CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                            name: resolved_name,
                            params,
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                body_val,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: scope,
                        }));
                    // 4. Check VM-intercepted function names (custom tags, etc.)
                    } else if matches!(
                        name_lower.as_str(),
                        "__cfcustomtag"
                            | "__cfcustomtag_start"
                            | "__cfcustomtag_end"
                            | "callstackget"
                            | "callstackdump"
                            | "precisionevaluate"
                    ) {
                        stack.push(CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                            name: name.clone(),
                            params: Vec::new(),
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                CfmlValue::Null,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: None,
                        }));
                    } else if matches!(
                        name_lower.as_str(),
                        "cfheader"
                            | "cfcontent"
                            | "cflocation"
                            | "cfabort"
                            | "cfsetting"
                            | "cfcookie"
                            | "cflog"
                            | "cfinvoke"
                    ) {
                        // Script-style tag calls: `cfcontent(reset=true)` routes to `__cfcontent`.
                        let underscored = format!("__{}", name_lower);
                        stack.push(CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                            name: underscored,
                            params: Vec::new(),
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                CfmlValue::Null,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: None,
                        }));
                    } else {
                        return Err(self.wrap_error(CfmlError::runtime(format!(
                            "Variable '{}' is undefined",
                            name
                        ))));
                    }
                }
                BytecodeOp::StoreGlobal(name) => {
                    if let Some(val) = stack.pop() {
                        self.globals.insert(name.clone(), val);
                    }
                }

                BytecodeOp::Pop => {
                    stack.pop();
                }
                BytecodeOp::Dup => {
                    if let Some(val) = stack.last() {
                        stack.push(val.clone());
                    }
                }
                BytecodeOp::Swap => {
                    let len = stack.len();
                    if len >= 2 {
                        stack.swap(len - 1, len - 2);
                    }
                }

                // Arithmetic
                BytecodeOp::Add => {
                    binary_op(&mut stack, |a, b| match (&a, &b) {
                        (CfmlValue::Int(i), CfmlValue::Int(j)) => CfmlValue::Int(i + j),
                        (CfmlValue::Double(x), CfmlValue::Double(y)) => CfmlValue::Double(x + y),
                        (CfmlValue::Int(i), CfmlValue::Double(d)) => {
                            CfmlValue::Double(*i as f64 + d)
                        }
                        (CfmlValue::Double(d), CfmlValue::Int(i)) => {
                            CfmlValue::Double(d + *i as f64)
                        }
                        (CfmlValue::String(s), CfmlValue::String(t)) => {
                            CfmlValue::String(format!("{}{}", s, t))
                        }
                        // CFML: try numeric coercion
                        _ => {
                            let a_num = to_number(&a);
                            let b_num = to_number(&b);
                            match (a_num, b_num) {
                                (Some(x), Some(y)) => CfmlValue::Double(x + y),
                                _ => {
                                    CfmlValue::String(format!("{}{}", a.as_string(), b.as_string()))
                                }
                            }
                        }
                    });
                }
                BytecodeOp::Sub => {
                    binary_op(&mut stack, |a, b| numeric_op(&a, &b, |x, y| x - y));
                }
                BytecodeOp::Mul => {
                    binary_op(&mut stack, |a, b| numeric_op(&a, &b, |x, y| x * y));
                }
                BytecodeOp::Div => {
                    if let (Some(b), Some(a)) = (stack.pop(), stack.pop()) {
                        let x = to_number(&a).unwrap_or(0.0);
                        let y = to_number(&b).unwrap_or(1.0);
                        if y == 0.0 {
                            // CFML throws on division by zero
                            let mut exception = IndexMap::new();
                            exception.insert(
                                "message".to_string(),
                                CfmlValue::String("Division by zero is not allowed.".to_string()),
                            );
                            exception.insert(
                                "type".to_string(),
                                CfmlValue::String("Expression".to_string()),
                            );
                            exception
                                .insert("detail".to_string(), CfmlValue::String(String::new()));
                            exception.insert("tagcontext".to_string(), self.build_tag_context());
                            let error_val = CfmlValue::strukt(exception);
                            self.last_exception = Some(error_val.clone());
                            if let Some(handler) = self.try_stack.pop() {
                                while stack.len() > handler.stack_depth {
                                    stack.pop();
                                }
                                stack.push(error_val);
                                ip = handler.catch_ip;
                                continue;
                            } else {
                                return Err(CfmlError::runtime(
                                    "Division by zero is not allowed.".to_string(),
                                ));
                            }
                        } else {
                            stack.push(CfmlValue::Double(x / y));
                        }
                    }
                }
                BytecodeOp::Mod => {
                    binary_op(&mut stack, |a, b| match (&a, &b) {
                        (CfmlValue::Int(i), CfmlValue::Int(j)) if *j != 0 => CfmlValue::Int(i % j),
                        _ => {
                            let x = to_number(&a).unwrap_or(0.0);
                            let y = to_number(&b).unwrap_or(1.0);
                            CfmlValue::Double(x % y)
                        }
                    });
                }
                BytecodeOp::Pow => {
                    binary_op(&mut stack, |a, b| {
                        let x = to_number(&a).unwrap_or(0.0);
                        let y = to_number(&b).unwrap_or(0.0);
                        CfmlValue::Double(x.powf(y))
                    });
                }
                BytecodeOp::IntDiv => {
                    binary_op(&mut stack, |a, b| {
                        let x = to_number(&a).unwrap_or(0.0) as i64;
                        let y = to_number(&b).unwrap_or(1.0) as i64;
                        if y == 0 {
                            CfmlValue::Int(0)
                        } else {
                            CfmlValue::Int(x / y)
                        }
                    });
                }
                BytecodeOp::Negate => {
                    if let Some(val) = stack.pop() {
                        match val {
                            CfmlValue::Int(i) => stack.push(CfmlValue::Int(-i)),
                            CfmlValue::Double(d) => stack.push(CfmlValue::Double(-d)),
                            _ => {
                                if let Some(n) = to_number(&val) {
                                    stack.push(CfmlValue::Double(-n));
                                } else {
                                    stack.push(CfmlValue::Int(0));
                                }
                            }
                        }
                    }
                }

                // String concatenation
                BytecodeOp::Concat => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::String(format!("{}{}", a.as_string(), b.as_string()))
                    });
                }

                // Comparison - proper value comparison
                BytecodeOp::Eq => {
                    compare_op(&mut stack, |a, b| cfml_equal(a, b));
                }
                BytecodeOp::Neq => {
                    compare_op(&mut stack, |a, b| !cfml_equal(a, b));
                }
                BytecodeOp::Lt => {
                    compare_op(&mut stack, |a, b| cfml_compare(a, b) < 0);
                }
                BytecodeOp::Lte => {
                    compare_op(&mut stack, |a, b| cfml_compare(a, b) <= 0);
                }
                BytecodeOp::Gt => {
                    compare_op(&mut stack, |a, b| cfml_compare(a, b) > 0);
                }
                BytecodeOp::Gte => {
                    compare_op(&mut stack, |a, b| cfml_compare(a, b) >= 0);
                }

                // CFML-specific operators
                BytecodeOp::Contains => {
                    compare_op(&mut stack, |a, b| {
                        let haystack = a.as_string().to_lowercase();
                        let needle = b.as_string().to_lowercase();
                        haystack.contains(&needle)
                    });
                }
                BytecodeOp::DoesNotContain => {
                    compare_op(&mut stack, |a, b| {
                        let haystack = a.as_string().to_lowercase();
                        let needle = b.as_string().to_lowercase();
                        !haystack.contains(&needle)
                    });
                }

                // Logical
                BytecodeOp::And => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::Bool(a.is_true() && b.is_true())
                    });
                }
                BytecodeOp::Or => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::Bool(a.is_true() || b.is_true())
                    });
                }
                BytecodeOp::Not => {
                    if let Some(a) = stack.pop() {
                        stack.push(CfmlValue::Bool(!a.is_true()));
                    }
                }
                BytecodeOp::Xor => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::Bool(a.is_true() ^ b.is_true())
                    });
                }
                BytecodeOp::Eqv => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::Bool(a.is_true() == b.is_true())
                    });
                }
                BytecodeOp::Imp => {
                    binary_op(&mut stack, |a, b| {
                        CfmlValue::Bool(!a.is_true() || b.is_true())
                    });
                }

                // Control flow
                BytecodeOp::Jump(target) => {
                    ip = *target;
                }
                BytecodeOp::JumpIfFalse(target) => {
                    if let Some(cond) = stack.pop() {
                        if !cond.is_true() {
                            ip = *target;
                        }
                    }
                }
                BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmp, target) => {
                    // Fused loop-condition super-instruction. Equivalent to
                    // LoadLocal(name) + Integer(c) + <cmp> + JumpIfFalse(target)
                    // but avoids 3 dispatches per iteration.
                    let matched = match locals.get(name.as_str()) {
                        Some(CfmlValue::Int(i)) => {
                            let c = *c;
                            let i = *i;
                            match cmp {
                                CmpOp::Lt => i < c,
                                CmpOp::Lte => i <= c,
                                CmpOp::Gt => i > c,
                                CmpOp::Gte => i >= c,
                                CmpOp::Eq => i == c,
                                CmpOp::Neq => i != c,
                            }
                        }
                        Some(CfmlValue::Double(d)) => {
                            let c = *c as f64;
                            let d = *d;
                            match cmp {
                                CmpOp::Lt => d < c,
                                CmpOp::Lte => d <= c,
                                CmpOp::Gt => d > c,
                                CmpOp::Gte => d >= c,
                                CmpOp::Eq => d == c,
                                CmpOp::Neq => d != c,
                            }
                        }
                        // Any other type (including missing): fall back to the
                        // full CFML comparison semantics. Keeps correctness
                        // for unusual cases (string loop var, null, etc.).
                        other => {
                            let left = other.cloned().unwrap_or(CfmlValue::Null);
                            let right = CfmlValue::Int(*c);
                            match cmp {
                                CmpOp::Lt => cfml_compare(&left, &right) < 0,
                                CmpOp::Lte => cfml_compare(&left, &right) <= 0,
                                CmpOp::Gt => cfml_compare(&left, &right) > 0,
                                CmpOp::Gte => cfml_compare(&left, &right) >= 0,
                                CmpOp::Eq => cfml_equal(&left, &right),
                                CmpOp::Neq => !cfml_equal(&left, &right),
                            }
                        }
                    };
                    if !matched {
                        ip = *target;
                    }
                }
                BytecodeOp::ForLoopStep(name, limit, cmp, step, target) => {
                    // Fused loop-step super-instruction emitted at the bottom
                    // of counted for-loops. Equivalent to:
                    //   Increment(name)   // or Decrement if step is -1
                    //   JumpIfLocalCmpConstTrue(name, limit, cmp, target)
                    // but one dispatch instead of two.
                    let new_val = match locals.get(name.as_str()) {
                        Some(CfmlValue::Int(i)) => CfmlValue::Int(*i + *step),
                        Some(CfmlValue::Double(d)) => CfmlValue::Double(*d + (*step as f64)),
                        _ => {
                            // Loop var changed type mid-loop (user mutated it).
                            // Fall back to a safe step of 0 so we don't silently
                            // coerce; loop will likely exit on the next cmp.
                            CfmlValue::Int(*step)
                        }
                    };
                    locals.insert(name.clone(), new_val.clone());
                    if let Some(ref env) = closure_env {
                        let mut m = env.write().unwrap();
                        if m.contains_key(name.as_str()) {
                            m.insert(name.clone(), new_val.clone());
                        }
                    }
                    // Test and jump-back.
                    let matched = match &new_val {
                        CfmlValue::Int(i) => {
                            let c = *limit;
                            let i = *i;
                            match cmp {
                                CmpOp::Lt => i < c,
                                CmpOp::Lte => i <= c,
                                CmpOp::Gt => i > c,
                                CmpOp::Gte => i >= c,
                                CmpOp::Eq => i == c,
                                CmpOp::Neq => i != c,
                            }
                        }
                        CfmlValue::Double(d) => {
                            let c = *limit as f64;
                            let d = *d;
                            match cmp {
                                CmpOp::Lt => d < c,
                                CmpOp::Lte => d <= c,
                                CmpOp::Gt => d > c,
                                CmpOp::Gte => d >= c,
                                CmpOp::Eq => d == c,
                                CmpOp::Neq => d != c,
                            }
                        }
                        _ => false,
                    };
                    if matched {
                        ip = *target;
                    }
                }
                BytecodeOp::JumpIfTrue(target) => {
                    if let Some(cond) = stack.pop() {
                        if cond.is_true() {
                            ip = *target;
                        }
                    }
                }

                BytecodeOp::Call(arg_count) => {
                    // Identify which local variables are being passed as args
                    // (for pass-by-reference writeback of complex types)
                    // ip was already incremented past this Call op, so use ip-1
                    let arg_sources = find_arg_sources(&func.instructions, ip - 1, *arg_count);

                    let mut args = Vec::with_capacity(*arg_count);
                    for _ in 0..*arg_count {
                        if let Some(v) = stack.pop() {
                            args.push(v);
                        }
                    }
                    args.reverse();

                    if let Some(func_ref) = stack.pop() {
                        self.closure_parent_writeback = None;
                        self.arg_ref_writeback = None;
                        // For closures with captured scope, merge defining scope + caller locals.
                        // For CFC method calls (this in locals), caller locals take priority.
                        // For plain UDF calls, pass caller locals by reference (no clone).
                        let merged_scope;
                        let effective_locals = if let CfmlValue::Function(ref f) = func_ref {
                            if let Some(ref shared_env) = f.captured_scope {
                                let is_cfc_context = locals.contains_key("this");
                                merged_scope = if is_cfc_context {
                                    // CFC methods: start with captured scope (has runtime data),
                                    // then overlay functions from caller locals (correct method overrides),
                                    // then add remaining caller locals (like `this`).
                                    // __variables and this ALWAYS come from caller (current state).
                                    let mut m = shared_env.read().unwrap().clone();
                                    for (k, v) in &locals {
                                        if matches!(v, CfmlValue::Function(_))
                                            || !m.contains_key(k)
                                            || k == "__variables"
                                            || k == "this"
                                        {
                                            m.insert(k.clone(), v.clone());
                                        }
                                    }
                                    m
                                } else {
                                    let mut m = shared_env.read().unwrap().clone();
                                    for (k, v) in &locals {
                                        if !m.contains_key(k) {
                                            m.insert(k.clone(), v.clone());
                                        }
                                    }
                                    m
                                };
                                &merged_scope
                            } else {
                                &locals
                            }
                        } else {
                            &locals
                        };
                        // Isolate try-stack so throws inside the callee
                        // don't consume the caller's handlers
                        let saved_try_stack = if self.try_stack.is_empty() {
                            None
                        } else {
                            Some(std::mem::take(&mut self.try_stack))
                        };
                        let call_result = self.call_function(&func_ref, args, effective_locals);
                        if let Some(saved) = saved_try_stack {
                            self.try_stack = saved;
                        }
                        match call_result {
                            Ok(result) => {
                                // Write back mutations into the shared closure environment
                                if let Some(ref writeback) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&func_ref, writeback);
                                }
                                // Merge closure write-back into caller's locals
                                if let Some(writeback) = self.closure_parent_writeback.take() {
                                    for (k, v) in writeback {
                                        locals.insert(k, v);
                                    }
                                }
                                // Pass-by-reference writeback: update caller's local variables
                                // with modified complex-type argument values
                                if let Some(ref_wb) = self.arg_ref_writeback.take() {
                                    for (idx_str, modified_val) in ref_wb {
                                        if let Ok(param_idx) = idx_str.parse::<usize>() {
                                            if param_idx < arg_sources.len() {
                                                if let Some(ref source_var) = arg_sources[param_idx]
                                                {
                                                    locals.insert(source_var.clone(), modified_val);
                                                }
                                            }
                                        }
                                    }
                                }
                                stack.push(result);
                            }
                            Err(e) => {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                // Route error through try-catch mechanism
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    // Use last_exception only if it was set by this call
                                    // (e.g. an inner throw). Build from the CfmlError
                                    // otherwise, to avoid reusing a stale exception from
                                    // a previous catch block.
                                    let error_val = self.resolve_catch_error_val(&e);
                                    stack.push(error_val);
                                    ip = handler.catch_ip;
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }

                BytecodeOp::CallNamed(names, arg_count) => {
                    // Identify arg sources for pass-by-reference writeback
                    // ip was already incremented past this op, so use ip-1
                    let named_arg_sources =
                        find_arg_sources(&func.instructions, ip - 1, *arg_count);

                    let mut named_values = Vec::with_capacity(*arg_count);
                    for _ in 0..*arg_count {
                        if let Some(v) = stack.pop() {
                            named_values.push(v);
                        }
                    }
                    named_values.reverse();

                    if let Some(func_ref) = stack.pop() {
                        // Expand argumentCollection: unpack struct keys as named args
                        let mut expanded_names = Vec::new();
                        let mut expanded_values = Vec::new();
                        for (i, name) in names.iter().enumerate() {
                            if name.eq_ignore_ascii_case("argumentcollection") {
                                if let Some(CfmlValue::Struct(s)) = named_values.get(i) {
                                    for (k, v) in s.iter() {
                                        expanded_names.push(k.clone());
                                        expanded_values.push(v.clone());
                                    }
                                    continue;
                                }
                            }
                            expanded_names.push(name.clone());
                            expanded_values
                                .push(named_values.get(i).cloned().unwrap_or(CfmlValue::Null));
                        }

                        // Reorder named args to match function param positions
                        let args = if let CfmlValue::Function(ref f) = func_ref {
                            let mut positional =
                                vec![CfmlValue::Null; f.params.len().max(expanded_names.len())];
                            for (i, name) in expanded_names.iter().enumerate() {
                                let value = if i < expanded_values.len() {
                                    std::mem::replace(&mut expanded_values[i], CfmlValue::Null)
                                } else {
                                    CfmlValue::Null
                                };
                                if name.is_empty() {
                                    if i < positional.len() {
                                        positional[i] = value;
                                    }
                                    continue;
                                }
                                let target = f
                                    .params
                                    .iter()
                                    .position(|p| p.name.eq_ignore_ascii_case(name));
                                match target {
                                    Some(pi) if pi < positional.len() => positional[pi] = value,
                                    Some(_) => {}
                                    None => positional.push(value),
                                }
                            }
                            positional
                        } else {
                            named_values
                        };

                        self.closure_parent_writeback = None;
                        self.arg_ref_writeback = None;
                        let merged_scope;
                        let effective_locals = if let CfmlValue::Function(ref f) = func_ref {
                            if let Some(ref shared_env) = f.captured_scope {
                                let is_cfc_context = locals.contains_key("this");
                                merged_scope = if is_cfc_context {
                                    let mut m = shared_env.read().unwrap().clone();
                                    for (k, v) in &locals {
                                        if matches!(v, CfmlValue::Function(_)) || !m.contains_key(k)
                                        {
                                            m.insert(k.clone(), v.clone());
                                        }
                                    }
                                    m
                                } else {
                                    let mut m = shared_env.read().unwrap().clone();
                                    for (k, v) in &locals {
                                        if !m.contains_key(k) {
                                            m.insert(k.clone(), v.clone());
                                        }
                                    }
                                    m
                                };
                                &merged_scope
                            } else {
                                &locals
                            }
                        } else {
                            &locals
                        };
                        let saved_try_stack = if self.try_stack.is_empty() {
                            None
                        } else {
                            Some(std::mem::take(&mut self.try_stack))
                        };
                        let call_result = self.call_function(&func_ref, args, effective_locals);
                        if let Some(saved) = saved_try_stack {
                            self.try_stack = saved;
                        }
                        match call_result {
                            Ok(result) => {
                                if let Some(ref writeback) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&func_ref, writeback);
                                }
                                if let Some(writeback) = self.closure_parent_writeback.take() {
                                    for (k, v) in writeback {
                                        locals.insert(k, v);
                                    }
                                }
                                // Pass-by-reference writeback for named calls
                                if let Some(ref_wb) = self.arg_ref_writeback.take() {
                                    for (idx_str, modified_val) in ref_wb {
                                        if let Ok(param_idx) = idx_str.parse::<usize>() {
                                            // For named args: find which call-site arg was mapped
                                            // to this param position, and get its source variable
                                            if let CfmlValue::Function(ref f) = func_ref {
                                                // Find which call-site index maps to this param
                                                for (call_idx, name) in names.iter().enumerate() {
                                                    let matches = if name.is_empty() {
                                                        call_idx == param_idx
                                                    } else {
                                                        f.params.get(param_idx).map_or(false, |p| {
                                                            p.name.eq_ignore_ascii_case(name)
                                                        })
                                                    };
                                                    if matches && call_idx < named_arg_sources.len()
                                                    {
                                                        if let Some(ref source_var) =
                                                            named_arg_sources[call_idx]
                                                        {
                                                            locals.insert(
                                                                source_var.clone(),
                                                                modified_val.clone(),
                                                            );
                                                        }
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                stack.push(result);
                            }
                            Err(e) => {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    let error_val = self.resolve_catch_error_val(&e);
                                    stack.push(error_val);
                                    ip = handler.catch_ip;
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }

                BytecodeOp::Return => {
                    // Save modified 'this' for component method write-back
                    if let Some(this_val) = locals.get("this") {
                        self.method_this_writeback = Some(this_val.clone());
                        // If the return value on top of the stack IS the component's
                        // `this` (the common `return this;` pattern from chained-setter
                        // CFCs), embed the current `variables` scope into the returned
                        // `this` so chained method calls on the temp see the same data
                        // a follow-up call on the original local would see. This matches
                        // Lucee's reference semantics where `this` and `variables` are
                        // two views of the same object.
                        if let (Some(top), Some(CfmlValue::Struct(vars))) =
                            (stack.last(), locals.get("__variables"))
                        {
                            if let (CfmlValue::Struct(top_s), CfmlValue::Struct(this_s)) =
                                (top, this_val)
                            {
                                if Arc::ptr_eq(top_s, this_s) && !vars.is_empty() {
                                    let mut updated = top_s.clone();
                                    let updated_mut = Arc::make_mut(&mut updated);
                                    let merged_vars = vars.clone();
                                    updated_mut.insert(
                                        "__variables".to_string(),
                                        CfmlValue::Struct(merged_vars),
                                    );
                                    let last_idx = stack.len() - 1;
                                    stack[last_idx] = CfmlValue::Struct(updated);
                                }
                            }
                        }
                        // Save variables scope mutations for component write-back
                        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                            if !vars.is_empty() {
                                self.method_variables_writeback = Some((**vars).clone());
                            }
                        } else {
                            let mut vars_wb = IndexMap::new();
                            for (k, v) in &locals {
                                let kl = k.to_lowercase();
                                if kl == "this"
                                    || kl == "arguments"
                                    || k.starts_with("__")
                                    || func.params.contains(k)
                                    || declared_locals.contains(k.as_str())
                                {
                                    continue;
                                }
                                vars_wb.insert(k.clone(), v.clone());
                            }
                            if !vars_wb.is_empty() {
                                self.method_variables_writeback = Some(vars_wb);
                            }
                        }
                    }
                    // Closure parent scope write-back on early return.
                    // Modern localmode (Lucee parity): no unscoped write — new
                    // or shadowing an existing captured var — propagates out
                    // of the closure. All bareword writes stayed in the
                    // closure's own `local`. Explicit `variables.x = …` writes
                    // bypass StoreLocal's locals path entirely, so they're
                    // unaffected by this skip.
                    if let Some(parent) = parent_scope {
                        if !effective_local_mode_modern {
                            let mut writeback = IndexMap::new();
                            for (k, v) in &locals {
                                if k == "arguments"
                                    || k == "this"
                                    || func.params.contains(k)
                                    || declared_locals.contains(k.as_str())
                                {
                                    continue;
                                }
                                if let Some(parent_val) = parent.get(k) {
                                    if !Self::values_equal_shallow(v, parent_val) {
                                        writeback.insert(k.clone(), v.clone());
                                    }
                                } else {
                                    writeback.insert(k.clone(), v.clone());
                                }
                            }
                            if !writeback.is_empty() {
                                self.closure_parent_writeback = Some(writeback);
                            }
                        }
                    }
                    // Pass-by-reference writeback: collect final values of complex-type params
                    self.collect_arg_ref_writeback(func, &locals);
                    // Pop call frame before early return (matches push at function entry)
                    self.call_stack.pop();
                    debug_assert_eq!(
                        stack.len(),
                        1,
                        "operand-stack discipline broken at Return in {} ({} values, expected 1)",
                        func.name,
                        stack.len()
                    );
                    return Ok(stack.pop().unwrap_or(CfmlValue::Null));
                }

                // Collections
                BytecodeOp::BuildArray(count) => {
                    let mut elements = Vec::new();
                    for _ in 0..*count {
                        if let Some(val) = stack.pop() {
                            elements.push(val);
                        }
                    }
                    elements.reverse();
                    stack.push(CfmlValue::array(elements));
                }
                BytecodeOp::BuildStruct(count) => {
                    let mut pairs = Vec::new();
                    for _ in 0..*count {
                        let value = stack.pop().unwrap_or(CfmlValue::Null);
                        let key = stack.pop().unwrap_or(CfmlValue::String(String::new()));
                        pairs.push((key.as_string(), value));
                    }
                    let mut map = IndexMap::new();
                    for (k, v) in pairs.into_iter().rev() {
                        map.insert(k, v);
                    }
                    stack.push(CfmlValue::strukt(map));
                }
                BytecodeOp::GetIndex => {
                    let index = stack.pop().unwrap_or(CfmlValue::Null);
                    let collection = stack.pop().unwrap_or(CfmlValue::Null);
                    match &collection {
                        CfmlValue::Array(arr) | CfmlValue::QueryColumn(arr) => {
                            let idx = match &index {
                                CfmlValue::Int(i) => *i as usize,
                                CfmlValue::Double(d) => *d as usize,
                                CfmlValue::String(s) => s.parse::<usize>().unwrap_or(0),
                                _ => 0,
                            };
                            // CFML arrays are 1-based
                            let idx = if idx > 0 { idx - 1 } else { 0 };
                            stack.push(arr.get(idx).cloned().unwrap_or(CfmlValue::Null));
                        }
                        CfmlValue::Struct(s) => {
                            let key = index.as_string();
                            let val = s
                                .get(&key)
                                .or_else(|| s.get(&key.to_uppercase()))
                                .or_else(|| s.get(&key.to_lowercase()))
                                .or_else(|| {
                                    let key_lower = key.to_lowercase();
                                    s.iter()
                                        .find(|(k, _)| k.to_lowercase() == key_lower)
                                        .map(|(_, v)| v)
                                })
                                .cloned()
                                .unwrap_or(CfmlValue::Null);
                            stack.push(val);
                        }
                        _ => stack.push(CfmlValue::Null),
                    }
                }
                BytecodeOp::SetIndex => {
                    let index = stack.pop().unwrap_or(CfmlValue::Null);
                    let mut collection = stack.pop().unwrap_or(CfmlValue::Null);
                    let value = stack.pop().unwrap_or(CfmlValue::Null);
                    match &mut collection {
                        CfmlValue::Array(arr) => {
                            let idx = match &index {
                                CfmlValue::Int(i) => (*i as usize).saturating_sub(1), // 1-based
                                _ => 0,
                            };
                            if idx < arr.len() {
                                Arc::make_mut(arr)[idx] = value;
                            }
                        }
                        CfmlValue::Struct(s) => {
                            let key = index.as_string();
                            // Propagate to __variables for declared CFC properties
                            if s.contains_key("__variables") && s.contains_key("__properties") {
                                let key_lower = key.to_lowercase();
                                let is_declared =
                                    if let Some(CfmlValue::Array(props)) = s.get("__properties") {
                                        props.iter().any(|p| {
                                            if let CfmlValue::Struct(ps) = p {
                                                ps.iter().any(|(k, v)| {
                                                    k.to_lowercase() == "name"
                                                        && v.as_string().to_lowercase() == key_lower
                                                })
                                            } else {
                                                false
                                            }
                                        })
                                    } else {
                                        false
                                    };
                                if is_declared {
                                    if let Some(vars) =
                                        Arc::make_mut(s).get_mut("__variables").and_then(|v| v.as_struct_mut())
                                    {
                                        vars.insert(key.clone(), value.clone());
                                    }
                                }
                            }
                            Arc::make_mut(s).insert(key, value);
                        }
                        _ => {}
                    }
                    stack.push(collection);
                }

                BytecodeOp::LoadLocalProperty(local_name, prop_name) => {
                    // Fused LoadLocal + GetProperty. Avoids the intermediate
                    // dispatch and the stack push/pop of the struct itself.
                    // Only emitted when the receiver is a plain identifier
                    // and access is non-null-safe (hot-path struct read).
                    let val = locals
                        .get(local_name.as_str())
                        .map(|obj| Self::lookup_property(obj, prop_name))
                        .unwrap_or(CfmlValue::Null);
                    stack.push(val);
                }
                BytecodeOp::StoreLocalProperty(local_name, prop_name) => {
                    // Fused StoreLocal + SetProperty. Pops value from stack,
                    // gets the local struct, sets the property, stores back.
                    if let Some(value) = stack.pop() {
                        if let Some(obj) = locals.get_mut(local_name.as_str()) {
                            // CFC with a Rust-backed parent: try the native
                            // setter first; None defers to the CFC struct.
                            if let CfmlValue::Struct(ref s) = *obj {
                                if let Some(CfmlValue::NativeObject(parent)) =
                                    s.get("__super").cloned()
                                {
                                    let handled = {
                                        let mut guard = parent.write().map_err(|_| {
                                            CfmlError::runtime(
                                                "NativeObject lock poisoned".to_string(),
                                            )
                                        })?;
                                        guard.set_property(prop_name, value.clone())
                                    };
                                    if let Some(result) = handled {
                                        result?;
                                        continue;
                                    }
                                }
                            }
                            if let Some(s) = obj.as_struct_mut() {
                                s.insert(prop_name.clone(), value);
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Cannot set property '{}' on non-struct in local '{}'",
                                    prop_name, local_name
                                )));
                            }
                        } else {
                            return Err(CfmlError::runtime(format!(
                                "Local variable '{}' not found",
                                local_name
                            )));
                        }
                    }
                }
                BytecodeOp::GetProperty(name) => {
                    if let Some(obj) = stack.pop() {
                        match &obj {
                            CfmlValue::Struct(s) => {
                                let val = s
                                    .get(name.as_str())
                                    .or_else(|| s.get(&name.to_uppercase()))
                                    .or_else(|| s.get(&name.to_lowercase()))
                                    .or_else(|| {
                                        // Full case-insensitive scan for mixed-case keys
                                        let name_lower = name.to_lowercase();
                                        s.iter()
                                            .find(|(k, _)| k.to_lowercase() == name_lower)
                                            .map(|(_, v)| v)
                                    })
                                    .or_else(|| {
                                        // Fall back to __variables for component properties
                                        if let Some(CfmlValue::Struct(vars)) = s.get("__variables") {
                                            let name_lower = name.to_lowercase();
                                            vars.get(name.as_str())
                                                .or_else(|| vars.get(&name_lower))
                                                .or_else(|| {
                                                    vars.iter()
                                                        .find(|(k, _)| k.to_lowercase() == name_lower)
                                                        .map(|(_, v)| v)
                                                })
                                        } else {
                                            None
                                        }
                                    })
                                    .cloned();
                                let val = match val {
                                    Some(v) => v,
                                    None => {
                                        // Fall through to a Rust-backed parent if attached.
                                        if let Some(CfmlValue::NativeObject(parent)) =
                                            s.get("__super")
                                        {
                                            if let Ok(guard) = parent.read() {
                                                guard.get_property(name).unwrap_or(CfmlValue::Null)
                                            } else {
                                                CfmlValue::Null
                                            }
                                        } else {
                                            CfmlValue::Null
                                        }
                                    }
                                };
                                stack.push(val);
                            }
                            CfmlValue::Array(arr) => {
                                // Array member functions
                                match name.to_lowercase().as_str() {
                                    "len" | "length" => {
                                        stack.push(CfmlValue::Int(arr.len() as i64));
                                    }
                                    _ => stack.push(CfmlValue::Null),
                                }
                            }
                            CfmlValue::String(s) => {
                                // String member functions
                                match name.to_lowercase().as_str() {
                                    "len" | "length" => {
                                        stack.push(CfmlValue::Int(s.len() as i64));
                                    }
                                    _ => stack.push(CfmlValue::Null),
                                }
                            }
                            CfmlValue::Query(q) => {
                                match name.to_lowercase().as_str() {
                                    "recordcount" => {
                                        stack.push(CfmlValue::Int(q.rows.len() as i64));
                                    }
                                    "columnlist" => {
                                        stack.push(CfmlValue::String(q.columns.join(",")));
                                    }
                                    _ => {
                                        // Column access: q.columnName returns a QueryColumn
                                        // proxy — acts as Array for indexing/iteration/length,
                                        // but stringifies to the first row (Lucee parity).
                                        let col_lower = name.to_lowercase();
                                        let is_col =
                                            q.columns.iter().any(|c| c.to_lowercase() == col_lower);
                                        if is_col {
                                            let col_data: Vec<CfmlValue> = q
                                                .rows
                                                .iter()
                                                .map(|row| {
                                                    row.iter()
                                                        .find(|(k, _)| {
                                                            k.to_lowercase() == col_lower
                                                        })
                                                        .map(|(_, v)| v.clone())
                                                        .unwrap_or(CfmlValue::Null)
                                                })
                                                .collect();
                                            stack.push(CfmlValue::QueryColumn(std::sync::Arc::new(col_data)));
                                        } else {
                                            stack.push(CfmlValue::Null);
                                        }
                                    }
                                }
                            }
                            _ => {
                                stack.push(obj.get(&name).unwrap_or(CfmlValue::Null));
                            }
                        }
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }
                BytecodeOp::SetProperty(name) => {
                    if let Some(value) = stack.pop() {
                        if let Some(mut obj) = stack.pop() {
                            // CFC with a Rust-backed parent: route writes the
                            // native side recognises before touching the CFC
                            // struct, so Rust state stays first-class. The
                            // parent returns None to defer to the CFC.
                            if let CfmlValue::Struct(ref s) = obj {
                                if let Some(CfmlValue::NativeObject(parent)) =
                                    s.get("__super").cloned()
                                {
                                    let handled = {
                                        let mut guard = parent.write().map_err(|_| {
                                            CfmlError::runtime(
                                                "NativeObject lock poisoned".to_string(),
                                            )
                                        })?;
                                        guard.set_property(name, value.clone())
                                    };
                                    if let Some(result) = handled {
                                        result?;
                                        stack.push(obj);
                                        continue;
                                    }
                                }
                            }
                            // If setting on a CFC struct with declared properties,
                            // also update __variables for properties declared via
                            // `property name="x"` so they're accessible unscoped in methods.
                            if let Some(s) = obj.as_struct_mut() {
                                if s.contains_key("__variables") && s.contains_key("__properties") {
                                    let name_lower = name.to_lowercase();
                                    let is_declared = if let Some(CfmlValue::Array(props)) =
                                        s.get("__properties")
                                    {
                                        props.iter().any(|p| {
                                            if let CfmlValue::Struct(ps) = p {
                                                ps.iter().any(|(k, v)| {
                                                    k.to_lowercase() == "name"
                                                        && v.as_string().to_lowercase()
                                                            == name_lower
                                                })
                                            } else {
                                                false
                                            }
                                        })
                                    } else {
                                        false
                                    };
                                    if is_declared {
                                        if let Some(vars) =
                                            s.get_mut("__variables").and_then(|v| v.as_struct_mut())
                                        {
                                            vars.insert(name.clone(), value.clone());
                                        }
                                    }
                                }
                            }
                            obj.set(name.clone(), value);
                            stack.push(obj);
                        }
                    }
                }

                BytecodeOp::NewObject(arg_count) => {
                    // Pop constructor arguments first
                    let ctor_args: Vec<CfmlValue> = (0..*arg_count)
                        .filter_map(|_| stack.pop())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();

                    if let Some(class_ref) = stack.pop() {
                        // Resolve the component template
                        let template = if let CfmlValue::Struct(s) = &class_ref {
                            CfmlValue::Struct(s.clone())
                        } else {
                            let class_name = match &class_ref {
                                CfmlValue::Function(f) => f.name.clone(),
                                CfmlValue::String(s) => s.clone(),
                                _ => class_ref.as_string(),
                            };
                            self.resolve_component_template(&class_name, &locals)
                                .unwrap_or(CfmlValue::strukt(IndexMap::new()))
                        };

                        // Resolve inheritance chain
                        let instance = self.resolve_inheritance(template, &locals);

                        // Attach a Rust-class parent if extends="rust:Name"
                        let instance = self.attach_native_parent(instance)?;

                        // Validate interface implementation and collect transitive interfaces
                        let instance = if let CfmlValue::Struct(ref s) = instance {
                            let all_ifaces = self.validate_interface_implementation(s, &locals)?;
                            if !all_ifaces.is_empty() {
                                let mut s = s.clone();
                                let chain: Vec<CfmlValue> = all_ifaces
                                    .into_iter()
                                    .map(|name| CfmlValue::String(name))
                                    .collect();
                                Arc::make_mut(&mut s).insert("__implements_chain".to_string(), CfmlValue::array(chain));
                                CfmlValue::Struct(s)
                            } else {
                                instance
                            }
                        } else {
                            instance
                        };

                        // Call init() constructor if present
                        let final_instance = if let CfmlValue::Struct(ref s) = instance {
                            let has_init = s
                                .get("init")
                                .or_else(|| s.get("INIT"))
                                .or_else(|| s.get("Init"))
                                .cloned();
                            if let Some(ref init_func) = has_init {
                                if matches!(init_func, CfmlValue::Function(_)) {
                                    // Build init scope from the component's own scope,
                                    // NOT the caller's locals (which may be a different CFC)
                                    let mut init_locals = IndexMap::new();
                                    init_locals.insert("this".to_string(), instance.clone());
                                    // Inject component __variables as a dedicated scope (like Lucee/BoxLang)
                                    if let CfmlValue::Struct(ref cs) = instance {
                                        if let Some(vars) = cs.get("__variables") {
                                            init_locals
                                                .insert("__variables".to_string(), vars.clone());
                                        }
                                    }
                                    self.closure_parent_writeback = None;
                                    if let Ok(result) =
                                        self.call_function(init_func, ctor_args, &init_locals)
                                    {
                                        self.closure_parent_writeback = None;
                                        // Apply variables scope writeback from init() to the component
                                        let vars_wb = self.method_variables_writeback.take();
                                        let mut final_obj = if let Some(modified_this) =
                                            self.method_this_writeback.take()
                                        {
                                            modified_this
                                        } else if let CfmlValue::Struct(_) = &result {
                                            result
                                        } else {
                                            instance
                                        };
                                        // Merge init()'s __variables mutations back into the component
                                        if let Some(vars) = vars_wb {
                                            if let Some(s) = final_obj.as_struct_mut() {
                                                s.insert(
                                                    "__variables".to_string(),
                                                    CfmlValue::strukt(vars),
                                                );
                                            }
                                        }
                                        final_obj
                                    } else {
                                        instance
                                    }
                                } else {
                                    instance
                                }
                            } else {
                                instance
                            }
                        } else {
                            instance
                        };

                        stack.push(final_instance);
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }

                BytecodeOp::DefineFunction(func_idx) => {
                    let func_idx = *func_idx;
                    let func_name = self.program.functions[func_idx].name.clone();
                    self.user_functions.insert(
                        func_name.clone(),
                        Arc::clone(&self.program.functions[func_idx]),
                    );
                    // Create or reuse a shared closure environment so all closures
                    // defined in this function invocation share the same mutable state.
                    // On first definition, seed directly from locals; on subsequent
                    // definitions, sync so later closures see intervening declarations.
                    let env = match closure_env {
                        Some(ref env) => {
                            let mut m = env.write().unwrap();
                            for (k, v) in &locals {
                                m.insert(k.clone(), v.clone());
                            }
                            env
                        }
                        None => closure_env.insert(Arc::new(RwLock::new(locals.clone()))),
                    };
                    // Push function reference — encode func_idx in body for super dispatch
                    let bc_func_ref = &self.program.functions[func_idx];
                    stack.push(CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                        name: func_name,
                        params: bc_func_ref
                            .params
                            .iter()
                            .enumerate()
                            .map(|(i, name)| cfml_common::dynamic::CfmlParam {
                                name: name.clone(),
                                param_type: None,
                                default: None,
                                required: bc_func_ref
                                    .required_params
                                    .get(i)
                                    .copied()
                                    .unwrap_or(false),
                            })
                            .collect(),
                        body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                            CfmlValue::Int(func_idx as i64),
                        )),
                        return_type: None,
                        access: cfml_common::dynamic::CfmlAccess::Public,
                        captured_scope: Some(Arc::clone(env)),
                    }));
                }

                BytecodeOp::Increment(name) => {
                    if let Some(val) = locals.get(name.as_str()) {
                        let new_val = match val {
                            CfmlValue::Int(i) => CfmlValue::Int(i + 1),
                            CfmlValue::Double(d) => CfmlValue::Double(d + 1.0),
                            _ => CfmlValue::Int(1),
                        };
                        locals.insert(name.clone(), new_val.clone());
                        if let Some(ref env) = closure_env {
                            let mut m = env.write().unwrap();
                            if m.contains_key(name.as_str()) {
                                m.insert(name.clone(), new_val);
                            }
                        }
                    }
                }
                BytecodeOp::AddLocalConst(name, k) => {
                    if let Some(val) = locals.get(name.as_str()) {
                        let new_val = match val {
                            CfmlValue::Int(i) => CfmlValue::Int(i + *k),
                            CfmlValue::Double(d) => CfmlValue::Double(d + *k as f64),
                            _ => CfmlValue::Int(*k),
                        };
                        locals.insert(name.clone(), new_val.clone());
                        if let Some(ref env) = closure_env {
                            let mut m = env.write().unwrap();
                            if m.contains_key(name.as_str()) {
                                m.insert(name.clone(), new_val);
                            }
                        }
                    }
                }
                BytecodeOp::MulLocalConst(name, k) => {
                    if let Some(val) = locals.get(name.as_str()) {
                        let new_val = match val {
                            CfmlValue::Int(i) => CfmlValue::Int(i * *k),
                            CfmlValue::Double(d) => CfmlValue::Double(d * *k as f64),
                            _ => CfmlValue::Int(*k),
                        };
                        locals.insert(name.clone(), new_val.clone());
                        if let Some(ref env) = closure_env {
                            let mut m = env.write().unwrap();
                            if m.contains_key(name.as_str()) {
                                m.insert(name.clone(), new_val);
                            }
                        }
                    }
                }
                BytecodeOp::Decrement(name) => {
                    if let Some(val) = locals.get(name.as_str()) {
                        let new_val = match val {
                            CfmlValue::Int(i) => CfmlValue::Int(i - 1),
                            CfmlValue::Double(d) => CfmlValue::Double(d - 1.0),
                            _ => CfmlValue::Int(-1),
                        };
                        locals.insert(name.clone(), new_val.clone());
                        // Sync to shared closure env so closures see updated value
                        if let Some(ref env) = closure_env {
                            let mut m = env.write().unwrap();
                            if m.contains_key(name.as_str()) {
                                m.insert(name.clone(), new_val);
                            }
                        }
                    }
                }

                // Exception handling
                BytecodeOp::TryStart(catch_ip) => {
                    self.try_stack.push(TryHandler {
                        catch_ip: *catch_ip,
                        stack_depth: stack.len(),
                    });
                }
                BytecodeOp::TryEnd => {
                    self.try_stack.pop();
                }
                BytecodeOp::Throw => {
                    let error_val = stack
                        .pop()
                        .unwrap_or(CfmlValue::String("Unknown error".to_string()));
                    self.last_exception = Some(error_val.clone());
                    if let Some(handler) = self.try_stack.pop() {
                        // Unwind stack
                        while stack.len() > handler.stack_depth {
                            stack.pop();
                        }
                        stack.push(error_val);
                        ip = handler.catch_ip;
                    } else {
                        return Err(CfmlError::runtime(error_val.as_string()));
                    }
                }
                BytecodeOp::Rethrow => {
                    let error_val = self
                        .last_exception
                        .clone()
                        .unwrap_or(CfmlValue::String("No exception to rethrow".to_string()));
                    if let Some(handler) = self.try_stack.pop() {
                        while stack.len() > handler.stack_depth {
                            stack.pop();
                        }
                        stack.push(error_val);
                        ip = handler.catch_ip;
                    } else {
                        return Err(CfmlError::runtime(error_val.as_string()));
                    }
                }

                BytecodeOp::CallMethod(method_name, arg_count, write_back) => {
                    let mut extra_args: Vec<CfmlValue> =
                        (0..*arg_count).filter_map(|_| stack.pop()).collect();
                    extra_args.reverse();
                    // Pop the object (receiver)
                    let object = stack.pop().unwrap_or(CfmlValue::Null);

                    // Clear method-writeback state before the call. Both fields must
                    // be cleared — leaving `method_variables_writeback` set from an
                    // earlier method call leaks the previous receiver's variables
                    // scope onto the current receiver in the post-call writeback,
                    // corrupting plain-struct receivers (e.g. `enc = { string: fn }`)
                    // by giving them a spurious `__variables` field that masquerades
                    // them as CFCs on subsequent calls.
                    self.method_this_writeback = None;
                    self.method_variables_writeback = None;

                    // Detect super calls: object is a __super struct (no __name key,
                    // but contains Function values). For super.method(), bind `this`
                    // to the actual child instance from the caller's locals.
                    // Isolate try-stack so throws inside the callee
                    // don't consume the caller's handlers
                    let saved_try_stack_method = std::mem::take(&mut self.try_stack);
                    let method_result: Result<CfmlValue, CfmlError> =
                        if let CfmlValue::Struct(ref s) = object {
                            if s.contains_key("__is_super") {
                                // Super dispatch — find the parent's function by stored index
                                let prop = object.get(&method_name).unwrap_or(CfmlValue::Null);
                                if let CfmlValue::Function(ref f) = &prop {
                                    // Extract stored bytecode index from function body
                                    let func_idx =
                                        if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                            ref body,
                                        ) = f.body
                                        {
                                            if let CfmlValue::Int(idx) = body.as_ref() {
                                                Some(*idx as usize)
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        };
                                    let args: Vec<CfmlValue> = extra_args.drain(..).collect();
                                    let mut method_locals = IndexMap::new();
                                    // Merge captured scope first (closure vars from defining scope)
                                    if let Some(ref shared_env) = f.captured_scope {
                                        for (k, v) in shared_env.read().unwrap().iter() {
                                            method_locals.insert(k.clone(), v.clone());
                                        }
                                    }
                                    // Inject component __variables as a dedicated scope
                                    let this_ref = locals.get("this").unwrap_or(&object);
                                    if let CfmlValue::Struct(ref ts) = this_ref {
                                        if let Some(vars) = ts.get("__variables") {
                                            method_locals
                                                .insert("__variables".to_string(), vars.clone());
                                        }
                                    }
                                    // Use the actual child 'this' from caller's locals
                                    if let Some(real_this) = locals.get("this") {
                                        method_locals.insert("this".to_string(), real_this.clone());
                                    } else {
                                        method_locals.insert("this".to_string(), object.clone());
                                    }
                                    // Execute directly by index to avoid name collision
                                    self.closure_parent_writeback = None;
                                    let call_result = if let Some(idx) = func_idx {
                                        if idx < self.program.functions.len() {
                                            let parent_func = self.program.functions[idx].clone();
                                            self.execute_function_with_args(
                                                &parent_func,
                                                args,
                                                Some(&method_locals),
                                            )
                                        } else {
                                            self.call_function(&prop, args, &method_locals)
                                        }
                                    } else {
                                        self.call_function(&prop, args, &method_locals)
                                    };
                                    // Write back closure mutations to shared environment
                                    if let Ok(ref _val) = call_result {
                                        if let Some(ref wb) = self.closure_parent_writeback {
                                            Self::write_back_to_captured_scope(&prop, wb);
                                        }
                                    }
                                    call_result
                                } else {
                                    self.call_member_function(
                                        &object,
                                        &method_name,
                                        &mut extra_args,
                                    )
                                }
                            } else {
                                self.call_member_function(&object, &method_name, &mut extra_args)
                            }
                        } else {
                            self.call_member_function(&object, &method_name, &mut extra_args)
                        };
                    self.try_stack = saved_try_stack_method;
                    let result = match method_result {
                        Ok(val) => val,
                        Err(e) => {
                            if Self::is_control_flow_error(&e) {
                                return Err(e);
                            }
                            // Route error through try-catch mechanism
                            if let Some(handler) = self.try_stack.pop() {
                                while stack.len() > handler.stack_depth {
                                    stack.pop();
                                }
                                let error_val = self.resolve_catch_error_val(&e);
                                stack.push(error_val);
                                ip = handler.catch_ip;
                                continue;
                            } else {
                                return Err(e);
                            }
                        }
                    };

                    // Write-back: emulate CFML pass-by-reference semantics for mutating methods.
                    // The compiler encodes a path vec: ["var"], ["var", "prop"], ["a", "b", "c"], etc.
                    if let Some(ref path) = write_back {
                        if path.len() == 1 {
                            // Direct variable write-back: var.method(args)
                            let var_name = &path[0];
                            if Self::is_mutating_method(&method_name) {
                                self.scope_aware_store(var_name, result.clone(), &mut locals);
                            }
                        } else if path.len() >= 2 && Self::is_mutating_method(&method_name) {
                            // Deep property write-back: var.prop1.prop2...propN.method(args)
                            let var_name = &path[0];
                            if let Some(mut root_obj) = self.scope_aware_load(var_name, &locals) {
                                let props = &path[1..];
                                Self::deep_set(&mut root_obj, props, result.clone());
                                self.scope_aware_store(var_name, root_obj, &mut locals);
                            }
                        }
                    }

                    // Propagate component method `this` modifications back to caller.
                    // When a component method modifies `this` internally (e.g. `this.foo = bar`),
                    // the modified `this` snapshot is saved by execute_function_with_args.
                    //
                    // For chained calls like `c.setX(1).withStatus(2)`, the codegen emits
                    // write_back=["c"] for BOTH calls (the inner call's mutations need to
                    // reach `c`). If we naively REPLACE `c` with the second call's snapshot,
                    // we clobber the first call's variables-writeback (which already
                    // updated `c.__variables`). To match Lucee's reference semantics
                    // (`this` is the same Java object across the chain), merge the snapshot's
                    // top-level fields into the current `c` rather than replacing wholesale,
                    // and PRESERVE the existing `__variables` so the variables-writeback
                    // pass below can continue building on what previous chain steps wrote.
                    if let Some(modified_this) = self.method_this_writeback.take() {
                        if let Some(ref path) = write_back {
                            let var_name = &path[0];
                            if path.len() == 1 {
                                let existing = self.scope_aware_load(var_name, &locals);
                                // Only do merge-preserve for CFC components (which carry
                                // __variables). Plain shims (Java shims, Map-like values)
                                // need full replacement so e.g. `map.remove(k)` actually
                                // removes the key — merging would only add fields, never
                                // delete them.
                                let is_cfc = matches!(
                                    &modified_this,
                                    CfmlValue::Struct(s) if s.contains_key("__variables")
                                );
                                let merged = if is_cfc {
                                    match (existing, modified_this) {
                                        (Some(CfmlValue::Struct(mut cur)), CfmlValue::Struct(snap)) => {
                                            let cur_mut = Arc::make_mut(&mut cur);
                                            let preserved_vars = cur_mut.get("__variables").cloned();
                                            for (k, v) in snap.iter() {
                                                if k == "__variables" {
                                                    continue;
                                                }
                                                cur_mut.insert(k.clone(), v.clone());
                                            }
                                            if let Some(vars) = preserved_vars {
                                                cur_mut.insert("__variables".to_string(), vars);
                                            }
                                            CfmlValue::Struct(cur)
                                        }
                                        (_, snap) => snap,
                                    }
                                } else {
                                    modified_this
                                };
                                self.scope_aware_store(var_name, merged, &mut locals);
                            } else {
                                // Deep write-back for component this
                                if let Some(mut root_obj) = self.scope_aware_load(var_name, &locals)
                                {
                                    let props = &path[1..];
                                    Self::deep_set(&mut root_obj, props, modified_this);
                                    self.scope_aware_store(var_name, root_obj, &mut locals);
                                }
                            }
                        }
                    }

                    // Propagate component method `variables` scope mutations back.
                    // When a method writes `variables.x = y`, persist it in __variables.
                    if let Some(vars_wb) = self.method_variables_writeback.take() {
                        if let Some(ref path) = write_back {
                            let var_name = &path[0];
                            // Load the component object, update __variables, store it back
                            let load_path = if path.len() == 1 {
                                path.clone()
                            } else {
                                path[..path.len() - 1].to_vec()
                            };
                            if let Some(mut comp_obj) =
                                self.scope_aware_load(&load_path[0], &locals)
                            {
                                if load_path.len() > 1 {
                                    // Navigate to the component object
                                    for part in &load_path[1..] {
                                        comp_obj = comp_obj.get(part).unwrap_or(CfmlValue::Null);
                                    }
                                }
                                if let Some(s) = comp_obj.as_struct_mut() {
                                    let vars = s
                                        .entry("__variables".to_string())
                                        .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));
                                    if let Some(vs) = vars.as_struct_mut() {
                                        for (k, v) in vars_wb {
                                            vs.insert(k, v);
                                        }
                                    }
                                }
                                // Store back
                                if load_path.len() == 1 {
                                    self.scope_aware_store(var_name, comp_obj, &mut locals);
                                } else {
                                    if let Some(mut root_obj) =
                                        self.scope_aware_load(var_name, &locals)
                                    {
                                        Self::deep_set(&mut root_obj, &load_path[1..], comp_obj);
                                        self.scope_aware_store(var_name, root_obj, &mut locals);
                                    }
                                }
                            }
                        }
                    }

                    // Closure-mutation writeback: when a member-call higher-order
                    // method (e.g. arr.each((x) => outer.append(x))) runs a closure
                    // that mutates a captured outer-scope variable, propagate those
                    // mutations into the caller's locals. The BIF flavour
                    // (arrayEach/arrayMap/etc.) handles this via parent_locals
                    // threading; the member-call path does not, so we merge here.
                    if let Some(wb) = self.closure_parent_writeback.take() {
                        for (k, v) in wb {
                            self.scope_aware_store(&k, v, &mut locals);
                        }
                    }

                    stack.push(result);
                }

                BytecodeOp::GetKeys => {
                    // For for-in: convert struct to array of keys, leave arrays unchanged
                    if let Some(val) = stack.pop() {
                        match val {
                            CfmlValue::Struct(s) => {
                                let keys: Vec<CfmlValue> =
                                    s.keys().map(|k| CfmlValue::String(k.clone())).collect();
                                stack.push(CfmlValue::array(keys));
                            }
                            CfmlValue::String(s) => {
                                // Iterating over a string: convert to array of chars
                                let chars: Vec<CfmlValue> = s
                                    .chars()
                                    .map(|c| CfmlValue::String(c.to_string()))
                                    .collect();
                                stack.push(CfmlValue::array(chars));
                            }
                            CfmlValue::Query(q) => {
                                // Iterating over a query: convert to array of row structs
                                let rows: Vec<CfmlValue> = q
                                    .rows
                                    .iter()
                                    .map(|row| CfmlValue::strukt(row.clone()))
                                    .collect();
                                stack.push(CfmlValue::array(rows));
                            }
                            // Lucee@7 parity: `for (v in q.col)` yields a single
                            // element — the stringified first row — because
                            // Lucee treats QueryColumn as a string in iter context.
                            CfmlValue::QueryColumn(a) => {
                                let first = a.first().cloned().unwrap_or(CfmlValue::Null);
                                stack.push(CfmlValue::array(vec![first]));
                            }
                            other => stack.push(other), // arrays pass through
                        }
                    }
                }

                BytecodeOp::IsNull => {
                    if let Some(val) = stack.pop() {
                        stack.push(CfmlValue::Bool(matches!(val, CfmlValue::Null)));
                    } else {
                        stack.push(CfmlValue::Bool(true));
                    }
                }

                BytecodeOp::JumpIfNotNull(target) => {
                    // Peek at the top of stack - if not null, jump (leave value on stack)
                    // If null, continue (leave null on stack)
                    if let Some(val) = stack.last() {
                        if !matches!(val, CfmlValue::Null) {
                            ip = *target;
                        }
                    }
                }

                BytecodeOp::Include(path) => {
                    // Resolve path relative to source file or CWD.
                    // NB: source_dir.join(path) with an *absolute* path returns
                    // the absolute path unchanged (Path::join semantics), so a
                    // leading-slash CFML include initially resolves to an OS-root
                    // path here. The mapping/webroot fallback below catches that.
                    let resolved = if let Some(ref source) = self.source_file {
                        let source_dir = std::path::Path::new(source)
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."));
                        normalize_path(&source_dir.join(&path).to_string_lossy())
                    } else {
                        path.clone()
                    };

                    // Leading-slash includes are webroot-relative in CFML, not
                    // OS-absolute. Try in order: configured mappings,
                    // serve-mode webroot, then CLI-mode base_template_path's
                    // parent.
                    let resolved = if !self.vfs.exists(&resolved) && path.starts_with('/') {
                        self.resolve_leading_slash_include(&path)
                            .unwrap_or(resolved)
                    } else {
                        resolved
                    };

                    // Read, parse, compile, and execute the included file
                    let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
                    match compile_file_cached(&resolved, cache, self.vfs.as_ref()) {
                        Ok(sub_program) => {
                            let mut old_program = std::mem::replace(&mut self.program, sub_program);
                            let old_source = self.source_file.clone();
                            self.source_file = Some(resolved.clone());
                            let main_idx = self
                                .program
                                .functions
                                .iter()
                                .position(|f| f.name == "__main__")
                                .unwrap_or(0);
                            let inc_func = self.program.functions[main_idx].clone();
                            // Snapshot caller's keys before include so we can detect new variables
                            let pre_include_keys: std::collections::HashSet<String> =
                                locals.keys().cloned().collect();
                            // Isolate try-stack so throws inside the include
                            // don't consume outer handlers
                            let saved_try_stack = std::mem::take(&mut self.try_stack);
                            let result = self.execute_function_with_args(
                                &inc_func,
                                Vec::new(),
                                Some(&locals),
                            );
                            self.try_stack = saved_try_stack;
                            // Merge newly created variables from the include back
                            // into the caller's locals. This makes variables set via
                            // `variables.foo = "bar"` in the include accessible from
                            // the caller. Only NEW keys are merged — existing keys are
                            // not overwritten to prevent closure write-back from
                            // reverting caller state. Function values are NOT merged
                            // (they're already in user_functions); merging them would
                            // inject captured_scope that triggers spurious write-backs.
                            if let Some(inc_locals) = self.captured_locals.take() {
                                for (k, v) in inc_locals {
                                    if k == "arguments" {
                                        continue;
                                    }
                                    // Only merge NEW variables that are not functions and
                                    // don't shadow builtin function names (e.g. "val").
                                    if !pre_include_keys.contains(&k)
                                        && !matches!(v, CfmlValue::Function(_))
                                        && !self.builtins.contains_key(&k)
                                    {
                                        locals.insert(k, v);
                                    }
                                }
                            }
                            // Merge included sub-program's non-main functions into old_program
                            // so that func_idx references (from DefineFunction) remain valid.
                            let sub_func_count = self.program.functions.len();
                            let base_idx = old_program.functions.len();
                            for i in 0..sub_func_count {
                                if self.program.functions[i].name != "__main__" {
                                    old_program
                                        .functions
                                        .push(self.program.functions[i].clone());
                                }
                            }
                            // Fix func_idx in any CfmlFunction values stored in locals
                            // that were created by DefineFunction in the include.
                            // Sub-program index i → old_program index (base_idx + i - 1)
                            // (subtract 1 because __main__ at index 0 was skipped)
                            if base_idx > 0 && sub_func_count > 1 {
                                let offset = base_idx - 1; // -1 because __main__ was skipped
                                for (_, v) in locals.iter_mut() {
                                    Self::fixup_included_func_indices(v, base_idx, sub_func_count);
                                }
                                for (_, v) in self.globals.iter_mut() {
                                    Self::fixup_included_func_indices(v, base_idx, sub_func_count);
                                }
                                // Also fix DefineFunction bytecode instructions inside the
                                // merged functions — they still reference sub-program indices.
                                for fi in base_idx..old_program.functions.len() {
                                    let func = Arc::make_mut(&mut old_program.functions[fi]);
                                    for op in func.instructions.iter_mut() {
                                        if let BytecodeOp::DefineFunction(ref mut idx) = op {
                                            if *idx > 0 && *idx < sub_func_count {
                                                *idx += offset;
                                            }
                                        }
                                    }
                                }
                                // Update user_functions entries — they shared the
                                // sub-program's Arcs, which still have pre-fixup indices.
                                // Point them at the fixed Arcs from old_program so that
                                // subsequent calls see the corrected DefineFunction indices.
                                for fi in base_idx..old_program.functions.len() {
                                    let name = old_program.functions[fi].name.clone();
                                    if self.user_functions.contains_key(&name) {
                                        self.user_functions.insert(
                                            name,
                                            Arc::clone(&old_program.functions[fi]),
                                        );
                                    }
                                }
                            }
                            self.program = old_program;
                            self.source_file = old_source;
                            // Propagate include errors through try-catch
                            if let Err(e) = result {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    let mut err_struct = IndexMap::new();
                                    err_struct.insert(
                                        "message".to_string(),
                                        CfmlValue::String(e.message.clone()),
                                    );
                                    err_struct.insert(
                                        "type".to_string(),
                                        CfmlValue::String(format!("{}", e.error_type)),
                                    );
                                    err_struct.insert(
                                        "detail".to_string(),
                                        CfmlValue::String(String::new()),
                                    );
                                    err_struct
                                        .insert("tagcontext".to_string(), self.build_tag_context());
                                    let error_val = CfmlValue::strukt(err_struct);
                                    stack.push(error_val);
                                    ip = handler.catch_ip;
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }

                BytecodeOp::IncludeDynamic => {
                    // Pop dynamic path from stack and include
                    let path = stack.pop().unwrap_or(CfmlValue::Null).as_string();

                    let resolved = if let Some(ref source) = self.source_file {
                        let source_dir = std::path::Path::new(source)
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."));
                        normalize_path(&source_dir.join(&path).to_string_lossy())
                    } else {
                        path.clone()
                    };

                    let resolved = if !self.vfs.exists(&resolved) && path.starts_with('/') {
                        self.resolve_leading_slash_include(&path)
                            .unwrap_or(resolved)
                    } else {
                        resolved
                    };

                    let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
                    match compile_file_cached(&resolved, cache, self.vfs.as_ref()) {
                        Ok(sub_program) => {
                            let mut old_program = std::mem::replace(&mut self.program, sub_program);
                            let old_source = self.source_file.clone();
                            self.source_file = Some(resolved.clone());
                            let main_idx = self
                                .program
                                .functions
                                .iter()
                                .position(|f| f.name == "__main__")
                                .unwrap_or(0);
                            let inc_func = self.program.functions[main_idx].clone();
                            let pre_include_keys: std::collections::HashSet<String> =
                                locals.keys().cloned().collect();
                            let saved_try_stack = std::mem::take(&mut self.try_stack);
                            let result = self.execute_function_with_args(
                                &inc_func,
                                Vec::new(),
                                Some(&locals),
                            );
                            self.try_stack = saved_try_stack;
                            // Merge new non-function variables from the include
                            if let Some(inc_locals) = self.captured_locals.take() {
                                for (k, v) in inc_locals {
                                    if k == "arguments" {
                                        continue;
                                    }
                                    if !pre_include_keys.contains(&k)
                                        && !matches!(v, CfmlValue::Function(_))
                                        && !self.builtins.contains_key(&k)
                                    {
                                        locals.insert(k, v);
                                    }
                                }
                            }
                            let sub_func_count = self.program.functions.len();
                            let base_idx = old_program.functions.len();
                            for i in 0..sub_func_count {
                                if self.program.functions[i].name != "__main__" {
                                    old_program
                                        .functions
                                        .push(self.program.functions[i].clone());
                                }
                            }
                            if base_idx > 0 && sub_func_count > 1 {
                                let offset = base_idx - 1;
                                for (_, v) in locals.iter_mut() {
                                    Self::fixup_included_func_indices(v, base_idx, sub_func_count);
                                }
                                for (_, v) in self.globals.iter_mut() {
                                    Self::fixup_included_func_indices(v, base_idx, sub_func_count);
                                }
                                // Fix DefineFunction bytecode indices in merged functions
                                for fi in base_idx..old_program.functions.len() {
                                    let func = Arc::make_mut(&mut old_program.functions[fi]);
                                    for op in func.instructions.iter_mut() {
                                        if let BytecodeOp::DefineFunction(ref mut idx) = op {
                                            if *idx > 0 && *idx < sub_func_count {
                                                *idx += offset;
                                            }
                                        }
                                    }
                                }
                                // Update user_functions entries with fixed bytecode.
                                // Share the Arc with the (now fixed) old_program entry
                                // rather than deep-cloning.
                                for fi in base_idx..old_program.functions.len() {
                                    let name = old_program.functions[fi].name.clone();
                                    if self.user_functions.contains_key(&name) {
                                        self.user_functions.insert(
                                            name,
                                            Arc::clone(&old_program.functions[fi]),
                                        );
                                    }
                                }
                            }
                            self.program = old_program;
                            self.source_file = old_source;
                            if let Err(e) = result {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    let mut err_struct = IndexMap::new();
                                    err_struct.insert(
                                        "message".to_string(),
                                        CfmlValue::String(e.message.clone()),
                                    );
                                    err_struct.insert(
                                        "type".to_string(),
                                        CfmlValue::String(format!("{}", e.error_type)),
                                    );
                                    err_struct.insert(
                                        "detail".to_string(),
                                        CfmlValue::String(String::new()),
                                    );
                                    err_struct
                                        .insert("tagcontext".to_string(), self.build_tag_context());
                                    let error_val = CfmlValue::strukt(err_struct);
                                    stack.push(error_val);
                                    ip = handler.catch_ip;
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }

                BytecodeOp::Print => {
                    if let Some(val) = stack.pop() {
                        self.output_buffer.push_str(&val.as_string());
                        self.output_buffer.push('\n');
                    }
                }
                BytecodeOp::IsDefined(var_name) => {
                    let defined = self.is_variable_defined(&var_name, &locals);
                    stack.push(CfmlValue::Bool(defined));
                }

                BytecodeOp::ConcatArrays => {
                    let right = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
                    let left = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
                    if let (CfmlValue::Array(mut a), CfmlValue::Array(b)) = (left, right) {
                        Arc::make_mut(&mut a).extend(b.iter().cloned());
                        stack.push(CfmlValue::Array(a));
                    } else {
                        stack.push(CfmlValue::array(Vec::new()));
                    }
                }

                BytecodeOp::MergeStructs => {
                    let right = stack.pop().unwrap_or(CfmlValue::strukt(IndexMap::new()));
                    let left = stack.pop().unwrap_or(CfmlValue::strukt(IndexMap::new()));
                    if let (CfmlValue::Struct(mut a), CfmlValue::Struct(b)) = (left, right) {
                        for (k, v) in b.iter() {
                            Arc::make_mut(&mut a).insert(k.clone(), v.clone());
                        }
                        stack.push(CfmlValue::Struct(a));
                    } else {
                        stack.push(CfmlValue::strukt(IndexMap::new()));
                    }
                }

                BytecodeOp::CallSpread => {
                    // Stack: [func_ref, args_array]
                    let args_val = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
                    let func_ref = stack.pop().unwrap_or(CfmlValue::Null);
                    let args = if let CfmlValue::Array(a) = args_val {
                        a
                    } else {
                        Arc::new(vec![args_val])
                    };
                    self.closure_parent_writeback = None;
                    let result = self.call_function(&func_ref, (*args).clone(), &locals)?;
                    // Write back mutations into the shared closure environment
                    if let Some(ref writeback) = self.closure_parent_writeback {
                        Self::write_back_to_captured_scope(&func_ref, writeback);
                    }
                    if let Some(writeback) = self.closure_parent_writeback.take() {
                        for (k, v) in writeback {
                            locals.insert(k, v);
                        }
                    }
                    stack.push(result);
                }

                BytecodeOp::LineInfo(line, col) => {
                    self.current_line = *line;
                    self.current_column = *col;
                    // Update the current call frame's line so the stack trace
                    // reflects where execution is within this function
                    if let Some(frame) = self.call_stack.last_mut() {
                        frame.line = *line;
                    }
                }

                BytecodeOp::CallRustSuperCtor(arg_count) => {
                    let mut ctor_args: Vec<CfmlValue> =
                        (0..*arg_count).filter_map(|_| stack.pop()).collect();
                    ctor_args.reverse();

                    let this_val = locals.get("this").cloned().ok_or_else(|| {
                        CfmlError::runtime(
                            "super(...) called outside of a CFC method".to_string(),
                        )
                    })?;
                    let mut this_struct = match this_val {
                        CfmlValue::Struct(s) => s,
                        _ => {
                            return Err(CfmlError::runtime(
                                "super(...) requires `this` to be a component instance".to_string(),
                            ));
                        }
                    };
                    let rust_class = match this_struct.get("__rust_extends") {
                        Some(CfmlValue::String(n)) => n.clone(),
                        _ => {
                            return Err(CfmlError::runtime(
                                "super(...) is only valid in a CFC that extends a rust: class"
                                    .to_string(),
                            ));
                        }
                    };
                    let key = rust_class.to_lowercase();
                    let ctor = self.native_classes.get(&key).copied().ok_or_else(|| {
                        CfmlError::runtime(format!(
                            "No native (Rust) class registered with name '{}'",
                            rust_class
                        ))
                    })?;
                    let parent = ctor(ctor_args)?;
                    Arc::make_mut(&mut this_struct).insert("__super".to_string(), parent);
                    let new_this = CfmlValue::Struct(this_struct);
                    locals.insert("this".to_string(), new_this.clone());
                    self.method_this_writeback = Some(new_this);
                    stack.push(CfmlValue::Null);
                }

                BytecodeOp::Halt => break,
            }
        }

        // Pop call frame on function exit
        self.call_stack.pop();

        // Save modified 'this' and variables scope for component method write-back
        if let Some(this_val) = locals.get("this") {
            self.method_this_writeback = Some(this_val.clone());
            // Save variables scope mutations for component write-back
            if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                // With dedicated __variables scope, just pass it through
                if !vars.is_empty() {
                    self.method_variables_writeback = Some((**vars).clone());
                }
            } else {
                // Non-CFC or legacy path: collect from locals
                let mut vars_wb = IndexMap::new();
                for (k, v) in &locals {
                    let kl = k.to_lowercase();
                    if kl == "this"
                        || kl == "arguments"
                        || k.starts_with("__")
                        || func.params.contains(k)
                        || declared_locals.contains(k.as_str())
                    {
                        continue;
                    }
                    vars_wb.insert(k.clone(), v.clone());
                }
                if !vars_wb.is_empty() {
                    self.method_variables_writeback = Some(vars_wb);
                }
            }
        }

        // Closure parent scope write-back: compute diff of parent-scope vars.
        // Modern localmode (Lucee parity): no unscoped write propagates out of
        // the closure — neither new keys nor shadowing-mutations of captured
        // outer vars. All bareword writes stayed in the closure's own `local`.
        // Explicit `variables.x = …` bypasses this path, so it still works.
        if let Some(parent) = parent_scope {
            if !effective_local_mode_modern {
                let mut writeback = IndexMap::new();
                for (k, v) in &locals {
                    // Skip function params, arguments scope, 'this', var-declared
                    // locals, and __variables (handled by method_variables_writeback)
                    if k == "arguments"
                        || k == "this"
                        || k == "__variables"
                        || func.params.contains(k)
                        || declared_locals.contains(k.as_str())
                    {
                        continue;
                    }
                    if let Some(parent_val) = parent.get(k) {
                        if !Self::values_equal_shallow(v, parent_val) {
                            writeback.insert(k.clone(), v.clone());
                        }
                    } else {
                        writeback.insert(k.clone(), v.clone());
                    }
                }
                if !writeback.is_empty() {
                    self.closure_parent_writeback = Some(writeback);
                }
            }
        }

        // Pass-by-reference writeback: collect final values of complex-type params
        self.collect_arg_ref_writeback(func, &locals);

        // Capture locals for component variables scope (for __main__ and __cfc_body__)
        if func.name == "__main__" || func.name == "__cfc_body__" {
            self.captured_locals = Some(locals);
        }

        debug_assert!(
            stack.len() <= 1,
            "operand-stack discipline broken at end of {} ({} values, expected 0 or 1)",
            func.name,
            stack.len()
        );
        Ok(stack.pop().unwrap_or(CfmlValue::Null))
    }

    fn call_function(
        &mut self,
        func_ref: &CfmlValue,
        args: Vec<CfmlValue>,
        parent_locals: &IndexMap<String, CfmlValue>,
    ) -> CfmlResult {
        if let CfmlValue::Function(func) = func_ref {
            // security.disallowedFunctions: enforced at the very top so the
            // user-defined fast path below can't bypass it. Checked once per
            // call; the HashSet lookup is cheap.
            if !self.disallowed_functions.is_empty() {
                let name_lower_dis = func.name.to_lowercase();
                if self.disallowed_functions.contains(&name_lower_dis) {
                    return Err(CfmlError::runtime(format!(
                        "Function '{}' is disallowed by security policy",
                        func.name
                    )));
                }
            }
            // Fast path: if the function has a stored bytecode index, dispatch directly
            // (skips all builtin matching for user-defined functions)
            if let cfml_common::dynamic::CfmlClosureBody::Expression(ref body) = func.body {
                if let CfmlValue::Int(idx) = body.as_ref() {
                    let idx = *idx as usize;
                    if idx < self.program.functions.len() {
                        let user_func = Arc::clone(&self.program.functions[idx]);
                        // Handle closure scope merging
                        let effective_locals;
                        let effective_parent = if let Some(ref shared_env) = func.captured_scope {
                            let is_cfc_method = parent_locals.contains_key("this");
                            effective_locals = if is_cfc_method {
                                let mut merged = shared_env.read().unwrap().clone();
                                for (k, v) in parent_locals {
                                    if matches!(v, CfmlValue::Function(_))
                                        || !merged.contains_key(k)
                                    {
                                        merged.insert(k.clone(), v.clone());
                                    }
                                }
                                merged
                            } else {
                                let mut merged = shared_env.read().unwrap().clone();
                                for (k, v) in parent_locals {
                                    if !merged.contains_key(k) {
                                        merged.insert(k.clone(), v.clone());
                                    }
                                }
                                merged
                            };
                            &effective_locals
                        } else {
                            parent_locals
                        };
                        return self.execute_function_with_args(
                            &user_func,
                            args,
                            Some(effective_parent),
                        );
                    }
                }
            }

            // Check builtin functions (case-insensitive)
            let name_lower = func.name.to_lowercase();

            // writeOutput/writeDump must be handled before the builtin lookup
            // so output goes to output_buffer (not stdout via the builtin fn)
            if name_lower == "writeoutput" {
                for arg in &args {
                    self.output_buffer.push_str(&arg.as_string());
                }
                return Ok(CfmlValue::Null);
            }

            // __writeText: same as writeOutput but suppressed when enableCFOutputOnly > 0
            if name_lower == "__writetext" {
                if self.enable_cfoutput_only <= 0 {
                    for arg in &args {
                        self.output_buffer.push_str(&arg.as_string());
                    }
                }
                return Ok(CfmlValue::Null);
            }
            if name_lower == "writedump" || name_lower == "dump" {
                for arg in &args {
                    self.output_buffer.push_str(&format!("{:?}\n", arg));
                }
                return Ok(CfmlValue::Null);
            }

            // queryAppend: mutates the first query in-place, returns boolean.
            if name_lower == "queryappend" {
                if let (Some(CfmlValue::Query(q1)), Some(CfmlValue::Query(q2))) =
                    (args.first(), args.get(1))
                {
                    let mut merged = q1.clone();
                    for col in &q2.columns {
                        let col_lower = col.to_lowercase();
                        if !merged.columns.iter().any(|c| c.to_lowercase() == col_lower) {
                            merged.columns.push(col.clone());
                        }
                    }
                    for row in &q2.rows {
                        merged.rows.push(row.clone());
                    }
                    self.arg_ref_writeback = Some(vec![
                        ("0".to_string(), CfmlValue::Query(merged)),
                    ]);
                    return Ok(CfmlValue::Bool(true));
                }
                return Ok(CfmlValue::Bool(false));
            }

            // querySetRow: mutates query in-place, returns boolean.
            if name_lower == "querysetrow" {
                if let (Some(CfmlValue::Query(q)), Some(row_pos), Some(CfmlValue::Struct(new_row))) =
                    (args.first(), args.get(1), args.get(2))
                {
                    let pos = match row_pos {
                        CfmlValue::Int(i) => *i as usize,
                        CfmlValue::Double(d) => *d as usize,
                        _ => 0,
                    };
                    if pos >= 1 && pos <= q.rows.len() {
                        let mut modified = q.clone();
                        let mut row: IndexMap<String, CfmlValue> = IndexMap::new();
                        for col in &modified.columns {
                            let col_lower = col.to_lowercase();
                            let val = new_row
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == col_lower)
                                .map(|(_, v)| v.clone())
                                .unwrap_or(CfmlValue::Null);
                            row.insert(col.clone(), val);
                        }
                        modified.rows[pos - 1] = row;
                        self.arg_ref_writeback = Some(vec![
                            ("0".to_string(), CfmlValue::Query(modified)),
                        ]);
                        return Ok(CfmlValue::Bool(true));
                    }
                }
                return Ok(CfmlValue::Bool(false));
            }

            // In-place array mutators that return boolean (matches Lucee):
            // arrayDelete, arrayDeleteNoCase. Mutate the caller's array via
            // arg_ref_writeback and return true/false based on whether the
            // element was found.
            if name_lower == "arraydelete" || name_lower == "arraydeletenocase" {
                if let Some(CfmlValue::Array(arr)) = args.first() {
                    let target = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                    let (pos, found) = if name_lower == "arraydeletenocase" {
                        let t = target.to_lowercase();
                        let p = arr.iter().position(|v| v.as_string().to_lowercase() == t);
                        (p, p.is_some())
                    } else {
                        let p = arr.iter().position(|v| v.as_string() == target);
                        (p, p.is_some())
                    };
                    if let Some(p) = pos {
                        let mut new_arr = arr.clone();
                        Arc::make_mut(&mut new_arr).remove(p);
                        // Write back the mutated array to the source variable
                        self.arg_ref_writeback = Some(vec![
                            ("0".to_string(), CfmlValue::Array(new_arr)),
                        ]);
                    }
                    return Ok(CfmlValue::Bool(found));
                }
                return Ok(CfmlValue::Bool(false));
            }

            // Higher-order functions must be handled BEFORE regular builtins
            // because they need VM access to invoke closures
            match name_lower.as_str() {
                "arraymap"
                | "arrayfilter"
                | "arrayreduce"
                | "arrayeach"
                | "arraysome"
                | "arrayevery"
                | "arrayfindall"
                | "arrayfindallnocase"
                | "structeach"
                | "structmap"
                | "structfilter"
                | "structreduce"
                | "structsome"
                | "structevery"
                | "listeach"
                | "listmap"
                | "listfilter"
                | "listreduce"
                | "listsome"
                | "listevery"
                | "listreduceright"
                | "stringeach"
                | "stringmap"
                | "stringfilter"
                | "stringreduce"
                | "stringsome"
                | "stringevery"
                | "stringsort"
                | "collectioneach"
                | "collectionmap"
                | "collectionfilter"
                | "collectionreduce"
                | "collectionsome"
                | "collectionevery"
                | "each"
                | "queryeach"
                | "querymap"
                | "queryfilter"
                | "queryreduce"
                | "querysort"
                | "querysome"
                | "queryevery"
                | "queryaddrow"
                | "querysetcell"
                | "createobject"
                | "getcurrenttemplatepath"
                | "getcomponentmetadata"
                | "__cfheader"
                | "__cfcontent"
                | "__cflocation"
                | "__cfabort"
                | "gethttprequestdata"
                | "__cfinvoke"
                | "__cfsavecontent_start"
                | "__cfsavecontent_end"
                | "invoke"
                | "getbasetemplatepath"
                | "gettimezone"
                | "expandpath"
                | "isdefined"
                | "__cfparam"
                | "queryexecute"
                | "__cftransaction_start"
                | "__cftransaction_commit"
                | "__cftransaction_rollback"
                | "__writetext"
                | "__cflog"
                | "__cfsetting"
                | "__cflock_start"
                | "__cflock_end"
                | "__cfcookie"
                | "fileupload"
                | "fileuploadall"
                | "__cffile_upload"
                | "sessioninvalidate"
                | "sessionrotate"
                | "sessiongetmetadata"
                | "getauthuser"
                | "isuserinrole"
                | "isuserloggedin"
                | "__cfloginuser"
                | "__cflogout"
                | "setvariable"
                | "getvariable"
                | "throw"
                | "__cfcustomtag"
                | "__cfcustomtag_start"
                | "__cfcustomtag_end"
                | "cacheput"
                | "cacheget"
                | "cachedelete"
                | "cacheclear"
                | "cachekeyexists"
                | "cachecount"
                | "cachegetall"
                | "cachegetallids"
                | "__cfcache"
                | "__cfexecute"
                | "__cfthread_run"
                | "__cfthread_join"
                | "__cfthread_terminate"
                | "callstackget"
                | "callstackdump"
                | "precisionevaluate" => {
                    // Will be handled at the end of this function (needs VM access)
                }
                _ => {
                    // S3 transparent VFS: intercept file/directory ops where the first
                    // path argument is an `s3://` URL.
                    #[cfg(feature = "s3")]
                    {
                        if let Some(result) = self.s3_intercept(&name_lower, &args) {
                            return result;
                        }
                    }
                    // Sandbox mode: intercept file operations
                    if self.sandbox {
                        if let Some(result) = self.sandbox_intercept(&name_lower, &args) {
                            return result;
                        }
                    }
                    // Try exact match first, then case-insensitive
                    if let Some(builtin) = self.builtins.get(&func.name) {
                        return builtin(args);
                    }

                    // Case-insensitive builtin lookup
                    let builtin_match = self
                        .builtins
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == name_lower)
                        .map(|(_, v)| *v);

                    if let Some(builtin) = builtin_match {
                        return builtin(args);
                    }
                }
            }

            // Check user-defined functions by name
            // If the function reference carries a captured scope (from LoadGlobal),
            // merge it with parent_locals so the function retains access to its
            // defining scope's variables when called from a different context.
            if let Some(user_func) = self.user_functions.get(&func.name).cloned() {
                let effective_parent;
                let parent = if let Some(ref shared_env) = func.captured_scope {
                    effective_parent = {
                        let mut merged = shared_env.read().unwrap().clone();
                        for (k, v) in parent_locals {
                            if matches!(v, CfmlValue::Function(_)) || !merged.contains_key(k) {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                        merged
                    };
                    &effective_parent
                } else {
                    parent_locals
                };
                return self.execute_function_with_args(&user_func, args, Some(parent));
            }

            // Case-insensitive user function lookup
            let user_match = self
                .user_functions
                .iter()
                .find(|(k, _)| k.to_lowercase() == name_lower)
                .map(|(_, v)| v.clone());

            if let Some(user_func) = user_match {
                let effective_parent;
                let parent = if let Some(ref shared_env) = func.captured_scope {
                    effective_parent = {
                        let mut merged = shared_env.read().unwrap().clone();
                        for (k, v) in parent_locals {
                            if matches!(v, CfmlValue::Function(_)) || !merged.contains_key(k) {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                        merged
                    };
                    &effective_parent
                } else {
                    parent_locals
                };
                return self.execute_function_with_args(&user_func, args, Some(parent));
            }

            // Higher-order standalone functions (arrayMap, arrayFilter, arrayReduce, etc.)
            match name_lower.as_str() {
                "arraymap" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let mut result = Vec::with_capacity(arr.len());
                            let callback = callback.clone();
                            // Lazily materialize parent_locals copy only if a
                            // writeback arrives. Most callbacks don't write back,
                            // so this skips a full map clone per call.
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                let mapped = self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k, v) in &wb {
                                        pl_ref.insert(k.clone(), v.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                                result.push(mapped);
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                            return Ok(CfmlValue::array(result));
                        }
                    }
                    return Ok(CfmlValue::array(Vec::new()));
                }
                "arrayfilter" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let mut result = Vec::new();
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                let keep = self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k, v) in &wb {
                                        pl_ref.insert(k.clone(), v.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                                if keep.is_true() {
                                    result.push(item.clone());
                                }
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                            return Ok(CfmlValue::array(result));
                        }
                    }
                    return Ok(CfmlValue::array(Vec::new()));
                }
                "arrayfindall" | "arrayfindallnocase" => {
                    // arrayFindAll(array, callback) - callback(item, index, array)
                    // When called with a callback, returns indices where callback returns true
                    if let (Some(arr_val), Some(arg1)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            // Check if second arg is a callback (Function) or a simple value
                            if matches!(arg1, CfmlValue::Function(_)) {
                                let callback = arg1.clone();
                                let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                                let mut result = Vec::new();
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(arr_val.clone());
                                    self.closure_parent_writeback = None;
                                    let scope = pl.as_ref().unwrap_or(parent_locals);
                                    let keep = self.call_function(&callback, cb_args, scope)?;
                                    if let Some(wb) = self.closure_parent_writeback.take() {
                                        let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                        for (k, v) in &wb {
                                            pl_ref.insert(k.clone(), v.clone());
                                        }
                                        Self::write_back_to_captured_scope(&callback, &wb);
                                        self.closure_parent_writeback = Some(wb);
                                    }
                                    if keep.is_true() {
                                        result.push(CfmlValue::Int((i + 1) as i64));
                                    }
                                }
                                if let Some(ref pl_ref) = pl {
                                    self.set_ho_final_writeback(pl_ref, parent_locals);
                                }
                                return Ok(CfmlValue::array(result));
                            } else {
                                // Simple value comparison: fall through to builtin
                            }
                        }
                    }
                    // Fall through to the builtin fn_array_find_all for simple value comparison
                }
                "arrayreduce" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(4);
                                cb_args.push(acc.clone());
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                acc = self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k, v) in &wb {
                                        pl_ref.insert(k.clone(), v.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                            return Ok(acc);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "arrayeach" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k, v) in &wb {
                                        pl_ref.insert(k.clone(), v.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "structeach" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k, v) in &wb {
                                        pl_ref.insert(k.clone(), v.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "structmap" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let mut result = IndexMap::new();
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                let mapped = self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k2, v2) in &wb {
                                        pl_ref.insert(k2.clone(), v2.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                                result.insert(k.clone(), mapped);
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                            return Ok(CfmlValue::strukt(result));
                        }
                    }
                    return Ok(CfmlValue::strukt(IndexMap::new()));
                }
                "structfilter" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let mut result = IndexMap::new();
                            let callback = callback.clone();
                            let mut pl: Option<IndexMap<String, CfmlValue>> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                let scope = pl.as_ref().unwrap_or(parent_locals);
                                let keep = self.call_function(&callback, cb_args, scope)?;
                                if let Some(wb) = self.closure_parent_writeback.take() {
                                    let pl_ref = pl.get_or_insert_with(|| parent_locals.clone());
                                    for (k2, v2) in &wb {
                                        pl_ref.insert(k2.clone(), v2.clone());
                                    }
                                    Self::write_back_to_captured_scope(&callback, &wb);
                                    self.closure_parent_writeback = Some(wb);
                                }
                                if keep.is_true() {
                                    result.insert(k.clone(), v.clone());
                                }
                            }
                            if let Some(ref pl_ref) = pl {
                                self.set_ho_final_writeback(pl_ref, parent_locals);
                            }
                            return Ok(CfmlValue::strukt(result));
                        }
                    }
                    return Ok(CfmlValue::strukt(IndexMap::new()));
                }
                "arraysome" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let callback = callback.clone();
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if result.is_true() {
                                    return Ok(CfmlValue::Bool(true));
                                }
                            }
                            return Ok(CfmlValue::Bool(false));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "arrayevery" => {
                    if let (Some(arr_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Array(arr) = arr_val {
                            let callback = callback.clone();
                            for (i, item) in arr.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(item.clone());
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(arr_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if !result.is_true() {
                                    return Ok(CfmlValue::Bool(false));
                                }
                            }
                            return Ok(CfmlValue::Bool(true));
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "structreduce" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                            let callback = callback.clone();
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(4);
                                cb_args.push(acc.clone());
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                acc = self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                            }
                            return Ok(acc);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "structsome" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let callback = callback.clone();
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if result.is_true() {
                                    return Ok(CfmlValue::Bool(true));
                                }
                            }
                            return Ok(CfmlValue::Bool(false));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "structevery" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let callback = callback.clone();
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::String(k.clone()));
                                cb_args.push(v.clone());
                                cb_args.push(struct_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if !result.is_true() {
                                    return Ok(CfmlValue::Bool(false));
                                }
                            }
                            return Ok(CfmlValue::Bool(true));
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "listeach" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let delimiter = args
                            .get(2)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "listmap" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let delimiter = args
                            .get(2)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        let mut result = Vec::with_capacity(items.len());
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            let mapped = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.push(mapped.as_string());
                        }
                        return Ok(CfmlValue::String(result.join(&first_delim)));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "listfilter" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let delimiter = args
                            .get(2)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        let mut result = Vec::new();
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                result.push(item.to_string());
                            }
                        }
                        return Ok(CfmlValue::String(result.join(&first_delim)));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "listreduce" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                        let delimiter = args
                            .get(3)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "listreduceright" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                        let delimiter = args
                            .get(3)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        for (i, item) in items.iter().enumerate().rev() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "listsome" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let delimiter = args
                            .get(2)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            let result = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if result.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "listevery" => {
                    if let (Some(list_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let list = list_val.as_string();
                        let delimiter = args
                            .get(2)
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let callback = callback.clone();
                        let items: Vec<&str> = list
                            .split(|c: char| delimiter.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect();
                        for (i, item) in items.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            let result = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if !result.is_true() {
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                // ---- String Higher-Order Functions ----
                "stringeach" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "stringmap" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        let mut result = String::new();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            let mapped = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.push_str(&mapped.as_string());
                        }
                        return Ok(CfmlValue::String(result));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "stringfilter" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        let mut result = String::new();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                result.push(ch);
                            }
                        }
                        return Ok(CfmlValue::String(result));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "stringreduce" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                        let callback = callback.clone();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "stringsome" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            let result = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if result.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "stringevery" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            let result = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if !result.is_true() {
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "stringsort" => {
                    if let Some(str_val) = args.get(0) {
                        let s = str_val.as_string();
                        let mut chars: Vec<char> = s.chars().collect();
                        if let Some(callback) = args.get(1) {
                            let callback = callback.clone();
                            // Bubble sort with callback comparator
                            let len = chars.len();
                            for i in 0..len {
                                for j in 0..len - 1 - i {
                                    let cb_args = vec![
                                        CfmlValue::String(chars[j].to_string()),
                                        CfmlValue::String(chars[j + 1].to_string()),
                                    ];
                                    self.closure_parent_writeback = None;
                                    let cmp =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    let cmp_val = match &cmp {
                                        CfmlValue::Int(n) => *n,
                                        CfmlValue::Double(d) => *d as i64,
                                        _ => 0,
                                    };
                                    if cmp_val > 0 {
                                        chars.swap(j, j + 1);
                                    }
                                }
                            }
                        } else {
                            chars.sort();
                        }
                        return Ok(CfmlValue::String(chars.into_iter().collect()));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                // ---- Collection Higher-Order Functions ----
                "collectioneach" | "each" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                            CfmlValue::Struct(s) => {
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                            CfmlValue::Query(q) => {
                                for (i, row) in q.rows.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::strukt(row.clone()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                            _ => {
                                // Treat as list
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "collectionmap" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                let mut result = Vec::with_capacity(arr.len());
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let mapped =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    result.push(mapped);
                                }
                                return Ok(CfmlValue::array(result));
                            }
                            CfmlValue::Struct(s) => {
                                let mut result = IndexMap::new();
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let mapped =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    result.insert(key.clone(), mapped);
                                }
                                return Ok(CfmlValue::strukt(result));
                            }
                            _ => {
                                // Treat as list
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                let mut result: Vec<String> = Vec::with_capacity(items.len());
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let mapped =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    result.push(mapped.as_string());
                                }
                                return Ok(CfmlValue::String(result.join(",")));
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "collectionfilter" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                let mut result = Vec::new();
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let keep =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if keep.is_true() {
                                        result.push(item.clone());
                                    }
                                }
                                return Ok(CfmlValue::array(result));
                            }
                            CfmlValue::Struct(s) => {
                                let mut result = IndexMap::new();
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let keep =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if keep.is_true() {
                                        result.insert(key.clone(), val.clone());
                                    }
                                }
                                return Ok(CfmlValue::strukt(result));
                            }
                            _ => {
                                // Treat as list
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                let mut result = Vec::new();
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let keep =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if keep.is_true() {
                                        result.push(item.to_string());
                                    }
                                }
                                return Ok(CfmlValue::String(result.join(",")));
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "collectionreduce" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(4);
                                    cb_args.push(acc.clone());
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    acc = self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                            CfmlValue::Struct(s) => {
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(4);
                                    cb_args.push(acc.clone());
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    acc = self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                            _ => {
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(4);
                                    cb_args.push(acc.clone());
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    acc = self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                }
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "collectionsome" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if result.is_true() {
                                        return Ok(CfmlValue::Bool(true));
                                    }
                                }
                            }
                            CfmlValue::Struct(s) => {
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if result.is_true() {
                                        return Ok(CfmlValue::Bool(true));
                                    }
                                }
                            }
                            _ => {
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if result.is_true() {
                                        return Ok(CfmlValue::Bool(true));
                                    }
                                }
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "collectionevery" => {
                    if let (Some(collection), Some(callback)) = (args.get(0), args.get(1)) {
                        let callback = callback.clone();
                        match collection {
                            CfmlValue::Array(arr) => {
                                for (i, item) in arr.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(item.clone());
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if !result.is_true() {
                                        return Ok(CfmlValue::Bool(false));
                                    }
                                }
                            }
                            CfmlValue::Struct(s) => {
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(key.clone()));
                                    cb_args.push(val.clone());
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if !result.is_true() {
                                        return Ok(CfmlValue::Bool(false));
                                    }
                                }
                            }
                            _ => {
                                let list = collection.as_string();
                                let items: Vec<&str> =
                                    list.split(',').filter(|s| !s.is_empty()).collect();
                                for (i, item) in items.iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::String(item.to_string()));
                                    cb_args.push(CfmlValue::Int((i + 1) as i64));
                                    cb_args.push(collection.clone());
                                    self.closure_parent_writeback = None;
                                    let result =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    if !result.is_true() {
                                        return Ok(CfmlValue::Bool(false));
                                    }
                                }
                            }
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "queryeach" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                            }
                        }
                    }
                    self.arg_ref_writeback = None;
                    return Ok(CfmlValue::Null);
                }
                "querymap" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            let mut new_rows = Vec::with_capacity(q.rows.len());
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                let mapped =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if let CfmlValue::Struct(s) = mapped {
                                    new_rows.push(s);
                                } else {
                                    new_rows.push(Arc::new(row.clone()));
                                }
                            }
                            let mut result = q.clone();
                            result.rows = new_rows.into_iter().map(|a| (*a).clone()).collect();
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(result));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "queryfilter" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            let mut new_rows = Vec::new();
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                let keep = self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if keep.is_true() {
                                    new_rows.push(row.clone());
                                }
                            }
                            let mut result = q.clone();
                            result.rows = new_rows;
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(result));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "queryreduce" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                            let callback = callback.clone();
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(4);
                                cb_args.push(acc.clone());
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                acc = self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                            }
                            self.arg_ref_writeback = None;
                            return Ok(acc);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "querysort" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            let mut rows = q.rows.clone();
                            // Bubble sort (closure calls can't be used with sort_by)
                            let n = rows.len();
                            for i in 0..n {
                                for j in 0..n - 1 - i {
                                    let a = CfmlValue::strukt(rows[j].clone());
                                    let b = CfmlValue::strukt(rows[j + 1].clone());
                                    let cb_args = vec![a, b];
                                    self.closure_parent_writeback = None;
                                    let cmp =
                                        self.call_function(&callback, cb_args, parent_locals)?;
                                    if let Some(ref wb) = self.closure_parent_writeback {
                                        Self::write_back_to_captured_scope(&callback, wb);
                                    }
                                    let cmp_val = match &cmp {
                                        CfmlValue::Int(n) => *n,
                                        CfmlValue::Double(d) => *d as i64,
                                        _ => 0,
                                    };
                                    if cmp_val > 0 {
                                        rows.swap(j, j + 1);
                                    }
                                }
                            }
                            let mut result = q.clone();
                            result.rows = rows;
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(result));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "querysome" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if result.is_true() {
                                    self.arg_ref_writeback = None;
                                    return Ok(CfmlValue::Bool(true));
                                }
                            }
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Bool(false));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "queryevery" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            for (i, row) in q.rows.iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row.clone()));
                                cb_args.push(CfmlValue::Int((i + 1) as i64));
                                cb_args.push(q_val.clone());
                                self.closure_parent_writeback = None;
                                let result =
                                    self.call_function(&callback, cb_args, parent_locals)?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                if !result.is_true() {
                                    self.arg_ref_writeback = None;
                                    return Ok(CfmlValue::Bool(false));
                                }
                            }
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Bool(true));
                        }
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "queryaddrow" => {
                    // Mutate query in place (like Lucee/BoxLang), return new row count
                    if let Some(CfmlValue::Query(q)) = args.first() {
                        let mut result = q.clone();
                        if args.len() >= 2 {
                            match &args[1] {
                                CfmlValue::Int(n) => {
                                    for _ in 0..*n {
                                        result.rows.push(IndexMap::new());
                                    }
                                }
                                CfmlValue::Struct(data) => {
                                    result.rows.push((**data).clone());
                                }
                                CfmlValue::Array(rows) => {
                                    for item in rows.iter() {
                                        if let CfmlValue::Struct(data) = item {
                                            result.rows.push((**data).clone());
                                        } else {
                                            result.rows.push(IndexMap::new());
                                        }
                                    }
                                }
                                _ => {
                                    result.rows.push(IndexMap::new());
                                }
                            }
                        } else {
                            result.rows.push(IndexMap::new());
                        }
                        let row_count = result.rows.len() as i64;
                        // Write mutated query back to caller via arg_ref_writeback
                        self.arg_ref_writeback =
                            Some(vec![("0".to_string(), CfmlValue::Query(result))]);
                        return Ok(CfmlValue::Int(row_count));
                    }
                    return Ok(CfmlValue::Int(0));
                }
                "querysetcell" => {
                    // Mutate query in place (like Lucee/BoxLang), return true
                    if args.len() >= 3 {
                        if let CfmlValue::Query(q) = &args[0] {
                            let mut result = q.clone();
                            let column = args[1].as_string();
                            let value = args[2].clone();
                            let row_idx = if args.len() >= 4 {
                                match &args[3] {
                                    CfmlValue::Int(n) => (*n as usize).saturating_sub(1),
                                    _ => result.rows.len().saturating_sub(1),
                                }
                            } else {
                                result.rows.len().saturating_sub(1)
                            };
                            if row_idx < result.rows.len() {
                                result.rows[row_idx].insert(column, value);
                            }
                            self.arg_ref_writeback =
                                Some(vec![("0".to_string(), CfmlValue::Query(result))]);
                            return Ok(CfmlValue::Bool(true));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "getcurrenttemplatepath" => {
                    if let Some(ref source) = self.source_file {
                        if let Ok(abs) = self.vfs.canonicalize(source) {
                            return Ok(CfmlValue::String(abs));
                        }
                        return Ok(CfmlValue::String(source.clone()));
                    }
                    // Fallback to CWD
                    if let Ok(cwd) = std::env::current_dir() {
                        return Ok(CfmlValue::String(cwd.to_string_lossy().to_string()));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "getbasetemplatepath" => {
                    if let Some(ref base) = self.base_template_path {
                        if let Ok(abs) = self.vfs.canonicalize(base) {
                            return Ok(CfmlValue::String(abs));
                        }
                        return Ok(CfmlValue::String(base.clone()));
                    }
                    // Fall back to source_file
                    if let Some(ref source) = self.source_file {
                        if let Ok(abs) = self.vfs.canonicalize(source) {
                            return Ok(CfmlValue::String(abs));
                        }
                        return Ok(CfmlValue::String(source.clone()));
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "expandpath" => {
                    // CFML expandPath: resolve relative to current template dir,
                    // absolute paths (starting with /) resolve via mappings then source dir
                    let rel = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let base_dir = self
                        .source_file
                        .as_ref()
                        .and_then(|s| std::path::Path::new(s).parent())
                        .unwrap_or_else(|| std::path::Path::new("."));

                    let resolved = if rel.starts_with('/') {
                        // Try mappings first
                        let mut found = None;
                        for mapping in &self.mappings {
                            let prefix = mapping.name.trim_end_matches('/');
                            if rel.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                let remainder = &rel[prefix.len()..];
                                let remainder = remainder.trim_start_matches('/');
                                let candidate =
                                    std::path::PathBuf::from(&mapping.path).join(remainder);
                                found = Some(candidate);
                                break;
                            }
                        }
                        found.unwrap_or_else(|| base_dir.join(rel.trim_start_matches('/')))
                    } else {
                        base_dir.join(&rel)
                    };

                    // Canonicalize if it exists, otherwise return the joined path
                    let path_str = resolved.to_string_lossy().to_string();
                    let result = self.vfs.canonicalize(&path_str).unwrap_or(path_str);
                    return Ok(CfmlValue::String(result));
                }
                "isdefined" => {
                    // Runtime isDefined: argument is a string variable name
                    let var_name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let defined = self.is_variable_defined(&var_name, parent_locals);
                    return Ok(CfmlValue::Bool(defined));
                }
                "__cfparam" => {
                    // Runtime fallback for `param name="<dynamic>" default=<v>`
                    // when the parser couldn't lower it statically. We can only
                    // mutate scopes we own through `&mut self` (variables /
                    // request / application / session) — caller-locals paths
                    // are handled by the parser-level lowering instead.
                    let var_name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let default_val = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                    if self.is_variable_defined(&var_name, parent_locals) {
                        return Ok(CfmlValue::Null);
                    }
                    let lower = var_name.to_lowercase();
                    let (scope, key) = if lower.starts_with("variables.") {
                        ("variables", &var_name[10..])
                    } else if lower.starts_with("request.") {
                        ("request", &var_name[8..])
                    } else if lower.starts_with("session.") {
                        ("session", &var_name[8..])
                    } else if lower.starts_with("application.") {
                        ("application", &var_name[12..])
                    } else {
                        // No mutable scope prefix — default to variables.
                        ("variables", var_name.as_str())
                    };
                    if key.is_empty() {
                        return Ok(CfmlValue::Null);
                    }
                    // Only support a flat key (no further '.' or brackets) at
                    // runtime. Nested-path mutation through caller locals is
                    // the parser's job.
                    if key.contains('.') || key.contains('[') {
                        return Err(CfmlError::runtime(format!(
                            "param: dynamic name '{}' addresses a nested path that cannot be assigned at runtime",
                            var_name
                        )));
                    }
                    match scope {
                        "variables" => {
                            self.globals.insert(key.to_string(), default_val);
                        }
                        "request" => {
                            self.request_scope.insert(key.to_string(), default_val);
                        }
                        "session" => {
                            self.set_session_variable(key, default_val);
                        }
                        "application" => {
                            if let Some(ref app_scope) = self.application_scope {
                                if let Ok(mut scope) = app_scope.lock() {
                                    scope.insert(key.to_string(), default_val);
                                }
                            }
                        }
                        _ => {}
                    }
                    return Ok(CfmlValue::Null);
                }
                "gettimezone" => {
                    // Return the system timezone name
                    // Try to get IANA timezone from environment variable first
                    if let Ok(tz) = std::env::var("TZ") {
                        if !tz.is_empty() {
                            return Ok(CfmlValue::String(tz));
                        }
                    }
                    // macOS/Linux: read /etc/localtime symlink target
                    #[cfg(unix)]
                    {
                        if let Ok(link) = std::fs::read_link("/etc/localtime") {
                            let link_str = link.to_string_lossy().to_string();
                            // Extract timezone from path like /usr/share/zoneinfo/America/New_York
                            if let Some(pos) = link_str.find("zoneinfo/") {
                                let tz = &link_str[pos + 9..];
                                return Ok(CfmlValue::String(tz.to_string()));
                            }
                        }
                    }
                    // Fallback: return UTC
                    return Ok(CfmlValue::String("UTC".to_string()));
                }
                "getcomponentmetadata" => {
                    // Helper: extract metadata from a component struct
                    fn extract_component_meta(
                        s: &IndexMap<String, CfmlValue>,
                        fallback_name: &str,
                    ) -> CfmlValue {
                        let mut meta = IndexMap::new();
                        meta.insert(
                            "name".to_string(),
                            s.get("__name")
                                .cloned()
                                .unwrap_or(CfmlValue::String(fallback_name.to_string())),
                        );
                        if let Some(chain) = s.get("__extends_chain") {
                            if let CfmlValue::Array(arr) = chain {
                                if let Some(first) = arr.first() {
                                    meta.insert("extends".to_string(), first.clone());
                                }
                            }
                        }
                        let mut functions = Vec::new();
                        for (k, v) in s {
                            if let CfmlValue::Function(f) = v {
                                if !k.starts_with("__") {
                                    let mut func_meta = IndexMap::new();
                                    func_meta
                                        .insert("name".to_string(), CfmlValue::String(k.clone()));
                                    func_meta.insert(
                                        "access".to_string(),
                                        CfmlValue::String(format!("{:?}", f.access).to_lowercase()),
                                    );
                                    if let Some(ref rt) = f.return_type {
                                        func_meta.insert(
                                            "returntype".to_string(),
                                            CfmlValue::String(rt.clone()),
                                        );
                                    }
                                    let params: Vec<CfmlValue> = f
                                        .params
                                        .iter()
                                        .map(|p| CfmlValue::String(p.name.clone()))
                                        .collect();
                                    func_meta
                                        .insert("parameters".to_string(), CfmlValue::array(params));
                                    // Merge custom function metadata emitted as __funcmeta_<name>
                                    let fmeta_key = format!("__funcmeta_{}", k);
                                    if let Some(CfmlValue::Struct(fm)) = s.get(&fmeta_key) {
                                        for (mk, mv) in fm.iter() {
                                            if !func_meta.contains_key(mk) {
                                                func_meta.insert(mk.clone(), mv.clone());
                                            }
                                        }
                                    }
                                    functions.push(CfmlValue::strukt(func_meta));
                                }
                            }
                        }
                        meta.insert("functions".to_string(), CfmlValue::array(functions));
                        if let Some(md) = s.get("__metadata") {
                            meta.insert("metadata".to_string(), md.clone());
                        }
                        if let Some(props) = s.get("__properties") {
                            meta.insert("properties".to_string(), props.clone());
                        }
                        CfmlValue::strukt(meta)
                    }

                    if let Some(arg) = args.get(0) {
                        // If the argument is already a struct (component instance), extract metadata directly
                        if let CfmlValue::Struct(ref s) = arg {
                            return Ok(extract_component_meta(s, ""));
                        }
                        // Otherwise treat as a component name/path to look up
                        let comp_name = arg.as_string();
                        if let Some(template) =
                            self.resolve_component_template(&comp_name, parent_locals)
                        {
                            let resolved = self.resolve_inheritance(template, parent_locals);
                            if let CfmlValue::Struct(ref s) = resolved {
                                return Ok(extract_component_meta(s, &comp_name));
                            }
                            return Ok(resolved);
                        }
                    }
                    return Ok(CfmlValue::strukt(IndexMap::new()));
                }
                "createobject" => {
                    if args.len() >= 2 {
                        let obj_type = args[0].as_string().to_lowercase();
                        // security.disallowedImports: block component / rust
                        // paths whose argument matches any compiled pattern.
                        if !self.disallowed_imports.is_empty()
                            && (obj_type == "component" || obj_type == "rust")
                        {
                            let target = args[1].as_string();
                            if self
                                .disallowed_imports
                                .iter()
                                .any(|re| re.is_match(&target))
                            {
                                return Err(CfmlError::runtime(format!(
                                    "createObject('{}', '{}') is disallowed by security policy",
                                    obj_type, target
                                )));
                            }
                        }
                        if obj_type == "component" {
                            let comp_name = args[1].as_string();
                            if let Some(template) =
                                self.resolve_component_template(&comp_name, parent_locals)
                            {
                                let instance = self.resolve_inheritance(template, parent_locals);
                                return self.attach_native_parent(instance);
                            }
                        } else if obj_type == "rust" {
                            let class_name = args[1].as_string();
                            let key = class_name.to_lowercase();
                            if let Some(ctor) = self.native_classes.get(&key).copied() {
                                let ctor_args: Vec<CfmlValue> =
                                    args.iter().skip(2).cloned().collect();
                                return ctor(ctor_args);
                            }
                            return Err(CfmlError::runtime(format!(
                                "No native (Rust) class registered with name '{}'",
                                class_name
                            )));
                        } else if obj_type == "java" {
                            let class_name = args[1].as_string().to_lowercase();
                            let empty_args: Vec<CfmlValue> = vec![];
                            return match class_name.as_str() {
                                "java.security.messagedigest" => {
                                    handle_java_messagedigest("init", empty_args, &CfmlValue::Null)
                                }
                                "java.util.uuid" => {
                                    handle_java_uuid("init", empty_args, &CfmlValue::Null)
                                }
                                "java.lang.thread" => {
                                    handle_java_thread("init", empty_args, &CfmlValue::Null)
                                }
                                "java.net.inetaddress" => {
                                    handle_java_inetaddress("init", empty_args, &CfmlValue::Null)
                                }
                                "java.io.file" => {
                                    handle_java_file("init", empty_args, &CfmlValue::Null)
                                }
                                "java.lang.system" => {
                                    handle_java_system("init", empty_args, &CfmlValue::Null)
                                }
                                "java.lang.stringbuilder" | "java.lang.stringbuffer" => {
                                    handle_java_stringbuilder("init", empty_args, &CfmlValue::Null)
                                }
                                "java.util.treemap" => {
                                    handle_java_treemap("init", empty_args, &CfmlValue::Null)
                                }
                                "java.util.linkedhashmap" => {
                                    handle_java_linkedhashmap("init", empty_args, &CfmlValue::Null)
                                }
                                "java.util.concurrent.linkedqueue"
                                | "java.util.concurrent.concurrentlinkedqueue" => {
                                    handle_java_concurrentlinkedqueue(
                                        "init",
                                        empty_args,
                                        &CfmlValue::Null,
                                    )
                                }
                                "java.util.concurrent.concurrenthashmap" => {
                                    handle_java_concurrenthashmap(
                                        "init",
                                        empty_args,
                                        &CfmlValue::Null,
                                    )
                                }
                                "java.util.collections" => {
                                    handle_java_collections(
                                        "init",
                                        empty_args,
                                        &CfmlValue::Null,
                                    )
                                }
                                "java.nio.file.paths" | "java.nio.file.path" => {
                                    handle_java_paths("init", empty_args, &CfmlValue::Null)
                                }
                                _ => Ok(CfmlValue::Null),
                            };
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfheader" => {
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        if let Some(code_val) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "statuscode")
                            .map(|(_, v)| v.clone())
                        {
                            let code = match &code_val {
                                CfmlValue::Int(n) => *n as u16,
                                CfmlValue::String(s) => s.parse::<u16>().unwrap_or(200),
                                CfmlValue::Double(d) => *d as u16,
                                _ => 200,
                            };
                            let text = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "statustext")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_else(|| "OK".to_string());
                            self.response_status = Some((code, text));
                        } else if let Some(name_val) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "name")
                            .map(|(_, v)| v.as_string())
                        {
                            let value = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "value")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_default();
                            self.response_headers.push((name_val, value));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfcontent" => {
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        if let Some(reset_val) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "reset")
                            .map(|(_, v)| v.clone())
                        {
                            if reset_val.is_true() {
                                self.output_buffer.clear();
                            }
                        }
                        if let Some(ct) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "type")
                            .map(|(_, v)| v.as_string())
                        {
                            self.response_content_type = Some(ct);
                        }
                        if let Some(var_val) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "variable")
                            .map(|(_, v)| v.clone())
                        {
                            self.response_body = Some(var_val);
                        }
                        if let Some(file_path) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "file")
                            .map(|(_, v)| v.as_string())
                        {
                            if let Ok(contents) = std::fs::read_to_string(&file_path) {
                                self.response_body = Some(CfmlValue::String(contents));
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfabort" => {
                    return Err(CfmlError::new(
                        "__cfabort".to_string(),
                        CfmlErrorType::Custom("abort".to_string()),
                    ));
                }
                "__cflocation" => {
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        let url = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "url")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let status_code = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "statuscode")
                            .map(|(_, v)| match v {
                                CfmlValue::Int(n) => *n as u16,
                                CfmlValue::String(s) => s.parse::<u16>().unwrap_or(302),
                                CfmlValue::Double(d) => *d as u16,
                                _ => 302,
                            })
                            .unwrap_or(302);
                        self.redirect_url = Some(url.clone());
                        self.response_headers.push(("Location".to_string(), url));
                        self.response_status = Some((status_code, "Found".to_string()));
                        return Err(CfmlError::new(
                            "__cflocation_redirect".to_string(),
                            CfmlErrorType::Custom("redirect".to_string()),
                        ));
                    }
                    return Ok(CfmlValue::Null);
                }
                "gethttprequestdata" => {
                    if let Some(ref data) = self.http_request_data {
                        return Ok(data.clone());
                    }
                    let mut empty = IndexMap::new();
                    empty.insert("headers".to_string(), CfmlValue::strukt(IndexMap::new()));
                    empty.insert("content".to_string(), CfmlValue::String(String::new()));
                    empty.insert("method".to_string(), CfmlValue::String(String::new()));
                    empty.insert("protocol".to_string(), CfmlValue::String(String::new()));
                    return Ok(CfmlValue::strukt(empty));
                }
                "__cfinvoke" => {
                    let comp_val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
                    let method_name = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                    let invoke_args = args.get(2).cloned().unwrap_or(CfmlValue::Null);

                    let component = match &comp_val {
                        CfmlValue::Struct(_) => comp_val.clone(),
                        CfmlValue::String(name) => {
                            if let Some(template) =
                                self.resolve_component_template(name, parent_locals)
                            {
                                self.resolve_inheritance(template, parent_locals)
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Component '{}' not found",
                                    name
                                )));
                            }
                        }
                        _ => {
                            let name = comp_val.as_string();
                            if let Some(template) =
                                self.resolve_component_template(&name, parent_locals)
                            {
                                self.resolve_inheritance(template, parent_locals)
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Component '{}' not found",
                                    name
                                )));
                            }
                        }
                    };

                    let method_lower = method_name.to_lowercase();
                    if let CfmlValue::Struct(ref comp_struct) = component {
                        let method_func = comp_struct
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == method_lower)
                            .map(|(_, v)| v.clone());

                        if let Some(func @ CfmlValue::Function(_)) = method_func {
                            let call_args = if let CfmlValue::Struct(ref arg_map) = invoke_args {
                                if arg_map.is_empty() {
                                    Vec::new()
                                } else if let CfmlValue::Function(ref f) = func {
                                    // Get param names from bytecode function if CfmlFunction.params is empty
                                    let param_names: Vec<String> = if f.params.is_empty() {
                                        // Look up actual bytecode function params via func_idx
                                        if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                            ref body,
                                        ) = f.body
                                        {
                                            if let CfmlValue::Int(idx) = body.as_ref() {
                                                let idx = *idx as usize;
                                                if idx < self.program.functions.len() {
                                                    self.program.functions[idx].params.clone()
                                                } else {
                                                    Vec::new()
                                                }
                                            } else {
                                                Vec::new()
                                            }
                                        } else {
                                            Vec::new()
                                        }
                                    } else {
                                        f.params.iter().map(|p| p.name.clone()).collect()
                                    };

                                    let mut positional = Vec::new();
                                    for param_name in &param_names {
                                        let param_lower = param_name.to_lowercase();
                                        let val = arg_map
                                            .iter()
                                            .find(|(k, _)| k.to_lowercase() == param_lower)
                                            .map(|(_, v)| v.clone())
                                            .unwrap_or(CfmlValue::Null);
                                        positional.push(val);
                                    }
                                    positional
                                } else {
                                    Vec::new()
                                }
                            } else if matches!(invoke_args, CfmlValue::Null) {
                                Vec::new()
                            } else {
                                vec![invoke_args]
                            };

                            let mut method_locals = IndexMap::new();
                            method_locals.insert("this".to_string(), component.clone());
                            // Inject __variables from component so unscoped references resolve
                            if let CfmlValue::Struct(ref cs) = component {
                                if let Some(vars) = cs.get("__variables") {
                                    method_locals.insert("__variables".to_string(), vars.clone());
                                }
                            }
                            return self.call_function(&func, call_args, &method_locals);
                        } else {
                            return Err(CfmlError::runtime(format!(
                                "Method '{}' not found in component",
                                method_name
                            )));
                        }
                    }
                    return Err(CfmlError::runtime(
                        "Invalid component for cfinvoke".to_string(),
                    ));
                }
                "__cfsavecontent_start" => {
                    self.saved_output_buffers
                        .push(std::mem::take(&mut self.output_buffer));
                    return Ok(CfmlValue::Null);
                }
                "__cfsavecontent_end" => {
                    let captured = std::mem::take(&mut self.output_buffer);
                    self.output_buffer = self.saved_output_buffers.pop().unwrap_or_default();
                    return Ok(CfmlValue::String(captured));
                }
                "invoke" => {
                    // Same as __cfinvoke: invoke(component, "methodName", argStruct)
                    let comp_val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
                    let method_name = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                    let invoke_args = args.get(2).cloned().unwrap_or(CfmlValue::Null);

                    let component = match &comp_val {
                        CfmlValue::Struct(_) => comp_val.clone(),
                        CfmlValue::String(name) => {
                            if let Some(template) =
                                self.resolve_component_template(name, parent_locals)
                            {
                                self.resolve_inheritance(template, parent_locals)
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Component '{}' not found",
                                    name
                                )));
                            }
                        }
                        _ => {
                            let name = comp_val.as_string();
                            if let Some(template) =
                                self.resolve_component_template(&name, parent_locals)
                            {
                                self.resolve_inheritance(template, parent_locals)
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Component '{}' not found",
                                    name
                                )));
                            }
                        }
                    };

                    let method_lower = method_name.to_lowercase();
                    if let CfmlValue::Struct(ref comp_struct) = component {
                        let method_func = comp_struct
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == method_lower)
                            .map(|(_, v)| v.clone());

                        if let Some(func @ CfmlValue::Function(_)) = method_func {
                            let call_args = if let CfmlValue::Struct(ref arg_map) = invoke_args {
                                if arg_map.is_empty() {
                                    Vec::new()
                                } else if let CfmlValue::Function(ref f) = func {
                                    let param_names: Vec<String> = if f.params.is_empty() {
                                        if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                            ref body,
                                        ) = f.body
                                        {
                                            if let CfmlValue::Int(idx) = body.as_ref() {
                                                let idx = *idx as usize;
                                                if idx < self.program.functions.len() {
                                                    self.program.functions[idx].params.clone()
                                                } else {
                                                    Vec::new()
                                                }
                                            } else {
                                                Vec::new()
                                            }
                                        } else {
                                            Vec::new()
                                        }
                                    } else {
                                        f.params.iter().map(|p| p.name.clone()).collect()
                                    };

                                    let mut positional = Vec::new();
                                    for param_name in &param_names {
                                        let param_lower = param_name.to_lowercase();
                                        let val = arg_map
                                            .iter()
                                            .find(|(k, _)| k.to_lowercase() == param_lower)
                                            .map(|(_, v)| v.clone())
                                            .unwrap_or(CfmlValue::Null);
                                        positional.push(val);
                                    }
                                    positional
                                } else {
                                    Vec::new()
                                }
                            } else if matches!(invoke_args, CfmlValue::Null) {
                                Vec::new()
                            } else {
                                vec![invoke_args]
                            };

                            let mut method_locals = IndexMap::new();
                            method_locals.insert("this".to_string(), component.clone());
                            // Inject __variables from component so unscoped references resolve
                            if let CfmlValue::Struct(ref cs) = component {
                                if let Some(vars) = cs.get("__variables") {
                                    method_locals.insert("__variables".to_string(), vars.clone());
                                }
                            }
                            return self.call_function(&func, call_args, &method_locals);
                        } else {
                            return Err(CfmlError::runtime(format!(
                                "Method '{}' not found on component",
                                method_name
                            )));
                        }
                    } else {
                        return Err(CfmlError::runtime(
                            "invoke() first argument must be a component or component name".into(),
                        ));
                    }
                }
                "queryexecute" => {
                    // VM intercept for queryExecute — routes through transaction conn if active
                    if self.transaction_conn.is_some() {
                        if let Some(txn_execute) = self.txn_execute {
                            let sql = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                            let params_arg = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                            let options_arg = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                            let return_type = match &options_arg {
                                CfmlValue::Struct(opts) => opts
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("returntype"))
                                    .map(|(_, v)| v.as_string().to_lowercase())
                                    .unwrap_or_else(|| "query".to_string()),
                                _ => "query".to_string(),
                            };
                            let txn_conn = self.transaction_conn.as_mut().unwrap();
                            return txn_execute(txn_conn, &sql, &params_arg, &return_type);
                        }
                    }
                    // No active transaction — delegate to normal builtin (via fn pointer or registered builtin)
                    if let Some(qe_fn) = self.query_execute_fn {
                        return qe_fn(args);
                    }
                    // Fall through to normal builtin dispatch
                    let builtin_match = self
                        .builtins
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == "queryexecute")
                        .map(|(_, v)| *v);
                    if let Some(builtin) = builtin_match {
                        return builtin(args);
                    }
                    return Err(CfmlError::runtime(
                        "queryExecute: database features not enabled".to_string(),
                    ));
                }
                "__cftransaction_start" => {
                    if self.transaction_conn.is_some() {
                        return Err(CfmlError::runtime(
                            "cftransaction: nested transactions are not supported".to_string(),
                        ));
                    }
                    // Args: __cftransaction_start("begin", [isolation], [datasource])
                    // Try arg[2] first (datasource after isolation), then arg[1] (datasource without isolation)
                    let datasource = args
                        .get(2)
                        .map(|v| v.as_string())
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            args.get(1)
                                .map(|v| v.as_string())
                                .filter(|s| !s.is_empty() && s != "begin")
                        })
                        .unwrap_or_else(|| self.get_default_datasource(parent_locals));
                    if datasource.is_empty() {
                        // Lucee/ACF defer transaction-connection setup until a query
                        // actually runs — `transaction { ... }` around non-query code
                        // is allowed. Commit/rollback are already no-ops without a
                        // connection, so we mirror that behaviour here.
                        return Ok(CfmlValue::Null);
                    }
                    if let Some(txn_begin) = self.txn_begin {
                        let conn = txn_begin(&datasource)?;
                        self.transaction_conn = Some(conn);
                        self.transaction_datasource = Some(datasource);
                        return Ok(CfmlValue::Null);
                    }
                    return Err(CfmlError::runtime(
                        "cftransaction: transaction support not initialized".to_string(),
                    ));
                }
                "__cftransaction_commit" => {
                    if let Some(ref mut conn) = self.transaction_conn {
                        if let Some(txn_commit) = self.txn_commit {
                            txn_commit(conn)?;
                        }
                    }
                    self.transaction_conn = None;
                    self.transaction_datasource = None;
                    return Ok(CfmlValue::Null);
                }
                "__cftransaction_rollback" => {
                    if let Some(ref mut conn) = self.transaction_conn {
                        if let Some(txn_rollback) = self.txn_rollback {
                            txn_rollback(conn)?;
                        }
                    }
                    self.transaction_conn = None;
                    self.transaction_datasource = None;
                    return Ok(CfmlValue::Null);
                }
                "__cflog" => {
                    // Extract log message from struct argument
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        let text = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "text")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let log_type = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "type")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_else(|| "Information".to_string());
                        let file = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "file")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_else(|| "application".to_string());
                        eprintln!("[CFLOG {}:{}] {}", file, log_type, text);
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfsetting" => {
                    // Handle cfsetting options
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        // enableCFOutputOnly: counter-based. true increments, false decrements.
                        // "reset" forces counter to 0. When > 0, only <cfoutput> content is emitted.
                        if let Some((_, v)) = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "enablecfoutputonly")
                        {
                            let val_str = v.as_string().to_lowercase();
                            if val_str == "reset" {
                                self.enable_cfoutput_only = 0;
                            } else if val_str == "true" || val_str == "yes" || val_str == "1" {
                                self.enable_cfoutput_only += 1;
                            } else {
                                self.enable_cfoutput_only = (self.enable_cfoutput_only - 1).max(0);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cflock_start" => {
                    // Extract lock attributes from struct argument
                    let (lock_name, lock_type, timeout_ms) =
                        if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                            let name = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "name")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_else(|| "default".to_string());
                            let ltype = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "type")
                                .map(|(_, v)| v.as_string().to_lowercase())
                                .unwrap_or_else(|| "exclusive".to_string());
                            let timeout = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "timeout")
                                .and_then(|(_, v)| match v {
                                    CfmlValue::Int(i) => Some(*i as u64 * 1000),
                                    CfmlValue::Double(d) => Some((*d * 1000.0) as u64),
                                    CfmlValue::String(s) => {
                                        s.parse::<f64>().ok().map(|d| (d * 1000.0) as u64)
                                    }
                                    _ => None,
                                })
                                .unwrap_or(5000);
                            (name, ltype, timeout)
                        } else {
                            // Positional args: name, type, timeout
                            let name = args
                                .get(0)
                                .map(|v| v.as_string())
                                .unwrap_or_else(|| "default".to_string());
                            let ltype = args
                                .get(1)
                                .map(|v| v.as_string().to_lowercase())
                                .unwrap_or_else(|| "exclusive".to_string());
                            let timeout = args
                                .get(2)
                                .and_then(|v| match v {
                                    CfmlValue::Int(i) => Some(*i as u64 * 1000),
                                    CfmlValue::Double(d) => Some((*d * 1000.0) as u64),
                                    CfmlValue::String(s) => {
                                        s.parse::<f64>().ok().map(|d| (d * 1000.0) as u64)
                                    }
                                    _ => None,
                                })
                                .unwrap_or(5000);
                            (name, ltype, timeout)
                        };

                    if let Some(ref server_state) = self.server_state {
                        // Get or create the named lock
                        let lock = {
                            let mut locks = server_state.named_locks.lock().unwrap();
                            locks
                                .entry(lock_name.clone())
                                .or_insert_with(|| Arc::new(RwLock::new(())))
                                .clone()
                        };

                        // Acquire lock with timeout using try_lock in a spin loop
                        let deadline = cfml_common::clock::Monotonic::now()
                            + std::time::Duration::from_millis(timeout_ms);
                        let is_exclusive = lock_type != "readonly";

                        if is_exclusive {
                            loop {
                                if let Ok(guard) = lock.try_write() {
                                    // SAFETY: We extend the lifetime because the Arc keeps the RwLock alive.
                                    // The guard is dropped in __cflock_end before the Arc can be dropped.
                                    let guard: std::sync::RwLockWriteGuard<'static, ()> =
                                        unsafe { std::mem::transmute(guard) };
                                    self.held_locks.push((lock_name, HeldLock::Write(guard)));
                                    break;
                                }
                                if cfml_common::clock::Monotonic::now() >= deadline {
                                    return Err(CfmlError::runtime(
                                        format!("cflock timeout: could not acquire exclusive lock within {}ms", timeout_ms)
                                    ));
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                        } else {
                            loop {
                                if let Ok(guard) = lock.try_read() {
                                    let guard: std::sync::RwLockReadGuard<'static, ()> =
                                        unsafe { std::mem::transmute(guard) };
                                    self.held_locks.push((lock_name, HeldLock::Read(guard)));
                                    break;
                                }
                                if cfml_common::clock::Monotonic::now() >= deadline {
                                    return Err(CfmlError::runtime(
                                        format!("cflock timeout: could not acquire readonly lock within {}ms", timeout_ms)
                                    ));
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                        }
                    }
                    // Without server_state (CLI mode), locks are a no-op
                    return Ok(CfmlValue::Null);
                }
                "__cflock_end" => {
                    // Release the most recently acquired lock
                    // Args may contain the lock name for matching
                    let lock_name = if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        opts.iter()
                            .find(|(k, _)| k.to_lowercase() == "name")
                            .map(|(_, v)| v.as_string())
                    } else {
                        args.get(0).map(|v| v.as_string())
                    };

                    if let Some(name) = lock_name {
                        // Find and remove the matching lock guard
                        if let Some(pos) = self.held_locks.iter().rposition(|(n, _)| *n == name) {
                            self.held_locks.remove(pos);
                        }
                    } else {
                        // Pop the most recent lock
                        self.held_locks.pop();
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfcookie" => {
                    // Set a cookie via response headers
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        let name = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "name")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let value = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "value")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let mut cookie = format!("{}={}", name, value);
                        if let Some((_, expires)) =
                            opts.iter().find(|(k, _)| k.to_lowercase() == "expires")
                        {
                            cookie.push_str(&format!("; Expires={}", expires.as_string()));
                        }
                        if let Some((_, domain)) =
                            opts.iter().find(|(k, _)| k.to_lowercase() == "domain")
                        {
                            cookie.push_str(&format!("; Domain={}", domain.as_string()));
                        }
                        if let Some((_, path)) =
                            opts.iter().find(|(k, _)| k.to_lowercase() == "path")
                        {
                            cookie.push_str(&format!("; Path={}", path.as_string()));
                        }
                        if let Some((_, secure)) =
                            opts.iter().find(|(k, _)| k.to_lowercase() == "secure")
                        {
                            if secure.as_string().to_lowercase() == "true"
                                || secure.as_string() == "yes"
                            {
                                cookie.push_str("; Secure");
                            }
                        }
                        if let Some((_, httponly)) =
                            opts.iter().find(|(k, _)| k.to_lowercase() == "httponly")
                        {
                            if httponly.as_string().to_lowercase() == "true"
                                || httponly.as_string() == "yes"
                            {
                                cookie.push_str("; HttpOnly");
                            }
                        }
                        self.response_headers
                            .push(("Set-Cookie".to_string(), cookie));
                    }
                    return Ok(CfmlValue::Null);
                }
                "fileupload" | "__cffile_upload" => {
                    // fileUpload(destination, formField, accept, nameConflict)
                    let destination = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let form_field = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                    let _accept = args.get(2).map(|v| v.as_string()).unwrap_or_default();
                    let name_conflict = args
                        .get(3)
                        .map(|v| v.as_string().to_lowercase())
                        .unwrap_or_else(|| "error".to_string());

                    // Look up the form field to find uploaded file info
                    let form_scope = self
                        .globals
                        .get("form")
                        .cloned()
                        .unwrap_or(CfmlValue::strukt(IndexMap::new()));

                    if let CfmlValue::Struct(form) = form_scope {
                        let field_lower = form_field.to_lowercase();
                        if let Some(CfmlValue::Struct(file_info)) = form
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == field_lower)
                            .map(|(_, v)| v)
                        {
                            let temp_path = file_info
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "tempfilepath")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_default();
                            let client_file = file_info
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "clientfile")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_default();

                            if !temp_path.is_empty() {
                                let dest_dir = std::path::Path::new(&destination);
                                let _ = std::fs::create_dir_all(dest_dir);
                                let dest_file = dest_dir.join(&client_file);

                                let final_path =
                                    if dest_file.exists() && name_conflict == "makeunique" {
                                        let stem = dest_file
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_default();
                                        let ext = dest_file
                                            .extension()
                                            .map(|s| format!(".{}", s.to_string_lossy()))
                                            .unwrap_or_default();
                                        let unique = dest_dir.join(format!(
                                            "{}_{}{}",
                                            stem,
                                            cfml_common::clock::now_unix_millis(),
                                            ext
                                        ));
                                        unique
                                    } else {
                                        dest_file
                                    };

                                match std::fs::copy(&temp_path, &final_path) {
                                    Ok(_) => {
                                        let _ = std::fs::remove_file(&temp_path);
                                        let mut result = file_info.clone();
                                        Arc::make_mut(&mut result).insert(
                                            "serverDirectory".to_string(),
                                            CfmlValue::String(destination),
                                        );
                                        Arc::make_mut(&mut result).insert(
                                            "serverFile".to_string(),
                                            CfmlValue::String(
                                                final_path
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .to_string(),
                                            ),
                                        );
                                        Arc::make_mut(&mut result).insert(
                                            "fileWasSaved".to_string(),
                                            CfmlValue::Bool(true),
                                        );
                                        return Ok(CfmlValue::Struct(result));
                                    }
                                    Err(e) => {
                                        return Err(CfmlError::runtime(format!(
                                            "fileUpload: {}",
                                            e
                                        )))
                                    }
                                }
                            }
                        }
                    }
                    return Err(CfmlError::runtime(format!(
                        "fileUpload: form field '{}' not found or no file uploaded",
                        form_field
                    )));
                }
                "fileuploadall" => {
                    // fileUploadAll(destination, accept, nameConflict)
                    let destination = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let _accept = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                    let name_conflict = args
                        .get(2)
                        .map(|v| v.as_string().to_lowercase())
                        .unwrap_or_else(|| "error".to_string());

                    let form_scope = self
                        .globals
                        .get("form")
                        .cloned()
                        .unwrap_or(CfmlValue::strukt(IndexMap::new()));

                    let mut results = Vec::new();
                    if let CfmlValue::Struct(form) = form_scope {
                        for (_, val) in form.iter() {
                            if let CfmlValue::Struct(file_info) = val {
                                let temp_path = file_info
                                    .iter()
                                    .find(|(k, _)| k.to_lowercase() == "tempfilepath")
                                    .map(|(_, v)| v.as_string())
                                    .unwrap_or_default();
                                if temp_path.is_empty() {
                                    continue;
                                }

                                let client_file = file_info
                                    .iter()
                                    .find(|(k, _)| k.to_lowercase() == "clientfile")
                                    .map(|(_, v)| v.as_string())
                                    .unwrap_or_default();

                                let dest_dir = std::path::Path::new(&destination);
                                let _ = std::fs::create_dir_all(dest_dir);
                                let dest_file = dest_dir.join(&client_file);

                                let final_path =
                                    if dest_file.exists() && name_conflict == "makeunique" {
                                        let stem = dest_file
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_default();
                                        let ext = dest_file
                                            .extension()
                                            .map(|s| format!(".{}", s.to_string_lossy()))
                                            .unwrap_or_default();
                                        let unique = dest_dir.join(format!(
                                            "{}_{}{}",
                                            stem,
                                            cfml_common::clock::now_unix_millis(),
                                            ext
                                        ));
                                        unique
                                    } else {
                                        dest_file
                                    };

                                if let Ok(_) = std::fs::copy(&temp_path, &final_path) {
                                    let _ = std::fs::remove_file(&temp_path);
                                    let mut result = file_info.clone();
                                    Arc::make_mut(&mut result).insert(
                                        "serverDirectory".to_string(),
                                        CfmlValue::String(destination.clone()),
                                    );
                                    Arc::make_mut(&mut result).insert(
                                        "serverFile".to_string(),
                                        CfmlValue::String(
                                            final_path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .to_string(),
                                        ),
                                    );
                                    Arc::make_mut(&mut result)
                                        .insert("fileWasSaved".to_string(), CfmlValue::Bool(true));
                                    results.push(CfmlValue::Struct(result));
                                }
                            }
                        }
                    }
                    return Ok(CfmlValue::array(results));
                }
                "sessioninvalidate" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        state.sessions.remove(sid);
                    }
                    return Ok(CfmlValue::Null);
                }
                "sessionrotate" => {
                    // Generate a new session ID and migrate data
                    if let (Some(ref state), Some(ref old_sid)) =
                        (&self.server_state, &self.session_id)
                    {
                        let new_sid = uuid::Uuid::new_v4().to_string();
                        state.sessions.rotate(old_sid, &new_sid);
                        self.session_id = Some(new_sid);
                    }
                    return Ok(CfmlValue::Null);
                }
                "sessiongetmetadata" => {
                    let mut meta = IndexMap::new();
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(sid) {
                            let now = now_epoch_secs();
                            meta.insert(
                                "sessionId".to_string(),
                                CfmlValue::String(sid.clone()),
                            );
                            meta.insert(
                                "timeCreated".to_string(),
                                CfmlValue::Int(now.saturating_sub(session.created_secs) as i64),
                            );
                            meta.insert("lastAccessed".to_string(), CfmlValue::Int(now.saturating_sub(session.last_accessed_secs) as i64));
                        }
                    }
                    return Ok(CfmlValue::strukt(meta));
                }
                "getauthuser" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(sid) {
                            if let Some(ref user) = session.auth_user {
                                return Ok(CfmlValue::String(user.clone()));
                            }
                        }
                    }
                    return Ok(CfmlValue::String(String::new()));
                }
                "isuserloggedin" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(sid) {
                            return Ok(CfmlValue::Bool(session.auth_user.is_some()));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "isuserinrole" => {
                    let role = args
                        .get(0)
                        .map(|v| v.as_string().to_lowercase())
                        .unwrap_or_default();
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(sid) {
                            let has_role =
                                session.auth_roles.iter().any(|r| r.to_lowercase() == role);
                            return Ok(CfmlValue::Bool(has_role));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "__cfloginuser" => {
                    // cfloginuser name="..." password="..." roles="..."
                    let name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let roles_str = args.get(2).map(|v| v.as_string()).unwrap_or_default();
                    let roles: Vec<String> = roles_str
                        .split(',')
                        .map(|r| r.trim().to_string())
                        .filter(|r| !r.is_empty())
                        .collect();
                    // Treat login as a session write → lazy-init.
                    self.lazy_init_session_if_pending();
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        let now = now_epoch_secs();
                        let mut session = state.sessions.get(sid).unwrap_or_else(|| SessionData {
                            variables: IndexMap::new(),
                            created_secs: now,
                            last_accessed_secs: now,
                            auth_user: None,
                            auth_roles: Vec::new(),
                            timeout_secs: 1800,
                        });
                        session.auth_user = Some(name);
                        session.auth_roles = roles;
                        state.sessions.set(sid, session);
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cflogout" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(mut session) = state.sessions.get(sid) {
                            session.auth_user = None;
                            session.auth_roles.clear();
                            state.sessions.set(sid, session);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "getvariable" => {
                    // getVariable(name) — walk scope chain to find variable
                    let var_name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let var_lower = var_name.to_lowercase();

                    // Handle dotted names like "variables.foo" or "request.bar"
                    if var_lower.contains('.') {
                        let parts: Vec<&str> = var_lower.splitn(2, '.').collect();
                        let scope_name = parts[0];
                        let key = parts.get(1).copied().unwrap_or("");
                        match scope_name {
                            "request" => {
                                if let Some(val) = self
                                    .request_scope
                                    .iter()
                                    .find(|(k, _)| k.to_lowercase() == key)
                                    .map(|(_, v)| v.clone())
                                {
                                    return Ok(val);
                                }
                                return Ok(CfmlValue::Null);
                            }
                            "session" => {
                                if let CfmlValue::Struct(s) = self.get_session_scope() {
                                    if let Some(val) = s
                                        .iter()
                                        .find(|(k, _)| k.to_lowercase() == key)
                                        .map(|(_, v)| v.clone())
                                    {
                                        return Ok(val);
                                    }
                                }
                                return Ok(CfmlValue::Null);
                            }
                            "application" => {
                                if let Some(ref app_scope) = self.application_scope {
                                    if let Ok(scope) = app_scope.lock() {
                                        if let Some(val) = scope
                                            .iter()
                                            .find(|(k, _)| k.to_lowercase() == key)
                                            .map(|(_, v)| v.clone())
                                        {
                                            return Ok(val);
                                        }
                                    }
                                }
                                return Ok(CfmlValue::Null);
                            }
                            _ => {}
                        }
                    }

                    // Check parent_locals
                    if let Some(val) = parent_locals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == var_lower)
                        .map(|(_, v)| v.clone())
                    {
                        return Ok(val);
                    }
                    // Request scope
                    if let Some(val) = self
                        .request_scope
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == var_lower)
                        .map(|(_, v)| v.clone())
                    {
                        return Ok(val);
                    }
                    // Session scope
                    if let CfmlValue::Struct(s) = self.get_session_scope() {
                        if let Some(val) = s
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == var_lower)
                            .map(|(_, v)| v.clone())
                        {
                            return Ok(val);
                        }
                    }
                    // Application scope
                    if let Some(ref app_scope) = self.application_scope {
                        if let Ok(scope) = app_scope.lock() {
                            if let Some(val) = scope
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == var_lower)
                                .map(|(_, v)| v.clone())
                            {
                                return Ok(val);
                            }
                        }
                    }
                    // Globals
                    if let Some(val) = self
                        .globals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == var_lower)
                        .map(|(_, v)| v.clone())
                    {
                        return Ok(val);
                    }
                    return Ok(CfmlValue::Null);
                }
                "setvariable" => {
                    // setVariable(name, value) — set a variable by dynamic name, return value
                    let var_name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let value = args.get(1).cloned().unwrap_or(CfmlValue::Null);

                    // Handle dotted scope names
                    let var_lower = var_name.to_lowercase();
                    if var_lower.starts_with("variables.") {
                        let key = var_name[10..].to_string();
                        self.globals.insert(key, value.clone());
                    } else if var_lower.starts_with("request.") {
                        let key = var_name[8..].to_string();
                        self.request_scope.insert(key, value.clone());
                    } else if var_lower.starts_with("session.") {
                        let key = var_name[8..].to_string();
                        self.set_session_variable(&key, value.clone());
                    } else if var_lower.starts_with("application.") {
                        let key = var_name[12..].to_string();
                        if let Some(ref app_scope) = self.application_scope {
                            if let Ok(mut scope) = app_scope.lock() {
                                scope.insert(key, value.clone());
                            }
                        }
                    } else {
                        // Default: set in variables (globals) scope
                        self.globals.insert(var_name, value.clone());
                    }
                    return Ok(value);
                }
                "throw" => {
                    // throw(message="...", type="...", detail="...", errorcode="...")
                    // Build exception struct from named args or positional
                    let mut exception = IndexMap::new();
                    let message = args
                        .get(0)
                        .map(|v| v.as_string())
                        .unwrap_or_else(|| "".to_string());
                    let error_type = args
                        .get(1)
                        .map(|v| v.as_string())
                        .unwrap_or_else(|| "Application".to_string());
                    let detail = args.get(2).map(|v| v.as_string()).unwrap_or_default();
                    let errorcode = args.get(3).map(|v| v.as_string()).unwrap_or_default();

                    exception.insert("message".to_string(), CfmlValue::String(message.clone()));
                    exception.insert("type".to_string(), CfmlValue::String(error_type));
                    exception.insert("detail".to_string(), CfmlValue::String(detail));
                    exception.insert("errorcode".to_string(), CfmlValue::String(errorcode));
                    exception.insert("tagcontext".to_string(), self.build_tag_context());

                    let error_val = CfmlValue::strukt(exception);
                    self.last_exception = Some(error_val.clone());

                    return Err(CfmlError::runtime(message));
                }
                // ---- Cache functions ----
                "cacheput" => {
                    let key = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let value = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                    let expiry = args.get(2).and_then(|v| {
                        // Timespan: value < 1 treated as fractional days (×86400→secs)
                        let secs = match v {
                            CfmlValue::Int(i) => *i as f64,
                            CfmlValue::Double(d) => {
                                if *d < 1.0 {
                                    *d * 86400.0
                                } else {
                                    *d
                                }
                            }
                            CfmlValue::String(s) => s.parse::<f64>().unwrap_or(0.0),
                            _ => 0.0,
                        };
                        if secs > 0.0 {
                            Some(
                                cfml_common::clock::Monotonic::now()
                                    + std::time::Duration::from_secs_f64(secs),
                            )
                        } else {
                            None
                        }
                    });
                    self.cache.insert(key, (value, expiry));
                    return Ok(CfmlValue::Null);
                }
                "cacheget" => {
                    let key = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    if let Some((val, expiry)) = self.cache.get(&key).cloned() {
                        if let Some(exp) = expiry {
                            if cfml_common::clock::Monotonic::now() > exp {
                                self.cache.remove(&key);
                                return Ok(CfmlValue::Null);
                            }
                        }
                        return Ok(val);
                    }
                    return Ok(CfmlValue::Null);
                }
                "cachedelete" => {
                    let key = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let throw_on_error = args
                        .get(1)
                        .map(|v| match v {
                            CfmlValue::Bool(b) => *b,
                            CfmlValue::String(s) => {
                                s.to_lowercase() == "true" || s.to_lowercase() == "yes"
                            }
                            _ => false,
                        })
                        .unwrap_or(false);
                    if self.cache.remove(&key).is_none() && throw_on_error {
                        return Err(CfmlError::runtime(format!(
                            "Cache key '{}' does not exist",
                            key
                        )));
                    }
                    return Ok(CfmlValue::Null);
                }
                "cacheclear" => {
                    let filter = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    if filter.is_empty() {
                        self.cache.clear();
                    } else {
                        // Simple wildcard matching: * matches any sequence
                        let pattern = filter.to_lowercase();
                        let keys_to_remove: Vec<String> = self
                            .cache
                            .keys()
                            .filter(|k| wildcard_match(&pattern, &k.to_lowercase()))
                            .cloned()
                            .collect();
                        for k in keys_to_remove {
                            self.cache.remove(&k);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "cachekeyexists" => {
                    let key = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    if let Some((_, expiry)) = self.cache.get(&key) {
                        if let Some(exp) = expiry {
                            if cfml_common::clock::Monotonic::now() > *exp {
                                self.cache.remove(&key);
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                        return Ok(CfmlValue::Bool(true));
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "cachecount" => {
                    let now = cfml_common::clock::Monotonic::now();
                    let count = self
                        .cache
                        .iter()
                        .filter(|(_, (_, exp))| exp.map_or(true, |e| now <= e))
                        .count();
                    return Ok(CfmlValue::Int(count as i64));
                }
                "cachegetall" => {
                    let now = cfml_common::clock::Monotonic::now();
                    let mut result = IndexMap::new();
                    for (k, (v, exp)) in &self.cache {
                        if exp.map_or(true, |e| now <= e) {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                    return Ok(CfmlValue::strukt(result));
                }
                "cachegetallids" => {
                    let now = cfml_common::clock::Monotonic::now();
                    let ids: Vec<CfmlValue> = self
                        .cache
                        .iter()
                        .filter(|(_, (_, exp))| exp.map_or(true, |e| now <= e))
                        .map(|(k, _)| CfmlValue::String(k.clone()))
                        .collect();
                    return Ok(CfmlValue::array(ids));
                }

                // ---- cfcache tag handler ----
                "__cfcache" => {
                    // Stub/no-op; in serve mode could push Cache-Control header
                    return Ok(CfmlValue::Null);
                }

                // ---- cfexecute tag handler ----
                "__cfexecute" => {
                    if let Some(CfmlValue::Struct(opts)) = args.get(0) {
                        let cmd_name = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "name")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let arguments = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "arguments")
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        let has_variable = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "variable")
                            .map(|(_, v)| match v {
                                CfmlValue::Bool(b) => *b,
                                CfmlValue::String(s) => s.to_lowercase() == "true",
                                _ => false,
                            })
                            .unwrap_or(false);
                        let body = opts
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "body")
                            .map(|(_, v)| v.as_string());

                        let cmd_args: Vec<&str> = if arguments.is_empty() {
                            Vec::new()
                        } else {
                            arguments.split_whitespace().collect()
                        };

                        let mut command = std::process::Command::new(&cmd_name);
                        command.args(&cmd_args);
                        if body.is_some() {
                            command.stdin(std::process::Stdio::piped());
                        }
                        command.stdout(std::process::Stdio::piped());
                        command.stderr(std::process::Stdio::piped());

                        match command.spawn() {
                            Ok(mut child) => {
                                if let Some(ref stdin_data) = body {
                                    if let Some(ref mut stdin) = child.stdin {
                                        use std::io::Write;
                                        let _ = stdin.write_all(stdin_data.as_bytes());
                                    }
                                    // Drop stdin to signal EOF
                                    child.stdin.take();
                                }
                                match child.wait_with_output() {
                                    Ok(output) => {
                                        let stdout =
                                            String::from_utf8_lossy(&output.stdout).to_string();
                                        let stderr =
                                            String::from_utf8_lossy(&output.stderr).to_string();
                                        if has_variable {
                                            let mut result = IndexMap::new();
                                            result.insert(
                                                "output".to_string(),
                                                CfmlValue::String(stdout),
                                            );
                                            result.insert(
                                                "error".to_string(),
                                                CfmlValue::String(stderr),
                                            );
                                            return Ok(CfmlValue::strukt(result));
                                        } else {
                                            self.output_buffer.push_str(&stdout);
                                            return Ok(CfmlValue::Null);
                                        }
                                    }
                                    Err(e) => {
                                        return Err(CfmlError::runtime(format!(
                                            "cfexecute: {}",
                                            e
                                        )));
                                    }
                                }
                            }
                            Err(e) => {
                                return Err(CfmlError::runtime(format!(
                                    "cfexecute: failed to spawn '{}': {}",
                                    cmd_name, e
                                )));
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }

                // ---- cfthread handlers ----
                "__cfthread_run" => {
                    let thread_name = args
                        .get(0)
                        .map(|v| v.as_string())
                        .unwrap_or_else(|| "thread1".to_string());

                    let mut thread_output = String::new();
                    let mut thread_error = String::new();
                    let mut thread_vars: IndexMap<String, CfmlValue> = IndexMap::new();
                    let mut elapsed: i64 = 0;

                    if let Some(callback) = args.get(1) {
                        let callback = callback.clone();

                        // Set up thread scope in globals (thread.varName = value)
                        self.globals
                            .insert("thread".to_string(), CfmlValue::strukt(IndexMap::new()));

                        // Capture output (same pattern as cfsavecontent)
                        self.saved_output_buffers
                            .push(std::mem::take(&mut self.output_buffer));

                        // Track elapsed time
                        let start_time = cfml_common::clock::Monotonic::now();

                        // Execute thread body, catching errors
                        let result = self.call_function(&callback, vec![], parent_locals);

                        elapsed = start_time.elapsed().as_millis() as i64;

                        // Capture any output written during thread body
                        thread_output = std::mem::take(&mut self.output_buffer);
                        self.output_buffer = self.saved_output_buffers.pop().unwrap_or_default();

                        // Capture error if any
                        if let Err(ref e) = result {
                            thread_error = format!("{}", e);
                        }

                        // Collect thread scope variables
                        if let Some(CfmlValue::Struct(ts)) = self.globals.shift_remove("thread") {
                            thread_vars = (*ts).clone();
                        }
                    }

                    // Build thread metadata. Status is TERMINATED if an error
                    // occurred, COMPLETED otherwise (matches Lucee).
                    let status = if thread_error.is_empty() { "COMPLETED" } else { "TERMINATED" };
                    let mut thread_meta = IndexMap::new();
                    thread_meta.insert(
                        "status".to_string(),
                        CfmlValue::String(status.to_string()),
                    );
                    thread_meta.insert("name".to_string(), CfmlValue::String(thread_name.clone()));
                    thread_meta.insert("output".to_string(), CfmlValue::String(thread_output));
                    thread_meta.insert("error".to_string(), CfmlValue::String(thread_error));
                    thread_meta.insert("elapsedtime".to_string(), CfmlValue::Int(elapsed));

                    // Merge thread scope variables into metadata
                    for (k, v) in thread_vars {
                        thread_meta.insert(k, v);
                    }

                    // Store in cfthread scope
                    let thread_struct = self.get_or_create_cfthread_scope();
                    if let Some(ts) = thread_struct.as_struct_mut() {
                        ts.insert(thread_name.to_lowercase(), CfmlValue::strukt(thread_meta));
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfthread_join" => {
                    // No-op since execution already happened (thread is already complete)
                    return Ok(CfmlValue::Null);
                }
                "__cfthread_terminate" => {
                    // No-op (thread already finished)
                    return Ok(CfmlValue::Null);
                }

                "callstackget" => {
                    let frames = self.build_stack_trace();
                    let offset = args
                        .get(0)
                        .map(|v| v.as_string().parse::<i64>().unwrap_or(0).max(0) as usize)
                        .unwrap_or(0);
                    let max_frames = args
                        .get(1)
                        .map(|v| v.as_string().parse::<usize>().unwrap_or(usize::MAX))
                        .unwrap_or(usize::MAX);
                    let result: Vec<CfmlValue> = frames
                        .into_iter()
                        .skip(offset)
                        .take(max_frames)
                        .map(|f| {
                            let mut s = IndexMap::new();
                            s.insert("Function".to_string(), CfmlValue::String(f.function));
                            s.insert("Template".to_string(), CfmlValue::String(f.template));
                            s.insert("LineNumber".to_string(), CfmlValue::Int(f.line as i64));
                            CfmlValue::strukt(s)
                        })
                        .collect();
                    return Ok(CfmlValue::array(result));
                }

                "callstackdump" => {
                    let frames = self.build_stack_trace();
                    let dump: String = frames
                        .iter()
                        .map(|f| format!("{} ({}:{})", f.function, f.template, f.line))
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.output_buffer.push_str(&dump);
                    self.output_buffer.push('\n');
                    return Ok(CfmlValue::Null);
                }

                "precisionevaluate" => {
                    let expr = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let result = precision_evaluate_expr(&expr)?;
                    return Ok(CfmlValue::String(result));
                }

                "__cfcustomtag" => {
                    // Self-closing custom tag: __cfcustomtag(path_spec, attrs_struct)
                    let path_spec = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let attrs_val = args
                        .get(1)
                        .cloned()
                        .unwrap_or(CfmlValue::strukt(IndexMap::new()));

                    let resolved = self.resolve_custom_tag_path(&path_spec)?;
                    let mut this_tag = IndexMap::new();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::String("start".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(false));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::String(String::new()),
                    );

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = IndexMap::new();
                    tag_locals.insert("attributes".to_string(), attrs_val);
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    self.execute_custom_tag_template(&resolved, &tag_locals)?;

                    // Caller write-back: read modified caller from captured_locals
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(CfmlValue::Struct(modified_caller)) = captured.get("caller") {
                            let mut wb = IndexMap::new();
                            for (k, v) in modified_caller.iter() {
                                if let Some(orig) = caller_snapshot.get(k) {
                                    if !Self::values_equal_shallow(v, orig) {
                                        wb.insert(k.clone(), v.clone());
                                    }
                                } else {
                                    wb.insert(k.clone(), v.clone());
                                }
                            }
                            if !wb.is_empty() {
                                self.closure_parent_writeback = Some(wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfcustomtag_start" => {
                    // Body custom tag start: __cfcustomtag_start(path_spec, attrs_struct)
                    let path_spec = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let attrs_val = args
                        .get(1)
                        .cloned()
                        .unwrap_or(CfmlValue::strukt(IndexMap::new()));

                    let resolved = self.resolve_custom_tag_path(&path_spec)?;

                    let mut this_tag = IndexMap::new();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::String("start".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(true));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::String(String::new()),
                    );

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = IndexMap::new();
                    tag_locals.insert("attributes".to_string(), attrs_val.clone());
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    self.execute_custom_tag_template(&resolved, &tag_locals)?;

                    // Caller write-back from start execution
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(CfmlValue::Struct(modified_caller)) = captured.get("caller") {
                            let mut wb = IndexMap::new();
                            for (k, v) in modified_caller.iter() {
                                if let Some(orig) = caller_snapshot.get(k) {
                                    if !Self::values_equal_shallow(v, orig) {
                                        wb.insert(k.clone(), v.clone());
                                    }
                                } else {
                                    wb.insert(k.clone(), v.clone());
                                }
                            }
                            if !wb.is_empty() {
                                self.closure_parent_writeback = Some(wb);
                            }
                        }
                    }

                    // Push state for end tag
                    self.custom_tag_stack.push(CustomTagState {
                        template_path: resolved,
                        attributes: attrs_val,
                    });

                    // Push output buffer to capture body content (like savecontent)
                    self.saved_output_buffers
                        .push(std::mem::take(&mut self.output_buffer));

                    return Ok(CfmlValue::Null);
                }
                "__cfcustomtag_end" => {
                    // Body custom tag end: capture body output, re-execute tag in "end" mode
                    let body_content = std::mem::take(&mut self.output_buffer);
                    self.output_buffer = self.saved_output_buffers.pop().unwrap_or_default();

                    let state = match self.custom_tag_stack.pop() {
                        Some(s) => s,
                        None => {
                            return Err(CfmlError::runtime(
                                "__cfcustomtag_end without matching start".to_string(),
                            ))
                        }
                    };

                    let mut this_tag = IndexMap::new();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::String("end".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(true));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::String(body_content),
                    );

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = IndexMap::new();
                    tag_locals.insert("attributes".to_string(), state.attributes);
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    self.execute_custom_tag_template(&state.template_path, &tag_locals)?;

                    // Read back generatedContent and append to output
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(CfmlValue::Struct(tag_info)) = captured.get("thistag") {
                            if let Some(CfmlValue::String(content)) =
                                tag_info.get("generatedcontent")
                            {
                                self.output_buffer.push_str(content);
                            }
                        }
                    }

                    // Caller write-back from end execution
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(CfmlValue::Struct(modified_caller)) = captured.get("caller") {
                            let mut wb = IndexMap::new();
                            for (k, v) in modified_caller.iter() {
                                if let Some(orig) = caller_snapshot.get(k) {
                                    if !Self::values_equal_shallow(v, orig) {
                                        wb.insert(k.clone(), v.clone());
                                    }
                                } else {
                                    wb.insert(k.clone(), v.clone());
                                }
                            }
                            if !wb.is_empty() {
                                self.closure_parent_writeback = Some(wb);
                            }
                        }
                    }

                    return Ok(CfmlValue::Null);
                }
                _ => {}
            }
        }

        Err(self.wrap_error(CfmlError::runtime(format!(
            "Variable is not a function or function '{}' is not defined",
            if let CfmlValue::Function(f) = func_ref {
                &f.name
            } else {
                "<unknown>"
            }
        ))))
    }

    /// Handle member function calls like "hello".ucase(), [1,2,3].len(), etc.
    /// CFML member functions are syntactic sugar for standalone function calls
    /// where the object becomes the first argument.
    /// Returns true if the method name is a mutating array/struct operation.
    /// These methods modify the receiver in-place in CFML (pass-by-reference semantics).
    fn is_mutating_method(method: &str) -> bool {
        let lower = method.to_lowercase();
        // Implicit property setters (setXxx) are mutating
        if lower.starts_with("set") && lower.len() > 3 {
            return true;
        }
        matches!(
            lower.as_str(),
            // Array mutators
            "append" | "push" | "prepend" | "deleteat" | "insertat" |
            "sort" | "reverse" | "clear" |
            // Struct mutators
            "delete" | "insert" | "update" |
            // Query mutators
            "addrow" | "setcell" | "addcolumn" | "deleterow" | "deletecolumn" |
            // Java shim mutators (Map.put, Map.putIfAbsent, Queue.offer)
            "put" | "putifabsent" | "offer"
        )
    }

    /// Set a value at an arbitrary depth in a nested struct.
    /// path = ["prop1", "prop2"] means set root.prop1.prop2 = value
    /// path = ["prop1"] means set root.prop1 = value
    fn deep_set(root: &mut CfmlValue, path: &[String], value: CfmlValue) {
        if path.is_empty() {
            return;
        }
        if path.len() == 1 {
            root.set(path[0].clone(), value);
            return;
        }
        // Recurse into the nested struct
        if let Some(s) = root.as_struct_mut() {
            if let Some(child) = s.get_mut(&path[0]) {
                Self::deep_set(child, &path[1..], value);
            }
        }
    }

    /// Load a variable by name, checking locals, globals, and special scopes (application, request, local/variables).
    fn scope_aware_load(
        &self,
        name: &str,
        locals: &IndexMap<String, CfmlValue>,
    ) -> Option<CfmlValue> {
        let name_lower = name.to_lowercase();
        if name_lower == "local" {
            // `local` is always the function-local scope.
            let mut scope = locals.clone();
            scope.shift_remove("__variables");
            return Some(CfmlValue::strukt(scope));
        }
        if name_lower == "variables" {
            if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                return Some(CfmlValue::Struct(vars.clone()));
            }
            return Some(CfmlValue::strukt(locals.clone()));
        }
        if name_lower == "application" {
            if let Some(ref app_scope) = self.application_scope {
                if let Ok(scope) = app_scope.lock() {
                    return Some(CfmlValue::strukt(scope.clone()));
                }
            }
        }
        if name_lower == "request" {
            return Some(CfmlValue::strukt(self.request_scope.clone()));
        }
        if let Some(v) = locals.get(name) {
            return Some(v.clone());
        }
        // Check __variables scope for CFC methods
        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
            if let Some(v) = vars.get(name).or_else(|| {
                vars.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
                    .map(|(_, v)| v)
            }) {
                return Some(v.clone());
            }
        }
        if let Some(v) = self.globals.get(name) {
            return Some(v.clone());
        }
        None
    }

    /// Shared identifier-lookup used by `LoadLocal` and `TryLoadLocal`.
    ///
    /// Walks the CFML scope chain after the caller has already handled
    /// explicit special-scope names (variables/local/request/application/
    /// session/cookie/server). The ordering is:
    ///   1. `locals` direct-case
    ///   2. `__variables` struct (direct-case, then case-insensitive)
    ///   3. `self.globals` direct-case (covers cgi/url/form/cookie/etc.
    ///      inserted by the host with lowercase keys)
    ///   4. `locals` case-insensitive scan
    ///   5. `self.globals` case-insensitive scan
    ///
    /// `name_lower` MUST be the lowercase form of `name`; the caller
    /// already computes it once for special-scope dispatch so we reuse it
    /// instead of re-allocating per scope.
    fn lookup_name_in_scopes(
        &self,
        name: &str,
        name_lower: &str,
        locals: &IndexMap<String, CfmlValue>,
    ) -> Option<CfmlValue> {
        if let Some(v) = locals.get(name) {
            return Some(v.clone());
        }
        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
            if let Some(v) = vars.get(name) {
                return Some(v.clone());
            }
            if let Some((_, v)) = vars
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name_lower))
            {
                return Some(v.clone());
            }
        }
        if let Some(v) = self.globals.get(name) {
            return Some(v.clone());
        }
        if let Some((_, v)) = locals
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name_lower))
        {
            return Some(v.clone());
        }
        if let Some((_, v)) = self
            .globals
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name_lower))
        {
            return Some(v.clone());
        }
        None
    }

    /// Store a variable by name, routing to the correct scope (locals, globals, application, request, local/variables).
    fn scope_aware_store(
        &mut self,
        name: &str,
        val: CfmlValue,
        locals: &mut IndexMap<String, CfmlValue>,
    ) {
        let name_lower = name.to_lowercase();
        if name_lower == "local" {
            // `local` is always the function-local scope — merge into locals, NOT __variables.
            if let CfmlValue::Struct(s) = val {
                let saved_vars = locals.get("__variables").cloned();
                for (k, v) in s.iter() {
                    locals.insert(k.clone(), v.clone());
                }
                if let Some(v) = saved_vars {
                    locals.insert("__variables".to_string(), v);
                }
            }
        } else if name_lower == "variables" {
            if let CfmlValue::Struct(s) = val {
                if locals.contains_key("__variables") {
                    locals.insert("__variables".to_string(), CfmlValue::Struct(s));
                } else {
                    for (k, v) in s.iter() {
                        locals.insert(k.clone(), v.clone());
                    }
                }
            }
        } else if name_lower == "application" {
            if let CfmlValue::Struct(s) = &val {
                if let Some(ref app_scope) = self.application_scope {
                    if let Ok(mut scope) = app_scope.lock() {
                        *scope = (**s).clone();
                    }
                }
            }
        } else if name_lower == "request" {
            if let CfmlValue::Struct(s) = &val {
                self.request_scope = (**s).clone();
            }
        } else if locals.contains_key(name) {
            locals.insert(name.to_string(), val);
        } else if self.globals.contains_key(name) {
            self.globals.insert(name.to_string(), val);
        } else {
            locals.insert(name.to_string(), val);
        }
    }

    fn call_member_function(
        &mut self,
        object: &CfmlValue,
        method: &str,
        extra_args: &mut Vec<CfmlValue>,
    ) -> CfmlResult {
        let method_lower = method.to_lowercase();

        // Native-object dispatch short-circuits the rest of the function.
        // Rust-backed objects implement their own method table via the
        // `CfmlNative` trait — none of the struct/component/java-shim
        // logic below applies to them. Lock for the duration of the call;
        // re-entrant calls on the same object will deadlock by design.
        if let CfmlValue::NativeObject(obj) = object {
            let args = std::mem::take(extra_args);
            let mut guard = obj.write().map_err(|_| {
                CfmlError::runtime("NativeObject lock poisoned".to_string())
            })?;
            return guard.call_method(method, args);
        }

        // Java shim dispatch must run BEFORE struct-method interception:
        // methods like append/clear/insert collide with struct-builtins and
        // would otherwise never reach the shim handler.
        if let CfmlValue::Struct(ref s) = object {
            if s.contains_key("__java_shim") {
                let java_class = s
                    .get("__java_class")
                    .map(|v| v.as_string().to_lowercase())
                    .unwrap_or_default();

                // Special: Queue.poll() returns the head and mutates in place.
                // Set method_this_writeback so the bytecode CallMethod handler
                // writes the reduced queue back to the variable.
                if method_lower == "poll"
                    && java_class == "java.util.concurrent.concurrentlinkedqueue"
                {
                    if let Some(CfmlValue::Array(q)) = s.get("__queue").cloned() {
                        if q.is_empty() {
                            return Ok(CfmlValue::Null);
                        }
                        let head = q[0].clone();
                        let mut ns = s.clone();
                        Arc::make_mut(&mut ns).insert("__queue".to_string(), CfmlValue::array(q[1..].to_vec()));
                        self.method_this_writeback = Some(CfmlValue::Struct(ns));
                        return Ok(head);
                    }
                    return Ok(CfmlValue::Null);
                }

                // Special: Map.remove(key) returns the removed value and
                // mutates in place — identical pattern to Queue.poll.
                if method_lower == "remove"
                    && matches!(
                        java_class.as_str(),
                        "java.util.concurrent.concurrenthashmap"
                            | "java.util.linkedhashmap"
                            | "java.util.treemap"
                    )
                {
                    let key = extra_args
                        .first()
                        .map(|a| a.as_string())
                        .unwrap_or_default();
                    let old = s.get(&key).cloned().unwrap_or(CfmlValue::Null);
                    let mut ns = s.clone();
                    Arc::make_mut(&mut ns).shift_remove(&key);
                    self.method_this_writeback = Some(CfmlValue::Struct(ns));
                    return Ok(old);
                }

                let all_args: Vec<CfmlValue> = std::mem::take(extra_args);
                let m = method_lower.clone();
                let result = match java_class.as_str() {
                    "java.security.messagedigest" => {
                        handle_java_messagedigest(&m, all_args, object)
                    }
                    "java.util.uuid" => handle_java_uuid(&m, all_args, object),
                    "java.lang.thread" | "java.lang.threadgroup" => {
                        handle_java_thread(&m, all_args, object)
                    }
                    "java.net.inetaddress" => handle_java_inetaddress(&m, all_args, object),
                    "java.io.file" => handle_java_file(&m, all_args, object),
                    "java.lang.system" => handle_java_system(&m, all_args, object),
                    "java.lang.stringbuilder" | "java.lang.stringbuffer" => {
                        handle_java_stringbuilder(&m, all_args, object)
                    }
                    "java.util.treemap" => handle_java_treemap(&m, all_args, object),
                    "java.util.linkedhashmap" => {
                        handle_java_linkedhashmap(&m, all_args, object)
                    }
                    "java.util.concurrent.linkedqueue"
                    | "java.util.concurrent.concurrentlinkedqueue" => {
                        handle_java_concurrentlinkedqueue(&m, all_args, object)
                    }
                    "java.util.concurrent.concurrenthashmap" => {
                        handle_java_concurrenthashmap(&m, all_args, object)
                    }
                    "java.util.collections" => {
                        handle_java_collections(&m, all_args, object)
                    }
                    "java.nio.file.paths" | "java.nio.file.path" => {
                        handle_java_paths(&m, all_args, object)
                    }
                    _ => Ok(CfmlValue::Null),
                };
                match result {
                    Ok(CfmlValue::Null) => {
                        // Shim didn't handle the method — fall through to the
                        // regular dispatch below so property access (e.g.
                        // system.out) still works.
                    }
                    Ok(val) => return Ok(val),
                    Err(e) => return Err(e),
                }
            }
        }

        // Map member function names to standalone builtin names
        // The object becomes the first argument
        let builtin_name = match object {
            CfmlValue::String(_) => match method_lower.as_str() {
                "len" | "length" => Some("len"),
                "getbytes" => {
                    // java.lang.String.getBytes() returns byte[]. Users wire
                    // this into e.g. MessageDigest.update(...).getBytes()).
                    // We honour the common no-arg form and ignore encoding
                    // arg (Rust strings are UTF-8 already).
                    return Ok(CfmlValue::Binary(object.as_string().into_bytes()));
                }
                "tochararray" => {
                    let chars: Vec<CfmlValue> = object
                        .as_string()
                        .chars()
                        .map(|c| CfmlValue::String(c.to_string()))
                        .collect();
                    return Ok(CfmlValue::array(chars));
                }
                "ucase" | "touppercase" => Some("ucase"),
                "lcase" | "tolowercase" => Some("lcase"),
                "trim" => Some("trim"),
                "ltrim" => Some("ltrim"),
                "rtrim" => Some("rtrim"),
                "reverse" => Some("reverse"),
                "left" => Some("left"),
                "right" => Some("right"),
                "mid" => Some("mid"),
                "find" | "indexof" => Some("find"),
                "findnocase" => Some("findNoCase"),
                "replace" => Some("replace"),
                "replacenocase" => Some("replaceNoCase"),
                "contains" => {
                    // "hello".contains("ell") => find("ell", "hello") > 0
                    if let Some(needle) = extra_args.first() {
                        let haystack = object.as_string().to_lowercase();
                        let needle_str = needle.as_string().to_lowercase();
                        return Ok(CfmlValue::Bool(haystack.contains(&needle_str)));
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "insert" => Some("insert"),
                "removechars" => Some("removeChars"),
                "repeatstring" | "repeat" => Some("repeatString"),
                "compare" => Some("compare"),
                "comparenocase" => Some("compareNoCase"),
                "asc" => Some("asc"),
                "chr" => Some("chr"),
                "split" => Some("listToArray"),
                "listtoarray" => Some("listToArray"),
                "listlen" => Some("listLen"),
                "listfirst" => Some("listFirst"),
                "listlast" => Some("listLast"),
                "listrest" => Some("listRest"),
                "listgetat" => Some("listGetAt"),
                "listfind" => Some("listFind"),
                "listcontains" => Some("listContains"),
                "listappend" => Some("listAppend"),
                "refind" => Some("reFind"),
                "rereplace" => Some("reReplace"),
                "rematch" => Some("reMatch"),
                "wrap" => Some("wrap"),
                "tojson" | "serializejson" => Some("serializeJSON"),
                "tonumeric" | "val" => Some("val"),
                "toboolean" => Some("toBoolean"),
                "ucfirst" => Some("ucFirst"),
                "lcfirst" => Some("lcFirst"),
                "timeformat" => Some("timeFormat"),
                "dateformat" => Some("dateFormat"),
                "datetimeformat" => Some("dateTimeFormat"),
                "lstimeformat" => Some("lsTimeFormat"),
                "lsdateformat" => Some("lsDateFormat"),
                "lsdatetimeformat" => Some("lsDateTimeFormat"),
                "parsedatetime" => Some("parseDateTime"),
                "year" => Some("year"),
                "month" => Some("month"),
                "day" => Some("day"),
                "hour" => Some("hour"),
                "minute" => Some("minute"),
                "second" => Some("second"),
                "dayofweek" => Some("dayOfWeek"),
                "dayofyear" => Some("dayOfYear"),
                _ => None,
            },
            CfmlValue::Array(arr) => match method_lower.as_str() {
                "len" | "length" | "size" => Some("arrayLen"),
                "toarray" => {
                    // .toArray() on a CFML array is a no-op; this matches
                    // java.util.Set.toArray() returning an Object[], which
                    // Lucee users chain to after keySet(). Keeping the same
                    // code path working on both engines.
                    return Ok(object.clone());
                }
                "append" | "push" => Some("arrayAppend"),
                "prepend" => Some("arrayPrepend"),
                "deleteat" => Some("arrayDeleteAt"),
                "insertat" => Some("arrayInsertAt"),
                "contains" => Some("arrayContains"),
                "containsnocase" => Some("arrayContainsNoCase"),
                "find" | "indexof" => Some("arrayFind"),
                "findnocase" => Some("arrayFindNoCase"),
                "findall" => Some("arrayFindAll"),
                "findallnocase" => Some("arrayFindAllNoCase"),
                "sort" => Some("arraySort"),
                "reverse" => Some("arrayReverse"),
                "slice" => Some("arraySlice"),
                "tolist" => Some("arrayToList"),
                "merge" => Some("arrayMerge"),
                "clear" => Some("arrayClear"),
                "min" => Some("arrayMin"),
                "max" => Some("arrayMax"),
                "avg" => Some("arrayAvg"),
                "sum" => Some("arraySum"),
                "map" => {
                    // arr.map(callback) - callback(item, index, array)
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut result = Vec::with_capacity(arr.len());
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let mapped =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.push(mapped);
                        }
                        return Ok(CfmlValue::array(result));
                    }
                    return Ok(object.clone());
                }
                "filter" => {
                    // arr.filter(callback) - callback(item, index, array)
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut result = Vec::new();
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                result.push(item.clone());
                            }
                        }
                        return Ok(CfmlValue::array(result));
                    }
                    return Ok(object.clone());
                }
                "reduce" => {
                    // arr.reduce(callback, initialValue) - callback(accumulator, item, index, array)
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut acc = extra_args.get(1).cloned().unwrap_or(CfmlValue::Null);
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "each" => {
                    // arr.each(callback) - callback(item, index, array)
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "some" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if result.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                        return Ok(CfmlValue::Bool(false));
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "every" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, item) in arr.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(item.clone());
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if !result.is_true() {
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                        return Ok(CfmlValue::Bool(true));
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "first" => {
                    return Ok(arr.first().cloned().unwrap_or(CfmlValue::Null));
                }
                "last" => {
                    return Ok(arr.last().cloned().unwrap_or(CfmlValue::Null));
                }
                "isempty" => {
                    return Ok(CfmlValue::Bool(arr.is_empty()));
                }
                "tojson" | "serializejson" => Some("serializeJSON"),
                _ => None,
            },
            CfmlValue::Struct(s) => match method_lower.as_str() {
                "count" | "len" | "size" => Some("structCount"),
                "keyexists" => Some("structKeyExists"),
                "keylist" => Some("structKeyList"),
                "keyarray" => Some("structKeyArray"),
                "delete" => Some("structDelete"),
                "insert" => Some("structInsert"),
                "update" => Some("structUpdate"),
                "find" => Some("structFind"),
                "findkey" => Some("structFindKey"),
                "findvalue" => Some("structFindValue"),
                "clear" => Some("structClear"),
                "copy" => Some("structCopy"),
                "append" => Some("structAppend"),
                "isempty" => Some("structIsEmpty"),
                "sort" => Some("structSort"),
                "each" => {
                    // struct.each(callback) - callback(key, value, struct)
                    if let Some(callback) = extra_args.first().cloned() {
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "map" => {
                    // struct.map(callback) - callback(key, value, struct) returns new value
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut result = IndexMap::new();
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let mapped =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.insert(k.clone(), mapped);
                        }
                        return Ok(CfmlValue::strukt(result));
                    }
                    return Ok(object.clone());
                }
                "filter" => {
                    // struct.filter(callback) - callback(key, value, struct) returns boolean
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut result = IndexMap::new();
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                result.insert(k.clone(), v.clone());
                            }
                        }
                        return Ok(CfmlValue::strukt(result));
                    }
                    return Ok(object.clone());
                }
                "some" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if result.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                        return Ok(CfmlValue::Bool(false));
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "every" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if !result.is_true() {
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                        return Ok(CfmlValue::Bool(true));
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                "reduce" => {
                    if extra_args.len() >= 1 {
                        let callback = extra_args[0].clone();
                        let mut acc = if extra_args.len() >= 2 {
                            extra_args[1].clone()
                        } else {
                            CfmlValue::Null
                        };
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::String(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "tojson" | "serializejson" => Some("serializeJSON"),
                _ => None,
            },
            CfmlValue::Query(q) => match method_lower.as_str() {
                "recordcount" | "len" | "size" => {
                    return Ok(CfmlValue::Int(q.rows.len() as i64));
                }
                "columnlist" => {
                    return Ok(CfmlValue::String(q.columns.join(",")));
                }
                "addrow" => Some("queryAddRow"),
                "getrow" => Some("queryGetRow"),
                "each" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "map" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut new_rows = Vec::with_capacity(q.rows.len());
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let mapped =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if let CfmlValue::Struct(s) = mapped {
                                new_rows.push(s);
                            } else {
                                new_rows.push(Arc::new(row.clone()));
                            }
                        }
                        let mut result = q.clone();
                        result.rows = new_rows.into_iter().map(|a| (*a).clone()).collect();
                        return Ok(CfmlValue::Query(result));
                    }
                    return Ok(object.clone());
                }
                "filter" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut new_rows = Vec::new();
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                new_rows.push(row.clone());
                            }
                        }
                        let mut result = q.clone();
                        result.rows = new_rows;
                        return Ok(CfmlValue::Query(result));
                    }
                    return Ok(object.clone());
                }
                "reduce" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut acc = extra_args.get(1).cloned().unwrap_or(CfmlValue::Null);
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                        return Ok(acc);
                    }
                    return Ok(CfmlValue::Null);
                }
                "sort" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut rows = q.rows.clone();
                        let n = rows.len();
                        for i in 0..n {
                            for j in 0..n.saturating_sub(1 + i) {
                                let a = CfmlValue::strukt(rows[j].clone());
                                let b = CfmlValue::strukt(rows[j + 1].clone());
                                let cb_args = vec![a, b];
                                self.closure_parent_writeback = None;
                                let cmp =
                                    self.call_function(&callback, cb_args, &IndexMap::new())?;
                                if let Some(ref wb) = self.closure_parent_writeback {
                                    Self::write_back_to_captured_scope(&callback, wb);
                                }
                                let cmp_val = match &cmp {
                                    CfmlValue::Int(n) => *n,
                                    CfmlValue::Double(d) => *d as i64,
                                    _ => 0,
                                };
                                if cmp_val > 0 {
                                    rows.swap(j, j + 1);
                                }
                            }
                        }
                        let mut result = q.clone();
                        result.rows = rows;
                        return Ok(CfmlValue::Query(result));
                    }
                    return Ok(object.clone());
                }
                "some" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if result.is_true() {
                                return Ok(CfmlValue::Bool(true));
                            }
                        }
                        return Ok(CfmlValue::Bool(false));
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "every" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, row) in q.rows.iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &IndexMap::new())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if !result.is_true() {
                                return Ok(CfmlValue::Bool(false));
                            }
                        }
                        return Ok(CfmlValue::Bool(true));
                    }
                    return Ok(CfmlValue::Bool(true));
                }
                _ => None,
            },
            CfmlValue::Int(_) | CfmlValue::Double(_) => match method_lower.as_str() {
                "tostring" => {
                    return Ok(CfmlValue::String(object.as_string()));
                }
                "abs" => Some("abs"),
                "ceiling" | "ceil" => Some("ceiling"),
                "floor" => Some("floor"),
                "round" => Some("round"),
                _ => None,
            },
            _ => None,
        };

        if let Some(name) = builtin_name {
            // Build args list: object as first arg, then extra args
            let mut args = vec![object.clone()];
            args.append(extra_args);

            // For string member functions where the standalone signature has the
            // "main" string as the second arg (e.g., find(substring, string),
            // insert(substring, string, pos)), swap the first two args so the
            // object (which the member was called on) becomes the second arg.
            if matches!(object, CfmlValue::String(_)) && args.len() >= 2 {
                match name {
                    "find" | "findNoCase" | "insert" => {
                        args.swap(0, 1);
                    }
                    _ => {}
                }
            }

            // Look up the builtin (case-insensitive)
            let name_lower = name.to_lowercase();
            if let Some(builtin) = self.builtins.get(name) {
                return builtin(args);
            }
            // Case-insensitive fallback
            let builtin_match = self
                .builtins
                .iter()
                .find(|(k, _)| k.to_lowercase() == name_lower)
                .map(|(_, v)| *v);
            if let Some(builtin) = builtin_match {
                return builtin(args);
            }
        }

        // NOTE: Java shim routing lives at the top of this function (before
        // struct-builtin interception), so it already ran for any __java_shim
        // receiver. Control only reaches here for non-shim objects.

        // If no builtin match found, try to get property and call it
        // This handles user-defined methods on components
        let prop = if let CfmlValue::Struct(ref s) = object {
            let method_lower = method.to_lowercase();
            s.iter()
                .find(|(k, _)| k.to_lowercase() == method_lower)
                .map(|(_, v)| v.clone())
                .unwrap_or(CfmlValue::Null)
        } else {
            object.get(method).unwrap_or(CfmlValue::Null)
        };
        if let CfmlValue::Function(ref _fdata) = &prop {
            let func_ref = prop.clone();
            let args: Vec<CfmlValue> = extra_args.drain(..).collect();
            // Bind 'this' / __variables ONLY when the receiver is an actual CFC.
            // For plain struct-stored closures (e.g. `enc = { string: fn }`), the
            // closure should keep whatever `this` it captured at definition (or
            // none) — setting `this = receiver` triggers method-writeback that
            // replaces `enc` in the caller's locals with the closure's captured
            // `this` on each subsequent call.
            let receiver_is_cfc = matches!(
                object,
                CfmlValue::Struct(ref s) if s.contains_key("__variables") || s.contains_key("__name")
            );
            let mut method_locals = IndexMap::new();
            if receiver_is_cfc {
                if let CfmlValue::Struct(ref s) = object {
                    if let Some(vars) = s.get("__variables") {
                        method_locals.insert("__variables".to_string(), vars.clone());
                    }
                }
                method_locals.insert("this".to_string(), object.clone());
            }
            self.closure_parent_writeback = None;
            // Save & clear method-writeback state so a stale `this` from the
            // captured scope can't leak to the caller's CallMethod handler.
            let saved_this_wb = self.method_this_writeback.take();
            let saved_vars_wb = self.method_variables_writeback.take();
            let result = self.call_function(&func_ref, args, &method_locals)?;
            if let Some(ref wb) = self.closure_parent_writeback {
                Self::write_back_to_captured_scope(&func_ref, wb);
            }
            // Clear writeback — component method calls don't leak to calling scope
            self.closure_parent_writeback = None;
            if !receiver_is_cfc {
                // Discard any method-writeback that the inner call set up; for
                // plain struct receivers, there is no `this` to write back to.
                self.method_this_writeback = saved_this_wb;
                self.method_variables_writeback = saved_vars_wb;
            }
            return Ok(result);
        }

        // Implicit property accessors (getXxx / setXxx) for components
        if let CfmlValue::Struct(ref s) = object {
            if s.contains_key("__name") || s.iter().any(|(k, _)| k.to_lowercase() == "__properties")
            {
                let method_lower = method.to_lowercase();
                if method_lower.starts_with("get") && method_lower.len() > 3 {
                    let prop_name = &method[3..];
                    let val = s
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == prop_name.to_lowercase())
                        .map(|(_, v)| v.clone());
                    if let Some(v) = val {
                        return Ok(v);
                    }
                }
                if method_lower.starts_with("set") && method_lower.len() > 3 {
                    let prop_name = &method[3..];
                    if let Some(value) = extra_args.first() {
                        let mut modified = object.clone();
                        if let Some(ms) = modified.as_struct_mut() {
                            let actual_key = ms
                                .keys()
                                .find(|k| k.to_lowercase() == prop_name.to_lowercase())
                                .cloned()
                                .unwrap_or_else(|| prop_name.to_string());
                            ms.insert(actual_key, value.clone());
                        }
                        return Ok(modified);
                    }
                }
            }
        }

        // Rust-parent fall-through: a CFC that extends="rust:Name" carries
        // its parent under __super as a NativeObject. If the method wasn't
        // found on the CFC and didn't match an implicit accessor, dispatch
        // it to the native parent. Mirrors how CFC-extends-CFC inheritance
        // merges parent methods into the child — the Rust parent isn't
        // merged, so it needs an explicit fall-through.
        if let CfmlValue::Struct(ref s) = object {
            if let Some(CfmlValue::NativeObject(parent_obj)) = s.get("__super").cloned() {
                let args: Vec<CfmlValue> = extra_args.drain(..).collect();
                let mut guard = parent_obj.write().map_err(|_| {
                    CfmlError::runtime("NativeObject lock poisoned".to_string())
                })?;
                return guard.call_method(method, args);
            }
        }

        // onMissingMethod fallback for components
        if let CfmlValue::Struct(ref s) = object {
            let missing_handler = s
                .iter()
                .find(|(k, _)| k.to_lowercase() == "onmissingmethod")
                .map(|(_, v)| v.clone());
            if let Some(handler @ CfmlValue::Function(_)) = missing_handler {
                let args_array: Vec<CfmlValue> = extra_args.drain(..).collect();
                let mut missing_args = IndexMap::new();
                for (i, a) in args_array.iter().enumerate() {
                    missing_args.insert((i + 1).to_string(), a.clone());
                }
                let mut method_locals = IndexMap::new();
                if let CfmlValue::Struct(ref s2) = object {
                    if let Some(vars) = s2.get("__variables") {
                        method_locals.insert("__variables".to_string(), vars.clone());
                    }
                }
                method_locals.insert("this".to_string(), object.clone());
                return self.call_function(
                    &handler,
                    vec![
                        CfmlValue::String(method.to_string()),
                        CfmlValue::strukt(missing_args),
                    ],
                    &method_locals,
                );
            }
        }

        Ok(CfmlValue::Null)
    }

    /// Check if a variable name (possibly dotted like "request.data.name") is defined
    /// by walking the scope chain: locals → request → application → server → globals
    fn is_variable_defined(&self, var_name: &str, locals: &IndexMap<String, CfmlValue>) -> bool {
        let parts: Vec<&str> = var_name.split('.').collect();
        if parts.is_empty() {
            return false;
        }

        let root = parts[0].to_lowercase();

        // Try to resolve the root variable from scope chain
        let root_val = if root == "local" || root == "variables" {
            Some(CfmlValue::strukt(locals.clone()))
        } else if root == "request" {
            Some(CfmlValue::strukt(self.request_scope.clone()))
        } else if root == "application" {
            if let Some(ref app_scope) = self.application_scope {
                if let Ok(scope) = app_scope.lock() {
                    Some(CfmlValue::strukt(scope.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        } else if root == "session" {
            Some(self.get_session_scope())
        } else if root == "cookie" {
            self.globals
                .get("cookie")
                .cloned()
                .or(Some(CfmlValue::strukt(IndexMap::new())))
        } else if root == "server" {
            Some(CfmlValue::strukt(build_server_scope()))
        } else {
            // Check locals (exact then CI)
            locals
                .get(parts[0])
                .cloned()
                .or_else(|| {
                    locals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == root)
                        .map(|(_, v)| v.clone())
                })
                // Check request scope
                .or_else(|| {
                    self.request_scope
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == root)
                        .map(|(_, v)| v.clone())
                })
                // Check globals
                .or_else(|| self.globals.get(parts[0]).cloned())
                .or_else(|| {
                    self.globals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == root)
                        .map(|(_, v)| v.clone())
                })
        };

        let root_val = match root_val {
            Some(v) => v,
            None => return false,
        };

        if parts.len() == 1 {
            return true;
        }

        // Walk the dotted path segments
        let mut current = root_val;
        // For scope-named roots (request, local, etc.), start resolving from parts[1]
        // For regular vars, start from parts[1] too
        for &segment in &parts[1..] {
            let seg_lower = segment.to_lowercase();
            match &current {
                CfmlValue::Struct(s) => {
                    if let Some(v) = s
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == seg_lower)
                        .map(|(_, v)| v.clone())
                    {
                        current = v;
                    } else {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// Shallow equality check for CfmlValues — avoids recursing into captured
    /// scopes (which could cause infinite recursion with shared environments).
    fn values_equal_shallow(a: &CfmlValue, b: &CfmlValue) -> bool {
        Self::values_equal_shallow_depth(a, b, 0)
    }

    fn values_equal_shallow_depth(a: &CfmlValue, b: &CfmlValue, depth: usize) -> bool {
        // Guard against circular references and exponential blowup in
        // deeply nested structs (e.g., scope chains with function captures).
        // Depth 3 catches practical top-level changes while avoiding O(n^d) cost.
        if depth > 3 {
            return false;
        }
        match (a, b) {
            (CfmlValue::Null, CfmlValue::Null) => true,
            (CfmlValue::Bool(a), CfmlValue::Bool(b)) => a == b,
            (CfmlValue::Int(a), CfmlValue::Int(b)) => a == b,
            (CfmlValue::Double(a), CfmlValue::Double(b)) => a == b,
            (CfmlValue::String(a), CfmlValue::String(b)) => a == b,
            (CfmlValue::Array(a), CfmlValue::Array(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| Self::values_equal_shallow_depth(x, y, depth + 1))
            }
            (CfmlValue::Struct(a), CfmlValue::Struct(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.get(k)
                            .map_or(false, |bv| Self::values_equal_shallow_depth(v, bv, depth + 1))
                    })
            }
            // Functions: compare by name only (avoids recursing into captured scopes).
            // Functions with the same name are considered equal for writeback diffing
            // since function definitions don't change at runtime.
            (CfmlValue::Function(a), CfmlValue::Function(b)) => a.name == b.name,
            (CfmlValue::Query(a), CfmlValue::Query(b)) => {
                a.columns == b.columns
                    && a.rows.len() == b.rows.len()
                    && a.rows.iter().zip(b.rows.iter()).all(|(ra, rb)| {
                        ra.len() == rb.len()
                            && ra.iter().all(|(k, v)| {
                                rb.get(k)
                                    .map_or(false, |bv| Self::values_equal_shallow_depth(v, bv, depth + 1))
                            })
                    })
            }
            (CfmlValue::Binary(a), CfmlValue::Binary(b)) => a == b,
            // NativeObjects: pointer identity. Two refs to the same Arc are
            // equal; otherwise treat as different (the underlying Rust state
            // is opaque to the writeback diff).
            (CfmlValue::NativeObject(a), CfmlValue::NativeObject(b)) => Arc::ptr_eq(a, b),
            // Components: treat as always different (complex state)
            _ => false,
        }
    }

    /// Collect modified complex-type (Struct, Array, Query) argument values for
    /// pass-by-reference writeback. Called at function return to store final param values.
    /// Stores (param_index, value) pairs so the caller can match to arg sources.
    fn collect_arg_ref_writeback(
        &mut self,
        func: &BytecodeFunction,
        locals: &IndexMap<String, CfmlValue>,
    ) {
        if func.params.is_empty() {
            self.arg_ref_writeback = None;
            return;
        }
        let mut writeback = Vec::new();
        for (i, param_name) in func.params.iter().enumerate() {
            if let Some(val) = locals.get(param_name.as_str()) {
                match val {
                    CfmlValue::Struct(_)
                    | CfmlValue::Array(_)
                    | CfmlValue::Query(_)
                    | CfmlValue::Component(_) => {
                        writeback.push((i.to_string(), val.clone()));
                    }
                    _ => {}
                }
            }
        }
        self.arg_ref_writeback = if writeback.is_empty() {
            None
        } else {
            Some(writeback)
        };
    }

    /// Write back mutations into a closure's shared Arc<RwLock> environment.
    /// Only updates variables that already exist in the captured scope (prevents pollution).
    fn write_back_to_captured_scope(func_ref: &CfmlValue, writeback: &IndexMap<String, CfmlValue>) {
        if let CfmlValue::Function(ref f) = func_ref {
            if let Some(ref shared_env) = f.captured_scope {
                let mut env = shared_env.write().unwrap();
                for (k, v) in writeback {
                    env.insert(k.clone(), v.clone());
                }
            }
        }
    }

    /// Compute final closure write-back after a higher-order function loop.
    /// Compares modified locals against original parent_locals and sets closure_parent_writeback.
    fn set_ho_final_writeback(
        &mut self,
        modified: &IndexMap<String, CfmlValue>,
        original: &IndexMap<String, CfmlValue>,
    ) {
        let mut final_wb = IndexMap::new();
        for (k, v) in modified {
            match original.get(k) {
                Some(pv) => {
                    if !Self::values_equal_shallow(v, pv) {
                        final_wb.insert(k.clone(), v.clone());
                    }
                }
                None => {
                    final_wb.insert(k.clone(), v.clone());
                }
            }
        }
        if !final_wb.is_empty() {
            self.closure_parent_writeback = Some(final_wb);
        }
    }

    /// Resolve a dot-path class name to a .cfc file path using component mappings.
    /// Mappings are sorted longest-prefix-first for correct precedence.
    fn resolve_path_with_mappings(&self, class_name: &str) -> Option<String> {
        if self.mappings.is_empty() {
            return None;
        }
        // Convert dot-path to slash-path: "taffy.core.api" → "/taffy/core/api"
        let slash_path = format!("/{}", class_name.replace('.', "/"));
        let slash_lower = slash_path.to_lowercase();

        for mapping in &self.mappings {
            let prefix_lower = mapping.name.to_lowercase();
            if slash_lower.starts_with(&prefix_lower)
                || (mapping.name == "/" && slash_lower.starts_with('/'))
            {
                let remainder = if mapping.name == "/" {
                    &slash_path[1..] // Strip leading /
                } else {
                    &slash_path[mapping.name.len()..]
                };
                let remainder = remainder.trim_start_matches('/');
                let cfc_path = format!(
                    "{}/{}.cfc",
                    mapping.path.trim_end_matches('/'),
                    remainder.replace('/', std::path::MAIN_SEPARATOR_STR)
                );
                if self.vfs.exists(&cfc_path) {
                    return Some(cfc_path);
                }
            }
        }
        None
    }

    /// Resolve an include path (e.g. "/taffy/core/foo.cfm") using component mappings.
    /// Resolve a CFML leading-slash include path (`/foo/bar.cfm`) by trying,
    /// in order: configured `this.mappings` from Application.cfc, the
    /// serve-mode webroot, then the CLI-mode entry template's parent
    /// directory. Returns `None` only if none of those produce an existing
    /// file. Leading-slash means "webroot-relative" in CFML — it must not be
    /// interpreted as OS-absolute.
    fn resolve_leading_slash_include(&self, include_path: &str) -> Option<String> {
        if let Some(via_mapping) = self.resolve_include_with_mappings(include_path) {
            return Some(via_mapping);
        }
        let stripped = include_path.trim_start_matches('/');
        if let Some(webroot) = self.server_state.as_ref().and_then(|s| s.webroot.as_ref()) {
            let candidate = webroot.join(stripped).to_string_lossy().to_string();
            if self.vfs.exists(&candidate) {
                return Some(candidate);
            }
        }
        if let Some(ref base) = self.base_template_path {
            let base_dir = std::path::Path::new(base)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let candidate = base_dir.join(stripped).to_string_lossy().to_string();
            if self.vfs.exists(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn resolve_include_with_mappings(&self, include_path: &str) -> Option<String> {
        if self.mappings.is_empty() {
            return None;
        }
        let path_lower = include_path.to_lowercase();
        for mapping in &self.mappings {
            let prefix_lower = mapping.name.to_lowercase();
            if path_lower.starts_with(&prefix_lower)
                || (mapping.name == "/" && path_lower.starts_with('/'))
            {
                let remainder = if mapping.name == "/" {
                    &include_path[1..]
                } else {
                    &include_path[mapping.name.len()..]
                };
                let remainder = remainder.trim_start_matches('/');
                let resolved = format!("{}/{}", mapping.path.trim_end_matches('/'), remainder);
                if self.vfs.exists(&resolved) {
                    return Some(resolved);
                }
            }
        }
        None
    }

    /// Get or create the cfthread scope on the variables scope.
    fn get_or_create_cfthread_scope(&mut self) -> &mut CfmlValue {
        if !self.globals.contains_key("cfthread") {
            self.globals
                .insert("cfthread".to_string(), CfmlValue::strukt(IndexMap::new()));
        }
        self.globals.get_mut("cfthread").unwrap()
    }

    /// Resolve a custom tag path specification to an actual filesystem path.
    fn resolve_custom_tag_path(&self, path_spec: &str) -> Result<String, CfmlError> {
        if path_spec.starts_with("__cf_:") {
            // cf_ prefix tag: find tagname.cfm
            let tag_name = &path_spec[6..];
            let filename = format!("{}.cfm", tag_name);

            // 1) Look in calling template directory
            if let Some(ref source) = self.source_file {
                let source_dir = std::path::Path::new(source)
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                let candidate = source_dir.join(&filename).to_string_lossy().to_string();
                if self.vfs.exists(&candidate) {
                    return Ok(candidate);
                }
            }

            // 2) Look in custom_tag_paths
            for dir in &self.custom_tag_paths {
                let candidate = std::path::Path::new(dir)
                    .join(&filename)
                    .to_string_lossy()
                    .to_string();
                if self.vfs.exists(&candidate) {
                    return Ok(candidate);
                }
            }

            // 3) Look in mappings
            for mapping in &self.mappings {
                let candidate = std::path::Path::new(&mapping.path)
                    .join(&filename)
                    .to_string_lossy()
                    .to_string();
                if self.vfs.exists(&candidate) {
                    return Ok(candidate);
                }
            }

            Err(CfmlError::runtime(format!(
                "Custom tag 'cf_{}' not found",
                tag_name
            )))
        } else if path_spec.starts_with("__name:") {
            // cfmodule name="dot.path" → convert dots to slashes
            let dot_path = &path_spec[7..];
            let rel_path = format!("{}.cfm", dot_path.replace('.', "/"));

            // Search in custom_tag_paths then mappings
            for dir in &self.custom_tag_paths {
                let candidate = std::path::Path::new(dir)
                    .join(&rel_path)
                    .to_string_lossy()
                    .to_string();
                if self.vfs.exists(&candidate) {
                    return Ok(candidate);
                }
            }

            for mapping in &self.mappings {
                let candidate = std::path::Path::new(&mapping.path)
                    .join(&rel_path)
                    .to_string_lossy()
                    .to_string();
                if self.vfs.exists(&candidate) {
                    return Ok(candidate);
                }
            }

            Err(CfmlError::runtime(format!(
                "Custom tag with name '{}' not found",
                dot_path
            )))
        } else {
            // Plain path: resolve relative to source_file
            let resolved = if let Some(ref source) = self.source_file {
                let source_dir = std::path::Path::new(source)
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                source_dir.join(path_spec).to_string_lossy().to_string()
            } else {
                path_spec.to_string()
            };

            if self.vfs.exists(&resolved) {
                Ok(resolved)
            } else {
                Err(CfmlError::runtime(format!(
                    "Custom tag template '{}' not found",
                    path_spec
                )))
            }
        }
    }

    /// Execute a custom tag template file with the given tag-local variables.
    /// Reuses the include pattern: save/restore source_file, program, try_stack.
    fn execute_custom_tag_template(
        &mut self,
        template_path: &str,
        tag_locals: &IndexMap<String, CfmlValue>,
    ) -> Result<(), CfmlError> {
        let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
        let sub_program = compile_file_cached(template_path, cache, self.vfs.as_ref())?;

        let old_program = std::mem::replace(&mut self.program, sub_program);
        let old_source = self.source_file.clone();
        self.source_file = Some(template_path.to_string());

        let main_idx = self
            .program
            .functions
            .iter()
            .position(|f| f.name == "__main__")
            .unwrap_or(0);
        let tag_func = self.program.functions[main_idx].clone();

        let saved_try_stack = std::mem::take(&mut self.try_stack);
        let result = self.execute_function_with_args(&tag_func, Vec::new(), Some(tag_locals));
        self.try_stack = saved_try_stack;
        self.program = old_program;
        self.source_file = old_source;

        result.map(|_| ())
    }

    /// Adjust func_idx values in CfmlFunction bodies within a component struct.
    /// When sub-program functions are merged into the main program at an offset,
    /// the stored func_idx values need to be updated to reflect their new positions.
    /// Also fixes up func_idx values inside captured closure scopes so that
    /// CFC methods sharing a closure environment reference the correct indices.
    fn fixup_func_indices(val: &mut CfmlValue, offset: usize) {
        // Track which captured scopes we've already fixed (they're shared via Arc)
        let mut fixed_scopes: Vec<usize> = Vec::new();
        Self::fixup_func_indices_inner(val, offset, &mut fixed_scopes);
    }

    fn fixup_func_indices_inner(val: &mut CfmlValue, offset: usize, fixed_scopes: &mut Vec<usize>) {
        match val {
            CfmlValue::Struct(s) => {
                // We need to collect keys first, then mutate
                let keys: Vec<String> = s.keys().cloned().collect();
                for key in keys {
                    if let Some(v) = Arc::make_mut(s).get_mut(&key) {
                        match v {
                            CfmlValue::Function(ref mut f) => {
                                // Update the func_idx stored in the body
                                if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                    ref mut body,
                                ) = f.body
                                {
                                    if let CfmlValue::Int(ref mut idx) = body.as_mut() {
                                        *idx += offset as i64;
                                    }
                                }
                                // Also fix up func_idx values inside the captured scope
                                if let Some(ref shared_env) = f.captured_scope {
                                    let ptr = Arc::as_ptr(shared_env) as usize;
                                    if !fixed_scopes.contains(&ptr) {
                                        fixed_scopes.push(ptr);
                                        if let Ok(mut env) = shared_env.write() {
                                            let env_keys: Vec<String> =
                                                env.keys().cloned().collect();
                                            for ek in env_keys {
                                                if let Some(ev) = env.get_mut(&ek) {
                                                    if let CfmlValue::Function(ref mut ef) = ev {
                                                        if let cfml_common::dynamic::CfmlClosureBody::Expression(ref mut body) = ef.body {
                                                            if let CfmlValue::Int(ref mut idx) = body.as_mut() {
                                                                *idx += offset as i64;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            CfmlValue::Struct(_) => {
                                Self::fixup_func_indices_inner(v, offset, fixed_scopes);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Fix func_idx for functions that came from a cfinclude sub-program.
    /// Sub-program func at index i (where i > 0, skipping __main__) is now at
    /// base_idx + (i - 1) in the merged program.
    fn fixup_included_func_indices(val: &mut CfmlValue, base_idx: usize, sub_func_count: usize) {
        match val {
            CfmlValue::Struct(s) => {
                let keys: Vec<String> = s.keys().cloned().collect();
                for key in keys {
                    if let Some(v) = Arc::make_mut(s).get_mut(&key) {
                        match v {
                            CfmlValue::Function(ref mut f) => {
                                if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                    ref mut body,
                                ) = f.body
                                {
                                    if let CfmlValue::Int(ref mut idx) = body.as_mut() {
                                        let i = *idx as usize;
                                        // Only fix indices that belong to the sub-program
                                        if i > 0 && i < sub_func_count {
                                            *idx = (base_idx + i - 1) as i64;
                                        }
                                    }
                                }
                            }
                            CfmlValue::Struct(_) => {
                                Self::fixup_included_func_indices(v, base_idx, sub_func_count);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Adjust func_idx values by a delta for indices >= min_index.
    /// Used when restoring cached functions at a different program offset
    /// than where they were originally inserted.
    fn adjust_func_indices(val: &mut CfmlValue, min_index: usize, delta: i64) {
        match val {
            CfmlValue::Struct(s) => {
                let keys: Vec<String> = s.keys().cloned().collect();
                for key in keys {
                    if let Some(v) = Arc::make_mut(s).get_mut(&key) {
                        match v {
                            CfmlValue::Function(ref mut f) => {
                                if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                    ref mut body,
                                ) = f.body
                                {
                                    if let CfmlValue::Int(ref mut idx) = body.as_mut() {
                                        if *idx >= min_index as i64 {
                                            *idx += delta;
                                        }
                                    }
                                }
                            }
                            CfmlValue::Struct(_) | CfmlValue::Array(_) => {
                                Self::adjust_func_indices(v, min_index, delta);
                            }
                            _ => {}
                        }
                    }
                }
            }
            CfmlValue::Array(arr) => {
                for item in Arc::make_mut(arr).iter_mut() {
                    Self::adjust_func_indices(item, min_index, delta);
                }
            }
            _ => {}
        }
    }

    /// Get default datasource from application scope or request scope
    fn get_default_datasource(&self, parent_locals: &IndexMap<String, CfmlValue>) -> String {
        // Check application scope for datasource config
        if let Some(ref app_scope) = self.application_scope {
            if let Ok(scope) = app_scope.lock() {
                if let Some(ds) = scope
                    .get("datasource")
                    .or_else(|| scope.get("defaultdatasource"))
                {
                    let s = ds.as_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
        // Check local variables
        if let Some(ds) = parent_locals.get("datasource") {
            let s = ds.as_string();
            if !s.is_empty() {
                return s;
            }
        }
        String::new()
    }

    /// Lazy-init handler: when `this.lazySessionCreation = true` and
    /// the request hasn't created a session record yet, the next write
    /// to `session.X` (or session-affecting call like `cfloginuser`)
    /// flows through this method first. It mints a session id if none
    /// is set, inserts an empty record, fires `onSessionStart`
    /// synchronously, and clears the pending flag.
    ///
    /// Reads of the session scope deliberately do NOT trigger this:
    /// reading a non-existent key returns the default (empty / null)
    /// without ever materialising a backing record. Only writes count,
    /// matching the Preside-CMS session-storage pattern.
    ///
    /// A re-entry guard prevents recursion when `onSessionStart` itself
    /// writes to session (which is the entire point of the lifecycle
    /// hook).
    /// Returns `true` if init actually ran on this call (so the
    /// caller knows the session record now contains whatever
    /// `onSessionStart` added and may need to merge rather than
    /// replace). Returns `false` if there was nothing to do.
    fn lazy_init_session_if_pending(&mut self) -> bool {
        if !self.session_lazy_pending || self.session_lazy_initializing {
            return false;
        }
        self.session_lazy_initializing = true;

        let sid = match self.session_id.clone() {
            Some(s) if !s.is_empty() => s,
            _ => {
                let new_sid = uuid::Uuid::new_v4().to_string();
                self.session_id = Some(new_sid.clone());
                new_sid
            }
        };

        if let Some(state) = self.server_state.clone() {
            let now = now_epoch_secs();
            state.sessions.set(
                &sid,
                SessionData {
                    variables: IndexMap::new(),
                    created_secs: now,
                    last_accessed_secs: now,
                    auth_user: None,
                    auth_roles: Vec::new(),
                    timeout_secs: self.session_timeout_secs,
                },
            );
        }
        self.session_record_created = true;
        self.session_lazy_pending = false;

        // Fire onSessionStart using the stashed Application.cfc
        // template. Any writes inside the lifecycle hook re-enter this
        // method but bail at the initializing guard.
        if let Some(mut tpl) = self.app_cfc_template.take() {
            let _ = self.call_lifecycle_method(&mut tpl, "onSessionStart", vec![]);
            self.app_cfc_template = Some(tpl);
        }

        self.session_lazy_initializing = false;
        true
    }

    /// Get the session scope for the current request
    fn get_session_scope(&self) -> CfmlValue {
        if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id) {
            if let Some(session) = state.sessions.get(sid) {
                return CfmlValue::strukt(session.variables.clone());
            }
        }
        CfmlValue::strukt(IndexMap::new())
    }

    /// Set the session scope for the current request
    fn set_session_scope(&mut self, vars: IndexMap<String, CfmlValue>) {
        // If lazy-init fires here, `onSessionStart` ran AFTER the user
        // code loaded the (then-empty) session scope. Their `vars`
        // snapshot doesn't contain any keys onSessionStart set, so we
        // merge: existing keys (from onSessionStart) are preserved,
        // and the user's writes overlay on top. Outside the lazy-init
        // case we fall through to the historical full-replace.
        let merge_with_existing = self.lazy_init_session_if_pending();
        if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id) {
            let now = now_epoch_secs();
            let mut session = state.sessions.get(sid).unwrap_or_else(|| SessionData {
                variables: IndexMap::new(),
                created_secs: now,
                last_accessed_secs: now,
                auth_user: None,
                auth_roles: Vec::new(),
                timeout_secs: 1800,
            });
            if merge_with_existing {
                for (k, v) in vars {
                    session.variables.insert(k, v);
                }
            } else {
                session.variables = vars;
            }
            session.last_accessed_secs = now;
            state.sessions.set(sid, session);
        }
    }

    /// Update a single key in the session scope
    #[allow(dead_code)]
    fn set_session_variable(&mut self, key: &str, value: CfmlValue) {
        self.lazy_init_session_if_pending();
        if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id) {
            let now = now_epoch_secs();
            let mut session = state.sessions.get(sid).unwrap_or_else(|| SessionData {
                variables: IndexMap::new(),
                created_secs: now,
                last_accessed_secs: now,
                auth_user: None,
                auth_roles: Vec::new(),
                timeout_secs: 1800,
            });
            session.variables.insert(key.to_string(), value);
            session.last_accessed_secs = now;
            state.sessions.set(sid, session);
        }
    }

    /// Resolve a component template by name: tries locals, globals (exact + CI),
    /// then loads from a .cfc file on disk.
    fn resolve_component_template(
        &mut self,
        class_name: &str,
        locals: &IndexMap<String, CfmlValue>,
    ) -> Option<CfmlValue> {
        // 1. Try locals
        if let Some(val) = locals.get(class_name) {
            if matches!(val, CfmlValue::Struct(_)) {
                return Some(val.clone());
            }
        }
        // 2. Try globals (exact)
        if let Some(val) = self.globals.get(class_name) {
            if matches!(val, CfmlValue::Struct(_)) {
                return Some(val.clone());
            }
        }
        // 3. Case-insensitive lookup in globals
        let lower = class_name.to_lowercase();
        if let Some(val) = self
            .globals
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.clone())
        {
            if matches!(val, CfmlValue::Struct(_)) {
                return Some(val);
            }
        }
        // 4. Try loading .cfc file — first relative, then via mappings
        let cfc_path = {
            // If class_name is already an absolute path or has .cfc extension, use directly
            let as_path = std::path::Path::new(class_name);
            if as_path.is_absolute() || class_name.to_lowercase().ends_with(".cfc") {
                let p = if class_name.to_lowercase().ends_with(".cfc") {
                    class_name.to_string()
                } else {
                    format!("{}.cfc", class_name)
                };
                if self.vfs.exists(&p) {
                    p
                } else if let Some(ref source) = self.source_file {
                    // Try relative to source file
                    let source_dir = std::path::Path::new(source)
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    source_dir.join(&p).to_string_lossy().to_string()
                } else {
                    p
                }
            } else {
                // Dot-path: convert dots to path separators
                let relative_path = if let Some(ref source) = self.source_file {
                    let source_dir = std::path::Path::new(source)
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    let file_name = class_name.replace('.', std::path::MAIN_SEPARATOR_STR);
                    source_dir
                        .join(format!("{}.cfc", file_name))
                        .to_string_lossy()
                        .to_string()
                } else {
                    format!(
                        "{}.cfc",
                        class_name.replace('.', std::path::MAIN_SEPARATOR_STR)
                    )
                };
                if self.vfs.exists(&relative_path) {
                    relative_path
                } else if let Some(mapped) = self.resolve_path_with_mappings(class_name) {
                    mapped
                } else if let Some(ref base) = self.base_template_path {
                    // Try resolving relative to the base template (web root equivalent)
                    let base_dir = std::path::Path::new(base)
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    let file_name = class_name.replace('.', std::path::MAIN_SEPARATOR_STR);
                    let base_path = base_dir
                        .join(format!("{}.cfc", file_name))
                        .to_string_lossy()
                        .to_string();
                    if self.vfs.exists(&base_path) {
                        base_path
                    } else if let Some(webroot_path) = self
                        .server_state
                        .as_ref()
                        .and_then(|s| s.webroot.as_ref())
                        .map(|w| {
                            w.join(format!("{}.cfc", &file_name))
                                .to_string_lossy()
                                .to_string()
                        })
                    {
                        if self.vfs.exists(&webroot_path) {
                            webroot_path
                        } else {
                            relative_path
                        }
                    } else {
                        relative_path
                    }
                } else if let Some(webroot_path) = self
                    .server_state
                    .as_ref()
                    .and_then(|s| s.webroot.as_ref())
                    .map(|w| {
                        let file_name = class_name.replace('.', std::path::MAIN_SEPARATOR_STR);
                        w.join(format!("{}.cfc", file_name))
                            .to_string_lossy()
                            .to_string()
                    })
                {
                    if self.vfs.exists(&webroot_path) {
                        webroot_path
                    } else {
                        relative_path
                    }
                } else {
                    relative_path // Fall back to relative (will fail at read_to_string below)
                }
            }
        };

        let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
        if let Ok(sub_program) = compile_file_cached(&cfc_path, cache, self.vfs.as_ref()) {
            let old_program = std::mem::replace(&mut self.program, sub_program);
            // Set source_file to CFC path so parent resolution works relative to CFC
            let old_source_file = self.source_file.clone();
            self.source_file = Some(cfc_path.clone());
            let main_idx = self
                .program
                .functions
                .iter()
                .position(|f| f.name == "__main__")
                .unwrap_or(0);
            let cfc_func = self.program.functions[main_idx].clone();

            // Bug G fix: detect `extends="<parent>"` in the cfc body bytecode
            // (codegen emits `String("__extends"); String(<parent>)` consecutively
            // when building the component struct) and pre-resolve the parent so
            // its `variables` scope is visible during child body execution. Without
            // this, page-level statements that reference inherited helpers
            // (e.g. `variables.encode.string(...)` from `taffy.core.resource`) throw
            // because inheritance only merges AFTER the child body has run.
            //
            // We deliberately copy parent's __variables and let the existing filter
            // in execute_function_with_args (line ~621) strip Function values —
            // injecting parent methods into the child body's initial scope causes
            // recursion through bound this/__variables (see TAFFY_NEXT_STEPS.md).
            let parent_name: Option<String> =
                cfc_func.instructions.windows(2).find_map(|w| match w {
                    [BytecodeOp::String(s1), BytecodeOp::String(s2)] if s1 == "__extends" => {
                        Some(s2.clone())
                    }
                    _ => None,
                });
            let injected_scope: IndexMap<String, CfmlValue> = if let Some(pname) = parent_name {
                if let Some(parent_template) = self.resolve_component_template(&pname, locals) {
                    let resolved_parent = self.resolve_inheritance(parent_template, locals);
                    if let CfmlValue::Struct(ref ps) = resolved_parent {
                        if let Some(CfmlValue::Struct(parent_vars)) = ps.get("__variables") {
                            (**parent_vars).clone()
                        } else {
                            IndexMap::new()
                        }
                    } else {
                        IndexMap::new()
                    }
                } else {
                    IndexMap::new()
                }
            } else {
                IndexMap::new()
            };

            // Snapshot user_functions AFTER parent resolution (parent body may have
            // registered helper functions which should not be flagged as
            // "added by cfinclude" inside this child body).
            let pre_exec_func_names: std::collections::HashSet<String> =
                self.user_functions.keys().cloned().collect();
            // CFC body executes with a scope containing parent's variables (so
            // unscoped lookups inside the child body resolve inherited values).
            // Mark as "__cfc_body__" so the VM treats it as function scope
            // (prevents globals leaking into `variables` via LoadLocal).
            let mut cfc_body = (*cfc_func).clone();
            cfc_body.name = "__cfc_body__".to_string();
            let _ = self.execute_function_with_args(&cfc_body, Vec::new(), Some(&injected_scope));
            self.source_file = old_source_file;
            // Capture component body variables
            let component_variables = self.captured_locals.take().unwrap_or_default();
            // Merge sub-program functions — track base offset for func_idx fixup
            let sub_funcs = self.program.functions.clone();
            self.program = old_program;
            let base_idx = self.program.functions.len();
            for func in sub_funcs {
                if func.name != "__main__" {
                    self.program.functions.push(Arc::clone(&func));
                    if self.user_functions.contains_key(&func.name) {
                        self.user_functions
                            .insert(func.name.clone(), Arc::clone(&func));
                    }
                }
            }
            // Fix up func_idx in the component struct stored in globals
            // Sub-program functions were at indices [0..N), now at [base_idx..base_idx+N)
            let short_name = class_name.split('.').last().unwrap_or(class_name);
            let mut result = self
                .globals
                .get(class_name)
                .cloned()
                .or_else(|| self.globals.get(short_name).cloned())
                .or_else(|| {
                    let lower = class_name.to_lowercase();
                    self.globals
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == lower)
                        .map(|(_, v)| v.clone())
                })
                .or_else(|| self.globals.get("Anonymous").cloned());
            // Adjust func_idx in all function values.
            // Sub-program had __main__ at index 0 which was skipped during merge.
            // So sub-program func at index i is now at base_idx + (i - 1).
            // The offset to add is (base_idx - 1).
            if base_idx > 0 {
                if let Some(ref mut val) = result {
                    Self::fixup_func_indices(val, base_idx - 1);
                }
            }
            // Strip captured_scope from all CFC methods on the component struct.
            // CFC methods are NOT closures — they were compiled in the CFC body
            // context where DefineFunction attaches a captured scope, but that scope
            // carries stale/unfixed data.  CFC method scope resolution should use
            // __variables (injected at call time), not captured scopes.
            if let Some(s) = result.as_mut().and_then(|v| v.as_struct_mut()) {
                for (_, v) in s.iter_mut() {
                    if let CfmlValue::Function(ref mut f) = v {
                        f.captured_scope = None;
                    }
                }
            }
            // Store the CFC source path for parent resolution during inheritance
            if let Some(s) = result.as_mut().and_then(|v| v.as_struct_mut()) {
                s.insert(
                    "__source_file".to_string(),
                    CfmlValue::String(cfc_path.clone()),
                );
                // Anonymous `component { ... }` declarations get __name = "Anonymous"
                // baked in by the parser. Override with the dotted path the caller
                // used (e.g. "oop.Greeter") so getMetadata(cfc).name matches Lucee/ACF.
                let needs_override = match s.get("__name") {
                    Some(CfmlValue::String(n)) => n == "Anonymous",
                    _ => true,
                };
                if needs_override {
                    s.insert(
                        "__name".to_string(),
                        CfmlValue::String(class_name.to_string()),
                    );
                }
            }
            // Inject functions added by cfinclude inside the component body
            // These were registered in user_functions during execution but aren't
            // in the component struct (which was built at compile time)
            if let Some(s) = result.as_mut().and_then(|v| v.as_struct_mut()) {
                let existing_keys: std::collections::HashSet<String> =
                    s.keys().map(|k| k.to_lowercase()).collect();
                for (func_name, func_def) in &self.user_functions {
                    if !pre_exec_func_names.contains(func_name)
                        && !existing_keys.contains(&func_name.to_lowercase())
                    {
                        // Find the func_idx in the merged program
                        if let Some(idx) = self
                            .program
                            .functions
                            .iter()
                            .position(|f| f.name == *func_name)
                        {
                            let cf = CfmlValue::Function(cfml_common::dynamic::CfmlFunction {
                                name: func_name.clone(),
                                params: func_def
                                    .params
                                    .iter()
                                    .enumerate()
                                    .map(|(i, name)| cfml_common::dynamic::CfmlParam {
                                        name: name.clone(),
                                        param_type: None,
                                        default: None,
                                        required: func_def
                                            .required_params
                                            .get(i)
                                            .copied()
                                            .unwrap_or(false),
                                    })
                                    .collect(),
                                body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                    CfmlValue::Int(idx as i64),
                                )),
                                return_type: None,
                                access: cfml_common::dynamic::CfmlAccess::Public,
                                captured_scope: None,
                            });
                            s.insert(func_name.clone(), cf);
                        }
                    }
                }
            }
            // Store component body variables + all methods as __variables
            // In CFML, component methods live in the variables scope so
            // unqualified calls inside methods resolve via the normal scope chain.
            if let Some(s) = result.as_mut().and_then(|v| v.as_struct_mut()) {
                let mut vars_scope: IndexMap<String, CfmlValue> = IndexMap::new();
                // Add component body variables (non-function values from pseudo-constructor)
                // Functions from component_variables have sub-program indices that need
                // fixup (offset = base_idx - 1), and captured_scopes must be stripped.
                let cv_offset = if base_idx > 0 { base_idx - 1 } else { 0 };
                for (k, v) in &component_variables {
                    let k_lower = k.to_lowercase();
                    if k_lower == "this" || k_lower == "arguments" || k.starts_with("__") {
                        continue;
                    }
                    if let CfmlValue::Function(ref f) = v {
                        // Fix body index and strip captured scope
                        let mut clean = f.clone();
                        clean.captured_scope = None;
                        if let cfml_common::dynamic::CfmlClosureBody::Expression(ref mut body) =
                            clean.body
                        {
                            if let CfmlValue::Int(ref mut idx) = body.as_mut() {
                                *idx += cv_offset as i64;
                            }
                        }
                        vars_scope.insert(k.clone(), CfmlValue::Function(clean));
                    } else {
                        // Recursively fix Function indices inside nested Struct values
                        // (e.g. `variables.encode = { string: function(d) {...} }` —
                        // encode is a Struct whose `string` Function carries a sub-program
                        // index that must be offset by cv_offset).
                        let mut cloned = v.clone();
                        if cv_offset > 0 {
                            Self::fixup_func_indices(&mut cloned, cv_offset);
                        }
                        vars_scope.insert(k.clone(), cloned);
                    }
                }
                // Add all component methods (public + private) to variables scope
                // These override component_variables entries for public methods.
                // Strip captured_scope — CFC methods use __variables, not closures.
                for (k, v) in s.iter() {
                    if k.starts_with("__") {
                        continue;
                    }
                    if let CfmlValue::Function(ref f) = v {
                        let mut clean = f.clone();
                        clean.captured_scope = None;
                        vars_scope.insert(k.clone(), CfmlValue::Function(clean));
                    }
                }
                // Merge compiler-generated __variables (property defaults) into
                // the runtime vars_scope. Runtime values take priority, but
                // defaults for properties not set during pseudo-constructor are preserved.
                if let Some(CfmlValue::Struct(ref compiled_vars)) = s.get("__variables") {
                    for (k, v) in compiled_vars.iter() {
                        if !vars_scope.contains_key(k) {
                            vars_scope.insert(k.clone(), v.clone());
                        }
                    }
                }
                if !vars_scope.is_empty() {
                    s.insert("__variables".to_string(), CfmlValue::strukt(vars_scope));
                }
            }
            return result;
        }
        None
    }

    /// Resolve the full inheritance chain for a component template.
    /// If the template has an `__extends` key, load the parent, recursively
    /// resolve its inheritance, then merge child on top of parent.
    /// Resolve all required method names from an interface, including inherited ones.
    fn resolve_interface_methods(
        &mut self,
        iface_name: &str,
        locals: &IndexMap<String, CfmlValue>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<String>, CfmlError> {
        let name_lower = iface_name.to_lowercase();
        if visited.contains(&name_lower) {
            return Ok(Vec::new()); // Cycle detected
        }
        visited.insert(name_lower);

        // Look up the interface in globals/locals
        let iface = locals
            .get(iface_name)
            .or_else(|| {
                // Case-insensitive lookup in globals
                self.globals
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == iface_name.to_lowercase())
                    .map(|(_, v)| v)
            })
            .cloned();

        let iface = match iface {
            Some(iface) => iface,
            None => {
                // Try resolving as a component template (file-based)
                match self.resolve_component_template(iface_name, locals) {
                    Some(t) => t,
                    None => {
                        return Err(CfmlError::runtime(format!(
                            "Interface '{}' not found",
                            iface_name
                        )))
                    }
                }
            }
        };

        let iface_struct = match &iface {
            CfmlValue::Struct(s) => s,
            _ => {
                return Err(CfmlError::runtime(format!(
                    "'{}' is not an interface",
                    iface_name
                )))
            }
        };

        // Verify it's actually an interface
        let is_interface = matches!(
            iface_struct.get("__is_interface"),
            Some(CfmlValue::Bool(true))
        );
        if !is_interface {
            return Err(CfmlError::runtime(format!(
                "'{}' is not an interface",
                iface_name
            )));
        }

        let mut methods = Vec::new();

        // Collect methods from __methods struct
        if let Some(CfmlValue::Struct(methods_map)) = iface_struct.get("__methods") {
            for key in methods_map.keys() {
                methods.push(key.clone());
            }
        }

        // Recursively collect from parent interfaces
        if let Some(CfmlValue::Array(parents)) = iface_struct.get("__extends") {
            for parent in parents.iter() {
                let parent_name = parent.as_string();
                let parent_methods =
                    self.resolve_interface_methods(&parent_name, locals, visited)?;
                for m in parent_methods {
                    if !methods
                        .iter()
                        .any(|existing| existing.to_lowercase() == m.to_lowercase())
                    {
                        methods.push(m);
                    }
                }
            }
        }

        Ok(methods)
    }

    /// Collect all transitive interface names from an interface's extends chain.
    fn collect_transitive_interfaces(
        &mut self,
        iface_name: &str,
        locals: &IndexMap<String, CfmlValue>,
        visited: &mut std::collections::HashSet<String>,
        result: &mut Vec<String>,
    ) {
        let name_lower = iface_name.to_lowercase();
        if visited.contains(&name_lower) {
            return;
        }
        visited.insert(name_lower);
        result.push(iface_name.to_string());

        // Look up the interface
        let iface = locals
            .get(iface_name)
            .or_else(|| {
                self.globals
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == iface_name.to_lowercase())
                    .map(|(_, v)| v)
            })
            .cloned();

        let iface = match iface {
            Some(i) => i,
            None => match self.resolve_component_template(iface_name, locals) {
                Some(t) => t,
                None => return,
            },
        };

        if let CfmlValue::Struct(s) = &iface {
            if let Some(CfmlValue::Array(parents)) = s.get("__extends") {
                for parent in parents.clone().iter() {
                    let parent_name = parent.as_string();
                    self.collect_transitive_interfaces(&parent_name, locals, visited, result);
                }
            }
        }
    }

    /// Validate that a component struct implements all methods required by its interfaces.
    /// Returns the full set of transitive interface names (for __implements_chain).
    fn validate_interface_implementation(
        &mut self,
        component: &IndexMap<String, CfmlValue>,
        locals: &IndexMap<String, CfmlValue>,
    ) -> Result<Vec<String>, CfmlError> {
        let iface_names = match component.get("__implements") {
            Some(CfmlValue::Array(arr)) => arr.clone(),
            _ => return Ok(Vec::new()), // No interfaces to validate
        };

        let comp_name = component
            .get("__name")
            .map(|v| v.as_string())
            .unwrap_or_else(|| "Anonymous".to_string());

        let mut all_interfaces = Vec::new();

        for iface_val in iface_names.iter() {
            let iface_name = iface_val.as_string();

            // Collect all transitive interface names
            let mut visited_ifaces = std::collections::HashSet::new();
            self.collect_transitive_interfaces(
                &iface_name,
                locals,
                &mut visited_ifaces,
                &mut all_interfaces,
            );

            // Validate methods
            let mut visited = std::collections::HashSet::new();
            let required_methods =
                self.resolve_interface_methods(&iface_name, locals, &mut visited)?;

            for method_name in &required_methods {
                // Check if component has this method (case-insensitive)
                let has_method = component.iter().any(|(k, v)| {
                    k.to_lowercase() == method_name.to_lowercase()
                        && matches!(v, CfmlValue::Function(_))
                });
                if !has_method {
                    return Err(CfmlError::runtime(format!(
                        "Component '{}' does not implement method '{}' required by interface '{}'",
                        comp_name, method_name, iface_name
                    )));
                }
            }
        }

        Ok(all_interfaces)
    }

    fn resolve_inheritance(
        &mut self,
        template: CfmlValue,
        locals: &IndexMap<String, CfmlValue>,
    ) -> CfmlValue {
        let s = match &template {
            CfmlValue::Struct(s) => s,
            _ => return template,
        };

        // Check for __extends key
        let extends_name = match s.get("__extends") {
            Some(CfmlValue::String(name)) => name.clone(),
            _ => return template, // No extends, return as-is
        };

        // Prevent circular inheritance
        let mut visited = std::collections::HashSet::new();
        if let Some(CfmlValue::String(name)) = s.get("__name") {
            visited.insert(name.to_lowercase());
        }

        self.resolve_inheritance_chain(template, &extends_name, locals, &mut visited)
    }

    fn resolve_inheritance_chain(
        &mut self,
        child: CfmlValue,
        parent_name: &str,
        locals: &IndexMap<String, CfmlValue>,
        visited: &mut std::collections::HashSet<String>,
    ) -> CfmlValue {
        // Check circular
        if visited.contains(&parent_name.to_lowercase()) {
            return child;
        }
        visited.insert(parent_name.to_lowercase());

        // Rust-class parent: no CFML template to merge. Stash the class name
        // for createObject to construct, and record the prefixed name in the
        // extends chain so isInstanceOf("rust:X") works.
        if let Some(rust_class) = parent_name.strip_prefix("rust:") {
            let mut child_map = match child {
                CfmlValue::Struct(s) => s,
                other => return other,
            };
            Arc::make_mut(&mut child_map).insert(
                "__rust_extends".to_string(),
                CfmlValue::String(rust_class.to_string()),
            );
            let mut chain = vec![CfmlValue::String(parent_name.to_string())];
            if let Some(CfmlValue::Array(existing)) = child_map.get("__extends_chain") {
                for item in existing.iter() {
                    chain.push(item.clone());
                }
            }
            Arc::make_mut(&mut child_map)
                .insert("__extends_chain".to_string(), CfmlValue::array(chain));
            return CfmlValue::Struct(child_map);
        }

        // Temporarily set source_file to the child CFC's path so parent
        // resolution finds siblings in the same directory
        let old_source_file = if let CfmlValue::Struct(ref cs) = child {
            if let Some(CfmlValue::String(src)) = cs.get("__source_file") {
                let prev = self.source_file.clone();
                self.source_file = Some(src.clone());
                Some(prev)
            } else {
                None
            }
        } else {
            None
        };

        // Resolve parent template
        let parent = match self.resolve_component_template(parent_name, locals) {
            Some(p) => p,
            None => {
                if let Some(prev) = old_source_file {
                    self.source_file = prev;
                }
                return child; // Parent not found, return child as-is
            }
        };

        // Restore source_file
        if let Some(prev) = old_source_file {
            self.source_file = prev;
        }

        // Recursively resolve parent's inheritance
        let parent = if let CfmlValue::Struct(ref ps) = parent {
            if let Some(CfmlValue::String(grandparent)) = ps.get("__extends") {
                let gp = grandparent.clone();
                self.resolve_inheritance_chain(parent, &gp, locals, visited)
            } else {
                parent
            }
        } else {
            parent
        };

        // Now merge: start with parent, layer child on top
        let child_map = match child {
            CfmlValue::Struct(s) => s,
            _ => return parent,
        };
        let mut parent_map = match parent {
            CfmlValue::Struct(s) => s,
            _ => return CfmlValue::Struct(child_map),
        };

        // Collect parent methods for __super
        let mut super_methods = IndexMap::new();
        for (k, v) in parent_map.iter() {
            if matches!(v, CfmlValue::Function(_)) && !k.starts_with("__") {
                super_methods.insert(k.clone(), v.clone());
            }
        }

        // Merge __variables from parent and child (child overrides parent)
        let parent_vars = parent_map
            .get("__variables")
            .and_then(|v| {
                if let CfmlValue::Struct(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let child_vars = child_map
            .get("__variables")
            .and_then(|v| {
                if let CfmlValue::Struct(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        if !parent_vars.is_empty() || !child_vars.is_empty() {
            let mut merged_vars = parent_vars;
            for (k, v) in child_vars.iter() {
                Arc::make_mut(&mut merged_vars).insert(k.clone(), v.clone());
            }
            Arc::make_mut(&mut parent_map).insert("__variables".to_string(), CfmlValue::Struct(merged_vars));
        }

        // Layer child on top of parent (child overrides parent)
        for (k, v) in child_map.iter() {
            if k == "__extends" || k == "__variables" {
                continue; // Already merged above; don't overwrite
            }
            Arc::make_mut(&mut parent_map).insert(k.clone(), v.clone());
            // Also update __variables when child overrides a method, so
            // unqualified calls within CFC methods resolve to the override
            if matches!(v, CfmlValue::Function(_)) && !k.starts_with("__") {
                if let Some(vars) = Arc::make_mut(&mut parent_map).get_mut("__variables").and_then(|v| v.as_struct_mut()) {
                    vars.insert(k.clone(), v.clone());
                }
            }
        }

        // Add __super struct with marker for dispatch detection
        if !super_methods.is_empty() {
            super_methods.insert("__is_super".to_string(), CfmlValue::Bool(true));
            Arc::make_mut(&mut parent_map).insert("__super".to_string(), CfmlValue::strukt(super_methods));
        }

        // Build __extends_chain for isInstanceOf
        let mut chain = Vec::new();
        chain.push(CfmlValue::String(parent_name.to_string()));
        if let Some(CfmlValue::Array(existing)) = parent_map.get("__extends_chain") {
            for item in existing.iter() {
                chain.push(item.clone());
            }
        }
        Arc::make_mut(&mut parent_map).insert("__extends_chain".to_string(), CfmlValue::array(chain));

        // Propagate __implements through inheritance: aggregate child + parent interfaces
        let mut all_implements = std::collections::HashSet::new();
        // Collect child's direct interfaces
        if let Some(CfmlValue::Array(child_ifaces)) = child_map.get("__implements") {
            for iface in child_ifaces.iter() {
                all_implements.insert(iface.as_string().to_lowercase());
            }
        }
        // Collect parent's interfaces (direct + inherited)
        if let Some(CfmlValue::Array(parent_ifaces)) = parent_map.get("__implements") {
            for iface in parent_ifaces.iter() {
                all_implements.insert(iface.as_string().to_lowercase());
            }
        }
        if let Some(CfmlValue::Array(parent_chain)) = parent_map.get("__implements_chain") {
            for iface in parent_chain.iter() {
                all_implements.insert(iface.as_string().to_lowercase());
            }
        }
        if !all_implements.is_empty() {
            let chain: Vec<CfmlValue> = all_implements
                .into_iter()
                .map(|s| CfmlValue::String(s))
                .collect();
            Arc::make_mut(&mut parent_map).insert("__implements_chain".to_string(), CfmlValue::array(chain));
        }

        CfmlValue::Struct(parent_map)
    }

    /// If `instance` was marked with `__rust_extends` by resolve_inheritance_chain,
    /// look up the native constructor, default-construct it, and stash the
    /// resulting NativeObject under `__super`. Errors if the named class is
    /// not registered. Idempotent: returns early if `__super` is already set.
    fn attach_native_parent(&mut self, instance: CfmlValue) -> CfmlResult {
        let mut s = match instance {
            CfmlValue::Struct(s) => s,
            other => return Ok(other),
        };
        let rust_class = match s.get("__rust_extends") {
            Some(CfmlValue::String(n)) => n.clone(),
            _ => return Ok(CfmlValue::Struct(s)),
        };
        if s.get("__super").is_some() {
            return Ok(CfmlValue::Struct(s));
        }
        let key = rust_class.to_lowercase();
        let ctor = self.native_classes.get(&key).copied().ok_or_else(|| {
            CfmlError::runtime(format!(
                "No native (Rust) class registered with name '{}'",
                rust_class
            ))
        })?;
        let parent = ctor(Vec::new())?;
        Arc::make_mut(&mut s).insert("__super".to_string(), parent);
        Ok(CfmlValue::Struct(s))
    }

    // ---------------------------------------------------------------------------
    // Sandbox mode: intercept file builtins
    // ---------------------------------------------------------------------------

    /// In sandbox mode, intercept file I/O builtins:
    /// - Read operations route through the VFS (embedded archive)
    /// - Write operations are blocked
    /// Returns None if the function is not a file operation (let normal dispatch handle it).
    fn sandbox_intercept(&self, name: &str, args: &[CfmlValue]) -> Option<CfmlResult> {
        let get_str =
            |idx: usize| -> String { args.get(idx).map(|v| v.as_string()).unwrap_or_default() };

        match name {
            // --- Read operations: route through VFS ---
            "fileread" => {
                let path = get_str(0);
                Some(
                    self.vfs
                        .read_to_string(&path)
                        .map(CfmlValue::String)
                        .map_err(|e| CfmlError::runtime(format!("fileRead: {}", e))),
                )
            }
            "filereadbinary" => {
                let path = get_str(0);
                Some(
                    self.vfs
                        .read(&path)
                        .map(CfmlValue::Binary)
                        .map_err(|e| CfmlError::runtime(format!("fileReadBinary: {}", e))),
                )
            }
            "fileexists" => {
                let path = get_str(0);
                Some(Ok(CfmlValue::Bool(self.vfs.exists(&path))))
            }
            "directoryexists" => {
                let path = get_str(0);
                Some(Ok(CfmlValue::Bool(self.vfs.is_dir(&path))))
            }
            "directorylist" => {
                let path = get_str(0);
                let recurse = args.get(1).map(|v| v.is_true()).unwrap_or(false);
                let list_info = args
                    .get(2)
                    .map(|v| v.as_string().to_lowercase())
                    .unwrap_or_else(|| "path".to_string());
                Some(self.sandbox_directory_list(&path, recurse, &list_info))
            }
            "getfileinfo" => {
                let path = get_str(0);
                Some(self.sandbox_get_file_info(&path))
            }
            "getprofilestring" => {
                if args.len() < 3 {
                    return Some(Err(CfmlError::runtime(
                        "getProfileString requires 3 arguments".to_string(),
                    )));
                }
                let path = get_str(0);
                let section = get_str(1);
                let entry = get_str(2);
                Some(self.sandbox_get_profile_string(&path, &section, &entry))
            }
            "getprofilesections" => {
                let path = get_str(0);
                Some(self.sandbox_get_profile_sections(&path))
            }
            "filegetmimetype" => {
                // No FS access needed — just path extension parsing, let builtin handle it
                None
            }
            "filereadline" => {
                // Route through VFS: read file, return Nth line
                if let Some(CfmlValue::Struct(handle)) = args.first() {
                    let path = handle
                        .get("path")
                        .map(|v| v.as_string())
                        .unwrap_or_default();
                    let line_num = handle
                        .get("line")
                        .and_then(|v| match v {
                            CfmlValue::Int(i) => Some(*i as usize),
                            _ => None,
                        })
                        .unwrap_or(0);
                    Some(
                        self.vfs
                            .read_to_string(&path)
                            .map(|content| {
                                let lines: Vec<&str> = content.lines().collect();
                                if line_num < lines.len() {
                                    CfmlValue::String(lines[line_num].to_string())
                                } else {
                                    CfmlValue::String(String::new())
                                }
                            })
                            .map_err(|e| CfmlError::runtime(format!("fileReadLine: {}", e))),
                    )
                } else {
                    Some(Err(CfmlError::runtime(
                        "fileReadLine requires a file handle".to_string(),
                    )))
                }
            }
            "fileiseof" => {
                if let Some(CfmlValue::Struct(handle)) = args.first() {
                    let path = handle
                        .get("path")
                        .map(|v| v.as_string())
                        .unwrap_or_default();
                    let line_num = handle
                        .get("line")
                        .and_then(|v| match v {
                            CfmlValue::Int(i) => Some(*i as usize),
                            _ => None,
                        })
                        .unwrap_or(0);
                    Some(
                        self.vfs
                            .read_to_string(&path)
                            .map(|content| CfmlValue::Bool(line_num >= content.lines().count()))
                            .map_err(|e| CfmlError::runtime(format!("fileIsEOF: {}", e))),
                    )
                } else {
                    Some(Ok(CfmlValue::Bool(true)))
                }
            }
            "fileopen" => {
                // Allow opening for read (returns handle struct), but the path must exist in VFS
                let path = get_str(0);
                if self.vfs.exists(&path) {
                    let mut handle = IndexMap::new();
                    handle.insert("path".to_string(), CfmlValue::String(path));
                    handle.insert("isOpen".to_string(), CfmlValue::Bool(true));
                    handle.insert("line".to_string(), CfmlValue::Int(0));
                    Some(Ok(CfmlValue::strukt(handle)))
                } else {
                    Some(Err(CfmlError::runtime(format!(
                        "fileOpen: file not found in sandbox: {}",
                        path
                    ))))
                }
            }
            "fileclose" => Some(Ok(CfmlValue::Null)),
            "gettempdirectory" => Some(Ok(CfmlValue::String(
                std::env::temp_dir().to_string_lossy().to_string(),
            ))),

            // --- Write operations: blocked ---
            "filewrite"
            | "fileappend"
            | "filedelete"
            | "filemove"
            | "filecopy"
            | "filewriteline"
            | "directorycreate"
            | "directorydelete"
            | "directoryrename"
            | "directorycopy"
            | "setprofilestring"
            | "filesetaccessmode"
            | "filesetattribute"
            | "filesetlastmodified"
            | "gettempfile" => Some(Err(CfmlError::runtime(format!(
                "{}(): filesystem writes are disabled in sandbox mode",
                name
            )))),

            // --- cfdirectory tag: allow list, block create/delete/rename ---
            "cfdirectory" | "__cfdirectory" => {
                if let Some(CfmlValue::Struct(opts)) = args.first() {
                    let action = opts
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == "action")
                        .map(|(_, v)| v.as_string().to_lowercase())
                        .unwrap_or_else(|| "list".to_string());
                    match action.as_str() {
                        "list" => {
                            let dir = opts.iter()
                                .find(|(k, _)| k.to_lowercase() == "directory")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_default();
                            Some(self.sandbox_directory_list(&dir, false, "query"))
                        }
                        _ => Some(Err(CfmlError::runtime(format!(
                            "cfdirectory action='{}': filesystem writes are disabled in sandbox mode", action
                        )))),
                    }
                } else {
                    None
                }
            }

            // Not a file operation — let normal dispatch handle it
            _ => None,
        }
    }

    /// Sandbox directoryList: list entries from the VFS.
    fn sandbox_directory_list(&self, path: &str, recurse: bool, list_info: &str) -> CfmlResult {
        let mut entries = Vec::new();
        self.sandbox_collect_entries(path, recurse, &mut entries)?;

        if list_info == "name" {
            Ok(CfmlValue::array(
                entries
                    .into_iter()
                    .map(|(name, _, _)| CfmlValue::String(name))
                    .collect(),
            ))
        } else if list_info == "query" {
            // Return a query object with name, directory, size, type, dateLastModified columns
            let mut names = Vec::new();
            let mut dirs = Vec::new();
            let mut sizes = Vec::new();
            let mut types = Vec::new();
            let mut dates = Vec::new();
            for (name, full_path, is_dir) in &entries {
                names.push(CfmlValue::String(name.clone()));
                dirs.push(CfmlValue::String(path.to_string()));
                sizes.push(CfmlValue::Int(0));
                types.push(CfmlValue::String(if *is_dir {
                    "Dir".to_string()
                } else {
                    "File".to_string()
                }));
                dates.push(CfmlValue::String(String::new()));
                let _ = full_path; // suppress unused
            }
            let mut columns = IndexMap::new();
            columns.insert("name".to_string(), CfmlValue::array(names));
            columns.insert("directory".to_string(), CfmlValue::array(dirs));
            columns.insert("size".to_string(), CfmlValue::array(sizes));
            columns.insert("type".to_string(), CfmlValue::array(types));
            columns.insert("datelastmodified".to_string(), CfmlValue::array(dates));
            let mut q = IndexMap::new();
            q.insert("__type".to_string(), CfmlValue::String("query".to_string()));
            q.insert("__columns".to_string(), CfmlValue::strukt(columns));
            q.insert(
                "recordcount".to_string(),
                CfmlValue::Int(entries.len() as i64),
            );
            Ok(CfmlValue::strukt(q))
        } else {
            // "path" mode: return array of full paths
            Ok(CfmlValue::array(
                entries
                    .into_iter()
                    .map(|(_, full, _)| CfmlValue::String(full))
                    .collect(),
            ))
        }
    }

    fn sandbox_collect_entries(
        &self,
        path: &str,
        recurse: bool,
        out: &mut Vec<(String, String, bool)>,
    ) -> Result<(), CfmlError> {
        let entries = self
            .vfs
            .read_dir(path)
            .map_err(|e| CfmlError::runtime(format!("directoryList: {}", e)))?;
        for entry in entries {
            let full_path = if path.ends_with('/') {
                format!("{}{}", path, entry.name)
            } else {
                format!("{}/{}", path, entry.name)
            };
            out.push((entry.name.clone(), full_path.clone(), entry.is_dir));
            if recurse && entry.is_dir {
                self.sandbox_collect_entries(&full_path, true, out)?;
            }
        }
        Ok(())
    }

    /// Sandbox getFileInfo: return metadata from VFS.
    fn sandbox_get_file_info(&self, path: &str) -> CfmlResult {
        if !self.vfs.exists(path) {
            return Err(CfmlError::runtime(format!(
                "getFileInfo: file not found: {}",
                path
            )));
        }
        let is_file = self.vfs.is_file(path);
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let size = if is_file {
            self.vfs.read(path).map(|d| d.len() as i64).unwrap_or(0)
        } else {
            0
        };
        let mut info = IndexMap::new();
        info.insert("name".to_string(), CfmlValue::String(name));
        info.insert("path".to_string(), CfmlValue::String(path.to_string()));
        info.insert("size".to_string(), CfmlValue::Int(size));
        info.insert(
            "type".to_string(),
            CfmlValue::String(if is_file { "file" } else { "dir" }.to_string()),
        );
        info.insert("canRead".to_string(), CfmlValue::Bool(true));
        info.insert("canWrite".to_string(), CfmlValue::Bool(false));
        info.insert("isHidden".to_string(), CfmlValue::Bool(false));
        Ok(CfmlValue::strukt(info))
    }

    /// Sandbox getProfileString: read INI from VFS.
    fn sandbox_get_profile_string(&self, path: &str, section: &str, entry: &str) -> CfmlResult {
        let content = self
            .vfs
            .read_to_string(path)
            .map_err(|e| CfmlError::runtime(format!("getProfileString: {}", e)))?;
        // Simple INI parser inline
        let section_lower = section.to_lowercase();
        let entry_lower = entry.to_lowercase();
        let mut in_section = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let name = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
                in_section = name == section_lower;
            } else if in_section && trimmed.contains('=') {
                let (key, val) = trimmed.split_once('=').unwrap();
                if key.trim().to_lowercase() == entry_lower {
                    return Ok(CfmlValue::String(val.trim().to_string()));
                }
            }
        }
        Ok(CfmlValue::String(String::new()))
    }

    /// Sandbox getProfileSections: read INI sections from VFS.
    fn sandbox_get_profile_sections(&self, path: &str) -> CfmlResult {
        let content = self
            .vfs
            .read_to_string(path)
            .map_err(|e| CfmlError::runtime(format!("getProfileSections: {}", e)))?;
        let mut result = IndexMap::new();
        let mut current_section = String::new();
        let mut current_keys: Vec<String> = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                if !current_section.is_empty() {
                    result.insert(
                        current_section.clone(),
                        CfmlValue::String(current_keys.join(",")),
                    );
                }
                current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
                current_keys = Vec::new();
            } else if !current_section.is_empty() && trimmed.contains('=') {
                if let Some((key, _)) = trimmed.split_once('=') {
                    current_keys.push(key.trim().to_string());
                }
            }
        }
        if !current_section.is_empty() {
            result.insert(current_section, CfmlValue::String(current_keys.join(",")));
        }
        Ok(CfmlValue::strukt(result))
    }

    /// Walk up the directory tree from source_file to find Application.cfc.
    /// In production mode, the resolved path (or absence) is memoized by
    /// start directory on `ServerState.app_cfc_path_cache`.
    fn find_application_cfc(&self) -> Option<String> {
        let start_dir = if let Some(ref source) = self.source_file {
            std::path::Path::new(source)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        } else {
            std::env::current_dir().unwrap_or_default()
        };

        // Production-mode cache hit
        if let Some(ref ss) = self.server_state {
            if ss.production_mode {
                if let Some(cached) = ss.app_cfc_path_cache.read().get(&start_dir) {
                    return cached.clone();
                }
            }
        }

        let mut dir = start_dir.as_path();
        let mut found: Option<String> = None;
        loop {
            // Check for Application.cfc (case-insensitive) via VFS
            let dir_str = dir.to_string_lossy().to_string();
            if let Ok(entries) = self.vfs.read_dir(&dir_str) {
                let mut hit = None;
                for entry in &entries {
                    if entry.name.to_lowercase() == "application.cfc" {
                        hit = Some(dir.join(&entry.name).to_string_lossy().to_string());
                        break;
                    }
                }
                if hit.is_some() {
                    found = hit;
                    break;
                }
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }

        if let Some(ref ss) = self.server_state {
            if ss.production_mode {
                ss.app_cfc_path_cache
                    .write()
                    .insert(start_dir, found.clone());
            }
        }

        found
    }

    /// Load and execute Application.cfc, returning the component struct
    fn load_application_cfc(&mut self, path: &str) -> Option<CfmlValue> {
        let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
        let sub_program = compile_file_cached(path, cache, self.vfs.as_ref()).ok()?;

        // Save current program, swap in sub-program
        let old_program = std::mem::replace(&mut self.program, sub_program);
        let main_idx = self
            .program
            .functions
            .iter()
            .position(|f| f.name == "__main__")
            .unwrap_or(0);
        let cfc_func = self.program.functions[main_idx].clone();
        // Mark as __cfc_body__ so the VM treats it as function scope
        // (prevents globals leaking into `variables` via LoadLocal)
        let mut cfc_body = (*cfc_func).clone();
        cfc_body.name = "__cfc_body__".to_string();
        let empty_locals = IndexMap::new();
        let _ = self.execute_function_with_args(&cfc_body, Vec::new(), Some(&empty_locals));

        // Capture component body locals as the variables scope
        let component_variables = self.captured_locals.take().unwrap_or_default();

        // Merge sub-program functions into main program
        let sub_funcs = self.program.functions.clone();
        self.program = old_program;
        let base_idx = self.program.functions.len();
        for func in sub_funcs {
            if func.name != "__main__" {
                self.program.functions.push(Arc::clone(&func));
                self.user_functions
                    .insert(func.name.clone(), Arc::clone(&func));
            }
        }

        // Find the component struct in globals
        let mut template = self
            .globals
            .iter()
            .find(|(k, v)| {
                let k_lower = k.to_lowercase();
                (k_lower == "application" || *k == "Anonymous")
                    && matches!(v, CfmlValue::Struct(_))
                    && if let CfmlValue::Struct(s) = v {
                        s.contains_key("__name")
                            || s.values().any(|v| matches!(v, CfmlValue::Function(_)))
                    } else {
                        false
                    }
            })
            .map(|(_, v)| v.clone())
            .or_else(|| {
                // Look for any struct with component-like structure
                self.globals
                    .iter()
                    .find(|(_, v)| {
                        if let CfmlValue::Struct(s) = v {
                            s.contains_key("__name")
                                || s.values().any(|val| matches!(val, CfmlValue::Function(_)))
                        } else {
                            false
                        }
                    })
                    .map(|(_, v)| v.clone())
            })?;

        // Fix up func_idx in the template's function values (sub-program index → merged index)
        if base_idx > 0 {
            Self::fixup_func_indices(&mut template, base_idx - 1);
        }

        // Store component body variables as __variables on the template
        // This makes variables.framework etc. accessible to component methods
        if !component_variables.is_empty() {
            let mut vars_scope: IndexMap<String, CfmlValue> = IndexMap::new();
            for (k, v) in &component_variables {
                let k_lower = k.to_lowercase();
                // Skip internal/meta keys and functions — keep only data variables
                if k_lower == "this"
                    || k_lower == "arguments"
                    || k.starts_with("__")
                    || matches!(v, CfmlValue::Function(_))
                {
                    continue;
                }
                vars_scope.insert(k.clone(), v.clone());
            }
            if !vars_scope.is_empty() {
                if let Some(s) = template.as_struct_mut() {
                    s.insert("__variables".to_string(), CfmlValue::strukt(vars_scope));
                }
            }
        }

        // Extract and install mappings early so resolve_inheritance can find parent classes
        let (_, _, mut early_mappings, _, _, _, _, _, _) = Self::extract_app_config(&template);
        let app_cfc_dir = std::path::Path::new(path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        for mapping in &mut early_mappings {
            let expanded = if std::path::Path::new(&mapping.path).is_absolute() {
                mapping.path.clone()
            } else {
                let joined = app_cfc_dir
                    .join(&mapping.path)
                    .to_string_lossy()
                    .to_string();
                self.vfs.canonicalize(&joined).unwrap_or(joined)
            };
            mapping.path = expanded;
        }
        if !early_mappings.is_empty() {
            early_mappings.sort_by(|a, b| b.name.len().cmp(&a.name.len()));
            // Add default "/" mapping for this directory
            if !early_mappings.iter().any(|m| m.name == "/") {
                early_mappings.push(CfmlMapping {
                    name: "/".to_string(),
                    path: app_cfc_dir.to_string_lossy().to_string(),
                });
            }
            self.mappings = early_mappings;
        }

        // Resolve inheritance (e.g. extends="taffy.core.api")
        let resolved = self.resolve_inheritance(template, &IndexMap::new());
        Some(resolved)
    }

    /// Extract application config from a component struct.
    /// Returns (app_name, config, mappings, session_management, session_timeout_secs,
    /// custom_tag_paths, local_mode_modern_default, session_storage, app_caches)
    fn extract_app_config(
        template: &CfmlValue,
    ) -> (
        String,
        IndexMap<String, CfmlValue>,
        Vec<CfmlMapping>,
        bool,
        u64,
        Vec<String>,
        Option<bool>,
        Option<String>,
        indexmap::IndexMap<String, cfml_config::CacheCfg>,
    ) {
        let s = match template {
            CfmlValue::Struct(s) => s,
            _ => {
                return (
                    "default".to_string(),
                    IndexMap::new(),
                    Vec::new(),
                    false,
                    1800,
                    Vec::new(),
                    None,
                    None,
                    indexmap::IndexMap::new(),
                )
            }
        };

        // Case-insensitive lookup for this.name
        let app_name = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "name")
            .and_then(|(_, v)| match v {
                CfmlValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "default".to_string());

        let mut config = IndexMap::new();
        for (k, v) in s.iter() {
            if !k.starts_with("__") && !matches!(v, CfmlValue::Function(_)) {
                config.insert(k.to_lowercase(), v.clone());
            }
        }

        // Extract mappings from this.mappings (case-insensitive key lookup)
        let mut mappings = Vec::new();
        if let Some(mappings_val) = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "mappings")
            .map(|(_, v)| v.clone())
        {
            if let CfmlValue::Struct(map_struct) = mappings_val {
                for (key, val) in map_struct.iter() {
                    // Normalize mapping name: ensure leading+trailing "/"
                    let mut name = key.clone();
                    if !name.starts_with('/') {
                        name = format!("/{}", name);
                    }
                    if !name.ends_with('/') {
                        name = format!("{}/", name);
                    }
                    // Extract path: either a String directly or a Struct with a "path" key
                    let path = match val {
                        CfmlValue::String(p) => Some(p.clone()),
                        CfmlValue::Struct(inner) => inner
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "path")
                            .and_then(|(_, v)| match v {
                                CfmlValue::String(p) => Some(p.clone()),
                                _ => None,
                            }),
                        _ => None,
                    };
                    if let Some(path) = path {
                        mappings.push(CfmlMapping { name, path });
                    }
                }
            }
        }

        // Extract session management config
        let session_management = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "sessionmanagement")
            .map(|(_, v)| match v {
                CfmlValue::Bool(b) => *b,
                CfmlValue::String(s) => s.to_lowercase() == "true" || s.to_lowercase() == "yes",
                _ => false,
            })
            .unwrap_or(false);

        let session_timeout = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "sessiontimeout")
            .and_then(|(_, v)| match v {
                CfmlValue::Int(i) => Some(*i as u64),
                CfmlValue::Double(d) => Some(*d as u64),
                CfmlValue::String(s) => s.parse::<u64>().ok(),
                // createTimeSpan returns a double representing days
                _ => None,
            })
            .unwrap_or(1800); // Default 30 minutes

        // Extract customTagPaths from this.customTagPaths (case-insensitive)
        let mut custom_tag_paths = Vec::new();
        if let Some(ctp_val) = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "customtagpaths")
            .map(|(_, v)| v.clone())
        {
            match ctp_val {
                CfmlValue::Array(arr) => {
                    for item in arr.iter() {
                        custom_tag_paths.push(item.as_string());
                    }
                }
                CfmlValue::String(s) => {
                    for part in s.split(',') {
                        let p = part.trim();
                        if !p.is_empty() {
                            custom_tag_paths.push(p.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // Extract this.localMode (Lucee compatibility — modern vs classic
        // function-local scope semantics). Accepts the same aliases as the
        // function-attribute helper.
        let local_mode_modern_default = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("localmode"))
            .and_then(|(_, v)| match v {
                CfmlValue::String(sv) => match sv.trim().to_ascii_lowercase().as_str() {
                    "modern" | "always" | "true" => Some(true),
                    "classic" | "update" | "false" => Some(false),
                    _ => None,
                },
                CfmlValue::Bool(b) => Some(*b),
                _ => None,
            });

        // Extract this.sessionStorage — name of the named cache to use for sessions.
        let session_storage = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("sessionstorage"))
            .and_then(|(_, v)| match v {
                CfmlValue::String(st) if !st.is_empty() => Some(st.clone()),
                _ => None,
            });

        // Extract this.cache — named cache definitions (Lucee-compatible struct of structs).
        let mut app_caches = indexmap::IndexMap::new();
        if let Some(cache_val) = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cache"))
            .map(|(_, v)| v)
        {
            if let CfmlValue::Struct(cache_map) = cache_val {
                for (cache_name, cache_def) in cache_map.iter() {
                    if let CfmlValue::Struct(def) = cache_def {
                        let provider = def
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("provider"))
                            .and_then(|(_, v)| match v {
                                CfmlValue::String(s) => Some(s.clone()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let mut props = cfml_config::schema::CacheProperties::default();
                        if let Some(CfmlValue::Struct(p)) = def
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("properties"))
                            .map(|(_, v)| v)
                        {
                            for (pk, pv) in p.iter() {
                                match pk.to_lowercase().as_str() {
                                    "servers" => {
                                        if let CfmlValue::Array(arr) = pv {
                                            props.servers =
                                                arr.iter().map(|v| v.as_string()).collect();
                                        }
                                    }
                                    "keyprefix" => props.key_prefix = pv.as_string(),
                                    "listenaddr" => props.listen_addr = pv.as_string(),
                                    "advertiseaddr" => props.advertise_addr = pv.as_string(),
                                    "seeds" => {
                                        if let CfmlValue::Array(arr) = pv {
                                            props.seeds =
                                                arr.iter().map(|v| v.as_string()).collect();
                                        }
                                    }
                                    "nodename" => props.node_name = pv.as_string(),
                                    "maxobjects" => {
                                        if let CfmlValue::Int(i) = pv {
                                            props.max_objects = *i as u64;
                                        }
                                    }
                                    "defaulttimeout" => {
                                        if let CfmlValue::Int(i) = pv {
                                            props.default_timeout = *i as u64;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        app_caches.insert(
                            cache_name.to_lowercase(),
                            cfml_config::CacheCfg {
                                provider,
                                properties: props,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }

        (
            app_name,
            config,
            mappings,
            session_management,
            session_timeout,
            custom_tag_paths,
            local_mode_modern_default,
            session_storage,
            app_caches,
        )
    }

    /// Call a lifecycle method on the Application.cfc template
    fn call_lifecycle_method(
        &mut self,
        template: &mut CfmlValue,
        method: &str,
        args: Vec<CfmlValue>,
    ) -> Result<bool, CfmlError> {
        let s = match template {
            CfmlValue::Struct(ref s) => s.clone(),
            _ => return Ok(false),
        };

        // Case-insensitive lookup for the method
        let method_lower = method.to_lowercase();
        let func_val = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == method_lower)
            .map(|(_, v)| v.clone());

        match func_val {
            Some(ref func @ CfmlValue::Function(_)) => {
                // Bind `this` and __variables as a single struct (not expanded)
                let mut parent_locals = IndexMap::new();
                if let Some(vars) = s
                    .iter()
                    .find(|(k, _)| *k == "__variables")
                    .map(|(_, v)| v.clone())
                {
                    parent_locals.insert("__variables".to_string(), vars);
                }
                parent_locals.insert("this".to_string(), template.clone());
                let result = self.call_function(func, args, &parent_locals);

                // Propagate variables scope mutations back into __variables
                if let Some(vars_wb) = self.method_variables_writeback.take() {
                    if let Some(ts) = template.as_struct_mut() {
                        let vars = ts
                            .entry("__variables".to_string())
                            .or_insert_with(|| CfmlValue::strukt(IndexMap::new()));
                        if let Some(vs) = vars.as_struct_mut() {
                            for (k, v) in vars_wb {
                                vs.insert(k, v);
                            }
                        }
                    }
                }

                // Propagate this modifications back into template
                if let Some(modified_this) = self.method_this_writeback.take() {
                    if let Some(ts) = template.as_struct_mut() {
                        if let CfmlValue::Struct(ref modified_s) = modified_this {
                            for (k, v) in modified_s.iter() {
                                if k != "__variables" && k != "__extends" {
                                    ts.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                }

                match result {
                    Ok(_) => Ok(true),
                    Err(e) => Err(e),
                }
            }
            _ => Ok(false),
        }
    }

    /// Return the page path exposed to Application.cfc lifecycle methods.
    ///
    /// The VM keeps `source_file` as the physical path for include and
    /// component resolution, but CFML engines pass web-root-relative paths
    /// such as `/_moopa.cfm` to onRequestStart/onRequest/onRequestEnd.
    fn lifecycle_target_page(&self) -> String {
        let source = self.source_file.clone().unwrap_or_default();
        let canonical_source = self.vfs.canonicalize(&source).unwrap_or(source.clone());
        let source_path = std::path::Path::new(&canonical_source);

        if let Some(ref server_state) = self.server_state {
            if let Some(ref webroot) = server_state.webroot {
                if let Ok(relative) = source_path.strip_prefix(webroot) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    return format!("/{}", relative.trim_start_matches('/'));
                }
            }
        }

        source
    }

    /// Execute with Application.cfc lifecycle
    pub fn execute_with_lifecycle(&mut self) -> CfmlResult {
        // 1. Find Application.cfc
        let app_cfc_path = self.find_application_cfc();

        let app_cfc_path = match app_cfc_path {
            Some(path) => path,
            None => return self.execute(), // No Application.cfc, just execute directly
        };

        // 2. Load Application.cfc
        let mut template = match self.load_application_cfc(&app_cfc_path) {
            Some(t) => t,
            None => return self.execute(), // Failed to load, fall through
        };

        // 3. Extract config and mappings
        let (
            app_name,
            config,
            mut mappings,
            session_management,
            session_timeout,
            mut custom_tag_paths,
            local_mode_modern_default,
            app_session_storage,
            app_caches,
        ) = Self::extract_app_config(&template);

        // `this.lazySessionCreation = true` (alias `this.lazySessions`):
        // Preside-style deferred session creation. When set, no session
        // record is inserted at request start; instead a record + cookie
        // + onSessionStart fire on the first write to `session`.
        let lazy_session_creation = config
            .get("lazysessioncreation")
            .or_else(|| config.get("lazysessions"))
            .map(|v| match v {
                CfmlValue::Bool(b) => *b,
                CfmlValue::String(s) => {
                    let l = s.trim().to_ascii_lowercase();
                    l == "true" || l == "yes" || l == "1"
                }
                _ => false,
            })
            .unwrap_or(false);
        self.lazy_session_creation = lazy_session_creation;

        // 3.0 Layer .cfconfig.json global mappings + customTagPaths underneath
        // the per-application ones, so Application.cfc wins on any conflict.
        // Mapping keys are normalised the same way extract_app_config does.
        if let Some(ref ss) = self.server_state {
            let cfg = ss.cfconfig.clone();
            let app_keys: Vec<String> =
                mappings.iter().map(|m| m.name.to_lowercase()).collect();
            for (raw_name, raw_path) in cfg.mappings.iter() {
                let mut name = raw_name.clone();
                if !name.starts_with('/') {
                    name = format!("/{}", name);
                }
                if !name.ends_with('/') {
                    name = format!("{}/", name);
                }
                if app_keys.contains(&name.to_lowercase()) {
                    continue;
                }
                mappings.push(CfmlMapping {
                    name,
                    path: raw_path.clone(),
                });
            }
            // customTagPaths: cfconfig paths appended after Application.cfc's
            // (Application.cfc searched first).
            for p in cfg.custom_tag_paths.iter() {
                if !custom_tag_paths.iter().any(|existing| existing == p) {
                    custom_tag_paths.push(p.clone());
                }
            }
        }

        // 3a. Apply this.localMode as the request-level default. Functions
        // without an explicit `localMode` attribute inherit this at runtime.
        if let Some(modern) = local_mode_modern_default {
            self.app_local_mode_modern = modern;
        }

        // 3b. Expand mapping paths relative to Application.cfc directory
        let app_cfc_dir = std::path::Path::new(&app_cfc_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        for mapping in &mut mappings {
            let expanded = if std::path::Path::new(&mapping.path).is_absolute() {
                mapping.path.clone()
            } else {
                let joined = app_cfc_dir
                    .join(&mapping.path)
                    .to_string_lossy()
                    .to_string();
                self.vfs.canonicalize(&joined).unwrap_or(joined)
            };
            mapping.path = expanded;
        }
        // Sort by name length descending (longest prefix first)
        mappings.sort_by(|a, b| b.name.len().cmp(&a.name.len()));
        // Add default "/" mapping if not already present
        if !mappings.iter().any(|m| m.name == "/") {
            let root_dir = if let Some(ref source) = self.source_file {
                std::path::Path::new(source)
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_string_lossy()
                    .to_string()
            } else {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            };
            mappings.push(CfmlMapping {
                name: "/".to_string(),
                path: root_dir,
            });
        }
        self.mappings = mappings;

        // 3c. Expand customTagPaths relative to Application.cfc directory
        let vfs = &self.vfs;
        self.custom_tag_paths = custom_tag_paths
            .into_iter()
            .map(|p| {
                if std::path::Path::new(&p).is_absolute() {
                    p
                } else {
                    let joined = app_cfc_dir.join(&p).to_string_lossy().to_string();
                    vfs.canonicalize(&joined).unwrap_or(joined)
                }
            })
            .collect();

        // 4. Wire up application scope
        if let Some(ref server_state) = self.server_state.clone() {
            if !server_state.applications.contains(&app_name) {
                // New application
                let app_state = ApplicationState {
                    name: app_name.clone(),
                    variables: IndexMap::new(),
                    started: false,
                    config: config.clone(),
                    cached_functions: Vec::new(),
                    cached_functions_original_offset: 0,
                    session_storage: app_session_storage.clone(),
                    app_caches: app_caches.clone(),
                };
                server_state.applications.insert(&app_name, app_state);
            }
            let app_snapshot = server_state.applications.get(&app_name).unwrap();
            let scope = Arc::new(Mutex::new(app_snapshot.variables.clone()));
            self.application_scope = Some(scope.clone());

            // 5. onApplicationStart (if not yet started)
            let already_started = app_snapshot.started;
            if !already_started {
                // Flip `started` first so concurrent requests don't re-fire
                // onApplicationStart. Release any internal lock the store
                // holds before calling the lifecycle method, since it may
                // recursively touch the store.
                server_state
                    .applications
                    .modify(&app_name, &mut |app| {
                        app.started = true;
                    });

                // Record how many functions exist before onApplicationStart.
                // Everything beyond this index was added by onApplicationStart
                // (e.g. factory components, resource CFCs).
                let funcs_before = self.program.functions.len();

                match self.call_lifecycle_method(&mut template, "onApplicationStart", vec![]) {
                    Ok(_) => {
                        // Cache only the functions ADDED during onApplicationStart.
                        let funcs_after = self.program.functions.len();
                        if funcs_after > funcs_before {
                            if let Some(ref server_state) = self.server_state.clone() {
                                let new_funcs: Vec<_> =
                                    self.program.functions[funcs_before..].to_vec();
                                server_state.applications.modify(
                                    &app_name,
                                    &mut |app| {
                                        app.cached_functions = new_funcs.clone();
                                        app.cached_functions_original_offset = funcs_before;
                                    },
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let _ = self.call_lifecycle_method(
                            &mut template,
                            "onError",
                            vec![
                                CfmlValue::String(e.message.clone()),
                                CfmlValue::String("onApplicationStart".to_string()),
                            ],
                        );
                        return Err(e);
                    }
                }
            } else {
                // Restore cached functions from onApplicationStart (and any
                // subsequent requests that grew the program — see the request-end
                // refresh below). Append them to the current program (which
                // already has the page's functions + Application.cfc functions
                // from load_application_cfc).
                let cached = app_snapshot.cached_functions.clone();
                let original_offset = app_snapshot.cached_functions_original_offset;
                if !cached.is_empty() {
                    let new_offset = self.program.functions.len();
                    self.program.functions.extend(cached);

                    // If functions ended up at a different offset than originally,
                    // fix up function indices in the application scope values.
                    if new_offset != original_offset {
                        let index_delta = new_offset as i64 - original_offset as i64;
                        if let Some(ref app_scope) = self.application_scope {
                            if let Ok(mut scope) = app_scope.lock() {
                                let keys: Vec<String> = scope.keys().cloned().collect();
                                for key in keys {
                                    if let Some(val) = scope.get_mut(&key) {
                                        Self::adjust_func_indices(
                                            val,
                                            original_offset,
                                            index_delta,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // CLI mode: fresh application scope each time
            let scope = Arc::new(Mutex::new(IndexMap::new()));
            self.application_scope = Some(scope);

            // Still call onApplicationStart in CLI mode
            let _ = self.call_lifecycle_method(&mut template, "onApplicationStart", vec![]);
        }

        // 5b. Session lifecycle
        //
        // Default (eager) mode: a missing record is created up-front and
        // `onSessionStart` fires immediately. Matches Lucee/ACF.
        //
        // Lazy mode (`this.lazySessionCreation = true`): a missing
        // record is left missing. The next session-scope write triggers
        // [`lazy_init_session_if_pending`] which creates the record and
        // fires `onSessionStart` synchronously, so a CFML page that
        // never touches `session` produces no record and no cookie.
        self.session_record_created = false;
        self.session_lazy_pending = false;
        // Stash the loaded Application.cfc so
        // `lazy_init_session_if_pending` can fire `onSessionStart`
        // synchronously from inside a session-write bytecode op.
        self.app_cfc_template = Some(template.clone());
        if session_management {
            if let Some(ref server_state) = self.server_state.clone() {
                let sid = self.session_id.clone().unwrap_or_default();
                let has_sid = !sid.is_empty();
                let record_exists = has_sid && server_state.sessions.contains(&sid);

                if record_exists {
                    // Existing session: bump last_accessed, no lifecycle fire.
                    if let Some(mut session) = server_state.sessions.get(&sid) {
                        session.last_accessed_secs = now_epoch_secs();
                        session.timeout_secs = session_timeout;
                        server_state.sessions.set(&sid, session);
                    }
                } else if lazy_session_creation {
                    // Lazy: defer record creation + onSessionStart until
                    // the first write to session scope. Keep any cookie
                    // sid intact so a re-used cookie value sticks.
                    self.session_lazy_pending = true;
                } else {
                    // Eager: create the record and fire onSessionStart now.
                    // Mint a fresh id if the embedder didn't supply one
                    // (e.g. a request with no CFID cookie). The embedder
                    // reads `vm.session_id` back to emit Set-Cookie.
                    let sid = if has_sid {
                        sid
                    } else {
                        let new_sid = uuid::Uuid::new_v4().to_string();
                        self.session_id = Some(new_sid.clone());
                        new_sid
                    };
                    let now = now_epoch_secs();
                    server_state.sessions.set(
                        &sid,
                        SessionData {
                            variables: IndexMap::new(),
                            created_secs: now,
                            last_accessed_secs: now,
                            auth_user: None,
                            auth_roles: Vec::new(),
                            timeout_secs: session_timeout,
                        },
                    );
                    self.session_record_created = true;
                    let _ = self.call_lifecycle_method(&mut template, "onSessionStart", vec![]);
                }
            }
        }

        // 6. onRequestStart
        let target_page = self.lifecycle_target_page();
        match self.call_lifecycle_method(
            &mut template,
            "onRequestStart",
            vec![CfmlValue::String(target_page.clone())],
        ) {
            Err(e) if e.message == "__cfabort" || e.message == "__cflocation_redirect" => {
                return Ok(CfmlValue::Null);
            }
            _ => {}
        }

        // 7. Check for onRequest — if exists, call it; else execute normally
        let has_on_request = if let CfmlValue::Struct(ref s) = template {
            s.iter().any(|(k, v)| {
                k.to_lowercase() == "onrequest" && matches!(v, CfmlValue::Function(_))
            })
        } else {
            false
        };

        let result = if has_on_request {
            match self.call_lifecycle_method(
                &mut template,
                "onRequest",
                vec![CfmlValue::String(target_page.clone())],
            ) {
                Ok(_) => Ok(CfmlValue::Null),
                Err(e) if e.message == "__cflocation_redirect" || e.message == "__cfabort" => {
                    Ok(CfmlValue::Null)
                }
                Err(e) => Err(e),
            }
        } else {
            match self.execute() {
                Ok(v) => Ok(v),
                Err(e) if e.message == "__cflocation_redirect" || e.message == "__cfabort" => {
                    Ok(CfmlValue::Null)
                }
                Err(e) => Err(e),
            }
        };

        // 8. onRequestEnd
        let _ = self.call_lifecycle_method(
            &mut template,
            "onRequestEnd",
            vec![CfmlValue::String(target_page)],
        );

        // 8b. Session expiry — scan and expire timed-out sessions
        if session_management {
            if let Some(ref server_state) = self.server_state.clone() {
                let expired = server_state.sessions.take_expired(now_epoch_secs());
                if !expired.is_empty() {
                    let app_scope_val = self
                        .application_scope
                        .as_ref()
                        .and_then(|a| a.lock().ok().map(|s| CfmlValue::strukt(s.clone())))
                        .unwrap_or(CfmlValue::strukt(IndexMap::new()));
                    for (_, session_vars) in &expired {
                        // Call onSessionEnd(sessionScope, applicationScope)
                        let _ = self.call_lifecycle_method(
                            &mut template,
                            "onSessionEnd",
                            vec![
                                CfmlValue::strukt(session_vars.clone()),
                                app_scope_val.clone(),
                            ],
                        );
                    }
                }
            }
        }

        // 9. Write application scope back to ServerState. Also refresh
        // cached_functions to include any functions registered DURING the request
        // (e.g. Taffy's `?reload=true` triggers `setupFramework` in
        // `onRequestStart`, which instantiates resource CFCs and appends factory
        // beans to `self.program` AFTER `onApplicationStart` already returned).
        // Without this refresh, later requests restore a stale cached_functions
        // and any function values that the application scope captured during the
        // request (e.g. `application._taffy.factory.getBean`) end up with body
        // indices beyond the restored program length.
        if let Some(ref server_state) = self.server_state.clone() {
            if let Some(ref app_scope) = self.application_scope {
                if let Ok(scope) = app_scope.lock() {
                    let scope_snapshot = scope.clone();
                    let program_functions = self.program.functions.clone();
                    server_state.applications.modify(&app_name, &mut |app| {
                        app.variables = scope_snapshot.clone();
                        // Refresh cache from current program. Use the
                        // original offset captured the first time
                        // onApplicationStart ran; that anchor stays
                        // constant for the lifetime of the application.
                        let offset = app.cached_functions_original_offset;
                        if offset > 0 && program_functions.len() > offset {
                            app.cached_functions = program_functions[offset..].to_vec();
                        }
                    });
                }
            }
        }

        // 10. Clear request scope + stashed Application.cfc template.
        self.request_scope.clear();
        self.app_cfc_template = None;
        self.session_lazy_pending = false;

        result
    }

    pub fn get_output(&self) -> String {
        self.output_buffer.clone()
    }
}

// ---- Helper functions ----

/// Simple wildcard matching: '*' matches any sequence of characters.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (plen, tlen) = (p.len(), t.len());
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0);

    while ti < tlen {
        if pi < plen && (p[pi] == t[ti] || p[pi] == '?') {
            pi += 1;
            ti += 1;
        } else if pi < plen && p[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < plen && p[pi] == '*' {
        pi += 1;
    }
    pi == plen
}

fn binary_op<F>(stack: &mut Vec<CfmlValue>, op: F)
where
    F: FnOnce(CfmlValue, CfmlValue) -> CfmlValue,
{
    if let (Some(b), Some(a)) = (stack.pop(), stack.pop()) {
        stack.push(op(a, b));
    }
}

fn compare_op<F>(stack: &mut Vec<CfmlValue>, op: F)
where
    F: FnOnce(&CfmlValue, &CfmlValue) -> bool,
{
    if let (Some(b), Some(a)) = (stack.pop(), stack.pop()) {
        stack.push(CfmlValue::Bool(op(&a, &b)));
    }
}

fn to_number(val: &CfmlValue) -> Option<f64> {
    match val {
        CfmlValue::Int(i) => Some(*i as f64),
        CfmlValue::Double(d) => Some(*d),
        CfmlValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        CfmlValue::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn numeric_op<F>(a: &CfmlValue, b: &CfmlValue, op: F) -> CfmlValue
where
    F: FnOnce(f64, f64) -> f64,
{
    match (a, b) {
        (CfmlValue::Int(i), CfmlValue::Int(j)) => {
            // Try integer arithmetic first
            let fi = *i as f64;
            let fj = *j as f64;
            let result = op(fi, fj);
            if result == (result as i64 as f64) && result.abs() < i64::MAX as f64 {
                CfmlValue::Int(result as i64)
            } else {
                CfmlValue::Double(result)
            }
        }
        _ => {
            let x = to_number(a).unwrap_or(0.0);
            let y = to_number(b).unwrap_or(0.0);
            CfmlValue::Double(op(x, y))
        }
    }
}

/// CFML equality comparison (case-insensitive for strings, type-coercing for numbers)
fn cfml_equal(a: &CfmlValue, b: &CfmlValue) -> bool {
    match (a, b) {
        (CfmlValue::Null, CfmlValue::Null) => true,
        (CfmlValue::Null, _) | (_, CfmlValue::Null) => false,
        // NativeObjects compare by identity. Two CfmlValue references that
        // hold the same Arc are equal; freshly-constructed instances are not,
        // even if their internal state matches. This matches how `==` works
        // for other object-shaped values in the language (CFCs, java shims).
        (CfmlValue::NativeObject(a), CfmlValue::NativeObject(b)) => Arc::ptr_eq(a, b),
        (CfmlValue::Bool(x), CfmlValue::Bool(y)) => x == y,
        // Bool-number coercion: true==1, false==0
        (CfmlValue::Bool(b), CfmlValue::Int(i)) | (CfmlValue::Int(i), CfmlValue::Bool(b)) => {
            *i == if *b { 1 } else { 0 }
        }
        (CfmlValue::Bool(b), CfmlValue::Double(d)) | (CfmlValue::Double(d), CfmlValue::Bool(b)) => {
            *d == if *b { 1.0 } else { 0.0 }
        }
        (CfmlValue::Int(x), CfmlValue::Int(y)) => x == y,
        (CfmlValue::Double(x), CfmlValue::Double(y)) => x == y,
        (CfmlValue::Int(x), CfmlValue::Double(y)) => (*x as f64) == *y,
        (CfmlValue::Double(x), CfmlValue::Int(y)) => *x == (*y as f64),
        (CfmlValue::String(x), CfmlValue::String(y)) => x.eq_ignore_ascii_case(y),
        // String-number comparison: try to coerce
        (CfmlValue::String(s), CfmlValue::Int(i)) | (CfmlValue::Int(i), CfmlValue::String(s)) => {
            s.trim().parse::<i64>().map_or(false, |n| n == *i)
        }
        (CfmlValue::String(s), CfmlValue::Double(d))
        | (CfmlValue::Double(d), CfmlValue::String(s)) => {
            s.trim().parse::<f64>().map_or(false, |n| n == *d)
        }
        (CfmlValue::String(s), CfmlValue::Bool(b)) | (CfmlValue::Bool(b), CfmlValue::String(s)) => {
            // Empty string is NOT a boolean (isBoolean("") is false), so comparison fails.
            // Matches Lucee/ACF: "" == false returns false.
            match s.to_lowercase().trim() {
                "true" | "yes" => *b,
                "false" | "no" => !*b,
                _ => {
                    // Numeric string: non-zero is true, zero is false
                    if let Ok(n) = s.trim().parse::<f64>() {
                        (n != 0.0) == *b
                    } else {
                        false
                    }
                }
            }
        }
        _ => false,
    }
}

/// CFML comparison ordering
fn cfml_compare(a: &CfmlValue, b: &CfmlValue) -> i32 {
    match (a, b) {
        (CfmlValue::Int(x), CfmlValue::Int(y)) => x.cmp(y) as i32,
        (CfmlValue::Double(x), CfmlValue::Double(y)) => x.partial_cmp(y).map_or(0, |o| o as i32),
        (CfmlValue::Int(x), CfmlValue::Double(y)) => {
            (*x as f64).partial_cmp(y).map_or(0, |o| o as i32)
        }
        (CfmlValue::Double(x), CfmlValue::Int(y)) => {
            x.partial_cmp(&(*y as f64)).map_or(0, |o| o as i32)
        }
        (CfmlValue::String(x), CfmlValue::String(y)) => {
            // Try numeric comparison first
            if let (Ok(a), Ok(b)) = (x.parse::<f64>(), y.parse::<f64>()) {
                return a.partial_cmp(&b).map_or(0, |o| o as i32);
            }
            x.to_lowercase().cmp(&y.to_lowercase()) as i32
        }
        _ => {
            let x = to_number(a).unwrap_or(0.0);
            let y = to_number(b).unwrap_or(0.0);
            x.partial_cmp(&y).map_or(0, |o| o as i32)
        }
    }
}

// ---- Pass-by-reference: backward bytecode scan to identify argument sources ----

/// Returns (pushes, pops) for a given bytecode op — how many values it pushes/pops on the stack.
fn stack_effect(op: &BytecodeOp) -> (usize, usize) {
    match op {
        // Literals: push 1, pop 0
        BytecodeOp::Null
        | BytecodeOp::True
        | BytecodeOp::False
        | BytecodeOp::Integer(_)
        | BytecodeOp::Double(_)
        | BytecodeOp::String(_) => (1, 0),
        // Variable loads: push 1, pop 0
        BytecodeOp::LoadLocal(_) | BytecodeOp::LoadGlobal(_) | BytecodeOp::TryLoadLocal(_) => {
            (1, 0)
        }
        // Variable stores: push 0, pop 1
        BytecodeOp::StoreLocal(_) | BytecodeOp::StoreGlobal(_) => (0, 1),
        // Stack ops
        BytecodeOp::Pop => (0, 1),
        BytecodeOp::Dup => (1, 0),  // net +1 (peeks and pushes copy)
        BytecodeOp::Swap => (2, 2), // pops 2, pushes 2
        // Binary ops: push 1, pop 2
        BytecodeOp::Add
        | BytecodeOp::Sub
        | BytecodeOp::Mul
        | BytecodeOp::Div
        | BytecodeOp::Mod
        | BytecodeOp::Pow
        | BytecodeOp::IntDiv
        | BytecodeOp::Concat
        | BytecodeOp::Eq
        | BytecodeOp::Neq
        | BytecodeOp::Lt
        | BytecodeOp::Lte
        | BytecodeOp::Gt
        | BytecodeOp::Gte
        | BytecodeOp::Contains
        | BytecodeOp::DoesNotContain
        | BytecodeOp::And
        | BytecodeOp::Or
        | BytecodeOp::Xor
        | BytecodeOp::Eqv
        | BytecodeOp::Imp => (1, 2),
        // Unary ops: push 1, pop 1
        BytecodeOp::Negate | BytecodeOp::Not => (1, 1),
        // Control flow
        BytecodeOp::Jump(_) => (0, 0),
        BytecodeOp::JumpIfFalse(_) | BytecodeOp::JumpIfTrue(_) => (0, 1),
        BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, _) => (0, 0),
        BytecodeOp::ForLoopStep(_, _, _, _, _) => (0, 0),
        BytecodeOp::Return => (0, 1),
        // Call: pops func + N args, pushes 1 result
        BytecodeOp::Call(n) => (1, n + 1),
        BytecodeOp::CallNamed(_, n) => (1, n + 1),
        BytecodeOp::CallSpread => (1, 3), // func, array, count — approximate
        // Collections
        BytecodeOp::BuildArray(n) => (1, *n),
        BytecodeOp::BuildStruct(n) => (1, n * 2),
        BytecodeOp::GetIndex => (1, 2),       // obj + key → value
        BytecodeOp::SetIndex => (0, 3),       // obj + key + value → (modifies in place)
        BytecodeOp::GetProperty(_) => (1, 1), // obj → value
        BytecodeOp::LoadLocalProperty(_, _) => (1, 0), // pushes value, reads nothing
        BytecodeOp::StoreLocalProperty(_, _) => (0, 1), // pops 1 (value), pushes 0
        BytecodeOp::SetProperty(_) => (0, 2), // obj + value → (modifies)
        BytecodeOp::GetKeys => (1, 1),
        BytecodeOp::ConcatArrays | BytecodeOp::MergeStructs => (1, 2),
        // Object
        BytecodeOp::NewObject(n) => (1, n + 1), // class + args → instance
        // Function definition: push 1
        BytecodeOp::DefineFunction(_) => (1, 0),
        // Postfix: push 1 (new value)
        BytecodeOp::Increment(_) | BytecodeOp::Decrement(_) => (1, 0),
        BytecodeOp::AddLocalConst(_, _) | BytecodeOp::MulLocalConst(_, _) => (0, 0), // pure local mutation, no stack traffic
        // Exception handling
        BytecodeOp::TryStart(_) | BytecodeOp::TryEnd => (0, 0),
        BytecodeOp::Throw | BytecodeOp::Rethrow => (0, 1),
        // Method call: pops obj + args, pushes 1
        BytecodeOp::CallMethod(_, n, _) => (1, n + 1),
        BytecodeOp::CallRustSuperCtor(n) => (1, *n),
        // Include
        BytecodeOp::Include(_) => (0, 0),
        BytecodeOp::IncludeDynamic => (0, 1),
        // Null
        BytecodeOp::IsNull => (1, 1),
        BytecodeOp::JumpIfNotNull(_) => (1, 1), // pops, pushes back if not null
        // Output
        BytecodeOp::Print => (0, 1),
        BytecodeOp::Halt => (0, 0),
        // Misc
        BytecodeOp::IsDefined(_) => (1, 0),
        BytecodeOp::DeclareLocal(_) => (0, 0),
        BytecodeOp::LineInfo(_, _) => (0, 0),
    }
}

/// Scan backward through bytecode from a Call site to find which local variables
/// were passed as arguments. Returns a Vec of Option<String> where Some(name) means
/// the arg at that position came directly from LoadLocal(name).
fn find_arg_sources(ops: &[BytecodeOp], call_ip: usize, arg_count: usize) -> Vec<Option<String>> {
    let mut sources: Vec<Option<String>> = vec![None; arg_count];
    if arg_count == 0 || call_ip == 0 {
        return sources;
    }

    let mut pos = call_ip;
    let mut depth: i32 = 0; // extra values above our args that need accounting
    let mut arg_idx = arg_count; // next arg slot to fill (going last→first)

    while pos > 0 && arg_idx > 0 {
        pos -= 1;
        let op = &ops[pos];
        let (pushes, pops) = stack_effect(op);

        // This op's pushes: first fill internal dependencies, then fill arg slots
        for _ in 0..pushes {
            if depth > 0 {
                depth -= 1;
            } else if arg_idx > 0 {
                arg_idx -= 1;
                if let BytecodeOp::LoadLocal(name)
                | BytecodeOp::TryLoadLocal(name)
                | BytecodeOp::LoadGlobal(name) = op
                {
                    sources[arg_idx] = Some(name.clone());
                }
            }
        }
        // This op's pops create internal dependencies
        depth += pops as i32;
    }
    sources
}

// ---- precisionEvaluate: recursive-descent parser operating on rust_decimal::Decimal ----

fn precision_evaluate_expr(expr: &str) -> Result<String, CfmlError> {
    use rust_decimal::Decimal;
    use std::str::FromStr;

    struct PrecParser<'a> {
        chars: &'a [u8],
        pos: usize,
    }

    impl<'a> PrecParser<'a> {
        fn new(input: &'a str) -> Self {
            Self {
                chars: input.as_bytes(),
                pos: 0,
            }
        }

        fn skip_ws(&mut self) {
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
        }

        fn parse_expr(&mut self) -> Result<Decimal, CfmlError> {
            self.parse_add_sub()
        }

        fn parse_add_sub(&mut self) -> Result<Decimal, CfmlError> {
            let mut left = self.parse_mul_div()?;
            loop {
                self.skip_ws();
                if self.pos >= self.chars.len() {
                    break;
                }
                match self.chars[self.pos] {
                    b'+' => {
                        self.pos += 1;
                        let right = self.parse_mul_div()?;
                        left = left.checked_add(right).unwrap_or(left);
                    }
                    b'-' => {
                        self.pos += 1;
                        let right = self.parse_mul_div()?;
                        left = left.checked_sub(right).unwrap_or(left);
                    }
                    _ => break,
                }
            }
            Ok(left)
        }

        fn parse_mul_div(&mut self) -> Result<Decimal, CfmlError> {
            let mut left = self.parse_unary()?;
            loop {
                self.skip_ws();
                if self.pos >= self.chars.len() {
                    break;
                }
                match self.chars[self.pos] {
                    b'*' => {
                        self.pos += 1;
                        let right = self.parse_unary()?;
                        left = left.checked_mul(right).unwrap_or(left);
                    }
                    b'/' => {
                        self.pos += 1;
                        let right = self.parse_unary()?;
                        if right.is_zero() {
                            return Err(CfmlError::runtime(
                                "Division by zero in precisionEvaluate".into(),
                            ));
                        }
                        left = left.checked_div(right).unwrap_or(left);
                    }
                    b'%' => {
                        self.pos += 1;
                        let right = self.parse_unary()?;
                        if right.is_zero() {
                            return Err(CfmlError::runtime(
                                "Division by zero in precisionEvaluate".into(),
                            ));
                        }
                        left = left.checked_rem(right).unwrap_or(left);
                    }
                    _ => break,
                }
            }
            Ok(left)
        }

        fn parse_unary(&mut self) -> Result<Decimal, CfmlError> {
            self.skip_ws();
            if self.pos < self.chars.len() && self.chars[self.pos] == b'-' {
                self.pos += 1;
                let val = self.parse_primary()?;
                Ok(-val)
            } else if self.pos < self.chars.len() && self.chars[self.pos] == b'+' {
                self.pos += 1;
                self.parse_primary()
            } else {
                self.parse_primary()
            }
        }

        fn parse_primary(&mut self) -> Result<Decimal, CfmlError> {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                return Err(CfmlError::runtime(
                    "Unexpected end of expression in precisionEvaluate".into(),
                ));
            }
            if self.chars[self.pos] == b'(' {
                self.pos += 1;
                let val = self.parse_expr()?;
                self.skip_ws();
                if self.pos < self.chars.len() && self.chars[self.pos] == b')' {
                    self.pos += 1;
                } else {
                    return Err(CfmlError::runtime(
                        "Missing closing parenthesis in precisionEvaluate".into(),
                    ));
                }
                Ok(val)
            } else {
                // Parse number
                let start = self.pos;
                while self.pos < self.chars.len()
                    && (self.chars[self.pos].is_ascii_digit() || self.chars[self.pos] == b'.')
                {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(CfmlError::runtime(format!(
                        "Unexpected character '{}' in precisionEvaluate",
                        self.chars[self.pos] as char
                    )));
                }
                let num_str = std::str::from_utf8(&self.chars[start..self.pos])
                    .map_err(|_| CfmlError::runtime("Invalid UTF-8 in precisionEvaluate".into()))?;
                Decimal::from_str(num_str).map_err(|_| {
                    CfmlError::runtime(format!("Invalid number '{}' in precisionEvaluate", num_str))
                })
            }
        }
    }

    let mut parser = PrecParser::new(expr.trim());
    let result = parser.parse_expr()?;
    // Normalize: remove trailing zeros for display
    let s = result.normalize().to_string();
    Ok(s)
}
