//! The classic CF debug output (footer/panel) — Phase 1 of the
//! observability/debugging plan.
//!
//! A [`DebugCollector`] subscribes to the [`crate::observe`] hook bus and
//! accumulates a per-request [`DebugData`] (queries, page timings, exceptions,
//! app-injected generic data). At request end the VM renders it through one of
//! the built-in templates (`modern`/`classic`/`simple`/`comment`/`none`),
//! mirroring Lucee's data model so the experience is native to CFML developers.
//!
//! The whole module is behind the `observability` feature; nothing here
//! compiles for the wasm crates.

#![cfg(feature = "observability")]

use crate::observe::{
    ErrorEvent, Interest, LogEvent, QueryEvent, QueryParam, TemplateEvent, VmObserver,
};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use std::sync::Mutex;
use std::time::Instant;

/// A row in the `queries` section. Columns follow Lucee 6/7. `time` is in
/// **microseconds** (Lucee's unit).
#[derive(Clone, Default)]
pub struct QueryRow {
    pub name: String,
    pub sql: String,
    pub datasource: String,
    pub count: i64,
    pub time: i64,
    pub cached: bool,
    pub src: String,
    pub line: usize,
    /// Bound parameters (name, value, cfsqltype), in supply order.
    pub params: Vec<QueryParam>,
}

/// A raw template-execution hit; aggregated into `pages` at render time.
/// `time` is in **microseconds**.
#[derive(Clone, Default)]
pub struct TemplateHit {
    pub path: String,
    pub time: i64,
}

/// A row in the `exceptions` section.
#[derive(Clone, Default)]
pub struct ExceptionRow {
    pub etype: String,
    pub message: String,
    pub detail: String,
    pub src: String,
    pub line: usize,
    /// `(template, line)` frames, outermost first.
    pub stack: Vec<(String, usize)>,
}

/// A row in the `genericData` section, injected by app code via `debugAdd()`.
#[derive(Clone, Default)]
pub struct GenericRow {
    pub category: String,
    pub name: String,
    pub value: String,
}

/// A row in the `traces` section (`trace()` / `<cflog>`).
#[derive(Clone, Default)]
pub struct TraceRow {
    pub category: String,
    pub text: String,
    pub log_type: String,
}

/// The accumulated per-request debug data. Mirrors Lucee 6/7's `DebugData`
/// shape (the sections we feed in stage 1 are populated; the rest are present
/// in the schema and rendered empty until their feed lands).
#[derive(Default)]
pub struct DebugData {
    pub queries: Vec<QueryRow>,
    pub templates: Vec<TemplateHit>,
    pub exceptions: Vec<ExceptionRow>,
    pub generic: Vec<GenericRow>,
    pub traces: Vec<TraceRow>,
    /// Per-section overflow counts when `maxRecords` clips a section.
    pub dropped_queries: usize,
}

/// Config snapshot the collector/renderer need (copied from `DebuggingCfg` so
/// the collector is self-contained and lock-free on the config).
#[derive(Clone)]
pub struct FooterCfg {
    pub template: String,
    pub highlight_ms: i64,
    pub max_records: usize,
    pub database: bool,
    pub exception: bool,
    pub tracing: bool,
}

impl Default for FooterCfg {
    fn default() -> Self {
        Self {
            template: "modern".into(),
            highlight_ms: 250,
            max_records: 10,
            database: true,
            exception: true,
            tracing: true,
        }
    }
}

/// The hook-bus subscriber. Interior-mutable so it can live behind the VM's
/// `Arc<dyn VmObserver>` while still accumulating.
pub struct DebugCollector {
    inner: Mutex<DebugData>,
    cfg: FooterCfg,
    started: Instant,
}

impl DebugCollector {
    pub fn new(cfg: FooterCfg) -> Self {
        Self {
            inner: Mutex::new(DebugData::default()),
            cfg,
            started: Instant::now(),
        }
    }

    /// Total request wall-clock so far, in microseconds (Lucee's unit).
    pub fn total_us(&self) -> i64 {
        self.started.elapsed().as_micros() as i64
    }

    pub fn cfg(&self) -> &FooterCfg {
        &self.cfg
    }

    /// Append a `genericData` row (the `debugAdd()` BIF channel).
    pub fn add_generic(&self, category: &str, name: &str, value: &str) {
        if let Ok(mut d) = self.inner.lock() {
            d.generic.push(GenericRow {
                category: category.to_string(),
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }

    /// Render the footer for the configured template, given the live scope
    /// snapshots gathered by the VM and the total request time. `main_page` is
    /// the base template being served — recorded as a `pages` row with the
    /// total request time, so the main page shows alongside its includes
    /// (Lucee lists every executed template, not just `<cfinclude>`s).
    /// `cfconfig` is the effective-vs-default settings diff (see
    /// [`cfconfig_diff_rows`]) rendered as its own table.
    pub fn render(
        &self,
        scopes: &[(String, ValueMap)],
        main_page: Option<&str>,
        cfconfig: &[(String, String)],
    ) -> String {
        if let Ok(d) = self.inner.lock() {
            render_footer(&self.cfg, &d, scopes, self.total_us(), main_page, cfconfig)
        } else {
            String::new()
        }
    }

    /// Build the `getDebugData()` struct.
    pub fn to_cfml(&self, scopes: &[(String, ValueMap)], main_page: Option<&str>) -> CfmlValue {
        if let Ok(d) = self.inner.lock() {
            to_cfml_struct(&d, scopes, self.total_us(), main_page)
        } else {
            CfmlValue::strukt(ValueMap::default())
        }
    }
}

impl VmObserver for DebugCollector {
    fn interest(&self) -> Interest {
        let mut i = Interest::REQUEST;
        if self.cfg.database {
            i |= Interest::QUERY;
        }
        i |= Interest::TEMPLATE;
        if self.cfg.exception {
            i |= Interest::ERROR;
        }
        if self.cfg.tracing {
            i |= Interest::LOG;
        }
        i
    }

    fn on_query(&self, q: &QueryEvent) {
        if let Ok(mut d) = self.inner.lock() {
            if d.queries.len() >= self.cfg.max_records {
                d.dropped_queries += 1;
                return;
            }
            d.queries.push(QueryRow {
                name: q.name.to_string(),
                sql: q.sql.to_string(),
                datasource: q.datasource.to_string(),
                count: q.rowcount,
                time: q.elapsed_us,
                cached: q.cached,
                src: q.src.to_string(),
                line: q.line,
                params: q.params.to_vec(),
            });
        }
    }

    fn on_template(&self, t: &TemplateEvent) {
        if let Ok(mut d) = self.inner.lock() {
            d.templates.push(TemplateHit {
                path: t.path.to_string(),
                time: t.elapsed_us,
            });
        }
    }

    fn on_error(&self, e: &ErrorEvent) {
        if let Ok(mut d) = self.inner.lock() {
            d.exceptions.push(ExceptionRow {
                etype: e.etype.to_string(),
                message: e.message.to_string(),
                detail: e.detail.to_string(),
                src: e.src.to_string(),
                line: e.line,
                stack: e.stack.clone(),
            });
        }
    }

    fn on_log(&self, l: &LogEvent) {
        if let Ok(mut d) = self.inner.lock() {
            d.traces.push(TraceRow {
                category: l.file.to_string(),
                text: l.text.to_string(),
                log_type: l.log_type.to_string(),
            });
        }
    }
}

// ── Aggregation ─────────────────────────────────────────────────────────────

/// One aggregated `pages` row.
struct PageAgg {
    id: String,
    count: i64,
    min: i64,
    max: i64,
    total: i64,
}

/// Aggregate template hits into `pages` rows, optionally leading with the main
/// page (recorded with the total request time). The main page is listed first,
/// then each included template in encounter order — matching Lucee's habit of
/// showing the requested page plus every `<cfinclude>`/render below it.
fn aggregate_pages_with_main(
    templates: &[TemplateHit],
    main_page: Option<&str>,
    total_us: i64,
) -> Vec<PageAgg> {
    let mut hits: Vec<TemplateHit> = Vec::new();
    if let Some(p) = main_page {
        hits.push(TemplateHit {
            path: p.to_string(),
            time: total_us,
        });
    }
    hits.extend_from_slice(templates);
    aggregate_pages(&hits)
}

fn aggregate_pages(templates: &[TemplateHit]) -> Vec<PageAgg> {
    let mut out: Vec<PageAgg> = Vec::new();
    for t in templates {
        if let Some(p) = out.iter_mut().find(|p| p.id == t.path) {
            p.count += 1;
            p.total += t.time;
            p.min = p.min.min(t.time);
            p.max = p.max.max(t.time);
        } else {
            out.push(PageAgg {
                id: t.path.clone(),
                count: 1,
                min: t.time,
                max: t.time,
                total: t.time,
            });
        }
    }
    out
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Format a microsecond duration as a millisecond string (Lucee shows ms,
/// stores µs). 3 decimals so sub-millisecond work is still visible.
fn fmt_us(us: i64) -> String {
    format!("{:.3}", us as f64 / 1000.0)
}

/// Render one query's bound parameters as `name=value` (with `:type` appended
/// when a cfsqltype is known).
fn fmt_params_html(params: &[QueryParam]) -> String {
    params
        .iter()
        .map(|p| {
            if p.sqltype.is_empty() {
                format!("<code>{}={}</code>", esc(&p.name), esc(&p.value))
            } else {
                format!(
                    "<code>{}={}</code> <span style=\"color:#999\">({})</span>",
                    esc(&p.name),
                    esc(&p.value),
                    esc(&p.sqltype)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Truncate a scope value's string form so a giant struct can't balloon the page.
fn short_val(v: &CfmlValue) -> String {
    let s = v.as_string();
    if s.len() > 200 {
        format!("{}…", &s[..200])
    } else {
        s
    }
}

/// Top-level renderer — dispatches on the configured template name.
pub fn render_footer(
    cfg: &FooterCfg,
    data: &DebugData,
    scopes: &[(String, ValueMap)],
    total_us: i64,
    main_page: Option<&str>,
    cfconfig: &[(String, String)],
) -> String {
    match cfg.template.to_ascii_lowercase().as_str() {
        "none" => String::new(),
        "comment" => render_comment(data, total_us),
        "simple" => render_html(cfg, data, scopes, total_us, main_page, cfconfig, false),
        "classic" => render_html(cfg, data, scopes, total_us, main_page, cfconfig, false),
        // "modern" (default) and any unknown template fall back to the rich panel.
        _ => render_html(cfg, data, scopes, total_us, main_page, cfconfig, true),
    }
}

/// Flatten the difference between the EFFECTIVE cfconfig (server baseline +
/// app overlay, post `${VAR:default}` expansion) and the engine's built-in defaults
/// into sorted `(dotted.path, value)` rows — i.e. exactly what this deploy's
/// `.cfconfig.json` picked up. Values whose key smells like a credential
/// (`password`/`secret`/`token`) are redacted.
pub fn cfconfig_diff_rows(
    effective: &serde_json::Value,
    defaults: &serde_json::Value,
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    cfconfig_diff_walk("", effective, defaults, &mut rows);
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn cfconfig_diff_walk(
    path: &str,
    eff: &serde_json::Value,
    def: &serde_json::Value,
    out: &mut Vec<(String, String)>,
) {
    use serde_json::Value;
    match (eff, def) {
        (Value::Object(em), _) => {
            let empty = serde_json::Map::new();
            let dm = match def {
                Value::Object(m) => m,
                _ => &empty,
            };
            for (k, ev) in em {
                let sub = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                cfconfig_diff_walk(&sub, ev, dm.get(k).unwrap_or(&Value::Null), out);
            }
        }
        _ => {
            if eff != def && !eff.is_null() {
                let leaf = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
                let shown = if leaf.contains("password")
                    || leaf.contains("secret")
                    || leaf.contains("token")
                {
                    "••••••".to_string()
                } else {
                    match eff {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }
                };
                out.push((path.to_string(), shown));
            }
        }
    }
}

fn render_comment(data: &DebugData, total_us: i64) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "\n<!-- RustCFML v{} Debug\n",
        env!("CARGO_PKG_VERSION")
    ));
    s.push_str(&format!("  Total time: {} ms\n", fmt_us(total_us)));
    s.push_str(&format!("  Queries: {}\n", data.queries.len()));
    for q in &data.queries {
        s.push_str(&format!(
            "    [{} ms] {} ({} rows) — {}\n",
            fmt_us(q.time),
            q.sql.replace('\n', " "),
            q.count,
            q.datasource
        ));
        if !q.params.is_empty() {
            let parts: Vec<String> = q
                .params
                .iter()
                .map(|p| {
                    if p.sqltype.is_empty() {
                        format!("{}={}", p.name, p.value)
                    } else {
                        format!("{}={} ({})", p.name, p.value, p.sqltype)
                    }
                })
                .collect();
            s.push_str(&format!("      params: {}\n", parts.join(", ")));
        }
    }
    if !data.exceptions.is_empty() {
        s.push_str(&format!("  Exceptions: {}\n", data.exceptions.len()));
        for e in &data.exceptions {
            s.push_str(&format!("    {}: {}\n", e.etype, e.message.replace('\n', " ")));
        }
    }
    s.push_str("-->\n");
    s
}

fn render_html(
    cfg: &FooterCfg,
    data: &DebugData,
    scopes: &[(String, ValueMap)],
    total_us: i64,
    main_page: Option<&str>,
    cfconfig: &[(String, String)],
    modern: bool,
) -> String {
    let mut s = String::new();
    let style = if modern {
        "font-family:monospace;font-size:12px;background:#f5f5f5;color:#222;border-top:3px solid #c33;margin-top:20px;padding:8px 12px"
    } else {
        "font-family:monospace;font-size:12px"
    };
    s.push_str(&format!(
        "\n<div class=\"rustcfml-debug\" style=\"{}\">\n",
        style
    ));
    s.push_str(&format!(
        "<h3 style=\"margin:4px 0\">RustCFML v{} Debug &mdash; total {} ms</h3>\n",
        env!("CARGO_PKG_VERSION"),
        fmt_us(total_us)
    ));

    // Queries
    if cfg.database {
        s.push_str(&format!(
            "<h4 style=\"margin:6px 0 2px\">Queries ({})</h4>\n",
            data.queries.len()
        ));
        if data.queries.is_empty() {
            s.push_str("<div>(none)</div>\n");
        } else {
            s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
            s.push_str("<tr><th>name</th><th>ms</th><th>rows</th><th>datasource</th><th>src</th><th>sql &amp; params</th></tr>\n");
            for q in &data.queries {
                // highlight_ms is in ms; query time is in µs.
                let slow = q.time >= cfg.highlight_ms * 1000;
                let row_style = if slow {
                    " style=\"background:#fdd\""
                } else {
                    ""
                };
                // SQL, then the bound parameters underneath (Lucee shows the
                // params used — name, value and cfsqltype — so you can see
                // exactly what was sent).
                let mut sql_cell = format!(
                    "<pre style=\"margin:0;white-space:pre-wrap\">{}</pre>",
                    esc(&q.sql)
                );
                if !q.params.is_empty() {
                    sql_cell.push_str("<div style=\"color:#555;margin-top:2px\">params: ");
                    sql_cell.push_str(&fmt_params_html(&q.params));
                    sql_cell.push_str("</div>");
                }
                s.push_str(&format!(
                    "<tr{}><td>{}</td><td class=\"txt-r\">{}</td><td>{}</td><td>{}</td><td>{}:{}</td><td>{}</td></tr>\n",
                    row_style,
                    esc(&q.name),
                    fmt_us(q.time),
                    q.count,
                    esc(&q.datasource),
                    esc(&q.src),
                    q.line,
                    sql_cell,
                ));
            }
            s.push_str("</table>\n");
            if data.dropped_queries > 0 {
                s.push_str(&format!(
                    "<div>(+{} more queries clipped by maxRecords)</div>\n",
                    data.dropped_queries
                ));
            }
        }
    }

    // Execution Time summary (Lucee's breakdown): Total, time spent in Query,
    // and Application (= total − query). Load/compilation is not yet tracked
    // separately, so it folds into Application.
    let query_total_us: i64 = data.queries.iter().map(|q| q.time).sum();
    if cfg.database || !data.templates.is_empty() {
        s.push_str("<h4 style=\"margin:6px 0 2px\">Execution Time</h4>\n");
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        s.push_str(&format!(
            "<tr><td class=\"txt-r\">{} ms</td><td>Total</td></tr>\n",
            fmt_us(total_us)
        ));
        s.push_str(&format!(
            "<tr><td class=\"txt-r\">{} ms</td><td>Application</td></tr>\n",
            fmt_us((total_us - query_total_us).max(0))
        ));
        s.push_str(&format!(
            "<tr><td class=\"txt-r\">{} ms</td><td>Query</td></tr>\n",
            fmt_us(query_total_us)
        ));
        s.push_str("</table>\n");
    }

    // Pages (templates) — main page first, then each include / component
    // method / Application.cfc execution, aggregated per file (Lucee parity).
    let pages = aggregate_pages_with_main(&data.templates, main_page, total_us);
    if !pages.is_empty() {
        s.push_str(&format!(
            "<h4 style=\"margin:6px 0 2px\">Templates ({} executed)</h4>\n",
            pages.iter().map(|p| p.count).sum::<i64>()
        ));
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        s.push_str("<tr><th>total ms</th><th>app ms</th><th>query ms</th><th>count</th><th>avg ms</th><th>template</th></tr>\n");
        for p in &pages {
            let avg = if p.count > 0 { p.total / p.count } else { 0 };
            // Per-template query time: sum of queries issued from this file
            // (Lucee's per-page Query column). `app` = total − query.
            let q_us: i64 = data
                .queries
                .iter()
                .filter(|q| q.src == p.id)
                .map(|q| q.time)
                .sum();
            let app_us = (p.total - q_us).max(0);
            s.push_str(&format!(
                "<tr><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td>{}</td></tr>\n",
                fmt_us(p.total),
                fmt_us(app_us),
                fmt_us(q_us),
                p.count,
                fmt_us(avg),
                esc(&p.id),
            ));
        }
        s.push_str("</table>\n");
    }

    // Exceptions
    if cfg.exception && !data.exceptions.is_empty() {
        s.push_str(&format!(
            "<h4 style=\"margin:6px 0 2px\">Exceptions ({})</h4>\n",
            data.exceptions.len()
        ));
        for e in &data.exceptions {
            s.push_str(&format!(
                "<div style=\"color:#900\"><b>{}</b>: {} <small>({}:{})</small></div>\n",
                esc(&e.etype),
                esc(&e.message),
                esc(&e.src),
                e.line,
            ));
        }
    }

    // Traces / log
    if cfg.tracing && !data.traces.is_empty() {
        s.push_str(&format!(
            "<h4 style=\"margin:6px 0 2px\">Trace / Log ({})</h4>\n",
            data.traces.len()
        ));
        for t in &data.traces {
            s.push_str(&format!(
                "<div>[{}] {} {}</div>\n",
                esc(&t.log_type),
                esc(&t.category),
                esc(&t.text),
            ));
        }
    }

    // Generic data (debugAdd)
    if !data.generic.is_empty() {
        s.push_str("<h4 style=\"margin:6px 0 2px\">Generic data</h4>\n");
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        s.push_str("<tr><th>category</th><th>name</th><th>value</th></tr>\n");
        for g in &data.generic {
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                esc(&g.category),
                esc(&g.name),
                esc(&g.value),
            ));
        }
        s.push_str("</table>\n");
    }

    // Scopes, in the canonical order set by `gather_debug_scopes`: url, form,
    // cgi. The deploy-level blocks (cfconfig overrides, then the CLI runtime
    // flags) render directly under the cgi scope — the natural place to look
    // for "what was this engine started with" while reading request context.
    // Process env vars are deliberately NOT rendered: they routinely hold
    // credentials (DSN passwords, API keys) and the footer can be exposed to
    // anyone holding the URL debug key. Order: CFConfig, Runtime flags.
    let mut deploy_blocks_rendered = false;
    for (name, map) in scopes {
        if map.is_empty() {
            continue;
        }
        s.push_str(&format!(
            "<h4 style=\"margin:6px 0 2px\">{} scope</h4>\n",
            esc(name)
        ));
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        for (k, v) in map.iter() {
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>\n",
                esc(k),
                esc(&short_val(v))
            ));
        }
        s.push_str("</table>\n");
        if name.eq_ignore_ascii_case("cgi") {
            render_cfconfig(&mut s, cfconfig);
            render_runtime_flags(&mut s);
            deploy_blocks_rendered = true;
        }
    }
    if !deploy_blocks_rendered {
        render_cfconfig(&mut s, cfconfig);
        render_runtime_flags(&mut s);
    }

    s.push_str("</div>\n");
    s
}

/// Render the runtime flags (CLI arguments) the engine was started with.
/// Process environment variables are intentionally never rendered — they
/// routinely carry secrets (DSN passwords, API keys, signing keys), and the
/// debug footer's only gate may be a URL key.
fn render_runtime_flags(s: &mut String) {
    let flags: Vec<String> = std::env::args().skip(1).collect();
    s.push_str(&format!(
        "<h4 style=\"margin:6px 0 2px\">Runtime flags ({})</h4>\n",
        flags.len()
    ));
    if flags.is_empty() {
        s.push_str("<div>(none)</div>\n");
    } else {
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        for f in &flags {
            s.push_str(&format!("<tr><td>{}</td></tr>\n", esc(f)));
        }
        s.push_str("</table>\n");
    }
}

/// Render the settings this deploy's cfconfig changed from engine defaults
/// (already flattened + credential-redacted by [`cfconfig_diff_rows`]).
fn render_cfconfig(s: &mut String, rows: &[(String, String)]) {
    s.push_str(&format!(
        "<h4 style=\"margin:6px 0 2px\">CFConfig ({})</h4>\n",
        rows.len()
    ));
    if rows.is_empty() {
        s.push_str("<div>(engine defaults — no cfconfig overrides)</div>\n");
    } else {
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        for (k, v) in rows {
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>\n",
                esc(k),
                esc(v)
            ));
        }
        s.push_str("</table>\n");
    }
}

// ── CFML struct projection (getDebugData()) ──────────────────────────────────

fn to_cfml_struct(
    data: &DebugData,
    scopes: &[(String, ValueMap)],
    total_us: i64,
    main_page: Option<&str>,
) -> CfmlValue {
    let mut root = ValueMap::default();
    root.insert("starttime".into(), CfmlValue::Int(0));
    // Times are microseconds (Lucee's unit).
    root.insert("total".into(), CfmlValue::Int(total_us));

    // queries
    let queries: Vec<CfmlValue> = data
        .queries
        .iter()
        .map(|q| {
            let mut m = ValueMap::default();
            m.insert("name".into(), CfmlValue::string(q.name.clone()));
            m.insert("sql".into(), CfmlValue::string(q.sql.clone()));
            m.insert("datasource".into(), CfmlValue::string(q.datasource.clone()));
            m.insert("count".into(), CfmlValue::Int(q.count));
            m.insert("time".into(), CfmlValue::Int(q.time));
            m.insert("cached".into(), CfmlValue::Bool(q.cached));
            m.insert("src".into(), CfmlValue::string(q.src.clone()));
            m.insert("line".into(), CfmlValue::Int(q.line as i64));
            let params: Vec<CfmlValue> = q
                .params
                .iter()
                .map(|p| {
                    let mut pm = ValueMap::default();
                    pm.insert("name".into(), CfmlValue::string(p.name.clone()));
                    pm.insert("value".into(), CfmlValue::string(p.value.clone()));
                    pm.insert("type".into(), CfmlValue::string(p.sqltype.clone()));
                    CfmlValue::strukt(pm)
                })
                .collect();
            m.insert("params".into(), CfmlValue::array(params));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("queries".into(), CfmlValue::array(queries));

    // pages
    let pages: Vec<CfmlValue> = aggregate_pages_with_main(&data.templates, main_page, total_us)
        .into_iter()
        .map(|p| {
            let mut m = ValueMap::default();
            m.insert("id".into(), CfmlValue::string(p.id));
            m.insert("count".into(), CfmlValue::Int(p.count));
            m.insert("min".into(), CfmlValue::Int(p.min));
            m.insert("max".into(), CfmlValue::Int(p.max));
            m.insert("total".into(), CfmlValue::Int(p.total));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("pages".into(), CfmlValue::array(pages));

    // exceptions
    let exceptions: Vec<CfmlValue> = data
        .exceptions
        .iter()
        .map(|e| {
            let mut m = ValueMap::default();
            m.insert("type".into(), CfmlValue::string(e.etype.clone()));
            m.insert("message".into(), CfmlValue::string(e.message.clone()));
            m.insert("detail".into(), CfmlValue::string(e.detail.clone()));
            m.insert("line".into(), CfmlValue::Int(e.line as i64));
            let ctx: Vec<CfmlValue> = e
                .stack
                .iter()
                .map(|(tmpl, line)| {
                    let mut cm = ValueMap::default();
                    cm.insert("template".into(), CfmlValue::string(tmpl.clone()));
                    cm.insert("line".into(), CfmlValue::Int(*line as i64));
                    CfmlValue::strukt(cm)
                })
                .collect();
            m.insert("tagContext".into(), CfmlValue::array(ctx));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("exceptions".into(), CfmlValue::array(exceptions));

    // genericData
    let generic: Vec<CfmlValue> = data
        .generic
        .iter()
        .map(|g| {
            let mut m = ValueMap::default();
            m.insert("category".into(), CfmlValue::string(g.category.clone()));
            m.insert("name".into(), CfmlValue::string(g.name.clone()));
            m.insert("value".into(), CfmlValue::string(g.value.clone()));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("genericData".into(), CfmlValue::array(generic));

    // traces
    let traces: Vec<CfmlValue> = data
        .traces
        .iter()
        .map(|t| {
            let mut m = ValueMap::default();
            m.insert("category".into(), CfmlValue::string(t.category.clone()));
            m.insert("text".into(), CfmlValue::string(t.text.clone()));
            m.insert("type".into(), CfmlValue::string(t.log_type.clone()));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("traces".into(), CfmlValue::array(traces));

    // scopes
    let mut scope_struct = ValueMap::default();
    for (name, map) in scopes {
        scope_struct.insert(name.clone(), CfmlValue::strukt(map.clone()));
    }
    root.insert("scopes".into(), CfmlValue::strukt(scope_struct));

    CfmlValue::strukt(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{Interest, QueryParam};

    fn p(name: &str, value: &str, sqltype: &str) -> QueryParam {
        QueryParam {
            name: name.into(),
            value: value.into(),
            sqltype: sqltype.into(),
        }
    }

    fn sample_collector() -> DebugCollector {
        let c = DebugCollector::new(FooterCfg::default());
        c.on_query(&QueryEvent {
            name: "getUsers",
            sql: "SELECT * FROM users",
            datasource: "main",
            rowcount: 3,
            elapsed_us: 5_000,
            cached: false,
            src: "/index.cfm",
            line: 12,
            params: &[p("id", "7", "cf_sql_integer"), p("active", "true", "")],
        });
        c.on_query(&QueryEvent {
            name: "slow",
            sql: "SELECT pg_sleep(1)",
            datasource: "main",
            rowcount: 1,
            // 999 ms in µs — over the 250 ms highlight threshold.
            elapsed_us: 999_000,
            cached: false,
            src: "/index.cfm",
            line: 20,
            params: &[],
        });
        c.on_template(&TemplateEvent {
            path: "/header.cfm",
            elapsed_us: 2_000,
        });
        c.on_error(&ErrorEvent {
            etype: "Custom.Boom",
            message: "kaboom",
            detail: "",
            src: "/index.cfm",
            line: 30,
            uncaught: true,
            stack: vec![("/index.cfm".into(), 30)],
        });
        c.add_generic("Wheels", "controller", "users");
        c
    }

    #[test]
    fn interest_contains_and_union() {
        let i = Interest::QUERY | Interest::TEMPLATE;
        assert!(i.contains(Interest::QUERY));
        assert!(i.contains(Interest::TEMPLATE));
        assert!(!i.contains(Interest::ERROR));
        // contains(NONE) is false by construction (avoids "everyone is interested").
        assert!(!i.contains(Interest::NONE));
        assert!(Interest::NONE.is_empty());
    }

    #[test]
    fn collector_interest_reflects_toggles() {
        let mut cfg = FooterCfg::default();
        cfg.database = false;
        let c = DebugCollector::new(cfg);
        assert!(!c.interest().contains(Interest::QUERY));
        assert!(c.interest().contains(Interest::TEMPLATE));
    }

    #[test]
    fn modern_render_has_sections() {
        let c = sample_collector();
        let html = c.render(&[], Some("/index.cfm"), &[("runtime.reportAsLucee".to_string(), "false".to_string())]);
        assert!(html.contains(&format!("RustCFML v{} Debug", env!("CARGO_PKG_VERSION"))));
        // deploy blocks render even without a cgi scope in the snapshot;
        // process env vars must never render (they can carry credentials)
        assert!(!html.contains("Environment variables"));
        assert!(html.contains("Runtime flags ("));
        assert!(html.contains("Queries (2)"));
        assert!(html.contains("SELECT * FROM users"));
        // bound parameters are shown under the SQL (Lucee parity)
        assert!(html.contains("params:"));
        assert!(html.contains("<code>id=7</code>"));
        assert!(html.contains("<code>active=true</code>"));
        // slow query (>= highlightMs 250) is red-highlighted
        assert!(html.contains("background:#fdd"));
        assert!(html.contains("Templates"));
        // the main page is listed alongside the include
        assert!(html.contains("/index.cfm"));
        assert!(html.contains("/header.cfm"));
        assert!(html.contains("Exceptions (1)"));
        assert!(html.contains("kaboom"));
        assert!(html.contains("Generic data"));
        assert!(html.contains("controller"));
    }

    #[test]
    fn scope_and_deploy_blocks_render_in_canonical_order() {
        let scope = |k: &str| {
            let mut m = ValueMap::default();
            m.insert(k.to_string(), CfmlValue::string("v".to_string()));
            m
        };
        // Passed cgi-first on purpose: the renderer must not depend on the
        // caller's ordering for the deploy blocks anchored to cgi.
        let scopes = vec![
            ("url".to_string(), scope("a")),
            ("form".to_string(), scope("b")),
            ("cgi".to_string(), scope("c")),
        ];
        let c = sample_collector();
        let html = c.render(
            &scopes,
            Some("/index.cfm"),
            &[("runtime.reportAsLucee".to_string(), "false".to_string())],
        );
        let at = |needle: &str| html.find(needle).unwrap_or_else(|| panic!("missing {needle}"));
        let order = [
            at("url scope"),
            at("form scope"),
            at("cgi scope"),
            at("CFConfig ("),
            at("Runtime flags ("),
        ];
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "expected URL, FORM, CGI, CFConfig, Runtime flags — got offsets {order:?}"
        );
        assert!(!html.contains("Environment variables"));
    }

    #[test]
    fn cfconfig_diff_only_changes_and_redacts_credentials() {
        let defaults = serde_json::json!({
            "runtime": { "reportAsLucee": true },
            "datasources": {}
        });
        let effective = serde_json::json!({
            "runtime": { "reportAsLucee": false },
            "datasources": { "main": { "host": "localhost", "password": "hunter2" } }
        });
        let rows = cfconfig_diff_rows(&effective, &defaults);
        assert!(rows.contains(&("runtime.reportAsLucee".to_string(), "false".to_string())));
        assert!(rows.contains(&("datasources.main.host".to_string(), "localhost".to_string())));
        assert!(rows.contains(&("datasources.main.password".to_string(), "••••••".to_string())));
        assert!(!rows.iter().any(|(_, v)| v == "hunter2"));
        // unchanged values are not listed
        assert_eq!(rows.len(), 3);

        // and the section renders in the footer
        let c = DebugCollector::new(FooterCfg::default());
        let html = c.render(&[], None, &rows);
        assert!(html.contains("CFConfig (3)"));
        assert!(html.contains("datasources.main.host"));
        assert!(!html.contains("hunter2"));
    }

    #[test]
    fn template_none_renders_empty_and_comment_renders_comment() {
        let mut cfg = FooterCfg::default();
        cfg.template = "none".into();
        let c = DebugCollector::new(cfg);
        c.on_query(&QueryEvent {
            name: "q",
            sql: "SELECT 1",
            datasource: "d",
            rowcount: 1,
            elapsed_us: 1_000,
            cached: false,
            src: "/a.cfm",
            line: 1,
            params: &[],
        });
        assert_eq!(c.render(&[], None, &[]), "");

        let mut cfg2 = FooterCfg::default();
        cfg2.template = "comment".into();
        let c2 = DebugCollector::new(cfg2);
        c2.on_query(&QueryEvent {
            name: "q",
            sql: "SELECT 1",
            datasource: "d",
            rowcount: 1,
            elapsed_us: 1_000,
            cached: false,
            src: "/a.cfm",
            line: 1,
            params: &[],
        });
        let out = c2.render(&[], None, &[]);
        assert!(out.contains(&format!("<!-- RustCFML v{} Debug", env!("CARGO_PKG_VERSION"))));
        assert!(out.contains("Queries: 1"));
        assert!(!out.contains("<table"));
    }

    #[test]
    fn html_is_escaped_in_output() {
        let c = DebugCollector::new(FooterCfg::default());
        c.add_generic("x", "name", "<script>alert(1)</script>");
        let html = c.render(&[], None, &[]);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn max_records_clips_queries() {
        let mut cfg = FooterCfg::default();
        cfg.max_records = 2;
        let c = DebugCollector::new(cfg);
        for n in 0..5 {
            c.on_query(&QueryEvent {
                name: "q",
                sql: "SELECT 1",
                datasource: "d",
                rowcount: 1,
                elapsed_us: n * 1_000,
                cached: false,
                src: "/a.cfm",
                line: 1,
            params: &[],
            });
        }
        let html = c.render(&[], None, &[]);
        assert!(html.contains("Queries (2)"));
        assert!(html.contains("+3 more queries clipped"));
    }

    #[test]
    fn to_cfml_projects_sections() {
        let c = sample_collector();
        let v = c.to_cfml(&[], Some("/index.cfm"));
        let s = match v {
            CfmlValue::Struct(s) => s,
            _ => panic!("expected struct"),
        };
        // queries array of 2
        match s.get_ci("queries") {
            Some(CfmlValue::Array(a)) => {
                assert_eq!(a.len(), 2);
                // first query carries its 2 bound params
                if let Some(CfmlValue::Struct(q0)) = a.snapshot().first() {
                    match q0.get_ci("params") {
                        Some(CfmlValue::Array(p)) => assert_eq!(p.len(), 2),
                        other => panic!("params not array: {:?}", other),
                    }
                } else {
                    panic!("first query not a struct");
                }
            }
            other => panic!("queries not array: {:?}", other),
        }
        match s.get_ci("exceptions") {
            Some(CfmlValue::Array(a)) => assert_eq!(a.len(), 1),
            other => panic!("exceptions not array: {:?}", other),
        }
        assert!(s.get_ci("total").is_some());
        assert!(s.get_ci("genericData").is_some());
    }
}
