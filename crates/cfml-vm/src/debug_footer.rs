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
    /// Bytes this execution allocated itself, when the request is metered.
    pub alloc: u64,
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
    pub templates: TemplateAgg,
    pub exceptions: Vec<ExceptionRow>,
    pub generic: Vec<GenericRow>,
    pub traces: Vec<TraceRow>,
    /// Per-section overflow counts when `maxRecords` clips a section.
    pub dropped_queries: usize,
    /// Time spent in queries whose ROW DETAIL was clipped by `maxRecords`.
    /// The clip drops the SQL/params bulk, never the accounting — the
    /// Execution Time summary and the per-file query column stay correct no
    /// matter how low the cap is.
    pub dropped_query_us: i64,
    /// The clipped queries' time attributed to the template that issued them
    /// (keyed by `src`), so the Files table's per-file query/app split stays
    /// correct too.
    pub dropped_query_us_by_src: std::collections::HashMap<String, i64>,
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
        self.render_with_memory(scopes, main_page, cfconfig, None)
    }

    /// [`render`](Self::render), plus the memory panel when the request's
    /// memory was metered.
    pub fn render_with_memory(
        &self,
        scopes: &[(String, ValueMap)],
        main_page: Option<&str>,
        cfconfig: &[(String, String)],
        memory: Option<&MemoryPanel>,
    ) -> String {
        if let Ok(d) = self.inner.lock() {
            render_footer(&self.cfg, &d, scopes, self.total_us(), main_page, cfconfig, memory)
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
                // Drop the row detail (SQL/params are the bulk) but keep the
                // accounting: total query time and its per-file attribution
                // must not depend on the display cap.
                d.dropped_queries += 1;
                d.dropped_query_us += q.elapsed_us;
                *d.dropped_query_us_by_src
                    .entry(q.src.to_string())
                    .or_insert(0) += q.elapsed_us;
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
            d.templates
                .record(t.path, t.method.unwrap_or_default(), t.elapsed_us, t.alloc_bytes);
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
#[derive(Clone)]
struct PageAgg {
    id: String,
    count: i64,
    min: i64,
    max: i64,
    total: i64,
    /// Bytes allocated (exclusive), when the request is metered.
    alloc: u64,
    /// Per-method breakdown, in first-call order. A file whose hits carry no
    /// method name (a plain include / custom tag) has an empty vec, so the row
    /// renders exactly as before.
    methods: Vec<MethodAgg>,
}

/// One method within a `PageAgg` — a CFC row is usually many *different*
/// methods, so the count on the file row alone hides where the time went.
#[derive(Clone)]
struct MethodAgg {
    name: String,
    count: i64,
    total: i64,
    alloc: u64,
}

impl PageAgg {
    fn new(id: &str) -> Self {
        PageAgg { id: id.to_string(), count: 0, min: i64::MAX, max: i64::MIN, total: 0, alloc: 0, methods: Vec::new() }
    }

    fn add(&mut self, method: &str, time: i64, alloc: u64) {
        self.count += 1;
        self.total += time;
        self.alloc += alloc;
        self.min = self.min.min(time);
        self.max = self.max.max(time);
        if method.is_empty() {
            return;
        }
        // Method names are case-insensitive in CFML; fold `getFoo`/`GETFOO`
        // into one row, keeping the casing first seen.
        if let Some(m) = self.methods.iter_mut().find(|m| m.name.eq_ignore_ascii_case(method)) {
            m.count += 1;
            m.total += time;
            m.alloc += alloc;
        } else {
            self.methods.push(MethodAgg { name: method.to_string(), count: 1, total: time, alloc });
        }
    }
}

/// Template executions aggregated AS THEY HAPPEN into one row per file (and
/// per method within it), in first-encounter order.
///
/// This used to keep one `TemplateHit` per execution — two freshly allocated
/// strings each — and aggregate at render time by cloning the whole list and
/// searching it linearly per hit. A Preside boot makes ~715k timed calls: the
/// footer's own bookkeeping was ~180 MB, and because a hit was recorded after
/// the frame's byte count was taken, that allocation landed on the CALLER's
/// row (anything making many small calls looked heavy). Memory is now per row.
#[derive(Default)]
pub struct TemplateAgg {
    pages: Vec<PageAgg>,
    index: std::collections::HashMap<String, usize>,
}

impl TemplateAgg {
    pub fn record(&mut self, path: &str, method: &str, time: i64, alloc: u64) {
        let i = match self.index.get(path) {
            Some(&i) => i,
            None => {
                self.pages.push(PageAgg::new(path));
                self.index.insert(path.to_string(), self.pages.len() - 1);
                self.pages.len() - 1
            }
        };
        self.pages[i].add(method, time, alloc);
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    #[cfg(test)]
    fn from_hits(hits: &[TemplateHit]) -> Self {
        let mut a = TemplateAgg::default();
        for h in hits {
            a.record(&h.path, &h.method, h.time, h.alloc);
        }
        a
    }
}

/// Aggregated `pages` rows, optionally leading with the main page. The main page
/// is listed first, then each included template in encounter order — matching
/// Lucee's habit of showing the requested page plus every `<cfinclude>`/render
/// below it.
///
/// The main page's time is the request total MINUS every recorded frame, not the
/// total itself. Only includes, component methods and custom tags open a timed
/// frame; the top-level page never does, so it has no self-time of its own to
/// report. Since every recorded hit carries EXCLUSIVE (self) time, the hits
/// partition the request between frames and the residual is exactly what the
/// top-level page spent in its own body.
///
/// Booking the full `total_us` here instead — which is what this did — put the
/// whole request into the first row of a self-time table: the column no longer
/// summed to anything (every frame below was counted twice, once in its own row
/// and once inside the main page's), the requested page always sorted to the top
/// of "total ms" however little work it actually did, and there was no way to see
/// the page's own cost. The request total is still reported on its own, in the
/// summary line above this table.
fn aggregate_pages_with_main(
    templates: &TemplateAgg,
    main_page: Option<&str>,
    total_us: i64,
    total_alloc: u64,
) -> Vec<PageAgg> {
    let mut out: Vec<PageAgg> = Vec::with_capacity(templates.pages.len() + 1);
    if let Some(p) = main_page {
        // Template hits are never clipped (unlike queries, which have
        // `max_records`), so this subtraction can't silently over-credit the
        // page. `max(0)` guards only against per-frame microsecond truncation.
        let frames_us: i64 = templates.pages.iter().map(|t| t.total).sum();
        // The same residual in bytes: what the request allocated outside every
        // timed frame is the page's own.
        let frames_alloc: u64 = templates.pages.iter().map(|t| t.alloc).sum();
        let mut main = PageAgg::new(p);
        main.add("", (total_us - frames_us).max(0), total_alloc.saturating_sub(frames_alloc));
        // The main page re-entered as a frame (included again) folds into its row.
        if let Some(&i) = templates.index.get(p) {
            let t = &templates.pages[i];
            main.count += t.count;
            main.total += t.total;
            main.alloc += t.alloc;
            main.min = main.min.min(t.min);
            main.max = main.max.max(t.max);
            main.methods = t.methods.clone();
        }
        out.push(main);
    }
    out.extend(templates.pages.iter().filter(|t| Some(t.id.as_str()) != main_page).cloned());
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
//
// The footer renders inside a shadow root (GH #437), so the page's CSS can't
// restyle it and its own styles and class names can't leak out. Controls are
// marked with data attributes rather than `onclick`, because an inline handler
// runs in the page's global scope: the script below adds ONE click listener to
// the shadow root and defines nothing on `window`.

/// Attaches the shadow root and wires every control. Emitted right after the
/// host element, which it finds as `document.currentScript`'s previous sibling.
///
/// Browsers with declarative shadow DOM (Chrome/Edge 111+, Safari 16.4+,
/// Firefox 123+) have already attached it from the `<template
/// shadowrootmode>`. Older ones see a plain `<template>`, so the script
/// attaches the root itself, or, with no `attachShadow` at all, falls back to
/// rendering the content in the page.
///
/// * `data-rcfml-tog="cls"` — flip everything in one group; the control's
///   `+`/`−` indicator is the control itself when it is a link, else its
///   first link (a section heading).
/// * `data-rcfml-togall="rowCls togCls"` — flip every group at once and
///   re-sync the individual toggles so their icons can't disagree with the
///   screen.
/// * `data-rcfml-sort="col"` — sort the table on a numeric column, toggling
///   descending → ascending → descending. Rows move as BLOCKS: a file row
///   drags its (collapsed) per-method sub-rows with it, and those sub-rows are
///   sorted the same way inside the block, so the method breakdown always
///   agrees with the direction shown in the header. Cells with no number (the
///   query column on a method sub-row) sort last in both directions, and ties
///   keep their original order.
const FOOTER_SCRIPT: &str = "<script>(function(s){\
var h=s&&s.previousElementSibling;if(!h)return;var r=h.shadowRoot;\
if(!r){var t=h.querySelector('template');if(!t)return;\
if(h.attachShadow){r=h.attachShadow({mode:'open'});r.appendChild(t.content);}\
else{h.appendChild(t.content);r=h;}t.parentNode.removeChild(t);}\
function all(c){return r.querySelectorAll('.'+c);}\
function setTog(a,open){if(!a)return;a.setAttribute('data-open',open?'1':'0');a.textContent=open?'\u{2212}':'+';}\
function ind(el){return el.tagName==='A'?el:el.getElementsByTagName('a')[0];}\
function tog(el,c){var a=ind(el),open=!a||a.getAttribute('data-open')!=='1',els=all(c),i;\
for(i=0;i<els.length;i++){els[i].style.display=open?'':'none';}setTog(a,open);}\
function togAll(el,rc,tc){var a=ind(el),open=!a||a.getAttribute('data-open')!=='1',rows=all(rc),togs=all(tc),i;\
for(i=0;i<rows.length;i++){rows[i].style.display=open?'':'none';}\
for(i=0;i<togs.length;i++){setTog(togs[i],open);}setTog(a,open);}\
function key(row,c){var td=row.cells[c];if(!td)return null;\
var t=td.textContent.replace(/,/g,'');if(t==='')return null;\
var n=parseFloat(t);return isNaN(n)?null:n;}\
function cmp(c,desc){return function(a,b){\
var x=key(a.r,c),y=key(b.r,c);\
if(x===null&&y===null)return a.i-b.i;\
if(x===null)return 1;if(y===null)return -1;\
if(x===y)return a.i-b.i;return desc?y-x:x-y;};}\
function sort(th,c){\
var t=th;while(t&&t.tagName!=='TABLE'){t=t.parentNode;}if(!t)return;\
var hdr=th.parentNode,ths=hdr.getElementsByTagName('th'),i,j,ix;\
var desc=th.getAttribute('data-dir')!=='desc';\
for(i=0;i<ths.length;i++){ths[i].removeAttribute('data-dir');\
ix=ths[i].getElementsByClassName('rcfml-ind')[0];\
if(ix){ix.textContent='\u{21C5}';ix.style.color='#999';}}\
th.setAttribute('data-dir',desc?'desc':'asc');\
ix=th.getElementsByClassName('rcfml-ind')[0];\
if(ix){ix.textContent=desc?'\u{25BC}':'\u{25B2}';ix.style.color='';}\
var body=t.tBodies[0]||t,rows=[],groups=[],g=null,row;\
for(i=0;i<body.rows.length;i++){rows.push(body.rows[i]);}\
for(i=0;i<rows.length;i++){row=rows[i];\
if(row.cells.length&&row.cells[0].tagName==='TH')continue;\
if(/(^|\\s)rcfml-sub(\\s|$)/.test(row.className)&&g){g.k.push({r:row,i:g.k.length});}\
else{g={r:row,i:groups.length,k:[]};groups.push(g);}}\
groups.sort(cmp(c,desc));\
for(i=0;i<groups.length;i++){g=groups[i];g.k.sort(cmp(c,desc));\
body.appendChild(g.r);for(j=0;j<g.k.length;j++){body.appendChild(g.k[j].r);}}}\
r.addEventListener('click',function(e){\
var el=e.target;while(el&&el!==r&&el.nodeType===1&&!el.hasAttribute('data-rcfml-tog')\
&&!el.hasAttribute('data-rcfml-togall')&&!el.hasAttribute('data-rcfml-sort')){el=el.parentNode;}\
if(!el||el===r||el.nodeType!==1)return;e.preventDefault();var v;\
if((v=el.getAttribute('data-rcfml-tog'))!==null){tog(el,v);}\
else if((v=el.getAttribute('data-rcfml-togall'))!==null){v=v.split(' ');togAll(el,v[0],v[1]);}\
else{sort(el,parseInt(el.getAttribute('data-rcfml-sort'),10));}});\
})(document.currentScript);</script>\n";

/// The footer's own styles, scoped to its shadow root. `:host` resets every
/// property, inherited ones included (`font`, `color`, `line-height` cross a
/// shadow boundary), so the footer starts from the same clean slate on every
/// page. `!important` because a page's `!important` rule on the host element
/// (`* { font: 30px serif !important }`) beats a normal `:host` rule, while
/// an `!important` one from inside the shadow tree beats the page's.
const FOOTER_STYLE: &str = "<style>\
:host{all:initial !important;display:block !important;box-sizing:border-box !important;\
width:100% !important;flex:1 1 100% !important;grid-column:1/-1 !important;clear:both !important}\
.rustcfml-debug{font-family:monospace;font-size:12px;font-weight:normal;font-style:normal;\
line-height:1.35;color:#222;text-align:left;letter-spacing:normal;word-spacing:normal;\
text-transform:none;white-space:normal}\
.rustcfml-debug *{font-family:inherit;font-size:inherit;line-height:inherit;letter-spacing:inherit}\
h3,h4{font-weight:bold;color:inherit;padding:0;border:0;background:none;text-transform:none}\
.rustcfml-debug h3{font-size:13px}\
table{border-collapse:collapse;border:1px solid #999;margin:0 0 4px;background:transparent;width:auto}\
th,td{border:1px solid #999;padding:3px;vertical-align:top;text-align:left;color:inherit;background:transparent}\
th{font-weight:bold}\
.txt-r{text-align:right}\
a{color:#333;text-decoration:none;cursor:pointer}\
pre,code{font-family:monospace;white-space:pre-wrap}\
small{font-size:11px}\
</style>\n";

const TOG_STYLE: &str =
    "text-decoration:none;color:#333;font-weight:bold;cursor:pointer;margin-right:4px";

/// A `+` link that flips one group of elements. `class_attr` tags it for the
/// section's expand-all (empty when it has none).
fn tog_link(group: &str, class_attr: &str, title: &str) -> String {
    format!(
        "<a href=\"#\"{} data-open=\"0\" data-rcfml-tog=\"{}\" title=\"{}\" style=\"{}\">+</a>",
        class_attr, group, title, TOG_STYLE
    )
}

/// A `+` link that flips every group in a section at once.
fn tog_all_link(row_class: &str, tog_class: &str, title: &str) -> String {
    format!(
        "<a href=\"#\" data-open=\"0\" data-rcfml-togall=\"{} {}\" title=\"{}\" style=\"{}\">+</a>",
        row_class, tog_class, title, TOG_STYLE
    )
}

/// A sortable column header. `col` is the cell index this header controls —
/// clicking it sorts the table on that column, biggest-first, and clicking
/// again flips to smallest-first. The trailing glyph is the state indicator:
/// `⇅` idle, `▼` descending, `▲` ascending.
fn sort_th(label: &str, col: usize) -> String {
    format!(
        "<th data-rcfml-sort=\"{}\" title=\"sort by {}\" style=\"cursor:pointer;user-select:none\">{} <span class=\"rcfml-ind\" style=\"color:#999\">\u{21c5}</span></th>",
        col, label, label
    )
}

/// An `<h4>` section heading with a leading `+` that shows/hides the block
/// tagged with `group` (which the caller must render `display:none`).
///
/// The WHOLE heading is the click target, not just the `+` — the anchor is
/// only the state indicator, and a click on it bubbles to the heading.
fn collapsible_heading(s: &mut String, group: &str, title: &str) {
    s.push_str(&format!(
        "<h4 style=\"margin:6px 0 2px;cursor:pointer;user-select:none\" title=\"show/hide this block\" data-rcfml-tog=\"{}\"><a href=\"#\" data-open=\"0\" style=\"{}\">+</a>{}</h4>\n",
        group, TOG_STYLE, title
    ));
}

/// [`collapsible_heading`] whose title carries an explanation as a hover
/// tooltip (marked with a small ⓘ), instead of a paragraph under the block.
fn collapsible_heading_with_tip(s: &mut String, group: &str, title: &str, tip: &str) {
    collapsible_heading(
        s,
        group,
        &format!(
            "<span title=\"{}\" style=\"cursor:help\">{} <span style=\"color:#999;font-weight:normal\">&#9432;</span></span>",
            esc(tip),
            title
        ),
    );
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

/// Put the rendered footer into the page: just before the last `</body>` when
/// there is one (inside the document, where the browser parses it as part of
/// the page, shadow root and all), else at the end, as it always was.
pub fn insert_footer(page: &mut String, footer: &str) {
    let bytes = page.as_bytes();
    let needle = b"</body";
    let at = (0..bytes.len().saturating_sub(needle.len() - 1))
        .rev()
        .find(|&i| bytes[i..i + needle.len()].eq_ignore_ascii_case(needle));
    match at {
        Some(i) => page.insert_str(i, footer),
        None => page.push_str(footer),
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
    memory: Option<&MemoryPanel>,
) -> String {
    match cfg.template.to_ascii_lowercase().as_str() {
        "none" => String::new(),
        "comment" => render_comment(data, total_us),
        "simple" => render_html(cfg, data, scopes, total_us, main_page, cfconfig, memory, false),
        "classic" => render_html(cfg, data, scopes, total_us, main_page, cfconfig, memory, false),
        // "modern" (default) and any unknown template fall back to the rich panel.
        _ => render_html(cfg, data, scopes, total_us, main_page, cfconfig, memory, true),
    }
}

// ── Memory panel ─────────────────────────────────────────────────────────────

/// What the memory panel shows. Built by the VM when the request's memory was
/// metered (allocation accounting on, i.e. debugging enabled).
#[derive(Debug, Clone, Default)]
pub struct MemoryPanel {
    /// This request, on the thread that ran it, up to the moment the footer
    /// renders (before the request-end cleanup).
    pub request: cfml_common::mem_account::RequestMemory,
    /// Bytes allocated by the `cfthread`s it joined.
    pub threads_allocated: u64,
    /// The containers it created, by kind, and what of them is still alive.
    pub census: Option<cfml_common::cycle_gc::RequestCensus>,
    /// Process: physical footprint, `--max-memory` limit, live heap.
    pub footprint: Option<u64>,
    pub limit: Option<u64>,
    pub live_heap: Option<u64>,
    /// Where the live heap is, estimated (see [`MemoryPot`]).
    pub pots: Vec<MemoryPot>,
    /// How old the pot estimates are, in seconds (they are cached).
    pub pots_age_secs: u64,
}

/// One estimated share of the live heap.
#[derive(Debug, Clone, Default)]
pub struct MemoryPot {
    pub name: String,
    pub bytes: u64,
    /// Entry counts and the like.
    pub detail: String,
}

/// Bytes as `1.23 MB` / `456.7 KB` / `89 B`.
pub fn fmt_bytes(b: u64) -> String {
    const K: f64 = 1024.0;
    let f = b as f64;
    if f >= K * K * K {
        format!("{:.2} GB", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.2} MB", f / (K * K))
    } else if f >= K {
        format!("{:.1} KB", f / K)
    } else {
        format!("{} B", b)
    }
}

/// Bytes as a plain KB number for a sortable column.
fn fmt_kb(b: u64) -> String {
    format!("{:.1}", b as f64 / 1024.0)
}

fn render_memory(s: &mut String, m: &MemoryPanel) {
    let r = &m.request;
    s.push_str("<h4 style=\"margin:6px 0 2px\">Request Memory</h4>\n");
    s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
    let row = |s: &mut String, v: String, label: &str, title: &str| {
        s.push_str(&format!(
            "<tr title=\"{}\"><td class=\"txt-r\">{}</td><td>{}</td></tr>\n",
            title, v, label
        ));
    };
    row(s, fmt_bytes(r.allocated), "Allocated by this request", "every byte this request allocated, including what it has since freed");
    row(s, fmt_bytes(r.peak), "Peak in use", "the most memory this request held at any one time");
    row(s, fmt_bytes(r.retained), "Still in use as the page ends", "allocated minus freed, before the request-end cleanup frees the page's own variables");
    if let Some(c) = &m.census {
        let swept = if c.sweeps == 0 {
            "no collector sweeps yet".to_string()
        } else {
            format!("{} of it by {} collector sweep{}", fmt_bytes(c.swept_bytes), c.sweeps, if c.sweeps == 1 { "" } else { "s" })
        };
        row(s, fmt_bytes(r.freed), &format!("Freed so far ({})", swept), "bytes freed while the request ran, by reference counting and the cycle collector; the request-end cleanup comes after this panel");
    } else {
        row(s, fmt_bytes(r.freed), "Freed", "bytes freed while the request ran");
    }
    if m.threads_allocated > 0 {
        row(s, fmt_bytes(m.threads_allocated), "Allocated by joined cfthreads", "allocated on the threads this request started and joined");
    }
    s.push_str("</table>\n");

    // Objects by type: what the request created and still holds, with the
    // component instances broken down by class under their row.
    if let Some(c) = &m.census {
        let kinds: [(&str, &cfml_common::cycle_gc::KindCensus); 5] = [
            ("Structs", &c.structs),
            ("Arrays", &c.arrays),
            ("Queries", &c.queries),
            ("Closure scopes", &c.scopes),
            ("Component instances", &c.instances),
        ];
        let created: u64 = kinds.iter().map(|(_, k)| k.created).sum();
        collapsible_heading_with_tip(
            s,
            "rcfml-memobj",
            &format!("Objects created ({})", created),
            "Alive = still referenced as the page ends; the request-end cleanup frees what only the page held. \
             Sizes are estimates, each object without the objects it holds (those count under their own type); \
             strings and other values are included in the object that holds them.",
        );
        s.push_str("<div class=\"rcfml-memobj\" style=\"display:none\">\n");
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        let any_classes = !c.classes.is_empty();
        s.push_str(&format!(
            "<tr><th style=\"text-align:center;width:1em\">{}</th>{}{}{}<th>type</th></tr>\n",
            "",
            sort_th("created", 1),
            sort_th("alive", 2),
            sort_th("alive KB (est.)", 3),
        ));
        for (name, k) in kinds {
            let is_inst = name == "Component instances";
            let toggle = if is_inst && any_classes {
                tog_link("rcfml-memcls", "", "show the instances by component")
            } else {
                String::new()
            };
            s.push_str(&format!(
                "<tr><td style=\"text-align:center;width:1em\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td>{}</td></tr>\n",
                toggle, k.created, k.alive, fmt_kb(k.alive_bytes), name
            ));
            if is_inst {
                for (cls, ck) in &c.classes {
                    s.push_str(&format!(
                        "<tr class=\"rcfml-memcls rcfml-sub\" style=\"display:none;color:#555\"><td></td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td style=\"padding-left:22px\">&#8627; {}</td></tr>\n",
                        ck.created, ck.alive, fmt_kb(ck.alive_bytes), esc(cls)
                    ));
                }
            }
        }
        s.push_str("</table>\n");
        // The explanation lives in the heading's tooltip; only this caveat,
        // which changes how to read the numbers, stays on the page.
        if c.incomplete {
            s.push_str("<div style=\"color:#555;margin:2px 0 4px\">This request created more objects than the collector logs, so the alive figures are a lower bound.</div>\n");
        }
        s.push_str("</div>\n");
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
    if data.dropped_queries > 0 {
        s.push_str(&format!(
            "  (+{} more queries, {} ms, clipped by maxRecords)\n",
            data.dropped_queries,
            fmt_us(data.dropped_query_us)
        ));
    }
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
            if !e.detail.is_empty() {
                s.push_str(&format!("      detail: {}\n", e.detail.replace('\n', " ")));
            }
        }
    }
    s.push_str("-->\n");
    s
}

#[allow(clippy::too_many_arguments)]
fn render_html(
    cfg: &FooterCfg,
    data: &DebugData,
    scopes: &[(String, ValueMap)],
    total_us: i64,
    main_page: Option<&str>,
    cfconfig: &[(String, String)],
    memory: Option<&MemoryPanel>,
    modern: bool,
) -> String {
    let mut s = String::new();
    let style = if modern {
        "font-family:monospace;font-size:12px;background:#f5f5f5;color:#222;border-top:3px solid #c33;margin-top:20px;padding:8px 12px"
    } else {
        "font-family:monospace;font-size:12px"
    };
    // A shadow root isolates the footer from the page's CSS (GH #437); see
    // FOOTER_STYLE and FOOTER_SCRIPT.
    s.push_str("\n<div class=\"rustcfml-debug-host\"><template shadowrootmode=\"open\">\n");
    s.push_str(FOOTER_STYLE);
    s.push_str(&format!(
        "<div class=\"rustcfml-debug\" style=\"{}\">\n",
        style
    ));
    s.push_str(&format!(
        "<h3 style=\"margin:4px 0\">RustCFML v{} Debug &mdash; total {} ms</h3>\n",
        env!("CARGO_PKG_VERSION"),
        fmt_us(total_us)
    ));

    // Execution Time summary (Lucee's breakdown): Total, time spent in Query,
    // and Application (= total − query). Load/compilation is not yet tracked
    // separately, so it folds into Application. Queries clipped from the list
    // by `maxRecords` still count here — the cap trims the display, not the
    // accounting (Lucee's own summary undercounts when debugMaxRecordsLogged
    // clips; deliberately not inherited).
    let query_total_us: i64 =
        data.queries.iter().map(|q| q.time).sum::<i64>() + data.dropped_query_us;
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
    if let Some(m) = memory {
        render_memory(&mut s, m);
    }

    let pages = aggregate_pages_with_main(
        &data.templates,
        main_page,
        total_us,
        memory.map(|m| m.request.allocated).unwrap_or(0),
    );
    // The per-file allocation column appears only when the request was
    // metered; otherwise the table is exactly as it always was.
    let mem_col = memory.is_some();
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
        // `app ms` used to sit between total and query; it's just total − query,
        // and on the (common) query-free row it duplicated total exactly — so
        // the column carried no information the eye couldn't do itself.
        s.push_str(&format!(
            "<tr><th style=\"text-align:center;width:1em\">{}</th>{}{}{}{}{}<th>file</th></tr>\n",
            all_toggle,
            sort_th("total ms", 1),
            sort_th("query ms", 2),
            sort_th("count", 3),
            sort_th("avg ms", 4),
            if mem_col { sort_th("alloc KB", 5) } else { String::new() },
        ));
        for (idx, p) in pages.iter().enumerate() {
            let avg = if p.count > 0 { p.total / p.count } else { 0 };
            // Per-template query time: sum of queries issued from this file
            // (Lucee's per-page Query column), including any clipped from the
            // Queries list by maxRecords. Whatever's left of `total` is app time.
            let q_us: i64 = data
                .queries
                .iter()
                .filter(|q| q.src == p.id)
                .map(|q| q.time)
                .sum::<i64>()
                + data
                    .dropped_query_us_by_src
                    .get(&p.id)
                    .copied()
                    .unwrap_or(0);
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
            let alloc_cell = if mem_col {
                format!("<td class=\"txt-r\">{}</td>", fmt_kb(p.alloc))
            } else {
                String::new()
            };
            s.push_str(&format!(
                "<tr><td style=\"text-align:center;width:1em\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td>{}<td>{}</td></tr>\n",
                toggle,
                fmt_us(p.total),
                fmt_us(q_us),
                p.count,
                fmt_us(avg),
                alloc_cell,
                esc(&p.id),
            ));
            // A CFC row aggregates every method called on that file, so the file
            // count alone can't tell you whether it was 321 calls to one method
            // or a handful each to twenty. The breakdown goes underneath —
            // COLLAPSED by default (a request with hundreds of files would
            // otherwise be unreadable), revealed per file by the `+` above.
            for m in &p.methods {
                let m_avg = if m.count > 0 { m.total / m.count } else { 0 };
                // `rcfml-sub` marks the row as belonging to the file row above
                // it, so a header-click sort moves the pair together and orders
                // the methods the same way (see `FOOTER_SCRIPT`).
                let m_alloc = if mem_col {
                    format!("<td class=\"txt-r\">{}</td>", fmt_kb(m.alloc))
                } else {
                    String::new()
                };
                s.push_str(&format!(
                    "<tr class=\"{} rcfml-mrow rcfml-sub\" style=\"display:none;color:#555\"><td></td><td class=\"txt-r\">{}</td><td></td><td class=\"txt-r\">{}</td><td class=\"txt-r\">{}</td>{}<td style=\"padding-left:22px\">&#8627; {}()</td></tr>\n",
                    grp,
                    fmt_us(m.total),
                    m.count,
                    fmt_us(m_avg),
                    m_alloc,
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
            // The heading counts every statement; rows past maxRecords are
            // clipped from the table but must not vanish from the count.
            &if data.dropped_queries > 0 {
                format!(
                    "Queries ({}, {} shown)",
                    data.queries.len() + data.dropped_queries,
                    data.queries.len()
                )
            } else {
                format!("Queries ({})", data.queries.len())
            },
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
                    "<div>(+{} more queries, {} ms, clipped by maxRecords — still counted in Execution Time and the per-file query column)</div>\n",
                    data.dropped_queries,
                    fmt_us(data.dropped_query_us)
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
            // Lucee shows the detail too, and it is usually where the useful
            // half of a framework error lives (the SQL, the driver message).
            if !e.detail.is_empty() {
                s.push_str(&format!(
                    "<div style=\"color:#900;margin:0 0 4px 1em\"><pre style=\"margin:0;white-space:pre-wrap\">{}</pre></div>\n",
                    esc(&e.detail),
                ));
            }
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
    // while reading request context: one Runtime section holding CFConfig,
    // Environment, Flags and the process memory.
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
            render_runtime(&mut s, cfconfig, memory);
            env_rendered = true;
        }
    }
    if !env_rendered {
        render_runtime(&mut s, cfconfig, memory);
    }

    s.push_str("</div>\n</template></div>\n");
    s.push_str(FOOTER_SCRIPT);
    s
}

/// Runtime: how this engine is running. One collapsed section holding the
/// deploy's cfconfig overrides, the process environment, the flags it was
/// started with and (when the request was metered) the process's memory, each
/// a collapsed block of its own.
fn render_runtime(s: &mut String, cfconfig: &[(String, String)], memory: Option<&MemoryPanel>) {
    collapsible_heading(s, "rcfml-runtime", "Runtime");
    s.push_str("<div class=\"rcfml-runtime\" style=\"display:none;padding-left:16px\">\n");
    render_cfconfig(s, cfconfig);
    render_env_and_flags(s);
    if let Some(m) = memory {
        render_process_memory(s, m);
    }
    s.push_str("</div>\n");
}

/// The process's memory (Runtime > Memory): what the OS sees, and where the
/// live heap is.
fn render_process_memory(s: &mut String, m: &MemoryPanel) {
    let row = |s: &mut String, v: String, label: &str, title: &str| {
        s.push_str(&format!(
            "<tr title=\"{}\"><td class=\"txt-r\">{}</td><td>{}</td></tr>\n",
            title, v, label
        ));
    };
    // The live heap's breakdown, indented under the row it breaks down.
    let breakdown = |s: &mut String| {
        if m.pots.is_empty() {
            return;
        }
        // In the label column, so the value column stays as narrow as its
        // numbers.
        s.push_str("<tr><td style=\"border:0\"></td><td style=\"padding:2px 0 4px 12px;border:0\">\n");
        s.push_str(&format!(
            "<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n<tr><th>est. size</th><th>what</th><th>estimated from the data structures, refreshed at most every 30 s ({} s old)</th></tr>\n",
            m.pots_age_secs
        ));
        for p in &m.pots {
            s.push_str(&format!(
                "<tr><td class=\"txt-r\">{}</td><td>{}</td><td>{}</td></tr>\n",
                fmt_bytes(p.bytes),
                esc(&p.name),
                esc(&p.detail)
            ));
        }
        s.push_str("</table>\n</td></tr>\n");
    };
    if m.footprint.is_some() || m.live_heap.is_some() || !m.pots.is_empty() {
        collapsible_heading(s, "rcfml-memproc", "Memory");
        s.push_str("<div class=\"rcfml-memproc\" style=\"display:none\">\n");
        s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"3\" style=\"border-collapse:collapse\">\n");
        if let Some(f) = m.footprint {
            let lim = match m.limit {
                Some(l) => format!(" of the {} limit", fmt_bytes(l)),
                None => String::new(),
            };
            row(s, fmt_bytes(f), &format!("Footprint{}", lim), "physical memory the OS charges this process: what --max-memory and the OOM killer act on");
        }
        match m.live_heap {
            Some(l) => {
                row(s, fmt_bytes(l), "Live heap", "bytes allocated and not yet freed, across every thread");
                breakdown(s);
                if let Some(f) = m.footprint {
                    row(s, fmt_bytes(f.saturating_sub(l)), "Outside the live heap", "footprint minus live heap: memory the allocator keeps for reuse rather than returning to the OS, thread stacks, and memory C libraries allocate themselves");
                }
            }
            // No live-heap figure (debugging was switched on after startup):
            // the estimates still stand on their own.
            None => breakdown(s),
        }
        s.push_str("</table>\n");
        s.push_str("</div>\n");
    }
}

/// The engine's process environment variables and the flags (CLI arguments)
/// it was started with.
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
    collapsible_heading(s, "rcfml-flags", &format!("Flags ({})", flags.len()));
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
    root.insert("starttime", CfmlValue::Int(0));
    // Times are microseconds (Lucee's unit).
    root.insert("total", CfmlValue::Int(total_us));

    // queries
    let queries: Vec<CfmlValue> = data
        .queries
        .iter()
        .map(|q| {
            let mut m = ValueMap::default();
            m.insert("name", CfmlValue::string(q.name.clone()));
            m.insert("sql", CfmlValue::string(q.sql.clone()));
            m.insert("datasource", CfmlValue::string(q.datasource.clone()));
            m.insert("count", CfmlValue::Int(q.count));
            m.insert("time", CfmlValue::Int(q.time));
            m.insert("cached", CfmlValue::Bool(q.cached));
            m.insert("src", CfmlValue::string(q.src.clone()));
            m.insert("line", CfmlValue::Int(q.line as i64));
            let params: Vec<CfmlValue> = q
                .params
                .iter()
                .map(|p| {
                    let mut pm = ValueMap::default();
                    pm.insert("name", CfmlValue::string(p.name.clone()));
                    pm.insert("value", CfmlValue::string(p.value.clone()));
                    pm.insert("type", CfmlValue::string(p.sqltype.clone()));
                    CfmlValue::strukt(pm)
                })
                .collect();
            m.insert("params", CfmlValue::array(params));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("queries", CfmlValue::array(queries));

    // pages
    let pages: Vec<CfmlValue> = aggregate_pages_with_main(&data.templates, main_page, total_us, 0)
        .into_iter()
        .map(|p| {
            let mut m = ValueMap::default();
            m.insert("id", CfmlValue::string(p.id));
            m.insert("count", CfmlValue::Int(p.count));
            m.insert("min", CfmlValue::Int(p.min));
            m.insert("max", CfmlValue::Int(p.max));
            m.insert("total", CfmlValue::Int(p.total));
            // Per-method breakdown for a CFC row (empty array for a plain
            // template). Each entry: name, count, total (µs).
            let methods: Vec<CfmlValue> = p
                .methods
                .iter()
                .map(|mm| {
                    let mut e = ValueMap::default();
                    e.insert("name", CfmlValue::string(mm.name.clone()));
                    e.insert("count", CfmlValue::Int(mm.count));
                    e.insert("total", CfmlValue::Int(mm.total));
                    CfmlValue::strukt(e)
                })
                .collect();
            m.insert("methods", CfmlValue::array(methods));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("pages", CfmlValue::array(pages));

    // exceptions
    let exceptions: Vec<CfmlValue> = data
        .exceptions
        .iter()
        .map(|e| {
            let mut m = ValueMap::default();
            m.insert("type", CfmlValue::string(e.etype.clone()));
            m.insert("message", CfmlValue::string(e.message.clone()));
            m.insert("detail", CfmlValue::string(e.detail.clone()));
            m.insert("line", CfmlValue::Int(e.line as i64));
            let ctx: Vec<CfmlValue> = e
                .stack
                .iter()
                .map(|(tmpl, line)| {
                    let mut cm = ValueMap::default();
                    cm.insert("template", CfmlValue::string(tmpl.clone()));
                    cm.insert("line", CfmlValue::Int(*line as i64));
                    CfmlValue::strukt(cm)
                })
                .collect();
            m.insert("tagContext", CfmlValue::array(ctx));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("exceptions", CfmlValue::array(exceptions));

    // genericData
    let generic: Vec<CfmlValue> = data
        .generic
        .iter()
        .map(|g| {
            let mut m = ValueMap::default();
            m.insert("category", CfmlValue::string(g.category.clone()));
            m.insert("name", CfmlValue::string(g.name.clone()));
            m.insert("value", CfmlValue::string(g.value.clone()));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("genericData", CfmlValue::array(generic));

    // traces
    let traces: Vec<CfmlValue> = data
        .traces
        .iter()
        .map(|t| {
            let mut m = ValueMap::default();
            m.insert("category", CfmlValue::string(t.category.clone()));
            m.insert("text", CfmlValue::string(t.text.clone()));
            m.insert("type", CfmlValue::string(t.log_type.clone()));
            CfmlValue::strukt(m)
        })
        .collect();
    root.insert("traces", CfmlValue::array(traces));

    // scopes
    let mut scope_struct = ValueMap::default();
    for (name, map) in scopes {
        scope_struct.insert(name.clone(), CfmlValue::strukt(map.clone()));
    }
    root.insert("scopes", CfmlValue::strukt(scope_struct));

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
            alloc_bytes: 0,
        });
        // Two different methods on one CFC — the per-file row must break down
        // into per-method sub-rows rather than showing a bare count of 3.
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("getUser"),
            elapsed_us: 1_000,
            alloc_bytes: 0,
        });
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("GETUSER"),
            elapsed_us: 3_000,
            alloc_bytes: 0,
        });
        c.on_template(&TemplateEvent {
            path: "/services/UserService.cfc",
            method: Some("saveUser"),
            elapsed_us: 500,
            alloc_bytes: 0,
        });
        c.on_error(&ErrorEvent {
            etype: "Custom.Boom",
            message: "kaboom",
            detail: "the detail that explains it",
            src: "/index.cfm",
            line: 30,
            uncaught: true,
            stack: vec![("/index.cfm".into(), 30)],
        });
        c.add_generic("Wheels", "controller", "users");
        c
    }

    #[test]
    fn main_page_row_is_self_time_not_the_request_total() {
        // Four frames totalling 6,500us of self time inside a 10,000us request.
        let hits = vec![
            TemplateHit { path: "/header.cfm".into(), method: String::new(), time: 2_000, alloc: 0 },
            TemplateHit { path: "/svc.cfc".into(), method: "a".into(), time: 1_000, alloc: 0 },
            TemplateHit { path: "/svc.cfc".into(), method: "b".into(), time: 3_000, alloc: 0 },
            TemplateHit { path: "/svc.cfc".into(), method: "c".into(), time: 500, alloc: 0 },
        ];
        let pages = aggregate_pages_with_main(&TemplateAgg::from_hits(&hits), Some("/index.cfm"), 10_000, 0);

        // The requested page reports what it spent in its OWN body, not the
        // request total — booking the total here double-counted every frame
        // below it and pinned the page to the top of the "total ms" sort.
        let main = pages.iter().find(|p| p.id == "/index.cfm").unwrap();
        assert_eq!(main.total, 3_500, "10,000us request - 6,500us of frames");

        // With that, the column is a genuine partition: the rows sum to the
        // request total exactly once.
        assert_eq!(pages.iter().map(|p| p.total).sum::<i64>(), 10_000);

        // Frames still report their own self time, untouched.
        let svc = pages.iter().find(|p| p.id == "/svc.cfc").unwrap();
        assert_eq!(svc.total, 4_500);
        assert_eq!(svc.count, 3);

        // A page that did all its work inside frames reports zero rather than
        // going negative on per-frame microsecond truncation.
        let all_in_frames = aggregate_pages_with_main(&TemplateAgg::from_hits(&hits), Some("/index.cfm"), 6_000, 0);
        assert_eq!(all_in_frames.iter().find(|p| p.id == "/index.cfm").unwrap().total, 0);
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
        assert!(html.contains("Flags ("));
        assert!(html.contains(">Runtime</h4>"));
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
        assert!(html.contains("r.addEventListener('click'"));
        assert!(html.contains("style=\"display:none;color:#555\""));
        assert_eq!(
            html.matches("class=\"rcfml-mtog\"").count(),
            1,
            "only the one file with methods gets a row toggle"
        );
        // plus the expand-all toggle in the column header
        assert_eq!(
            html.matches("data-rcfml-togall=\"rcfml-mrow rcfml-mtog\"")
                .count(),
            1
        );
        assert_eq!(
            html.matches("rcfml-mrow").count(),
            3,
            "2 sub-rows + the expand-all control"
        );
        // queries: SQL/params moved into a collapsed sub-row, one toggle each
        // plus the section-wide one in the header cell
        assert_eq!(html.matches("class=\"rcfml-qtog\"").count(), 2);
        assert_eq!(
            html.matches("data-rcfml-togall=\"rcfml-qrow rcfml-qtog\"")
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
        // the detail is rendered too — it is where a framework error keeps the
        // SQL / driver message that makes it diagnosable
        assert!(html.contains("the detail that explains it"));
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
        // same as the scope dumps — and the whole heading is the click target.
        assert!(html.contains("title=\"show/hide this block\" data-rcfml-tog=\"rcfml-files\""));
        assert!(html.contains("title=\"show/hide this block\" data-rcfml-tog=\"rcfml-queries\""));
        assert!(html.contains("<div class=\"rcfml-files\" style=\"display:none\">"));
        assert!(html.contains("<div class=\"rcfml-queries\" style=\"display:none\">"));
    }

    #[test]
    fn files_table_is_sortable_and_has_no_app_column() {
        let html = sample_collector().render(&[], Some("/index.cfm"), &[]);
        // `app ms` is gone — it was always total − query, and identical to
        // total on the many rows that ran no query.
        assert!(!html.contains("app ms"));
        // Every numeric column heading sorts, in cell-index order, and carries
        // the idle direction indicator.
        for (label, col) in [("total ms", 1), ("query ms", 2), ("count", 3), ("avg ms", 4)] {
            assert!(
                html.contains(&format!(
                    "<th data-rcfml-sort=\"{col}\" title=\"sort by {label}\""
                )),
                "{label} header is not sortable"
            );
        }
        assert_eq!(
            html.matches("class=\"rcfml-ind\"").count(),
            4,
            "one direction indicator per sortable column"
        );
        assert!(html.contains("function sort(th,c)"));
        // Method rows are tagged as sub-rows so a sort drags them along with
        // their file row instead of stranding them under a stranger.
        assert_eq!(html.matches("rcfml-mrow rcfml-sub").count(), 2);
        // Data rows are 6 cells wide, matching the 6 headers.
        let file_row = html
            .split("<tr>")
            .find(|r| r.starts_with("<td style=\"text-align:center;width:1em\">"))
            .expect("a file row")
            .split("</tr>")
            .next()
            .unwrap();
        assert_eq!(file_row.matches("<td").count(), 6, "row: {file_row}");
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
            at("Runtime</h4>"),
            at("CFConfig ("),
            at("Environment variables ("),
            at("Flags ("),
        ];
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "expected URL, FORM, CGI, Runtime, CFConfig, Environment, Flags — got offsets {order:?}"
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
                html.contains(&format!(
                    "title=\"show/hide this block\" data-rcfml-tog=\"{grp}\""
                )),
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
    fn footer_renders_inside_a_shadow_root_with_scoped_styles() {
        let html = sample_collector().render(&[], Some("/index.cfm"), &[]);
        let host = html
            .find("<div class=\"rustcfml-debug-host\"><template shadowrootmode=\"open\">")
            .expect("shadow-root host");
        let style = html.find("<style>:host{all:initial !important").expect("scoped style");
        let panel = html.find("<div class=\"rustcfml-debug\"").expect("panel");
        let close = html.find("</template></div>").expect("template closed");
        let script = html.find("<script>(function(s){").expect("script");
        assert!(host < style && style < panel && panel < close && close < script);
        // The style and the panel are inside the template; the script, which
        // must run, is after it.
        assert!(!html[..host].contains("<style>"));
        // Nothing reaches the page's global scope: no window.* helpers and no
        // inline handlers.
        assert!(!html.contains("window."));
        assert!(!html.contains("onclick"));
    }

    #[test]
    fn memory_panel_renders_only_when_metered() {
        use cfml_common::cycle_gc::{KindCensus, RequestCensus};
        let c = sample_collector();
        let plain = c.render(&[], Some("/index.cfm"), &[]);
        assert!(!plain.contains("Allocated by this request"));
        assert!(!plain.contains("alloc KB"), "no allocation column without metering");

        let panel = MemoryPanel {
            request: cfml_common::mem_account::RequestMemory {
                allocated: 3 * 1024 * 1024,
                freed: 1024 * 1024,
                peak: 2 * 1024 * 1024,
                retained: 2 * 1024 * 1024,
            },
            census: Some(RequestCensus {
                structs: KindCensus { created: 40, alive: 7, alive_bytes: 2048 },
                instances: KindCensus { created: 3, alive: 3, alive_bytes: 900 },
                classes: vec![("models.User".to_string(), KindCensus { created: 3, alive: 3, alive_bytes: 900 })],
                sweeps: 2,
                swept_bytes: 512 * 1024,
                ..Default::default()
            }),
            footprint: Some(200 * 1024 * 1024),
            limit: Some(1024 * 1024 * 1024),
            live_heap: Some(50 * 1024 * 1024),
            pots: vec![MemoryPot { name: "Compiled code".into(), bytes: 4096, detail: "3 files".into() }],
            ..Default::default()
        };
        let html = c.render_with_memory(&[], Some("/index.cfm"), &[], Some(&panel));
        for needle in [
            "<td class=\"txt-r\">3.00 MB</td><td>Allocated by this request</td>",
            "<td class=\"txt-r\">2.00 MB</td><td>Peak in use</td>",
            "Freed so far (512.0 KB of it by 2 collector sweeps)",
            "Objects created (43) <span",
            "&#8627; models.User",
            "Footprint of the 1.00 GB limit",
            "<td class=\"txt-r\">150.00 MB</td><td>Outside the live heap</td>",
            "Compiled code",
            "data-rcfml-sort=\"5\" title=\"sort by alloc KB\"",
        ] {
            assert!(html.contains(needle), "missing {needle:?}");
        }
        // The breakdown sits indented under the Live heap row it breaks
        // down, before Outside the live heap; no separate note under it.
        let live = html.find("<td>Live heap</td>").unwrap();
        let pots = html.find("<td>Compiled code</td>").unwrap();
        let outside = html.find("<td>Outside the live heap</td>").unwrap();
        assert!(live < pots && pots < outside);
        assert!(html.contains("refreshed at most every 30 s"));
        assert!(!html.contains("Breakdown estimated"));
        // The Objects created explanation is a tooltip on its heading, not a
        // paragraph under the table.
        assert!(html.contains("title=\"Alive = still referenced as the page ends;"));
        assert!(!html.contains("\">Alive = still referenced"));
        // The panel sits between Execution Time and Files.
        let mem = html.find("Allocated by this request").unwrap();
        assert!(html.find("Execution Time").unwrap() < mem);
        assert!(mem < html.find("Files (Templates/Tags/CFCs)").unwrap());
    }

    #[test]
    fn main_page_allocation_is_the_residual_of_the_frames() {
        let hits = vec![
            TemplateHit { path: "/a.cfm".into(), method: String::new(), time: 10, alloc: 300 },
            TemplateHit { path: "/svc.cfc".into(), method: "go".into(), time: 10, alloc: 200 },
        ];
        let pages = aggregate_pages_with_main(&TemplateAgg::from_hits(&hits), Some("/index.cfm"), 100, 1000);
        assert_eq!(pages[0].alloc, 500, "1000 allocated, 500 of it in frames");
        assert_eq!(pages[2].methods[0].alloc, 200);
    }

    #[test]
    fn footer_goes_before_the_closing_body_tag() {
        let mut page = "<html><body><p>hi</p></BODY>\n</html>".to_string();
        insert_footer(&mut page, "[F]");
        assert_eq!(page, "<html><body><p>hi</p>[F]</BODY>\n</html>");
        // Only the last one counts (an earlier "</body>" can be inside a
        // string or a code sample).
        let mut page = "<pre>&lt;/body&gt; </body></pre></body>".to_string();
        insert_footer(&mut page, "[F]");
        assert_eq!(page, "<pre>&lt;/body&gt; </body></pre>[F]</body>");
        let mut page = "no body tag".to_string();
        insert_footer(&mut page, "[F]");
        assert_eq!(page, "no body tag[F]");
        let mut page = String::new();
        insert_footer(&mut page, "[F]");
        assert_eq!(page, "[F]");
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
        // The heading counts every statement run, not just the rows shown.
        assert!(html.contains("Queries (5, 2 shown)"));
        assert!(html.contains("+3 more queries, 9.000 ms, clipped"));
        // The clip trims the display only — Execution Time still counts all 5
        // queries: kept 0+1 ms plus dropped 2+3+4 ms = 10 ms total.
        assert!(
            html.contains("<tr><td class=\"txt-r\">10.000 ms</td><td>Query</td></tr>"),
            "Query total must include clipped queries' time"
        );
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
