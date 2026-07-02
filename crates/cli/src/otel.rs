//! OpenTelemetry integration — Phase 3 of the observability plan
//! (`docs/observability-implementation-plan.md`).
//!
//! Distributed traces reproduce the request → method → query transaction tree as
//! standard OTel spans, exported over OTLP (HTTP/protobuf) on a background batch
//! thread so export never touches the request path. RED metrics
//! (request/error/duration + DB) are exposed on a native Prometheus scrape
//! endpoint using the standalone `prometheus` crate.
//!
//! The whole module is behind `#[cfg(all(feature = "obs-otel", not(wasm)))]` —
//! none of the heavy deps (opentelemetry, reqwest, prometheus) can reach the
//! wasm crates.
//!
//! ## How VM spans are built
//! The VM emits hook-bus events on its own (blocking) request thread. An
//! [`OtelObserver`] — installed per request via `vm.install_observer` — turns
//! `on_fn_enter`/`on_fn_exit`/`on_query` into child spans, maintaining an
//! explicit context stack rooted at the request's SERVER span. We use the raw
//! OTel API (not the `tracing` macros) because the VM's manual enter/exit across
//! bytecode dispatch doesn't fit `tracing`'s RAII-guard model.

#![cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use opentelemetry::global::BoxedTracer;
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::{
    SpanKind, Status, TraceContextExt, Tracer,
};
use opentelemetry::{global, Context, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use cfml_vm::observe::{
    ErrorEvent, FnEnter, FnExit, Interest, QueryEvent, TemplateEvent, VmObserver,
};

/// Resolved OTel settings, copied from `cfml_config::OtelCfg` at startup.
#[derive(Clone)]
pub struct OtelRuntime {
    pub depth_cap: usize,
    pub allow: Vec<String>,
    pub metrics: Option<std::sync::Arc<Metrics>>,
}

/// Initialise the global tracer provider + W3C propagator from config. Returns
/// the provider handle (keep it to flush/shutdown on exit) and an [`OtelRuntime`]
/// carrying the per-request knobs the observer needs. Returns `None` when the
/// OTLP exporter can't be built (bad endpoint) — the server still runs, just
/// without traces.
pub fn init(cfg: &cfml_config::OtelCfg) -> Option<(SdkTracerProvider, OtelRuntime)> {
    // Only the `http-proto` (protobuf) transport is compiled in; `http/json`
    // would need the `http-json` feature. Config may still name a protocol for
    // forward-compat, but we always export protobuf.
    let _ = &cfg.protocol;
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(cfg.endpoint.clone())
        .with_protocol(Protocol::HttpBinary)
        .with_timeout(Duration::from_millis(cfg.timeout_ms))
        .build()
        .map_err(|e| eprintln!("otel: failed to build OTLP exporter: {e}"))
        .ok()?;

    let sampler = Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(cfg.sample_ratio)));
    let resource = Resource::builder()
        .with_service_name(cfg.service_name.clone())
        .build();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(sampler)
        .with_resource(resource)
        .build();

    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());

    let metrics = if cfg.metrics.enabled {
        Some(std::sync::Arc::new(Metrics::new()))
    } else {
        None
    };
    let rt = OtelRuntime {
        depth_cap: cfg.span_depth_cap,
        allow: cfg.span_allow_list.clone(),
        metrics,
    };
    PROVIDER.set(provider.clone()).ok();
    OTEL_RT.set(rt.clone()).ok();
    Some((provider, rt))
}

// OTel is process-wide config, so — like the global tracer provider it wraps —
// the runtime + provider live in statics rather than being threaded through the
// request path (which cannot carry a CLI type on the `cfml-vm` `ServerState`).
static PROVIDER: std::sync::OnceLock<SdkTracerProvider> = std::sync::OnceLock::new();
static OTEL_RT: std::sync::OnceLock<OtelRuntime> = std::sync::OnceLock::new();

/// Flush + shut down the exporter. Call on graceful server shutdown so buffered
/// spans aren't lost.
pub fn shutdown() {
    if let Some(p) = PROVIDER.get() {
        let _ = p.force_flush();
        let _ = p.shutdown();
    }
}

/// Render the Prometheus metrics text exposition, or `None` when metrics are off.
pub fn render_metrics() -> Option<String> {
    OTEL_RT.get()?.metrics.as_ref().map(|m| m.render())
}

/// Open the request root span and install the per-request [`OtelObserver`] on the
/// VM. Reads request metadata from the VM's `cgi` scope (already populated at
/// this point) and continues any inbound W3C `traceparent`. Returns the root
/// [`Context`] to close with [`end_request`]. `None` when OTel is off.
pub fn begin_request(vm: &mut cfml_vm::CfmlVirtualMachine) -> Option<(Context, String)> {
    let rt = OTEL_RT.get()?;
    let cgi = |k: &str| vm.web_scope_value("cgi", k).unwrap_or_default();
    let method = {
        let m = cgi("request_method");
        if m.is_empty() {
            "GET".to_string()
        } else {
            m
        }
    };
    let route = cgi("script_name");
    let scheme = if cgi("https").eq_ignore_ascii_case("on") {
        "https"
    } else {
        "http"
    };
    let server_addr = cgi("server_name");
    let client_addr = cgi("remote_addr");
    let tp = cgi("http_traceparent");
    let ts = cgi("http_tracestate");
    let parent = parent_context(
        (!tp.is_empty()).then_some(tp.as_str()),
        (!ts.is_empty()).then_some(ts.as_str()),
    );
    let root = start_root_span(&method, &route, scheme, &route, &server_addr, &client_addr, &parent);
    let observer = std::sync::Arc::new(OtelObserver::new(root.clone(), rt));
    vm.install_observer(observer);
    Some((root, route))
}

/// Close the request root span and record RED metrics. `error_type` is `Some`
/// when the request failed (drives the error counter + span status).
pub fn end_request(
    root: &Context,
    route: &str,
    status: u16,
    elapsed_secs: f64,
    error_type: Option<&str>,
) {
    end_root_span(root, status);
    if let Some(rt) = OTEL_RT.get() {
        if let Some(m) = &rt.metrics {
            m.record_request(route, status, elapsed_secs, error_type);
        }
    }
}

/// The tracer used for all RustCFML spans.
fn tracer() -> BoxedTracer {
    global::tracer("rustcfml")
}

/// Build the parent [`Context`] from an inbound `traceparent`/`tracestate`
/// (W3C). Returns the root context when there is no inbound trace.
pub fn parent_context(traceparent: Option<&str>, tracestate: Option<&str>) -> Context {
    let mut carrier: HashMap<String, String> = HashMap::new();
    if let Some(tp) = traceparent {
        if !tp.is_empty() {
            carrier.insert("traceparent".to_string(), tp.to_string());
        }
    }
    if let Some(ts) = tracestate {
        if !ts.is_empty() {
            carrier.insert("tracestate".to_string(), ts.to_string());
        }
    }
    global::get_text_map_propagator(|prop| prop.extract(&MapExtractor(&carrier)))
}

struct MapExtractor<'a>(&'a HashMap<String, String>);
impl Extractor for MapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|s| s.as_str()).collect()
    }
}

/// Open the request's root SERVER span and return the [`Context`] holding it.
/// `route` should be the matched route template (low cardinality), not the raw
/// path. Sets the HTTP semantic-convention request attributes.
pub fn start_root_span(
    method: &str,
    route: &str,
    scheme: &str,
    path: &str,
    server_addr: &str,
    client_addr: &str,
    parent: &Context,
) -> Context {
    let tracer = tracer();
    let span = tracer
        .span_builder(format!("{method} {route}"))
        .with_kind(SpanKind::Server)
        .with_attributes(vec![
            KeyValue::new("http.request.method", method.to_string()),
            KeyValue::new("http.route", route.to_string()),
            KeyValue::new("url.path", path.to_string()),
            KeyValue::new("url.scheme", scheme.to_string()),
            KeyValue::new("server.address", server_addr.to_string()),
            KeyValue::new("client.address", client_addr.to_string()),
        ])
        .start_with_context(&tracer, parent);
    parent.with_span(span)
}

/// Close the root span: record the response status code and set span status.
pub fn end_root_span(root: &Context, status_code: u16) {
    let span = root.span();
    span.set_attribute(KeyValue::new(
        "http.response.status_code",
        status_code as i64,
    ));
    if status_code >= 500 {
        span.set_status(Status::error("server error"));
    } else {
        span.set_status(Status::Ok);
    }
    span.end();
}

// ── The VM observer ─────────────────────────────────────────────────────────

enum EntryKind {
    Function,
}

struct StackEntry {
    cx: Context,
    depth: usize,
    #[allow(dead_code)]
    kind: EntryKind,
}

/// Per-request OTel observer. Holds the request's root context and an explicit
/// child-span stack. Interior mutability via `Mutex` (uncontended — one request
/// runs on one thread; `Sync` is only needed to satisfy the trait bound).
pub struct OtelObserver {
    root: Context,
    depth_cap: usize,
    allow: Vec<String>,
    metrics: Option<std::sync::Arc<Metrics>>,
    stack: Mutex<Vec<StackEntry>>,
}

impl OtelObserver {
    pub fn new(root: Context, rt: &OtelRuntime) -> Self {
        Self {
            root,
            depth_cap: rt.depth_cap,
            allow: rt.allow.clone(),
            metrics: rt.metrics.clone(),
            stack: Mutex::new(Vec::new()),
        }
    }

    fn allowed(&self, name: &str) -> bool {
        self.allow.iter().any(|g| glob_match(g, name))
    }

    /// The parent context for a new child span: the top of the stack, or the
    /// request root when the stack is empty.
    fn parent_cx(&self, stack: &[StackEntry]) -> Context {
        stack
            .last()
            .map(|e| e.cx.clone())
            .unwrap_or_else(|| self.root.clone())
    }
}

impl VmObserver for OtelObserver {
    fn interest(&self) -> Interest {
        // Functions (span tree), queries (DB spans + metrics), errors (exception
        // events). Templates too — cheap, and they show the include tree.
        Interest::FUNCTION | Interest::QUERY | Interest::TEMPLATE | Interest::ERROR
    }

    fn on_fn_enter(&self, f: &FnEnter) {
        if f.depth > self.depth_cap || !self.allowed(f.name) {
            return;
        }
        let tracer = tracer();
        let mut stack = self.stack.lock().unwrap();
        let parent = self.parent_cx(&stack);
        let span = tracer
            .span_builder(f.name.to_string())
            .with_kind(SpanKind::Internal)
            .start_with_context(&tracer, &parent);
        let cx = parent.with_span(span);
        stack.push(StackEntry {
            cx,
            depth: f.depth,
            kind: EntryKind::Function,
        });
    }

    fn on_fn_exit(&self, f: &FnExit) {
        let mut stack = self.stack.lock().unwrap();
        // Pop only if the top corresponds to THIS call (same depth). If this
        // function wasn't spanned (over cap / not allow-listed) the top is a
        // shallower ancestor, so the depths won't match and we leave it be.
        let matches = stack
            .last()
            .map(|e| e.depth == f.depth)
            .unwrap_or(false);
        if matches {
            let entry = stack.pop().unwrap();
            if f.is_error {
                entry.cx.span().set_status(Status::error("unhandled exception"));
            }
            entry.cx.span().end();
        }
    }

    fn on_query(&self, q: &QueryEvent) {
        // DB metrics are always-on (record regardless of trace sampling).
        if let Some(m) = &self.metrics {
            m.record_query(q.datasource, q.elapsed_us);
        }
        let tracer = tracer();
        let stack = self.stack.lock().unwrap();
        let parent = self.parent_cx(&stack);
        // Backdate the span so its duration reflects the measured query time
        // (the query has already completed by the time the hook fires).
        let now = SystemTime::now();
        let start = now
            .checked_sub(Duration::from_micros(q.elapsed_us.max(0) as u64))
            .unwrap_or(now);
        let op = sql_operation(q.sql);
        let span = tracer
            .span_builder(format!("{op} {}", q.datasource))
            .with_kind(SpanKind::Client)
            .with_start_time(start)
            .with_attributes(vec![
                KeyValue::new("db.system.name", "sql"),
                KeyValue::new("db.query.text", truncate(q.sql, 2000)),
                KeyValue::new("db.operation.name", op.to_string()),
                KeyValue::new("db.namespace", q.datasource.to_string()),
                KeyValue::new("db.response.returned_rows", q.rowcount),
            ])
            .start_with_context(&tracer, &parent);
        parent.with_span(span).span().end_with_timestamp(now);
    }

    fn on_template(&self, t: &TemplateEvent) {
        let tracer = tracer();
        let stack = self.stack.lock().unwrap();
        let parent = self.parent_cx(&stack);
        let now = SystemTime::now();
        let start = now
            .checked_sub(Duration::from_micros(t.elapsed_us.max(0) as u64))
            .unwrap_or(now);
        let span = tracer
            .span_builder(format!("render {}", short_path(t.path)))
            .with_kind(SpanKind::Internal)
            .with_start_time(start)
            .with_attributes(vec![KeyValue::new("code.filepath", t.path.to_string())])
            .start_with_context(&tracer, &parent);
        parent.with_span(span).span().end_with_timestamp(now);
    }

    fn on_error(&self, e: &ErrorEvent) {
        // Record only genuinely uncaught errors on the trace (a try/catch-
        // recovered exception is normal control flow). Attach an exception event
        // to the current span and set Error status.
        if !e.uncaught {
            return;
        }
        let stack = self.stack.lock().unwrap();
        let cx = self.parent_cx(&stack);
        let span = cx.span();
        span.add_event(
            "exception",
            vec![
                KeyValue::new("exception.type", e.etype.to_string()),
                KeyValue::new("exception.message", e.message.to_string()),
            ],
        );
        span.set_status(Status::error(e.message.to_string()));
    }
}

/// Extract a coarse SQL operation verb for the span name / `db.operation.name`.
fn sql_operation(sql: &str) -> &'static str {
    let s = sql.trim_start();
    let word = s.split_whitespace().next().unwrap_or("");
    match word.to_ascii_uppercase().as_str() {
        "SELECT" => "SELECT",
        "INSERT" => "INSERT",
        "UPDATE" => "UPDATE",
        "DELETE" => "DELETE",
        "CREATE" => "CREATE",
        "DROP" => "DROP",
        "ALTER" => "ALTER",
        _ => "QUERY",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

fn short_path(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// A minimal case-insensitive glob: `*` matches any run of characters. Supports
/// leading/trailing/embedded `*`; everything else is a literal.
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let p = pattern.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == n;
    }
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !n[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if i == parts.len() - 1 {
            return n[pos..].ends_with(part);
        } else if let Some(idx) = n[pos..].find(part) {
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    true
}

// ── RED metrics (Prometheus) ──────────────────────────────────────────────

/// The RED metric instruments, held in a Prometheus registry and rendered on
/// demand at the scrape endpoint.
pub struct Metrics {
    pub registry: prometheus::Registry,
    requests: prometheus::IntCounterVec,
    errors: prometheus::IntCounterVec,
    duration: prometheus::HistogramVec,
    db_queries: prometheus::IntCounterVec,
    db_duration: prometheus::HistogramVec,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = prometheus::Registry::new();
        let requests = prometheus::IntCounterVec::new(
            prometheus::Opts::new("rustcfml_http_requests_total", "Total HTTP requests"),
            &["route", "status"],
        )
        .unwrap();
        let errors = prometheus::IntCounterVec::new(
            prometheus::Opts::new("rustcfml_http_errors_total", "HTTP requests that errored"),
            &["route", "error_type"],
        )
        .unwrap();
        let duration = prometheus::HistogramVec::new(
            prometheus::HistogramOpts::new(
                "rustcfml_http_request_duration_seconds",
                "HTTP request duration in seconds",
            ),
            &["route"],
        )
        .unwrap();
        let db_queries = prometheus::IntCounterVec::new(
            prometheus::Opts::new("rustcfml_db_queries_total", "Total DB queries"),
            &["datasource"],
        )
        .unwrap();
        let db_duration = prometheus::HistogramVec::new(
            prometheus::HistogramOpts::new(
                "rustcfml_db_query_duration_seconds",
                "DB query duration in seconds",
            ),
            &["datasource"],
        )
        .unwrap();
        registry.register(Box::new(requests.clone())).ok();
        registry.register(Box::new(errors.clone())).ok();
        registry.register(Box::new(duration.clone())).ok();
        registry.register(Box::new(db_queries.clone())).ok();
        registry.register(Box::new(db_duration.clone())).ok();
        Self {
            registry,
            requests,
            errors,
            duration,
            db_queries,
            db_duration,
        }
    }

    /// Record one completed HTTP request (RED: rate, errors, duration).
    pub fn record_request(&self, route: &str, status: u16, elapsed_secs: f64, error_type: Option<&str>) {
        self.requests
            .with_label_values(&[route, &status.to_string()])
            .inc();
        self.duration.with_label_values(&[route]).observe(elapsed_secs);
        if let Some(et) = error_type {
            self.errors.with_label_values(&[route, et]).inc();
        }
    }

    fn record_query(&self, datasource: &str, elapsed_us: i64) {
        self.db_queries.with_label_values(&[datasource]).inc();
        self.db_duration
            .with_label_values(&[datasource])
            .observe(elapsed_us.max(0) as f64 / 1_000_000.0);
    }

    /// Render the Prometheus text exposition for the scrape endpoint.
    pub fn render(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        if encoder.encode(&self.registry.gather(), &mut buf).is_err() {
            return String::new();
        }
        String::from_utf8(buf).unwrap_or_default()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("get*", "getUser"));
        assert!(glob_match("*Service", "UserService"));
        assert!(glob_match("*handle*", "preHandleRequest"));
        assert!(glob_match("exact", "EXACT"));
        assert!(!glob_match("get*", "setUser"));
        assert!(!glob_match("*Service", "ServiceHelper"));
    }

    #[test]
    fn sql_operation_verb() {
        assert_eq!(sql_operation("  SELECT * FROM t"), "SELECT");
        assert_eq!(sql_operation("insert into t"), "INSERT");
        assert_eq!(sql_operation("WITH x AS ()"), "QUERY");
    }

    #[test]
    fn metrics_render_contains_names() {
        let m = Metrics::new();
        m.record_request("/posts", 200, 0.012, None);
        m.record_query("main", 5000);
        let text = m.render();
        assert!(text.contains("rustcfml_http_requests_total"));
        assert!(text.contains("rustcfml_db_queries_total"));
    }

    // End-to-end span-tree test: drive the observer through a request's worth of
    // hook events and assert the exported OTel span tree shape, parenting,
    // db.* attributes, the depth cap, and the uncaught-exception event. Uses an
    // in-memory exporter + a global provider (this is the only test in the crate
    // that touches the global tracer, so there is no cross-test race).
    #[test]
    fn observer_builds_span_tree() {
        use opentelemetry::trace::TraceContextExt;
        use opentelemetry_sdk::trace::InMemorySpanExporter;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider.clone());

        let rt = OtelRuntime {
            depth_cap: 3,
            allow: vec!["*".to_string()],
            metrics: None,
        };

        // Open the request root and drive a call chain: outer → inner, a query
        // issued while inner is on the stack, plus a too-deep call that must NOT
        // be spanned (depth 5 > cap 3).
        let root = start_root_span(
            "GET", "/posts", "http", "/posts", "localhost", "127.0.0.1",
            &parent_context(None, None),
        );
        let obs = OtelObserver::new(root.clone(), &rt);
        obs.on_fn_enter(&FnEnter { name: "outer", called_name: "outer", depth: 0 });
        obs.on_fn_enter(&FnEnter { name: "inner", called_name: "inner", depth: 1 });
        obs.on_query(&QueryEvent {
            name: "q", sql: "SELECT * FROM posts", datasource: "main",
            rowcount: 3, elapsed_us: 4200, cached: false, src: "/posts", line: 1, params: &[],
        });
        obs.on_fn_enter(&FnEnter { name: "tooDeep", called_name: "tooDeep", depth: 5 });
        obs.on_fn_exit(&FnExit { name: "tooDeep", depth: 5, is_error: false });
        // An uncaught error occurs inside inner, which then unwinds (is_error).
        obs.on_error(&ErrorEvent {
            etype: "MyError", message: "boom", detail: "", src: "/posts", line: 2,
            uncaught: true, stack: vec![],
        });
        obs.on_fn_exit(&FnExit { name: "inner", depth: 1, is_error: true });
        obs.on_fn_exit(&FnExit { name: "outer", depth: 0, is_error: false });
        end_root_span(&root, 200);

        provider.force_flush().ok();
        let spans = exporter.get_finished_spans().unwrap();
        let by_name = |n: &str| spans.iter().find(|s| s.name == n);

        // The too-deep call produced no span.
        assert!(by_name("tooDeep").is_none(), "depth cap should suppress tooDeep");

        let root_s = by_name("GET /posts").expect("root span");
        let outer = by_name("outer").expect("outer span");
        let inner = by_name("inner").expect("inner span");
        let query = by_name("SELECT main").expect("query span");

        // Parenting: root ← outer ← inner ← query.
        assert_eq!(outer.parent_span_id, root_s.span_context.span_id());
        assert_eq!(inner.parent_span_id, outer.span_context.span_id());
        assert_eq!(query.parent_span_id, inner.span_context.span_id());

        // db.* semconv attributes on the query span.
        let has_attr = |s: &opentelemetry_sdk::trace::SpanData, k: &str, v: &str| {
            s.attributes.iter().any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
        };
        assert!(has_attr(query, "db.namespace", "main"));
        assert!(has_attr(query, "db.operation.name", "SELECT"));

        // Root carries the HTTP status.
        assert!(root_s.attributes.iter().any(|kv| kv.key.as_str() == "http.response.status_code"));

        // The uncaught error attached an exception event + Error status to the
        // span active at throw time (inner), and inner unwound with is_error.
        assert!(
            inner.events.iter().any(|e| e.name == "exception"),
            "inner span should carry an exception event"
        );
        assert!(matches!(inner.status, Status::Error { .. }), "inner status = Error");
    }
}
