//! Strongly-typed schema for `.cfconfig.json`.
//!
//! Every field is `#[serde(default)]`. Unknown keys are silently ignored so a
//! config file authored for Lucee/BoxLang loads cleanly even when it carries
//! engine-specific sections RustCFML cannot use.
//!
//! All string fields support `${env.VAR:default}` placeholders, expanded in a
//! single pass by [`RustCfmlConfig::expand_env`] right after parse.

use indexmap::IndexMap;
use serde::Deserialize;
use std::path::PathBuf;

use crate::env::expand_env_vars;

// ─────────────────────────────────────────────
// Root
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct RustCfmlConfig {
    pub server: ServerCfg,
    pub runtime: RuntimeCfg,
    pub datasources: IndexMap<String, DatasourceCfg>,
    /// Component/path mappings (virtual prefix → physical directory). Accepts
    /// both RustCFML's native `mappings` key and the CommandBox/cfconfig
    /// `CFMappings` alias, and per-value either a plain string path or the
    /// cfconfig object form `{ "physical": "/path", "primary": "physical" }`.
    #[serde(alias = "CFMappings", deserialize_with = "de_mappings", default)]
    pub mappings: IndexMap<String, String>,
    #[serde(rename = "customTagPaths")]
    pub custom_tag_paths: Vec<String>,
    #[serde(rename = "mailServers")]
    pub mail_servers: Vec<MailServerCfg>,
    pub caches: IndexMap<String, CacheCfg>,
    #[serde(rename = "sessionStorage")]
    pub session_storage: String,
    pub session: SessionCfg,
    pub logging: LoggingCfg,
    pub debugging: DebuggingCfg,
    /// RustCFML-native observability subsystems (sampling profiler, OpenTelemetry,
    /// DAP debugger). Distinct from `debugging`, which is the Lucee-compatible
    /// classic footer. See `docs/observability-*.md`.
    pub observability: ObservabilityCfg,
    pub security: SecurityCfg,
    #[serde(rename = "urlRewriting")]
    pub url_rewriting: UrlRewritingCfg,

    /// Set by the loader; not part of the JSON schema. `None` when the config
    /// was synthesised from defaults (no file found) or parsed from a string.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

/// Deserialize a mappings map whose values are either a plain string path or
/// the cfconfig object form `{ "physical": "/path", "primary": "physical" }`.
/// (CommandBox/cfconfig writes the object form; RustCFML's own files use the
/// string form.) Either way the resolved value is the physical directory.
fn de_mappings<'de, D>(d: D) -> Result<IndexMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MapVal {
        Path(String),
        Detailed {
            #[serde(default)]
            physical: String,
        },
    }
    let raw: IndexMap<String, MapVal> = IndexMap::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                match v {
                    MapVal::Path(s) => s,
                    MapVal::Detailed { physical } => physical,
                },
            )
        })
        .collect())
}

// ─────────────────────────────────────────────
// Server
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ServerCfg {
    pub host: String,
    // NOTE: the listening port is intentionally NOT a cfconfig setting. The port
    // is a server/environment concern, set via `--port` (or its default). cfconfig
    // is application-level config; a per-app `.cfconfig.json` must never be able to
    // change the port. A stray `"port"` key in a config file is silently ignored.
    pub webroot: String,
    #[serde(rename = "welcomeFiles")]
    pub welcome_files: Vec<String>,
    #[serde(rename = "cfmlExtensions")]
    pub cfml_extensions: Vec<String>,
    #[serde(rename = "maxConcurrentRequests")]
    pub max_concurrent_requests: u32,
    /// Bytes. `0` = unlimited.
    #[serde(rename = "maxRequestBodySize")]
    pub max_request_body_size: u64,
    /// Seconds. `0` = no timeout.
    #[serde(rename = "requestTimeout")]
    pub request_timeout: u32,
    pub http2: bool,
    /// Front-controller fallback: run a configured template for URLs that
    /// resolve to no file, instead of returning 404.
    pub fallback: FallbackCfg,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            webroot: String::new(),
            welcome_files: vec!["index.cfm".into(), "index.htm".into(), "index.html".into()],
            cfml_extensions: vec!["cfm".into(), "cfc".into()],
            max_concurrent_requests: 0,
            max_request_body_size: 10 * 1024 * 1024,
            request_timeout: 0,
            http2: false,
            fallback: FallbackCfg::default(),
        }
    }
}

/// Front-controller fallback routing. When `template` is non-empty, any URL
/// that resolves to no file (would otherwise 404) is dispatched to that
/// web-root-relative CFML template, with the original path exposed in the URL
/// scope under `route_param` and the original query string preserved.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct FallbackCfg {
    /// Web-root-relative CFML template to run for unresolved URLs. Empty = off.
    pub template: String,
    /// URL var name that receives the original (unresolved) path.
    #[serde(rename = "routeParam")]
    pub route_param: String,
}

impl Default for FallbackCfg {
    fn default() -> Self {
        Self {
            template: String::new(),
            route_param: "route".into(),
        }
    }
}

// ─────────────────────────────────────────────
// Runtime
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct RuntimeCfg {
    #[serde(rename = "nullSupport")]
    pub null_support: bool,
    #[serde(rename = "dotNotationUpperCase")]
    pub dot_notation_upper_case: bool,
    pub locale: String,
    pub timezone: String,
    #[serde(rename = "whitespaceCompressionEnabled")]
    pub whitespace_compression_enabled: bool,
    #[serde(rename = "trustedCache")]
    pub trusted_cache: bool,
    /// When true, `server.coldfusion.productname` reports `"Lucee"` instead of
    /// `"RustCFML"`. RustCFML targets the Lucee dialect and already advertises
    /// `server.lucee`, but some frameworks (e.g. ColdBox's mapping-helper
    /// selection) branch specifically on `productname` and only take their
    /// Lucee code path when it equals "Lucee". Opt in per-app when a framework
    /// needs that. `server.lucee.versionName` stays `"RustCFML"` regardless, so
    /// engine self-identification is unaffected.
    #[serde(rename = "reportAsLucee")]
    pub report_as_lucee: bool,
    /// `"days,hours,minutes,seconds"`.
    #[serde(rename = "applicationTimeout")]
    pub application_timeout: String,
    #[serde(rename = "sessionTimeout")]
    pub session_timeout: String,
    #[serde(rename = "clientTimeout")]
    pub client_timeout: String,
}

impl Default for RuntimeCfg {
    fn default() -> Self {
        Self {
            null_support: false,
            dot_notation_upper_case: true,
            locale: String::new(),
            timezone: String::new(),
            whitespace_compression_enabled: false,
            trusted_cache: false,
            report_as_lucee: false,
            application_timeout: "1,0,0,0".into(),
            session_timeout: "0,0,30,0".into(),
            client_timeout: "7,0,0,0".into(),
        }
    }
}

impl RuntimeCfg {
    /// Convert a `"d,h,m,s"` timeout string to total seconds. Returns `None`
    /// on parse failure so callers can fall back to a hard-coded default.
    pub fn parse_timeout_seconds(spec: &str) -> Option<u64> {
        let mut parts = spec.split(',').map(str::trim).map(str::parse::<u64>);
        let d = parts.next()?.ok()?;
        let h = parts.next()?.ok()?;
        let m = parts.next()?.ok()?;
        let s = parts.next()?.ok()?;
        Some(d * 86_400 + h * 3_600 + m * 60 + s)
    }
}

// ─────────────────────────────────────────────
// Session reaper
// ─────────────────────────────────────────────

/// Background session-expiry reaper settings (serve mode only). The reaper
/// drains expired sessions off the request path on a timer, so a normal
/// request pays ~zero expiry cost and idle servers still evict expired data.
/// `onSessionEnd` itself fires opportunistically on the next request for the
/// owning application (cleanup-only delivery — see docs/known-issues.md).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct SessionCfg {
    /// Reaper tick in seconds. `0` disables the background reaper entirely
    /// (read-path exactness + native store TTL still apply).
    #[serde(rename = "reapIntervalSecs")]
    pub reap_interval_secs: u64,
    /// When true, sleep until the next session's expiry instant (capped at
    /// `reapIntervalSecs`) instead of waking on the fixed interval. Only stores
    /// that can compute the next expiry cheaply benefit; others fall back to
    /// the fixed tick.
    #[serde(rename = "reapAdaptive")]
    pub reap_adaptive: bool,
    /// Maximum number of pending `onSessionEnd` deliveries buffered per
    /// application between requests. Beyond this the oldest are dropped (with a
    /// log line) so a never-revisited application cannot leak memory.
    #[serde(rename = "reapBatchMax")]
    pub reap_batch_max: usize,
}

impl Default for SessionCfg {
    fn default() -> Self {
        Self {
            reap_interval_secs: 60,
            reap_adaptive: false,
            reap_batch_max: 1000,
        }
    }
}

// ─────────────────────────────────────────────
// Datasources
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DatasourceCfg {
    /// Native driver id (`mysql`, `postgresql`, `mssql`, `sqlite`, …). Also
    /// accepts the Lucee/ACF `type` and `dbdriver` keys as aliases, so a
    /// standard `this.datasources` / `.cfconfig.json` entry declared the Lucee
    /// way (`{ type: "MySQL", … }`) resolves to the right driver.
    #[serde(alias = "type", alias = "dbdriver")]
    pub driver: String,
    /// JDBC driver class name (e.g. `com.mysql.cj.jdbc.Driver`). Used as a
    /// fallback when `driver`/`type` is absent.
    #[serde(default)]
    pub class: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(rename = "connectionString")]
    pub connection_string: String,
    #[serde(rename = "connectionLimit")]
    pub connection_limit: i32,
    #[serde(rename = "connectionTimeout")]
    pub connection_timeout: u32,
    #[serde(rename = "idleTimeout")]
    pub idle_timeout: u32,
    pub timezone: String,
    pub default: bool,
}

impl DatasourceCfg {
    /// Build a connection string that the cfml-stdlib query driver layer can
    /// consume. Honors `connectionString` verbatim when provided; otherwise
    /// synthesises a URL from `driver` + host/port/database/credentials.
    /// Returns `None` for an unsupported / unrecognised driver.
    pub fn connection_url(&self) -> Option<String> {
        if !self.connection_string.is_empty() {
            return Some(self.connection_string.clone());
        }
        let driver = self.canonical_driver();
        let creds = if self.username.is_empty() && self.password.is_empty() {
            String::new()
        } else if self.password.is_empty() {
            format!("{}@", self.username)
        } else {
            format!("{}:{}@", self.username, self.password)
        };
        let port = if self.port.is_empty() {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        match driver.as_str() {
            "mysql" | "mariadb" => Some(format!(
                "mysql://{}{}{}/{}",
                creds, self.host, port, self.database
            )),
            "postgresql" | "postgres" => Some(format!(
                "postgresql://{}{}{}/{}",
                creds, self.host, port, self.database
            )),
            "mssql" | "sqlserver" => Some(format!(
                "mssql://{}{}{}/{}",
                creds, self.host, port, self.database
            )),
            "sqlite" => Some(format!("sqlite://{}", self.database)),
            _ => None,
        }
    }

    /// Resolve the canonical lowercase driver id from the `driver` key (also
    /// aliased from Lucee's `type`/`dbdriver`) or, failing that, a JDBC `class`
    /// name. Lucee/ACF apps declare drivers as `type:"MySQL"` or via the JDBC
    /// driver class; both normalise to the same ids the URL builder understands.
    pub fn canonical_driver(&self) -> String {
        let raw = if !self.driver.is_empty() {
            &self.driver
        } else {
            &self.class
        };
        let lc = raw.trim().to_ascii_lowercase();
        match lc.as_str() {
            // JDBC driver class names (cfconfig `class` / Lucee `class`).
            "com.mysql.cj.jdbc.driver" | "com.mysql.jdbc.driver" => "mysql".to_string(),
            "org.mariadb.jdbc.driver" => "mariadb".to_string(),
            "org.postgresql.driver" => "postgresql".to_string(),
            "com.microsoft.sqlserver.jdbc.sqlserverdriver"
            | "net.sourceforge.jtds.jdbc.driver" => "mssql".to_string(),
            "org.sqlite.jdbc" => "sqlite".to_string(),
            // Otherwise treat it as a driver id / Lucee `type` (e.g. "MySQL",
            // "PostgreSQL", "MSSQL") — the URL builder match handles the rest.
            other => other.to_string(),
        }
    }
}

// ─────────────────────────────────────────────
// Mail
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct MailServerCfg {
    pub smtp: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub tls: bool,
    pub ssl: bool,
    pub timeout: u32,
}

// ─────────────────────────────────────────────
// Caches
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct CacheCfg {
    /// RustCFML / BoxLang-style provider name: "memory", "memcached", "cluster".
    pub provider: String,
    /// Lucee-style Java class name (e.g. "org.lucee.extension.io.cache.memcache.MemCacheRaw").
    /// When non-empty and `provider` is empty, the class is mapped to the equivalent provider.
    pub class: String,
    /// Must be `true` for the cache to be eligible for session/client storage.
    /// Lucee requires this flag explicitly; RustCFML emits a warning when it is
    /// absent but does not refuse to use the cache.
    pub storage: bool,
    /// Lucee-style flat property map (all values are strings). Used when a
    /// `.cfconfig.json` was exported from Lucee — the Memcached extension stores
    /// connection details here rather than in `properties`.
    pub custom: IndexMap<String, String>,
    pub properties: CacheProperties,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct CacheProperties {
    // Generic cache settings
    #[serde(rename = "maxObjects")]
    pub max_objects: u64,
    #[serde(rename = "defaultTimeout")]
    pub default_timeout: u64,
    #[serde(rename = "evictionPolicy")]
    pub eviction_policy: String,

    // memcached provider
    /// Memcached server addresses, e.g. ["localhost:11211"]
    pub servers: Vec<String>,
    /// Key prefix prepended to every session ID stored in Memcached.
    /// Defaults to "rustcfml:sess:".
    #[serde(rename = "keyPrefix")]
    pub key_prefix: String,

    // datasource provider (SQL-backed session storage)
    /// Name of a configured datasource (see top-level `datasources`) that backs
    /// session storage. Resolved through the same registry cfquery/queryExecute
    /// use. When `sessionStorage` names a datasource directly (no cache entry),
    /// this is filled in automatically.
    pub datasource: String,
    /// Table name for the datasource provider. Defaults to "cf_session_data".
    /// Auto-created (`CREATE TABLE IF NOT EXISTS`) on first use.
    pub table: String,

    // cluster provider (memberlist + CRDT)
    /// UDP/QUIC address this node binds for cluster gossip. Default "0.0.0.0:7946".
    #[serde(rename = "listenAddr")]
    pub listen_addr: String,
    /// Public address advertised to other cluster members (required when
    /// `listenAddr` binds 0.0.0.0). Leave empty to use `listenAddr`.
    #[serde(rename = "advertiseAddr")]
    pub advertise_addr: String,
    /// Seed node addresses used to bootstrap cluster membership.
    /// Legacy: when `discovery` is not specified but `seeds` is non-empty,
    /// behaves as `discovery.method = "static"`.
    pub seeds: Vec<String>,
    /// Stable human-readable node name. Defaults to hostname:listenPort.
    #[serde(rename = "nodeName")]
    pub node_name: String,
    /// Peer discovery strategy. When absent, falls back to static seeds
    /// (see `seeds`) for backwards compatibility.
    pub discovery: Discovery,
}

/// Cluster-peer discovery configuration.
///
/// `method` selects one of:
/// - `"static"`  — use `seeds` from the parent properties (or `seeds` here)
/// - `"dns"`     — resolve `name` to A/AAAA records every `interval`
/// - `"multicast"` — broadcast self on `group:port` every `interval`
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Discovery {
    /// "static" | "dns" | "multicast". Empty falls back to "static".
    pub method: String,

    // dns + static
    /// DNS name to resolve, or for static an inline seed list (via `seeds`).
    pub name: String,
    /// Port to attach to addresses returned by DNS resolution.
    /// Defaults to the cluster listen port.
    pub port: u16,
    /// Optional explicit seed list (overrides parent `seeds` when set).
    pub seeds: Vec<String>,

    // multicast
    /// IPv4 multicast group, e.g. "239.255.42.42". Admin-scoped (239/8) recommended.
    pub group: String,

    // shared
    /// Refresh interval in seconds. Default 10s for dns, 5s for multicast.
    #[serde(rename = "intervalSecs")]
    pub interval_secs: u64,
}

impl Default for Discovery {
    fn default() -> Self {
        Self {
            method: String::new(),
            name: String::new(),
            port: 0,
            seeds: Vec::new(),
            group: "239.255.42.42".into(),
            interval_secs: 0,
        }
    }
}

impl Default for CacheProperties {
    fn default() -> Self {
        Self {
            max_objects: 1000,
            default_timeout: 3600,
            eviction_policy: "LRU".into(),
            servers: Vec::new(),
            key_prefix: "rustcfml:sess:".into(),
            datasource: String::new(),
            table: String::new(),
            listen_addr: "0.0.0.0:7946".into(),
            advertise_addr: String::new(),
            seeds: Vec::new(),
            node_name: String::new(),
            discovery: Discovery::default(),
        }
    }
}

// ─────────────────────────────────────────────
// Logging
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct LoggingCfg {
    #[serde(rename = "logsDirectory")]
    pub logs_directory: String,
    pub level: String,
    pub format: String,
    pub loggers: IndexMap<String, LoggerCfg>,
}

impl Default for LoggingCfg {
    fn default() -> Self {
        Self {
            logs_directory: String::new(),
            level: "warn".into(),
            format: "text".into(),
            loggers: IndexMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct LoggerCfg {
    pub level: String,
    pub appender: String,
}

// ─────────────────────────────────────────────
// Debugging
// ─────────────────────────────────────────────

/// Classic CF debug output (the footer/panel). Modelled on Lucee 6/7's
/// `debugging` block so a `.cfconfig.json` authored for Lucee is drop-in
/// compatible, with two RustCFML enhancements: a fully configurable URL trigger
/// (param name *and* value) and reverse-proxy-aware client-IP resolution.
///
/// `enabled` is the master switch (off by default). The footer renders only
/// when all four activation gates pass: enabled, viewer-allowed (IP whitelist
/// OR URL trigger), not suppressed by `<cfsetting showDebugOutput="false">`,
/// and the response is renderable HTML. See `docs/observability-*.md`.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DebuggingCfg {
    /// Master switch (Lucee `debuggingEnabled`). Set `true` and restrict
    /// `showFromIPs` to run live in production with no leakage to other visitors.
    pub enabled: bool,
    /// The security gate: only these client IPs (and the URL trigger) see the
    /// footer. Honoured in production too. Exact-match for stage 1; CIDR ranges
    /// are a documented follow-up.
    #[serde(rename = "showFromIPs", alias = "showfromips")]
    pub show_from_ips: Vec<String>,
    /// Reverse-proxy client-IP resolution. `false` (default) uses the socket
    /// peer; `true` trusts `X-Forwarded-For` / `X-Real-IP` (the documented
    /// foot-gun — only safe when your edge overwrites the header on ingress).
    #[serde(rename = "trustForwardedFor")]
    pub trust_forwarded_for: bool,
    /// RustCFML enhancement — a configurable URL trigger (Lucee core matches by
    /// IP only). Both the param NAME and required value are configurable, so a
    /// secret `?myhiddenvar=s3cr3t` can gate the footer (security-by-obscurity).
    #[serde(rename = "urlTrigger")]
    pub url_trigger: UrlTriggerCfg,
    /// `modern` (default) | `classic` | `simple` | `comment` | `none`.
    pub template: String,
    /// Slow-row red-highlight threshold in ms (Adobe/Lucee universal default 250).
    #[serde(rename = "highlightMs")]
    pub highlight_ms: u64,
    /// Rolling per-section row cap (≈ Lucee `debugMaxRecordsLogged`).
    #[serde(rename = "maxRecords")]
    pub max_records: usize,
    /// The seven Lucee section toggles + the scope-dump selection.
    pub fields: DebugFieldsCfg,

    // ── Error-page settings (pre-existing; unrelated to the footer) ──
    #[serde(rename = "errorTemplate")]
    pub error_template: String,
    #[serde(rename = "errorStatusCode")]
    pub error_status_code: bool,
    #[serde(rename = "showExecutionTime")]
    pub show_execution_time: bool,
}

impl Default for DebuggingCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            show_from_ips: vec!["127.0.0.1".into(), "::1".into()],
            trust_forwarded_for: false,
            url_trigger: UrlTriggerCfg::default(),
            template: "modern".into(),
            highlight_ms: 250,
            max_records: 10,
            fields: DebugFieldsCfg::default(),
            error_template: String::new(),
            error_status_code: true,
            show_execution_time: false,
        }
    }
}

/// Configurable URL trigger for the debug footer (RustCFML enhancement).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct UrlTriggerCfg {
    pub enabled: bool,
    /// The URL/form variable NAME (default `debug`). Rename for obscurity.
    pub param: String,
    /// Required value (default `true`). Set an unguessable secret to gate by it;
    /// empty = presence-only (refused when `production_mode` is on).
    pub value: String,
}

impl Default for UrlTriggerCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            param: "debug".into(),
            value: "true".into(),
        }
    }
}

/// Lucee's seven section toggles plus the scope-dump selection.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DebugFieldsCfg {
    pub database: bool,
    pub exception: bool,
    pub tracing: bool,
    pub timer: bool,
    #[serde(rename = "implicitAccess")]
    pub implicit_access: bool,
    #[serde(rename = "queryUsage")]
    pub query_usage: bool,
    pub dump: bool,
    /// Which scopes the scope-dump renders. Never `variables`/`local`.
    pub scopes: Vec<String>,
}

impl Default for DebugFieldsCfg {
    fn default() -> Self {
        Self {
            database: true,
            exception: true,
            tracing: true,
            timer: true,
            implicit_access: false,
            query_usage: false,
            dump: true,
            scopes: vec!["cgi".into(), "url".into(), "form".into()],
        }
    }
}

// ─────────────────────────────────────────────
// Observability (profiler / OTel / DAP) — RustCFML-native
// ─────────────────────────────────────────────

/// The RustCFML-native observability subsystems. `enabled` is the master switch;
/// each subsystem also has its own `enabled` flag so they can be armed
/// independently. All default off — zero cost until explicitly turned on.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ObservabilityCfg {
    /// Master switch for the observability subsystems (default off).
    pub enabled: bool,
    /// Threshold-gated cooperative sampling profiler (Phase 2).
    pub profiler: ProfilerCfg,
}

impl Default for ObservabilityCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            profiler: ProfilerCfg::default(),
        }
    }
}

/// Threshold-gated cooperative sampling profiler. When a request runs longer
/// than `thresholdMs`, a watchdog thread asks the request's own VM to snapshot
/// its CFML call stack every `intervalMs`, up to `maxSamples`. Fast requests
/// pay nothing (one relaxed atomic load per source line, always false).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ProfilerCfg {
    /// Arm the profiler (default off). When off, the VM never installs a profile
    /// handle and the per-line check compiles to a `None` branch.
    pub enabled: bool,
    /// Only requests slower than this begin sampling (ms).
    #[serde(rename = "thresholdMs")]
    pub threshold_ms: u64,
    /// Sampling cadence once armed (ms).
    #[serde(rename = "intervalMs")]
    pub interval_ms: u64,
    /// Hard cap on samples per request (bounds memory on a runaway request).
    #[serde(rename = "maxSamples")]
    pub max_samples: u32,
    /// How often the watchdog scans in-flight requests (ms).
    #[serde(rename = "watchdogTickMs")]
    pub watchdog_tick_ms: u64,
}

impl Default for ProfilerCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_ms: 3000,
            interval_ms: 200,
            max_samples: 500,
            watchdog_tick_ms: 50,
        }
    }
}

// ─────────────────────────────────────────────
// Security
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct SecurityCfg {
    pub sandbox: bool,
    #[serde(rename = "disallowedFunctions")]
    pub disallowed_functions: Vec<String>,
    #[serde(rename = "disallowedImports")]
    pub disallowed_imports: Vec<String>,
    #[serde(rename = "blockedPaths")]
    pub blocked_paths: Vec<String>,
    #[serde(rename = "csrfEnabled")]
    pub csrf_enabled: bool,
    #[serde(rename = "secureJSON")]
    pub secure_json: bool,
    #[serde(rename = "secureJSONPrefix")]
    pub secure_json_prefix: String,
}

impl Default for SecurityCfg {
    fn default() -> Self {
        Self {
            sandbox: false,
            disallowed_functions: Vec::new(),
            disallowed_imports: Vec::new(),
            blocked_paths: vec![
                "*.cfm.bak".into(),
                "*.cfm~".into(),
                "Application.cfc".into(),
                "*.config.cfm".into(),
            ],
            csrf_enabled: true,
            secure_json: false,
            secure_json_prefix: "//".into(),
        }
    }
}

// ─────────────────────────────────────────────
// URL rewriting
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct UrlRewritingCfg {
    #[serde(rename = "configFile")]
    pub config_file: String,
    pub enabled: bool,
}

impl Default for UrlRewritingCfg {
    fn default() -> Self {
        Self {
            config_file: "urlrewrite.xml".into(),
            enabled: true,
        }
    }
}

// ─────────────────────────────────────────────
// Env expansion
// ─────────────────────────────────────────────

impl RustCfmlConfig {
    /// Walk every string field and expand `${env.VAR:default}` placeholders
    /// in place. Called automatically after parse.
    pub fn expand_env(&mut self) {
        // server
        expand(&mut self.server.host);
        expand(&mut self.server.webroot);
        for s in &mut self.server.welcome_files {
            expand(s);
        }
        for s in &mut self.server.cfml_extensions {
            expand(s);
        }
        // runtime
        expand(&mut self.runtime.locale);
        expand(&mut self.runtime.timezone);
        expand(&mut self.runtime.application_timeout);
        expand(&mut self.runtime.session_timeout);
        expand(&mut self.runtime.client_timeout);
        // datasources
        for ds in self.datasources.values_mut() {
            expand(&mut ds.driver);
            expand(&mut ds.host);
            expand(&mut ds.port);
            expand(&mut ds.database);
            expand(&mut ds.username);
            expand(&mut ds.password);
            expand(&mut ds.connection_string);
            expand(&mut ds.timezone);
        }
        // mappings: keys are virtual paths (rarely templated), values are physical
        let new_mappings: IndexMap<String, String> = self
            .mappings
            .iter()
            .map(|(k, v)| (k.clone(), expand_env_vars(v)))
            .collect();
        self.mappings = new_mappings;
        // custom tag paths
        for s in &mut self.custom_tag_paths {
            expand(s);
        }
        // mail
        for m in &mut self.mail_servers {
            expand(&mut m.smtp);
            expand(&mut m.username);
            expand(&mut m.password);
        }
        // caches
        for c in self.caches.values_mut() {
            expand(&mut c.provider);
            expand(&mut c.class);
            for v in c.custom.values_mut() {
                expand(v);
            }
            expand(&mut c.properties.eviction_policy);
        }
        expand(&mut self.session_storage);
        // logging
        expand(&mut self.logging.logs_directory);
        expand(&mut self.logging.level);
        expand(&mut self.logging.format);
        for l in self.logging.loggers.values_mut() {
            expand(&mut l.level);
            expand(&mut l.appender);
        }
        // debugging
        expand(&mut self.debugging.error_template);
        expand(&mut self.debugging.template);
        expand(&mut self.debugging.url_trigger.param);
        expand(&mut self.debugging.url_trigger.value);
        for s in &mut self.debugging.show_from_ips {
            expand(s);
        }
        // security
        for s in &mut self.security.disallowed_functions {
            expand(s);
        }
        for s in &mut self.security.disallowed_imports {
            expand(s);
        }
        for s in &mut self.security.blocked_paths {
            expand(s);
        }
        expand(&mut self.security.secure_json_prefix);
        // url rewriting
        expand(&mut self.url_rewriting.config_file);
    }
}

fn expand(s: &mut String) {
    if s.contains("${") {
        *s = expand_env_vars(s);
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_uses_defaults() {
        let cfg: RustCfmlConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.runtime.session_timeout, "0,0,30,0");
        assert!(cfg.security.csrf_enabled);
        assert!(cfg.url_rewriting.enabled);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let json = r#"{
            "server": {"host": "0.0.0.0", "port": 9000, "luceeOnlyKey": 42},
            "extensions": [{"id": "lucee-thing"}],
            "adminPassword": "secret"
        }"#;
        let cfg: RustCfmlConfig = serde_json::from_str(json).unwrap();
        // `port` is intentionally not a schema field — it is silently ignored,
        // like any other unknown key.
        assert_eq!(cfg.server.host, "0.0.0.0"); // known key still parses
    }

    #[test]
    fn datasource_parses() {
        let json = r#"{
            "datasources": {
                "myDSN": {
                    "driver": "mysql",
                    "host": "localhost",
                    "port": "3306",
                    "database": "mydb",
                    "default": true
                }
            }
        }"#;
        let cfg: RustCfmlConfig = serde_json::from_str(json).unwrap();
        let ds = cfg.datasources.get("myDSN").expect("missing dsn");
        assert_eq!(ds.driver, "mysql");
        assert!(ds.default);
    }

    #[test]
    fn datasource_lucee_type_key_is_accepted_as_driver_alias() {
        // GitHub #173: Lucee/ACF/Preside declare datasources with `type` rather
        // than RustCFML's `driver`. It must alias onto `driver`.
        let json = r#"{
            "datasources": {
                "ds": {
                    "type": "MySQL",
                    "host": "127.0.0.1",
                    "port": "3309",
                    "database": "preside_test",
                    "username": "root",
                    "password": "password"
                }
            }
        }"#;
        let cfg: RustCfmlConfig = serde_json::from_str(json).unwrap();
        let ds = cfg.datasources.get("ds").unwrap();
        assert_eq!(ds.driver, "MySQL");
        assert_eq!(ds.canonical_driver(), "mysql");
        assert_eq!(
            ds.connection_url().unwrap(),
            "mysql://root:password@127.0.0.1:3309/preside_test"
        );
    }

    #[test]
    fn datasource_dbdriver_alias_and_jdbc_class_normalise() {
        // `dbdriver` is another Lucee alias for the driver id.
        let mut ds = DatasourceCfg::default();
        ds.driver = "PostgreSQL".into();
        assert_eq!(ds.canonical_driver(), "postgresql");

        // A JDBC `class` name resolves when no driver/type is given.
        let json = r#"{
            "datasources": { "ds": { "class": "com.mysql.cj.jdbc.Driver", "database": "x" } }
        }"#;
        let cfg: RustCfmlConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.datasources.get("ds").unwrap().canonical_driver(), "mysql");
    }

    #[test]
    fn timeout_parser_handles_typical_inputs() {
        assert_eq!(RuntimeCfg::parse_timeout_seconds("0,0,30,0"), Some(1800));
        assert_eq!(RuntimeCfg::parse_timeout_seconds("1,0,0,0"), Some(86_400));
        assert_eq!(RuntimeCfg::parse_timeout_seconds("0, 1, 0, 0"), Some(3600));
        assert_eq!(RuntimeCfg::parse_timeout_seconds("bad"), None);
        assert_eq!(RuntimeCfg::parse_timeout_seconds("1,2,3"), None);
    }

    #[test]
    fn datasource_connection_url_mysql() {
        let mut ds = DatasourceCfg::default();
        ds.driver = "mysql".into();
        ds.host = "db.example.com".into();
        ds.port = "3306".into();
        ds.database = "app".into();
        ds.username = "u".into();
        ds.password = "p".into();
        assert_eq!(
            ds.connection_url().unwrap(),
            "mysql://u:p@db.example.com:3306/app"
        );
    }

    #[test]
    fn datasource_connection_url_sqlite_path() {
        let mut ds = DatasourceCfg::default();
        ds.driver = "sqlite".into();
        ds.database = "./data/dev.db".into();
        assert_eq!(ds.connection_url().unwrap(), "sqlite://./data/dev.db");
    }

    #[test]
    fn datasource_connection_url_passthrough() {
        let mut ds = DatasourceCfg::default();
        ds.driver = "mysql".into();
        ds.connection_string = "mysql://override/db".into();
        assert_eq!(ds.connection_url().unwrap(), "mysql://override/db");
    }

    #[test]
    fn datasource_connection_url_unknown_driver() {
        let mut ds = DatasourceCfg::default();
        ds.driver = "h2".into();
        assert!(ds.connection_url().is_none());
    }

    #[test]
    fn env_expansion_runs_after_parse() {
        std::env::set_var("RUSTCFML_TEST_HOST_VAL", "db.internal");
        let json = r#"{
            "datasources": {
                "x": {
                    "driver": "mysql",
                    "host": "${env.RUSTCFML_TEST_HOST_VAL}",
                    "database": "${env.RUSTCFML_MISSING_DB:fallback_db}"
                }
            }
        }"#;
        let mut cfg: RustCfmlConfig = serde_json::from_str(json).unwrap();
        cfg.expand_env();
        let ds = cfg.datasources.get("x").unwrap();
        assert_eq!(ds.host, "db.internal");
        assert_eq!(ds.database, "fallback_db");
        std::env::remove_var("RUSTCFML_TEST_HOST_VAL");
    }
}
