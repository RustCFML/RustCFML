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
    /// Component/lifecycle method this hit entered; empty for a plain template
    /// execution (include, custom tag, `<cfmodule>`).
    pub method: String,
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
                method: t.method.unwrap_or_default().to_string(),
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
    /// Per-method breakdown, in first-call order. A file whose hits carry no
    /// method name (a plain include / custom tag) has an empty vec, so the row
    /// renders exactly as before.
    methods: Vec<MethodAgg>,
}

/// One method within a `PageAgg` — a CFC row is usually many *different*
/// methods, so the count on the file row alone hides where the time went.
struct MethodAgg {
    name: String,
    count: i64,
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
            method: String::new(),
            time: total_us,
        });
    }
    hits.extend_from_slice(templates);
    aggregate_pages(&hits)
}

fn aggregate_pages(templates: &[TemplateHit]) -> Vec<PageAgg> {
    let mut out: Vec<PageAgg> = Vec::new();
    for t in templates {
        let page = match out.iter_mut().position(|p| p.id == t.path) {
            Some(i) => {
                let p = &mut out[i];
                p.count += 1;
                p.total += t.time;
                p.min = p.min.min(t.time);
                p.max = p.max.max(t.time);
                p
            }
            None => {
                out.push(PageAgg {
                    id: t.path.clone(),
                    count: 1,
                    min: t.time,
                    max: t.time,
                    total: t.time,
                    methods: Vec::new(),
                });
                out.last_mut().expect("just pushed")
            }
        };
        if t.method.is_empty() {
            continue;
        }
        // Method names are case-insensitive in CFML; fold `getFoo`/`GETFOO`
        // into one row, keeping the casing first seen.
        if let Some(m) = page
            .methods
            .iter_mut()
            .find(|m| m.name.eq_ignore_ascii_case(&t.method))
        {
            m.count += 1;
            m.total += t.time;
        } else {
            page.methods.push(MethodAgg {
                name: t.method.clone(),
                count: 1,
                total: t.time,
            });
        }
    }
    // Busiest method first within each file — that's the one you're looking for.
    for p in &mut out {
        p.methods.sort_by(|a, b| b.total.cmp(&a.total));
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

// ── Collapse/expand plumbing ────────────────────────────────────────────────
//
// Every collapsible thing in the footer — a file's per-method sub-rows, a
// query's SQL, a whole scope dump — works the same way: the collapsible
// elements carry a group class and `display:none`, and a `+`/`−` link flips
// them. One inline script serves the lot; no external assets.

/// The single inline script backing every toggle. Emitted once per footer,
/// guarded so a page carrying two footers doesn't redefine it.
///
/// * `rcfmlTog(a, cls)` — flip everything in one group.
/// * `rcfmlTogAll(a, rowCls, togCls)` — flip every group at once and re-sync
///   the individual toggles so their icons can't disagree with the screen.
const TOGGLE_SCRIPT: &str = "<script>if(!window.rcfmlTog){\
window.rcfmlSetTog=function(a,open){a.setAttribute('data-open',open?'1':'0');a.textContent=open?'\u{2212}':'+';};\
window.rcfmlTog=function(a,c){\
var els=document.getElementsByClassName(c),open=a.getAttribute('data-open')!=='1',i;\
for(i=0;i<els.length;i++){els[i].style.display=open?'':'none';}\
window.rcfmlSetTog(a,open);return false;};\
window.rcfmlTogAll=function(a,rc,tc){\
var open=a.getAttribute('data-open')!=='1',\
rows=document.getElementsByClassName(rc),togs=document.getElementsByClassName(tc),i;\
for(i=0;i<rows.length;i++){rows[i].style.display=open?'':'none';}\
for(i=0;i<togs.length;i++){window.rcfmlSetTog(togs[i],open);}\
window.rcfmlSetTog(a,open);return false;};}</script>\n";

const TOG_STYLE: &str =
    "text-decoration:none;color:#333;font-weight:bold;cursor:pointer;margin-right:4px";

/// A `+` link that flips one group of elements. `class_attr` tags it for the
/// section's expand-all (empty when it has none).
fn tog_link(group: &str, class_attr: &str, title: &str) -> String {
    format!(
        "<a href=\"#\"{} data-open=\"0\" onclick=\"return window.rcfmlTog(this,'{}')\" title=\"{}\" style=\"{}\">+</a>",
        class_attr, group, title, TOG_STYLE
    )
}

/// A `+` link that flips every group in a section at once.
fn tog_all_link(row_class: &str, tog_class: &str, title: &str) -> String {
    format!(
        "<a href=\"#\" data-open=\"0\" onclick=\"return window.rcfmlTogAll(this,'{}','{}')\" title=\"{}\" style=\"{}\">+</a>",
        row_class, tog_class, title, TOG_STYLE
    )
}

/// An `<h4>` section heading with a leading `+` that shows/hides the block
/// tagged with `group` (which the caller must render `display:none`).
fn collapsible_heading(s: &mut String, group: &str, title: &str) {
    s.push_str(&format!(
        "<h4 style=\"margin:6px 0 2px\">{}{}</h4>\n",
        tog_link(group, "", "show/hide this block"),
        title
    ));
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
    s.push_str(TOGGLE_SCRIPT);

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
    // The whole section collapses behind its heading and starts CLOSED — on a
    // framework request it is hundreds of rows, and the summary above already
    // answers "where did the time go" at a glance.
    let pages = aggregate_pages_with_main(&data.templates, main_page, total_us);
    if !pages.is_empty() {
        collapsible_heading(
            &mut s,
            "rcfml-files",
            &format!(
                "Files (Templates/Tags/CFCs) ({} executed)",
                pages.iter().map(|p| p.count).sum::<i64>()
            ),
        );
        s.push_str("<div class=\"rcfml-files\" style=\"display:none\">\n");
        let any_methods = pages.iter().any(|p| !p.methods.is_empty());
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        // The header cell of the toggle column expands/collapses EVERY file's
        // breakdown at once, and keeps the per-row icons in sync with it.
        let all_toggle = if any_methods {
            tog_all_link(
                "rcfml-mrow",
                "rcfml-mtog",
                "show/hide every per-method breakdown",
            )
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<tr><th style=\"text-align:center;width:1em\">{}</th><th>total ms</th><th>app ms</th><th>query ms</th><th>count</th><th>avg ms</th><th>file</th></tr>\n",
            all_toggle
        ));
        for (idx, p) in pages.iter().enumerate() {
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
            // Files with a method breakdown get a `+` toggle in the leading
            // column; everything else gets an empty cell so the grid lines up.
            let grp = format!("rcfml-m{}", idx);
            let toggle = if p.methods.is_empty() {
                String::new()
            } else {
                tog_link(
                    &grp,
                    " class=\"rcfml-mtog\"",
                    "show the per-method breakdown",
                )
            };
            s.push_str(&format!(
                "<tr><td style=\"text-align:center;width:1em\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td>{}</td></tr>\n",
                toggle,
                fmt_us(p.total),
                fmt_us(app_us),
                fmt_us(q_us),
                p.count,
                fmt_us(avg),
                esc(&p.id),
            ));
            // A CFC row aggregates every method called on that file, so the file
            // count alone can't tell you whether it was 321 calls to one method
            // or a handful each to twenty. The breakdown goes underneath —
            // COLLAPSED by default (a request with hundreds of files would
            // otherwise be unreadable), revealed per file by the `+` above.
            for m in &p.methods {
                let m_avg = if m.count > 0 { m.total / m.count } else { 0 };
                s.push_str(&format!(
                    "<tr class=\"{} rcfml-mrow\" style=\"display:none;color:#555\"><td></td><td class=\"txt-r\">{}</td><td></td><td></td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td style=\"padding-left:22px\">&#8627; {}()</td></tr>\n",
                    grp,
                    fmt_us(m.total),
                    m.count,
                    fmt_us(m_avg),
                    esc(&m.name),
                ));
            }
        }
        s.push_str("</table>\n");
        s.push_str("</div>\n");
    }

    // Queries — like Files, the section collapses behind its heading and
    // starts CLOSED (the Execution Time table above shows the total query
    // cost; the detail is one click away).
    if cfg.database {
        collapsible_heading(
            &mut s,
            "rcfml-queries",
            &format!("Queries ({})", data.queries.len()),
        );
        s.push_str("<div class=\"rcfml-queries\" style=\"display:none\">\n");
        if data.queries.is_empty() {
            s.push_str("<div>(none)</div>\n");
        } else {
            s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
            // The SQL + params live in a sub-row, collapsed by default, so a
            // request with a dozen queries stays a readable one-line-per-query
            // list. The header `+` opens them all.
            s.push_str(&format!(
                "<tr><th style=\"text-align:center;width:1em\">{}</th><th>name</th><th>ms</th><th>rows</th><th>datasource</th><th>src</th></tr>\n",
                tog_all_link("rcfml-qrow", "rcfml-qtog", "show/hide every query's SQL")
            ));
            for (qidx, q) in data.queries.iter().enumerate() {
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
                let grp = format!("rcfml-q{}", qidx);
                s.push_str(&format!(
                    "<tr{}><td style=\"text-align:center;width:1em\">{}</td><td>{}</td><td class=\"txt-r\">{}</td><td>{}</td><td>{}</td><td>{}:{}</td></tr>\n",
                    row_style,
                    tog_link(&grp, " class=\"rcfml-qtog\"", "show the SQL and bound params"),
                    esc(&q.name),
                    fmt_us(q.time),
                    q.count,
                    esc(&q.datasource),
                    esc(&q.src),
                    q.line,
                ));
                s.push_str(&format!(
                    "<tr class=\"{} rcfml-qrow\" style=\"display:none\"><td></td><td colspan=\"5\">{}</td></tr>\n",
                    grp, sql_cell,
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
        s.push_str("</div>\n");
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
    // cgi. The deploy-level blocks (cfconfig overrides, then the engine
    // environment: process env vars + CLI flags) render directly under the cgi
    // scope — the natural place to look for "what was this engine started with"
    // while reading request context. Order: CFConfig, Environment, Runtime flags.
    //
    // These are bulk dumps — a cgi scope alone is ~30 rows and the environment
    // block can be hundreds — so each one collapses behind its heading and
    // starts closed, keeping the timing sections above it on screen.
    let mut env_rendered = false;
    for (idx, (name, map)) in scopes.iter().enumerate() {
        if map.is_empty() {
            continue;
        }
        let grp = format!("rcfml-s{}", idx);
        collapsible_heading(&mut s, &grp, &format!("{} scope", esc(name)));
        s.push_str(&format!("<table class=\"{}\" border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"display:none;border-collapse:collapse\">\n", grp));
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
            render_env_and_flags(&mut s);
            env_rendered = true;
        }
    }
    if !env_rendered {
        render_cfconfig(&mut s, cfconfig);
        render_env_and_flags(&mut s);
    }

    s.push_str("</div>\n");
    s
}

/// Render the engine's process environment variables and the runtime flags
/// (CLI arguments) it was started with.
fn render_env_and_flags(s: &mut String) {
    let mut envs: Vec<(String, String)> = std::env::vars().collect();
    envs.sort_by(|a, b| a.0.cmp(&b.0));
    collapsible_heading(
        s,
        "rcfml-env",
        &format!("Environment variables ({})", envs.len()),
    );
    if envs.is_empty() {
        s.push_str("<div class=\"rcfml-env\" style=\"display:none\">(none)</div>\n");
    } else {
        s.push_str("<table class=\"rcfml-env\" border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"display:none;border-collapse:collapse\">\n");
        for (k, v) in &envs {
            let shown = if v.len() > 200 {
                let mut end = 200;
                while !v.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…", &v[..end])
            } else {
                v.clone()
            };
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>\n",
                esc(k),
                esc(&shown)
            ));
        }
        s.push_str("</table>\n");
    }

    let flags: Vec<String> = std::env::args().skip(1).collect();
    collapsible_heading(s, "rcfml-flags", &format!("Runtime flags ({})", flags.len()));
    if flags.is_empty() {
        s.push_str("<div class=\"rcfml-flags\" style=\"display:none\">(none)</div>\n");
    } else {
        s.push_str("<table class=\"rcfml-flags\" border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"display:none;border-collapse:collapse\">\n");
        for f in &flags {
            s.push_str(&format!("<tr><td>{}</td></tr>\n", esc(f)));
        }
        s.push_str("</table>\n");
    }
}

/// Render the settings this deploy's cfconfig changed from engine defaults
/// (already flattened + credential-redacted by [`cfconfig_diff_rows`]).
fn render_cfconfig(s: &mut String, rows: &[(String, String)]) {
    collapsible_heading(s, "rcfml-cfg", &format!("CFConfig ({})", rows.len()));
    if rows.is_empty() {
        s.push_str(
            "<div class=\"rcfml-cfg\" style=\"display:none\">(engine defaults — no cfconfig overrides)</div>\n",
        );
    } else {
        s.push_str("<table class=\"rcfml-cfg\" border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"display:none;border-collapse:collapse\">\n");
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
            // Per-method breakdown for a CFC row (empty array for a plain
            // template). Each entry: name, count, total (µs).
            let methods: Vec<CfmlValue> = p
                .methods
                .iter()
                .map(|mm| {
                    let mut e = ValueMap::default();
                    e.insert("name".into(), CfmlValue::string(mm.name.clone()));
                    e.insert("count".into(), CfmlValue::Int(mm.count));
                    e.insert("total".into(), CfmlValue::Int(mm.total));
                    CfmlValue::strukt(e)
                })
                .collect();
            m.insert("methods".into(), CfmlValue::array(methods));
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
            method: None,
            elapsed_us: 2_000,
        });
        // Two different methods on one CFC — the per-file row must break down
        // into per-method sub-rows rather than showing a bare count of 3.
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("getUser"),
            elapsed_us: 1_000,
        });
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("GETUSER"),
            elapsed_us: 3_000,
        });
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("saveUser"),
            elapsed_us: 500,
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
        // engine environment renders even without a cgi scope in the snapshot
        assert!(html.contains("Environment variables ("));
        assert!(html.contains("Runtime flags ("));
        assert!(html.contains("Queries (2)"));
        assert!(html.contains("SELECT * FROM users"));
        // bound parameters are shown under the SQL (Lucee parity)
        assert!(html.contains("params:"));
        assert!(html.contains("<code>id=7</code>"));
        assert!(html.contains("<code>active=true</code>"));
        // slow query (>= highlightMs 250) is red-highlighted
        assert!(html.contains("background:#fdd"));
        assert!(html.contains("Files (Templates/Tags/CFCs) (5 executed)"));
        // the main page is listed alongside the include
        assert!(html.contains("/index.cfm"));
        assert!(html.contains("/header.cfm"));
        // per-method breakdown under the CFC row, busiest first, case-folded
        assert!(html.contains("/services/UserService.cfc"));
        let get_at = html.find("getUser()").expect("getUser sub-row");
        let save_at = html.find("saveUser()").expect("saveUser sub-row");
        assert!(get_at < save_at, "busiest method should sort first");
        // getUser + GETUSER folded into one row with count 2
        assert!(!html.contains("GETUSER()"));
        // collapsed by default, with a `+` toggle on the CFC row only
        assert!(html.contains("window.rcfmlTog=function"));
        assert!(html.contains("style=\"display:none;color:#555\""));
        assert_eq!(
            html.matches("class=\"rcfml-mtog\"").count(),
            1,
            "only the one file with methods gets a row toggle"
        );
        // plus the expand-all toggle in the column header
        assert_eq!(
            html.matches("window.rcfmlTogAll(this,'rcfml-mrow','rcfml-mtog')")
                .count(),
            1
        );
        assert_eq!(
            html.matches("rcfml-mrow").count(),
            3,
            "2 sub-rows + the expand-all onclick"
        );
        // queries: SQL/params moved into a collapsed sub-row, one toggle each
        // plus the section-wide one in the header cell
        assert_eq!(html.matches("class=\"rcfml-qtog\"").count(), 2);
        assert_eq!(
            html.matches("window.rcfmlTogAll(this,'rcfml-qrow','rcfml-qtog')")
                .count(),
            1
        );
        assert_eq!(
            html.matches("<tr class=\"rcfml-q0 rcfml-qrow\" style=\"display:none\">")
                .count(),
            1
        );
        assert!(html.contains("Exceptions (1)"));
        assert!(html.contains("kaboom"));
        assert!(html.contains("Generic data"));
        assert!(html.contains("controller"));
        // Section order: Execution Time first, then Files, then Queries.
        let time_at = html.find("Execution Time").expect("Execution Time section");
        let files_at = html
            .find("Files (Templates/Tags/CFCs)")
            .expect("Files section");
        let queries_at = html.find("Queries (2)").expect("Queries section");
        assert!(
            time_at < files_at && files_at < queries_at,
            "expected Execution Time < Files < Queries, got {time_at}/{files_at}/{queries_at}"
        );
        // Files and Queries collapse behind their headings and start CLOSED,
        // same as the scope dumps.
        assert!(html.contains("window.rcfmlTog(this,'rcfml-files')"));
        assert!(html.contains("window.rcfmlTog(this,'rcfml-queries')"));
        assert!(html.contains("<div class=\"rcfml-files\" style=\"display:none\">"));
        assert!(html.contains("<div class=\"rcfml-queries\" style=\"display:none\">"));
    }

    #[test]
    fn scope_and_env_blocks_render_in_canonical_order() {
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
            at("Environment variables ("),
            at("Runtime flags ("),
        ];
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "expected URL, FORM, CGI, CFConfig, Environment, Runtime flags — got offsets {order:?}"
        );
    }

    #[test]
    fn dump_blocks_are_collapsed_behind_their_headings() {
        let scope = |k: &str| {
            let mut m = ValueMap::default();
            m.insert(k.to_string(), CfmlValue::string("v".to_string()));
            m
        };
        let scopes = vec![
            ("url".to_string(), scope("a")),
            ("cgi".to_string(), scope("c")),
        ];
        let html = sample_collector().render(
            &scopes,
            Some("/index.cfm"),
            &[("runtime.reportAsLucee".to_string(), "false".to_string())],
        );
        // Every dump block ships hidden, each behind its own heading toggle.
        for grp in ["rcfml-s0", "rcfml-s1", "rcfml-cfg", "rcfml-env", "rcfml-flags"] {
            assert!(
                html.contains(&format!("window.rcfmlTog(this,'{grp}')")),
                "{grp} has no heading toggle"
            );
            assert!(
                html.contains(&format!("class=\"{grp}\"")),
                "{grp} block is not tagged"
            );
        }
        // …and none of them is visible on load: every tagged block carries
        // display:none.
        for block in html.split("class=\"rcfml-").skip(1) {
            let tag = block.split('"').next().unwrap_or("");
            if tag.starts_with('s')
                || tag == "cfg"
                || tag == "env"
                || tag == "flags"
            {
                let head: String = block.chars().take(160).collect();
                assert!(
                    head.contains("display:none"),
                    "rcfml-{tag} block renders expanded: {head}"
                );
            }
        }
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
