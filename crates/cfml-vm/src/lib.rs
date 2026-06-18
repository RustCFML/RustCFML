//! CFML Virtual Machine - Bytecode execution engine

use cfml_codegen::{BytecodeFunction, BytecodeOp, BytecodeProgram, CmpOp};
use cfml_common::dynamic::{CfmlQuery, CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vfs::{RealFs, Vfs};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};
use cfml_qoq::function::{QoQFn, QoQFnKind, QoQFunctionRegistry};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

pub mod application_store;
pub mod async_kernel;
mod java_shims;
/// Optional Cranelift JIT (native targets, `--features jit`). The interpreter
/// remains the default and fallback; see `jit/mod.rs` and `JIT_DESIGN.md`.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
pub mod jit;
#[cfg(feature = "s3")]
mod s3_vfs;
pub mod session_store;
pub mod web;
pub use application_store::{ApplicationStore, MemoryApplicationStore};
pub use session_store::{MemoryStore, SessionStore};
use java_shims::{
    handle_java_collections, handle_java_concurrenthashmap, handle_java_concurrentlinkedqueue,
    handle_java_date, handle_java_file, handle_java_inetaddress, handle_java_linkedhashmap,
    handle_java_messagedigest, handle_java_paths, handle_java_pattern, handle_java_stringbuilder,
    handle_java_system, handle_java_thread, handle_java_treemap, handle_java_uuid,
};

pub type BuiltinFunction = fn(Vec<CfmlValue>) -> CfmlResult;

/// Lower bound for a session timeout, in seconds. Matches Cloudflare KV's
/// minimum `expiration_ttl`; clamping here guarantees a session never carries
/// a 0/sub-minimum timeout that would make its KV write invalid or expire it
/// immediately.
const MIN_SESSION_TIMEOUT_SECS: u64 = 60;

/// Reserved `locals` key under which a frame's `arguments` scope is stored.
/// CFML keeps `local` and `arguments` as fully independent scopes (Lucee and
/// BoxLang both back them with separate scope objects), so the scope must NOT
/// occupy the literal `"arguments"` key — that key is reserved for a user's own
/// `local.arguments = …` variable. The `__` prefix means every introspection
/// filter that already skips `__`-prefixed keys hides this automatically. All
/// bare-`arguments` reads/writes route here via `scope_aware_load` /
/// `scope_aware_store` / `lookup_name_in_scopes`.
const ARGUMENTS_SCOPE_KEY: &str = "__arguments__";

/// A stored `CfmlValue::Function` body is `CfmlClosureBody::Expression(Int(n))`
/// where `n` is the target function's process-stable `BytecodeFunction.global_id`
/// (see `cfml_codegen::compiler::next_global_fn_id`). `DefineFunction` ops carry
/// the same id. The VM resolves it through the dense per-request `fn_registry`
/// (`CfmlVirtualMachine::resolve_fn`) with an O(1) array index — no hashing, and
/// independent of the volatile `self.program` layout. This is the single function
/// identity scheme: there are no program-relative indices in stored bodies or ops
/// any more, so the stale-index bug class (cross-request dispatch and the issue
/// #70 intra-request program swap) cannot occur by construction. A non-`Int`
/// body (e.g. `Null` for native/intercepted functions) simply isn't a UDF
/// reference and dispatches by name.

/// Persistent application state, keyed by app name.
#[derive(Clone)]
pub struct ApplicationState {
    pub name: String,
    pub variables: ValueMap,
    pub started: bool,
    pub config: ValueMap,
    /// Functions reachable from application scope, carried across requests so a
    /// long-lived CFC instance / factory / closure stays resolvable even on a
    /// request that doesn't reload its source file. At request start these `Arc`s
    /// are registered into the VM's `fn_registry` by their stable `global_id`,
    /// which is the single function identity stored bodies carry — so dispatch
    /// never depends on a per-request program-table layout (no append, no remap).
    /// Recomputed from reachability each request that defines a function, so it
    /// stays bounded (abandoned functions drop out).
    pub app_function_table: Vec<std::sync::Arc<cfml_codegen::compiler::BytecodeFunction>>,
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
    pub variables: ValueMap,
    /// Unix epoch seconds when the session was created.
    pub created_secs: u64,
    /// Unix epoch seconds of the most recent access.
    pub last_accessed_secs: u64,
    pub auth_user: Option<String>,
    pub auth_roles: Vec<String>,
    /// Session timeout in seconds (default 1800 = 30 minutes)
    pub timeout_secs: u64,
    /// Name of the CFML application that owns this session. Recorded at create
    /// time so the background reaper can route `onSessionEnd` to the owning
    /// application's live context. Backward-compatible: older serialised
    /// records (datasource/cluster stores) deserialise with `""`.
    #[serde(default)]
    pub app_name: String,
}

/// True when a struct carries CFC instance markers — this engine materialises
/// CFCs as marker-bearing structs (as well as the `Component` variant), so a
/// struct with a `__variables` scope plus a `this`/`__name` marker is a
/// component instance, not user data. Used to suppress engine internals during
/// for-in iteration and serialization.
fn is_component_struct(s: &cfml_common::dynamic::CfmlStruct) -> bool {
    s.contains_key_ci("__variables")
        && (s.contains_key_ci("this") || s.contains_key_ci("__name"))
}

/// Data-only session rule (issue #88): the session scope persists data values
/// only — no components, closures/functions, or native objects. Returns the
/// offending key-path on the first violation so the error names exactly what
/// to remove. Enforced across every store (memory included) so the rule is a
/// session-scope semantic, not a deployment surprise that only bites once an
/// external store silently drops the value to null.
fn validate_session_value(path: &str, v: &CfmlValue) -> Result<(), String> {
    match v {
        CfmlValue::Closure(_) => Err(format!("{} is a closure", path)),
        CfmlValue::Function(_) => Err(format!("{} is a function", path)),
        CfmlValue::Component(_) => Err(format!("{} is a component", path)),
        CfmlValue::NativeObject(_) => Err(format!("{} is a native object", path)),
        CfmlValue::Array(a) => {
            for (i, item) in a.snapshot().iter().enumerate() {
                validate_session_value(&format!("{}[{}]", path, i + 1), item)?;
            }
            Ok(())
        }
        CfmlValue::Struct(m) => {
            let snap = m.snapshot();
            // A struct carrying CFC instance markers is a component instance
            // (this engine materialises CFCs as marker-bearing structs as well
            // as the `Component` variant).
            let looks_like_cfc = snap.keys().any(|k| k.eq_ignore_ascii_case("__variables"))
                && snap.keys().any(|k| {
                    k.eq_ignore_ascii_case("this") || k.eq_ignore_ascii_case("__name")
                });
            if looks_like_cfc {
                return Err(format!("{} is a component", path));
            }
            for (k, val) in snap.iter() {
                // Skip internal markers; user-visible keys only.
                if k.starts_with("__") {
                    continue;
                }
                validate_session_value(&format!("{}.{}", path, k), val)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate a whole session variable map against the data-only rule, turning a
/// violation into a `CfmlError` whose message names the offending key path.
fn validate_session_data_only(vars: &ValueMap) -> Result<(), CfmlError> {
    for (k, v) in vars.iter() {
        if k.starts_with("__") {
            continue;
        }
        validate_session_value(&format!("session.{}", k), v).map_err(|p| {
            CfmlError::runtime(format!(
                "{}; the session scope only persists data values \
                 (no components, closures, functions, or native objects)",
                p
            ))
        })?;
    }
    Ok(())
}

/// Returns current Unix epoch seconds.
#[inline]
fn now_epoch_secs() -> u64 {
    cfml_common::clock::now_unix_secs()
}

/// Parse a CFML date/datetime string for cookie-expiry rendering. Mirrors the
/// common formats accepted by the stdlib `parse_cfml_date`, including the
/// `createDateTime` output form (`%Y-%m-%d %H:%M:%S`).
fn parse_cookie_expiry_date(s: &str) -> Option<chrono::NaiveDateTime> {
    use chrono::{NaiveDate, NaiveDateTime};
    for fmt in &[
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
        "%d-%b-%Y %H:%M:%S",
        "%a, %d-%b-%Y %H:%M:%S GMT",
        "%a, %d %b %Y %H:%M:%S GMT",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }
    for fmt in &["%Y-%m-%d", "%m/%d/%Y", "%m-%d-%Y", "%d-%b-%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return d.and_hms_opt(0, 0, 0);
        }
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.naive_utc());
    }
    None
}

/// Render a `<cfcookie expires=...>` value as an RFC-1123 / RFC-6265 cookie date
/// in GMT (Lucee parity), e.g. `Tue, 14-Jul-2026 06:30:57 GMT`. Browsers require
/// a month *name* and `GMT`; a raw `2026-12-31 00:00:00` or bare `30` fails to
/// parse and the cookie silently degrades to a session cookie.
///
/// Accepts a date/datetime, a number of days from now, `"now"` (expires
/// immediately), or `"never"` (far future). Unparseable values pass through
/// unchanged.
fn format_cookie_expires(raw: &str) -> String {
    use chrono::{Duration, Utc};
    const FMT: &str = "%a, %d-%b-%Y %H:%M:%S GMT";
    let now = Utc::now().naive_utc();
    let trimmed = raw.trim();
    let lower = trimmed.to_lowercase();

    let dt = match lower.as_str() {
        "never" => now + Duration::days(3650),
        "now" => now,
        _ => {
            if let Ok(days) = trimmed.parse::<f64>() {
                now + Duration::seconds((days * 86400.0) as i64)
            } else if let Some(parsed) = parse_cookie_expiry_date(trimmed) {
                parsed
            } else {
                return trimmed.to_string();
            }
        }
    };
    dt.format(FMT).to_string()
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
        Err(_) => CfmlValue::strukt(ValueMap::default()),
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
        serde_json::Value::String(s) => CfmlValue::string(s),
        serde_json::Value::Array(arr) => {
            CfmlValue::array(arr.into_iter().map(json_value_to_cfml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = ValueMap::default();
            for (k, v) in obj {
                map.insert(k, json_value_to_cfml(v));
            }
            CfmlValue::strukt(map)
        }
    }
}

fn build_server_scope() -> ValueMap {
    let mut info = ValueMap::default();

    let mut cf = ValueMap::default();
    cf.insert(
        "productname".to_string(),
        CfmlValue::string("RustCFML".to_string()),
    );
    cf.insert(
        "productversion".to_string(),
        CfmlValue::string(env!("CARGO_PKG_VERSION").to_string()),
    );
    cf.insert(
        "productlevel".to_string(),
        CfmlValue::string("Final".to_string()),
    );
    info.insert("coldfusion".to_string(), CfmlValue::strukt(cf));

    let mut os = ValueMap::default();
    os.insert(
        "name".to_string(),
        CfmlValue::string(std::env::consts::OS.to_string()),
    );
    os.insert(
        "arch".to_string(),
        CfmlValue::string(std::env::consts::ARCH.to_string()),
    );
    if let Ok(host) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
        os.insert("hostname".to_string(), CfmlValue::string(host));
    }
    info.insert("os".to_string(), CfmlValue::strukt(os));

    let file_sep = std::path::MAIN_SEPARATOR.to_string();
    let path_sep = if cfg!(windows) { ";" } else { ":" }.to_string();
    let line_sep = if cfg!(windows) { "\r\n" } else { "\n" }.to_string();

    let mut sep = ValueMap::default();
    sep.insert("file".to_string(), CfmlValue::string(file_sep.clone()));
    sep.insert("path".to_string(), CfmlValue::string(path_sep.clone()));
    sep.insert("line".to_string(), CfmlValue::string(line_sep.clone()));
    info.insert("separator".to_string(), CfmlValue::strukt(sep));

    let mut java = ValueMap::default();
    java.insert(
        "version".to_string(),
        CfmlValue::string(String::new()),
    );
    java.insert(
        "vendor".to_string(),
        CfmlValue::string("RustCFML (no JVM)".to_string()),
    );
    java.insert(
        "archModel".to_string(),
        CfmlValue::string(
            if cfg!(target_pointer_width = "64") { "64" } else { "32" }.to_string(),
        ),
    );
    info.insert("java".to_string(), CfmlValue::strukt(java));

    let mut system = ValueMap::default();

    let mut env = ValueMap::default();
    for (k, v) in std::env::vars() {
        env.insert(k, CfmlValue::string(v));
    }
    system.insert("environment".to_string(), CfmlValue::strukt(env));

    let mut props = ValueMap::default();
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
        CfmlValue::string(std::env::consts::OS.to_string()),
    );
    props.insert(
        "os.arch".to_string(),
        CfmlValue::string(std::env::consts::ARCH.to_string()),
    );
    props.insert("user.dir".to_string(), CfmlValue::string(cwd));
    props.insert("user.home".to_string(), CfmlValue::string(home));
    props.insert("user.name".to_string(), CfmlValue::string(user));
    props.insert("java.io.tmpdir".to_string(), CfmlValue::string(tmp));
    props.insert("file.separator".to_string(), CfmlValue::string(file_sep));
    props.insert("path.separator".to_string(), CfmlValue::string(path_sep));
    props.insert("line.separator".to_string(), CfmlValue::string(line_sep));
    props.insert(
        "file.encoding".to_string(),
        CfmlValue::string("UTF-8".to_string()),
    );
    system.insert("properties".to_string(), CfmlValue::strukt(props));

    let args: Vec<CfmlValue> = std::env::args()
        .skip(1)
        .map(CfmlValue::string)
        .collect();
    system.insert("arguments".to_string(), CfmlValue::array(args));

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
    // cfinclude is expected to splice in verbatim (markup, stylesheets,
    // scripts, data files). Without this, e.g. `<cfinclude template="x.css">`
    // parses raw CSS (`body { background-color: #f3f4f6; }`) as CFML script and
    // explodes on the first hex color, and `<cfinclude template="jquery.min.js">`
    // (Wheels' error-page layout does exactly this) parses minified JS as
    // cfscript and dies on `typeof window`. Skip preprocessing for .cfc files
    // that don't have CFML tags — those are script-only components
    // (`component { ... }`).
    let lower_path = path.to_lowercase();
    let is_template_ext = lower_path.ends_with(".cfm")
        || lower_path.ends_with(".css")
        || lower_path.ends_with(".html")
        || lower_path.ends_with(".htm")
        || lower_path.ends_with(".txt")
        || lower_path.ends_with(".js")
        || lower_path.ends_with(".mjs")
        || lower_path.ends_with(".json")
        || lower_path.ends_with(".svg")
        || lower_path.ends_with(".xml")
        || lower_path.ends_with(".map");
    let needs_tag_parse =
        cfml_compiler::tag_parser::has_cfml_tags(&source_code) || is_template_ext;
    let source_code = if needs_tag_parse {
        let converted = cfml_compiler::tag_parser::tags_to_script_checked(&source_code)
            .map_err(|msg| CfmlError::runtime(format!("{} in '{}'", msg, path)))?;
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

    // Compile. Stamp the source path onto every function so app-scope-reachable
    // functions carry a stable `(source_file, name, ordinal)` identity.
    let compiler =
        cfml_codegen::compiler::CfmlCompiler::new().with_source_file(Some(path.to_string()));
    let program = compiler.compile(ast);

    // --jit-coverage / RUSTCFML_JIT_COVERAGE=1: dump the Option-γ forecast
    // for this compilation unit before it gets cached. Cheap, side-effect-
    // free walk of the bytecode (see `JIT_POLY_DESIGN.md` and v0.88.0).
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    if matches!(
        std::env::var("RUSTCFML_JIT_COVERAGE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    ) {
        let report = jit::coverage::scan_program(&program);
        eprintln!("=== {} ===\n{}", path, report.render());
    }

    // Cache the result
    if let Some(c) = cache {
        if let Ok(mtime) = vfs.modified(path) {
            c.insert(path.to_string(), program.clone(), mtime);
        }
    }

    Ok(program)
}

/// Bound the `named_locks` map so long-lived `--serve` processes using
/// dynamic lock names (e.g. `name="user_#id#"`) don't grow it unboundedly.
/// When the map is at/over `cap` and `new_name` isn't already present, evict
/// every entry whose `Arc::strong_count == 1`: only the map holds those, so no
/// thread is holding or waiting on the lock and dropping the `RwLock` cannot
/// invalidate a live `held_locks` guard. Entries with `strong_count > 1` (held
/// or contended) are always kept.
fn evict_idle_named_locks(
    locks: &mut HashMap<String, Arc<RwLock<()>>>,
    new_name: &str,
    cap: usize,
) {
    if locks.len() >= cap && !locks.contains_key(new_name) {
        locks.retain(|_, l| Arc::strong_count(l) > 1);
    }
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
    /// Cache of per-application `.cfconfig.json` files (the overlaid result of
    /// baseline + the file beside an Application.cfc), keyed by the candidate
    /// file path. `Some(None)` = checked, no file there. Only populated when
    /// `production_mode` is true — the file read + JSON parse + overlay is a
    /// pure function of a static file, so it is held in memory rather than
    /// repeated every request. (Derived `this.*` values are NOT cached — those
    /// come from re-executing Application.cfc, which must run every request.)
    pub app_cfconfig_cache:
        Arc<parking_lot::RwLock<HashMap<std::path::PathBuf, Option<Arc<cfml_config::RustCfmlConfig>>>>>,
    /// Resolved `.cfconfig.json` (or defaults if no file). Wraps in `Arc` so
    /// every cloned ServerState shares the same struct without re-parsing.
    pub cfconfig: Arc<cfml_config::RustCfmlConfig>,
    /// `onSessionEnd` deliveries queued by the background reaper, keyed by the
    /// owning application name. The reaper drains expired session *data* off
    /// the request path (cleanup); the hook itself needs a live per-app VM
    /// context, so each request for an application drains its queue here and
    /// fires `onSessionEnd` with that context. Bounded per app by
    /// `session.reapBatchMax` (oldest dropped beyond the cap).
    pub pending_session_ends:
        Arc<Mutex<HashMap<String, std::collections::VecDeque<ValueMap>>>>,
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
            app_cfconfig_cache: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            cfconfig,
            pending_session_ends: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Queue an expired session's variables for an opportunistic `onSessionEnd`
    /// on the next request for `app_name`. Bounded: once a per-app queue reaches
    /// `batch_max`, the oldest entry is dropped (and counted) so an application
    /// that is never revisited cannot grow the queue without limit.
    pub fn queue_session_end(
        &self,
        app_name: &str,
        vars: ValueMap,
        batch_max: usize,
    ) -> bool {
        if let Ok(mut q) = self.pending_session_ends.lock() {
            let dq = q.entry(app_name.to_string()).or_default();
            let mut dropped = false;
            while batch_max > 0 && dq.len() >= batch_max {
                dq.pop_front();
                dropped = true;
            }
            dq.push_back(vars);
            return dropped;
        }
        false
    }

    /// Drain all queued `onSessionEnd` deliveries for `app_name`, returning
    /// their variable maps in arrival order. Empty when nothing is pending.
    pub fn drain_session_ends(&self, app_name: &str) -> Vec<ValueMap> {
        if let Ok(mut q) = self.pending_session_ends.lock() {
            if let Some(dq) = q.get_mut(app_name) {
                return dq.drain(..).collect();
            }
        }
        Vec::new()
    }
}

/// A held lock guard for cflock (keeps the lock alive during the block).
/// The guard fields are never read directly — they are held for their Drop behavior.
#[allow(dead_code)]
enum HeldLock {
    Write(std::sync::RwLockWriteGuard<'static, ()>),
    Read(std::sync::RwLockReadGuard<'static, ()>),
    /// A reentrant re-acquire of a lock already held by this request/thread.
    /// Holds no real guard — named locks are reentrant per thread, so the inner
    /// acquire must not block on the non-reentrant RwLock. Popped in LIFO order
    /// by __cflock_end without releasing the underlying guard.
    Reentrant,
}

pub struct CfmlVirtualMachine {
    pub program: BytecodeProgram,
    pub globals: ValueMap,
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
    method_variables_writeback: Option<CfmlStruct>,
    /// After a closure executes, holds modified parent-scope variables for write-back
    /// to the caller's locals. Enables closures to mutate parent scope.
    closure_parent_writeback: Option<ValueMap>,
    /// Request scope — lives for the duration of one request
    /// Request scope, backed by a shared `CfmlStruct` (`Arc<RwLock<IndexMap>>`)
    /// so spawned `cfthread` child VMs can share it live with the parent (CFML
    /// request scope is shared across threads). Reads still return a snapshot;
    /// writes mutate the shared backing store in place.
    pub request_scope: CfmlStruct,
    /// Application scope — backed by a shared `CfmlStruct` (`Arc<RwLock<IndexMap>>`)
    /// so that reading the `application` scope returns a LIVE reference (handle
    /// clone), not a snapshot. This makes the CFML "scope pointer" pattern
    /// (`var p = application; p[k] = v;`) write through, matching Lucee/ACF —
    /// e.g. WireBox's ScopeStorage caches via that pattern. Across requests in
    /// --serve mode the contents are synced to/from ServerState.applications.
    pub application_scope: Option<CfmlStruct>,
    /// Name of the application currently attached to `application_scope`.
    /// Needed so `applicationStop()` knows which shared entry to reset.
    current_application_name: Option<String>,
    /// Live session scope for the current request, backed by a shared
    /// `CfmlStruct`. Like `application_scope`, reading `session` returns a live
    /// handle clone (not a snapshot) so the scope-pointer pattern writes through
    /// (WireBox session-scoped caching). Loaded from the session store when the
    /// session is established and synced back at request end. `None` until a
    /// session exists this request.
    session_scope: Option<CfmlStruct>,
    /// Set when `applicationStop()` has torn down the attached application during
    /// this request, so the end-of-request writeback does not resurrect it.
    application_stopped: bool,
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
    captured_locals: Option<ValueMap>,
    /// Active transaction connection (held during cftransaction block, type-erased)
    pub transaction_conn: Option<Box<dyn std::any::Any>>,
    /// Datasource URL of the active transaction
    pub transaction_datasource: Option<String>,
    /// True while inside a `transaction { }` block whose connection has NOT yet
    /// been established because no datasource was resolvable at block entry.
    /// The first query inside the block begins the transaction lazily on its
    /// own datasource — Lucee/ACF "defer begin until the first query" behaviour.
    pub transaction_pending: bool,
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
    /// Web-root-relative path of a requested template that does NOT exist on
    /// disk. When set, `execute_with_lifecycle` runs the Application.cfc
    /// `onMissingTemplate(targetPage)` handler (after onApplicationStart /
    /// onSessionStart) instead of executing a target page. The embedder sets
    /// this for a 404'd `.cfm`/`.cfc` request and reads `missing_template_handled`
    /// back to decide whether to emit the handler's output or a default 404.
    pub missing_template: Option<String>,
    /// Set by the onMissingTemplate path: true when a handler ran and returned
    /// a value other than `false` (i.e. it handled the request and the default
    /// 404 should be suppressed). Stays false when no handler exists or it
    /// explicitly returned `false`.
    pub missing_template_handled: bool,
    /// Resolved `this.sessioncookie` attributes for the current application.
    /// Embedders (the `--serve` HTTP layer, the Workers fetch handler) read
    /// this to render the session `Set-Cookie`. Defaults until an
    /// Application.cfc is loaded by `extract_app_config`.
    pub session_cookie_policy: cfml_common::session_cookie::SessionCookiePolicy,
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
    /// Per-request application-level cfconfig: a `.cfconfig.json` discovered
    /// beside this request's `Application.cfc`, overlaid on the server baseline
    /// (`server_state.cfconfig`). When present it is the source for the
    /// CFML-visible `server.cfconfig` struct. Server-level keys (port, etc.) are
    /// never overlaid. Cleared at request end. `None` ⇒ use the server baseline.
    pub app_cfconfig: Option<Arc<cfml_config::RustCfmlConfig>>,
    /// Per-request application-level datasources: lowercased name → resolved
    /// connection URL. Seeded from `this.datasources` in Application.cfc (highest
    /// precedence) and the per-request cfconfig (`app_cfconfig`, else baseline).
    /// `cfquery`/`queryExecute` consult this BEFORE the process-global registry,
    /// so each application's datasources are isolated (Lucee/BoxLang parity).
    /// Empty ⇒ fall back to the global registry / bare-string parsing.
    app_datasources: IndexMap<String, String>,
    /// Per-request default datasource URL (from `this.datasource` singular, or a
    /// cfconfig `default: true` datasource). Used when a query omits a datasource.
    app_default_datasource: Option<String>,
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
    /// `cfexit` control-flow signal. `__cfexit` sets this to the lowered method
    /// ("exittag" / "exittemplate" / "loop"); the bytecode loop then unwinds the
    /// current function like reaching its end (preserving partial locals so a
    /// custom tag's start-phase caller writes still propagate). The owning
    /// `__main__`/`__cfc_body__` frame moves the method into `last_exit_method`
    /// as it unwinds, so callers (e.g. the custom-tag handlers) can act on it.
    pending_exit: Option<String>,
    /// The `cfexit` method carried out of the most recently unwound template
    /// frame. Consumed by the custom-tag handlers to decide exittag (skip body +
    /// end) vs exittemplate (run body + end normally).
    last_exit_method: Option<String>,
    /// Buffered `<cfhtmlhead>` / `<cfhtmlbody>` content, injected into the
    /// response `<head>` / `<body>` (or appended) at output finalization.
    html_head_buffer: String,
    html_body_buffer: String,
    /// Per-request handle on the application's carried function table
    /// (`ApplicationState::app_function_table`). At request start these Arcs are
    /// registered into `fn_registry` by global_id so an application-scope
    /// function whose source file isn't reloaded this request still resolves.
    app_function_table: Vec<Arc<BytecodeFunction>>,
    /// Dense per-request function registry, indexed by `BytecodeFunction.global_id`.
    /// Populated as programs become reachable (root program at construction, every
    /// `push_program_swap`, and the carried `app_function_table` at request start).
    /// Every stored `CfmlValue::Function` body and every `DefineFunction` op carries
    /// a `global_id`; dispatch resolves it here with an O(1) index — no hashing, and
    /// independent of the volatile `self.program` layout, so cross-request dispatch
    /// and the issue #70 intra-request swap are both correct by construction. Sized
    /// to the max `global_id` seen, i.e. the app's distinct-function count (cached
    /// programs reuse ids), not per-request growth.
    fn_registry: Vec<Option<Arc<BytecodeFunction>>>,
    /// Set whenever a `DefineFunction` op runs during a request. A new
    /// `CfmlValue::Function` can ONLY be born via that op (closures, arrows,
    /// component methods, and function references all compile to it), so if it
    /// never fires this request, no function created this request can have
    /// entered application scope — every app-reachable function is already
    /// stable-tagged from an earlier request and the end-of-request re-homing
    /// walk is a guaranteed no-op. Gating the walk on this flag lets a request
    /// that defines no function skip the app-scope graph traversal entirely.
    /// Reset at the start of `execute_with_lifecycle`.
    app_fn_table_dirty: bool,
    /// In-memory cache: key -> (value, optional expiry instant)
    pub cache: HashMap<String, (CfmlValue, Option<cfml_common::clock::Monotonic>)>,
    /// cfsetting enableCFOutputOnly counter (>0 means only cfoutput content is emitted)
    pub enable_cfoutput_only: i32,
    /// Sandbox mode: blocks host filesystem access, routes reads through VFS
    pub sandbox: bool,
    /// After a function call, holds modified complex-type argument values for
    /// pass-by-reference writeback. Maps param name → final value.
    arg_ref_writeback: Option<Vec<(String, CfmlValue)>>,
    /// Caller-scope variable deliveries requested by a VM-intercepted call —
    /// queryExecute's `result` option and cfquery's `name`/`result`
    /// attributes (which arrive at runtime, e.g. inside Wheels'
    /// attributeCollection, so no compile-time assignment can see them).
    /// Each entry is a (possibly dotted, scope-prefixed) variable path plus
    /// the value. The intercept runs inside `call_function`, which cannot
    /// reach the calling frame's locals, so the Call/CallNamed op handlers
    /// apply these right after the call returns (same pattern as
    /// `arg_ref_writeback`).
    pending_result_writeback: Option<Vec<(String, CfmlValue)>>,
    /// Named arguments supplied at a callsite that do not match any declared
    /// param on the callee. Each entry is `(positional_index, name)`. Drained
    /// by the next `execute_function_with_args` so the callee's `arguments`
    /// scope keeps the original names (matches Lucee/ACF/BoxLang: extras
    /// stay reachable as `arguments.<name>`).
    pending_extra_named_args: Option<Vec<(usize, String)>>,
    /// The name a member method was invoked under at its call site. Set by
    /// `call_member_function` just before dispatching, and drained by the next
    /// `execute_function_with_args` into the new frame's `called_name`. Lets
    /// `getFunctionCalledName()` report the alias a UDF was called by — the
    /// primitive WireBox delegation relies on (one `getByDelegate` UDF injected
    /// under many method names, dispatched by the called name).
    pending_called_name: Option<String>,
    /// Registry of Rust-backed classes, keyed by lowercased class name.
    /// Populated via `register_native_class`. The function is invoked when
    /// CFML calls `createObject("rust", "Name", ...)` / `new rust:Name(...)`
    /// to produce a fresh `CfmlValue::NativeObject`.
    pub native_classes: HashMap<String, NativeConstructor>,

    /// Query-of-Queries function registry: native scalar/aggregate functions
    /// registered via `register_native_qoq_fn`, plus CFML UDFs/closures
    /// registered at runtime via `queryRegisterFunction`. Used inside
    /// `queryExecute(sql, params, {dbtype:"query"})`.
    pub qoq_registry: QoQFunctionRegistry,

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

    /// Injected `cfthread` spawn function. `None` ⇒ run thread bodies
    /// synchronously inline (wasm targets, or the `real-threads` feature off).
    /// Set by the CLI to a real-OS-thread spawner; mirrors the `txn_*` fn-ptr
    /// injection pattern.
    pub thread_spawn_fn: Option<ThreadSpawnFn>,
    /// Live `cfthread` handles keyed by lowercased thread name, awaiting join.
    /// Empty and untouched for code that never spawns a thread.
    pub live_threads: HashMap<String, ThreadHandle>,
    /// Set on a spawned child VM: when flipped true (by thread terminate), the
    /// execute loop aborts cooperatively. `None` on the main/parent VM, so the
    /// non-threaded hot path pays nothing.
    pub cancel_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Optional Cranelift JIT engine. `Some` only under `--features jit` on a
    /// native target when not disabled via `RUSTCFML_JIT=0`. Consulted at the
    /// top of `execute_function_with_args`; the interpreter is always the
    /// fallback, so this never changes behaviour. Field absent entirely when
    /// the feature is off.
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    jit: Option<jit::JitEngine>,
}

/// Constructor signature for a Rust-backed class registered via
/// `register_native_class`. Receives the constructor arguments and must
/// return a `CfmlValue::NativeObject` wrapping the new instance.
pub type NativeConstructor = fn(Vec<CfmlValue>) -> CfmlResult;

/// Outcome of running a `cfthread` body (inline or on a real OS thread): the
/// metadata the parent surfaces via the `cfthread` scope after join.
#[derive(Debug, Clone, Default)]
pub struct ThreadResult {
    /// `COMPLETED` or `TERMINATED`.
    pub status: String,
    /// Anything the body wrote to the page output (captured separately).
    pub output: String,
    /// Stringified error if the body threw, else empty.
    pub error: String,
    /// Wall-clock duration of the body in milliseconds.
    pub elapsed: i64,
    /// The body's `thread` scope (thread.x = ...), surfaced as cfthread.NAME.x.
    pub thread_vars: ValueMap,
    /// The closure's return value (Ok-arm). Used by the async kernel
    /// (`future.get()`) to surface the result; `cfthread` ignores it.
    pub return_value: Option<CfmlValue>,
}

/// Everything a freshly-spawned child VM needs to run one `cfthread` body on a
/// real OS thread. `Send` by construction (asserted below): it carries only
/// shareable state — `Arc`-backed program/functions/scopes plus `CfmlValue`
/// (which is `Send + Sync`). The parent VM itself never crosses the boundary.
pub struct ThreadSeed {
    pub program: BytecodeProgram,
    pub user_functions: HashMap<String, Arc<BytecodeFunction>>,
    /// Per-thread copy of the parent `variables` scope at spawn (CFML copy
    /// semantics: top-level reassignments don't leak back; nested objects stay
    /// by-reference since `CfmlValue` arrays/structs are `Arc`-backed).
    pub variables_snapshot: ValueMap,
    pub vfs: Arc<dyn Vfs>,
    pub server_state: Option<ServerState>,
    /// Shared live with the parent (handle clone) — see VM `application_scope`.
    pub application_scope: Option<CfmlStruct>,
    /// Shared live with the parent (CFML request scope is shared across threads).
    pub request_scope: CfmlStruct,
    /// Shared live with the parent (handle clone) — see VM `session_scope`.
    pub session_scope: Option<CfmlStruct>,
    pub session_id: Option<String>,
    pub current_application_name: Option<String>,
    pub base_template_path: Option<String>,
    pub source_file: Option<String>,
    pub mappings: Vec<CfmlMapping>,
    pub custom_tag_paths: Vec<String>,
    pub app_local_mode_modern: bool,
    pub sandbox: bool,
    pub null_support: bool,
    pub dot_notation_upper: bool,
    pub locale: String,
    pub timezone: String,
    pub whitespace_compression: bool,
    pub session_timeout_secs: u64,
    pub application_timeout_secs: u64,
    pub client_timeout_secs: u64,
    pub disallowed_functions: std::collections::HashSet<String>,
    pub disallowed_imports: Vec<regex::Regex>,
    /// The thread body (a `function(){...}` closure) to invoke.
    pub closure: CfmlValue,
    /// Passed `cfthread` attributes, exposed as the `attributes` scope.
    pub attributes: Option<CfmlValue>,
    /// Cooperative-cancellation flag the child polls (set by thread terminate).
    pub cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// A spawned `cfthread`'s live handle, held by the parent until join. Lives on
/// the single-threaded parent VM, so it need not be `Send`.
pub struct ThreadHandle {
    /// Original-case thread name (the `cfthread` scope key is lowercased).
    pub name: String,
    /// Receives the body's `ThreadResult` exactly once, on completion.
    pub rx: std::sync::mpsc::Receiver<ThreadResult>,
    /// Cooperative-cancel flag shared with the running body.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// OS thread join handle (taken on first join).
    pub join: Option<std::thread::JoinHandle<()>>,
    /// Cached result once joined, so repeat reads of cfthread.NAME work.
    pub result: Option<ThreadResult>,
}

/// Injected `cfthread` spawner: builds a child VM from the seed, runs the body
/// on a real OS thread, and returns a handle the parent joins on. Defined by
/// the CLI (needs `std::thread` + the stdlib builtins). `None` ⇒ run the body
/// synchronously inline (wasm targets, or the `real-threads` feature off).
pub type ThreadSpawnFn = fn(ThreadSeed) -> ThreadHandle;

// Compile-time proof that a thread body's payload can cross a thread boundary.
// If a future field breaks this, the build fails here with a clear pointer.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<ThreadSeed>();
};

#[derive(Debug, Clone)]
struct CallFrame {
    function_name: String,
    /// The name this function was actually invoked under at the call site —
    /// for member calls this is the method name used (which can differ from
    /// `function_name` when one UDF is injected under several aliases, as
    /// WireBox does for delegated methods). Exposed by `getFunctionCalledName()`.
    /// Defaults to `function_name` for plain named calls.
    called_name: String,
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
    /// Depth of `saved_output_buffers` when the try region was entered. On
    /// unwind, any buffers pushed since (by an unterminated cfsavecontent /
    /// cfsilent / custom-tag body) are popped back so `output_buffer` is
    /// restored to the level it had at try-start.
    saved_buffers_depth: usize,
    /// Depth of `custom_tag_stack` when the try region was entered. On unwind,
    /// stale entries from custom-tag bodies that threw before their end op are
    /// truncated away.
    custom_tag_depth: usize,
}

/// State for a body-mode custom tag execution
#[derive(Debug, Clone)]
struct CustomTagState {
    template_path: String,
    attributes: CfmlValue,
    start_locals: ValueMap,
    /// `cfexit method="exittag"` fired during the start phase: the body and the
    /// end phase must be skipped, but partial start-phase caller writes still
    /// propagate.
    exit_tag: bool,
}

impl CfmlVirtualMachine {
    pub fn new(program: BytecodeProgram) -> Self {
        let mut vm = Self {
            program,
            globals: ValueMap::default(),
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
            request_scope: CfmlStruct::empty(),
            application_scope: None,
            session_scope: None,
            current_application_name: None,
            application_stopped: false,
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
            transaction_pending: false,
            txn_begin: None,
            txn_commit: None,
            txn_rollback: None,
            txn_execute: None,
            session_id: None,
            missing_template: None,
            missing_template_handled: false,
            session_cookie_policy: cfml_common::session_cookie::SessionCookiePolicy::default(),
            lazy_session_creation: false,
            session_lazy_pending: false,
            session_record_created: false,
            session_lazy_initializing: false,
            app_cfc_template: None,
            app_cfconfig: None,
            app_datasources: IndexMap::new(),
            app_default_datasource: None,
            query_execute_fn: None,
            held_locks: Vec::new(),
            custom_tag_paths: Vec::new(),
            app_local_mode_modern: false,
            custom_tag_stack: Vec::new(),
            pending_exit: None,
            last_exit_method: None,
            html_head_buffer: String::new(),
            html_body_buffer: String::new(),
            app_function_table: Vec::new(),
            fn_registry: Vec::new(),
            app_fn_table_dirty: false,
            cache: HashMap::new(),
            enable_cfoutput_only: 0,
            sandbox: false,
            arg_ref_writeback: None,
            pending_result_writeback: None,
            pending_extra_named_args: None,
            pending_called_name: None,
            native_classes: HashMap::new(),
            qoq_registry: QoQFunctionRegistry::new(),
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
            thread_spawn_fn: None,
            live_threads: HashMap::new(),
            cancel_flag: None,
            #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
            jit: jit::JitEngine::maybe_new(),
        };
        // Register the root program's functions so stored references and
        // DefineFunction ops resolve by global_id from the first instruction.
        let root = vm.program.clone();
        vm.register_program_fns(&root);
        vm
    }

    /// Number of user functions the JIT has compiled to native code so far.
    /// `0` when the `jit` feature is off or the engine is disabled. Exposed for
    /// observability and tests that need to confirm the JIT actually fired.
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn jit_compiled_count(&self) -> usize {
        self.jit.as_ref().map_or(0, |j| j.compiled_count())
    }

    /// Number of loop bodies the OSR engine has compiled. `0` when the `jit`
    /// feature is off or the engine is disabled. Exposed for tests asserting
    /// OSR fired (distinct from whole-function JIT).
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn osr_compiled_count(&self) -> usize {
        self.jit.as_ref().map_or(0, |j| j.osr_compiled_count())
    }

    /// Replace this VM's JIT engine with one at an explicit hotness threshold,
    /// ignoring `RUSTCFML_JIT` / `RUSTCFML_JIT_THRESHOLD`. Tests use this for
    /// deterministic JIT engagement: mutating the process environment instead
    /// races when the test runner executes other tests on parallel threads.
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn jit_set_threshold(&mut self, threshold: u32) {
        self.jit = jit::JitEngine::new_with_threshold(threshold);
    }

    /// Force-disable the JIT for this VM regardless of environment — the
    /// deterministic equivalent of `RUSTCFML_JIT=0` (interpreter oracle).
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn jit_disable(&mut self) {
        self.jit = None;
    }

    /// Adopt this thread's persistent JIT engine into `self.jit`, replacing the
    /// empty per-request engine `new()` installed. Used by serve mode so the
    /// hotness counters and compiled native code accumulate across requests
    /// landing on the same `spawn_blocking` worker thread, instead of being
    /// rebuilt cold every request. The Cranelift module is `!Send`, so it can
    /// never be *shared* across threads — but a `thread_local!` keeps one engine
    /// per worker thread, and the module never leaves the thread that built it.
    ///
    /// First use per thread lazily constructs the engine via `maybe_new()`
    /// (honouring `RUSTCFML_JIT` / `RUSTCFML_JIT_THRESHOLD`). A `None` engine
    /// (JIT disabled) round-trips cleanly. Pair with [`Self::jit_return_persistent`]
    /// — or, better, use [`JitLease`] which guarantees the return even on unwind.
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn jit_take_persistent(&mut self) {
        self.jit = PERSISTENT_JIT.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                // `take()` below would yield `None` and the worker would never
                // warm up. `Some(None)` marks "constructed, but disabled" so we
                // don't re-run `maybe_new()` (and its env reads) every request.
                *slot = Some(jit::JitEngine::maybe_new());
            }
            slot.take().flatten()
        });
    }

    /// Return this VM's JIT engine to the thread-local so the next request on
    /// this worker reuses its warmed cache. Inverse of [`Self::jit_take_persistent`].
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    pub fn jit_return_persistent(&mut self) {
        let engine = self.jit.take();
        PERSISTENT_JIT.with(|cell| {
            *cell.borrow_mut() = Some(engine);
        });
    }
}

// Per-worker-thread persistent JIT engine for serve mode. `None` = not yet
// initialised on this thread; `Some(None)` = initialised but JIT disabled.
// The engine is moved out into the executing VM for the duration of a request
// (so all the hot-path `self.jit.as_mut()` sites are unchanged) and moved back
// afterwards. The Cranelift `JITModule` it owns is `!Send`, so it lives and
// dies on a single thread — never shared, only persisted per thread.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
thread_local! {
    static PERSISTENT_JIT: std::cell::RefCell<Option<Option<jit::JitEngine>>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard that adopts the thread-local persistent JIT engine into a VM for
/// the lifetime of the guard and returns it on `Drop` — even on an early return
/// or panic unwind. This is what makes serve-mode persistence panic-safe: a
/// panic inside `execute_with_lifecycle` costs at most a one-thread cold engine
/// rebuild, never a permanently-lost engine. No-op when the `jit` feature is off.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
pub struct JitLease<'a> {
    vm: &'a mut CfmlVirtualMachine,
}

#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
impl<'a> JitLease<'a> {
    pub fn new(vm: &'a mut CfmlVirtualMachine) -> Self {
        vm.jit_take_persistent();
        JitLease { vm }
    }
    /// Borrow the leased VM (so callers can drive execution through the guard).
    pub fn vm(&mut self) -> &mut CfmlVirtualMachine {
        self.vm
    }
}

#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
impl Drop for JitLease<'_> {
    fn drop(&mut self) {
        self.vm.jit_return_persistent();
    }
}

/// Shared OSR/JIT shadow-guard predicate: `true` when calling the canonical
/// builtin named `name` would be wrong because the live VM has been told to
/// resolve it differently (user-defined function with the same name, or a
/// non-canonical entry in `globals`). Hot-path: every cached compiled body
/// re-runs this for each builtin it references. Pulled out of the inline
/// closures in `execute_function_with_args` so the whole-fn JIT hook and
/// the OSR hooks (ForLoopStep / Jump / JumpIfTrue / JumpIfFalse) all share
/// one implementation.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
fn jit_is_shadowed(
    user_functions: &HashMap<String, Arc<BytecodeFunction>>,
    globals: &ValueMap,
    name: &str,
) -> bool {
    let lower = name.to_ascii_lowercase();
    let ufn_hit = user_functions.contains_key(name)
        || user_functions.keys().any(|k| k.eq_ignore_ascii_case(&lower));
    if ufn_hit {
        return true;
    }
    let g = globals.get(name).or_else(|| {
        globals
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
            .map(|(_, v)| v)
    });
    match g {
        Some(v) => {
            // Canonical builtin wrapper = `Function{ body: Expression(Null),
            // params: [], captured_scope: None }` produced by
            // `cfml_stdlib::create_builtin_func`. Anything else for an
            // allowlist name is real shadowing.
            if let CfmlValue::Function(f) = v {
                if !f.params.is_empty() || f.captured_scope.is_some() {
                    return true;
                }
                let body: &cfml_common::dynamic::CfmlClosureBody = &f.body;
                !matches!(
                    body,
                    cfml_common::dynamic::CfmlClosureBody::Expression(b) if matches!(b.as_ref(), CfmlValue::Null)
                )
            } else {
                true
            }
        }
        None => false,
    }
}

/// v0.91.0 — case-insensitive lookup against `user_functions` returning the
/// `(global_id, arity)` pair OSR / whole-fn JIT need to bind a `LoadGlobal`
/// of a non-builtin name to a UDF. Pulled out of the inline closures so the
/// OSR hooks (4 sites) and the whole-fn try_call site share one impl.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
fn jit_udf_lookup(
    user_functions: &HashMap<String, Arc<BytecodeFunction>>,
    name: &str,
) -> Option<jit::UdfMeta> {
    let lower = name.to_ascii_lowercase();
    let f = user_functions.get(name).or_else(|| {
        user_functions
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
            .map(|(_, v)| v)
    })?;
    Some(jit::UdfMeta {
        global_id: f.global_id,
        nparams: f.params.len(),
    })
}

impl CfmlVirtualMachine {
    // continuation of the previous impl block (split only to insert the
    // free `jit_is_shadowed` helper above with the matching cfg gates).

    /// Register every function in `prog` into `fn_registry` by its `global_id`
    /// (idempotent — a cached program registers the same Arc into the same slot).
    /// Grows the registry as needed; ids are dense across a process so the Vec
    /// stays sized to the app's distinct-function count.
    fn register_program_fns(&mut self, prog: &BytecodeProgram) {
        for f in &prog.functions {
            self.register_fn(f);
        }
    }

    /// Register a single function Arc into `fn_registry` by its `global_id`.
    fn register_fn(&mut self, f: &Arc<BytecodeFunction>) {
        let id = f.global_id as usize;
        if id >= self.fn_registry.len() {
            self.fn_registry.resize(id + 1, None);
        }
        if self.fn_registry[id].is_none() {
            self.fn_registry[id] = Some(Arc::clone(f));
        }
    }

    /// Resolve a `global_id` to its `BytecodeFunction` via the dense registry.
    /// O(1), no hashing, independent of the active `self.program`.
    #[inline]
    fn resolve_fn(&self, global_id: i64) -> Option<Arc<BytecodeFunction>> {
        if global_id < 0 {
            return None;
        }
        self.fn_registry
            .get(global_id as usize)
            .and_then(|slot| slot.clone())
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

    /// Snapshot everything a child VM needs to run `closure` as a `cfthread`
    /// body on its own OS thread. Shareable state (program/functions, request/
    /// application scope, server state, vfs) is `Arc`-cloned; the `variables`
    /// scope is copied (CFML copy-at-spawn semantics). The closure's captured
    /// lexical scope is deep-copied into a fresh `Arc` so the child's top-level
    /// reassignments don't leak back to the parent.
    pub fn build_thread_seed(
        &self,
        closure: CfmlValue,
        attributes: Option<CfmlValue>,
    ) -> ThreadSeed {
        let mut body = closure;
        if let CfmlValue::Function(f) = &mut body {
            let snap = f
                .captured_scope
                .as_ref()
                .map(|cap| cap.read().map(|g| g.clone()).unwrap_or_default());
            if let Some(snap) = snap {
                Arc::make_mut(f).captured_scope = Some(Arc::new(std::sync::RwLock::new(snap)));
            }
        }
        ThreadSeed {
            program: self.program.clone(),
            user_functions: self.user_functions.clone(),
            variables_snapshot: self.globals.clone(),
            vfs: self.vfs.clone(),
            server_state: self.server_state.clone(),
            application_scope: self.application_scope.clone(),
            request_scope: self.request_scope.clone(),
            session_scope: self.session_scope.clone(),
            session_id: self.session_id.clone(),
            current_application_name: self.current_application_name.clone(),
            base_template_path: self.base_template_path.clone(),
            source_file: self.source_file.clone(),
            mappings: self.mappings.clone(),
            custom_tag_paths: self.custom_tag_paths.clone(),
            app_local_mode_modern: self.app_local_mode_modern,
            sandbox: self.sandbox,
            null_support: self.null_support,
            dot_notation_upper: self.dot_notation_upper,
            locale: self.locale.clone(),
            timezone: self.timezone.clone(),
            whitespace_compression: self.whitespace_compression,
            session_timeout_secs: self.session_timeout_secs,
            application_timeout_secs: self.application_timeout_secs,
            client_timeout_secs: self.client_timeout_secs,
            disallowed_functions: self.disallowed_functions.clone(),
            disallowed_imports: self.disallowed_imports.clone(),
            closure: body,
            attributes,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Apply a `ThreadSeed` onto a freshly-constructed child VM (call after
    /// `new()` + the CLI's runtime registration). Wires shared scopes/config
    /// and overlays the parent's `variables` snapshot over the builtins.
    /// Returns the body closure + attributes to run. The two parent-only
    /// !Send fields (`held_locks`, `transaction_conn`) are intentionally left
    /// at their fresh-VM defaults — a child starts with no locks/transaction.
    pub fn apply_thread_seed(&mut self, seed: ThreadSeed) -> (CfmlValue, Option<CfmlValue>) {
        self.vfs = seed.vfs;
        self.server_state = seed.server_state;
        self.application_scope = seed.application_scope;
        self.request_scope = seed.request_scope;
        self.session_scope = seed.session_scope;
        self.session_id = seed.session_id;
        self.current_application_name = seed.current_application_name;
        self.base_template_path = seed.base_template_path;
        self.source_file = seed.source_file;
        self.mappings = seed.mappings;
        self.custom_tag_paths = seed.custom_tag_paths;
        self.user_functions = seed.user_functions;
        self.app_local_mode_modern = seed.app_local_mode_modern;
        self.sandbox = seed.sandbox;
        self.null_support = seed.null_support;
        self.dot_notation_upper = seed.dot_notation_upper;
        self.locale = seed.locale;
        self.timezone = seed.timezone;
        self.whitespace_compression = seed.whitespace_compression;
        self.session_timeout_secs = seed.session_timeout_secs;
        self.application_timeout_secs = seed.application_timeout_secs;
        self.client_timeout_secs = seed.client_timeout_secs;
        self.disallowed_functions = seed.disallowed_functions;
        self.disallowed_imports = seed.disallowed_imports;
        self.cancel_flag = Some(seed.cancel_flag);
        for (k, v) in seed.variables_snapshot {
            self.globals.insert(k, v);
        }
        (seed.closure, seed.attributes)
    }

    /// Run a single `cfthread` body closure and collect its outcome. Shared by
    /// the synchronous-inline path (on the parent VM) and by a spawned child
    /// VM, so both produce identical metadata. Does NOT store into the
    /// `cfthread` scope — the caller decides when (immediately, or on join).
    pub fn run_thread_body(
        &mut self,
        closure: &CfmlValue,
        attributes: Option<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> ThreadResult {
        // Fresh `thread` scope the body writes into (thread.x = ...).
        self.globals
            .insert("thread".to_string(), CfmlValue::strukt(ValueMap::default()));
        // Expose any passed attributes as the `attributes` scope.
        if let Some(attrs) = attributes {
            self.globals.insert("attributes".to_string(), attrs);
        }
        // Capture body output separately (same pattern as cfsavecontent).
        self.saved_output_buffers
            .push(std::mem::take(&mut self.output_buffer));
        let start_time = cfml_common::clock::Monotonic::now();
        let result = self.call_function(closure, vec![], parent_locals);
        let elapsed = start_time.elapsed().as_millis() as i64;
        let output = std::mem::take(&mut self.output_buffer);
        self.output_buffer = self.saved_output_buffers.pop().unwrap_or_default();
        let (error, return_value) = match &result {
            Err(e) => (format!("{}", e), None),
            Ok(v) => (String::new(), Some(v.clone())),
        };
        let thread_vars = match self.globals.shift_remove("thread") {
            Some(CfmlValue::Struct(ts)) => ts.snapshot(),
            _ => ValueMap::default(),
        };
        let status = if error.is_empty() {
            "COMPLETED"
        } else {
            "TERMINATED"
        };
        ThreadResult {
            status: status.to_string(),
            output,
            error,
            elapsed,
            thread_vars,
            return_value,
        }
    }

    /// Block until the named (lowercased) thread completes, or `timeout_ms`
    /// elapses (0 = wait forever), then publish its result into the `cfthread`
    /// scope. A timeout leaves the thread RUNNING and returns without error.
    /// No-op for an unknown or already-joined name.
    pub fn join_thread(&mut self, name: &str, timeout_ms: i64) {
        let collected: Option<ThreadResult> = {
            let handle = match self.live_threads.get_mut(name) {
                Some(h) => h,
                None => return,
            };
            if handle.result.is_some() {
                return; // already joined and published
            }
            let recv = if timeout_ms > 0 {
                handle
                    .rx
                    .recv_timeout(std::time::Duration::from_millis(timeout_ms as u64))
                    .ok()
            } else {
                handle.rx.recv().ok()
            };
            match recv {
                Some(res) => {
                    if let Some(j) = handle.join.take() {
                        let _ = j.join();
                    }
                    handle.result = Some(res.clone());
                    Some(res)
                }
                None => None, // timed out — thread keeps running
            }
        };
        if let Some(res) = collected {
            let orig = self
                .live_threads
                .get(name)
                .map(|h| h.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| name.to_string());
            self.store_cfthread_result(&orig, res);
        }
    }

    /// Store a completed `ThreadResult` into the `cfthread` scope as
    /// `cfthread.NAME = { status, name, output, error, elapsedtime, ...vars }`.
    pub fn store_cfthread_result(&mut self, thread_name: &str, r: ThreadResult) {
        let mut meta = ValueMap::default();
        meta.insert("status".to_string(), CfmlValue::string(r.status));
        meta.insert(
            "name".to_string(),
            CfmlValue::string(thread_name.to_string()),
        );
        meta.insert("output".to_string(), CfmlValue::string(r.output));
        meta.insert("error".to_string(), CfmlValue::string(r.error));
        meta.insert("elapsedtime".to_string(), CfmlValue::Int(r.elapsed));
        for (k, v) in r.thread_vars {
            meta.insert(k, v);
        }
        let thread_struct = self.get_or_create_cfthread_scope();
        if let Some(ts) = thread_struct.as_cfml_struct() {
            ts.insert(thread_name.to_lowercase(), CfmlValue::strukt(meta));
        }
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
            CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
                name: name.to_string(),
                params: Vec::new(),
                body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                    CfmlValue::Null,
                )),
                return_type: None,
                access: cfml_common::dynamic::CfmlAccess::Public,
                captured_scope: None,
            })),
        );
    }

    /// Register a native function that is callable both as an ordinary BIF and
    /// inside QoQ SQL (`SELECT myFn(col) FROM q`). Scalar functions receive
    /// per-row values; aggregate functions receive each argument as a
    /// `CfmlValue::Array` over the partition.
    pub fn register_native_qoq_fn(&mut self, name: &str, f: QoQFn, kind: QoQFnKind) {
        self.register_native_fn(name, f);
        self.qoq_registry.register_native(name, f, kind);
    }

    /// Execute a Query-of-Queries (`dbtype="query"`) request: parse the SQL,
    /// resolve its source query variables from scope, run the engine, and apply
    /// the requested `returntype`.
    fn execute_qoq(
        &mut self,
        sql: &str,
        params_arg: &CfmlValue,
        return_type: &str,
        column_key: Option<String>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let stmt = cfml_qoq::parse(sql)
            .map_err(|e| CfmlError::runtime(format!("Query of Queries syntax error: {}", e)))?;

        // Resolve each referenced query variable from scope (owned clones, so
        // the borrow can't collide with the &mut self UDF callback below).
        let names = cfml_qoq::base_table_names(&stmt);
        let owned: Vec<(String, cfml_common::dynamic::CfmlQuery)> = names
            .iter()
            .filter_map(|n| self.find_query_in_scope(n, parent_locals).map(|q| (n.clone(), q)))
            .collect();
        let sources: Vec<(String, &cfml_common::dynamic::CfmlQuery)> =
            owned.iter().map(|(n, q)| (n.clone(), q)).collect();

        let params = build_qoq_params(params_arg);

        // The engine may invoke CFML UDFs; `call_function` needs `&mut self`, so
        // take the registry out to avoid borrowing self both ways, then restore.
        let registry = std::mem::take(&mut self.qoq_registry);
        let result = {
            let mut udf = |func: &CfmlValue, args: Vec<CfmlValue>| {
                self.call_function(func, args, parent_locals)
            };
            cfml_qoq::execute(&stmt, &sources, &params, &registry, &mut udf)
        };
        self.qoq_registry = registry;

        let query_val = result?;
        Ok(convert_query_return(query_val, return_type, column_key.as_deref()))
    }

    /// Find a `CfmlValue::Query` bound to `name` in the active scope chain
    /// (local/arguments → variables → request → application), returning an owned
    /// clone of the query.
    fn find_query_in_scope(
        &self,
        name: &str,
        parent_locals: &ValueMap,
    ) -> Option<cfml_common::dynamic::CfmlQuery> {
        let lower = name.to_lowercase();
        // Cloning a query handle shares the Arc (cheap) — the QoQ engine only
        // reads the source tables, so sharing is correct and avoids a deep copy.
        if let Some(CfmlValue::Query(q)) = parent_locals
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
            .map(|(_, v)| v)
        {
            return Some(q.clone());
        }
        if let Some(CfmlValue::Query(q)) = self
            .globals
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
            .map(|(_, v)| v)
        {
            return Some(q.clone());
        }
        if let Some(CfmlValue::Query(q)) = self.request_scope.get_ci(&lower) {
            return Some(q);
        }
        if let Some(app) = self.application_scope.as_ref() {
            if let Some(CfmlValue::Query(q)) = app.get_ci(&lower) {
                return Some(q);
            }
        }
        None
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
                let mut entry = ValueMap::default();
                entry.insert(
                    "template".to_string(),
                    CfmlValue::string(f.template.clone()),
                );
                entry.insert("line".to_string(), CfmlValue::Int(f.line as i64));
                entry.insert("id".to_string(), CfmlValue::string("CFML".to_string()));
                entry.insert(
                    "raw_trace".to_string(),
                    CfmlValue::string(format!("at {}({}:{})", f.function, f.template, f.line)),
                );
                entry.insert("column".to_string(), CfmlValue::Int(0));
                CfmlValue::strukt(entry)
            })
            .collect();
        CfmlValue::array(context)
    }

    fn build_error_struct(e: &CfmlError, tag_context: CfmlValue) -> CfmlValue {
        let mut err_struct = ValueMap::default();
        err_struct.insert("message".to_string(), CfmlValue::string(e.message.clone()));
        err_struct.insert(
            "type".to_string(),
            CfmlValue::string(format!("{}", e.error_type)),
        );
        err_struct.insert("detail".to_string(), CfmlValue::string(String::new()));
        err_struct.insert("tagcontext".to_string(), tag_context);
        Self::add_root_cause(&mut err_struct);
        CfmlValue::strukt(err_struct)
    }

    /// Populate a `rootCause` key on an exception struct (Lucee/ACF parity): a
    /// snapshot of the error's own core fields. Frameworks inspect
    /// `exception.rootCause.type` to classify errors — Wheels' `onError` uses it
    /// to render its styled error/404 page (GitHub issue #158). The copy
    /// deliberately omits its own `rootCause`, so cfdump/serializeJSON over an
    /// exception never recurse. A pre-existing `rootCause` (e.g. carried by an
    /// object re-thrown via `throw(object=…)`) is left untouched.
    fn add_root_cause(exc: &mut ValueMap) {
        if exc.keys().any(|k| k.eq_ignore_ascii_case("rootcause")) {
            return;
        }
        let mut rc = ValueMap::default();
        for key in [
            "type",
            "message",
            "detail",
            "extendedinfo",
            "errorcode",
            "tagcontext",
        ] {
            if let Some((k, v)) = exc.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                rc.insert(k.clone(), v.clone());
            }
        }
        exc.insert("rootCause".to_string(), CfmlValue::strukt(rc));
    }

    /// Does a `catch (catch_type e)` clause match an exception whose `type` is
    /// `exc_type`? Mirrors Lucee/ACF: `any` (or an empty/missing type) catches
    /// everything; otherwise a case-insensitive exact match, or a dotted-type
    /// hierarchy match where catching a parent type (`Foo`) also catches its
    /// subtypes (`Foo.Bar`). Custom exception types are dotted strings, so this
    /// keeps `catch (Foo.InvalidConfiguration)` and `catch (Foo)` both working.
    fn catch_type_matches(catch_type: &str, exc_type: &str) -> bool {
        let ct = catch_type.trim();
        if ct.is_empty() || ct.eq_ignore_ascii_case("any") {
            return true;
        }
        if ct.eq_ignore_ascii_case(exc_type) {
            return true;
        }
        // Hierarchy: catch "Foo" also catches "Foo.Bar" (but not "Foobar").
        let ct_l = ct.to_lowercase();
        let exc_l = exc_type.to_lowercase();
        exc_l.len() > ct_l.len()
            && exc_l.starts_with(&ct_l)
            && exc_l.as_bytes()[ct_l.len()] == b'.'
    }

    // If `last_exception` already holds a struct whose `message` matches
    // `e.message`, reuse it (inner throw preserved detail); otherwise build a
    // fresh error struct. Avoids cloning the whole exception just to compare
    // a message string.
    fn resolve_catch_error_val(&mut self, e: &CfmlError) -> CfmlValue {
        let matched = matches!(
            self.last_exception.as_ref(),
            Some(CfmlValue::Struct(s))
                if matches!(s.get("message"), Some(CfmlValue::String(msg)) if msg.as_str() == e.message)
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

    /// Invoke `Application.cfc::onError(exception, eventName)` for an uncaught
    /// runtime error and translate the outcome into the request's final result.
    ///
    /// Lucee/ACF parity: when an uncaught exception escapes the target page (or
    /// `onRequest`/`onRequestStart`), the engine calls `onError` with the
    /// exception struct and the name of the event that was running ("" for the
    /// target page itself). If `onError` exists and returns normally, it owns the
    /// response — the request ends 200 with whatever `onError` wrote, and the
    /// engine's default error page is suppressed.
    ///
    /// - `Ok(Null)`   → `onError` handled it (existed and returned), OR `onError`
    ///   itself aborted/redirected (control-flow sentinel).
    /// - `Err(orig)`  → no `onError` defined: surface the original error so the
    ///   default error page renders.
    /// - `Err(other)` → `onError` itself threw: surface that error instead.
    fn run_on_error(
        &mut self,
        template: &mut CfmlValue,
        e: CfmlError,
        event_name: &str,
    ) -> CfmlResult {
        let exc = self.resolve_catch_error_val(&e);
        match self.call_lifecycle_method(
            template,
            "onError",
            vec![exc, CfmlValue::string(event_name.to_string())],
        ) {
            Ok(true) => Ok(CfmlValue::Null),
            Ok(false) => Err(e),
            Err(oe) if Self::is_control_flow_error(&oe) => Ok(CfmlValue::Null),
            Err(oe) => Err(oe),
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
                // get_ci does exact-then-CI under one read lock, cloning only the
                // matched value — replaces the old get/get(upper)/get(lower) chain
                // plus a full `s.iter()` snapshot (whole-IndexMap clone) per miss.
                let val = s.get_ci(name).or_else(|| {
                    if let Some(CfmlValue::Struct(vars)) = s.get("__variables") {
                        vars.get_ci(name)
                    } else {
                        None
                    }
                });
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
                    CfmlValue::Int(q.row_count() as i64)
                } else if name.eq_ignore_ascii_case("columnlist") {
                    // columnList reports column names uppercased, matching Lucee/ACF.
                    CfmlValue::string(q.column_list())
                } else if let Some(col_data) = q.column_values_ci(name) {
                    // QueryColumn proxy: acts as Array for indexing/iteration
                    // but stringifies to first row (Lucee parity). Shares the
                    // column's Arc directly with the source query (zero-copy).
                    CfmlValue::QueryColumn(col_data)
                } else {
                    CfmlValue::Null
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

    /// Swap `new_program` into `self.program`, registering its functions into
    /// `fn_registry` by global_id (so they resolve regardless of which program
    /// is active), and return the displaced program for the caller to restore.
    /// Pair every call with exactly one `pop_program_swap`. Because function
    /// identity is the program-independent global_id, the VM no longer needs to
    /// track a stack of displaced programs to resolve `DefineFunction` ops —
    /// that was the issue #70 workaround, now obsolete by construction.
    fn push_program_swap(&mut self, new_program: BytecodeProgram) -> BytecodeProgram {
        self.register_program_fns(&new_program);
        std::mem::replace(&mut self.program, new_program)
    }

    /// Undo the most recent `push_program_swap`, restoring `restored` as the
    /// active program.
    fn pop_program_swap(&mut self, restored: BytecodeProgram) {
        self.program = restored;
    }

    /// Restore output-capture state when an exception is caught by `handler`.
    /// A cfsavecontent / cfsilent / custom-tag body that throws before its
    /// matching end op leaves `saved_output_buffers` (and `custom_tag_stack`)
    /// unbalanced and `output_buffer` pointing at the abandoned body buffer.
    /// Popping back to the depths recorded at try-start discards the partial
    /// body output and points `output_buffer` at the buffer that was active
    /// when the try region was entered. A no-op for the common case (no
    /// buffers pushed inside the try region).
    fn restore_capture_state(&mut self, handler: &TryHandler) {
        while self.saved_output_buffers.len() > handler.saved_buffers_depth {
            self.output_buffer = self.saved_output_buffers.pop().unwrap_or_default();
        }
        if self.custom_tag_stack.len() > handler.custom_tag_depth {
            self.custom_tag_stack.truncate(handler.custom_tag_depth);
        }
    }

    fn execute_function_with_args(
        &mut self,
        func: &BytecodeFunction,
        args: Vec<CfmlValue>,
        parent_scope: Option<&ValueMap>,
    ) -> CfmlResult {
        // Tier-1 JIT fast path. Returns `Some` only when a compiled native body
        // ran to completion for these exact (all-Int) arguments; otherwise this
        // falls through to the interpreter unchanged. `func`/`args` are the
        // caller's, not borrowed from `self.jit`, so there is no borrow conflict;
        // with the feature off the whole block compiles away. See `jit/mod.rs`.
        #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
        {
            // The shadowing guard makes sure a user-defined function or
            // global with the same name as an allowlisted builtin (e.g.
            // `function abs(x) { … }`) wins over the JIT's native call. We
            // peek directly at the VM's lookup tables; matches the
            // case-insensitive lookup the interpreter does in LoadGlobal.
            // Borrowing `self.jit` and `self.user_functions` / `self.globals`
            // simultaneously is fine via split borrows on the field pattern,
            // but to keep it simple we snapshot the field references first.
            let user_functions = &self.user_functions;
            let globals = &self.globals;
            if let Some(engine) = self.jit.as_mut() {
                // A canonical builtin entry in `globals` is the no-op wrapper
                // `Function{ body: Expression(Null), params: [], captured_scope: None }`
                // produced by `cfml_stdlib::create_builtin_func`. Anything else
                // for an allowlist name (a user-assigned value, a redefined
                // function, etc.) is real shadowing — we must bail so the
                // interpreter resolves the user's version through LoadGlobal's
                // normal lookup order. User-defined `function abs(){}` lives in
                // `user_functions` *behind* the globals entry in CFML's lookup,
                // so by itself it doesn't shadow — but we still bail on it as
                // a conservative second guard: the user clearly intended an
                // override, and the analysis is cheap.
                let is_canonical_builtin_wrapper = |v: &CfmlValue| -> bool {
                    if let CfmlValue::Function(f) = v {
                        if !f.params.is_empty() || f.captured_scope.is_some() {
                            return false;
                        }
                        let body: &cfml_common::dynamic::CfmlClosureBody = &f.body;
                        return matches!(
                            body,
                            cfml_common::dynamic::CfmlClosureBody::Expression(b) if matches!(b.as_ref(), CfmlValue::Null)
                        );
                    }
                    false
                };
                let mut is_shadowed = |name: &str| -> bool {
                    let lower = name.to_ascii_lowercase();
                    let ufn_hit = user_functions.contains_key(name)
                        || user_functions.keys().any(|k| k.eq_ignore_ascii_case(&lower));
                    if ufn_hit {
                        return true;
                    }
                    let g = globals.get(name).or_else(|| {
                        globals
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
                            .map(|(_, v)| v)
                    });
                    match g {
                        Some(v) => !is_canonical_builtin_wrapper(v),
                        None => false,
                    }
                };
                // UDF resolver: case-insensitive lookup against the live
                // `user_functions` map, returning (global_id, arity). Used
                // by the JIT analyser to bind `LoadGlobal(name)` calls to
                // already-compiled UDFs in the cache; rejecting the
                // caller's analysis when the callee isn't yet warm.
                let udf_lookup = |name: &str| -> Option<jit::UdfMeta> {
                    let lower = name.to_ascii_lowercase();
                    let f = user_functions.get(name).or_else(|| {
                        user_functions
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
                            .map(|(_, v)| v)
                    })?;
                    Some(jit::UdfMeta {
                        global_id: f.global_id,
                        nparams: f.params.len(),
                    })
                };
                if let Some(result) = engine.try_call(func, &args, &mut is_shadowed, &udf_lookup) {
                    return result;
                }
            }
        }

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

        let mut locals: ValueMap = ValueMap::default();
        let mut stack: Vec<CfmlValue> = Vec::new();
        let mut ip = 0;
        // Track variables declared with `var` (function-local, not written back to parent)
        let mut declared_locals: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Shared closure environment: all closures defined within this function
        // invocation share one Rc<RefCell<HashMap>>. Lazily created on first DefineFunction.
        let mut closure_env: Option<Arc<RwLock<ValueMap>>> = None;

        // Effective Lucee localMode for this frame: function's declared attribute
        // wins; otherwise inherits the application default (`this.localMode` in
        // Application.cfc); otherwise classic.
        let effective_local_mode_modern: bool =
            func.declared_local_mode.unwrap_or(self.app_local_mode_modern);

        // A component pseudo-constructor (`__cfc_body__`) or page template
        // (`__main__`) frame: its locals are the component's `variables` scope,
        // captured via `captured_locals`, and must never leak out as a closure
        // parent-scope writeback (see the two write-back sites below).
        let is_template_frame = func.name == "__cfc_body__" || func.name == "__main__";

        // Copy parent scope variables (closures and nested functions see parent vars).
        // Skip Function values — they're immutable and already available via
        // user_functions, so cloning them (and their captured scopes) is pure waste.
        //
        // PR #93: track which keys were *inherited* from the parent so the
        // per-call `local` scope view (LoadLocal "local" etc.) can exclude
        // them. The caller's vars must NOT be visible through this frame's
        // `local` — `local` is strictly per-call. Anything in `locals` that
        // isn't an inherited key (and isn't a function parameter — those
        // belong to `arguments`, not `local`) was established in this frame
        // and is part of `local`.
        let mut inherited_or_param_keys: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        if let Some(parent) = parent_scope {
            for (k, v) in parent {
                if !matches!(v, CfmlValue::Function(_)) {
                    locals.insert(k.clone(), v.clone());
                    inherited_or_param_keys.insert(k.clone());
                }
            }
        }

        // Build CFML arguments scope.
        //
        // A declared param that the caller omits is only materialized when it
        // has a default value: the default-value preamble (emitted by codegen)
        // seeds the local + the `arguments` key at runtime. An omitted param
        // with no default must stay ABSENT from both `locals` and `arguments`,
        // so `structKeyExists(arguments, "p")` is false — matching Lucee/ACF.
        let mut arguments_map: ValueMap = ValueMap::default();
        // Lucee/ACF/BoxLang: a value bound to a declared parameter appears in
        // the `arguments` scope under its parameter NAME — never under its
        // 1-based positional index. Positional access (`arguments[1]`) still
        // works via the `__arguments_scope` marker handled in GetIndex below.
        // Overflow positional args (paramless fn called positionally, or
        // extras beyond declared params with no matching name) DO get numeric
        // keys — that's the only handle they have.
        for (i, param_name) in func.params.iter().enumerate() {
            inherited_or_param_keys.insert(param_name.clone());
            let has_default = func.has_default.get(i).copied().unwrap_or(false);
            // A Null arg value counts as "not supplied": CFML has no way to pass
            // an explicit null, and the named-argument rebinder pads omitted
            // slots with Null to keep later named args at the right index.
            let supplied = match args.get(i) {
                Some(v) if !matches!(v, CfmlValue::Null) => Some(v.clone()),
                _ => None,
            };
            match supplied {
                Some(value) => {
                    locals.insert(param_name.clone(), value.clone());
                    arguments_map.insert(param_name.clone(), value);
                }
                None => {
                    // Omitted. Pre-seed Null only so the default preamble's
                    // LoadLocal/IsNull check works; it then fills the real
                    // default into both the local and the arguments key. A param
                    // with no default stays absent from `arguments` entirely.
                    if has_default {
                        locals.insert(param_name.clone(), CfmlValue::Null);
                    }
                }
            }
        }
        // Extras: named args from the callsite that didn't match a declared
        // param. Take the pending list (set by the named-args reorder) so a
        // recursive call doesn't see stale state.
        let extras = self.pending_extra_named_args.take().unwrap_or_default();
        // Add any args beyond declared params. A named overflow arg is keyed by
        // name ONLY; a positional overflow arg is keyed by its 1-based position.
        // (Lucee/ACF/BoxLang: calling a paramless function purely by name yields
        // an arguments scope of exactly those names — no spurious numeric keys.)
        for i in func.params.len()..args.len() {
            let value = args[i].clone();
            if let Some((_, name)) = extras.iter().find(|(idx, _)| *idx == i) {
                arguments_map.insert(name.clone(), value);
            } else {
                arguments_map.insert((i + 1).to_string(), value);
            }
        }
        // Check required parameters. A param declared `required` BUT also given
        // a default is NOT enforced as required when the arg is omitted — the
        // default satisfies it (Lucee/ACF/BoxLang treat the contradictory
        // `required` as effectively ignored; TestBox's TestResult.cfc relies on
        // `required count = 1`). The default-application preamble then fills it.
        for (i, param_name) in func.params.iter().enumerate() {
            let has_default = func.has_default.get(i).copied().unwrap_or(false);
            if func.required_params.get(i).copied().unwrap_or(false)
                && args.get(i).is_none()
                && !has_default
            {
                return Err(self.wrap_error(CfmlError::runtime(format!(
                    "The parameter [{}] to function [{}] is required but was not passed in.",
                    param_name, func.name
                ))));
            }
        }
        // Tag this struct as the arguments scope. `__arguments_scope` is the
        // sentinel; `__arguments_params` carries the declared param names so
        // `arguments[N]` (1-based) can fall through to params[N-1] at the
        // GetIndex site without having to thread the param list separately.
        // Both markers are filtered from user-visible struct introspection.
        arguments_map.insert("__arguments_scope".to_string(), CfmlValue::Bool(true));
        let params_array: Vec<CfmlValue> = func
            .params
            .iter()
            .map(|p| CfmlValue::string(p.clone()))
            .collect();
        arguments_map.insert(
            "__arguments_params".to_string(),
            CfmlValue::array(params_array),
        );
        // A `cfinclude`d template (the included file's `__main__`, run with the
        // caller's scope as parent) shares the CALLER's `arguments` scope in
        // CFML — e.g. Wheels renders error/view templates with
        // `$includeAndReturnOutput($template=…, wheelsError=…)` and the template
        // reads `arguments.wheelsError`. The included `__main__` has no params of
        // its own, so without this its `arguments` would be empty and every such
        // reference would silently render blank (GitHub issue #158). Seed this
        // frame's `arguments` from the parent's so those entries are visible;
        // our freshly-built markers/params take precedence.
        if func.name == "__main__" {
            if let Some(parent) = parent_scope {
                if let Some(CfmlValue::Struct(parent_args)) = parent.get(ARGUMENTS_SCOPE_KEY) {
                    for (k, v) in parent_args.snapshot() {
                        if !arguments_map.contains_key(&k) {
                            arguments_map.insert(k, v);
                        }
                    }
                }
            }
        }
        locals.insert(
            ARGUMENTS_SCOPE_KEY.to_string(),
            CfmlValue::strukt(arguments_map),
        );

        // The name this call was dispatched under (a member-call alias, if any).
        // Always drained so it can't leak into a later call; falls back to the
        // function's declared name for plain named calls.
        let called_name = self
            .pending_called_name
            .take()
            .unwrap_or_else(|| func.name.clone());

        // Push call frame for stack trace tracking (skip __main__ — it's the root)
        if func.name != "__main__" {
            self.call_stack.push(CallFrame {
                function_name: func.name.clone(),
                called_name,
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
            // `cfexit` unwind: a pending exit signal terminates the current
            // function as if it had run off its end (the epilogue below captures
            // partial locals). The signal keeps propagating up the call stack
            // until the owning template frame (`__main__`/`__cfc_body__`) moves
            // it into `last_exit_method`.
            if self.pending_exit.is_some() {
                break;
            }
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
                BytecodeOp::String(s) => stack.push(CfmlValue::string(s.clone())),

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
                        // `local` is strictly per-call (PR #93): only keys
                        // established in THIS frame are visible — inherited
                        // parent vars (page `variables`, CFC bridge keys) and
                        // function params are excluded.
                        CfmlValue::strukt(Self::build_local_scope_view(
                            &locals,
                            &inherited_or_param_keys,
                        ))
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
                        CfmlValue::strukt(self.request_scope.snapshot())
                    } else if name_lower == "application" {
                        if let Some(ref app_scope) = self.application_scope {
                            // Live handle clone, not a snapshot, so `var p =
                            // application; p.x = 1` writes through (Lucee semantics).
                            CfmlValue::Struct(app_scope.clone())
                        } else {
                            CfmlValue::strukt(ValueMap::default())
                        }
                    } else if name_lower == "session" {
                        // Attach the live session scope before returning it, so a
                        // bare `session` READ hands back the same Arc-backed
                        // handle on every read (Lucee scope-reference semantics).
                        // Reads previously returned a detached snapshot/empty
                        // struct until a write attached the scope, which broke
                        // the read-first scope-pointer caching pattern.
                        self.attach_session_scope();
                        self.get_session_scope()
                    } else if name_lower == "cookie" {
                        self.globals
                            .get("cookie")
                            .cloned()
                            .unwrap_or(CfmlValue::strukt(ValueMap::default()))
                    } else if name_lower == "server" {
                        let mut scope = build_server_scope();
                        // Prefer the per-request application-level cfconfig overlay
                        // (a `.cfconfig.json` beside the Application.cfc) over the
                        // server baseline when present.
                        if let Some(ref app_cfg) = self.app_cfconfig {
                            scope.insert(
                                "cfconfig".to_string(),
                                cfconfig_to_cfml(app_cfg),
                            );
                        } else if let Some(ref ss) = self.server_state {
                            scope.insert(
                                "cfconfig".to_string(),
                                cfconfig_to_cfml(&ss.cfconfig),
                            );
                        }
                        CfmlValue::strukt(scope)
                    } else if let Some(val) =
                        self.lookup_name_in_scopes(name.as_str(), name_lower, &locals)
                    {
                        // Bug #9: a CFC method retrieved by bare name in value
                        // position (e.g. `.each( processPropertyMetadata )`) is
                        // stored in `__variables` with `captured_scope = None`
                        // (stripped at CFC-body frame exit for cycle safety).
                        // When the function is later invoked via a higher-order
                        // BIF (.each/.map/.filter), the receiver passes an
                        // empty parent_locals — so the callee's `this` and
                        // `variables` resolutions both fail.
                        //
                        // Bind here at the load site: if we're inside a CFC
                        // method context (frame carries `this` or `__variables`)
                        // and the loaded value is an unbound CfmlFunction,
                        // clone it and stash the relevant scopes into a fresh
                        // captured_scope. `call_function` already merges this
                        // captured_scope with parent_locals (lib.rs:4578) — so
                        // a `.each(...)` call site that previously dropped
                        // `this` will now see it via the function's binding.
                        if let CfmlValue::Function(ref f) = val {
                            if f.captured_scope.is_none()
                                && (locals.contains_key("this")
                                    || locals.contains_key("__variables"))
                            {
                                let mut bound: ValueMap =
                                    ValueMap::default();
                                for key in [
                                    "this",
                                    "__variables",
                                    "variables",
                                    "super",
                                ] {
                                    if let Some(v) = locals.get(key) {
                                        bound.insert(key.to_string(), v.clone());
                                    }
                                }
                                let mut bound_fn = (**f).clone();
                                bound_fn.captured_scope =
                                    Some(Arc::new(std::sync::RwLock::new(bound)));
                                CfmlValue::Function(Arc::new(bound_fn))
                            } else {
                                val
                            }
                        } else {
                            val
                        }
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
                        let filtered: ValueMap = locals
                            .iter()
                            .filter(|(_, v)| !matches!(v, CfmlValue::Function(_)))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        let scope = Arc::new(RwLock::new(filtered));
                        CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
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
                                CfmlValue::Int(bc_func.global_id as i64),
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: Some(scope),
                        }))
                    } else {
                        // Variable not found — check try_stack for error handler
                        if let Some(handler) = self.try_stack.pop() {
                            let mut exception = ValueMap::default();
                            exception.insert(
                                "message".to_string(),
                                CfmlValue::string(format!("Variable '{}' is undefined", name)),
                            );
                            exception.insert(
                                "type".to_string(),
                                CfmlValue::string("expression".to_string()),
                            );
                            exception
                                .insert("detail".to_string(), CfmlValue::string(String::new()));
                            exception.insert("tagcontext".to_string(), self.build_tag_context());
                            stack.truncate(handler.stack_depth);
                            self.restore_capture_state(&handler);
                            Self::add_root_cause(&mut exception);
                            let exc = CfmlValue::strukt(exception);
                            self.last_exception = Some(exc.clone());
                            // The catch handler begins with StoreLocal(catch_var),
                            // which pops the in-flight error from the stack.
                            stack.push(exc);
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
                    let val = if name_lower == "local" {
                        // PR #93: per-frame `local` — only keys established here.
                        CfmlValue::strukt(Self::build_local_scope_view(
                            &locals,
                            &inherited_or_param_keys,
                        ))
                    } else if name_lower == "variables" {
                        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                            CfmlValue::Struct(vars.clone())
                        } else {
                            CfmlValue::strukt(locals.clone())
                        }
                    } else if name_lower == "request" {
                        CfmlValue::strukt(self.request_scope.snapshot())
                    } else if name_lower == "application" {
                        if let Some(ref app_scope) = self.application_scope {
                            // Live handle clone (see LoadLocal), not a snapshot.
                            CfmlValue::Struct(app_scope.clone())
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
                    // PR #93: a `var x` / `local.x` declaration RECLAIMS the
                    // name into THIS frame's `local` scope, shadowing any
                    // same-named key inherited from the caller. Removing it
                    // from `inherited_or_param_keys` makes the subsequent
                    // `local.x` reads return this frame's value.
                    inherited_or_param_keys.remove(name);
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
                                self.request_scope.with_write(|m| *m = s.snapshot());
                            }
                        } else if name_lower == "application" {
                            if let CfmlValue::Struct(s) = &val {
                                if let Some(ref app_scope) = self.application_scope {
                                    // Same self-alias guard as request above.
                                    if s.backing_ptr() != app_scope.backing_ptr() {
                                        let snap = s.snapshot();
                                        app_scope.with_write(|m| *m = snap);
                                    }
                                }
                            }
                        } else if name_lower == "session" {
                            if let CfmlValue::Struct(s) = &val {
                                self.set_session_scope(s.snapshot())?;
                            }
                        } else if name_lower == "thread" && self.globals.contains_key("thread") {
                            self.globals.insert("thread".to_string(), val);
                        } else if name_lower == "arguments"
                            && is_inside_function
                            && !declared_locals.contains(name.as_str())
                            && !declared_locals.contains(&name_lower)
                        {
                            // Storing the `arguments` SCOPE (bare `arguments = …`
                            // or an `arguments.x = …` round-trip). A user
                            // `local.arguments = …` emits DeclareLocal first, so
                            // it lands in `declared_locals` and falls through to
                            // the plain locals-insert branch (literal "arguments"
                            // key) instead — keeping the two scopes independent.
                            //
                            // When the arguments scope is stored, sync complex-type
                            // params back to their named locals so that modifications
                            // via `arguments.param.prop = val` are visible to the
                            // pass-by-reference writeback mechanism.
                            if let CfmlValue::Struct(ref args) = val {
                                for (k, v) in args.iter() {
                                    // Never sync internal markers (__variables, __name,
                                    // this, super) from the arguments scope back into the
                                    // frame's locals: they are not real parameters, and a
                                    // stray __variables (e.g. one injected onto the
                                    // arguments scope by a deep variables-writeback such as
                                    // `arguments.cfc.getName()`) would otherwise clobber the
                                    // frame's real component scope, nulling out
                                    // `variables.*`. See tests/oop/test_mixin_writeback.cfm.
                                    if k.starts_with("__")
                                        || k.eq_ignore_ascii_case("this")
                                        || k.eq_ignore_ascii_case("super")
                                    {
                                        continue;
                                    }
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
                            // The arguments scope lives under the reserved key, not
                            // the literal "arguments" (which is a user local var).
                            locals.insert(ARGUMENTS_SCOPE_KEY.to_string(), val);
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
                                locals.get_mut("__variables").and_then(|v| v.as_cfml_struct())
                            {
                                vars.insert(name.clone(), val);
                            }
                        } else {
                            locals.insert(name.clone(), val.clone());
                            // PR #93: in modern localmode, a bare assignment IS a
                            // local-scope assignment — it claims the key for this
                            // frame's `local` view, shadowing any inherited
                            // same-named key from the caller / closure parent.
                            if effective_local_mode_modern {
                                inherited_or_param_keys.remove(name);
                            }
                            // Bidirectional sync: when a function param is stored,
                            // also update arguments[param] so `arguments.x` sees
                            // the latest value (and vice versa, handled above for arguments).
                            // PR #96: NOT for `var X` / `local.X` declarations — those
                            // create a separate local-scope slot; `arguments.X` must keep
                            // resolving to the passed value / declared default.
                            if is_inside_function
                                && !declared_locals.contains(name.as_str())
                                && !declared_locals.contains(&name_lower)
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
                                    locals.get_mut(ARGUMENTS_SCOPE_KEY).and_then(|v| v.as_cfml_struct())
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
                BytecodeOp::SetDynamicVar => {
                    // Dynamic/quoted-string LHS assignment: the path string was
                    // resolved at runtime (e.g. "variables.propDep" from
                    // `"#scope#.#prop#" = v`). Store scope-aware into the current
                    // frame so `variables.x` lands in a CFC's __variables (not the
                    // page scope) — matching a normal `variables.x = v` assignment.
                    let value = stack.pop().unwrap_or(CfmlValue::Null);
                    let path = stack
                        .pop()
                        .map(|v| v.as_string())
                        .unwrap_or_default();
                    self.store_runtime_path(&path, value.clone(), &mut locals);
                    stack.push(value);
                }
                BytecodeOp::UnsetPath(path) => {
                    // CFML null-assignment: `x = voidFn()` (Null RHS) must DELETE
                    // the target rather than materialize a null-valued key. The
                    // value (Null) was already popped by the guard's `Pop`.
                    self.delete_scope_path(path, &mut locals);
                    // Drop any closure-env copy so sibling closures see the
                    // deletion too (mirrors StoreLocal's env sync).
                    if let Some(ref env) = closure_env {
                        let mut m = env.write().unwrap();
                        let found = m.keys().find(|k| k.eq_ignore_ascii_case(path)).cloned();
                        if let Some(k) = found {
                            m.shift_remove(&k);
                        }
                    }
                    // A `var x` / `local.x` that resolved to Null leaves no key,
                    // but the name was still claimed for this frame's `local`
                    // view — keep it shadowing any inherited caller key.
                    let leaf = path.rsplit('.').next().unwrap_or(path.as_str());
                    inherited_or_param_keys.remove(leaf);
                }
                BytecodeOp::ArrayAppendLocal(name) => {
                    // Fused arrayAppend(<ident>, value). The value is on top of
                    // the stack; the array lives in the named variable. With
                    // reference-typed arrays the variable holds a shared handle,
                    // so pushing in place is O(1) AND visible to every alias —
                    // no clone, no store-back, no env sync needed. (Pre-reference
                    // this had to fight Arc copy-on-write; that's all gone now.)
                    let value = stack.pop().unwrap_or(CfmlValue::Null);

                    // Fast path: array held directly in this frame's locals.
                    if let Some(CfmlValue::Array(arr)) = locals.get(name.as_str()) {
                        arr.push(value);
                        continue;
                    }

                    // Resolve through the full scope chain; the returned handle
                    // shares the backing with the scope slot, so a push is seen
                    // by the original (globals/__variables/case-insensitive).
                    let name_lower_owned: String;
                    let name_lower: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
                        name_lower_owned = name.to_lowercase();
                        &name_lower_owned
                    } else {
                        name.as_str()
                    };
                    if let Some(CfmlValue::Array(arr)) =
                        self.lookup_name_in_scopes(name.as_str(), name_lower, &locals)
                    {
                        arr.push(value);
                        continue;
                    }

                    // Not found (or not an array): create a fresh single-element
                    // array and store it in the correct scope, mirroring how
                    // StoreLocal routes a plain identifier.
                    let val = CfmlValue::array(vec![value]);
                    if locals.contains_key("__variables")
                        && !declared_locals.contains(name.as_str())
                        && !declared_locals.contains(name_lower)
                        && !locals.contains_key(name.as_str())
                        && !effective_local_mode_modern
                    {
                        // CFC method, classic localmode: component (variables) scope.
                        if let Some(vars) =
                            locals.get_mut("__variables").and_then(|v| v.as_cfml_struct())
                        {
                            vars.insert(name.clone(), val);
                        }
                    } else {
                        locals.insert(name.clone(), val.clone());
                        if is_inside_function
                            && !declared_locals.contains(name.as_str())
                            && !declared_locals.contains(name_lower)
                            && func.params.iter().any(|p| p.eq_ignore_ascii_case(name))
                        {
                            if let Some(args) =
                                locals.get_mut(ARGUMENTS_SCOPE_KEY).and_then(|v| v.as_cfml_struct())
                            {
                                args.insert(name.clone(), val.clone());
                            }
                        }
                        if let Some(ref env) = closure_env {
                            let mut m = env.write().unwrap();
                            if m.contains_key(name.as_str()) {
                                m.insert(name.clone(), val);
                            }
                        }
                    }
                }
                BytecodeOp::LoadGlobal(name) | BytecodeOp::LoadVariablesKey(name) => {
                    // Avoid allocating a lowercase String when the identifier is
                    // already all-lowercase ASCII (the common case for the most
                    // frequent read op). Unicode/mixed-case idents still get full
                    // case-folding. Mirrors the LoadLocal guard above.
                    let name_lower_owned: String;
                    let name_lower: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
                        name_lower_owned = name.to_lowercase();
                        &name_lower_owned
                    } else {
                        name.as_str()
                    };
                    // Resolve from this frame's locals (exact, then CI), keeping the
                    // matched key so we can ask whether it was inherited.
                    let local_hit = locals
                        .get_key_value(name.as_str())
                        .or_else(|| locals.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name_lower)))
                        .map(|(k, v)| (k.clone(), v.clone()));
                    // PR #97: CFML is lexically scoped — a non-Function value that
                    // leaked in from an ANCESTOR frame (the parent-scope copy above)
                    // must stay invisible to bare-name call resolution; only data
                    // established in THIS frame (params, `var`s, bare writes) may
                    // shadow a same-named function. Builtin names are immune to
                    // data shadowing entirely (Lucee binds BIFs at compile time:
                    // `function f(struct lcase={}) { lcase("X") }` calls the BIF).
                    // When the hit is skipped here and nothing else resolves, the
                    // final else pushes the data back so plain reads keep working.
                    //
                    // LoadVariablesKey is the page-scope `variables.foo` READ
                    // peephole: same resolution chain, but a data hit is always
                    // visible (`variables.log` must return the variable, not the
                    // log() builtin).
                    let is_read_position = matches!(op, BytecodeOp::LoadVariablesKey(_));
                    let local_hit_visible = match &local_hit {
                        None => false,
                        _ if is_read_position => true,
                        Some((_, CfmlValue::Function(_))) => true,
                        Some((k, _)) => {
                            let is_own_param =
                                func.params.iter().any(|p| p.eq_ignore_ascii_case(k));
                            let inherited_data =
                                inherited_or_param_keys.contains(k.as_str()) && !is_own_param;
                            let is_builtin_name = self.builtins.contains_key(name.as_str())
                                || self
                                    .builtins
                                    .keys()
                                    .any(|b| b.eq_ignore_ascii_case(&name_lower));
                            !inherited_data && !is_builtin_name
                        }
                    };
                    // 1. Check locals (exact, then CI)
                    if local_hit_visible {
                        // Move the already-cloned value out of `local_hit` rather
                        // than cloning it a second time — this is the hottest read
                        // path. `local_hit` is only consumed again in the
                        // mutually-exclusive "restore the data" else-if below.
                        stack.push(local_hit.unwrap().1);
                    // 1b. Check __variables scope for CFC methods
                    } else if let Some(val) = locals.get("__variables").and_then(|v| {
                        if let CfmlValue::Struct(vars) = v {
                            // get_ci does exact-then-CI under one read lock and
                            // clones only the matched value — never snapshots the
                            // whole __variables map (the old `vars.iter()` path
                            // cloned the entire IndexMap on every CI-fallback read,
                            // the single hottest clone site on the Wheels /posts path).
                            vars.get_ci(name.as_str())
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                        let (body_val, scope) = if let Some(uf) =
                            self.user_functions.get(name.as_str())
                        {
                            // Reference the function by its stable global_id.
                            (CfmlValue::Int(uf.global_id as i64), None)
                        } else {
                            (CfmlValue::Null, None)
                        };
                        stack.push(CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
                            name: name.clone(),
                            params,
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                body_val,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: scope,
                        })));
                    } else if self.builtins.keys().any(|k| k.eq_ignore_ascii_case(&name_lower))
                        || self
                            .user_functions
                            .keys()
                            .any(|k| k.eq_ignore_ascii_case(&name_lower))
                    {
                        let canonical = self
                            .builtins
                            .keys()
                            .find(|k| k.eq_ignore_ascii_case(&name_lower))
                            .or_else(|| {
                                self.user_functions
                                    .keys()
                                    .find(|k| k.eq_ignore_ascii_case(&name_lower))
                            })
                            .cloned()
                            .unwrap_or(name.clone());
                        let params = self
                            .user_functions
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                        // For user functions (CI match), reference by stable global_id.
                        let (body_val, scope, resolved_name) = if let Some((uf_name, uf)) = self
                            .user_functions
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
                        {
                            (
                                CfmlValue::Int(uf.global_id as i64),
                                None,
                                uf_name.clone(),
                            )
                        } else {
                            (CfmlValue::Null, None, canonical)
                        };
                        stack.push(CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
                            name: resolved_name,
                            params,
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                body_val,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: scope,
                        })));
                    // 4. Check VM-intercepted function names (custom tags, etc.)
                    } else if matches!(
                        name_lower,
                        "__cfcustomtag"
                            | "__cfcustomtag_start"
                            | "__cfcustomtag_end"
                            | "callstackget"
                            | "callstackdump"
                            | "precisionevaluate"
                    ) {
                        stack.push(CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
                            name: name.clone(),
                            params: Vec::new(),
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                CfmlValue::Null,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: None,
                        })));
                    } else if matches!(
                        name_lower,
                        "cfheader"
                            | "cfcontent"
                            | "cflocation"
                            | "location"
                            | "cfabort"
                            | "cfsetting"
                            | "cfcookie"
                            | "cflog"
                            | "cfinvoke"
                            | "cfhtmlhead"
                            | "cfhtmlbody"
                    ) {
                        // Script-style tag calls: `cfcontent(reset=true)` routes to `__cfcontent`.
                        // `location(...)` is the documented script alias for `cflocation(...)`
                        // (CFML BIF `Location(url, addToken, statusCode)`); both must reach the
                        // `__cflocation` VM intercept, else `location()` falls through to the
                        // builtin stub and 500s with "requires VM intercept".
                        let underscored = if name_lower == "location" {
                            "__cflocation".to_string()
                        } else {
                            format!("__{}", name_lower)
                        };
                        stack.push(CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
                            name: underscored,
                            params: Vec::new(),
                            body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(
                                CfmlValue::Null,
                            )),
                            return_type: None,
                            access: cfml_common::dynamic::CfmlAccess::Public,
                            captured_scope: None,
                        })));
                    } else if let Some((_, val)) = local_hit {
                        // The locals hit was skipped above (inherited data /
                        // builtin-named data) but no function resolved either:
                        // restore the data so plain reads behave as before and
                        // a call on it reports "Variable is not a function".
                        stack.push(val);
                    } else {
                        // Unresolved name in call/global position. Route through
                        // the active try handler if there is one: calling an
                        // undefined function must be catchable (the standard
                        // CFML feature-detection idiom relies on it).
                        if let Some(handler) = self.try_stack.pop() {
                            let mut exception = ValueMap::default();
                            exception.insert(
                                "message".to_string(),
                                CfmlValue::string(format!("Variable '{}' is undefined", name)),
                            );
                            exception.insert(
                                "type".to_string(),
                                CfmlValue::string("expression".to_string()),
                            );
                            exception
                                .insert("detail".to_string(), CfmlValue::string(String::new()));
                            exception.insert("tagcontext".to_string(), self.build_tag_context());
                            stack.truncate(handler.stack_depth);
                            self.restore_capture_state(&handler);
                            Self::add_root_cause(&mut exception);
                            let exc = CfmlValue::strukt(exception);
                            self.last_exception = Some(exc.clone());
                            stack.push(exc);
                            ip = handler.catch_ip;
                            continue;
                        }
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
                            CfmlValue::string(format!("{}{}", s, t))
                        }
                        // CFML: try numeric coercion
                        _ => {
                            let a_num = to_number(&a);
                            let b_num = to_number(&b);
                            match (a_num, b_num) {
                                (Some(x), Some(y)) => CfmlValue::Double(x + y),
                                _ => {
                                    CfmlValue::string(format!("{}{}", a.as_string(), b.as_string()))
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
                            let mut exception = ValueMap::default();
                            exception.insert(
                                "message".to_string(),
                                CfmlValue::string("Division by zero is not allowed.".to_string()),
                            );
                            exception.insert(
                                "type".to_string(),
                                CfmlValue::string("Expression".to_string()),
                            );
                            exception
                                .insert("detail".to_string(), CfmlValue::string(String::new()));
                            exception.insert("tagcontext".to_string(), self.build_tag_context());
                            Self::add_root_cause(&mut exception);
                            let error_val = CfmlValue::strukt(exception);
                            self.last_exception = Some(error_val.clone());
                            if let Some(handler) = self.try_stack.pop() {
                                while stack.len() > handler.stack_depth {
                                    stack.pop();
                                }
                                self.restore_capture_state(&handler);
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
                        CfmlValue::string(format!("{}{}", a.as_string(), b.as_string()))
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
                    // Cooperative cancellation checkpoint. A backward jump is a
                    // loop back-edge — the bounded place a terminated thread
                    // aborts. `cancel_flag` is `None` on the main VM, so this is
                    // a single predictable branch only on back-edges; the whole
                    // check vanishes when the `real-threads` feature is off.
                    #[cfg(feature = "real-threads")]
                    {
                        if *target <= ip {
                            if let Some(flag) = &self.cancel_flag {
                                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                                    return Err(CfmlError::runtime(
                                        "thread terminated".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    // OSR (Phase 2): back-edge from a while/until/repeat loop.
                    // `ip` was already incremented past this Jump, so `ip - 1`
                    // is the Jump's own ip and a strictly backward `target`
                    // means we're at the end of a loop body about to loop
                    // back. Region = `[target, ip)`. On success the compiled
                    // body writes back locals and we resume at `exit_ip`
                    // (== region_end_excl == ip), so the `continue` simply
                    // skips the `ip = *target` reassignment below. On bail
                    // we fall through and re-execute the body once more.
                    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
                    {
                        if *target < ip - 1 {
                            let user_functions = &self.user_functions;
                            let globals = &self.globals;
                            if let Some(engine) = self.jit.as_mut() {
                                let mut is_shadowed =
                                    |name: &str| jit_is_shadowed(user_functions, globals, name);
                                if let Some(exit_ip) = engine.try_run_loop(
                                    func,
                                    *target,
                                    ip,
                                    &mut locals,
                                    closure_env.as_ref(),
                                    &mut is_shadowed,
                                    &|name: &str| jit_udf_lookup(user_functions, name),
                                ) {
                                    ip = exit_ip;
                                    continue;
                                }
                            }
                        }
                    }
                    ip = *target;
                }
                BytecodeOp::JumpIfFalse(target) => {
                    if let Some(cond) = stack.pop() {
                        if !cond.is_true() {
                            // OSR Phase 2: a JumpIfFalse whose target is
                            // strictly behind this op is the back-edge of a
                            // `do { body } until (cond)` shape (i.e. loop
                            // back when cond is false). Same region/hook
                            // mechanics as the unconditional Jump case.
                            #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
                            {
                                if *target < ip - 1 {
                                    let user_functions = &self.user_functions;
                                    let globals = &self.globals;
                                    if let Some(engine) = self.jit.as_mut() {
                                        let mut is_shadowed = |name: &str| {
                                            jit_is_shadowed(user_functions, globals, name)
                                        };
                                        if let Some(exit_ip) = engine.try_run_loop(
                                            func,
                                            *target,
                                            ip,
                                            &mut locals,
                                            closure_env.as_ref(),
                                            &mut is_shadowed,
                                            &|name: &str| jit_udf_lookup(user_functions, name),
                                        ) {
                                            ip = exit_ip;
                                            continue;
                                        }
                                    }
                                }
                            }
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
                        // OSR (on-stack replacement) hook. When a hot back-edge
                        // crosses the JIT threshold we compile the loop's body
                        // region `[*target, ip)` to native code; subsequent
                        // back-edges marshal locals across, run the compiled
                        // body to completion (or until a runtime bail), and
                        // resume the interpreter at `ip` (the natural
                        // fall-through after this ForLoopStep). On bail we
                        // simply fall through to the existing `ip = *target`
                        // path and let the interpreter re-execute the body
                        // once more — the compiled body has written back
                        // the in-flight slot values so the next iteration
                        // resumes from exactly the trapping point. With the
                        // `jit` feature off the whole block compiles away.
                        #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
                        {
                            let user_functions = &self.user_functions;
                            let globals = &self.globals;
                            if let Some(engine) = self.jit.as_mut() {
                                let mut is_shadowed =
                                    |name: &str| jit_is_shadowed(user_functions, globals, name);
                                if let Some(exit_ip) = engine.try_run_loop(
                                    func,
                                    *target,
                                    ip,
                                    &mut locals,
                                    closure_env.as_ref(),
                                    &mut is_shadowed,
                                    &|name: &str| jit_udf_lookup(user_functions, name),
                                ) {
                                    ip = exit_ip;
                                    continue;
                                }
                            }
                        }
                        ip = *target;
                    }
                }
                BytecodeOp::JumpIfTrue(target) => {
                    if let Some(cond) = stack.pop() {
                        if cond.is_true() {
                            // OSR Phase 2: a JumpIfTrue whose target is
                            // strictly behind this op is the back-edge of a
                            // `do { body } while (cond)` shape (loop back
                            // when cond is true). Same region/hook mechanics
                            // as the unconditional Jump case.
                            #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
                            {
                                if *target < ip - 1 {
                                    let user_functions = &self.user_functions;
                                    let globals = &self.globals;
                                    if let Some(engine) = self.jit.as_mut() {
                                        let mut is_shadowed = |name: &str| {
                                            jit_is_shadowed(user_functions, globals, name)
                                        };
                                        if let Some(exit_ip) = engine.try_run_loop(
                                            func,
                                            *target,
                                            ip,
                                            &mut locals,
                                            closure_env.as_ref(),
                                            &mut is_shadowed,
                                            &|name: &str| jit_udf_lookup(user_functions, name),
                                        ) {
                                            ip = exit_ip;
                                            continue;
                                        }
                                    }
                                }
                            }
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
                        self.pending_result_writeback = None;
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
                                // queryExecute result=/cfquery name= delivery
                                self.apply_pending_result_writeback(&mut locals, &mut inherited_or_param_keys);
                                stack.push(result);
                            }
                            Err(e) => {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                self.pending_result_writeback = None;
                                // Route error through try-catch mechanism
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    self.restore_capture_state(&handler);
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
                        // The parenthesized `cfinvoke(...)` CALL form (Lucee
                        // accepts every tag as a script call; Wheels' Global.cfc
                        // $invoke() rides it). It reaches here as a CallNamed on
                        // the __cfinvoke stub, but unlike the tag/statement forms
                        // its attributes arrive as NAMED args — and may be wrapped
                        // in `attributeCollection`. Marshal them by hand: pull out
                        // component/method/returnVariable/argumentCollection,
                        // dispatch, and deliver returnVariable scope-aware into the
                        // caller's frame. (The generic reorder path below would
                        // bundle every named arg as an `arguments` overflow and the
                        // positional __cfinvoke intercept would see method="".)
                        if matches!(&func_ref, CfmlValue::Function(f)
                            if f.name.eq_ignore_ascii_case("__cfinvoke"))
                        {
                            let mut comp_val: Option<CfmlValue> = None;
                            let mut method_str: Option<String> = None;
                            let mut return_var: Option<String> = None;
                            let mut arg_collection: Option<CfmlValue> = None;
                            let mut extra_args: ValueMap = ValueMap::default();
                            let mut positional: Vec<CfmlValue> = Vec::new();

                            // attributeCollection supplies the base attributes;
                            // explicit named args override (Lucee precedence).
                            let apply_attr = |key: &str, v: CfmlValue,
                                comp_val: &mut Option<CfmlValue>,
                                method_str: &mut Option<String>,
                                return_var: &mut Option<String>,
                                arg_collection: &mut Option<CfmlValue>,
                                extra_args: &mut ValueMap| {
                                match key.to_lowercase().as_str() {
                                    "component" => *comp_val = Some(v),
                                    "method" => *method_str = Some(v.as_string()),
                                    "returnvariable" => *return_var = Some(v.as_string()),
                                    "argumentcollection" => *arg_collection = Some(v),
                                    _ => {
                                        extra_args.insert(key.to_string(), v);
                                    }
                                }
                            };

                            // Pass 1: attributeCollection (base).
                            for (i, name) in names.iter().enumerate() {
                                if name.eq_ignore_ascii_case("attributecollection") {
                                    if let Some(CfmlValue::Struct(s)) = named_values.get(i) {
                                        for (k, v) in s.iter() {
                                            apply_attr(&k, v, &mut comp_val, &mut method_str,
                                                &mut return_var, &mut arg_collection, &mut extra_args);
                                        }
                                    }
                                }
                            }
                            // Pass 2: explicit named args (override the base).
                            for (i, name) in names.iter().enumerate() {
                                let val = named_values.get(i).cloned().unwrap_or(CfmlValue::Null);
                                if name.is_empty() {
                                    positional.push(val);
                                    continue;
                                }
                                if name.eq_ignore_ascii_case("attributecollection") {
                                    continue;
                                }
                                apply_attr(name, val, &mut comp_val, &mut method_str,
                                    &mut return_var, &mut arg_collection, &mut extra_args);
                            }
                            // Bare positionals (rare): component, then method.
                            let mut positional = positional.into_iter();
                            if comp_val.is_none() {
                                comp_val = positional.next();
                            }
                            if method_str.is_none() {
                                if let Some(m) = positional.next() {
                                    method_str = Some(m.as_string());
                                }
                            }

                            let method_str = method_str.unwrap_or_default();
                            let invoke_args = arg_collection
                                .unwrap_or_else(|| CfmlValue::strukt(extra_args));

                            // Resolve the component. No component (or an empty one)
                            // inside a CFC method means a SIBLING call — dispatch
                            // the method on the current component (this + its
                            // private functions), NOT as a component path.
                            let have_component = matches!(&comp_val, Some(v)
                                if !matches!(v, CfmlValue::Null)
                                    && !matches!(v, CfmlValue::String(s) if s.is_empty()));
                            let component = if have_component {
                                comp_val.unwrap()
                            } else if let Some(CfmlValue::Struct(this_s)) = locals.get("this") {
                                let mut merged = this_s.snapshot();
                                if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                                    for (k, v) in vars.snapshot() {
                                        if matches!(v, CfmlValue::Function(_))
                                            && !merged.keys()
                                                .any(|mk| mk.eq_ignore_ascii_case(&k))
                                        {
                                            merged.insert(k, v);
                                        }
                                    }
                                }
                                CfmlValue::strukt(merged)
                            } else {
                                comp_val.unwrap_or(CfmlValue::Null)
                            };

                            let saved_try_stack = if self.try_stack.is_empty() {
                                None
                            } else {
                                Some(std::mem::take(&mut self.try_stack))
                            };
                            let call_result = self.call_function(
                                &func_ref,
                                vec![component, CfmlValue::string(method_str), invoke_args],
                                &locals,
                            );
                            if let Some(saved) = saved_try_stack {
                                self.try_stack = saved;
                            }
                            match call_result {
                                Ok(result) => {
                                    if let Some(rv) = return_var {
                                        if !rv.is_empty() {
                                            self.pending_result_writeback =
                                                Some(vec![(rv, result.clone())]);
                                            self.apply_pending_result_writeback(
                                                &mut locals,
                                                &mut inherited_or_param_keys,
                                            );
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
                                        self.restore_capture_state(&handler);
                                        let error_val = self.resolve_catch_error_val(&e);
                                        stack.push(error_val);
                                        ip = handler.catch_ip;
                                        continue;
                                    }
                                    return Err(self.wrap_error(e));
                                }
                            }
                            continue;
                        }
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

                        // Tag-call builtins (e.g. `cfdirectory(action=..., name=...)`)
                        // take a single struct-of-options at the bytecode level —
                        // bundle the named args into one struct here. `name`/
                        // `variable` becomes a side-effect write-back of the
                        // return value into the caller's scope (matches the
                        // tag form).
                        let tag_builtin_name = match &func_ref {
                            CfmlValue::Function(f)
                                if Self::is_tag_call_builtin(&f.name) => Some(f.name.to_lowercase()),
                            _ => None,
                        };
                        if let Some(ref tag_name) = tag_builtin_name {
                            let mut opts: ValueMap = ValueMap::default();
                            let mut writeback_var: Option<String> = None;
                            let writeback_attr = Self::tag_call_writeback_attr(tag_name);
                            let is_dbinfo = matches!(
                                tag_builtin_name.as_deref(),
                                Some("cfdbinfo") | Some("dbinfo")
                            );
                            // Fold one attribute into `opts`, tracking the
                            // `name`/`variable` write-back target as we go.
                            let apply_attr =
                                |key: &str, value: CfmlValue,
                                 opts: &mut ValueMap,
                                 writeback_var: &mut Option<String>| {
                                    if key.is_empty() {
                                        return;
                                    }
                                    if !is_dbinfo
                                        && writeback_attr
                                            .iter()
                                            .any(|a| a.eq_ignore_ascii_case(key))
                                    {
                                        *writeback_var = Some(value.as_string());
                                    }
                                    opts.insert(key.to_string(), value);
                                };
                            // Pass 1: attributeCollection supplies the base attrs.
                            // (cfdirectory/cfmail/cfhttp accept the tag's
                            // attributeCollection just like cfinvoke/cfquery —
                            // Wheels' Global.cfc::$directory rides this exact
                            // shape. Without this the struct lands as a literal
                            // `attributecollection` key and the real attrs —
                            // directory/action/name — never reach the intercept.)
                            for (i, name) in expanded_names.iter().enumerate() {
                                if name.eq_ignore_ascii_case("attributecollection") {
                                    if let Some(CfmlValue::Struct(s)) = expanded_values.get(i) {
                                        for (k, v) in s.iter() {
                                            apply_attr(&k, v, &mut opts, &mut writeback_var);
                                        }
                                    }
                                }
                            }
                            // Pass 2: explicit named args override the base.
                            for (i, name) in expanded_names.iter().enumerate() {
                                if name.is_empty()
                                    || name.eq_ignore_ascii_case("attributecollection")
                                {
                                    continue;
                                }
                                let value = if i < expanded_values.len() {
                                    std::mem::replace(&mut expanded_values[i], CfmlValue::Null)
                                } else {
                                    CfmlValue::Null
                                };
                                apply_attr(name, value, &mut opts, &mut writeback_var);
                            }
                            // cfhttp with no explicit result= writes its return
                            // struct into a variable named `cfhttp` (tag-form parity).
                            if writeback_var.is_none() && tag_name.eq_ignore_ascii_case("cfhttp") {
                                writeback_var = Some("cfhttp".to_string());
                            }
                            self.closure_parent_writeback = None;
                            self.arg_ref_writeback = None;
                            self.pending_result_writeback = None;
                            let saved_try_stack = if self.try_stack.is_empty() {
                                None
                            } else {
                                Some(std::mem::take(&mut self.try_stack))
                            };
                            let call_result = self.call_function(
                                &func_ref,
                                vec![CfmlValue::strukt(opts)],
                                &locals,
                            );
                            if let Some(saved) = saved_try_stack {
                                self.try_stack = saved;
                            }
                            match call_result {
                                Ok(result) => {
                                    if let Some(var) = writeback_var {
                                        // Scope-qualified targets (e.g.
                                        // `variable="local.content"` on
                                        // cffile(action="read")) must route
                                        // through the scope-aware store, not a
                                        // literal dotted key in `locals`.
                                        if var.contains('.') {
                                            self.store_runtime_path(&var, result.clone(), &mut locals);
                                        } else {
                                            locals.insert(var, result.clone());
                                        }
                                    }
                                    self.apply_pending_result_writeback(&mut locals, &mut inherited_or_param_keys);
                                    stack.push(result);
                                }
                                Err(e) => {
                                    self.pending_result_writeback = None;
                                    if Self::is_control_flow_error(&e) {
                                        return Err(e);
                                    }
                                    // Tag-call errors must be catchable —
                                    // try { cfdbinfo(...) } catch is the
                                    // standard feature-detection idiom.
                                    if let Some(handler) = self.try_stack.pop() {
                                        while stack.len() > handler.stack_depth {
                                            stack.pop();
                                        }
                                        self.restore_capture_state(&handler);
                                        let error_val = self.resolve_catch_error_val(&e);
                                        stack.push(error_val);
                                        ip = handler.catch_ip;
                                        continue;
                                    }
                                    return Err(self.wrap_error(e));
                                }
                            }
                            continue;
                        }
                        // Reorder named args to match function param positions.
                        // Track overflow (named args with no matching param) so the
                        // callee's `arguments` scope keeps their names.
                        let mut extras: Vec<(usize, String)> = Vec::new();
                        let args = if let CfmlValue::Function(ref f) = func_ref {
                            // Size to the declared params only; positional overflow
                            // and unmatched named args are appended below. Padding to
                            // expanded_names.len() created spurious empty slots that
                            // leaked into the arguments scope as numeric keys when a
                            // paramless function was called purely by name.
                            let mut positional = vec![CfmlValue::Null; f.params.len()];
                            for (i, name) in expanded_names.iter().enumerate() {
                                let value = if i < expanded_values.len() {
                                    std::mem::replace(&mut expanded_values[i], CfmlValue::Null)
                                } else {
                                    CfmlValue::Null
                                };
                                if name.is_empty() {
                                    // Positional arg: fill its slot, or append when it
                                    // overflows the declared params.
                                    if i < positional.len() {
                                        positional[i] = value;
                                    } else {
                                        positional.push(value);
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
                                    None => {
                                        let idx = positional.len();
                                        positional.push(value);
                                        extras.push((idx, name.clone()));
                                    }
                                }
                            }
                            positional
                        } else {
                            named_values
                        };
                        self.pending_extra_named_args =
                            if extras.is_empty() { None } else { Some(extras) };

                        self.closure_parent_writeback = None;
                        self.arg_ref_writeback = None;
                        self.pending_result_writeback = None;
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
                        // CFML forbids mixing positional and named arguments.
                        let call_result = if let Err(e) = Self::validate_named_args(names) {
                            Err(e)
                        } else {
                            self.call_function(&func_ref, args, effective_locals)
                        };
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
                                // queryExecute result=/cfquery name= delivery
                                self.apply_pending_result_writeback(&mut locals, &mut inherited_or_param_keys);
                                stack.push(result);
                            }
                            Err(e) => {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                self.pending_result_writeback = None;
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    self.restore_capture_state(&handler);
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
                                if top_s.ptr_eq(this_s) && !vars.is_empty() {
                                    let mut updated = top_s.snapshot();
                                    updated.insert(
                                        "__variables".to_string(),
                                        CfmlValue::Struct(vars.clone()),
                                    );
                                    let last_idx = stack.len() - 1;
                                    stack[last_idx] = CfmlValue::strukt(updated);
                                }
                            }
                        }
                        // Save variables scope mutations for component write-back
                        if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                            if !vars.is_empty() {
                                // Arc-handle clone, not a full map copy: the method
                                // mutated this store in place, so the writeback site
                                // (same backing store) skips via `merge_from`'s
                                // ptr_eq fast path.
                                self.method_variables_writeback = Some(vars.clone());
                            }
                        } else {
                            let mut vars_wb = ValueMap::default();
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
                                self.method_variables_writeback =
                                    Some(CfmlStruct::new(vars_wb));
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
                    //
                    // A component pseudo-constructor (`__cfc_body__`) / template
                    // (`__main__`) frame is NOT a closure: its locals are the
                    // component's `variables` scope (captured separately via
                    // `captured_locals`) and must never propagate out as a
                    // closure writeback. When such a body runs with an injected
                    // parent scope (e.g. an inherited component's vars), a stale
                    // writeback of all the body's method declarations would be
                    // misattributed to the enclosing method's call site and leak
                    // the freshly-built component's methods into the caller's
                    // locals — poisoning later bare-name calls.
                    if let Some(parent) = parent_scope.filter(|_| !is_template_frame) {
                        if !effective_local_mode_modern {
                            let mut writeback = ValueMap::default();
                            for (k, v) in &locals {
                                if k == "arguments"
                                    || k == "this"
                                    || k.starts_with("__")
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
                    // Capture locals on early return too (mirrors the normal-exit
                    // epilogue below). Without this, an explicit <cfreturn> in a
                    // __main__/__cfc_body__ template skips the capture, so a custom
                    // tag's start phase that mutates `attributes` then returns early
                    // loses those mutations for the end phase. See epilogue ~L4136.
                    if func.name == "__main__" || func.name == "__cfc_body__" {
                        for (_, v) in locals.iter_mut() {
                            if let CfmlValue::Function(ref mut f) = v {
                                Arc::make_mut(f).captured_scope = None;
                            }
                        }
                        if let Some(ref env) = closure_env {
                            env.write().unwrap().clear();
                        }
                        self.captured_locals = Some(std::mem::take(&mut locals));
                    }
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
                        let key = stack.pop().unwrap_or(CfmlValue::string(String::new()));
                        pairs.push((key.as_string(), value));
                    }
                    let mut map = ValueMap::default();
                    for (k, v) in pairs.into_iter().rev() {
                        map.insert(k, v);
                    }
                    stack.push(CfmlValue::strukt(map));
                }
                BytecodeOp::GetIndex => {
                    let index = stack.pop().unwrap_or(CfmlValue::Null);
                    let collection = stack.pop().unwrap_or(CfmlValue::Null);
                    let one_based_to_zero = |index: &CfmlValue| -> usize {
                        let idx = match index {
                            CfmlValue::Int(i) => *i as usize,
                            CfmlValue::Double(d) => *d as usize,
                            CfmlValue::String(s) => s.parse::<usize>().unwrap_or(0),
                            _ => 0,
                        };
                        if idx > 0 { idx - 1 } else { 0 }
                    };
                    match &collection {
                        CfmlValue::Array(arr) => {
                            let idx = one_based_to_zero(&index);
                            stack.push(arr.get(idx).unwrap_or(CfmlValue::Null));
                        }
                        CfmlValue::QueryColumn(arr) => {
                            let idx = one_based_to_zero(&index);
                            stack.push(arr.get(idx).cloned().unwrap_or(CfmlValue::Null));
                        }
                        // Lucee/ACF/BoxLang parity: q["colName"] returns the column
                        // proxy (same as q.colName via GetProperty); q[N] returns
                        // row N as a struct. Frameworks need the bracket form for
                        // dynamic column names (e.g. Wheels' ORM column processing).
                        CfmlValue::Query(q) => {
                            let row_at_oneless = |n: i64| -> CfmlValue {
                                if n >= 1 {
                                    q.get_row((n - 1) as usize)
                                        .map(|m| CfmlValue::Struct(cfml_common::dynamic::CfmlStruct::new(m)))
                                        .unwrap_or(CfmlValue::Null)
                                } else {
                                    CfmlValue::Null
                                }
                            };
                            match &index {
                                CfmlValue::String(name) => {
                                    if let Some(col_data) = q.column_values_ci(name.as_str()) {
                                        stack.push(CfmlValue::QueryColumn(col_data));
                                    } else if let Ok(n) = name.trim().parse::<i64>() {
                                        stack.push(row_at_oneless(n));
                                    } else {
                                        stack.push(CfmlValue::Null);
                                    }
                                }
                                CfmlValue::Int(n) => stack.push(row_at_oneless(*n)),
                                CfmlValue::Double(d) => stack.push(row_at_oneless(*d as i64)),
                                _ => stack.push(CfmlValue::Null),
                            }
                        }
                        CfmlValue::Struct(s) => {
                            let key = index.as_string();
                            let direct = s
                                .get(&key)
                                .or_else(|| s.get(&key.to_uppercase()))
                                .or_else(|| s.get(&key.to_lowercase()))
                                .or_else(|| {
                                    let key_lower = key.to_lowercase();
                                    s.iter()
                                        .find(|(k, _)| k.eq_ignore_ascii_case(&key_lower))
                                        .map(|(_, v)| v)
                                });
                            // Arguments-scope positional fallback: when the
                            // index is numeric N (1-based) and there's no
                            // direct key match, resolve via the declared
                            // param name at position N-1. A value bound to
                            // a declared param lives under its name, not
                            // under the numeric alias.
                            let val = if direct.is_none()
                                && s.contains_key("__arguments_scope")
                            {
                                if let Ok(n) = key.parse::<i64>() {
                                    if n >= 1 {
                                        let idx = (n - 1) as usize;
                                        // First try the declared-param name at
                                        // position N-1 (named call to a fn with
                                        // declared params: value lives under the
                                        // param name).
                                        let by_param = if let Some(CfmlValue::Array(params)) =
                                            s.get("__arguments_params")
                                        {
                                            params
                                                .get(idx)
                                                .map(|p| p.as_string())
                                                .and_then(|name| s.get(&name))
                                        } else {
                                            None
                                        };
                                        // Fall through to the N-th non-marker entry's
                                        // value in insertion order. Lucee/ACF: the
                                        // arguments scope is array-addressable for
                                        // named calls too — `arguments[1]` reads the
                                        // first bound arg even when the callee declares
                                        // no params (the Wheels $set() shape).
                                        by_param.unwrap_or_else(|| {
                                            s.iter()
                                                .filter(|(k, _)| {
                                                    k.as_str() != "__arguments_scope"
                                                        && k.as_str() != "__arguments_params"
                                                })
                                                .nth(idx)
                                                .map(|(_, v)| v)
                                                .unwrap_or(CfmlValue::Null)
                                        })
                                    } else {
                                        CfmlValue::Null
                                    }
                                } else {
                                    CfmlValue::Null
                                }
                            } else {
                                direct.unwrap_or(CfmlValue::Null)
                            };
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
                            // 1-based index; accept Int or numeric Double/String.
                            let one_based: i64 = match &index {
                                CfmlValue::Int(i) => *i,
                                CfmlValue::Double(d) => *d as i64,
                                other => other.as_string().trim().parse::<i64>().unwrap_or(0),
                            };
                            if one_based >= 1 {
                                let idx = (one_based - 1) as usize;
                                // Interior mutability on the shared handle: the
                                // assignment is visible to every alias. Auto-grow
                                // past the end leaves skipped slots as null holes
                                // (Lucee): `a=[]; a[3]="x"` → len 3, [1]/[2] null.
                                arr.set_or_grow(idx, value);
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
                                    if let Some(CfmlValue::Struct(vars)) = s.get("__variables") {
                                        vars.insert(key.clone(), value.clone());
                                    }
                                }
                            }
                            s.insert(key, value);
                        }
                        CfmlValue::Null => {
                            // Auto-vivification: subscript-assigning into a variable
                            // (or member) that does not yet exist creates it, matching
                            // Lucee/ACF/BoxLang. A genuine numeric index creates an
                            // Array; any other key creates a Struct. e.g.
                            // `this.mappings["/app"] = x` where this.mappings is unset.
                            let numeric_idx = match &index {
                                CfmlValue::Int(i) => Some(*i),
                                CfmlValue::Double(d) => Some(*d as i64),
                                _ => None,
                            };
                            if let Some(i) = numeric_idx {
                                let arr = cfml_common::dynamic::CfmlArray::empty();
                                if i >= 1 {
                                    arr.set_or_grow((i - 1) as usize, value);
                                }
                                collection = CfmlValue::Array(arr);
                            } else {
                                let mut s = ValueMap::default();
                                s.insert(index.as_string(), value);
                                collection = CfmlValue::strukt(s);
                            }
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
                    //
                    // Resolve the receiver through the full scope chain, not
                    // just `locals`: at page scope (template top-level), user
                    // variables live in `globals`. Falling back through
                    // `lookup_name_in_scopes` matches the semantics of plain
                    // `LoadLocal` so `p.foo` reads agree with `p["foo"]`.
                    let name_lower_owned: String;
                    let name_lower: &str =
                        if local_name.bytes().any(|b| b.is_ascii_uppercase()) {
                            name_lower_owned = local_name.to_lowercase();
                            &name_lower_owned
                        } else {
                            local_name.as_str()
                        };
                    let receiver = locals
                        .get(local_name.as_str())
                        .cloned()
                        .or_else(|| {
                            self.lookup_name_in_scopes(
                                local_name.as_str(),
                                name_lower,
                                &locals,
                            )
                        });
                    let val = receiver
                        .map(|obj| Self::lookup_property(&obj, prop_name))
                        .unwrap_or(CfmlValue::Null);
                    stack.push(val);
                }
                BytecodeOp::LoadLocalKey(prop_name) => {
                    // Fused LoadLocal("local") + GetProperty for an explicit
                    // `local.foo` read. Reads the single member directly from
                    // `locals` rather than materializing the whole per-call
                    // `local` scope view. Applies the SAME visibility filter as
                    // `build_local_scope_view`: a key is part of `local` only if
                    // it was established in THIS frame — inherited/param keys,
                    // `this`/`super`, and `__`-prefixed bridge keys are invisible.
                    // A miss yields Null, matching GetProperty on the view struct.
                    let is_visible = |k: &str| {
                        !inherited_or_param_keys.contains(k)
                            && k != "this"
                            && k != "super"
                            && !k.starts_with("__")
                    };
                    let name_lower_owned: String;
                    let name_lower: &str =
                        if prop_name.bytes().any(|b| b.is_ascii_uppercase()) {
                            name_lower_owned = prop_name.to_lowercase();
                            &name_lower_owned
                        } else {
                            prop_name.as_str()
                        };
                    // Exact hit first (the common case), then a case-insensitive
                    // scan — mirrors GetProperty's resolution order.
                    let val = locals
                        .get_key_value(prop_name.as_str())
                        .filter(|(k, _)| is_visible(k))
                        .or_else(|| {
                            locals
                                .iter()
                                .find(|(k, _)| {
                                    k.eq_ignore_ascii_case(name_lower) && is_visible(k)
                                })
                        })
                        .map(|(_, v)| v.clone())
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
                                    s.get("__super")
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
                            if let Some(s) = obj.as_cfml_struct() {
                                s.insert(prop_name.clone(), value);
                            } else {
                                return Err(CfmlError::runtime(format!(
                                    "Cannot set property '{}' on non-struct in local '{}'",
                                    prop_name, local_name
                                )));
                            }
                        } else {
                            // Auto-vivification: assigning to a member path of a
                            // variable that does not yet exist creates that variable
                            // as a struct, matching Lucee/ACF/BoxLang. e.g.
                            // `initArgs.path = "x"` where initArgs was never declared.
                            let mut s = ValueMap::default();
                            s.insert(prop_name.clone(), value);
                            locals.insert(local_name.clone(), CfmlValue::strukt(s));
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
                                            .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                                                        .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
                                                        .map(|(_, v)| v)
                                                })
                                        } else {
                                            None
                                        }
                                    })
                                    ;
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
                                        stack.push(CfmlValue::Int(q.row_count() as i64));
                                    }
                                    "columnlist" => {
                                        // Uppercase column names, matching Lucee/ACF columnList.
                                        stack.push(CfmlValue::string(q.column_list()));
                                    }
                                    _ => {
                                        // Column access: q.columnName returns a QueryColumn
                                        // proxy — acts as Array for indexing/iteration/length,
                                        // but stringifies to the first row (Lucee parity).
                                        if let Some(col_data) = q.column_values_ci(name) {
                                            stack.push(CfmlValue::QueryColumn(col_data));
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
                                    s.get("__super")
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
                            if let Some(s) = obj.as_cfml_struct() {
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
                                        if let Some(CfmlValue::Struct(vars)) =
                                            s.get("__variables")
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

                BytecodeOp::NewObject(arg_count)
                | BytecodeOp::NewObjectNamed(_, arg_count) => {
                    // When `new X(...)` supplied named arguments, recover the
                    // call-site names so init() binds by name (not position).
                    // Cloned eagerly to an owned value to avoid borrowing `op`
                    // across the &mut self init call below.
                    let ctor_arg_names: Option<Vec<String>> = match op {
                        BytecodeOp::NewObjectNamed(names, _) => Some(names.clone()),
                        _ => None,
                    };
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
                                CfmlValue::String(s) => (**s).clone(),
                                _ => class_ref.as_string(),
                            };
                            match self.resolve_component_template(&class_name, &locals) {
                                Some(t) => t,
                                // Unresolved component path: throw rather than
                                // instantiate an empty struct (Lucee/ACF parity).
                                None => {
                                    return Err(self.wrap_error(CfmlError::runtime(format!(
                                        "Could not find the component [{}].",
                                        class_name
                                    ))))
                                }
                            }
                        };

                        // Resolve inheritance chain
                        let instance = self.resolve_inheritance(template, &locals);

                        // Attach a Rust-class parent if extends="rust:Name"
                        let instance = self.attach_native_parent(instance)?;

                        // Validate interface implementation and stamp the
                        // transitive interface set (for isInstanceOf).
                        let instance = self.attach_implements_chain(instance, &locals)?;

                        // Call init() constructor if present
                        let final_instance = if let CfmlValue::Struct(ref s) = instance {
                            let has_init = s
                                .get("init")
                                .or_else(|| s.get("INIT"))
                                .or_else(|| s.get("Init"))
                                ;
                            if let Some(ref init_func) = has_init {
                                if matches!(init_func, CfmlValue::Function(_)) {
                                    // Build init scope from the component's own scope,
                                    // NOT the caller's locals (which may be a different CFC)
                                    let mut init_locals = ValueMap::default();
                                    init_locals.insert("this".to_string(), instance.clone());
                                    // Inject component __variables as a dedicated scope (like Lucee/BoxLang)
                                    if let CfmlValue::Struct(ref cs) = instance {
                                        if let Some(vars) = cs.get("__variables") {
                                            init_locals
                                                .insert("__variables".to_string(), vars.clone());
                                        }
                                    }
                                    // Reorder named constructor args to match
                                    // init()'s declared params (extras spill into
                                    // the arguments scope, mirroring CallMethodNamed).
                                    let init_args = if ctor_arg_names.is_some() {
                                        let (reordered, extras) =
                                            Self::reorder_named_args_with_extras(
                                                init_func,
                                                ctor_arg_names.as_deref(),
                                                ctor_args,
                                            );
                                        self.pending_extra_named_args =
                                            if extras.is_empty() { None } else { Some(extras) };
                                        reordered
                                    } else {
                                        ctor_args
                                    };
                                    self.closure_parent_writeback = None;
                                    // Isolate the try-stack so a throw inside init()
                                    // doesn't consume the caller's handlers (and jump
                                    // init's own ip to the caller's catch_ip). Without
                                    // this, `new Comp(args)` inside a try{} swallowed
                                    // init()'s exception while the explicit
                                    // createObject(...).init(args) path (which saves the
                                    // try-stack around CallMethod) propagated it.
                                    let saved_try_stack = if self.try_stack.is_empty() {
                                        None
                                    } else {
                                        Some(std::mem::take(&mut self.try_stack))
                                    };
                                    let init_call_result =
                                        self.call_function(init_func, init_args, &init_locals);
                                    if let Some(saved) = saved_try_stack {
                                        self.try_stack = saved;
                                    }
                                    // Propagate any exception thrown by init() —
                                    // `new Comp(args)` must fail-fast exactly like the
                                    // explicit createObject(...).init(args) path does.
                                    // Route the error through the in-loop try/catch
                                    // mechanism (NOT `?`, which would unwind past an
                                    // active try handler and escape the request).
                                    let result = match init_call_result {
                                        Ok(val) => val,
                                        Err(e) => {
                                            if Self::is_control_flow_error(&e) {
                                                return Err(e);
                                            }
                                            if let Some(handler) = self.try_stack.pop() {
                                                while stack.len() > handler.stack_depth {
                                                    stack.pop();
                                                }
                                                self.restore_capture_state(&handler);
                                                let error_val = self.resolve_catch_error_val(&e);
                                                stack.push(error_val);
                                                ip = handler.catch_ip;
                                                continue;
                                            } else {
                                                return Err(e);
                                            }
                                        }
                                    };
                                    self.closure_parent_writeback = None;
                                    // Apply variables scope writeback from init() to the component
                                    let vars_wb = self.method_variables_writeback.take();
                                    let final_obj = if let Some(modified_this) =
                                        self.method_this_writeback.take()
                                    {
                                        modified_this
                                    } else if let CfmlValue::Struct(_) = &result {
                                        result
                                    } else {
                                        instance
                                    };
                                    // Merge init()'s __variables mutations back into the component.
                                    // `vars` is the (Arc-shared) __variables the init frame
                                    // mutated in place; storing the handle is an identity write
                                    // when the component already shares that store.
                                    if let Some(vars) = vars_wb {
                                        if let Some(s) = final_obj.as_cfml_struct() {
                                            s.insert(
                                                "__variables".to_string(),
                                                CfmlValue::Struct(vars),
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
                        };

                        stack.push(final_instance);
                    } else {
                        stack.push(CfmlValue::Null);
                    }
                }

                BytecodeOp::DefineFunction(global_id) => {
                    let global_id = *global_id as i64;
                    // A new function value is being created this request, so the
                    // end-of-request re-homing walk has potential work to do.
                    self.app_fn_table_dirty = true;
                    // Resolve the target function by its process-stable global_id
                    // through the registry. This is independent of the active
                    // `self.program`, so a previously-merged function (e.g. a CFC
                    // method dispatched by name) that defines a closure while a
                    // smaller sub-program is swapped in resolves correctly by
                    // construction — issue #70 cannot recur.
                    let bc_func_arc = match self.resolve_fn(global_id) {
                        Some(arc) => arc,
                        None => {
                            return Err(self.wrap_error(CfmlError::runtime(format!(
                                "Internal error: DefineFunction global_id {} is not registered",
                                global_id
                            ))));
                        }
                    };
                    let func_name = bc_func_arc.name.clone();
                    // Lucee parity: a named function declaration that collides
                    // with a built-in function is a compile/parse-time error in
                    // Lucee (`AbstrCFMLScriptTransformer.funcStatement` throws
                    // "The name [X] is already used by a built in Function"). We
                    // don't have a separate compile pass that owns the builtin
                    // set, so we enforce the same rule the first time the
                    // DefineFunction op runs — fires on script load, which is
                    // UX-equivalent. Skip synthesized names so closures, arrow
                    // functions, and `__main__` are unaffected: only `function
                    // abs(x) { … }`-style decls are rejected. Component methods
                    // are also exempt — Lucee/ACF allow a CFC to define a method
                    // whose name matches a builtin (object dispatch wins over
                    // the BIF for `obj.method()`); the guard would otherwise
                    // poison the whole component (PR #79).
                    if !func_name.starts_with("__")
                        && !bc_func_arc.is_component_method
                        && self.builtins.contains_key(func_name.as_str())
                    {
                        return Err(self.wrap_error(CfmlError::runtime(format!(
                            "The name [{}] is already used by a built in Function",
                            func_name
                        ))));
                    }
                    self.user_functions
                        .insert(func_name.clone(), Arc::clone(&bc_func_arc));
                    // Create or reuse a shared closure environment so all closures
                    // defined in this function invocation share the same mutable state.
                    // On first definition, seed directly from locals; on subsequent
                    // definitions, sync so later closures see intervening declarations.
                    let env = match closure_env {
                        Some(ref env) => {
                            let mut m = env.write().unwrap();
                            for (k, v) in &locals {
                                // Do NOT store Function values in the shared closure
                                // env: each closure's captured_scope is an Arc clone
                                // of this env, so an env-resident Function creates a
                                // self-referential Arc cycle (env -> Function ->
                                // captured_scope -> env) that leaks the whole env on
                                // frame exit. Sibling closures resolve each other via
                                // user_functions / by-name fast paths, and call sites
                                // overlay caller-local Functions, so omitting them
                                // here is observationally transparent.
                                if !matches!(v, CfmlValue::Function(_)) {
                                    m.insert(k.clone(), v.clone());
                                }
                            }
                            env
                        }
                        None => {
                            let seed: ValueMap = locals
                                .iter()
                                .filter(|(_, v)| !matches!(v, CfmlValue::Function(_)))
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            closure_env.insert(Arc::new(RwLock::new(seed)))
                        }
                    };
                    // Push function reference — encode func_idx in body for super dispatch
                    let bc_func_ref = &bc_func_arc;
                    stack.push(CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
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
                            CfmlValue::Int(global_id),
                        )),
                        return_type: None,
                        access: cfml_common::dynamic::CfmlAccess::Public,
                        captured_scope: Some(Arc::clone(env)),
                    })));
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
                        saved_buffers_depth: self.saved_output_buffers.len(),
                        custom_tag_depth: self.custom_tag_stack.len(),
                    });
                }
                BytecodeOp::TryEnd => {
                    self.try_stack.pop();
                }
                BytecodeOp::Throw => {
                    let error_val = stack
                        .pop()
                        .unwrap_or(CfmlValue::string("Unknown error".to_string()));
                    self.last_exception = Some(error_val.clone());
                    if let Some(handler) = self.try_stack.pop() {
                        // Unwind stack
                        while stack.len() > handler.stack_depth {
                            stack.pop();
                        }
                        self.restore_capture_state(&handler);
                        stack.push(error_val);
                        ip = handler.catch_ip;
                    } else {
                        // Propagate the original message (not the serialized
                        // struct) so resolve_catch_error_val matches last_exception
                        // and reuses the full cfcatch struct — preserving the
                        // error's `type`/`detail` across the frame boundary.
                        return Err(CfmlError::runtime(match &error_val {
                            CfmlValue::Struct(s) => s
                                .get("message")
                                .map(|m| m.as_string())
                                .unwrap_or_else(|| error_val.as_string()),
                            _ => error_val.as_string(),
                        }));
                    }
                }
                BytecodeOp::Rethrow => {
                    let error_val = self
                        .last_exception
                        .clone()
                        .unwrap_or(CfmlValue::string("No exception to rethrow".to_string()));
                    if let Some(handler) = self.try_stack.pop() {
                        while stack.len() > handler.stack_depth {
                            stack.pop();
                        }
                        self.restore_capture_state(&handler);
                        stack.push(error_val);
                        ip = handler.catch_ip;
                    } else {
                        // Propagate the original message (not the serialized
                        // struct) so resolve_catch_error_val matches last_exception
                        // and reuses the full cfcatch struct — preserving the
                        // error's `type`/`detail` across the frame boundary.
                        return Err(CfmlError::runtime(match &error_val {
                            CfmlValue::Struct(s) => s
                                .get("message")
                                .map(|m| m.as_string())
                                .unwrap_or_else(|| error_val.as_string()),
                            _ => error_val.as_string(),
                        }));
                    }
                }

                BytecodeOp::CatchMatch(catch_type) => {
                    // Peek (do NOT consume) the exception value the catch handler
                    // was entered with, and push whether its `type` matches this
                    // clause's declared type.
                    let exc_type = match stack.last() {
                        Some(CfmlValue::Struct(s)) => {
                            s.get("type").map(|v| v.as_string()).unwrap_or_default()
                        }
                        _ => String::new(),
                    };
                    let matches = Self::catch_type_matches(catch_type, &exc_type);
                    stack.push(CfmlValue::Bool(matches));
                }

                BytecodeOp::CallMethod(method_name, arg_count, write_back)
                | BytecodeOp::CallMethodNamed(method_name, _, arg_count, write_back) => {
                    // For the named variant, recover the call-site argument names
                    // from the instruction (ip was already advanced past it). The
                    // `|`-pattern can't bind the names field directly because the
                    // two variants differ in shape, so read them back here. This
                    // mirrors how CallNamed recovers arg sources via ip - 1.
                    let method_arg_names: Option<&[String]> =
                        match &func.instructions[ip - 1] {
                            BytecodeOp::CallMethodNamed(_, names, _, _) => Some(names.as_slice()),
                            _ => None,
                        };
                    let mut extra_args: Vec<CfmlValue> =
                        (0..*arg_count).filter_map(|_| stack.pop()).collect();
                    extra_args.reverse();
                    // Pop the object (receiver)
                    let object = stack.pop().unwrap_or(CfmlValue::Null);

                    // Does this receiver have `this`/variables write-back semantics?
                    // Only CFCs (carry __variables/__name) and Java shims (e.g.
                    // Queue.poll / Map.remove mutate-in-place) do. A plain struct or
                    // array does NOT — so any method_this_writeback / variables
                    // writeback that surfaces after the call on such a receiver is
                    // stale: it leaked from a closure that captured `this` and ran
                    // inside a higher-order member method (e.g. `mappings.some( (k,m)
                    // => m.isAspect() )`). Applying it would overwrite the receiver
                    // variable with the closure's captured `this`. Gate on this flag
                    // so the leaked writeback is discarded for plain receivers.
                    let receiver_writeback_ok = matches!(
                        &object,
                        CfmlValue::Struct(ref s)
                            if s.contains_key("__variables")
                                || s.contains_key("__name")
                                || s.contains_key("__java_shim")
                    );

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
                    // CFML forbids mixing positional and named arguments.
                    let named_args_check =
                        method_arg_names.map_or(Ok(()), Self::validate_named_args);
                    let method_result: Result<CfmlValue, CfmlError> =
                        if let Err(e) = named_args_check {
                            Err(e)
                        } else if let CfmlValue::Struct(ref s) = object {
                            if s.contains_key("__is_super") {
                                // Super dispatch — find the parent's function by stored index
                                let prop = s.get_ci(method_name.as_str()).unwrap_or(CfmlValue::Null);
                                if let CfmlValue::Function(ref f) = &prop {
                                    // Extract the stored global_id from the body.
                                    let func_idx =
                                        if let cfml_common::dynamic::CfmlClosureBody::Expression(
                                            ref body,
                                        ) = f.body
                                        {
                                            if let CfmlValue::Int(idx) = body.as_ref() {
                                                Some(*idx)
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        };
                                    let raw_args: Vec<CfmlValue> = extra_args.drain(..).collect();
                                    let (args, extras) = Self::reorder_named_args_with_extras(
                                        &prop,
                                        method_arg_names,
                                        raw_args,
                                    );
                                    self.pending_extra_named_args =
                                        if extras.is_empty() { None } else { Some(extras) };
                                    let mut method_locals = ValueMap::default();
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
                                    // Execute directly by global_id to avoid a
                                    // name collision with the child's override;
                                    // resolved through the registry, independent
                                    // of the active program.
                                    self.closure_parent_writeback = None;
                                    let parent_func = func_idx.and_then(|i| self.resolve_fn(i));
                                    let call_result = if let Some(parent_func) = parent_func {
                                        self.execute_function_with_args(
                                            &parent_func,
                                            args,
                                            Some(&method_locals),
                                        )
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
                                        method_arg_names,
                                    )
                                }
                            } else {
                                self.call_member_function(
                                    &object,
                                    &method_name,
                                    &mut extra_args,
                                    method_arg_names,
                                )
                            }
                        } else {
                            self.call_member_function(
                                &object,
                                &method_name,
                                &mut extra_args,
                                method_arg_names,
                            )
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
                                self.restore_capture_state(&handler);
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
                                // Chained-CFC identity guard: for `a.getDep().mutate()`
                                // the result is a foreign CFC (codegen propagates
                                // write_back=["a"] to the outer call). Writing it back
                                // would clobber `a`'s identity. Skip when `a` and the
                                // result are distinct CFC instances; allow same-instance,
                                // non-CFC results (arrays/Java shims), or non-CFC `a`.
                                let clobbers_foreign_cfc = matches!(
                                    (self.scope_aware_load(var_name, &locals), &result),
                                    (Some(CfmlValue::Struct(ref cur)), CfmlValue::Struct(ref res))
                                        if cur.contains_key("__variables")
                                            && res.contains_key("__variables")
                                            && !cur.ptr_eq(res)
                                );
                                if !clobbers_foreign_cfc {
                                    self.scope_aware_store(var_name, result.clone(), &mut locals);
                                }
                            }
                        } else if path.len() >= 2 && Self::is_mutating_method(&method_name) {
                            // Deep property write-back: var.prop1.prop2...propN.method(args)
                            let var_name = &path[0];
                            if let Some(mut root_obj) = self.scope_aware_load(var_name, &locals) {
                                let props = &path[1..];
                                // Chained-CFC identity guard (deep path): for
                                // `a.b.getDep().mutate()` the mutating call runs on a
                                // foreign CFC and returns it; codegen propagates
                                // write_back=["a","b"], so deep_set would overwrite a.b
                                // with that foreign CFC (e.g.
                                // `injector.getScopeStorage().put(...)` clobbering
                                // `variables.injector` with the ScopeStorage put()
                                // returns). Skip when the current leaf and the result
                                // are distinct CFC instances. Non-CFC results
                                // (arrays/Java shims, e.g. `a.b.append(x)`) are ungated.
                                let result_is_cfc = matches!(
                                    &result,
                                    CfmlValue::Struct(s) if s.contains_key("__variables")
                                );
                                let mut skip_for_identity = false;
                                if result_is_cfc {
                                    let mut node = root_obj.clone();
                                    let mut reached = true;
                                    for part in props {
                                        match node.get(part) {
                                            Some(v) => node = v,
                                            None => { reached = false; break; }
                                        }
                                    }
                                    if reached {
                                        if let (CfmlValue::Struct(cur), CfmlValue::Struct(res)) =
                                            (&node, &result)
                                        {
                                            if cur.contains_key("__variables") && !cur.ptr_eq(res) {
                                                skip_for_identity = true;
                                            }
                                        }
                                    }
                                }
                                if !skip_for_identity {
                                    Self::deep_set(&mut root_obj, props, result.clone());
                                    self.scope_aware_store(var_name, root_obj, &mut locals);
                                }
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
                        if receiver_writeback_ok {
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
                                // Chained-CFC identity guard (single-segment path):
                                // `a.getDep().mutate()` where getDep() returns a
                                // *different* CFC. Codegen propagates write_back=["a"]
                                // to the outer call too, so its `this` is a foreign
                                // CFC; merging it into `a` would clobber `a`'s identity
                                // (__name/methods). Skip when `a` and modified_this are
                                // distinct CFC instances. Mirrors the deep-path guard
                                // below. Java shims lack __variables → not gated, so
                                // chained shim mutation (sb.append().append()) still
                                // propagates.
                                let skip_for_identity = is_cfc
                                    && matches!(
                                        (&existing, &modified_this),
                                        (Some(CfmlValue::Struct(cur)), CfmlValue::Struct(snap))
                                            if cur.contains_key("__variables") && !cur.ptr_eq(snap)
                                    );
                                // Same-instance guard: when the snapshot IS the receiver
                                // (same Arc), every `this.x =` in the callee already
                                // mutated the shared struct in place — there is nothing
                                // to write back. Rebuilding via snapshot() would DETACH
                                // the binding onto a fresh Arc; for `this.method()` inside
                                // a CFC method that detaches the frame's own `this`, so
                                // all later this-writes in the frame are silently
                                // discarded on return (PR #100, broke Wheels create()/
                                // update() persistence).
                                let same_instance = matches!(
                                    (&existing, &modified_this),
                                    (Some(CfmlValue::Struct(cur)), CfmlValue::Struct(snap))
                                        if cur.ptr_eq(snap)
                                );
                                if skip_for_identity {
                                    self.method_variables_writeback = None;
                                } else if !same_instance {
                                let merged = if is_cfc {
                                    match (existing, modified_this) {
                                        (Some(CfmlValue::Struct(cur)), CfmlValue::Struct(snap)) => {
                                            let mut cur_map = cur.snapshot();
                                            let preserved_vars = cur_map.get("__variables").cloned();
                                            for (k, v) in snap.iter() {
                                                if k == "__variables" {
                                                    continue;
                                                }
                                                cur_map.insert(k, v);
                                            }
                                            if let Some(vars) = preserved_vars {
                                                cur_map.insert("__variables".to_string(), vars);
                                            }
                                            CfmlValue::strukt(cur_map)
                                        }
                                        (_, snap) => snap,
                                    }
                                } else {
                                    modified_this
                                };
                                self.scope_aware_store(var_name, merged, &mut locals);
                                }
                            } else {
                                // Chained-CFC identity guard: for
                                // `a.foo().bar()` with `foo` returning a
                                // *different* CFC (e.g.
                                // `injector.getBinder().getCustomDSL()`),
                                // codegen propagates write_back=path to BOTH
                                // calls. Outer `bar`'s `this` is then a
                                // foreign CFC, and a naive deep_set would
                                // clobber `a.b.c…` with that foreign CFC.
                                // Detect by walking the path and comparing
                                // Arc identity. Only fires when both sides
                                // are CFC structs (cheap to gate on
                                // __variables presence in modified_this).
                                let modified_is_cfc = matches!(
                                    &modified_this,
                                    CfmlValue::Struct(s) if s.contains_key("__variables")
                                );
                                let mut skip_for_identity = false;
                                if modified_is_cfc {
                                    if let Some(mut node) =
                                        self.scope_aware_load(var_name, &locals)
                                    {
                                        let mut reached = true;
                                        for part in &path[1..] {
                                            match node.get(part) {
                                                Some(v) => node = v,
                                                None => { reached = false; break; }
                                            }
                                        }
                                        if reached {
                                            if let (
                                                CfmlValue::Struct(cur),
                                                CfmlValue::Struct(snap),
                                            ) = (&node, &modified_this)
                                            {
                                                if !cur.ptr_eq(snap)
                                                    && cur.contains_key("__variables")
                                                {
                                                    skip_for_identity = true;
                                                }
                                            }
                                        }
                                    }
                                }
                                if skip_for_identity {
                                    self.method_variables_writeback = None;
                                } else if let Some(mut root_obj) =
                                    self.scope_aware_load(var_name, &locals)
                                {
                                    let props = &path[1..];
                                    Self::deep_set(&mut root_obj, props, modified_this);
                                    self.scope_aware_store(var_name, root_obj, &mut locals);
                                }
                            }
                        }
                        }
                    }

                    // Propagate component method `variables` scope mutations back.
                    // When a method writes `variables.x = y`, persist it in __variables.
                    if let Some(vars_wb) = self.method_variables_writeback.take() {
                        if receiver_writeback_ok {
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
                                if let Some(s) = comp_obj.as_cfml_struct() {
                                    // Only write a method's `variables` mutations back into
                                    // an actual CFC instance (carries `__name`). A deep
                                    // write_back path like `arguments.cfc.getName()` resolves
                                    // comp_obj to the `arguments` SCOPE, which is not a
                                    // component — injecting a synthetic `__variables` there
                                    // pollutes the scope and (via the arguments param-sync)
                                    // clobbers the caller frame's real component scope.
                                    if s.contains_key("__name") {
                                        let vs = s.get_or_insert_struct("__variables");
                                        vs.merge_from(&vars_wb);
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
                                // Hide the private arguments-scope markers
                                // from for-in iteration. Real numeric keys
                                // (overflow positional args) still surface,
                                // matching Lucee.
                                let is_args = s.contains_key("__arguments_scope");
                                // A struct carrying CFC instance markers is a
                                // component instance (this engine materialises
                                // CFCs as marker-bearing structs). Lucee/ACF
                                // for-in over a component yields only its data
                                // members — never engine-internal `__` keys, the
                                // `this` scope itself, or methods (UDFs).
                                let is_cfc = is_component_struct(&s);
                                let keys: Vec<CfmlValue> = s
                                    .keys()
                                    .into_iter()
                                    .filter(|k| {
                                        !is_args
                                            || (k != "__arguments_scope"
                                                && k != "__arguments_params")
                                    })
                                    .filter(|k| {
                                        if !is_cfc {
                                            return true;
                                        }
                                        if k.starts_with("__") || k.eq_ignore_ascii_case("this") {
                                            return false;
                                        }
                                        !matches!(
                                            s.get(k),
                                            Some(CfmlValue::Function(_))
                                                | Some(CfmlValue::Closure(_))
                                        )
                                    })
                                    .map(CfmlValue::string)
                                    .collect();
                                stack.push(CfmlValue::array(keys));
                            }
                            CfmlValue::String(s) => {
                                // Lucee parity: for-in over a string iterates it
                                // as a comma-delimited LIST, not characters.
                                // Comma is the only delimiter, items are not
                                // trimmed, and empty items are KEPT ("a,,b" is
                                // 3 items) — unlike ListToArray's default. An
                                // empty string never enters the loop.
                                let items: Vec<CfmlValue> = if s.is_empty() {
                                    Vec::new()
                                } else {
                                    s.split(',')
                                        .map(|item| CfmlValue::string(item.to_string()))
                                        .collect()
                                };
                                stack.push(CfmlValue::array(items));
                            }
                            CfmlValue::Query(q) => {
                                // Iterating over a query: convert to array of row structs
                                let rows: Vec<CfmlValue> = q
                                    .rows()
                                    .into_iter()
                                    .map(CfmlValue::strukt)
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
                            let old_program = self.push_program_swap(sub_program);
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
                                    if k == "arguments" || k.starts_with("__") {
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
                            // Functions defined in the included file are already
                            // resolvable after the swap is popped: each was
                            // registered into `fn_registry` by its global_id when
                            // the sub-program was swapped in, and named ones were
                            // inserted into `user_functions` by their DefineFunction
                            // op. Stored Function values created in the include
                            // carry the same stable global_id. So no merge-append
                            // or index fixup is needed (those existed only to keep
                            // the old program-relative indices valid).
                            self.pop_program_swap(old_program);
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
                                    self.restore_capture_state(&handler);
                                    let mut err_struct = ValueMap::default();
                                    err_struct.insert(
                                        "message".to_string(),
                                        CfmlValue::string(e.message.clone()),
                                    );
                                    err_struct.insert(
                                        "type".to_string(),
                                        CfmlValue::string(format!("{}", e.error_type)),
                                    );
                                    err_struct.insert(
                                        "detail".to_string(),
                                        CfmlValue::string(String::new()),
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
                            let old_program = self.push_program_swap(sub_program);
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
                                    if k == "arguments" || k.starts_with("__") {
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
                            // No merge-append or index fixup needed: the dynamic
                            // include's functions are already registered by global_id
                            // (sub-program swap-in) and by name (DefineFunction), and
                            // stored Function values carry stable global_ids.
                            self.pop_program_swap(old_program);
                            self.source_file = old_source;
                            if let Err(e) = result {
                                if Self::is_control_flow_error(&e) {
                                    return Err(e);
                                }
                                if let Some(handler) = self.try_stack.pop() {
                                    while stack.len() > handler.stack_depth {
                                        stack.pop();
                                    }
                                    self.restore_capture_state(&handler);
                                    let mut err_struct = ValueMap::default();
                                    err_struct.insert(
                                        "message".to_string(),
                                        CfmlValue::string(e.message.clone()),
                                    );
                                    err_struct.insert(
                                        "type".to_string(),
                                        CfmlValue::string(format!("{}", e.error_type)),
                                    );
                                    err_struct.insert(
                                        "detail".to_string(),
                                        CfmlValue::string(String::new()),
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
                    if let (CfmlValue::Array(a), CfmlValue::Array(b)) = (left, right) {
                        // Concatenation produces a NEW array (not a mutation of
                        // either operand), so snapshot both into a fresh Vec.
                        let mut v = a.snapshot();
                        v.extend(b.iter());
                        stack.push(CfmlValue::array(v));
                    } else {
                        stack.push(CfmlValue::array(Vec::new()));
                    }
                }

                BytecodeOp::MergeStructs => {
                    let right = stack.pop().unwrap_or(CfmlValue::strukt(ValueMap::default()));
                    let left = stack.pop().unwrap_or(CfmlValue::strukt(ValueMap::default()));
                    if let (CfmlValue::Struct(a), CfmlValue::Struct(b)) = (left, right) {
                        let mut m = a.snapshot();
                        for (k, v) in b.iter() {
                            m.insert(k, v);
                        }
                        stack.push(CfmlValue::strukt(m));
                    } else {
                        stack.push(CfmlValue::strukt(ValueMap::default()));
                    }
                }

                BytecodeOp::CallSpread => {
                    // Stack: [func_ref, args_array]
                    let args_val = stack.pop().unwrap_or(CfmlValue::array(Vec::new()));
                    let func_ref = stack.pop().unwrap_or(CfmlValue::Null);
                    let args: Vec<CfmlValue> = if let CfmlValue::Array(a) = args_val {
                        a.snapshot()
                    } else {
                        vec![args_val]
                    };
                    self.closure_parent_writeback = None;
                    let result = self.call_function(&func_ref, args, &locals)?;
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
                    let this_struct = match this_val {
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
                    this_struct.insert("__super".to_string(), parent);
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
                // Arc-handle clone (see Return path): the writeback site skips
                // the copy when it's the same backing store.
                if !vars.is_empty() {
                    self.method_variables_writeback = Some(vars.clone());
                }
            } else {
                // Non-CFC or legacy path: collect from locals
                let mut vars_wb = ValueMap::default();
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
                    self.method_variables_writeback = Some(CfmlStruct::new(vars_wb));
                }
            }
        }

        // Closure parent scope write-back: compute diff of parent-scope vars.
        // Modern localmode (Lucee parity): no unscoped write propagates out of
        // the closure — neither new keys nor shadowing-mutations of captured
        // outer vars. All bareword writes stayed in the closure's own `local`.
        // Explicit `variables.x = …` bypasses this path, so it still works.
        //
        // Skip for `__cfc_body__`/`__main__` frames — see the matching note on
        // the early-return path above: a component-body/template frame's locals
        // are captured as the component's `variables` scope and must never leak
        // out as a closure writeback (which would poison the enclosing method's
        // caller locals with the new component's methods).
        if let Some(parent) = parent_scope.filter(|_| !is_template_frame) {
            if !effective_local_mode_modern {
                let mut writeback = ValueMap::default();
                for (k, v) in &locals {
                    // Skip function params, arguments scope, 'this', var-declared
                    // locals, and __variables (handled by method_variables_writeback)
                    if k == "arguments"
                        || k == "this"
                        || k.starts_with("__")
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
            // Strip captured_scope from any top-level Function values before
            // snapshotting — CFC methods resolve via __variables, not closures.
            for (_, v) in locals.iter_mut() {
                if let CfmlValue::Function(ref mut f) = v {
                    Arc::make_mut(f).captured_scope = None;
                }
            }
            // Break the closure-env reference cycle. The pseudo-constructor's
            // shared `closure_env` holds scope structs (`this`, `variables`) that
            // contain the just-defined methods, and each method's `captured_scope`
            // is an Arc clone of this same env: env -> this(Struct) -> method(Fn)
            // -> captured_scope -> env. That self-referential island is never
            // collected by Arc refcounting, so it leaks per instantiation (and
            // per repeated call to any closure-defining function). CFC methods
            // never use this env (they use __variables injected at call time), so
            // clearing it here severs the cycle without affecting behaviour.
            if let Some(ref env) = closure_env {
                env.write().unwrap().clear();
            }
            self.captured_locals = Some(locals);

            // `cfexit` stops at the template boundary: a template frame is the
            // owner of an unwinding exit. Hand the method off to
            // `last_exit_method` so the surrounding handler (custom tag, page
            // runner) can decide what to do, and stop the signal propagating
            // past this frame.
            if let Some(method) = self.pending_exit.take() {
                self.last_exit_method = Some(method);
            }
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
        parent_locals: &ValueMap,
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
            // Fast path: if the function carries a stored global_id, dispatch
            // directly (skips all builtin matching for user-defined functions).
            // Resolution is an O(1) registry index — no hashing — and independent
            // of the active program; an unregistered id yields `None` and falls
            // through to the by-name dispatch below.
            if let cfml_common::dynamic::CfmlClosureBody::Expression(ref body) = func.body {
                if let CfmlValue::Int(idx) = body.as_ref() {
                    if let Some(user_func) = self.resolve_fn(*idx) {
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
                        // Lexical relative-component resolution: while a user
                        // function runs, a bare createObject("component","X")/
                        // `new X()` inside it must resolve X relative to the
                        // package of the file that DEFINES the function, not the
                        // caller's file. call_member_function does this for
                        // receiver dispatch (this.m()/child.m(), the #132/#133
                        // fixes); a BARE call to an inherited method (e.g. a
                        // child-defined `go()` calling the inherited
                        // `makeSibling()`) routes here instead, so it needs the
                        // same swap or it would search the outer frame's dir.
                        // Skip when the callee carries no source (dynamic
                        // closures) or already matches the active file — the
                        // common same-file / self-recursion case pays only a
                        // cheap comparison, no clone. (Wheels migrator shape:
                        // migration up() -> inherited createTable() ->
                        // createObject("TableDefinition").)
                        let saved_source_file = if user_func.source_file.is_some()
                            && user_func.source_file != self.source_file
                        {
                            let prev = self.source_file.clone();
                            self.source_file = user_func.source_file.clone();
                            Some(prev)
                        } else {
                            None
                        };
                        let result = self.execute_function_with_args(
                            &user_func,
                            args,
                            Some(effective_parent),
                        );
                        if let Some(prev) = saved_source_file {
                            self.source_file = prev;
                        }
                        return result;
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

            if matches!(name_lower.as_str(), "cfdirectory" | "__cfdirectory") {
                if let Some(CfmlValue::Struct(opts)) = args.first() {
                    let action = opts
                        .get_ci("action")
                        .map(|v| v.as_string().to_lowercase())
                        .unwrap_or_else(|| "list".to_string());
                    if action == "list" {
                        return self.cfdirectory_list_from_opts(opts);
                    }
                }
            }

            // queryAppend: mutates the first query in-place (reference-typed —
            // the shared handle propagates to the caller), returns boolean.
            if name_lower == "queryappend" {
                if let (Some(CfmlValue::Query(q1)), Some(CfmlValue::Query(q2))) =
                    (args.first(), args.get(1))
                {
                    let q2_data: cfml_common::dynamic::CfmlQueryData =
                        q2.with_read(|d| d.clone());
                    q1.with_write(|d| d.append_query(&q2_data));
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
                    let new_row = new_row.snapshot();
                    let ok = q.with_write(|d| {
                        if pos >= 1 && pos <= d.row_count() {
                            for ci in 0..d.columns.len() {
                                let col_name = d.columns[ci].clone();
                                let val = new_row
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(&col_name))
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or(CfmlValue::Null);
                                std::sync::Arc::make_mut(&mut d.data[ci])[pos - 1] = val;
                            }
                            true
                        } else {
                            false
                        }
                    });
                    return Ok(CfmlValue::Bool(ok));
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
                        // Reference semantics: remove in place on the shared
                        // handle so the caller's array reflects the deletion.
                        arr.with_write(|v| {
                            v.remove(p);
                        });
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
                | "getapplicationmetadata"
                | "__cfheader"
                | "__cfcontent"
                | "__cflocation"
                | "__cfabort"
                | "__cfexit"
                | "__cfhtmlhead"
                | "__cfhtmlbody"
                | "gethttprequestdata"
                | "__cfinvoke"
                | "__cfsavecontent_start"
                | "__cfsavecontent_end"
                | "invoke"
                | "getbasetemplatepath"
                | "getfunctioncalledname"
                | "gettimezone"
                | "expandpath"
                | "isdefined"
                | "__cfparam"
                | "queryexecute"
                | "cfdbinfo"
                | "dbinfo"
                | "queryregisterfunction"
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
                | "applicationstop"
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
                | "__cfloop_file_lines"
                | "__cfexecute"
                | "__cfthread_run"
                | "__cfthread_join"
                | "__cfthread_terminate"
                | "threadjoin"
                | "threadterminate"
                | "runasync"
                | "_schedule"
                | "callstackget"
                | "callstackdump"
                | "getpagecontext"
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                            let mut pl: Option<ValueMap> = None;
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
                            let mut pl: Option<ValueMap> = None;
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
                                let mut pl: Option<ValueMap> = None;
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
                            let mut pl: Option<ValueMap> = None;
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
                            let mut pl: Option<ValueMap> = None;
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
                            let mut pl: Option<ValueMap> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::string(k.clone()));
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
                            let mut result = ValueMap::default();
                            let callback = callback.clone();
                            let mut pl: Option<ValueMap> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::string(k.clone()));
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
                    return Ok(CfmlValue::strukt(ValueMap::default()));
                }
                "structfilter" => {
                    if let (Some(struct_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Struct(s) = struct_val {
                            let mut result = ValueMap::default();
                            let callback = callback.clone();
                            let mut pl: Option<ValueMap> = None;
                            for (k, v) in s.iter() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::string(k.clone()));
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
                    return Ok(CfmlValue::strukt(ValueMap::default()));
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
                                cb_args.push(CfmlValue::string(k.clone()));
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
                                cb_args.push(CfmlValue::string(k.clone()));
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
                                cb_args.push(CfmlValue::string(k.clone()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(list_val.clone());
                            self.closure_parent_writeback = None;
                            let mapped = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.push(mapped.as_string());
                        }
                        return Ok(CfmlValue::string(result.join(&first_delim)));
                    }
                    return Ok(CfmlValue::string(String::new()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                        return Ok(CfmlValue::string(result.join(&first_delim)));
                    }
                    return Ok(CfmlValue::string(String::new()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                            cb_args.push(CfmlValue::string(item.to_string()));
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
                            cb_args.push(CfmlValue::string(ch.to_string()));
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
                            cb_args.push(CfmlValue::string(ch.to_string()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(str_val.clone());
                            self.closure_parent_writeback = None;
                            let mapped = self.call_function(&callback, cb_args, parent_locals)?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            result.push_str(&mapped.as_string());
                        }
                        return Ok(CfmlValue::string(result));
                    }
                    return Ok(CfmlValue::string(String::new()));
                }
                "stringfilter" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let callback = callback.clone();
                        let mut result = String::new();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::string(ch.to_string()));
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
                        return Ok(CfmlValue::string(result));
                    }
                    return Ok(CfmlValue::string(String::new()));
                }
                "stringreduce" => {
                    if let (Some(str_val), Some(callback)) = (args.get(0), args.get(1)) {
                        let s = str_val.as_string();
                        let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                        let callback = callback.clone();
                        for (i, ch) in s.chars().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::string(ch.to_string()));
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
                            cb_args.push(CfmlValue::string(ch.to_string()));
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
                            cb_args.push(CfmlValue::string(ch.to_string()));
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
                                        CfmlValue::string(chars[j].to_string()),
                                        CfmlValue::string(chars[j + 1].to_string()),
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
                        return Ok(CfmlValue::string(chars.into_iter().collect::<String>()));
                    }
                    return Ok(CfmlValue::string(String::new()));
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
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                for (i, row) in q.rows().into_iter().enumerate() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::strukt(row));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                                let mut result = ValueMap::default();
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                                return Ok(CfmlValue::string(result.join(",")));
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
                                let mut result = ValueMap::default();
                                for (key, val) in s.iter() {
                                    let mut cb_args = Vec::with_capacity(3);
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                                return Ok(CfmlValue::string(result.join(",")));
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
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                                    cb_args.push(CfmlValue::string(key.clone()));
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
                                    cb_args.push(CfmlValue::string(item.to_string()));
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
                            // Snapshot rows before the loop: the callback may
                            // re-enter and touch this same query.
                            for (i, row) in q.rows().into_iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row));
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
                            let snapshot = q.rows();
                            let mut new_rows = Vec::with_capacity(snapshot.len());
                            for (i, row) in snapshot.into_iter().enumerate() {
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
                                    new_rows.push(s.snapshot());
                                } else {
                                    new_rows.push(row);
                                }
                            }
                            // queryMap returns a NEW, independent query.
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(CfmlQuery::from_parts(q.columns(), new_rows)));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "queryfilter" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            let mut new_rows = Vec::new();
                            for (i, row) in q.rows().into_iter().enumerate() {
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
                                    new_rows.push(row);
                                }
                            }
                            // queryFilter returns a NEW, independent query.
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(CfmlQuery::from_parts(q.columns(), new_rows)));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "queryreduce" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let mut acc = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                            let callback = callback.clone();
                            for (i, row) in q.rows().into_iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(4);
                                cb_args.push(acc.clone());
                                cb_args.push(CfmlValue::strukt(row));
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
                            let mut rows = q.rows();
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
                            // querySort sorts IN PLACE (reference-typed): write the
                            // ordered rows back to the shared handle, return it.
                            q.with_write(|d| {
                                let cols = d.columns.clone();
                                *d = cfml_common::dynamic::CfmlQueryData::from_named_rows(cols, rows);
                            });
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Query(q.clone()));
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "querysome" => {
                    if let (Some(q_val), Some(callback)) = (args.get(0), args.get(1)) {
                        if let CfmlValue::Query(q) = q_val {
                            let callback = callback.clone();
                            for (i, row) in q.rows().into_iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row));
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
                            for (i, row) in q.rows().into_iter().enumerate() {
                                let mut cb_args = Vec::with_capacity(3);
                                cb_args.push(CfmlValue::strukt(row));
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
                    // Reference-typed: push rows onto the shared handle IN PLACE
                    // (O(1) per row — this is what makes building an N-row query
                    // O(n) instead of O(n²)). The caller's query sees the rows
                    // through the shared Arc, so no writeback is needed.
                    if let Some(CfmlValue::Query(q)) = args.first() {
                        if args.len() >= 2 {
                            match &args[1] {
                                CfmlValue::Int(n) => {
                                    for _ in 0..*n {
                                        q.add_row(ValueMap::default());
                                    }
                                }
                                CfmlValue::Struct(data) => {
                                    q.add_row(data.snapshot());
                                }
                                CfmlValue::Array(items) => {
                                    // Lucee semantics: array-of-arrays → one
                                    // positional row per inner array; array-of-
                                    // structs → one row per struct; a flat array
                                    // of scalars → a single positional row.
                                    let items = items.snapshot();
                                    let cols = q.columns();
                                    let all_arrays = !items.is_empty()
                                        && items.iter().all(|it| matches!(it, CfmlValue::Array(_)));
                                    if all_arrays {
                                        for it in &items {
                                            if let CfmlValue::Array(vals) = it {
                                                q.add_row(positional_row(&cols, &vals.snapshot()));
                                            }
                                        }
                                    } else if items.iter().all(|it| matches!(it, CfmlValue::Struct(_))) {
                                        for it in &items {
                                            if let CfmlValue::Struct(s) = it {
                                                q.add_row(s.snapshot());
                                            }
                                        }
                                    } else {
                                        q.add_row(positional_row(&cols, &items));
                                    }
                                }
                                _ => {
                                    q.add_row(ValueMap::default());
                                }
                            }
                        } else {
                            q.add_row(ValueMap::default());
                        }
                        self.arg_ref_writeback = None;
                        return Ok(CfmlValue::Int(q.row_count() as i64));
                    }
                    return Ok(CfmlValue::Int(0));
                }
                "querysetcell" => {
                    // Reference-typed: set the cell on the shared handle in place.
                    if args.len() >= 3 {
                        if let CfmlValue::Query(q) = &args[0] {
                            let column = args[1].as_string();
                            let value = args[2].clone();
                            let row_idx = if args.len() >= 4 {
                                match &args[3] {
                                    CfmlValue::Int(n) => (*n as usize).saturating_sub(1),
                                    _ => q.row_count().saturating_sub(1),
                                }
                            } else {
                                q.row_count().saturating_sub(1)
                            };
                            q.set_cell(row_idx, column, value);
                            self.arg_ref_writeback = None;
                            return Ok(CfmlValue::Bool(true));
                        }
                    }
                    return Ok(CfmlValue::Bool(false));
                }
                "getcurrenttemplatepath" => {
                    if let Some(ref source) = self.source_file {
                        if let Ok(abs) = self.vfs.canonicalize(source) {
                            return Ok(CfmlValue::string(abs));
                        }
                        return Ok(CfmlValue::string(source.clone()));
                    }
                    // Fallback to CWD
                    if let Ok(cwd) = std::env::current_dir() {
                        return Ok(CfmlValue::string(cwd.to_string_lossy().to_string()));
                    }
                    return Ok(CfmlValue::string(String::new()));
                }
                "getbasetemplatepath" => {
                    if let Some(ref base) = self.base_template_path {
                        if let Ok(abs) = self.vfs.canonicalize(base) {
                            return Ok(CfmlValue::string(abs));
                        }
                        return Ok(CfmlValue::string(base.clone()));
                    }
                    // Fall back to source_file
                    if let Some(ref source) = self.source_file {
                        if let Ok(abs) = self.vfs.canonicalize(source) {
                            return Ok(CfmlValue::string(abs));
                        }
                        return Ok(CfmlValue::string(source.clone()));
                    }
                    return Ok(CfmlValue::string(String::new()));
                }
                "expandpath" => {
                    // CFML expandPath: resolve relative to current template dir,
                    // absolute paths (starting with /) resolve via mappings then source dir
                    let raw = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    // Collapse a leading run of slashes to a single '/' before
                    // mapping resolution — Lucee/ACF/BoxLang treat expandPath("//x")
                    // as expandPath("/x"). Wheels' bootstrap concatenates paths that
                    // yield "//plugins", which must still hit this.mappings.
                    let rel = if raw.starts_with('/') || raw.starts_with('\\') {
                        let trimmed = raw.trim_start_matches(['/', '\\']);
                        format!("/{}", trimmed)
                    } else {
                        raw.clone()
                    };
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
                    let mut result = self.vfs.canonicalize(&path_str).unwrap_or(path_str);
                    // Preserve a trailing slash from the input. Lucee/ACF/BoxLang
                    // mirror the input's trailing slash on the output; canonicalize
                    // strips it for existing paths, so reapply when the caller had one.
                    if (rel.ends_with('/') || rel.ends_with('\\'))
                        && !result.ends_with('/')
                        && !result.ends_with('\\')
                    {
                        result.push('/');
                    }
                    return Ok(CfmlValue::string(result));
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
                            self.set_session_variable(key, default_val)?;
                        }
                        "application" => {
                            if let Some(ref app_scope) = self.application_scope {
                                app_scope.insert(key.to_string(), default_val);
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
                            return Ok(CfmlValue::string(tz));
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
                                return Ok(CfmlValue::string(tz.to_string()));
                            }
                        }
                    }
                    // Fallback: return UTC
                    return Ok(CfmlValue::string("UTC".to_string()));
                }
                "getapplicationmetadata" => {
                    // Build application metadata from the loaded Application.cfc
                    // `this` scope (name, sessionManagement, sessionTimeout,
                    // mappings, datasources, and any custom this.* settings),
                    // matching Lucee/ACF. Falls back to {name:""} when no
                    // Application.cfc is active. WireBox's ScopeStorage reads
                    // `getApplicationMetadata().sessionManagement` here.
                    let mut meta = ValueMap::default();
                    if let Some(CfmlValue::Struct(s)) = &self.app_cfc_template {
                        for (k, v) in s.snapshot() {
                            // Settings only — skip lifecycle methods + internals.
                            if k.starts_with("__") || matches!(v, CfmlValue::Function(_)) {
                                continue;
                            }
                            meta.insert(k, v);
                        }
                    }
                    if !meta.keys().any(|k| k.eq_ignore_ascii_case("name")) {
                        let nm = self.current_application_name.clone().unwrap_or_default();
                        meta.insert("name".to_string(), CfmlValue::string(nm));
                    }
                    return Ok(CfmlValue::strukt(meta));
                }
                "getcomponentmetadata" => {
                    // Helper: extract metadata from a component struct
                    fn extract_component_meta(
                        s: &ValueMap,
                        fallback_name: &str,
                    ) -> CfmlValue {
                        let mut meta = ValueMap::default();
                        let name_val = s
                            .get("__name")
                            .cloned()
                            .unwrap_or(CfmlValue::string(fallback_name.to_string()));
                        meta.insert("name".to_string(), name_val.clone());
                        // fullname mirrors getMetadata(): the dotted component path.
                        meta.insert("fullname".to_string(), name_val);
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
                                    let mut func_meta = ValueMap::default();
                                    func_meta
                                        .insert("name".to_string(), CfmlValue::string(k.clone()));
                                    func_meta.insert(
                                        "access".to_string(),
                                        CfmlValue::string(format!("{:?}", f.access).to_lowercase()),
                                    );
                                    if let Some(ref rt) = f.return_type {
                                        func_meta.insert(
                                            "returntype".to_string(),
                                            CfmlValue::string(rt.clone()),
                                        );
                                    }
                                    let params: Vec<CfmlValue> = f
                                        .params
                                        .iter()
                                        .map(|p| CfmlValue::string(p.name.clone()))
                                        .collect();
                                    func_meta
                                        .insert("parameters".to_string(), CfmlValue::array(params));
                                    // Merge custom function metadata emitted as __funcmeta_<name>
                                    let fmeta_key = format!("__funcmeta_{}", k);
                                    if let Some(CfmlValue::Struct(fm)) = s.get(&fmeta_key) {
                                        for (mk, mv) in fm.iter() {
                                            if !func_meta.contains_key(&mk) {
                                                func_meta.insert(mk, mv);
                                            }
                                        }
                                    }
                                    functions.push(CfmlValue::strukt(func_meta));
                                }
                            }
                        }
                        meta.insert("functions".to_string(), CfmlValue::array(functions));
                        if let Some(CfmlValue::Struct(md)) = s.get("__metadata") {
                            // Custom component attributes appear as top-level keys in
                            // CFML metadata (Lucee/ACF parity) — e.g. `component
                            // delegates="..."` => md.delegates. WireBox's
                            // getAnnotationValue reads them at the top level. Mirrors
                            // getMetadata(). The guard keeps the reserved keys
                            // (name/extends/functions/properties) authoritative.
                            for (mk, mv) in md.iter() {
                                if !meta.contains_key(&mk) {
                                    meta.insert(mk, mv);
                                }
                            }
                            meta.insert("metadata".to_string(), CfmlValue::Struct(md.clone()));
                        }
                        if let Some(props) = s.get("__properties") {
                            meta.insert("properties".to_string(), props.clone());
                        }
                        CfmlValue::strukt(meta)
                    }

                    if let Some(arg) = args.get(0) {
                        // If the argument is already a struct (component instance), extract metadata directly
                        if let CfmlValue::Struct(ref s) = arg {
                            return Ok(extract_component_meta(&s.snapshot(), ""));
                        }
                        // Otherwise treat as a component name/path to look up
                        let comp_name = arg.as_string();
                        if let Some(template) =
                            self.resolve_component_template(&comp_name, parent_locals)
                        {
                            let resolved = self.resolve_inheritance(template, parent_locals);
                            if let CfmlValue::Struct(ref s) = resolved {
                                return Ok(extract_component_meta(&s.snapshot(), &comp_name));
                            }
                            return Ok(resolved);
                        }
                    }
                    return Ok(CfmlValue::strukt(ValueMap::default()));
                }
                "createobject" => {
                    // Single-argument shorthand: createObject("comp.path") is
                    // equivalent to createObject("component", "comp.path")
                    // (Lucee/ACF parity). Normalize to the two-arg form so the
                    // shared resolution below handles it; previously a lone
                    // argument fell through to the trailing `Null` return.
                    let args = if args.len() == 1 {
                        vec![
                            CfmlValue::string("component".to_string()),
                            args.into_iter().next().unwrap(),
                        ]
                    } else {
                        args
                    };
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
                                let instance = self.attach_native_parent(instance)?;
                                return self.attach_implements_chain(instance, parent_locals);
                            }
                            // Unresolved component path: throw rather than return
                            // null silently (Lucee/ACF both raise here).
                            return Err(CfmlError::runtime(format!(
                                "Could not find the component [{}].",
                                comp_name
                            )));
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
                                "java.util.date" => {
                                    handle_java_date("init", empty_args, &CfmlValue::Null)
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
                                "java.util.regex.pattern" => {
                                    handle_java_pattern("init", empty_args, &CfmlValue::Null)
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
                                self.response_body = Some(CfmlValue::string(contents));
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfabort" => {
                    // `cfabort showError="msg"` passes a non-empty message: it is a
                    // catchable application error (a user `try`/`catch` can trap it,
                    // and an uncaught one is handed to onError), NOT the silent
                    // control-flow unwind that fires onAbort. Plain `cfabort` (no
                    // arg) raises the abort sentinel.
                    if let Some(msg) = args.get(0) {
                        let m = msg.as_string();
                        if !m.is_empty() {
                            return Err(CfmlError::runtime(m));
                        }
                    }
                    return Err(CfmlError::new(
                        "__cfabort".to_string(),
                        CfmlErrorType::Custom("abort".to_string()),
                    ));
                }
                "__cfexit" => {
                    // `cfexit` control-flow signal. Set `pending_exit`; the
                    // bytecode loop unwinds the current function at its next op
                    // (preserving partial locals) and the owning template frame
                    // hands the method off to `last_exit_method`. Unlike an
                    // exception this is invisible to user `try`/`catch`.
                    let method = args
                        .get(0)
                        .map(|v| v.as_string().to_lowercase())
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "exittemplate".to_string());
                    self.pending_exit = Some(method);
                    return Ok(CfmlValue::Null);
                }
                "__cfhtmlhead" | "__cfhtmlbody" => {
                    // Buffer the content; injected into the response <head>/<body>
                    // at output finalization (see finalize_html_injections).
                    // Two shapes reach here: the tag form `<cfhtmlhead text="…">`
                    // lowers to a single positional string; the script call form
                    // `cfhtmlhead(text="…")` bundles a `{text: …}` options struct
                    // (see is_tag_call_builtin). Unwrap the struct's `text` key.
                    let text = match args.get(0) {
                        Some(CfmlValue::Struct(opts)) => opts
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("text"))
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default(),
                        Some(v) => v.as_string(),
                        None => String::new(),
                    };
                    if name_lower == "__cfhtmlhead" {
                        self.html_head_buffer.push_str(&text);
                        if !text.ends_with('\n') {
                            self.html_head_buffer.push('\n');
                        }
                    } else {
                        self.html_body_buffer.push_str(&text);
                        if !text.ends_with('\n') {
                            self.html_body_buffer.push('\n');
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cflocation" => {
                    let to_status = |v: &CfmlValue| -> u16 {
                        match v {
                            CfmlValue::Int(n) => *n as u16,
                            CfmlValue::String(s) => s.parse::<u16>().unwrap_or(302),
                            CfmlValue::Double(d) => *d as u16,
                            _ => 302,
                        }
                    };
                    // Two call shapes reach here: the tag / named-arg form bundles
                    // a single options struct `{url, statusCode, addToken}`; the
                    // positional script BIF `Location(url, addToken, statusCode)`
                    // arrives as raw positional args. Support both.
                    let (url, status_code) = match args.get(0) {
                        Some(CfmlValue::Struct(opts)) => {
                            let url = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "url")
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_default();
                            let status_code = opts
                                .iter()
                                .find(|(k, _)| k.to_lowercase() == "statuscode")
                                .map(|(_, v)| to_status(&v))
                                .unwrap_or(302);
                            (url, status_code)
                        }
                        Some(url_val) => {
                            // Positional: Location(url, addToken, statusCode).
                            let url = url_val.as_string();
                            let status_code = args.get(2).map(to_status).unwrap_or(302);
                            (url, status_code)
                        }
                        None => return Ok(CfmlValue::Null),
                    };
                    if url.is_empty() {
                        return Ok(CfmlValue::Null);
                    }
                    self.redirect_url = Some(url.clone());
                    self.response_headers.push(("Location".to_string(), url));
                    self.response_status = Some((status_code, "Found".to_string()));
                    return Err(CfmlError::new(
                        "__cflocation_redirect".to_string(),
                        CfmlErrorType::Custom("redirect".to_string()),
                    ));
                }
                "gethttprequestdata" => {
                    if let Some(ref data) = self.http_request_data {
                        return Ok(data.clone());
                    }
                    let mut empty = ValueMap::default();
                    empty.insert("headers".to_string(), CfmlValue::strukt(ValueMap::default()));
                    empty.insert("content".to_string(), CfmlValue::string(String::new()));
                    empty.insert("method".to_string(), CfmlValue::string(String::new()));
                    empty.insert("protocol".to_string(), CfmlValue::string(String::new()));
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
                            .find(|(k, _)| k.eq_ignore_ascii_case(&method_lower))
                            .map(|(_, v)| v.clone());

                        if let Some(func @ CfmlValue::Function(_)) = method_func {
                            let call_args = self.build_invoke_call_args(&func, invoke_args);

                            let mut method_locals = ValueMap::default();
                            method_locals.insert("this".to_string(), component.clone());
                            // Inject __variables from component so unscoped references resolve
                            if let CfmlValue::Struct(ref cs) = component {
                                if let Some(vars) = cs.get("__variables") {
                                    method_locals.insert("__variables".to_string(), vars.clone());
                                }
                            }
                            return self.call_function(&func, call_args, &method_locals);
                        } else {
                            // An undeclared method must follow the same dispatch
                            // rules as a direct dot-call: if the component defines
                            // onMissingMethod, route through it (Lucee/ACF/BoxLang
                            // all do). Wheels dispatches association cascade methods
                            // (deleteAll<assoc> etc.) by name via cfinvoke and relies
                            // on this landing in the model's onMissingMethod.
                            let omm = comp_struct
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("onmissingmethod"))
                                .map(|(_, v)| v.clone());
                            if let Some(handler @ CfmlValue::Function(_)) = omm {
                                // cfinvoke marshals its args into a struct; pass it
                                // straight through as missingMethodArguments (already
                                // keyed by name, mirroring the dot-call fallback).
                                let missing_args = match invoke_args {
                                    s @ CfmlValue::Struct(_) => s,
                                    _ => CfmlValue::strukt(ValueMap::default()),
                                };
                                let mut method_locals = ValueMap::default();
                                method_locals.insert("this".to_string(), component.clone());
                                if let CfmlValue::Struct(ref cs) = component {
                                    if let Some(vars) = cs.get("__variables") {
                                        method_locals.insert("__variables".to_string(), vars.clone());
                                    }
                                }
                                return self.call_function(
                                    &handler,
                                    vec![
                                        CfmlValue::string(method_name.clone()),
                                        missing_args,
                                    ],
                                    &method_locals,
                                );
                            }
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
                    return Ok(CfmlValue::string(captured));
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
                            .find(|(k, _)| k.eq_ignore_ascii_case(&method_lower))
                            .map(|(_, v)| v.clone());

                        if let Some(func @ CfmlValue::Function(_)) = method_func {
                            let call_args = self.build_invoke_call_args(&func, invoke_args);

                            let mut method_locals = ValueMap::default();
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
                "queryregisterfunction" => {
                    // queryRegisterFunction(name, udf [, type]) — register a CFML
                    // UDF/closure for use inside QoQ SQL. type: scalar|aggregate.
                    let fname = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let func = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                    let kind = match args.get(2).map(|v| v.as_string().to_lowercase()).as_deref() {
                        Some("aggregate") => QoQFnKind::Aggregate,
                        _ => QoQFnKind::Scalar,
                    };
                    if fname.is_empty() {
                        return Err(CfmlError::runtime(
                            "queryRegisterFunction: a function name is required".to_string(),
                        ));
                    }
                    if !matches!(func, CfmlValue::Function(_) | CfmlValue::Closure(_)) {
                        return Err(CfmlError::runtime(
                            "queryRegisterFunction: the second argument must be a function or closure"
                                .to_string(),
                        ));
                    }
                    self.qoq_registry.register_custom(&fname, func, kind);
                    return Ok(CfmlValue::Null);
                }
                "queryexecute" => {
                    let mut args = args;
                    // cfquery routes its name=/result= attributes through the
                    // options struct — they can only be known at runtime
                    // (Wheels passes everything via attributeCollection), so
                    // no compile-time assignment can deliver them. Expand
                    // attributeCollection into the options (explicit keys
                    // win, like Lucee), record the delivery directives, and
                    // strip the internal cfquery marker.
                    let sql_text = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let mut result_name: Option<String> = None;
                    let mut name_attr: Option<String> = None;
                    let mut from_cfquery_tag = false;
                    let mut return_type_opt: Option<String> = None;
                    if let Some(CfmlValue::Struct(opts)) = args.get(2) {
                        let snap = opts.snapshot();
                        let mut attr_coll: Option<ValueMap> = None;
                        let mut merged: ValueMap = ValueMap::default();
                        for (k, v) in snap.into_iter() {
                            if k.eq_ignore_ascii_case("attributecollection") {
                                if let CfmlValue::Struct(inner) = &v {
                                    attr_coll = Some(inner.snapshot());
                                    continue;
                                }
                            }
                            if k.eq_ignore_ascii_case("__cfquery_tag") {
                                from_cfquery_tag = true;
                                continue;
                            }
                            merged.insert(k, v);
                        }
                        if let Some(coll) = attr_coll {
                            for (ik, iv) in coll.into_iter() {
                                if !merged.keys().any(|mk| mk.eq_ignore_ascii_case(&ik)) {
                                    merged.insert(ik, iv);
                                }
                            }
                        }
                        result_name = merged
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("result"))
                            .map(|(_, v)| v.as_string())
                            .filter(|s| !s.is_empty());
                        name_attr = merged
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("name"))
                            .map(|(_, v)| v.as_string())
                            .filter(|s| !s.is_empty());
                        return_type_opt = merged
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("returntype"))
                            .map(|(_, v)| v.as_string().to_lowercase());
                        args[2] = CfmlValue::strukt(merged);
                    }

                    // Query of Queries: dbtype="query" runs against in-memory
                    // query variables instead of a datasource.
                    let is_qoq = match args.get(2) {
                        Some(CfmlValue::Struct(opts)) => opts
                            .get_ci("dbtype")
                            .map(|v| v.as_string().eq_ignore_ascii_case("query"))
                            .unwrap_or(false),
                        _ => false,
                    };
                    // Measure wall-clock execution time of the driver
                    // round-trip (Lucee's `executionTime`, in ms). Monotonic
                    // clock isn't available on wasm32, so it reports 0 there.
                    #[cfg(not(target_arch = "wasm32"))]
                    let __query_start = std::time::Instant::now();
                    let ret = if is_qoq {
                        let sql = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                        let params_arg = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                        let (return_type, column_key) = match args.get(2) {
                            Some(CfmlValue::Struct(opts)) => {
                                let rt = opts
                                    .get_ci("returntype")
                                    .map(|v| v.as_string().to_lowercase())
                                    .unwrap_or_else(|| "query".to_string());
                                let ck = opts.get_ci("columnkey").map(|v| v.as_string());
                                (rt, ck)
                            }
                            _ => ("query".to_string(), None),
                        };
                        self.execute_qoq(
                            &sql,
                            &params_arg,
                            &return_type,
                            column_key,
                            parent_locals,
                        )?
                    } else {
                        // Resolve a per-application datasource (this.datasources /
                        // per-app cfconfig) to its connection URL before any path
                        // below sees the args. No-op when this request has none.
                        let args = self.rewrite_query_datasource(args);
                        // Lazy transaction begin: a `transaction { }` block that
                        // had no datasource at entry begins here, on the first
                        // query's datasource (Lucee/ACF defer-begin parity).
                        if self.transaction_pending
                            && self.transaction_conn.is_none()
                        {
                            if let Some(txn_begin) = self.txn_begin {
                                let ds = match args.get(2) {
                                    Some(CfmlValue::Struct(opts)) => {
                                        opts.get_ci("datasource").map(|v| v.as_string())
                                    }
                                    _ => None,
                                }
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| {
                                    self.get_default_datasource(parent_locals)
                                });
                                if !ds.is_empty() {
                                    let conn = txn_begin(&ds)?;
                                    self.transaction_conn = Some(conn);
                                    self.transaction_datasource = Some(ds);
                                    self.transaction_pending = false;
                                }
                            }
                        }
                        // Route through the transaction conn if one is active.
                        if self.transaction_conn.is_some() && self.txn_execute.is_some() {
                            let txn_execute = self.txn_execute.unwrap();
                            let sql = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                            let params_arg = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                            let return_type =
                                return_type_opt.clone().unwrap_or_else(|| "query".to_string());
                            let txn_conn = self.transaction_conn.as_mut().unwrap();
                            txn_execute(txn_conn, &sql, &params_arg, &return_type)?
                        } else if let Some(qe_fn) = self.query_execute_fn {
                            // No active transaction — delegate to the normal
                            // builtin (via fn pointer or registered builtin).
                            qe_fn(args)?
                        } else if let Some(builtin) = self
                            .builtins
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "queryexecute")
                            .map(|(_, v)| *v)
                        {
                            builtin(args)?
                        } else {
                            return Err(CfmlError::runtime(
                                "queryExecute: database features not enabled".to_string(),
                            ));
                        }
                    };

                    #[cfg(not(target_arch = "wasm32"))]
                    let exec_ms = __query_start.elapsed().as_millis() as i64;
                    #[cfg(target_arch = "wasm32")]
                    let exec_ms = 0i64;

                    // Patch the measured time into a mutation-result struct that
                    // the driver built with a placeholder 0 (INSERT/UPDATE/DELETE
                    // return this struct directly). SELECT/QoQ return a Query (or
                    // a returntype=struct shape that has no executionTime key), so
                    // those are left untouched here and timed when the `result=`
                    // struct is built below.
                    if let CfmlValue::Struct(s) = &ret {
                        if s.get_ci("executionTime").is_some()
                            && s.get_ci("recordCount").is_some()
                            && s.get_ci("cached").is_some()
                        {
                            s.insert("executionTime".to_string(), CfmlValue::Int(exec_ms));
                        }
                    }

                    // Deliver `result` (queryExecute option + cfquery attr) and
                    // `name` (cfquery attr only — direct queryExecute ignores
                    // it, matching Lucee) into the caller's scope. The names
                    // may be dotted ("local.wheels.result"); the Call op
                    // handler applies the writeback in the caller's frame.
                    let mut deliveries: Vec<(String, CfmlValue)> = Vec::new();
                    if let Some(rn) = result_name {
                        let result_struct = match &ret {
                            CfmlValue::Query(q) => {
                                let mut m = ValueMap::default();
                                m.insert(
                                    "recordCount".to_string(),
                                    CfmlValue::Int(q.row_count() as i64),
                                );
                                m.insert("cached".to_string(), CfmlValue::Bool(false));
                                m.insert(
                                    "columnList".to_string(),
                                    CfmlValue::string(q.column_list()),
                                );
                                m.insert(
                                    "sql".to_string(),
                                    CfmlValue::string(
                                        q.sql().unwrap_or_else(|| sql_text.clone()),
                                    ),
                                );
                                m.insert("executionTime".to_string(), CfmlValue::Int(exec_ms));
                                CfmlValue::strukt(m)
                            }
                            // Mutation metadata struct from the driver — it IS
                            // the result shape. Snapshot so the delivered
                            // struct is independent of the returned one.
                            CfmlValue::Struct(s) => CfmlValue::strukt(s.snapshot()),
                            other => {
                                let mut m = ValueMap::default();
                                if let CfmlValue::Array(a) = other {
                                    m.insert(
                                        "recordCount".to_string(),
                                        CfmlValue::Int(a.len() as i64),
                                    );
                                }
                                m.insert("cached".to_string(), CfmlValue::Bool(false));
                                m.insert("sql".to_string(), CfmlValue::string(sql_text.clone()));
                                m.insert("executionTime".to_string(), CfmlValue::Int(exec_ms));
                                CfmlValue::strukt(m)
                            }
                        };
                        deliveries.push((rn, result_struct));
                    }
                    if from_cfquery_tag {
                        if let Some(n) = name_attr {
                            // Lucee: `name` is only set when the statement
                            // produced a resultset (SELECT / returning) — it
                            // stays untouched for INSERT/UPDATE/DDL.
                            let rt = return_type_opt.as_deref().unwrap_or("query");
                            let deliver = match &ret {
                                CfmlValue::Query(_) => true,
                                CfmlValue::Array(_) => rt == "array",
                                CfmlValue::Struct(_) => rt.starts_with("struct"),
                                _ => false,
                            };
                            if deliver {
                                deliveries.push((n, ret.clone()));
                            }
                        }
                    }
                    if !deliveries.is_empty() {
                        self.pending_result_writeback = Some(deliveries);
                    }
                    return Ok(ret);
                }
                "cfdbinfo" | "dbinfo" => {
                    // cfdbinfo — datasource metadata (issue #90 Gap B). The
                    // attributes arrive as one struct (tag form and script
                    // form both bundle); attributeCollection is the only
                    // shape Wheels uses ($dbinfo in Global.cfc). The result
                    // query is delivered to the (possibly dotted) `name`
                    // variable via pending_result_writeback; the call itself
                    // returns null, like Lucee's tag.
                    let mut attrs: ValueMap = match args.first() {
                        Some(CfmlValue::Struct(s)) => s.snapshot(),
                        _ => ValueMap::default(),
                    };
                    // Expand attributeCollection (explicit keys win).
                    let attr_coll = attrs
                        .keys()
                        .find(|k| k.eq_ignore_ascii_case("attributecollection"))
                        .cloned();
                    if let Some(coll_key) = attr_coll {
                        if let Some(CfmlValue::Struct(inner)) = attrs.shift_remove(&coll_key) {
                            for (ik, iv) in inner.snapshot().into_iter() {
                                if !attrs.keys().any(|mk| mk.eq_ignore_ascii_case(&ik)) {
                                    attrs.insert(ik, iv);
                                }
                            }
                        }
                    }
                    let name_attr = attrs
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("name"))
                        .map(|(_, v)| v.as_string())
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            CfmlError::runtime("Missing attribute [name] on cfdbinfo".to_string())
                        })?;
                    // Per-application datasource resolution (this.datasources /
                    // per-app cfconfig) — same routing queryExecute gets.
                    if !self.app_datasources.is_empty() || self.app_default_datasource.is_some() {
                        let current = attrs
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
                            .map(|(_, v)| v.as_string());
                        let new_url = match current {
                            Some(ref n) => self.resolve_app_datasource(n),
                            None => self.app_default_datasource.clone(),
                        };
                        if let Some(url) = new_url {
                            let key = attrs
                                .keys()
                                .find(|k| k.eq_ignore_ascii_case("datasource"))
                                .cloned()
                                .unwrap_or_else(|| "datasource".to_string());
                            attrs.insert(key, CfmlValue::string(url));
                        }
                    }
                    let impl_fn = self
                        .builtins
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("__dbinfo_impl"))
                        .map(|(_, v)| *v)
                        .ok_or_else(|| {
                            CfmlError::runtime(
                                "cfdbinfo: database features not enabled".to_string(),
                            )
                        })?;
                    let result = impl_fn(vec![CfmlValue::strukt(attrs)])?;
                    self.pending_result_writeback = Some(vec![(name_attr, result)]);
                    return Ok(CfmlValue::Null);
                }
                "__cftransaction_start" => {
                    if self.transaction_conn.is_some() || self.transaction_pending {
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
                    // Resolve a per-application datasource name to its URL so
                    // transactions honour this.datasources too (same as queries).
                    let datasource = self
                        .resolve_app_datasource(&datasource)
                        .unwrap_or(datasource);
                    if datasource.is_empty() {
                        // Lucee/ACF defer transaction-connection setup until a query
                        // actually runs — `transaction { ... }` around non-query code
                        // is allowed, and a query inside the block begins the
                        // transaction lazily on its own datasource. Mark the block
                        // pending so the queryExecute path performs that lazy begin.
                        self.transaction_pending = true;
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
                    self.transaction_pending = false;
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
                    self.transaction_pending = false;
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
                                    CfmlValue::Int(i) => Some(i as u64 * 1000),
                                    CfmlValue::Double(d) => Some((d * 1000.0) as u64),
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
                        // Named locks are reentrant within the same request/thread:
                        // if this VM already holds a lock by this name, re-acquiring
                        // it must succeed immediately rather than self-deadlock on the
                        // non-reentrant RwLock (Lucee/ACF/BoxLang all permit same-thread
                        // re-entry). Push a placeholder that __cflock_end pops in LIFO
                        // order without releasing the real guard.
                        if self.held_locks.iter().any(|(n, _)| *n == lock_name) {
                            self.held_locks.push((lock_name, HeldLock::Reentrant));
                            return Ok(CfmlValue::Null);
                        }
                        // Get or create the named lock
                        let lock = {
                            let mut locks = server_state.named_locks.lock().unwrap();
                            const NAMED_LOCK_CAP: usize = 1024;
                            evict_idle_named_locks(&mut locks, lock_name.as_str(), NAMED_LOCK_CAP);
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
                            cookie.push_str(&format!(
                                "; Expires={}",
                                format_cookie_expires(&expires.as_string())
                            ));
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
                        .unwrap_or(CfmlValue::strukt(ValueMap::default()));

                    if let CfmlValue::Struct(form) = form_scope {
                        let field_lower = form_field.to_lowercase();
                        if let Some(CfmlValue::Struct(file_info)) = form
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&field_lower))
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
                                        let mut result = file_info.snapshot();
                                        result.insert(
                                            "serverDirectory".to_string(),
                                            CfmlValue::string(destination),
                                        );
                                        result.insert(
                                            "serverFile".to_string(),
                                            CfmlValue::string(
                                                final_path
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .to_string(),
                                            ),
                                        );
                                        result.insert(
                                            "fileWasSaved".to_string(),
                                            CfmlValue::Bool(true),
                                        );
                                        return Ok(CfmlValue::strukt(result));
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
                        .unwrap_or(CfmlValue::strukt(ValueMap::default()));

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
                                    let mut result = file_info.snapshot();
                                    result.insert(
                                        "serverDirectory".to_string(),
                                        CfmlValue::string(destination.clone()),
                                    );
                                    result.insert(
                                        "serverFile".to_string(),
                                        CfmlValue::string(
                                            final_path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .to_string(),
                                        ),
                                    );
                                    result
                                        .insert("fileWasSaved".to_string(), CfmlValue::Bool(true));
                                    results.push(CfmlValue::strukt(result));
                                }
                            }
                        }
                    }
                    return Ok(CfmlValue::array(results));
                }
                "sessioninvalidate" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        state.sessions.remove(self.current_application_name.as_deref().unwrap_or(""), sid);
                    }
                    // Drop the live scope so subsequent reads see an empty session.
                    self.session_scope = None;
                    return Ok(CfmlValue::Null);
                }
                "sessionrotate" => {
                    // Flush any pending live-scope writes to the old record first
                    // so rotate migrates the current data, then migrate + re-attach
                    // the live scope from the new id.
                    self.sync_session_scope_to_store()?;
                    if let (Some(ref state), Some(ref old_sid)) =
                        (&self.server_state, &self.session_id)
                    {
                        let new_sid = uuid::Uuid::new_v4().to_string();
                        state.sessions.rotate(self.current_application_name.as_deref().unwrap_or(""), old_sid, &new_sid);
                        self.session_id = Some(new_sid);
                    }
                    self.session_scope = None;
                    self.attach_session_scope();
                    return Ok(CfmlValue::Null);
                }
                "sessiongetmetadata" => {
                    let mut meta = ValueMap::default();
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(self.current_application_name.as_deref().unwrap_or(""), sid) {
                            let now = now_epoch_secs();
                            meta.insert(
                                "sessionId".to_string(),
                                CfmlValue::string(sid.clone()),
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
                "applicationstop" => {
                    self.stop_current_application();
                    return Ok(CfmlValue::Null);
                }
                "getauthuser" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(self.current_application_name.as_deref().unwrap_or(""), sid) {
                            if let Some(ref user) = session.auth_user {
                                return Ok(CfmlValue::string(user.clone()));
                            }
                        }
                    }
                    return Ok(CfmlValue::string(String::new()));
                }
                "isuserloggedin" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        if let Some(session) = state.sessions.get(self.current_application_name.as_deref().unwrap_or(""), sid) {
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
                        if let Some(session) = state.sessions.get(self.current_application_name.as_deref().unwrap_or(""), sid) {
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
                        let app = self.current_application_name.clone().unwrap_or_default();
                        let mut session = state.sessions.get(&app, sid).unwrap_or_else(|| SessionData {
                            variables: ValueMap::default(),
                            created_secs: now,
                            last_accessed_secs: now,
                            auth_user: None,
                            auth_roles: Vec::new(),
                            timeout_secs: 1800,
                            app_name: app.clone(),
                        });
                        session.auth_user = Some(name);
                        session.auth_roles = roles;
                        state.sessions.set(&app, sid, session);
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cflogout" => {
                    if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id)
                    {
                        let app = self.current_application_name.as_deref().unwrap_or("");
                        if let Some(mut session) = state.sessions.get(app, sid) {
                            session.auth_user = None;
                            session.auth_roles.clear();
                            state.sessions.set(app, sid, session);
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
                                if let Some(val) = self.request_scope.get_ci(key) {
                                    return Ok(val);
                                }
                                return Ok(CfmlValue::Null);
                            }
                            "session" => {
                                if let CfmlValue::Struct(s) = self.get_session_scope() {
                                    if let Some(val) = s
                                        .iter()
                                        .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                                        .map(|(_, v)| v.clone())
                                    {
                                        return Ok(val);
                                    }
                                }
                                return Ok(CfmlValue::Null);
                            }
                            "application" => {
                                if let Some(ref app_scope) = self.application_scope {
                                    if let Some(val) = app_scope.get_ci(key) {
                                        return Ok(val);
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&var_lower))
                        .map(|(_, v)| v.clone())
                    {
                        return Ok(val);
                    }
                    // Request scope
                    if let Some(val) = self.request_scope.get_ci(&var_lower) {
                        return Ok(val);
                    }
                    // Session scope
                    if let CfmlValue::Struct(s) = self.get_session_scope() {
                        if let Some(val) = s
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&var_lower))
                            .map(|(_, v)| v.clone())
                        {
                            return Ok(val);
                        }
                    }
                    // Application scope
                    if let Some(ref app_scope) = self.application_scope {
                        if let Some(val) = app_scope.get_ci(&var_lower) {
                            return Ok(val);
                        }
                    }
                    // Globals
                    if let Some(val) = self
                        .globals
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&var_lower))
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
                        self.set_session_variable(&key, value.clone())?;
                    } else if var_lower.starts_with("application.") {
                        let key = var_name[12..].to_string();
                        if let Some(ref app_scope) = self.application_scope {
                            app_scope.insert(key, value.clone());
                        }
                    } else {
                        // Default: set in variables (globals) scope
                        self.globals.insert(var_name, value.clone());
                    }
                    return Ok(value);
                }
                "throw" => {
                    // throw(message, type, detail, errorcode, extendedinfo, object)
                    // The positional order is fixed by the parser's throw(...) call
                    // lowering (see parser.rs). `object` re-throws an existing
                    // exception object (CF/Lucee/BoxLang `cfthrow object=`): any
                    // struct it carries is preserved verbatim, with explicitly
                    // supplied attributes overriding its fields. Without this the
                    // standard re-throw idiom `throw(object=e)` lost the original
                    // message/type/detail and surfaced as a blank error — Wheels'
                    // onError default path (`Throw(object=arguments.exception)`)
                    // depended on it (GitHub issue #158).
                    let is_present = |v: &CfmlValue| match v {
                        CfmlValue::Null => false,
                        CfmlValue::String(s) => !s.is_empty(),
                        _ => true,
                    };

                    // When `object` is a struct, start from it so all of its keys
                    // (rootCause, stacktrace, custom fields, …) survive the throw.
                    let mut exception: ValueMap = match args.get(5) {
                        Some(CfmlValue::Struct(s)) => s.snapshot(),
                        _ => ValueMap::default(),
                    };
                    let object_supplied = matches!(args.get(5), Some(CfmlValue::Struct(_)));

                    // Helper: set `key` from the positional arg when it was supplied,
                    // otherwise keep whatever the object already had (or the default).
                    let apply = |exc: &mut ValueMap, key: &str, arg: Option<&CfmlValue>, default: &str| {
                        let provided = arg.filter(|v| is_present(v)).map(|v| v.as_string());
                        match provided {
                            Some(v) => {
                                exc.insert(key.to_string(), CfmlValue::string(v));
                            }
                            None => {
                                if !exc.keys().any(|k| k.eq_ignore_ascii_case(key)) {
                                    exc.insert(key.to_string(), CfmlValue::string(default.to_string()));
                                }
                            }
                        }
                    };
                    apply(&mut exception, "message", args.get(0), "");
                    apply(&mut exception, "type", args.get(1), "Application");
                    apply(&mut exception, "detail", args.get(2), "");
                    apply(&mut exception, "errorcode", args.get(3), "");
                    apply(&mut exception, "extendedinfo", args.get(4), "");
                    // Preserve the original tagcontext when re-throwing an object;
                    // otherwise capture the current location.
                    if !object_supplied
                        || !exception.keys().any(|k| k.eq_ignore_ascii_case("tagcontext"))
                    {
                        exception.insert("tagcontext".to_string(), self.build_tag_context());
                    }

                    let message = exception
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("message"))
                        .map(|(_, v)| v.as_string())
                        .unwrap_or_default();

                    Self::add_root_cause(&mut exception);
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
                    let mut result = ValueMap::default();
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
                        .map(|(k, _)| CfmlValue::string(k.clone()))
                        .collect();
                    return Ok(CfmlValue::array(ids));
                }

                // ---- cfcache tag handler ----
                "__cfcache" => {
                    // Stub/no-op; in serve mode could push Cache-Control header
                    return Ok(CfmlValue::Null);
                }

                // ---- <cfloop file="..." index="..."> line iterator ----
                // Returns the file's lines as a 1-based array, splitting on \n and
                // \r\n and preserving empty lines (so line numbers stay accurate —
                // Wheels' error page renders source context by line number). Routed
                // through the VFS like fileRead so it works in CLI and serve modes.
                // Without a `file=` handler `<cfloop file=…>` previously fell through
                // to `while(true)` and hung the request (GitHub issue #158).
                "__cfloop_file_lines" => {
                    let path = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    return match self.vfs.read_to_string(&path) {
                        Ok(content) => {
                            let lines: Vec<CfmlValue> = content
                                .lines()
                                .map(|l| CfmlValue::string(l.to_string()))
                                .collect();
                            Ok(CfmlValue::array(lines))
                        }
                        Err(e) => Err(CfmlError::runtime(format!(
                            "cfloop file: cannot read '{}': {}",
                            path, e
                        ))),
                    };
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
                                CfmlValue::Bool(b) => b,
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
                                            let mut result = ValueMap::default();
                                            result.insert(
                                                "output".to_string(),
                                                CfmlValue::string(stdout),
                                            );
                                            result.insert(
                                                "error".to_string(),
                                                CfmlValue::string(stderr),
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
                    let callback = match args.get(1) {
                        Some(c) => c.clone(),
                        None => return Ok(CfmlValue::Null),
                    };
                    let attributes = args.get(2).cloned();

                    // Real OS thread when a spawner is injected AND the feature
                    // is on; otherwise run synchronously inline (wasm / off).
                    #[cfg(feature = "real-threads")]
                    let spawn = self.thread_spawn_fn;
                    #[cfg(not(feature = "real-threads"))]
                    let spawn: Option<ThreadSpawnFn> = None;

                    if let Some(spawn_fn) = spawn {
                        let seed = self.build_thread_seed(callback, attributes);
                        let mut handle = spawn_fn(seed);
                        handle.name = thread_name.clone();
                        self.live_threads.insert(thread_name.to_lowercase(), handle);
                        // Pre-seed cfthread.NAME as RUNNING so reads before join
                        // see a live status rather than a missing key.
                        let mut meta = ValueMap::default();
                        meta.insert(
                            "status".to_string(),
                            CfmlValue::string("RUNNING".to_string()),
                        );
                        meta.insert(
                            "name".to_string(),
                            CfmlValue::string(thread_name.clone()),
                        );
                        let cf = self.get_or_create_cfthread_scope();
                        if let Some(ts) = cf.as_cfml_struct() {
                            ts.insert(thread_name.to_lowercase(), CfmlValue::strukt(meta));
                        }
                        return Ok(CfmlValue::Null);
                    }

                    // Inline fallback: run now on this VM and store immediately.
                    let r = self.run_thread_body(&callback, attributes, parent_locals);
                    self.store_cfthread_result(&thread_name, r);
                    return Ok(CfmlValue::Null);
                }
                // Also reached by the threadJoin() script BIF (same arg shape:
                // optional name, optional timeout-ms).
                "__cfthread_join" | "threadjoin" => {
                    // Join named thread(s), comma-separated; empty/absent name
                    // joins all currently-live threads.
                    let names: Vec<String> = match args.get(0).map(|v| v.as_string()) {
                        Some(n) if !n.trim().is_empty() => n
                            .split(',')
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        _ => self.live_threads.keys().cloned().collect(),
                    };
                    // Timeout in ms; 0 or absent = wait indefinitely.
                    let timeout_ms = args
                        .get(1)
                        .map(|v| v.as_string().parse::<i64>().unwrap_or(0))
                        .unwrap_or(0);
                    for name in names {
                        self.join_thread(&name, timeout_ms);
                    }
                    return Ok(CfmlValue::Null);
                }
                // Also reached by the threadTerminate() script BIF.
                "__cfthread_terminate" | "threadterminate" => {
                    // Request cooperative cancellation: flip the thread's cancel
                    // flag. The body aborts at its next loop back-edge; a later
                    // join then reports status TERMINATED. Harmless on an
                    // already-finished thread. (Rust threads can't be force-
                    // killed mid-instruction — documented Lucee divergence.)
                    if let Some(name) = args.get(0).map(|v| v.as_string()) {
                        if let Some(handle) = self.live_threads.get(&name.to_lowercase()) {
                            handle
                                .cancel
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }

                // ---- async kernel: runAsync + _schedule ----
                "runasync" => {
                    let callback = match args.get(0) {
                        Some(CfmlValue::Function(_)) => args[0].clone(),
                        _ => {
                            return Err(CfmlError::runtime(
                                "runAsync requires a function/closure argument".to_string(),
                            ));
                        }
                    };
                    // Optional second arg: a struct exposed inside the body as
                    // the `attributes` scope (same pattern as cfthread's
                    // attribute pass-through). This is the workaround for
                    // closure capture not carrying function values across the
                    // method-call boundary — callers thread the data through
                    // explicitly.
                    let attributes = match args.get(1) {
                        Some(v @ CfmlValue::Struct(_)) => Some(v.clone()),
                        _ => None,
                    };

                    #[cfg(feature = "real-threads")]
                    let spawn = self.thread_spawn_fn;
                    #[cfg(not(feature = "real-threads"))]
                    let spawn: Option<ThreadSpawnFn> = None;

                    if let Some(spawn_fn) = spawn {
                        let seed = self.build_thread_seed(callback, attributes);
                        let handle = spawn_fn(seed);
                        let fut = async_kernel::FutureNative::from_handle(handle);
                        return Ok(CfmlValue::NativeObject(Arc::new(RwLock::new(fut))));
                    }

                    // Inline fallback (wasm / real-threads off): run now, store
                    // a resolved Future. Output is captured inside run_thread_body.
                    let r = self.run_thread_body(&callback, attributes, parent_locals);
                    let fut = async_kernel::FutureNative::resolved(r);
                    return Ok(CfmlValue::NativeObject(Arc::new(RwLock::new(fut))));
                }
                "_schedule" => {
                    // _schedule(closure, delayMs [, everyMs] [, spacedMs])
                    // OR _schedule(closure, optsStruct) where opts may have
                    // delayMs / everyMs / spacedMs keys.
                    let callback = match args.get(0) {
                        Some(CfmlValue::Function(_)) => args[0].clone(),
                        _ => {
                            return Err(CfmlError::runtime(
                                "_schedule requires a function/closure argument".to_string(),
                            ));
                        }
                    };
                    let (delay_ms, every_ms, spaced_ms) = match args.get(1) {
                        Some(CfmlValue::Struct(s)) => {
                            let snap = s.snapshot();
                            (
                                async_kernel::struct_get_i64(&snap, "delayMs").unwrap_or(0),
                                async_kernel::struct_get_i64(&snap, "everyMs"),
                                async_kernel::struct_get_i64(&snap, "spacedMs"),
                            )
                        }
                        Some(v) => {
                            let d = v.as_string().parse::<i64>().unwrap_or(0);
                            let e = args.get(2).and_then(|v| v.as_string().parse::<i64>().ok());
                            let sp = args.get(3).and_then(|v| v.as_string().parse::<i64>().ok());
                            (d, e, sp)
                        }
                        None => (0, None, None),
                    };

                    #[cfg(feature = "real-threads")]
                    let spawn = self.thread_spawn_fn;
                    #[cfg(not(feature = "real-threads"))]
                    let spawn: Option<ThreadSpawnFn> = None;

                    // v1 supports ONE-SHOT scheduling (delayMs then run once).
                    // Periodic `everyMs`/`spacedMs` is deferred to v2 — would
                    // need a respawn driver that can't run a CFML closure
                    // without VM access. v1 ignores those params with a doc'd
                    // caveat; callers can compose via runAsync chains.
                    let _ = (every_ms, spaced_ms);

                    #[cfg(feature = "real-threads")]
                    if let Some(spawn_fn) = spawn {
                        let seed = self.build_thread_seed(callback, None);
                        let outer_cancel = seed.cancel_flag.clone();

                        if delay_ms <= 0 {
                            let handle = spawn_fn(seed);
                            let fut = async_kernel::FutureNative::from_handle(handle);
                            return Ok(CfmlValue::NativeObject(Arc::new(RwLock::new(fut))));
                        }

                        // Relay thread: sleep `delayMs`, then spawn the real
                        // cfthread worker via spawn_fn, then forward its
                        // ThreadResult out our own channel. The Future holds
                        // our channel's rx side so .get() blocks until the
                        // relay forwards.
                        let (tx, rx) = std::sync::mpsc::channel::<ThreadResult>();
                        let cancel_for_relay = outer_cancel.clone();
                        let join = std::thread::Builder::new()
                            .name("rustcfml-schedule-relay".to_string())
                            .spawn(move || {
                                // Cooperative-cancellable sleep: poll every
                                // 50ms so cancel() takes effect promptly.
                                let start = std::time::Instant::now();
                                let total =
                                    std::time::Duration::from_millis(delay_ms as u64);
                                let step = std::time::Duration::from_millis(50);
                                loop {
                                    if cancel_for_relay
                                        .load(std::sync::atomic::Ordering::Relaxed)
                                    {
                                        let _ = tx.send(ThreadResult {
                                            status: "TERMINATED".to_string(),
                                            ..Default::default()
                                        });
                                        return;
                                    }
                                    let elapsed = start.elapsed();
                                    if elapsed >= total {
                                        break;
                                    }
                                    let remaining = total - elapsed;
                                    std::thread::sleep(remaining.min(step));
                                }
                                let inner = spawn_fn(seed);
                                // Wait for the inner cfthread to publish.
                                if let Ok(res) = inner.rx.recv() {
                                    let _ = tx.send(res);
                                }
                                if let Some(j) = {
                                    let mut h = inner;
                                    h.join.take()
                                } {
                                    let _ = j.join();
                                }
                            })
                            .map_err(|e| {
                                CfmlError::runtime(format!(
                                    "_schedule: failed to spawn relay thread: {}",
                                    e
                                ))
                            })?;

                        let handle = ThreadHandle {
                            name: String::new(),
                            rx,
                            cancel: outer_cancel,
                            join: Some(join),
                            result: None,
                        };
                        let fut = async_kernel::FutureNative::from_handle(handle);
                        return Ok(CfmlValue::NativeObject(Arc::new(RwLock::new(fut))));
                    }

                    // Inline fallback (wasm / real-threads off): no real
                    // scheduling — run immediately and return a resolved
                    // Future. delay/period args ignored (no thread to park).
                    let _ = spawn;
                    let _ = delay_ms;
                    let r = self.run_thread_body(&callback, None, parent_locals);
                    let fut = async_kernel::FutureNative::resolved(r);
                    return Ok(CfmlValue::NativeObject(Arc::new(RwLock::new(fut))));
                }

                "getfunctioncalledname" => {
                    // The name the currently-executing UDF was invoked under.
                    // The top frame is this builtin's caller (getFunctionCalledName
                    // is a builtin and pushes no frame of its own).
                    let name = self
                        .call_stack
                        .last()
                        .map(|f| f.called_name.clone())
                        .unwrap_or_default();
                    return Ok(CfmlValue::string(name));
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
                            let mut s = ValueMap::default();
                            s.insert("Function".to_string(), CfmlValue::string(f.function));
                            s.insert("Template".to_string(), CfmlValue::string(f.template));
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
                    return Ok(CfmlValue::string(result));
                }

                "getpagecontext" => {
                    // Build the servlet-bridge page context from the request's
                    // CGI scope (serve mode) or Lucee's synthetic task-context
                    // defaults (bare CLI). getRequest()/getResponse() resolve to
                    // method-faithful shims dispatched in call_member_function.
                    let cgi = match self.globals.get("cgi") {
                        Some(CfmlValue::Struct(c)) => Some(c.snapshot()),
                        _ => None,
                    };
                    return Ok(java_shims::build_page_context_shim(cgi.as_ref()));
                }

                "__cfcustomtag" => {
                    // Self-closing custom tag: __cfcustomtag(path_spec, attrs_struct)
                    let path_spec = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let attrs_val = Self::merge_attribute_collection(
                        args.get(1)
                            .cloned()
                            .unwrap_or(CfmlValue::strukt(ValueMap::default())),
                    );
                    let run_end_phase = args.get(2).map(|v| v.is_true()).unwrap_or(false);

                    let resolved = self.resolve_custom_tag_path(&path_spec)?;
                    let mut this_tag = ValueMap::default();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::string("start".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(run_end_phase));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::string(String::new()),
                    );

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = ValueMap::default();
                    tag_locals.insert("attributes".to_string(), attrs_val.clone());
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    self.execute_custom_tag_template(&resolved, &tag_locals)?;

                    // `cfexit method="exittag"` in the start phase skips the end
                    // phase entirely (partial start-phase caller writes were still
                    // captured and propagate below).
                    let exit_tag = matches!(
                        self.last_exit_method.take().as_deref(),
                        Some("exittag")
                    );

                    let start_locals = self.captured_locals.clone().unwrap_or_default();

                    if run_end_phase && !exit_tag {
                        let caller_for_end = if let Some(CfmlValue::Struct(modified_caller)) =
                            start_locals.get("caller")
                        {
                            modified_caller.snapshot()
                        } else {
                            caller_snapshot.clone()
                        };

                        let mut this_tag = ValueMap::default();
                        this_tag.insert(
                            "executionmode".to_string(),
                            CfmlValue::string("end".to_string()),
                        );
                        this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(true));
                        this_tag.insert(
                            "generatedcontent".to_string(),
                            CfmlValue::string(String::new()),
                        );

                        let mut end_locals = start_locals;
                        if !end_locals
                            .keys()
                            .any(|key| key.eq_ignore_ascii_case("attributes"))
                        {
                            end_locals.insert("attributes".to_string(), attrs_val);
                        }
                        end_locals.insert("caller".to_string(), CfmlValue::strukt(caller_for_end));
                        end_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                        let outer_output = std::mem::take(&mut self.output_buffer);
                        self.execute_custom_tag_template(&resolved, &end_locals)?;
                        let end_output = std::mem::take(&mut self.output_buffer);
                        self.output_buffer = outer_output;

                        if let Some(ref captured) = self.captured_locals {
                            if let Some(CfmlValue::Struct(tag_info)) = captured
                                .iter()
                                .rev()
                                .find(|(key, _)| key.eq_ignore_ascii_case("thistag"))
                                .map(|(_, value)| value)
                            {
                                if let Some(CfmlValue::String(content)) = tag_info
                                    .iter()
                                    .rev()
                                    .find(|(key, _)| key.eq_ignore_ascii_case("generatedcontent"))
                                    .map(|(_, value)| value)
                                {
                                    self.output_buffer.push_str(&content);
                                }
                            }
                        }
                        self.output_buffer.push_str(&end_output);
                    }

                    // Caller write-back: read modified caller from captured_locals.
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(wb) =
                            Self::caller_writeback_from_captured(captured, &caller_snapshot)
                        {
                            self.closure_parent_writeback = Some(wb);
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "__cfcustomtag_start" => {
                    // Body custom tag start: __cfcustomtag_start(path_spec, attrs_struct)
                    let path_spec = args.get(0).map(|v| v.as_string()).unwrap_or_default();
                    let attrs_val = Self::merge_attribute_collection(
                        args.get(1)
                            .cloned()
                            .unwrap_or(CfmlValue::strukt(ValueMap::default())),
                    );

                    let resolved = self.resolve_custom_tag_path(&path_spec)?;

                    let mut this_tag = ValueMap::default();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::string("start".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(true));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::string(String::new()),
                    );

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = ValueMap::default();
                    tag_locals.insert("attributes".to_string(), attrs_val.clone());
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    self.execute_custom_tag_template(&resolved, &tag_locals)?;

                    // `cfexit method="exittag"` in the start phase: the body and
                    // the end phase are skipped, but partial start-phase caller
                    // writes were still captured and must propagate (below).
                    let exit_tag = matches!(
                        self.last_exit_method.take().as_deref(),
                        Some("exittag")
                    );

                    let start_locals = self.captured_locals.clone().unwrap_or_default();

                    // Caller write-back from start execution
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(wb) =
                            Self::caller_writeback_from_captured(captured, &caller_snapshot)
                        {
                            self.closure_parent_writeback = Some(wb);
                        }
                    }

                    // Push state for end tag
                    self.custom_tag_stack.push(CustomTagState {
                        template_path: resolved,
                        attributes: attrs_val,
                        start_locals,
                        exit_tag,
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

                    // `cfexit method="exittag"` in the start phase: drop the
                    // captured body output and skip the end phase. Start-phase
                    // caller writes already propagated from `__cfcustomtag_start`.
                    if state.exit_tag {
                        let _ = body_content;
                        return Ok(CfmlValue::Null);
                    }

                    let mut this_tag = ValueMap::default();
                    this_tag.insert(
                        "executionmode".to_string(),
                        CfmlValue::string("end".to_string()),
                    );
                    this_tag.insert("hasendtag".to_string(), CfmlValue::Bool(true));
                    this_tag.insert(
                        "generatedcontent".to_string(),
                        CfmlValue::string(body_content),
                    );

                    let CustomTagState {
                        template_path,
                        attributes,
                        start_locals,
                        exit_tag: _,
                    } = state;

                    let caller_snapshot = parent_locals.clone();
                    let mut tag_locals = start_locals;
                    if !tag_locals
                        .keys()
                        .any(|key| key.eq_ignore_ascii_case("attributes"))
                    {
                        tag_locals.insert("attributes".to_string(), attributes);
                    }
                    tag_locals.insert(
                        "caller".to_string(),
                        CfmlValue::strukt(caller_snapshot.clone()),
                    );
                    tag_locals.insert("thistag".to_string(), CfmlValue::strukt(this_tag));

                    let outer_output = std::mem::take(&mut self.output_buffer);
                    self.execute_custom_tag_template(&template_path, &tag_locals)?;
                    let end_output = std::mem::take(&mut self.output_buffer);
                    self.output_buffer = outer_output;

                    // Read back generatedContent and append to output
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(CfmlValue::Struct(tag_info)) = captured
                            .iter()
                            .rev()
                            .find(|(key, _)| key.eq_ignore_ascii_case("thistag"))
                            .map(|(_, value)| value)
                        {
                            if let Some(CfmlValue::String(content)) = tag_info
                                .iter()
                                .rev()
                                .find(|(key, _)| key.eq_ignore_ascii_case("generatedcontent"))
                                .map(|(_, value)| value)
                            {
                                self.output_buffer.push_str(&content);
                            }
                        }
                    }
                    self.output_buffer.push_str(&end_output);

                    // Caller write-back from end execution
                    if let Some(ref captured) = self.captured_locals {
                        if let Some(wb) =
                            Self::caller_writeback_from_captured(captured, &caller_snapshot)
                        {
                            self.closure_parent_writeback = Some(wb);
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
        // Recurse into the nested struct. The child is fetched as an owned
        // handle; because structs/arrays are reference-typed, mutating through
        // it propagates back into `root` without a write-back.
        if let Some(s) = root.as_cfml_struct() {
            if let Some(mut child) = s.get(&path[0]) {
                Self::deep_set(&mut child, &path[1..], value);
            }
        }
    }

    /// Load a variable by name, checking locals, globals, and special scopes (application, request, local/variables).
    fn scope_aware_load(
        &self,
        name: &str,
        locals: &ValueMap,
    ) -> Option<CfmlValue> {
        let name_lower = name.to_lowercase();
        if name_lower == "local" {
            // `local` is always the function-local scope. Strip the internal
            // bridge keys (`__variables`, the `arguments` scope under
            // ARGUMENTS_SCOPE_KEY, …) — they are not local variables.
            let mut scope = locals.clone();
            scope.retain(|k, _| !k.starts_with("__"));
            return Some(CfmlValue::strukt(scope));
        }
        if name_lower == "arguments" {
            // The arguments scope lives under the reserved key; a literal
            // "arguments" key (if any) is a user local var, handled below.
            if let Some(v) = locals.get(ARGUMENTS_SCOPE_KEY) {
                return Some(v.clone());
            }
        }
        if name_lower == "variables" {
            if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                return Some(CfmlValue::Struct(vars.clone()));
            }
            return Some(CfmlValue::strukt(locals.clone()));
        }
        if name_lower == "application" {
            if let Some(ref app_scope) = self.application_scope {
                // Live handle clone (Lucee scope-reference semantics).
                return Some(CfmlValue::Struct(app_scope.clone()));
            }
        }
        if name_lower == "request" {
            return Some(CfmlValue::strukt(self.request_scope.snapshot()));
        }
        if name_lower == "session" {
            // Session must resolve to the live/persisted scope, not a stray
            // `session` local — otherwise a nested write (`session.a.b = v`,
            // routed here via store_runtime_path) loads an empty struct and
            // its mutation never reaches the persisted SessionData.
            return Some(self.get_session_scope());
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
        locals: &ValueMap,
    ) -> Option<CfmlValue> {
        // Bare `arguments` resolves to the scope stored under the reserved key
        // (independent of any user `local.arguments` var at the literal key).
        if name_lower == "arguments" {
            if let Some(v) = locals.get(ARGUMENTS_SCOPE_KEY) {
                return Some(v.clone());
            }
        }
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
        locals: &mut ValueMap,
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
                    // Self-alias guard (see StoreLocal application): skip storing
                    // the live handle back onto itself to avoid a reentrant lock.
                    if s.backing_ptr() != app_scope.backing_ptr() {
                        let snap = s.snapshot();
                        app_scope.with_write(|m| *m = snap);
                    }
                }
            }
        } else if name_lower == "request" {
            if let CfmlValue::Struct(s) = &val {
                self.request_scope.with_write(|m| *m = s.snapshot());
            }
        } else if name_lower == "session" {
            // Commit the (whole) session scope back so nested writes routed
            // through store_runtime_path persist into the SessionData. `val`
            // is the full session struct loaded via get_session_scope above,
            // walked + leaf-inserted, so a full snapshot replace is correct.
            if let CfmlValue::Struct(s) = &val {
                let _ = self.set_session_scope(s.snapshot());
            }
        } else if name_lower == "arguments" && locals.contains_key(ARGUMENTS_SCOPE_KEY) {
            // Write back the arguments scope under its reserved key (so an
            // `arguments.x = …` round-trip doesn't fork a literal "arguments"
            // key, which belongs to a user `local.arguments` var).
            locals.insert(ARGUMENTS_SCOPE_KEY.to_string(), val);
        } else if locals.contains_key(name) {
            locals.insert(name.to_string(), val);
        } else if self.globals.contains_key(name) {
            self.globals.insert(name.to_string(), val);
        } else {
            locals.insert(name.to_string(), val);
        }
    }

    /// Assign a value to a runtime-resolved, possibly dotted variable path
    /// ("rr", "local.wheels.result", "variables.x.y", "request.foo", …),
    /// auto-vivifying intermediate structs. Shared by `SetDynamicVar`
    /// (dynamic quoted-LHS assignment) and the pending-result writeback
    /// (queryExecute `result` / cfquery `name`/`result` delivery).
    fn store_runtime_path(
        &mut self,
        path: &str,
        value: CfmlValue,
        locals: &mut ValueMap,
    ) {
        // CFML null-assignment: storing Null through a scope path (a dynamic
        // `"variables.x" = voidFn()` LHS, or a null result-writeback delivery)
        // DELETES the target rather than materializing a null-valued key.
        if matches!(value, CfmlValue::Null) {
            self.delete_scope_path(path, locals);
            return;
        }
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() >= 2 {
            let scope = parts[0];
            let root = self
                .scope_aware_load(scope, locals)
                .unwrap_or_else(|| CfmlValue::strukt(ValueMap::default()));
            let root = if matches!(root, CfmlValue::Struct(_)) {
                root
            } else {
                // Leading segment exists but isn't a struct — replace it,
                // matching auto-vivification on a normal dotted assignment.
                CfmlValue::strukt(ValueMap::default())
            };
            if let CfmlValue::Struct(ref s) = root {
                // Walk/auto-vivify intermediate structs, set the leaf.
                let mut cur = s.clone();
                for key in &parts[1..parts.len() - 1] {
                    cur = cur.get_or_insert_struct(key);
                }
                cur.insert(parts[parts.len() - 1].to_string(), value);
            }
            // Write the (possibly copied) scope container back. For
            // reference-typed scopes (a CFC's __variables) the leaf is
            // already mutated in place; for page scope this commits the
            // new key into locals.
            self.scope_aware_store(scope, root, locals);
        } else {
            // Bare name (no scope prefix) — single-variable store.
            self.scope_aware_store(path, value, locals);
        }
    }

    /// Delete a scope-qualified / nested variable path, the deletion half of
    /// CFML null-assignment semantics (`variables.x = voidFn()` removes the key
    /// rather than materializing a null one). The bare-name case is handled
    /// inline in the `UnsetPath` opcode (it also clears globals + closure env);
    /// this covers `<scope>.leaf` and deeper `<scope>.a.b…leaf` paths.
    fn delete_scope_path(&mut self, path: &str, locals: &mut ValueMap) {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() < 2 {
            imap_remove_ci(locals, path);
            if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                vars.remove_ci(path);
            }
            imap_remove_ci(&mut self.globals, path);
            return;
        }
        let scope = parts[0];
        let leaf = parts[parts.len() - 1];

        // Single-level scoped target: <scope>.<leaf>.
        if parts.len() == 2 {
            match scope.to_lowercase().as_str() {
                "local" => {
                    imap_remove_ci(locals, leaf);
                }
                "variables" => {
                    if let Some(CfmlValue::Struct(vars)) = locals.get("__variables") {
                        // CFC method: the component scope is a live handle.
                        vars.remove_ci(leaf);
                    } else {
                        // Page scope: `variables` is the frame's locals (+globals spill).
                        imap_remove_ci(locals, leaf);
                        let found = self
                            .globals
                            .keys()
                            .find(|k| k.eq_ignore_ascii_case(leaf))
                            .cloned();
                        if let Some(k) = found {
                            self.globals.shift_remove(&k);
                        }
                    }
                }
                "request" => {
                    self.request_scope.remove_ci(leaf);
                }
                "application" => {
                    if let Some(ref app) = self.application_scope {
                        app.remove_ci(leaf);
                    }
                }
                "session" => {
                    if let CfmlValue::Struct(s) = self.get_session_scope() {
                        let mut m = s.snapshot();
                        imap_remove_ci(&mut m, leaf);
                        let _ = self.set_session_scope(m);
                    }
                }
                _ => {
                    // Plain struct variable: `obj.member`. Reference-typed structs
                    // mutate in place; store back covers a value-typed page scope.
                    if let Some(obj) = self.scope_aware_load(scope, locals) {
                        if let Some(s) = obj.as_cfml_struct() {
                            s.remove_ci(leaf);
                        }
                        self.scope_aware_store(scope, obj, locals);
                    }
                }
            }
            return;
        }

        // Nested target: `<scope>.a.b…leaf`. Walk to the parent container — every
        // intermediate is a reference-typed struct, so removing the leaf in place
        // is visible through the scope. A missing link means nothing to delete.
        if let Some(root) = self.scope_aware_load(scope, locals) {
            // `root.clone()` shares the Arc backing (CfmlValue::Struct is
            // reference-typed), so the in-place `remove_ci` below is reflected
            // in `root` — which we commit back at the end.
            let mut cur = root.clone();
            for key in &parts[1..parts.len() - 1] {
                match cur.as_cfml_struct().and_then(|s| s.get_ci(key)) {
                    Some(next) => cur = next,
                    None => return,
                }
            }
            if let Some(s) = cur.as_cfml_struct() {
                s.remove_ci(leaf);
            }
            // Commit the scope container back so the deletion persists for scopes
            // whose load returns a snapshot copy rather than a live handle — e.g.
            // `session` before it is attached, or a value-typed page scope.
            // Mirrors the set path in store_runtime_path; for already-reference-typed
            // scopes the leaf is removed in place and this re-commit is a no-op.
            self.scope_aware_store(scope, root, locals);
        }
    }

    /// Apply (and clear) any caller-scope deliveries requested by the call
    /// that just returned — see `pending_result_writeback`.
    ///
    /// A delivery that lands in the frame's locals must also RECLAIM the key
    /// into this frame's `local` view (PR #93 tracking) — exactly like a
    /// `local.x = ...` assignment — otherwise a same-named key inherited from
    /// the caller/captured scope keeps the delivered variable invisible to
    /// `local.x` reads.
    fn apply_pending_result_writeback(
        &mut self,
        locals: &mut ValueMap,
        inherited_or_param_keys: &mut std::collections::HashSet<String>,
    ) {
        if let Some(sets) = self.pending_result_writeback.take() {
            for (path, value) in sets {
                self.store_runtime_path(&path, value, locals);
                let parts: Vec<&str> = path.split('.').collect();
                if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("local") {
                    inherited_or_param_keys.remove(parts[1]);
                } else if parts.len() == 1 && locals.contains_key(parts[0]) {
                    inherited_or_param_keys.remove(parts[0]);
                }
            }
        }
    }

    /// Reject calls that mix positional and named arguments.
    ///
    /// CFML (matching Lucee) requires that once any argument is named, every
    /// argument must be named — `f(a, name=b)` is an error, not a lenient
    /// best-effort bind. `names` carries one entry per call-site argument: an
    /// empty string marks a positional argument, a non-empty string a named one.
    /// Returns an `Expression` error when both kinds are present.
    fn validate_named_args(names: &[String]) -> Result<(), CfmlError> {
        let any_named = names.iter().any(|n| !n.is_empty());
        let any_positional = names.iter().any(|n| n.is_empty());
        if any_named && any_positional {
            return Err(CfmlError::new(
                "When using named parameters to a function, all parameters must be named. \
                 Either name all arguments (e.g., argumentName=value) or use positional \
                 arguments only."
                    .to_string(),
                CfmlErrorType::Expression,
            ));
        }
        Ok(())
    }

    /// Rebind call-site method arguments to a function's parameters by name.
    ///
    /// `arg_names` carries one entry per positional value in `arg_values`: an
    /// empty string means the argument was passed positionally, a non-empty
    /// string is its call-site name. An `argumentCollection` entry whose value
    /// is a struct is expanded into named arguments. The result is the values
    /// ordered to match `func.params`, with any unmatched named arguments
    /// appended (so the callee's `arguments` scope still receives them).
    ///
    /// When `arg_names` is `None` (a plain CallMethod) or the receiver is not a
    /// function, the values are returned untouched. Mirrors the inline logic in
    /// the `CallNamed` handler for free-function calls.
    /// Builtins whose script-form invocation takes a single struct of attrs
    /// (matching the corresponding cf<tag>): `cfdirectory(action="list", ...)`
    /// is shorthand for the tag and is bundled into one struct argument.
    fn is_tag_call_builtin(name: &str) -> bool {
        matches!(
            name.to_lowercase().as_str(),
            "cfdirectory"
                | "__cfdirectory"
                | "cffile"
                | "cfhttp"
                | "cfmail"
                | "__cfmail"
                | "cfdbinfo"
                | "dbinfo"
                // Script-form response/control tags. These resolve to a
                // `__cfX` builtin that consumes a single struct-of-options
                // (see the intercepts in `call_function`). Without bundling,
                // the script named-arg form `cfheader(name=…, value=…)` drops
                // every attribute and silently no-ops — only the
                // `attributeCollection={…}` single-struct form reached the
                // intercept. (GitHub issue #141.)
                | "__cfheader"
                | "__cfcontent"
                | "__cflocation"
                | "__cfcookie"
                | "__cflog"
                | "__cfsetting"
                // cfhtmlhead/cfhtmlbody exist as tags (v0.186); the script
                // call form `cfhtmlhead(text=…)` bundles its named args into a
                // single options struct that the intercept unwraps. (PR #161.)
                | "__cfhtmlhead"
                | "__cfhtmlbody"
        )
    }

    /// Attribute names whose value (a string) is the caller-scope variable
    /// the tag-call writes its return value back to (e.g. `name="dirQ"` on
    /// `cfdirectory(...)` populates `dirQ` with the listing query). `cfhttp`
    /// uses `result` instead — without it the bare statement form
    /// `cfhttp(url=…, result="r")` silently dropped its return struct (the
    /// block form `cfhttp(…){…}` is rewritten to an explicit assignment in the
    /// parser, so only the no-block call form was affected).
    fn tag_call_writeback_attr(tag: &str) -> &'static [&'static str] {
        let tag = tag.trim_start_matches("__");
        match tag.to_lowercase().as_str() {
            "cfhttp" => &["result"],
            // Response/control tags have no return-value write-back: their
            // `name` attribute is a real option (header/cookie name), not a
            // caller-scope target.
            "cfheader" | "cfcontent" | "cflocation" | "cfcookie" | "cflog"
            | "cfsetting" | "cfhtmlhead" | "cfhtmlbody" => &[],
            _ => &["name", "variable"],
        }
    }

    /// Reorder named args to declared-param positions and surface the named
    /// args that overflow past those params. The VM stashes the extras in
    /// `pending_extra_named_args` so the callee's `arguments` scope keeps
    /// their original names — matching Lucee/ACF/BoxLang.
    fn reorder_named_args_with_extras(
        func_ref: &CfmlValue,
        arg_names: Option<&[String]>,
        arg_values: Vec<CfmlValue>,
    ) -> (Vec<CfmlValue>, Vec<(usize, String)>) {
        let Some(arg_names) = arg_names else {
            return (arg_values, Vec::new());
        };
        let CfmlValue::Function(func) = func_ref else {
            return (arg_values, Vec::new());
        };

        // Expand argumentCollection structs into individual named arguments.
        let mut expanded_names = Vec::with_capacity(arg_names.len());
        let mut expanded_values = Vec::with_capacity(arg_names.len());
        for (i, name) in arg_names.iter().enumerate() {
            let value = arg_values.get(i).cloned().unwrap_or(CfmlValue::Null);
            if name.eq_ignore_ascii_case("argumentcollection") {
                if let CfmlValue::Struct(s) = &value {
                    for (k, v) in s.iter() {
                        expanded_names.push(k.clone());
                        expanded_values.push(v.clone());
                    }
                    continue;
                }
            }
            expanded_names.push(name.clone());
            expanded_values.push(value);
        }

        // Size to the declared params only; positional overflow and unmatched
        // named args are appended below. Padding to expanded_names.len() created
        // spurious empty slots that leaked into the arguments scope as numeric
        // keys when a paramless function was called purely by name.
        let mut positional = vec![CfmlValue::Null; func.params.len()];
        let mut extras: Vec<(usize, String)> = Vec::new();
        for (i, name) in expanded_names.iter().enumerate() {
            let value = expanded_values.get(i).cloned().unwrap_or(CfmlValue::Null);
            if name.is_empty() {
                // Positional arg: fill its slot, or append when it overflows the
                // declared params.
                if i < positional.len() {
                    positional[i] = value;
                } else {
                    positional.push(value);
                }
                continue;
            }
            match func
                .params
                .iter()
                .position(|param| param.name.eq_ignore_ascii_case(name))
            {
                Some(param_index) if param_index < positional.len() => {
                    positional[param_index] = value;
                }
                Some(_) => {}
                None => {
                    let idx = positional.len();
                    positional.push(value);
                    extras.push((idx, name.clone()));
                }
            }
        }
        (positional, extras)
    }

    /// Bind an `invoke(o, m, argStruct)` arg struct to a target method's
    /// declared params. Special-cases a top-level `argumentCollection` key —
    /// its inner struct is spread into the callee's arguments scope, matching
    /// Lucee/ACF/BoxLang. Keys that don't match a declared param are appended
    /// as extras (via `pending_extra_named_args`) so they surface in
    /// `arguments` by name even when the callee declares no params.
    fn build_invoke_call_args(
        &mut self,
        func: &CfmlValue,
        invoke_args: CfmlValue,
    ) -> Vec<CfmlValue> {
        let CfmlValue::Struct(arg_map) = invoke_args else {
            return match invoke_args {
                CfmlValue::Null => Vec::new(),
                other => vec![other],
            };
        };
        if arg_map.is_empty() {
            return Vec::new();
        }
        let CfmlValue::Function(f) = func else {
            return Vec::new();
        };

        let param_names: Vec<String> = if !f.params.is_empty() {
            f.params.iter().map(|p| p.name.clone()).collect()
        } else if let cfml_common::dynamic::CfmlClosureBody::Expression(ref body) = f.body {
            if let CfmlValue::Int(idx) = body.as_ref() {
                self.resolve_fn(*idx)
                    .map(|bf| bf.params.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Expand a top-level `argumentCollection` struct key into its inner
        // entries; the literal key itself never reaches the callee.
        let mut flat: Vec<(String, CfmlValue)> = Vec::with_capacity(arg_map.len());
        for (k, v) in arg_map.iter() {
            if k.eq_ignore_ascii_case("argumentcollection") {
                if let CfmlValue::Struct(inner) = v {
                    for (ik, iv) in inner.iter() {
                        flat.push((ik.clone(), iv.clone()));
                    }
                    continue;
                }
                // Non-struct argumentCollection: drop it (Lucee errors;
                // dropping keeps the literal key from leaking through).
                continue;
            }
            flat.push((k.clone(), v.clone()));
        }

        let mut positional: Vec<CfmlValue> = vec![CfmlValue::Null; param_names.len()];
        let mut consumed = vec![false; flat.len()];
        // Pass 1: a NUMERIC string key ("1", "2", ...) forwards as the Nth
        // POSITIONAL argument (1-based), matching Lucee/ACF/BoxLang. This is
        // the shape Wheels' Global.cfc $invoke() builds when it rebuilds a
        // dynamic call's positional args into a numeric-keyed
        // argumentCollection; without this they were dropped as unmatched
        // named extras and the callee's positional param kept its default.
        for (fi, (fname, fval)) in flat.iter().enumerate() {
            if let Ok(n) = fname.trim().parse::<usize>() {
                if n >= 1 {
                    let pi = n - 1;
                    if pi >= positional.len() {
                        positional.resize(pi + 1, CfmlValue::Null);
                    }
                    positional[pi] = fval.clone();
                    consumed[fi] = true;
                }
            }
        }
        // Pass 2: named keys bind to declared params by (case-insensitive) name.
        for (pi, pname) in param_names.iter().enumerate() {
            for (fi, (fname, fval)) in flat.iter().enumerate() {
                if !consumed[fi] && fname.eq_ignore_ascii_case(pname) {
                    positional[pi] = fval.clone();
                    consumed[fi] = true;
                    break;
                }
            }
        }
        let mut extras: Vec<(usize, String)> = Vec::new();
        for (fi, (fname, fval)) in flat.iter().enumerate() {
            if !consumed[fi] {
                let idx = positional.len();
                positional.push(fval.clone());
                extras.push((idx, fname.clone()));
            }
        }
        if !extras.is_empty() {
            self.pending_extra_named_args = Some(extras);
        }
        positional
    }

    /// Build the `local` scope view for the current call frame. CFML
    /// defines `local` as strictly per-call: even when the caller's locals
    /// are propagated into the callee's frame (so the callee can read the
    /// page-scope `variables` and CFC bridge keys), the `local` scope
    /// itself must contain only keys established in THIS frame.
    ///
    /// PR #93: a callee that never declares `local.rv` must see
    /// `StructKeyExists(local, "rv") == false` even when the caller has a
    /// same-named `local.rv`. Filter `locals` to keys that aren't inherited
    /// from the parent and aren't function parameters (params live in
    /// `arguments`, not `local`). The CFC bridge keys (`this`, `super`,
    /// `__variables`, `__*` internals) and the `arguments` scope are also
    /// excluded.
    fn build_local_scope_view(
        locals: &ValueMap,
        inherited_or_param_keys: &std::collections::HashSet<String>,
    ) -> ValueMap {
        let mut out = ValueMap::default();
        for (k, v) in locals {
            if inherited_or_param_keys.contains(k) {
                continue;
            }
            // `this`/`super`/the `arguments` scope (ARGUMENTS_SCOPE_KEY) and
            // every other internal bridge key are `__`-prefixed or filtered
            // here. A literal "arguments" key is now a genuine user
            // `local.arguments` var and stays visible.
            if k == "this" || k == "super" || k.starts_with("__") {
                continue;
            }
            out.insert(k.clone(), v.clone());
        }
        out
    }

    /// Dispatch a method on the getPageContext() shim. getRequest()/getResponse()
    /// hand back the embedded servlet shims; the request-side accessors Lucee and
    /// BoxLang expose directly on the page context (getRequestURL, getMethod, …)
    /// are forwarded to the request shim so the surface is a superset of both.
    fn dispatch_page_context(
        &mut self,
        object: &CfmlValue,
        method_lower: &str,
        extra_args: &mut Vec<CfmlValue>,
    ) -> CfmlResult {
        let pc = match object {
            CfmlValue::Struct(s) => s,
            _ => return Ok(CfmlValue::Null),
        };
        match method_lower {
            "getrequest" | "gethttpservletrequest" => {
                Ok(pc.get("__pc_request").unwrap_or(CfmlValue::Null))
            }
            "getresponse" | "gethttpservletresponse" => {
                Ok(pc.get("__pc_response").unwrap_or(CfmlValue::Null))
            }
            // Response-side forwards (BoxLang exposes these on the page context).
            "getstatus" | "setstatus" | "getcontenttype" | "setcontenttype"
            | "addheader" | "setheader" | "containsheader" | "iscommitted"
            | "getcharacterencoding" | "encodeurl" | "encoderedirecturl"
            | "sendredirect" | "reset" | "resethtmlhead" => {
                // Forward to the response handler; pass the page context itself
                // as the receiver so a setter's write-back stays an identity
                // store on `pc` rather than nulling it.
                self.dispatch_servlet_response(object, method_lower, extra_args)
            }
            // Anything else (getRequestURL, getMethod, getQueryString, …) is a
            // request-side accessor — forward to the request shim.
            _ => {
                let req = pc.get("__pc_request").unwrap_or(CfmlValue::Null);
                let args = std::mem::take(extra_args);
                java_shims::handle_servlet_request(method_lower, args, &req)
            }
        }
    }

    /// Dispatch a method on the HttpServletResponse shim. State lives on the VM
    /// so setStatus()/setHeader() faithfully drive the real serve-mode response;
    /// in CLI mode they update the same fields harmlessly (matching Lucee's
    /// response dummy, which also doesn't reach a client).
    ///
    /// `receiver` is the shim the call was made on; the `set*` mutators return it
    /// (not their void/null result) so the CallMethod write-back — which fires
    /// for any `setXxx` method — stays a harmless identity store instead of
    /// overwriting the receiver variable with null.
    fn dispatch_servlet_response(
        &mut self,
        receiver: &CfmlValue,
        method_lower: &str,
        extra_args: &mut Vec<CfmlValue>,
    ) -> CfmlResult {
        let args = std::mem::take(extra_args);
        let header_get = |name: &str| {
            self.response_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        Ok(match method_lower {
            "getstatus" => {
                let code = self.response_status.as_ref().map(|(c, _)| *c).unwrap_or(200);
                CfmlValue::Int(code as i64)
            }
            "setstatus" => {
                let code = args.first().and_then(to_number).unwrap_or(200.0);
                let text = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                self.response_status = Some((code as u16, text));
                receiver.clone()
            }
            "getcontenttype" => header_get("Content-Type")
                .map(CfmlValue::string)
                .unwrap_or_else(|| CfmlValue::string("text/html;charset=UTF-8".to_string())),
            "setcontenttype" => {
                let v = args.first().map(|a| a.as_string()).unwrap_or_default();
                self.response_headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Type"));
                self.response_headers.push(("Content-Type".to_string(), v));
                receiver.clone()
            }
            "setheader" => {
                let name = args.first().map(|a| a.as_string()).unwrap_or_default();
                let val = args.get(1).map(|a| a.as_string()).unwrap_or_default();
                self.response_headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
                self.response_headers.push((name, val));
                receiver.clone()
            }
            "addheader" => {
                let name = args.first().map(|a| a.as_string()).unwrap_or_default();
                let val = args.get(1).map(|a| a.as_string()).unwrap_or_default();
                self.response_headers.push((name, val));
                CfmlValue::Null
            }
            "containsheader" => {
                let name = args.first().map(|a| a.as_string()).unwrap_or_default();
                CfmlValue::Bool(header_get(&name).is_some())
            }
            "sendredirect" => {
                let url = args.first().map(|a| a.as_string()).unwrap_or_default();
                self.response_headers.push(("Location".to_string(), url));
                self.response_status = Some((302, "Found".to_string()));
                CfmlValue::Null
            }
            "iscommitted" => CfmlValue::Bool(false),
            "getcharacterencoding" => CfmlValue::string("UTF-8".to_string()),
            "encodeurl" | "encoderedirecturl" => {
                args.into_iter().next().unwrap_or(CfmlValue::Null)
            }
            "reset" | "resethtmlhead" => CfmlValue::Null,
            _ => CfmlValue::Null,
        })
    }

    fn call_member_function(
        &mut self,
        object: &CfmlValue,
        method: &str,
        extra_args: &mut Vec<CfmlValue>,
        arg_names: Option<&[String]>,
    ) -> CfmlResult {
        let method_lower = method.to_lowercase();

        // A method call on a null receiver is an error in CFML (Lucee throws);
        // silently returning Null lets `<null>.save(...)` no-op and report fake
        // success (the Wheels create() failure mode). Null-safe `?.` calls never
        // reach here — codegen jumps over the CallMethod op on a null receiver.
        if matches!(object, CfmlValue::Null) {
            return Err(CfmlError::new(
                format!("cannot call method [{}] on a null value", method),
                CfmlErrorType::Expression,
            ));
        }

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

                // Servlet bridge: getPageContext() and its request/response
                // objects. Dispatched here (not via the generic shim match) so
                // the response side can read/write `self.response_status` /
                // `self.response_headers` for faithful serve-mode behaviour.
                match java_class.as_str() {
                    java_shims::SERVLET_PAGE_CONTEXT_CLASS => {
                        return self.dispatch_page_context(object, &method_lower, extra_args);
                    }
                    java_shims::SERVLET_REQUEST_CLASS => {
                        let args = std::mem::take(extra_args);
                        return java_shims::handle_servlet_request(&method_lower, args, object);
                    }
                    java_shims::SERVLET_RESPONSE_CLASS => {
                        return self.dispatch_servlet_response(object, &method_lower, extra_args);
                    }
                    _ => {}
                }

                // Special: Queue.poll() returns the head and mutates in place.
                // Set method_this_writeback so the bytecode CallMethod handler
                // writes the reduced queue back to the variable.
                if method_lower == "poll"
                    && java_class == "java.util.concurrent.concurrentlinkedqueue"
                {
                    if let Some(CfmlValue::Array(q)) = s.get("__queue") {
                        let qv = q.snapshot();
                        if qv.is_empty() {
                            return Ok(CfmlValue::Null);
                        }
                        let head = qv[0].clone();
                        let mut ns = s.snapshot();
                        ns.insert("__queue".to_string(), CfmlValue::array(qv[1..].to_vec()));
                        self.method_this_writeback = Some(CfmlValue::strukt(ns));
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
                    let old = s.get(&key).unwrap_or(CfmlValue::Null);
                    let mut ns = s.snapshot();
                    ns.shift_remove(&key);
                    self.method_this_writeback = Some(CfmlValue::strukt(ns));
                    return Ok(old);
                }

                // Special: Matcher.find()/matches()/lookingAt() advance the
                // matcher cursor and refresh its capture groups; the updated
                // matcher must be written back so a following group(n) reads
                // this step's match — same pattern as Queue.poll / Map.remove.
                if java_class == "java.util.regex.matcher"
                    && matches!(method_lower.as_str(), "find" | "matches" | "lookingat")
                {
                    let mode = match method_lower.as_str() {
                        "matches" => java_shims::MatchMode::Matches,
                        "lookingat" => java_shims::MatchMode::LookingAt,
                        _ => java_shims::MatchMode::Find,
                    };
                    let (matched, new_matcher) =
                        java_shims::java_matcher_step(s, mode).map_err(|e| self.wrap_error(e))?;
                    self.method_this_writeback = Some(new_matcher);
                    return Ok(CfmlValue::Bool(matched));
                }

                let all_args: Vec<CfmlValue> = std::mem::take(extra_args);
                let m = method_lower.clone();
                let result = match java_class.as_str() {
                    "java.security.messagedigest" => {
                        handle_java_messagedigest(&m, all_args, object)
                    }
                    "java.util.uuid" => handle_java_uuid(&m, all_args, object),
                    "java.util.date" => handle_java_date(&m, all_args, object),
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
                    "java.util.regex.pattern" | "java.util.regex.matcher" => {
                        handle_java_pattern(&m, all_args, object)
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
                        .map(|c| CfmlValue::string(c.to_string()))
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
                "gettoken" => Some("getToken"),
                "listfind" => Some("listFind"),
                "listcontains" => Some("listContains"),
                "listappend" => Some("listAppend"),
                "refind" => Some("reFind"),
                "refindnocase" => Some("reFindNoCase"),
                "rereplace" => Some("reReplace"),
                "rereplacenocase" => Some("reReplaceNoCase"),
                "rematch" => Some("reMatch"),
                "rematchnocase" => Some("reMatchNoCase"),
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
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            let keep = self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            acc = self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                    return Ok(arr.first().unwrap_or(CfmlValue::Null));
                }
                "last" => {
                    return Ok(arr.last().unwrap_or(CfmlValue::Null));
                }
                "isempty" => {
                    return Ok(CfmlValue::Bool(arr.is_empty()));
                }
                "tojson" | "serializejson" => Some("serializeJSON"),
                _ => None,
            },
            CfmlValue::Struct(s) => match method_lower.as_str() {
                // Struct member-function helpers must NEVER fire on a component.
                // A CFC is represented internally as a struct, but it is not a
                // struct to user code: a method named like a struct helper
                // (delete/count/find/insert/each/...) must resolve to the
                // component's own method, an inherited method, or onMissingMethod
                // — never to structDelete/structCount/etc. Returning None here
                // skips builtin mapping so dispatch falls through to the user
                // method / onMissingMethod path below. (This matches Lucee, where
                // struct member functions are not available on components.)
                _ if s.contains_key("__variables") || s.contains_key("__name") => None,
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
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                        let mut result = ValueMap::default();
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let mapped =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                        let mut result = ValueMap::default();
                        for (k, v) in s.iter() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                            cb_args.push(CfmlValue::string(k.clone()));
                            cb_args.push(v.clone());
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                    return Ok(CfmlValue::Int(q.row_count() as i64));
                }
                "columnlist" => {
                    // Uppercase column names, matching Lucee/ACF columnList.
                    return Ok(CfmlValue::string(q.column_list()));
                }
                "addrow" => Some("queryAddRow"),
                "getrow" => Some("queryGetRow"),
                "each" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, row) in q.rows().into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            self.call_function(&callback, cb_args, &ValueMap::default())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                        }
                    }
                    return Ok(CfmlValue::Null);
                }
                "map" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let snapshot = q.rows();
                        let mut new_rows = Vec::with_capacity(snapshot.len());
                        for (i, row) in snapshot.into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let mapped =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if let CfmlValue::Struct(s) = mapped {
                                new_rows.push(s.snapshot());
                            } else {
                                new_rows.push(row);
                            }
                        }
                        return Ok(CfmlValue::Query(CfmlQuery::from_parts(q.columns(), new_rows)));
                    }
                    return Ok(object.clone());
                }
                "filter" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut new_rows = Vec::new();
                        for (i, row) in q.rows().into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row.clone()));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let keep = self.call_function(&callback, cb_args, &ValueMap::default())?;
                            if let Some(ref wb) = self.closure_parent_writeback {
                                Self::write_back_to_captured_scope(&callback, wb);
                            }
                            if keep.is_true() {
                                new_rows.push(row);
                            }
                        }
                        return Ok(CfmlValue::Query(CfmlQuery::from_parts(q.columns(), new_rows)));
                    }
                    return Ok(object.clone());
                }
                "reduce" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        let mut acc = extra_args.get(1).cloned().unwrap_or(CfmlValue::Null);
                        for (i, row) in q.rows().into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(4);
                            cb_args.push(acc.clone());
                            cb_args.push(CfmlValue::strukt(row));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            acc = self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                        let mut rows = q.rows();
                        let n = rows.len();
                        for i in 0..n {
                            for j in 0..n.saturating_sub(1 + i) {
                                let a = CfmlValue::strukt(rows[j].clone());
                                let b = CfmlValue::strukt(rows[j + 1].clone());
                                let cb_args = vec![a, b];
                                self.closure_parent_writeback = None;
                                let cmp =
                                    self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                        // Member .sort() sorts the receiver IN PLACE (reference-typed).
                        q.with_write(|d| {
                            let cols = d.columns.clone();
                            *d = cfml_common::dynamic::CfmlQueryData::from_named_rows(cols, rows);
                        });
                        return Ok(object.clone());
                    }
                    return Ok(object.clone());
                }
                "some" => {
                    if let Some(callback) = extra_args.first().cloned() {
                        for (i, row) in q.rows().into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                        for (i, row) in q.rows().into_iter().enumerate() {
                            let mut cb_args = Vec::with_capacity(3);
                            cb_args.push(CfmlValue::strukt(row));
                            cb_args.push(CfmlValue::Int((i + 1) as i64));
                            cb_args.push(object.clone());
                            self.closure_parent_writeback = None;
                            let result =
                                self.call_function(&callback, cb_args, &ValueMap::default())?;
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
                    return Ok(CfmlValue::string(object.as_string()));
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
            // insert(substring, string, pos), reFind(pattern, string)), swap the
            // first two args so the object (which the member was called on)
            // becomes the second arg. Note reReplace(string, pattern, replace)
            // is string-first and intentionally NOT swapped.
            if matches!(object, CfmlValue::String(_)) && args.len() >= 2 {
                match name {
                    "find" | "findNoCase" | "insert" | "reFind" | "reFindNoCase"
                    | "reMatch" | "reMatchNoCase" => {
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
                .find(|(k, _)| k.eq_ignore_ascii_case(&name_lower))
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
                .find(|(k, _)| k.eq_ignore_ascii_case(&method_lower))
                .map(|(_, v)| v.clone())
                .unwrap_or(CfmlValue::Null)
        } else {
            object.get(method).unwrap_or(CfmlValue::Null)
        };
        if let CfmlValue::Function(ref _fdata) = &prop {
            let func_ref = prop.clone();
            let raw_args: Vec<CfmlValue> = extra_args.drain(..).collect();
            let (args, extras) =
                Self::reorder_named_args_with_extras(&prop, arg_names, raw_args);
            self.pending_extra_named_args =
                if extras.is_empty() { None } else { Some(extras) };
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
            let mut method_locals = ValueMap::default();
            // While a CFC method runs, relative component resolution (bare
            // `createObject("component","Sibling")` / `new Sibling()`) must
            // search the package of the component that LEXICALLY DEFINES the
            // method, not the caller's file. For an INHERITED method that is
            // the parent's directory, not the concrete subclass instance's —
            // so prefer the method's own BytecodeFunction.source_file (stamped
            // per-CFC at compile time) and only fall back to the instance's
            // __source_file when the function carries no source (e.g. a
            // dynamically-built closure). Mirrors resolve_inheritance's
            // sibling-parent swap; without the defining-source preference,
            // `InhChild` (oop/inhsub/) inheriting `InhParent.viaCreate()`
            // (oop/inh/) wrongly searched oop/inhsub/ for the bare sibling.
            let defining_source: Option<String> =
                if let CfmlValue::Function(ref f) = prop {
                    if let cfml_common::dynamic::CfmlClosureBody::Expression(ref body) = f.body {
                        if let CfmlValue::Int(idx) = body.as_ref() {
                            self.resolve_fn(*idx).and_then(|bf| bf.source_file.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
            let swap_source: Option<String> = defining_source.or_else(|| {
                if let CfmlValue::Struct(ref s) = object {
                    if let Some(CfmlValue::String(src)) = s.get("__source_file") {
                        Some(src.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            let saved_source_file_method: Option<Option<String>> = if receiver_is_cfc {
                if let Some(src) = swap_source {
                    let prev = self.source_file.clone();
                    self.source_file = Some(src);
                    Some(prev)
                } else {
                    None
                }
            } else {
                None
            };
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
            // Record the name this method was invoked under so the callee's
            // getFunctionCalledName() reports the alias (WireBox delegation
            // injects one UDF under many method names and dispatches by it).
            self.pending_called_name = Some(method.to_string());
            let result = self.call_function(&func_ref, args, &method_locals);
            // Restore the caller's source_file before propagating any error or
            // result, so relative resolution outside the method is unaffected.
            if let Some(prev) = saved_source_file_method {
                self.source_file = prev;
            }
            let result = result?;
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&prop_name))
                        .map(|(_, v)| v.clone());
                    if let Some(v) = val {
                        return Ok(v);
                    }
                }
                if method_lower.starts_with("set") && method_lower.len() > 3 {
                    let prop_name = &method[3..];
                    if let Some(value) = extra_args.first() {
                        let modified = object.clone();
                        if let Some(ms) = modified.as_cfml_struct() {
                            let actual_key = ms
                                .keys().into_iter()
                                .find(|k| k.eq_ignore_ascii_case(&prop_name))
                                
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
            if let Some(CfmlValue::NativeObject(parent_obj)) = s.get("__super") {
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
                // Key missingMethodArguments by the call-site argument NAME when
                // the call used named args (obj.probe(label="x") -> key "label"),
                // falling back to 1-based numeric keys for positional args. Lucee/
                // ACF/BoxLang all preserve names here; Wheels' dynamic query scopes
                // read them by name (arguments.status) via onMissingMethod.
                let names = arg_names.unwrap_or(&[]);
                let mut missing_args: ValueMap = ValueMap::default();
                let mut positional = 0usize;
                for (i, a) in args_array.into_iter().enumerate() {
                    let name = names.get(i).map(String::as_str).unwrap_or("");
                    if name.eq_ignore_ascii_case("argumentcollection") {
                        // Spread an argumentCollection struct into named entries,
                        // mirroring reorder_named_args_with_extras.
                        if let CfmlValue::Struct(ref s) = a {
                            for (k, v) in s.iter() {
                                missing_args.insert(k.clone(), v.clone());
                            }
                            continue;
                        }
                    }
                    if name.is_empty() {
                        positional += 1;
                        missing_args.insert(positional.to_string(), a);
                    } else {
                        missing_args.insert(name.to_string(), a);
                    }
                }
                let mut method_locals = ValueMap::default();
                if let CfmlValue::Struct(ref s2) = object {
                    if let Some(vars) = s2.get("__variables") {
                        method_locals.insert("__variables".to_string(), vars.clone());
                    }
                }
                method_locals.insert("this".to_string(), object.clone());
                return self.call_function(
                    &handler,
                    vec![
                        CfmlValue::string(method.to_string()),
                        CfmlValue::strukt(missing_args),
                    ],
                    &method_locals,
                );
            }
        }

        // A method call on a component that resolved to nothing — no own/inherited
        // method, no implicit accessor, no native parent, no onMissingMethod — is
        // an error in CFML, not a silent Null. Matches Lucee, which throws
        // "Component [x] has no function with name [y]". (Non-component receivers
        // keep the lenient Null return; tightening those is a separate concern.)
        if let CfmlValue::Struct(ref s) = object {
            if s.contains_key("__variables") || s.contains_key("__name") {
                let comp_name = s
                    .get("__name")
                    .map(|v| v.as_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "component".to_string());
                return Err(CfmlError::new(
                    format!(
                        "Component [{}] has no function with name [{}]",
                        comp_name, method
                    ),
                    CfmlErrorType::Expression,
                ));
            }
        }

        Ok(CfmlValue::Null)
    }

    /// Check if a variable name (possibly dotted like "request.data.name") is defined
    /// by walking the scope chain: locals → request → application → server → globals
    fn is_variable_defined(&self, var_name: &str, locals: &ValueMap) -> bool {
        let parts: Vec<&str> = var_name.split('.').collect();
        if parts.is_empty() {
            return false;
        }

        let root = parts[0].to_lowercase();

        // Try to resolve the root variable from scope chain
        let root_val = if root == "local" || root == "variables" {
            Some(CfmlValue::strukt(locals.clone()))
        } else if root == "arguments" {
            // The arguments scope lives under the reserved ARGUMENTS_SCOPE_KEY,
            // not under the literal "arguments" key — without this case a
            // nested probe like `isDefined("arguments.md.properties")` would
            // fall through to the regular-var branch, fail to find an
            // "arguments" local, and wrongly return false (which made ColdBox's
            // `param arguments.metadata.properties = []` clobber a populated
            // array — the WireBox property-injection regression).
            locals.get(ARGUMENTS_SCOPE_KEY).cloned()
        } else if root == "request" {
            Some(CfmlValue::strukt(self.request_scope.snapshot()))
        } else if root == "application" {
            if let Some(ref app_scope) = self.application_scope {
                // Live handle clone (Lucee scope-reference semantics).
                Some(CfmlValue::Struct(app_scope.clone()))
            } else {
                None
            }
        } else if root == "session" {
            Some(self.get_session_scope())
        } else if root == "cookie" {
            self.globals
                .get("cookie")
                .cloned()
                .or(Some(CfmlValue::strukt(ValueMap::default())))
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&root))
                        .map(|(_, v)| v.clone())
                })
                // Check request scope
                .or_else(|| self.request_scope.get_ci(&root))
                // Check globals
                .or_else(|| self.globals.get(parts[0]).cloned())
                .or_else(|| {
                    self.globals
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&root))
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
                        .find(|(k, _)| k.eq_ignore_ascii_case(&seg_lower))
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

    fn caller_writeback_from_captured(
        captured: &ValueMap,
        caller_snapshot: &ValueMap,
    ) -> Option<ValueMap> {
        let Some(CfmlValue::Struct(modified_caller)) = captured.get("caller") else {
            return None;
        };

        let mut writeback = ValueMap::default();
        for (key, value) in modified_caller.iter() {
            if let Some(original) = caller_snapshot.get(&key) {
                if !Self::values_equal_shallow(&value, original) {
                    writeback.insert(key.clone(), value.clone());
                }
            } else {
                writeback.insert(key.clone(), value.clone());
            }
        }

        if writeback.is_empty() {
            None
        } else {
            Some(writeback)
        }
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
                if a.ptr_eq(b) {
                    return true;
                }
                let (a, b) = (a.snapshot(), b.snapshot());
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| Self::values_equal_shallow_depth(x, y, depth + 1))
            }
            (CfmlValue::Struct(a), CfmlValue::Struct(b)) => {
                if a.ptr_eq(b) {
                    return true;
                }
                let (a, b) = (a.snapshot(), b.snapshot());
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
            // Queries are reference-typed: identity is equality (two handles onto
            // the same backing). Used only by the writeback diff, which no longer
            // tracks queries anyway.
            (CfmlValue::Query(a), CfmlValue::Query(b)) => a.ptr_eq(b),
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
        locals: &ValueMap,
    ) {
        if func.params.is_empty() {
            self.arg_ref_writeback = None;
            return;
        }
        let mut writeback = Vec::new();
        for (i, param_name) in func.params.iter().enumerate() {
            if let Some(val) = locals.get(param_name.as_str()) {
                match val {
                    // Arrays, structs AND queries are reference-typed now:
                    // in-place mutations through the parameter already propagate
                    // to the caller via the shared handle, so writing the
                    // parameter's value back is unnecessary — and WRONG when the
                    // parameter was *reassigned* (`a = [9]` / `q = queryNew()`),
                    // which CFML scopes to the local only. Only the still-value-
                    // typed Component needs the simulated pass-by-reference.
                    CfmlValue::Component(_) => {
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
    fn write_back_to_captured_scope(func_ref: &CfmlValue, writeback: &ValueMap) {
        if let CfmlValue::Function(ref f) = func_ref {
            if let Some(ref shared_env) = f.captured_scope {
                let mut env = shared_env.write().unwrap();
                for (k, v) in writeback {
                    // Never store Function values in the shared env — that would
                    // reintroduce the env -> Function -> captured_scope -> env
                    // cycle that leaks the env (see DefineFunction). Non-Function
                    // mutations propagate as normal.
                    if matches!(v, CfmlValue::Function(_)) {
                        continue;
                    }
                    env.insert(k.clone(), v.clone());
                }
            }
        }
    }

    /// Compute final closure write-back after a higher-order function loop.
    /// Compares modified locals against original parent_locals and sets closure_parent_writeback.
    fn set_ho_final_writeback(
        &mut self,
        modified: &ValueMap,
        original: &ValueMap,
    ) {
        let mut final_wb = ValueMap::default();
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
                .insert("cfthread".to_string(), CfmlValue::strukt(ValueMap::default()));
        }
        self.globals.get_mut("cfthread").unwrap()
    }

    /// If `attrs` carries an `attributeCollection` key whose value is a struct,
    /// merge those entries into a new attribute struct with explicit attrs
    /// winning. The source `attributeCollection` struct is not mutated.
    /// Returns the input unchanged when there is no collection to expand.
    fn merge_attribute_collection(attrs: CfmlValue) -> CfmlValue {
        let s = match &attrs {
            CfmlValue::Struct(s) => s.clone(),
            _ => return attrs,
        };
        let ac = match s.get_ci("attributeCollection") {
            Some(CfmlValue::Struct(ac)) => ac,
            _ => return attrs,
        };
        let mut merged: ValueMap = ac.snapshot();
        for (k, v) in s.snapshot() {
            if k.eq_ignore_ascii_case("attributeCollection") {
                continue;
            }
            if let Some(existing_key) = merged
                .keys()
                .find(|ek| ek.eq_ignore_ascii_case(&k))
                .cloned()
            {
                merged.shift_remove(&existing_key);
            }
            merged.insert(k, v);
        }
        CfmlValue::strukt(merged)
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
        } else if path_spec.starts_with('/') {
            // Leading-slash template: webroot-relative in CFML — resolve
            // through this.mappings (Application.cfc), then webroot, then the
            // entry template dir. Must NOT be treated as OS-absolute or as a
            // source-relative path.
            if let Some(resolved) = self.resolve_leading_slash_include(path_spec) {
                Ok(resolved)
            } else {
                Err(CfmlError::runtime(format!(
                    "Custom tag template '{}' not found",
                    path_spec
                )))
            }
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
        tag_locals: &ValueMap,
    ) -> Result<(), CfmlError> {
        let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
        let sub_program = compile_file_cached(template_path, cache, self.vfs.as_ref())?;

        let old_program = self.push_program_swap(sub_program);
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
        self.pop_program_swap(old_program);
        self.source_file = old_source;

        result.map(|_| ())
    }

    /// Walk a value graph (read-only) collecting the `global_id` of every
    /// reachable `Function` into `ids`. Mirrors the application-scope object
    /// graph (structs, arrays, query columns, components, captured closure
    /// scopes) with the same Arc-pointer cycle guard as the old reachability
    /// walk. Stored bodies already carry stable global_ids, so this only needs
    /// to *collect* — there is nothing to rewrite.
    fn collect_app_fn_ids(
        value: &CfmlValue,
        ids: &mut HashSet<i64>,
        visited: &mut HashSet<(u8, usize)>,
    ) {
        match value {
            CfmlValue::Function(function) => {
                Self::collect_app_fn_ids_from_fn(function, ids, visited);
            }
            CfmlValue::Struct(values) => {
                let ptr = values.backing_ptr();
                if !visited.insert((1, ptr)) {
                    return;
                }
                for (_, value) in values.iter() {
                    Self::collect_app_fn_ids(&value, ids, visited);
                }
            }
            CfmlValue::Array(values) => {
                let ptr = values.backing_ptr();
                if !visited.insert((2, ptr)) {
                    return;
                }
                for value in values.iter() {
                    Self::collect_app_fn_ids(&value, ids, visited);
                }
            }
            CfmlValue::QueryColumn(values) => {
                let ptr = Arc::as_ptr(values) as usize;
                if !visited.insert((4, ptr)) {
                    return;
                }
                for value in values.iter() {
                    Self::collect_app_fn_ids(value, ids, visited);
                }
            }
            CfmlValue::Component(component) => {
                for value in component.properties.values() {
                    Self::collect_app_fn_ids(value, ids, visited);
                }
                for function in component.methods.values() {
                    Self::collect_app_fn_ids_from_fn(function, ids, visited);
                }
            }
            _ => {}
        }
    }

    fn collect_app_fn_ids_from_fn(
        function: &cfml_common::dynamic::CfmlFunction,
        ids: &mut HashSet<i64>,
        visited: &mut HashSet<(u8, usize)>,
    ) {
        if let cfml_common::dynamic::CfmlClosureBody::Expression(ref body) = function.body {
            if let CfmlValue::Int(gid) = body.as_ref() {
                ids.insert(*gid);
            }
        }
        if let Some(shared_scope) = &function.captured_scope {
            let ptr = Arc::as_ptr(shared_scope) as usize;
            if !visited.insert((3, ptr)) {
                return;
            }
            if let Ok(scope) = shared_scope.read() {
                for value in scope.values() {
                    Self::collect_app_fn_ids(value, ids, visited);
                }
            }
        }
    }

    /// Refresh the per-application function table to the `Arc`s reachable from
    /// application scope, so functions stored there (CFC instances, factories,
    /// closures) stay resolvable on later requests — even ones that don't reload
    /// their source file. Stored bodies already carry stable global_ids, so this
    /// is purely a reachability collect: no body rewriting, no per-request remap.
    /// Reachability is recomputed each (dirty) request, so abandoned functions
    /// drop out — bounded growth, no stale retention.
    ///
    /// The carried set is closed under `DefineFunction`: a stored function's
    /// bytecode may define nested closures/UDFs at call time (e.g. a CFC method
    /// whose body does `var f = function(){...}`). Those nested functions are
    /// referenced only by a `DefineFunction(global_id)` op, never as a stored
    /// value, so the value walk alone misses them — and on a later request that
    /// doesn't reload their source file they would be unregistered (the bug a
    /// warm `application.svc.method()` call would hit). We therefore also scan
    /// each carried function's instructions for `DefineFunction` targets and
    /// carry those transitively. Their Arcs are registered this request (their
    /// program was loaded when the function was first compiled/instantiated).
    fn rehome_application_functions(&mut self) {
        let Some(app_scope) = self.application_scope.clone() else {
            return;
        };
        let mut ids: HashSet<i64> = HashSet::new();
        let mut visited = HashSet::new();
        app_scope.with_read(|scope| {
            for value in scope.values() {
                Self::collect_app_fn_ids(value, &mut ids, &mut visited);
            }
        });
        // Transitive `DefineFunction` closure (see doc comment).
        let mut worklist: Vec<i64> = ids.iter().copied().collect();
        while let Some(gid) = worklist.pop() {
            if let Some(arc) = self.resolve_fn(gid) {
                for op in &arc.instructions {
                    if let BytecodeOp::DefineFunction(child) = op {
                        let child = *child as i64;
                        if ids.insert(child) {
                            worklist.push(child);
                        }
                    }
                }
            }
        }
        // Carry the Arc for every reachable, currently-registered function.
        let mut table = Vec::with_capacity(ids.len());
        for gid in ids {
            if let Some(arc) = self.resolve_fn(gid) {
                table.push(arc);
            }
        }
        self.app_function_table = table;
    }

    /// Get default datasource from application scope or request scope
    fn get_default_datasource(&self, parent_locals: &ValueMap) -> String {
        // Per-application default datasource (this.datasource / cfconfig default)
        // takes precedence — already resolved to a URL.
        if let Some(ref url) = self.app_default_datasource {
            if !url.is_empty() {
                return url.clone();
            }
        }
        // Check application scope for datasource config
        if let Some(ref app_scope) = self.application_scope {
            if let Some(ds) = app_scope
                .get("datasource")
                .or_else(|| app_scope.get("defaultdatasource"))
            {
                let s = ds.as_string();
                if !s.is_empty() {
                    return s;
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
            let app = self.current_application_name.clone().unwrap_or_default();
            state.sessions.set(
                &app,
                &sid,
                SessionData {
                    variables: ValueMap::default(),
                    created_secs: now,
                    last_accessed_secs: now,
                    auth_user: None,
                    auth_roles: Vec::new(),
                    timeout_secs: self.session_timeout_secs,
                    app_name: app.clone(),
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

    /// Load the live session scope from the session store into `session_scope`
    /// (once per request). After this, reads return the live handle and writes
    /// mutate it in place; the result is synced back to the store at request end
    /// via `sync_session_scope_to_store`.
    fn attach_session_scope(&mut self) {
        if self.session_scope.is_some() {
            return;
        }
        // Establish a single stable, live (Arc-backed) handle for the session
        // scope so reads and writes round-trip for the lifetime of this request
        // — exactly like `application`, which is always a process-lifetime live
        // Arc. Seed it from the session store when we have a server AND a
        // session id (serve mode with an established session); otherwise start
        // empty (CLI, or a serve request before any session id exists). Without
        // a stable handle every `session` read returned a throwaway snapshot/
        // empty struct and writes through the "scope pointer" pattern
        // (`var p = session; p[k]=v; … session[k]`) vanished — WireBox
        // session-scoped singletons never cached.
        let vars = if let (Some(ref state), Some(ref sid)) =
            (&self.server_state, &self.session_id)
        {
            let app = self.current_application_name.as_deref().unwrap_or("");
            state
                .sessions
                .get(app, sid)
                .map(|s| s.variables)
                .unwrap_or_default()
        } else {
            ValueMap::default()
        };
        self.session_scope = Some(CfmlStruct::new(vars));
    }

    /// Persist the live session scope back to the session store. Called at the
    /// end of the request (after user code) so scope-pointer writes that bypass
    /// `set_session_*` (e.g. `var p = session; p[k]=v`) are committed.
    fn sync_session_scope_to_store(&mut self) -> Result<(), CfmlError> {
        let snap = match &self.session_scope {
            Some(ss) => ss.snapshot(),
            None => return Ok(()),
        };
        // Airtight data-only gate: catches values smuggled in via reference
        // mutation (`local.x = {}; session.cart = local.x; local.x.p = new C()`),
        // which no assignment-time check can see.
        validate_session_data_only(&snap)?;
        if let (Some(state), Some(sid)) = (self.server_state.clone(), self.session_id.clone()) {
            let now = now_epoch_secs();
            let app = self.current_application_name.clone().unwrap_or_default();
            let existing = state.sessions.get(&app, &sid);
            // Read-only access must NOT materialise a record (the Preside-CMS
            // defensive-read pattern). Reading `session` now attaches an empty
            // live scope; if nothing was written into it and no record exists,
            // there is nothing to persist — bail rather than mint an empty
            // session. A genuine (pointer or direct) write leaves a non-empty
            // snapshot, and an already-existing record is still refreshed.
            if existing.is_none() && snap.is_empty() {
                return Ok(());
            }
            // Create the record if missing so a write after sessionInvalidate
            // re-creates the session (matching the pre-cache set_session_* path).
            let mut session = existing.unwrap_or_else(|| SessionData {
                variables: ValueMap::default(),
                created_secs: now,
                last_accessed_secs: now,
                auth_user: None,
                auth_roles: Vec::new(),
                timeout_secs: self.session_timeout_secs,
                app_name: app.clone(),
            });
            session.variables = snap;
            session.last_accessed_secs = now;
            state.sessions.set(&app, &sid, session);
        }
        Ok(())
    }

    /// Get the session scope for the current request — a LIVE handle clone when
    /// the session scope is attached (so `var p = session; p.x = 1` writes
    /// through), falling back to a store snapshot otherwise.
    fn get_session_scope(&self) -> CfmlValue {
        if let Some(ref ss) = self.session_scope {
            return CfmlValue::Struct(ss.clone());
        }
        if let (Some(ref state), Some(ref sid)) = (&self.server_state, &self.session_id) {
            let app = self.current_application_name.as_deref().unwrap_or("");
            if let Some(session) = state.sessions.get(app, sid) {
                return CfmlValue::strukt(session.variables.clone());
            }
        }
        CfmlValue::strukt(ValueMap::default())
    }

    /// Set the session scope for the current request (mutates the live scope).
    fn set_session_scope(&mut self, vars: ValueMap) -> Result<(), CfmlError> {
        // Data-only gate at the write site (ergonomics): the common direct
        // case `session.x = new C()` fails here rather than at request end.
        // Validate BEFORE lazy-init so a rejected write mints no session.
        validate_session_data_only(&vars)?;
        // If lazy-init fires here, `onSessionStart` ran AFTER the user
        // code loaded the (then-empty) session scope. Their `vars`
        // snapshot doesn't contain any keys onSessionStart set, so we
        // merge: existing keys (from onSessionStart) are preserved,
        // and the user's writes overlay on top. Outside the lazy-init
        // case we fall through to the historical full-replace.
        let merge_with_existing = self.lazy_init_session_if_pending();
        self.attach_session_scope();
        if let Some(ref ss) = self.session_scope {
            if merge_with_existing {
                ss.with_write(|m| {
                    for (k, v) in vars {
                        m.insert(k, v);
                    }
                });
            } else {
                ss.with_write(|m| *m = vars);
            }
        }
        Ok(())
    }

    /// Update a single key in the session scope (mutates the live scope).
    fn set_session_variable(&mut self, key: &str, value: CfmlValue) -> Result<(), CfmlError> {
        // Data-only gate at the write site (ergonomics).
        validate_session_value(&format!("session.{}", key), &value).map_err(|p| {
            CfmlError::runtime(format!(
                "{}; the session scope only persists data values \
                 (no components, closures, functions, or native objects)",
                p
            ))
        })?;
        self.lazy_init_session_if_pending();
        self.attach_session_scope();
        if let Some(ref ss) = self.session_scope {
            ss.insert(key.to_string(), value);
        }
        Ok(())
    }

    /// Resolve a component template by name: tries locals, globals (exact + CI),
    /// then loads from a .cfc file on disk.
    fn resolve_component_template(
        &mut self,
        class_name: &str,
        locals: &ValueMap,
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
            .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
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
            if class_name.starts_with('/') {
                // A CFML leading-slash component path ("/oop/Widget") is
                // webroot/mapping-relative, NOT OS-absolute. Resolve it the same
                // way as a leading-slash include: configured mappings, then the
                // serve-mode webroot, then the entry template's parent directory.
                // Only fall back to treating it as a literal filesystem path when
                // none of those produce an existing file (preserving the case
                // where a genuinely OS-absolute .cfc is passed).
                let with_ext = if class_name.to_lowercase().ends_with(".cfc") {
                    class_name.to_string()
                } else {
                    format!("{}.cfc", class_name)
                };
                if self.vfs.exists(&with_ext) {
                    with_ext
                } else if let Some(resolved) = self.resolve_leading_slash_include(&with_ext) {
                    resolved
                } else {
                    with_ext
                }
            } else if as_path.is_absolute() || class_name.to_lowercase().ends_with(".cfc") {
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
            let old_program = self.push_program_swap(sub_program);
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
            let injected_scope: ValueMap = if let Some(pname) = parent_name {
                if let Some(parent_template) = self.resolve_component_template(&pname, locals) {
                    let resolved_parent = self.resolve_inheritance(parent_template, locals);
                    if let CfmlValue::Struct(ref ps) = resolved_parent {
                        if let Some(CfmlValue::Struct(parent_vars)) = ps.get("__variables") {
                            parent_vars.snapshot()
                        } else {
                            ValueMap::default()
                        }
                    } else {
                        ValueMap::default()
                    }
                } else {
                    ValueMap::default()
                }
            } else {
                ValueMap::default()
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
            // The CFC's functions were registered into `fn_registry` by global_id
            // when its sub-program was swapped in, and its methods were inserted
            // into `user_functions` by their DefineFunction ops. Stored method
            // values on the component struct carry the same stable global_ids. So
            // there is no merge-append, op remap, or index fixup to do — just
            // restore the caller's program.
            self.pop_program_swap(old_program);
            let short_name = class_name.split('.').last().unwrap_or(class_name);
            // Deep-copy the cached template into an independent instance. Structs
            // are reference-typed, so a plain handle clone would alias mutable
            // state across instances; the deep copy restores the per-instance
            // independence the old value-type copy-on-write gave implicitly.
            let mut result = self
                .globals
                .get(class_name)
                .cloned()
                .or_else(|| self.globals.get(short_name).cloned())
                .or_else(|| {
                    let lower = class_name.to_lowercase();
                    self.globals
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
                        .map(|(_, v)| v.clone())
                })
                .or_else(|| self.globals.get("Anonymous").cloned())
                .map(|v| v.deep_copy());
            // The component struct's method values carry stable global_ids (set
            // when the CFC body's DefineFunction ops ran), so no func_idx fixup
            // is needed here any more.
            // Strip captured_scope from all CFC methods on the component struct.
            // CFC methods are NOT closures — they were compiled in the CFC body
            // context where DefineFunction attaches a captured scope, but that scope
            // carries stale/unfixed data.  CFC method scope resolution should use
            // __variables (injected at call time), not captured scopes.
            //
            // Guard on `is_some()`: `Arc::make_mut` deep-copies the inner
            // CfmlFunction (name+params+body) whenever the Arc is shared — and
            // these method Arcs are always shared with the canonical
            // bytecode-cached program. The common case is captured_scope already
            // None, so the unconditional make_mut needlessly cloned every method
            // body on every component resolution. Skipping it when None keeps the
            // instance's methods as cheap shared Arcs.
            if let Some(s) = result.as_mut().and_then(|v| v.as_cfml_struct()) {
                s.with_write(|m| {
                    for (_, v) in m.iter_mut() {
                        if let CfmlValue::Function(f) = v {
                            if f.captured_scope.is_some() {
                                Arc::make_mut(f).captured_scope = None;
                            }
                        }
                    }
                });
            }
            // Store the CFC source path for parent resolution during inheritance
            if let Some(s) = result.as_mut().and_then(|v| v.as_cfml_struct()) {
                s.insert(
                    "__source_file".to_string(),
                    CfmlValue::string(cfc_path.clone()),
                );
                // Anonymous `component { ... }` declarations get __name = "Anonymous"
                // baked in by the parser. Override with the dotted path the caller
                // used (e.g. "oop.Greeter") so getMetadata(cfc).name matches Lucee/ACF.
                let needs_override = match s.get("__name") {
                    Some(CfmlValue::String(n)) => n.as_str() == "Anonymous",
                    _ => true,
                };
                if needs_override {
                    s.insert(
                        "__name".to_string(),
                        CfmlValue::string(class_name.to_string()),
                    );
                }
            }
            // Inject functions added by cfinclude inside the component body
            // These were registered in user_functions during execution but aren't
            // in the component struct (which was built at compile time)
            if let Some(s) = result.as_mut().and_then(|v| v.as_cfml_struct()) {
                let existing_keys: std::collections::HashSet<String> =
                    s.keys().into_iter().map(|k| k.to_lowercase()).collect();
                for (func_name, func_def) in &self.user_functions {
                    if !pre_exec_func_names.contains(func_name)
                        && !existing_keys.contains(&func_name.to_lowercase())
                    {
                        // Expose the function if it belongs to this program,
                        // referencing it by its stable global_id.
                        if self
                            .program
                            .functions
                            .iter()
                            .any(|f| f.name == *func_name)
                        {
                            let cf = CfmlValue::Function(Arc::new(cfml_common::dynamic::CfmlFunction {
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
                                    CfmlValue::Int(func_def.global_id as i64),
                                )),
                                return_type: None,
                                access: cfml_common::dynamic::CfmlAccess::Public,
                                captured_scope: None,
                            }));
                            s.insert(func_name.clone(), cf);
                        }
                    }
                }
            }
            // Store component body variables + all methods as __variables
            // In CFML, component methods live in the variables scope so
            // unqualified calls inside methods resolve via the normal scope chain.
            if let Some(s) = result.as_mut().and_then(|v| v.as_cfml_struct()) {
                let mut vars_scope: ValueMap = ValueMap::default();
                // Add component body variables (non-function values from the
                // pseudo-constructor). Function values carry stable global_ids
                // (no index fixup needed); strip their captured_scope because CFC
                // methods resolve via __variables, not closures.
                for (k, v) in &component_variables {
                    let k_lower = k.to_lowercase();
                    if k_lower == "this" || k_lower == "arguments" || k.starts_with("__") {
                        continue;
                    }
                    if let CfmlValue::Function(ref f) = v {
                        let mut clean = f.clone();
                        Arc::make_mut(&mut clean).captured_scope = None;
                        vars_scope.insert(k.clone(), CfmlValue::Function(clean));
                    } else {
                        vars_scope.insert(k.clone(), v.clone());
                    }
                }
                // Add all component methods (public + private) to variables scope
                // These override component_variables entries for public methods.
                // Strip captured_scope — CFC methods use __variables, not closures.
                //
                // EXCEPTION: if the pseudo-constructor explicitly reassigned a
                // method's name to a non-function value (e.g. a CFC with both a
                // `property name="foo"` accessor and a same-named method `foo()`,
                // where the body did `variables.foo = {...}`), that write must win.
                // Lucee/ACF hoist methods into the variables scope FIRST, then run
                // the pseudo-constructor, so a `variables.foo = value` assignment
                // shadows the same-named method. Without this guard the method
                // clobbers the assigned value and `getFoo()` reads back empty.
                for (k, v) in s.iter() {
                    if k.starts_with("__") {
                        continue;
                    }
                    if let CfmlValue::Function(ref f) = v {
                        let shadowed_by_ctor = component_variables.iter().any(|(ck, cv)| {
                            ck.eq_ignore_ascii_case(&k) && !matches!(cv, CfmlValue::Function(_))
                        });
                        if shadowed_by_ctor {
                            continue;
                        }
                        let mut clean = f.clone();
                        Arc::make_mut(&mut clean).captured_scope = None;
                        vars_scope.insert(k.clone(), CfmlValue::Function(clean));
                    }
                }
                // Merge compiler-generated __variables (property defaults) into
                // the runtime vars_scope. Runtime values take priority, but
                // defaults for properties not set during pseudo-constructor are preserved.
                if let Some(CfmlValue::Struct(ref compiled_vars)) = s.get("__variables") {
                    for (k, v) in compiled_vars.iter() {
                        if !vars_scope.contains_key(&k) {
                            vars_scope.insert(k, v);
                        }
                    }
                }
                if !vars_scope.is_empty() {
                    s.insert("__variables".to_string(), CfmlValue::strukt(vars_scope));
                }
            }
            // Break the per-instantiation closure-env retention: clear captured
            // scopes on every member function of the assembled instance. CFC
            // members use __variables, not closures, so this is semantically inert
            // and lets the closure env (heavily aliased across method copies) drop.
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
        locals: &ValueMap,
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
                    .find(|(k, _)| k.eq_ignore_ascii_case(&iface_name))
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
        locals: &ValueMap,
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
                    .find(|(k, _)| k.eq_ignore_ascii_case(&iface_name))
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
        component: &ValueMap,
        locals: &ValueMap,
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
                    k.eq_ignore_ascii_case(&method_name)
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

    /// Validate a freshly-instantiated component against its declared interfaces
    /// and stamp the transitive interface set onto `__implements_chain`, so that
    /// `isInstanceOf` recognises inherited interfaces (an interface's `extends`
    /// ancestors). Shared by `new X()` and `createObject("component", …)` so both
    /// instantiation forms honour interface inheritance identically.
    fn attach_implements_chain(
        &mut self,
        instance: CfmlValue,
        locals: &ValueMap,
    ) -> Result<CfmlValue, CfmlError> {
        if let CfmlValue::Struct(ref s) = instance {
            let all_ifaces = self.validate_interface_implementation(&s.snapshot(), locals)?;
            if !all_ifaces.is_empty() {
                let mut m = s.snapshot();
                let chain: Vec<CfmlValue> =
                    all_ifaces.into_iter().map(CfmlValue::string).collect();
                m.insert("__implements_chain".to_string(), CfmlValue::array(chain));
                return Ok(CfmlValue::strukt(m));
            }
        }
        Ok(instance)
    }

    fn resolve_inheritance(
        &mut self,
        template: CfmlValue,
        locals: &ValueMap,
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
        locals: &ValueMap,
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
            let child_map = match child {
                CfmlValue::Struct(s) => s,
                other => return other,
            };
            child_map.insert(
                "__rust_extends".to_string(),
                CfmlValue::string(rust_class.to_string()),
            );
            let mut chain = vec![CfmlValue::string(parent_name.to_string())];
            if let Some(CfmlValue::Array(existing)) = child_map.get("__extends_chain") {
                for item in existing.iter() {
                    chain.push(item.clone());
                }
            }
            child_map
                .insert("__extends_chain".to_string(), CfmlValue::array(chain));
            return CfmlValue::Struct(child_map);
        }

        // Temporarily set source_file to the child CFC's path so parent
        // resolution finds siblings in the same directory
        let old_source_file = if let CfmlValue::Struct(ref cs) = child {
            if let Some(CfmlValue::String(src)) = cs.get("__source_file") {
                let prev = self.source_file.clone();
                self.source_file = Some(src.to_string());
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
        // Snapshot the parent into an owned map so the merge never mutates the
        // shared parent template (preserves the old `Arc::make_mut` copy-on-write
        // behaviour now that structs are reference-typed).
        let mut parent_map: ValueMap = match parent {
            CfmlValue::Struct(s) => s.snapshot(),
            _ => return CfmlValue::Struct(child_map),
        };

        // Collect parent methods for __super
        let mut super_methods = ValueMap::default();
        for (k, v) in parent_map.iter() {
            if matches!(v, CfmlValue::Function(_)) && !k.starts_with("__") {
                super_methods.insert(k.clone(), v.clone());
            }
        }

        // Merge __variables from parent and child (child overrides parent)
        let parent_vars: ValueMap = parent_map
            .get("__variables")
            .and_then(|v| v.as_struct())
            .unwrap_or_default();
        let child_vars: ValueMap = child_map
            .get("__variables")
            .and_then(|v| v.as_struct())
            .unwrap_or_default();
        if !parent_vars.is_empty() || !child_vars.is_empty() {
            let mut merged_vars = parent_vars;
            for (k, v) in child_vars {
                merged_vars.insert(k, v);
            }
            parent_map.insert("__variables".to_string(), CfmlValue::strukt(merged_vars));
        }

        // Component-level metadata (displayName, hint, and any other
        // <cfcomponent>-declared attributes) is per-class and must NOT be
        // inherited onto a child's leaf metadata. Lucee/ACF/BoxLang expose a
        // parent's attributes only via the metadata `extends` chain, never
        // duplicated on the child. The parent snapshot carries the parent's
        // __metadata, so replace it with the child's own (or drop it entirely
        // when the child declared no component attributes).
        match child_map.get("__metadata") {
            Some(child_md) => {
                parent_map.insert("__metadata".to_string(), child_md.clone());
            }
            None => {
                parent_map.shift_remove("__metadata");
            }
        }

        // Layer child on top of parent (child overrides parent)
        for (k, v) in child_map.iter() {
            if k == "__extends" || k == "__variables" || k == "__metadata" {
                continue; // Already merged above; don't overwrite
            }
            // CFML is case-insensitive: a child member overrides a parent member
            // even when their casing differs (e.g. parent OnApplicationStart vs
            // child onApplicationStart). A plain insert would leave BOTH casings
            // in the map, so drop any differently-cased parent entry first so the
            // child's definition is the sole winner.
            if let Some(stale) = parent_map
                .keys()
                .find(|ek| ek.as_str() != k.as_str() && ek.eq_ignore_ascii_case(&k))
                .cloned()
            {
                parent_map.shift_remove(&stale);
            }
            parent_map.insert(k.clone(), v.clone());
            // Also update __variables when child overrides a method, so
            // unqualified calls within CFC methods resolve to the override
            if matches!(v, CfmlValue::Function(_)) && !k.starts_with("__") {
                if let Some(vars) = parent_map.get_mut("__variables").and_then(|v| v.as_cfml_struct()) {
                    // remove_ci drops any differently-cased parent entry too
                    vars.remove_ci(&k);
                    vars.insert(k.clone(), v.clone());
                }
            }
        }

        // Add __super struct with marker for dispatch detection
        if !super_methods.is_empty() {
            super_methods.insert("__is_super".to_string(), CfmlValue::Bool(true));
            parent_map.insert("__super".to_string(), CfmlValue::strukt(super_methods));
        }

        // Build __extends_chain for isInstanceOf
        let mut chain = Vec::new();
        chain.push(CfmlValue::string(parent_name.to_string()));
        if let Some(CfmlValue::Array(existing)) = parent_map.get("__extends_chain") {
            for item in existing.iter() {
                chain.push(item.clone());
            }
        }
        parent_map.insert("__extends_chain".to_string(), CfmlValue::array(chain));

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
                .map(|s| CfmlValue::string(s))
                .collect();
            parent_map.insert("__implements_chain".to_string(), CfmlValue::array(chain));
        }

        CfmlValue::strukt(parent_map)
    }

    /// If `instance` was marked with `__rust_extends` by resolve_inheritance_chain,
    /// look up the native constructor, default-construct it, and stash the
    /// resulting NativeObject under `__super`. Errors if the named class is
    /// not registered. Idempotent: returns early if `__super` is already set.
    fn attach_native_parent(&mut self, instance: CfmlValue) -> CfmlResult {
        let s = match instance {
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
        s.insert("__super".to_string(), parent);
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
                        .map(CfmlValue::string)
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
                let filter = args.get(3).map(|v| v.as_string()).unwrap_or_default();
                Some(self.sandbox_directory_list(&path, recurse, &list_info, &filter))
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
                            CfmlValue::Int(i) => Some(i as usize),
                            _ => None,
                        })
                        .unwrap_or(0);
                    Some(
                        self.vfs
                            .read_to_string(&path)
                            .map(|content| {
                                let lines: Vec<&str> = content.lines().collect();
                                if line_num < lines.len() {
                                    CfmlValue::string(lines[line_num].to_string())
                                } else {
                                    CfmlValue::string(String::new())
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
                            CfmlValue::Int(i) => Some(i as usize),
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
                    let mut handle = ValueMap::default();
                    handle.insert("path".to_string(), CfmlValue::string(path));
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
            "gettempdirectory" => Some(Ok(CfmlValue::string(
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
                        "list" => Some(self.cfdirectory_list_from_opts(opts)),
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
    fn sandbox_directory_list(&self, path: &str, recurse: bool, list_info: &str, filter: &str) -> CfmlResult {
        let mut entries = Vec::new();
        let mut visited = std::collections::HashSet::new();
        if let Ok(canon) = self.vfs.canonicalize(path) {
            visited.insert(canon);
        }
        self.sandbox_collect_entries(path, recurse, &mut entries, &mut visited)?;

        // The name filter applies to both files and directories (Lucee/ACF);
        // recursion already descended into every subdir above regardless.
        if !filter.is_empty() {
            entries.retain(|(name, _, _)| Self::matches_directory_filter(name, filter));
        }

        if list_info == "name" {
            Ok(CfmlValue::array(
                entries
                    .into_iter()
                    .map(|(name, _, _)| CfmlValue::string(name))
                    .collect(),
            ))
        } else if list_info == "query" {
            Ok(Self::build_directory_query(&entries, path))
        } else {
            // "path" mode: return array of full paths
            Ok(CfmlValue::array(
                entries
                    .into_iter()
                    .map(|(_, full, _)| CfmlValue::string(full))
                    .collect(),
            ))
        }
    }

    fn cfdirectory_list_from_opts(&self, opts: &CfmlStruct) -> CfmlResult {
        let dir = opts
            .get_ci("directory")
            .map(|v| v.as_string())
            .unwrap_or_default();
        let filter = opts
            .get_ci("filter")
            .map(|v| v.as_string())
            .unwrap_or_else(|| "*".to_string());
        let recurse = opts
            .get_ci("recurse")
            .map(|v| v.is_true())
            .unwrap_or(false);
        match self.resolve_directory_path_with_mappings(&dir) {
            Some(resolved) => self.sandbox_cfdirectory_list(&resolved, recurse, &filter),
            None => Err(CfmlError::runtime(format!(
                "cfdirectory: directory not found: {}",
                dir
            ))),
        }
    }

    fn sandbox_cfdirectory_list(&self, path: &str, recurse: bool, filter: &str) -> CfmlResult {
        let mut entries = Vec::new();
        let mut visited = std::collections::HashSet::new();
        if let Ok(canon) = self.vfs.canonicalize(path) {
            visited.insert(canon);
        }
        self.sandbox_collect_entries(path, recurse, &mut entries, &mut visited)?;
        entries.retain(|(name, _, _)| Self::matches_directory_filter(name, filter));
        Ok(Self::build_directory_query(&entries, path))
    }

    /// Build a real `Query` value from collected directory entries
    /// (`(name, full_path, is_dir)`), matching Lucee's `cfdirectory action="list"`
    /// column set: `name, size, type, dateLastModified, attributes, mode,
    /// directory`. Returning an actual `CfmlValue::Query` (rather than a struct
    /// tagged `__type="query"`) means `isQuery()`, `queryColumnExists()`, and
    /// row access all behave like the reference engine.
    fn build_directory_query(entries: &[(String, String, bool)], fallback_dir: &str) -> CfmlValue {
        let query = cfml_common::dynamic::CfmlQuery::new(vec![
            "name".to_string(),
            "size".to_string(),
            "type".to_string(),
            "dateLastModified".to_string(),
            "attributes".to_string(),
            "mode".to_string(),
            "directory".to_string(),
        ]);
        for (name, full_path, is_dir) in entries {
            let meta = std::fs::metadata(full_path).ok();
            let size = if *is_dir {
                0
            } else {
                meta.as_ref().map(|m| m.len() as i64).unwrap_or(0)
            };
            let date = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                })
                .unwrap_or_default();
            let directory = std::path::Path::new(full_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| fallback_dir.to_string());
            let mut row = ValueMap::default();
            row.insert("name".to_string(), CfmlValue::string(name.clone()));
            row.insert("size".to_string(), CfmlValue::Int(size));
            row.insert(
                "type".to_string(),
                CfmlValue::string(if *is_dir { "Dir" } else { "File" }.to_string()),
            );
            row.insert("dateLastModified".to_string(), CfmlValue::string(date));
            row.insert("attributes".to_string(), CfmlValue::string(String::new()));
            row.insert("mode".to_string(), CfmlValue::string(String::new()));
            row.insert("directory".to_string(), CfmlValue::string(directory));
            query.add_row(row);
        }
        CfmlValue::Query(query)
    }

    fn resolve_directory_path_with_mappings(&self, path: &str) -> Option<String> {
        if self.vfs.exists(path) {
            return Some(path.to_string());
        }
        if path.starts_with('/') {
            return self.resolve_include_with_mappings(path);
        }
        if let Some(ref base) = self.base_template_path {
            let base_dir = std::path::Path::new(base)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let candidate = base_dir.join(path).to_string_lossy().to_string();
            if self.vfs.exists(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    fn matches_directory_filter(name: &str, pattern: &str) -> bool {
        if pattern == "*" || pattern.is_empty() {
            return true;
        }
        if let Some(ext) = pattern.strip_prefix("*.") {
            return name.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase()));
        }
        name.eq_ignore_ascii_case(pattern)
    }

    fn sandbox_collect_entries(
        &self,
        path: &str,
        recurse: bool,
        out: &mut Vec<(String, String, bool)>,
        visited: &mut std::collections::HashSet<String>,
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
                // Cycle protection: following directory symlinks can form loops.
                // `visited` holds the canonical paths of the current ancestor
                // chain only — so a symlink pointing back at an ancestor is
                // skipped, but a symlink to a sibling/already-listed directory is
                // still followed (Lucee lists such targets again).
                let canon = self
                    .vfs
                    .canonicalize(&full_path)
                    .unwrap_or_else(|_| full_path.clone());
                if visited.insert(canon.clone()) {
                    self.sandbox_collect_entries(&full_path, true, out, visited)?;
                    visited.remove(&canon);
                }
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
        let mut info = ValueMap::default();
        info.insert("name".to_string(), CfmlValue::string(name));
        info.insert("path".to_string(), CfmlValue::string(path.to_string()));
        info.insert("size".to_string(), CfmlValue::Int(size));
        info.insert(
            "type".to_string(),
            CfmlValue::string(if is_file { "file" } else { "dir" }.to_string()),
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
                    return Ok(CfmlValue::string(val.trim().to_string()));
                }
            }
        }
        Ok(CfmlValue::string(String::new()))
    }

    /// Sandbox getProfileSections: read INI sections from VFS.
    fn sandbox_get_profile_sections(&self, path: &str) -> CfmlResult {
        let content = self
            .vfs
            .read_to_string(path)
            .map_err(|e| CfmlError::runtime(format!("getProfileSections: {}", e)))?;
        let mut result = ValueMap::default();
        let mut current_section = String::new();
        let mut current_keys: Vec<String> = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                if !current_section.is_empty() {
                    result.insert(
                        current_section.clone(),
                        CfmlValue::string(current_keys.join(",")),
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
            result.insert(current_section, CfmlValue::string(current_keys.join(",")));
        }
        Ok(CfmlValue::strukt(result))
    }

    /// Walk up the directory tree from source_file to find Application.cfc.
    /// In production mode, the resolved path (or absence) is memoized by
    /// start directory on `ServerState.app_cfc_path_cache`.
    /// Directory to begin the Application.cfc walk-up from, given the entry
    /// `source` path and the process `cwd`. A bare filename ("run_tests.cfm")
    /// has an EMPTY parent (not None) — using it directly made the walk start
    /// from "" and immediately fail, so a sibling Application.cfc was never
    /// found. Treat empty/None parents as the current directory. Pure (no I/O)
    /// so it can be unit-tested.
    fn app_cfc_start_dir(source: Option<&str>, cwd: &std::path::Path) -> std::path::PathBuf {
        match source {
            Some(src) => match std::path::Path::new(src).parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                _ => cwd.to_path_buf(),
            },
            None => cwd.to_path_buf(),
        }
    }

    fn find_application_cfc(&self) -> Option<String> {
        let cwd = std::env::current_dir().unwrap_or_default();
        // Prefer the resolved `base_template_path` (absolute under the VFS root in
        // serve/embedded mode) over the raw `source_file` (which may be a bare,
        // VFS-relative filename) so the walk-up starts from a directory the VFS
        // can actually read.
        let discovery_path = self
            .base_template_path
            .as_deref()
            .or(self.source_file.as_deref());
        let start_dir = Self::app_cfc_start_dir(discovery_path, &cwd);

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

    /// Discover and apply an application-level `.cfconfig.json` sitting beside
    /// the given `Application.cfc`. Overlays it on the server baseline
    /// (`server_state.cfconfig`), stashes the result in `self.app_cfconfig`, and
    /// applies the per-VM runtime knobs via `apply_cfconfig`. The server section
    /// of the app file is ignored (port etc. are server-level). No-op when there
    /// is no server baseline (CLI single-shot) or no file is found.
    fn discover_app_cfconfig(&mut self, app_cfc_path: &str) {
        let dir = match std::path::Path::new(app_cfc_path).parent() {
            Some(d) => d.to_string_lossy().to_string(),
            None => return,
        };
        let candidate = format!("{}/{}", dir.trim_end_matches('/'), cfml_config::resolve::FILENAME);
        let candidate_key = std::path::PathBuf::from(&candidate);

        // The overlaid config is a pure function of a static file + the (constant)
        // server baseline, so in production hold it in memory keyed by path —
        // file read + JSON parse + overlay run once, not per request.
        let production = self
            .server_state
            .as_ref()
            .map(|ss| ss.production_mode)
            .unwrap_or(false);
        if production {
            // Resolve the lookup to an owned value so no cache read-guard / self
            // borrow is held across the mutable apply below.
            let hit = self.server_state.as_ref().and_then(|ss| {
                ss.app_cfconfig_cache.read().get(&candidate_key).cloned()
            });
            if let Some(cached) = hit {
                if let Some(merged) = cached {
                    self.apply_app_cfconfig(merged);
                }
                return;
            }
        }

        // Cache miss (or non-production): compute the overlaid config.
        let merged: Option<Arc<cfml_config::RustCfmlConfig>> = self
            .compute_app_cfconfig(&candidate);

        if production {
            if let Some(ref ss) = self.server_state {
                ss.app_cfconfig_cache
                    .write()
                    .insert(candidate_key, merged.clone());
            }
        }
        if let Some(merged) = merged {
            self.apply_app_cfconfig(merged);
        }
    }

    /// Read + parse + overlay a per-app `.cfconfig.json` (no caching, no apply).
    /// Returns the overlaid config, or `None` when the file is absent/invalid.
    fn compute_app_cfconfig(&self, candidate: &str) -> Option<Arc<cfml_config::RustCfmlConfig>> {
        if !self.vfs.is_file(candidate) {
            return None;
        }
        let bytes = match self.vfs.read(candidate) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("failed to read app cfconfig {}: {}", candidate, e);
                return None;
            }
        };
        let app_json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("invalid app cfconfig {}: {}", candidate, e);
                return None;
            }
        };
        let baseline = self
            .server_state
            .as_ref()
            .map(|ss| ss.cfconfig.clone())
            .unwrap_or_else(|| Arc::new(cfml_config::RustCfmlConfig::default()));
        match baseline.overlay_app_json(app_json) {
            Ok(merged) => Some(Arc::new(merged)),
            Err(e) => {
                log::warn!("failed to overlay app cfconfig {}: {}", candidate, e);
                None
            }
        }
    }

    /// Apply an already-computed per-app cfconfig to this request: overlay the
    /// per-VM runtime knobs, seed per-app datasources, and stash it as the source
    /// for the CFML-visible `server.cfconfig`. Runs every request (cheap); the
    /// expensive file parse is what `compute_app_cfconfig` does once.
    fn apply_app_cfconfig(&mut self, merged: Arc<cfml_config::RustCfmlConfig>) {
        self.apply_cfconfig(&merged);
        // Seed per-request datasources from the overlaid cfconfig so
        // `cfquery datasource="x"` resolves the app's view, not just the
        // process-global baseline. `this.datasources` (read later from
        // Application.cfc) overrides these.
        for (name, ds) in merged.datasources.iter() {
            if let Some(url) = ds.connection_url() {
                self.app_datasources.insert(name.to_lowercase(), url.clone());
                if ds.default {
                    self.app_default_datasource = Some(url);
                }
            }
        }
        self.app_cfconfig = Some(merged);
    }

    /// Convert a single `this.datasources` entry to a connection URL. Accepts a
    /// bare connection-string (used verbatim) or a struct
    /// (`{driver, host, port, database, username, password, connectionString}`)
    /// — the struct form is synthesised through the same `DatasourceCfg` URL
    /// builder the cfconfig datasources use, so both config paths agree.
    fn datasource_value_to_url(val: &CfmlValue) -> Option<String> {
        match val {
            CfmlValue::String(s) if !s.is_empty() => Some(s.to_string()),
            CfmlValue::Struct(s) => {
                let get = |key: &str| -> String {
                    s.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v.as_string())
                        .unwrap_or_default()
                };
                let mut ds = cfml_config::DatasourceCfg::default();
                ds.driver = get("driver");
                ds.host = get("host");
                ds.port = get("port");
                ds.database = get("database");
                ds.username = get("username");
                ds.password = get("password");
                ds.connection_string = {
                    let cs = get("connectionString");
                    if cs.is_empty() { get("connectionstring") } else { cs }
                };
                ds.connection_url()
            }
            _ => None,
        }
    }

    /// Read `this.datasources` (named) and `this.datasource` (default) from the
    /// loaded Application.cfc component and seed `app_datasources` /
    /// `app_default_datasource`. These take precedence over cfconfig datasources.
    /// This is what makes per-application datasources work (Lucee/BoxLang parity)
    /// — previously RustCFML ignored `this.datasources` entirely.
    fn seed_app_datasources_from_template(&mut self, template: &CfmlValue) {
        let CfmlValue::Struct(s) = template else { return };
        if let Some(CfmlValue::Struct(map)) = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasources"))
            .map(|(_, v)| v)
        {
            for (name, def) in map.iter() {
                if let Some(url) = Self::datasource_value_to_url(&def) {
                    self.app_datasources.insert(name.to_lowercase(), url);
                }
            }
        }
        // `this.datasource` (singular) names the application's default datasource.
        if let Some(name) = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
            .map(|(_, v)| v.as_string())
            .filter(|n| !n.is_empty())
        {
            // Resolve the named default through the per-app map; if it isn't a
            // known per-app name, store it as-is for the global registry to map.
            let url = self
                .app_datasources
                .get(&name.to_lowercase())
                .cloned()
                .unwrap_or(name);
            self.app_default_datasource = Some(url);
        }
    }

    /// Resolve a datasource name to a per-application connection URL, if this
    /// request defined one (via `this.datasources` or a per-app cfconfig). Used
    /// by the query/transaction paths before consulting the global registry.
    fn resolve_app_datasource(&self, name: &str) -> Option<String> {
        self.app_datasources.get(&name.to_lowercase()).cloned()
    }

    /// Rewrite a `queryExecute(sql, params, options)` arg list so a per-app
    /// datasource name resolves to its connection URL before the (global)
    /// builtin sees it. The builtin treats an unrecognised name as a literal
    /// connection string, so substituting the URL routes the query to the
    /// application's datasource. Only fires when this request has per-app
    /// datasources; otherwise the args pass through untouched and the global
    /// registry / dynamic-driver path resolves the name as before.
    ///
    /// Builds a fresh options struct rather than mutating in place, so the
    /// caller's CFML struct keeps its original `datasource` name.
    fn rewrite_query_datasource(&self, mut args: Vec<CfmlValue>) -> Vec<CfmlValue> {
        if self.app_datasources.is_empty() && self.app_default_datasource.is_none() {
            return args;
        }
        let Some(CfmlValue::Struct(opts)) = args.get(2) else {
            return args;
        };
        let mut map = opts.snapshot();
        let current = map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
            .map(|(_, v)| v.as_string());
        let new_url = match current {
            Some(ref name) => self.resolve_app_datasource(name),
            None => self.app_default_datasource.clone(),
        };
        let Some(url) = new_url else {
            return args;
        };
        let key = map
            .keys()
            .find(|k| k.eq_ignore_ascii_case("datasource"))
            .cloned()
            .unwrap_or_else(|| "datasource".to_string());
        map.insert(key, CfmlValue::string(url));
        args[2] = CfmlValue::strukt(map);
        args
    }

    /// Load and execute Application.cfc, returning the component struct
    ///
    /// Returns `Ok(Some(template))` on success, `Ok(None)` when no usable
    /// component could be located/compiled (caller falls through to the page),
    /// and `Err(e)` when the Application.cfc pseudo-constructor itself threw —
    /// a load failure must abort the request, not silently run the page.
    fn load_application_cfc(&mut self, path: &str) -> Result<Option<CfmlValue>, CfmlError> {
        let cache = self.server_state.as_ref().map(|s| &s.bytecode_cache);
        let sub_program = match compile_file_cached(path, cache, self.vfs.as_ref()) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };

        // Save current program, swap in sub-program
        let old_program = self.push_program_swap(sub_program);
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
        let empty_locals = ValueMap::default();
        let exec_result =
            self.execute_function_with_args(&cfc_body, Vec::new(), Some(&empty_locals));

        // Capture component body locals as the variables scope
        let component_variables = self.captured_locals.take().unwrap_or_default();

        // Merge sub-program functions into main program
        let sub_funcs = self.program.functions.clone();
        self.pop_program_swap(old_program);

        // If the pseudo-constructor threw, surface it now (after restoring the
        // program) so the request fails rather than falling through to the page.
        if let Err(e) = exec_result {
            return Err(e);
        }

        // Re-assert by-name registration for the Application.cfc's functions so
        // lifecycle methods (onApplicationStart, ...) dispatch even under a later
        // program swap. They are already in `fn_registry` by global_id from the
        // sub-program swap-in, so no program-index append is needed.
        for func in sub_funcs {
            if func.name != "__main__" {
                self.user_functions
                    .insert(func.name.clone(), Arc::clone(&func));
            }
        }

        // Find the component struct in globals
        let template = self
            .globals
            .iter()
            .find(|(k, v)| {
                let k_lower = k.to_lowercase();
                (k_lower == "application" || *k == "Anonymous")
                    && matches!(v, CfmlValue::Struct(_))
                    && if let CfmlValue::Struct(s) = v {
                        s.contains_key("__name")
                            || s.with_read(|m| m.values().any(|v| matches!(v, CfmlValue::Function(_))))
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
                                || s.with_read(|m| m.values().any(|val| matches!(val, CfmlValue::Function(_))))
                        } else {
                            false
                        }
                    })
                    .map(|(_, v)| v.clone())
            });
        let template = match template {
            Some(t) => t,
            None => return Ok(None),
        };

        // No func_idx fixup: the template's method values carry stable global_ids.

        // Store component body variables as __variables on the template
        // This makes variables.framework etc. accessible to component methods
        if !component_variables.is_empty() {
            // The component instance is visible in its own pseudo-constructor
            // body locals under its auto-generated name (e.g. "Anonymous").
            // Copying that self-alias into __variables (which we then store back
            // ON the component) forms an Arc cycle
            //   template -> __variables -> <self-alias> -> template
            // that refcounting never frees — leaking the entire Application.cfc
            // instance every request in serve mode (PR #149). Skip any body
            // local whose backing store IS this component's.
            let self_ptr = template.as_cfml_struct().map(|s| s.backing_ptr());
            let mut vars_scope: ValueMap = ValueMap::default();
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
                // Skip the component's own self-alias (would form a cycle).
                if let (Some(sp), Some(vs)) = (self_ptr, v.as_cfml_struct()) {
                    if vs.backing_ptr() == sp {
                        continue;
                    }
                }
                vars_scope.insert(k.clone(), v.clone());
            }
            if !vars_scope.is_empty() {
                if let Some(s) = template.as_cfml_struct() {
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
        let resolved = self.resolve_inheritance(template, &ValueMap::default());
        Ok(Some(resolved))
    }

    /// Extract application config from a component struct.
    /// Returns (app_name, config, mappings, session_management, session_timeout_secs,
    /// custom_tag_paths, local_mode_modern_default, session_storage, app_caches)
    fn extract_app_config(
        template: &CfmlValue,
    ) -> (
        String,
        ValueMap,
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
                    ValueMap::default(),
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
                CfmlValue::String(s) => Some(s.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "default".to_string());

        let mut config = ValueMap::default();
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
                        CfmlValue::String(p) => Some(p.to_string()),
                        CfmlValue::Struct(inner) => inner
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == "path")
                            .and_then(|(_, v)| match v {
                                CfmlValue::String(p) => Some(p.to_string()),
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
                CfmlValue::Bool(b) => b,
                CfmlValue::String(s) => s.to_lowercase() == "true" || s.to_lowercase() == "yes",
                _ => false,
            })
            .unwrap_or(false);

        let session_timeout = s
            .iter()
            .find(|(k, _)| k.to_lowercase() == "sessiontimeout")
            .and_then(|(_, v)| match v {
                // `createTimeSpan(d,h,m,s)` returns a Double expressed in
                // *days* (e.g. one hour = 1/24 ≈ 0.0417). It must be scaled
                // to seconds — casting the day-fraction straight to u64
                // truncated every sub-day timeout to 0, which yielded an
                // invalid KV `expiration_ttl(0)` and made sessions instantly
                // expirable (the root of "sessions never persist").
                CfmlValue::Double(d) => Some((d * 86_400.0).round() as u64),
                // A bare integer/string is taken as a literal seconds count.
                CfmlValue::Int(i) => Some(i as u64),
                CfmlValue::String(s) => s.parse::<u64>().ok(),
                _ => None,
            })
            // Never let a misconfigured/zero timeout through: clamp to KV's
            // 60s TTL floor so the value is always a valid expiration.
            .map(|secs| secs.max(MIN_SESSION_TIMEOUT_SECS))
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
                CfmlValue::Bool(b) => Some(b),
                _ => None,
            });

        // Extract this.sessionStorage — name of the named cache to use for sessions.
        let session_storage = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("sessionstorage"))
            .and_then(|(_, v)| match v {
                CfmlValue::String(st) if !st.is_empty() => Some(st.to_string()),
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
                                CfmlValue::String(s) => Some(s.to_string()),
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
                                            props.max_objects = i as u64;
                                        }
                                    }
                                    "defaulttimeout" => {
                                        if let CfmlValue::Int(i) = pv {
                                            props.default_timeout = i as u64;
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
            .find(|(k, _)| k.eq_ignore_ascii_case(&method_lower))
            .map(|(_, v)| v.clone());

        match func_val {
            Some(ref func @ CfmlValue::Function(_)) => {
                // Bind `this` and __variables as a single struct (not expanded)
                let mut parent_locals = ValueMap::default();
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
                    if let Some(ts) = template.as_cfml_struct() {
                        let vs = ts.get_or_insert_struct("__variables");
                        vs.merge_from(&vars_wb);
                    }
                }

                // Propagate this modifications back into template
                if let Some(modified_this) = self.method_this_writeback.take() {
                    if let Some(ts) = template.as_cfml_struct() {
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

    /// Invoke Application.cfc `onMissingTemplate(targetPage)`, returning the
    /// handler's CFML return value so the caller can honour its boolean
    /// contract (true/void = handled, false = fall through). This mirrors
    /// [`call_lifecycle_method`]'s `this`/`variables` binding and writeback,
    /// but — unlike that helper, which collapses every successful call to
    /// `Ok(true)` — it preserves the actual return value. `Ok(None)` means no
    /// `onMissingTemplate` method is defined on the component.
    fn call_missing_template(
        &mut self,
        template: &mut CfmlValue,
        target_page: CfmlValue,
    ) -> Result<Option<CfmlValue>, CfmlError> {
        let s = match template {
            CfmlValue::Struct(ref s) => s.clone(),
            _ => return Ok(None),
        };

        let func_val = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("onmissingtemplate"))
            .map(|(_, v)| v.clone());

        let func = match func_val {
            Some(f @ CfmlValue::Function(_)) => f,
            _ => return Ok(None),
        };

        // Bind `this` and __variables exactly as call_lifecycle_method does.
        let mut parent_locals = ValueMap::default();
        if let Some(vars) = s
            .iter()
            .find(|(k, _)| *k == "__variables")
            .map(|(_, v)| v.clone())
        {
            parent_locals.insert("__variables".to_string(), vars);
        }
        parent_locals.insert("this".to_string(), template.clone());

        let result = self.call_function(&func, vec![target_page], &parent_locals);

        // Propagate variables/this mutations back into the template.
        if let Some(vars_wb) = self.method_variables_writeback.take() {
            if let Some(ts) = template.as_cfml_struct() {
                let vs = ts.get_or_insert_struct("__variables");
                vs.merge_from(&vars_wb);
            }
        }
        if let Some(modified_this) = self.method_this_writeback.take() {
            if let Some(ts) = template.as_cfml_struct() {
                if let CfmlValue::Struct(ref modified_s) = modified_this {
                    for (k, v) in modified_s.iter() {
                        if k != "__variables" && k != "__extends" {
                            ts.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }

        result.map(Some)
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

    /// Tear down the application attached to this request, as `applicationStop()`
    /// requires: fire `onApplicationEnd`, clear the shared application scope, mark
    /// the app unstarted so the next request re-fires `onApplicationStart`, and
    /// drop every cached lifecycle function so the restarted app rebuilds them.
    fn stop_current_application(&mut self) {
        // Lucee fires onApplicationEnd(applicationScope) synchronously when
        // applicationStop() runs, BEFORE the scope is destroyed, so the handler
        // still sees the live application data. Snapshot the scope and invoke it
        // exactly like the onSessionEnd path does.
        let app_scope_snapshot = self
            .application_scope
            .as_ref()
            .map(|a| CfmlValue::strukt(a.snapshot()))
            .unwrap_or_else(|| CfmlValue::strukt(ValueMap::default()));
        if let Some(mut template) = self.app_cfc_template.take() {
            let _ = self.call_lifecycle_method(
                &mut template,
                "onApplicationEnd",
                vec![app_scope_snapshot],
            );
            self.app_cfc_template = Some(template);
        }

        if let Some(app_name) = self.current_application_name.clone() {
            if let Some(ref server_state) = self.server_state {
                server_state.applications.modify(&app_name, &mut |app| {
                    app.variables.clear();
                    app.started = false;
                    // Discard the carried function table so a restarted
                    // application re-homes from a clean slate.
                    app.app_function_table.clear();
                });
            }
        }

        // Clear the live scope so any further `application.*` access in this
        // request sees an empty scope, matching the destroyed shared state.
        if let Some(ref app_scope) = self.application_scope {
            app_scope.clear();
        }

        self.application_stopped = true;
    }

    /// Execute with Application.cfc lifecycle
    pub fn execute_with_lifecycle(&mut self) -> CfmlResult {
        self.application_stopped = false;
        self.current_application_name = None;
        // No function defined yet this request; the re-homing walk is skipped
        // unless a DefineFunction op flips this.
        self.app_fn_table_dirty = false;

        // 1. Find Application.cfc
        let app_cfc_path = self.find_application_cfc();

        let app_cfc_path = match app_cfc_path {
            Some(path) => path,
            None => {
                // No Application.cfc: there is no named application, but a single
                // run is still one execution context — give it a process/request
                // -lifetime application scope (when none is already attached) so
                // `application.*` writes persist within the run instead of being
                // silently dropped. Matches the expectation that an app-scoped
                // component (e.g. a WireBox app-scoped singleton) caches within a
                // run. Serve requests WITH an Application.cfc set their shared
                // scope above and never reach here.
                if self.application_scope.is_none() {
                    self.application_scope = Some(CfmlStruct::empty());
                }
                return self.execute(); // No Application.cfc, just execute directly
            }
        };

        // 1b. Application-level cfconfig: a `.cfconfig.json` beside this
        // Application.cfc overlays the server baseline for this request. Applied
        // before Application.cfc `this.*` settings so those still win on conflict.
        self.discover_app_cfconfig(&app_cfc_path);

        // 2. Load Application.cfc. A pseudo-constructor throw (or compile error)
        // aborts the request — Lucee does not fall through to the target page
        // when Application.cfc fails to load.
        let mut template = match self.load_application_cfc(&app_cfc_path) {
            Ok(Some(t)) => t,
            Ok(None) => return self.execute(), // No usable component, fall through
            Err(e) => return Err(e),
        };

        // 2b. Seed per-application datasources from `this.datasources` /
        // `this.datasource`. Overrides any cfconfig datasources for this request.
        self.seed_app_datasources_from_template(&template);

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

        // Lazy session creation is the engine-wide DEFAULT (issue #88):
        // no session record, no CFID cookie, and no `onSessionStart` until
        // code actually writes to the `session` scope. A page that only reads
        // session (or never touches it) mints nothing — privacy-friendly, and
        // it stops every crawler/`curl` hit from persisting an empty session.
        //
        // This is a deliberate divergence from Lucee 7 (which still mints the
        // cookie on a read/check) — see docs/known-issues.md. Apps that need
        // the historical eager behaviour opt back in with
        // `this.lazySessionCreation = false` (alias `this.lazySessions`).
        let lazy_session_creation = config
            .get("lazysessioncreation")
            .or_else(|| config.get("lazysessions"))
            .map(|v| match v {
                CfmlValue::Bool(b) => *b,
                CfmlValue::String(s) => {
                    let l = s.trim().to_ascii_lowercase();
                    !(l == "false" || l == "no" || l == "0")
                }
                _ => true,
            })
            .unwrap_or(true);
        self.lazy_session_creation = lazy_session_creation;

        // Resolve this.sessioncookie attribute overrides (secure / httponly /
        // samesite / domain / path). The embedder consults the resolved policy
        // when rendering the session Set-Cookie.
        self.session_cookie_policy =
            cfml_common::session_cookie::SessionCookiePolicy::from_app_config(&config);

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
                    variables: ValueMap::default(),
                    started: false,
                    config: config.clone(),
                    app_function_table: Vec::new(),
                    session_storage: app_session_storage.clone(),
                    app_caches: app_caches.clone(),
                };
                server_state.applications.insert(&app_name, app_state);
            }
            let app_snapshot = server_state.applications.get(&app_name).unwrap();
            self.current_application_name = Some(app_name.clone());
            self.application_scope = Some(CfmlStruct::new(app_snapshot.variables.clone()));
            // Carry the app-reachable function Arcs forward and register them
            // into `fn_registry` by global_id, so an application-scope function
            // whose source file isn't (re)loaded this request still resolves.
            self.app_function_table = app_snapshot.app_function_table.clone();
            let carried = self.app_function_table.clone();
            for f in &carried {
                self.register_fn(f);
            }

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

                // Functions created during onApplicationStart (factory beans,
                // resource CFCs) that stay reachable from application scope are
                // re-homed into the stable function table by the end-of-request
                // pass; no separate "delta added during start" cache is needed,
                // and a warm request needs no append/remap — app-scope `Function`
                // bodies already hold stable ids resolved via the table loaded at
                // request start.
                if let Err(e) =
                    self.call_lifecycle_method(&mut template, "onApplicationStart", vec![])
                {
                    let _ = self.call_lifecycle_method(
                        &mut template,
                        "onError",
                        vec![
                            CfmlValue::string(e.message.clone()),
                            CfmlValue::string("onApplicationStart".to_string()),
                        ],
                    );
                    return Err(e);
                }
            }
        } else {
            // CLI mode: fresh application scope each time
            self.application_scope = Some(CfmlStruct::empty());

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
            // Honour the Application.cfc timeout on the lazy-creation path
            // too: `lazy_init_session_if_pending` stamps new records with
            // `self.session_timeout_secs`, which otherwise keeps the 1800s
            // default and ignores `this.sessionTimeout`.
            self.session_timeout_secs = session_timeout;
            if let Some(ref server_state) = self.server_state.clone() {
                let sid = self.session_id.clone().unwrap_or_default();
                let has_sid = !sid.is_empty();
                let record_exists = has_sid && server_state.sessions.contains(&app_name, &sid);

                if record_exists {
                    // Existing session: bump last_accessed, no lifecycle fire.
                    if let Some(mut session) = server_state.sessions.get(&app_name, &sid) {
                        session.last_accessed_secs = now_epoch_secs();
                        session.timeout_secs = session_timeout;
                        server_state.sessions.set(&app_name, &sid, session);
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
                        &app_name,
                        &sid,
                        SessionData {
                            variables: ValueMap::default(),
                            created_secs: now,
                            last_accessed_secs: now,
                            auth_user: None,
                            auth_roles: Vec::new(),
                            timeout_secs: session_timeout,
                            app_name: app_name.clone(),
                        },
                    );
                    self.session_record_created = true;
                    let _ = self.call_lifecycle_method(&mut template, "onSessionStart", vec![]);
                }
            }
            // Attach the live session scope so `session` reads return a live
            // handle (scope-pointer pattern). Skip when lazy creation is pending:
            // there is no session record yet, so the scope attaches on the first
            // write via set_session_*.
            if !self.session_lazy_pending {
                self.attach_session_scope();
            }
        }

        // 5c. onMissingTemplate — the requested template does not exist on disk.
        // Adobe semantics: onApplicationStart/onSessionStart have already fired
        // above; onRequestStart/onRequest/onRequestEnd are SKIPPED. The handler
        // receives the web-root-relative requested path and returns `true` (or
        // nothing) to mark the request handled — suppressing the default 404 —
        // or `false` to fall through to it. A missing handler also falls through.
        // The embedder reads `missing_template_handled` to choose the response.
        if let Some(missing) = self.missing_template.clone() {
            self.missing_template_handled = match self.call_missing_template(
                &mut template,
                CfmlValue::string(missing.clone()),
            ) {
                // Ran and returned a value: anything but an explicit `false`
                // means handled (true / void / a string body are all "handled").
                Ok(Some(v)) => !matches!(v, CfmlValue::Bool(false)),
                // No onMissingTemplate defined: fall through to the default 404.
                Ok(None) => false,
                // cflocation/cfabort inside the handler is a deliberate response.
                Err(e) if e.message == "__cflocation_redirect" || e.message == "__cfabort" => true,
                // An uncaught error routes to onError (like any lifecycle event);
                // if onError handles it the request is considered handled.
                Err(e) => {
                    let r = self.run_on_error(&mut template, e, "onMissingTemplate");
                    self.missing_template_handled = r.is_ok();
                    if session_management {
                        let _ = self.sync_session_scope_to_store();
                    }
                    return r;
                }
            };
            // Persist session writes the handler may have made, then end the
            // request — no onRequestEnd for the missing-template path.
            if session_management {
                if let Err(e) = self.sync_session_scope_to_store() {
                    let _ = self.call_lifecycle_method(
                        &mut template,
                        "onError",
                        vec![
                            CfmlValue::string(e.message.clone()),
                            CfmlValue::string("onMissingTemplate".to_string()),
                        ],
                    );
                    return Err(e);
                }
            }
            return Ok(CfmlValue::Null);
        }

        // 6. onRequestStart
        let target_page = self.lifecycle_target_page();
        match self.call_lifecycle_method(
            &mut template,
            "onRequestStart",
            vec![CfmlValue::string(target_page.clone())],
        ) {
            Err(e) if e.message == "__cfabort" => {
                // A request aborted in onRequestStart still fires onAbort, then
                // ends (onRequest/target page/onRequestEnd are skipped).
                let _ = self.call_lifecycle_method(
                    &mut template,
                    "onAbort",
                    vec![CfmlValue::string(target_page.clone())],
                );
                return Ok(CfmlValue::Null);
            }
            Err(e) if e.message == "__cflocation_redirect" => {
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

        // `aborted` → the request unwound via cfabort/abort, so step 8 fires
        // `onAbort` instead of `onRequestEnd`. `errored` → an uncaught exception
        // was routed to `onError` (whether or not a handler existed): onError is
        // the terminal handler, so onRequestEnd is skipped (Lucee/ACF parity).
        let mut aborted = false;
        let mut errored = false;
        let result = if has_on_request {
            match self.call_lifecycle_method(
                &mut template,
                "onRequest",
                vec![CfmlValue::string(target_page.clone())],
            ) {
                Ok(_) => Ok(CfmlValue::Null),
                Err(e) if e.message == "__cflocation_redirect" => Ok(CfmlValue::Null),
                Err(e) if e.message == "__cfabort" => {
                    aborted = true;
                    Ok(CfmlValue::Null)
                }
                // An uncaught exception in onRequest is handed to onError with
                // the running event name; if no onError exists it surfaces.
                Err(e) => {
                    errored = true;
                    self.run_on_error(&mut template, e, "onRequest")
                }
            }
        } else {
            match self.execute() {
                Ok(v) => Ok(v),
                Err(e) if e.message == "__cflocation_redirect" => Ok(CfmlValue::Null),
                Err(e) if e.message == "__cfabort" => {
                    aborted = true;
                    Ok(CfmlValue::Null)
                }
                // An uncaught exception in the target page is handed to onError;
                // the event name is empty since no lifecycle event was running.
                Err(e) => {
                    errored = true;
                    self.run_on_error(&mut template, e, "")
                }
            }
        };

        // 8. onAbort (cfabort unwind) or onRequestEnd (normal completion).
        // Both are skipped when an uncaught error was routed to onError.
        if aborted {
            let _ = self.call_lifecycle_method(
                &mut template,
                "onAbort",
                vec![CfmlValue::string(target_page)],
            );
        } else if !errored {
            let _ = self.call_lifecycle_method(
                &mut template,
                "onRequestEnd",
                vec![CfmlValue::string(target_page)],
            );
        }

        // 8a. Persist the live session scope back to the store. Session
        // reads/writes during the request mutate a cached live CfmlStruct (so
        // the scope-pointer pattern works); commit it now before expiry runs.
        if session_management {
            // A data-only violation smuggled in via reference mutation surfaces
            // here (the airtight persist gate). The bad value is never written;
            // route the error through onError like other lifecycle failures.
            if let Err(e) = self.sync_session_scope_to_store() {
                let _ = self.call_lifecycle_method(
                    &mut template,
                    "onError",
                    vec![
                        CfmlValue::string(e.message.clone()),
                        CfmlValue::string("onRequestEnd".to_string()),
                    ],
                );
                return Err(e);
            }
        }

        // 8b. Session expiry — deliver any `onSessionEnd` hooks queued by the
        // background reaper for THIS application. The reaper evicts expired
        // session data off the request path (so a normal request pays ~zero
        // expiry cost); the hook itself needs the owning application's live
        // context, which only exists on a request, so it fires opportunistically
        // here. Keyed by app name, so a request only ever runs hooks for its own
        // application's expired sessions. (Cleanup-only delivery — onSessionEnd
        // is bounded by traffic to the application; see docs/known-issues.md.)
        if session_management {
            if let Some(ref server_state) = self.server_state.clone() {
                let pending = server_state.drain_session_ends(&app_name);
                if !pending.is_empty() {
                    let app_scope_val = self
                        .application_scope
                        .as_ref()
                        .map(|a| CfmlValue::strukt(a.snapshot()))
                        .unwrap_or(CfmlValue::strukt(ValueMap::default()));
                    for session_vars in pending {
                        // Call onSessionEnd(sessionScope, applicationScope)
                        let _ = self.call_lifecycle_method(
                            &mut template,
                            "onSessionEnd",
                            vec![CfmlValue::strukt(session_vars), app_scope_val.clone()],
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
        //
        // Skip entirely when `applicationStop()` ran this request: it already
        // reset the shared entry, and re-persisting here would re-anchor the
        // function cache from the just-cleared scope.
        if !self.application_stopped {
            if let Some(ref server_state) = self.server_state.clone() {
                // Re-home every function reachable from application scope into
                // the stable per-application table, rewriting their live
                // app-scope bodies to tagged stable ids. After this the snapshot
                // persisted below carries stable ids that resolve identically on
                // any later request — no per-request append, no remap, and the
                // stale-index bug class is gone by construction. Idempotent:
                // functions already re-homed on an earlier request are untouched.
                //
                // Skip the whole walk when no function was defined this request:
                // the table cannot have gained anything (a `Function` value is
                // only born via a DefineFunction op), so it is byte-identical to
                // what was loaded and needs no re-persist. The application-scope
                // *variables* are still written back below, since non-function
                // app state may have changed.
                let rehomed = self.app_fn_table_dirty;
                if rehomed {
                    self.rehome_application_functions();
                }
                if let Some(ref app_scope) = self.application_scope {
                    // Snapshot outside the store write so the application-scope
                    // lock and the applications-store lock never nest.
                    let scope = app_scope.snapshot();
                    // Only re-persist the carried function table when it changed
                    // (a function was defined this request); otherwise it is
                    // byte-identical to what was loaded.
                    let table_update = rehomed.then(|| self.app_function_table.clone());
                    server_state.applications.modify(&app_name, &mut |app| {
                        app.variables = scope.clone();
                        if let Some(table) = table_update.clone() {
                            app.app_function_table = table;
                        }
                    });
                }
            }
        }

        // 10. Clear request scope + stashed Application.cfc template.
        self.request_scope.clear();
        self.app_cfc_template = None;
        self.app_cfconfig = None;
        self.app_datasources.clear();
        self.app_default_datasource = None;
        self.session_lazy_pending = false;

        result
    }

    pub fn get_output(&self) -> String {
        self.output_buffer.clone()
    }

    /// Flush buffered `<cfhtmlhead>` / `<cfhtmlbody>` content into the response
    /// output. Head content is inserted just before the last `</head>`; body
    /// content just before the last `</body>`. When the corresponding marker is
    /// absent the content is appended to the end of the output so it is never
    /// silently dropped. Idempotent — clears the buffers after flushing. Called
    /// once at the end of request execution (CLI, serve, worker).
    pub fn finalize_html_injections(&mut self) {
        if !self.html_head_buffer.is_empty() {
            let head = std::mem::take(&mut self.html_head_buffer);
            if let Some(pos) = Self::rfind_ci(&self.output_buffer, "</head>") {
                self.output_buffer.insert_str(pos, &head);
            } else {
                self.output_buffer.push_str(&head);
            }
        }
        if !self.html_body_buffer.is_empty() {
            let body = std::mem::take(&mut self.html_body_buffer);
            if let Some(pos) = Self::rfind_ci(&self.output_buffer, "</body>") {
                self.output_buffer.insert_str(pos, &body);
            } else {
                self.output_buffer.push_str(&body);
            }
        }
    }

    /// ASCII-case-insensitive `rfind` returning the byte offset of the last
    /// match of `needle` in `haystack`. Operates on bytes so the returned offset
    /// is valid for the original string even with non-ASCII (multi-byte) content
    /// — `needle` is always an ASCII HTML close tag.
    fn rfind_ci(haystack: &str, needle: &str) -> Option<usize> {
        let h = haystack.as_bytes();
        let n = needle.as_bytes();
        if n.is_empty() || h.len() < n.len() {
            return None;
        }
        (0..=h.len() - n.len())
            .rev()
            .find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
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

/// Case-insensitively remove a key from a scope `IndexMap`, returning whether
/// anything was removed. Direct-case first (the common path), then a scan —
/// mirrors `CfmlStruct::remove_ci` for the plain-`IndexMap` scopes (locals/
/// globals) used by CFML null-assignment deletion.
fn imap_remove_ci(m: &mut ValueMap, key: &str) -> bool {
    if m.shift_remove(key).is_some() {
        return true;
    }
    let found = m.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned();
    if let Some(k) = found {
        m.shift_remove(&k);
        return true;
    }
    false
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
    // A QueryColumn proxy behaves as its first-row value in scalar numeric
    // contexts (arithmetic, coercion) — mirrors cfml_equal/cfml_compare.
    let val = val.query_column_scalar();
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
    // A QueryColumn proxy behaves as its first-row value in scalar comparison.
    let (a, b) = (a.query_column_scalar(), b.query_column_scalar());
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
    // A QueryColumn proxy behaves as its first-row value in scalar comparison.
    let (a, b) = (a.query_column_scalar(), b.query_column_scalar());
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
        BytecodeOp::LoadLocal(_)
        | BytecodeOp::LoadGlobal(_)
        | BytecodeOp::LoadVariablesKey(_)
        | BytecodeOp::TryLoadLocal(_) => (1, 0),
        // Variable stores: push 0, pop 1
        BytecodeOp::StoreLocal(_) | BytecodeOp::StoreGlobal(_) => (0, 1),
        // Fused in-place append: pops the value, pushes nothing
        BytecodeOp::ArrayAppendLocal(_) => (0, 1),
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
        BytecodeOp::LoadLocalKey(_) => (1, 0), // pushes value, reads nothing
        BytecodeOp::StoreLocalProperty(_, _) => (0, 1), // pops 1 (value), pushes 0
        BytecodeOp::SetProperty(_) => (0, 2), // obj + value → (modifies)
        BytecodeOp::SetDynamicVar => (1, 2),  // path + value → value
        BytecodeOp::UnsetPath(_) => (0, 0),   // value already popped by the guard
        BytecodeOp::GetKeys => (1, 1),
        BytecodeOp::ConcatArrays | BytecodeOp::MergeStructs => (1, 2),
        // Object
        BytecodeOp::NewObject(n) | BytecodeOp::NewObjectNamed(_, n) => (1, n + 1), // class + args → instance
        // Function definition: push 1
        BytecodeOp::DefineFunction(_) => (1, 0),
        // Postfix: push 1 (new value)
        BytecodeOp::Increment(_) | BytecodeOp::Decrement(_) => (1, 0),
        BytecodeOp::AddLocalConst(_, _) | BytecodeOp::MulLocalConst(_, _) => (0, 0), // pure local mutation, no stack traffic
        // Exception handling
        BytecodeOp::TryStart(_) | BytecodeOp::TryEnd => (0, 0),
        BytecodeOp::Throw | BytecodeOp::Rethrow => (0, 1),
        BytecodeOp::CatchMatch(_) => (1, 0), // peeks exception, pushes match bool
        // Method call: pops obj + args, pushes 1
        BytecodeOp::CallMethod(_, n, _) | BytecodeOp::CallMethodNamed(_, _, n, _) => (1, n + 1),
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
                | BytecodeOp::LoadGlobal(name)
                | BytecodeOp::LoadVariablesKey(name) = op
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

#[cfg(test)]
mod named_lock_tests {
    use super::evict_idle_named_locks;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    fn lock() -> Arc<RwLock<()>> {
        Arc::new(RwLock::new(()))
    }

    #[test]
    fn evicts_idle_entries_when_over_cap() {
        let mut locks: HashMap<String, Arc<RwLock<()>>> = HashMap::new();
        for i in 0..1024 {
            locks.insert(format!("idle_{i}"), lock());
        }
        // A held/contended lock: a second Arc clone keeps strong_count > 1.
        let held = lock();
        let _held_clone = held.clone();
        locks.insert("held".to_string(), held);

        assert_eq!(locks.len(), 1025);
        evict_idle_named_locks(&mut locks, "brand_new", 1024);

        // All idle entries (strong_count == 1) are gone; the held one survives.
        assert_eq!(locks.len(), 1, "idle entries should be evicted");
        assert!(locks.contains_key("held"), "held lock must never be evicted");
    }

    #[test]
    fn never_evicts_a_held_lock() {
        let mut locks: HashMap<String, Arc<RwLock<()>>> = HashMap::new();
        let held = lock();
        let held_clone = held.clone();
        let _guard = held_clone.write().unwrap(); // simulate a live held_locks guard
        locks.insert("held".to_string(), held);
        for i in 0..2000 {
            locks.insert(format!("idle_{i}"), lock());
        }
        evict_idle_named_locks(&mut locks, "another", 1024);
        assert!(locks.contains_key("held"));
    }

    #[test]
    fn no_eviction_below_cap() {
        let mut locks: HashMap<String, Arc<RwLock<()>>> = HashMap::new();
        for i in 0..10 {
            locks.insert(format!("idle_{i}"), lock());
        }
        evict_idle_named_locks(&mut locks, "new", 1024);
        assert_eq!(locks.len(), 10, "nothing evicted when under cap");
    }

    #[test]
    fn no_eviction_when_name_already_present() {
        let mut locks: HashMap<String, Arc<RwLock<()>>> = HashMap::new();
        for i in 0..1024 {
            locks.insert(format!("idle_{i}"), lock());
        }
        // Re-locking an existing name must not trigger an eviction sweep.
        evict_idle_named_locks(&mut locks, "idle_5", 1024);
        assert_eq!(locks.len(), 1024);
    }
}

#[cfg(test)]
mod app_cfc_discovery_tests {
    use super::CfmlVirtualMachine;
    use std::path::{Path, PathBuf};

    #[test]
    fn start_dir_handles_bare_filename_relative_and_absolute() {
        let cwd = Path::new("/work");
        // Bare filename (no directory component): the empty parent must resolve
        // to the current directory so the walk-up can find a sibling
        // Application.cfc. This was the bug — it used to start from "".
        assert_eq!(
            CfmlVirtualMachine::app_cfc_start_dir(Some("run_tests.cfm"), cwd),
            PathBuf::from("/work")
        );
        // Relative path with a directory component: use that directory.
        assert_eq!(
            CfmlVirtualMachine::app_cfc_start_dir(Some("tests/runner.cfm"), cwd),
            PathBuf::from("tests")
        );
        // Absolute path: use its parent directory unchanged.
        assert_eq!(
            CfmlVirtualMachine::app_cfc_start_dir(Some("/abs/dir/app.cfm"), cwd),
            PathBuf::from("/abs/dir")
        );
        // No source file: current directory.
        assert_eq!(
            CfmlVirtualMachine::app_cfc_start_dir(None, cwd),
            PathBuf::from("/work")
        );
    }
}

// ── Query-of-Queries helpers ────────────────────────────────────────────

/// Build engine bind parameters from the `queryExecute` params argument: an
/// Array → positional, a Struct → named. `cfqueryparam`-style `{value: …}`
/// wrappers are unwrapped to their underlying value.
fn build_qoq_params(arg: &CfmlValue) -> cfml_qoq::QoQParams {
    let mut params = cfml_qoq::QoQParams::none();
    match arg {
        CfmlValue::Array(a) => {
            params.positional = a.iter().map(|v| unwrap_query_param(&v)).collect();
        }
        CfmlValue::Struct(s) => {
            for (k, v) in s.iter() {
                params.named.insert(k, unwrap_query_param(&v));
            }
        }
        _ => {}
    }
    params
}

/// Unwrap a `cfqueryparam` struct (`{value: x, cfsqltype: …}`) to its value.
fn unwrap_query_param(v: &CfmlValue) -> CfmlValue {
    if let CfmlValue::Struct(s) = v {
        if let Some(inner) = s.get_ci("value") {
            return inner;
        }
    }
    v.clone()
}

/// Map positional cell values to a query's columns (extra values dropped,
/// missing ones filled with Null) — used by `queryAddRow` with array input.
fn positional_row(columns: &[String], values: &[CfmlValue]) -> ValueMap {
    let mut row = ValueMap::with_capacity_and_hasher(columns.len(), Default::default());
    for (i, col) in columns.iter().enumerate() {
        row.insert(col.clone(), values.get(i).cloned().unwrap_or(CfmlValue::Null));
    }
    row
}

/// Apply `returntype` to a QoQ result query: `array` → array of row structs,
/// `struct` (with `columnkey`) → struct keyed by that column, else the query.
fn convert_query_return(value: CfmlValue, return_type: &str, column_key: Option<&str>) -> CfmlValue {
    match return_type {
        "array" => {
            if let CfmlValue::Query(query) = &value {
                let rows: Vec<CfmlValue> =
                    query.rows().into_iter().map(CfmlValue::strukt).collect();
                return CfmlValue::array(rows);
            }
            value
        }
        "struct" => {
            if let (CfmlValue::Query(query), Some(key)) = (&value, column_key) {
                let mut out: ValueMap = ValueMap::default();
                for r in query.rows() {
                    let k = r
                        .iter()
                        .find(|(c, _)| c.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v.as_string())
                        .unwrap_or_default();
                    out.insert(k, CfmlValue::strukt(r));
                }
                return CfmlValue::strukt(out);
            }
            value
        }
        _ => value,
    }
}
