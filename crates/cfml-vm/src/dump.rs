//! `writeDump` / `<cfdump>` rendering.
//!
//! In a web request (serve mode) we emit a self-contained HTML widget — a
//! collapsible, table-based view of the value with inline CSS + JS and a
//! Rust-inspired colour scheme (rust-orange accents on a warm cream ground).
//! The CSS/JS preamble is emitted only once per request (the VM tracks this
//! via `dump_assets_emitted`) so multiple dumps on a page share one stylesheet
//! and one toggle function.
//!
//! Outside a web request (CLI) we emit a plain indented text tree — HTML tags
//! would just be noise on a terminal.

use cfml_common::dynamic::{CfmlAccess, CfmlValue};

/// Options parsed from `writeDump(var, label=, expand=, top=)`.
pub struct DumpOptions {
    pub label: Option<String>,
    pub expand: bool,
    /// Maximum depth of nested containers to render (None = unlimited).
    pub top: Option<usize>,
}

impl Default for DumpOptions {
    fn default() -> Self {
        DumpOptions { label: None, expand: true, top: None }
    }
}

/// The one-time CSS + JS preamble. Class names are namespaced under `rcf-dump`
/// so they can't collide with the page's own styles.
const DUMP_ASSETS: &str = r#"<style>
.rcf-dump{
  /* Rust-inspired palette */
  --rcf-ink:#2b2018; --rcf-bg:#fffdf9; --rcf-cell:#fffdf9; --rcf-line:#e9d3c4;
  --rcf-rust:#b7410e; --rcf-rust-lo:#d14e2c; --rcf-key-bg:#faece3; --rcf-key:#8a2a12;
  --rcf-meta:#fff8f2; --rcf-lbl-bg:#3a2a20; --rcf-lbl:#ffd9a8;
  --rcf-str:#2b6e1f; --rcf-num:#0b62b5; --rcf-bool:#b7410e; --rcf-null:#9a8f86; --rcf-fn:#7a4ec2;
  font-family:"SFMono-Regular",ui-monospace,Menlo,Consolas,monospace;font-size:12px;line-height:1.45;
  color:var(--rcf-ink);margin:6px 0;border:1px solid var(--rcf-accent,var(--rcf-rust));border-radius:6px;
  overflow:hidden;display:inline-block;min-width:120px;max-width:100%;
  box-shadow:0 1px 3px rgba(120,40,10,.18);vertical-align:top}
.rcf-dump *{box-sizing:border-box}
/* Per-type accent — a warm rust family with one cool tone for queries */
.rcf-dump.k-struct{--rcf-accent:#b7410e;--rcf-accent-lo:#d14e2c}
.rcf-dump.k-array {--rcf-accent:#c4651a;--rcf-accent-lo:#dd7e2b}
.rcf-dump.k-query {--rcf-accent:#4e6e81;--rcf-accent-lo:#5f8197}
.rcf-dump.k-comp  {--rcf-accent:#8a2b12;--rcf-accent-lo:#a8350f}
.rcf-dump.k-fn    {--rcf-accent:#7a4ec2;--rcf-accent-lo:#8d63d1}
.rcf-dump .rcf-h{background:linear-gradient(var(--rcf-accent-lo,var(--rcf-rust-lo)),var(--rcf-accent,var(--rcf-rust)));
  color:var(--rcf-meta);font-weight:700;padding:4px 10px;cursor:pointer;user-select:none;letter-spacing:.02em;
  display:flex;align-items:center;gap:6px}
.rcf-dump .rcf-h:hover{filter:brightness(1.06)}
.rcf-dump .rcf-h .rcf-tw{font-size:10px;width:9px;display:inline-block;transition:transform .15s;opacity:.85}
.rcf-dump.rcf-c>.rcf-h .rcf-tw{transform:rotate(-90deg)}
.rcf-dump .rcf-meta{font-weight:400;opacity:.85;font-size:11px}
.rcf-dump .rcf-lbl{font-weight:700;background:var(--rcf-lbl-bg);color:var(--rcf-lbl);padding:3px 10px;font-size:11px}
.rcf-dump table{border-collapse:collapse;width:100%;background:var(--rcf-bg)}
.rcf-dump.rcf-c>table,.rcf-dump.rcf-c>.rcf-empty{display:none}
.rcf-dump td,.rcf-dump th{border:1px solid var(--rcf-line);padding:3px 8px;text-align:left;vertical-align:top}
.rcf-dump th{background:var(--rcf-key-bg);color:var(--rcf-key);font-weight:700}
.rcf-dump td.rcf-k{background:var(--rcf-key-bg);color:var(--rcf-key);font-weight:700;white-space:nowrap;width:1%}
.rcf-dump td.rcf-i{background:var(--rcf-key-bg);color:var(--rcf-accent,var(--rcf-rust));font-weight:700;text-align:right;white-space:nowrap;width:1%}
.rcf-dump .rcf-v-str{color:var(--rcf-str);white-space:pre-wrap;word-break:break-word}
.rcf-dump .rcf-v-num{color:var(--rcf-num)}
.rcf-dump .rcf-v-bool{color:var(--rcf-bool);font-weight:700}
.rcf-dump .rcf-v-null{color:var(--rcf-null);font-style:italic}
.rcf-dump .rcf-v-fn{color:var(--rcf-fn)}
.rcf-dump .rcf-empty{padding:4px 10px;color:var(--rcf-null);font-style:italic;background:var(--rcf-bg)}
.rcf-dump .rcf-qmeta{padding:3px 10px;background:var(--rcf-key-bg);color:var(--rcf-key);font-size:11px;border-top:1px solid var(--rcf-line)}
.rcf-dump .rcf-qsql{margin-top:2px;color:var(--rcf-ink);opacity:.8;white-space:pre-wrap;word-break:break-word}
@media (prefers-color-scheme:dark){.rcf-dump{
  --rcf-ink:#e8ddd3;--rcf-bg:#231a14;--rcf-cell:#231a14;--rcf-line:#4a382c;
  --rcf-key-bg:#33261d;--rcf-key:#f0a878;--rcf-str:#8fd07a;--rcf-num:#67b0e8;--rcf-null:#8a7b6f;--rcf-fn:#b596e8}}
</style>
<script>
function rcfDt(h){h.parentNode.classList.toggle('rcf-c');}
</script>
"#;

/// Render `value` for `writeDump`. When `web` is true, returns an HTML widget;
/// otherwise a plain indented text tree. `include_assets` (web only) prepends
/// the CSS/JS preamble — pass true the first time per request.
pub fn render(value: &CfmlValue, opts: &DumpOptions, web: bool, include_assets: bool) -> String {
    if !web {
        let mut out = String::new();
        if let Some(ref l) = opts.label {
            out.push_str(l);
            out.push('\n');
        }
        render_text(value, 0, &mut out, &mut Vec::new());
        out.push('\n');
        return out;
    }

    let mut out = String::new();
    if include_assets {
        out.push_str(DUMP_ASSETS);
    }
    if let Some(ref l) = opts.label {
        // A labelled dump wraps the value box so the label sits above it.
        out.push_str("<div class=\"rcf-dump\"><div class=\"rcf-lbl\">");
        out.push_str(&esc(l));
        out.push_str("</div>");
        render_html(value, opts, 0, &mut out, &mut Vec::new());
        out.push_str("</div>");
    } else {
        render_html(value, opts, 0, &mut out, &mut Vec::new());
    }
    out
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

fn render_html(
    value: &CfmlValue,
    opts: &DumpOptions,
    depth: usize,
    out: &mut String,
    visited: &mut Vec<usize>,
) {
    match value {
        // Simple scalars render inline (no collapsible box).
        CfmlValue::Null
        | CfmlValue::Bool(_)
        | CfmlValue::Int(_)
        | CfmlValue::Double(_)
        | CfmlValue::TimeSpan(_)
        | CfmlValue::String(_)
        | CfmlValue::Binary(_) => {
            out.push_str("<div class=\"rcf-dump\"><table><tr><td>");
            out.push_str(&scalar_html(value));
            out.push_str("</td></tr></table></div>");
        }
        CfmlValue::Function(f) => {
            box_open(out, "k-fn", &format!("Function {}", esc(&f.name)), &fn_signature(f), opts.expand);
            out.push_str("<table><tr><td><span class=\"rcf-v-fn\">");
            out.push_str(&esc(&fn_signature_full(f)));
            out.push_str("</span></td></tr></table></div>");
        }
        CfmlValue::Closure(_) => {
            out.push_str("<div class=\"rcf-dump\"><div class=\"rcf-h\"><span class=\"rcf-tw\">\u{25be}</span>Closure</div><table><tr><td><span class=\"rcf-v-fn\">[closure]</span></td></tr></table></div>");
        }
        CfmlValue::NativeObject(o) => {
            let cls = o.read().map(|g| g.class_name().to_string()).unwrap_or_else(|_| "native".into());
            box_open(out, "k-comp", &format!("Native {}", esc(&cls)), "", opts.expand);
            out.push_str("<table><tr><td><span class=\"rcf-v-fn\">[native object]</span></td></tr></table></div>");
        }
        CfmlValue::Array(a) => {
            let ptr = a.backing_ptr();
            let items = a.snapshot();
            box_open(out, "k-array", "Array", &items.len().to_string(), opts.expand);
            if recursion_guard(out, visited, ptr) { return; }
            if depth_exceeded(out, opts, depth) || items.is_empty() {
                if items.is_empty() { out.push_str("<div class=\"rcf-empty\">[empty]</div>"); }
                close_box(out, visited, ptr);
                return;
            }
            out.push_str("<table>");
            for (i, item) in items.iter().enumerate() {
                out.push_str("<tr><td class=\"rcf-i\">");
                out.push_str(&(i + 1).to_string());
                out.push_str("</td><td>");
                render_child(item, opts, depth, out, visited);
                out.push_str("</td></tr>");
            }
            out.push_str("</table>");
            close_box(out, visited, ptr);
        }
        CfmlValue::QueryColumn(col, _) => {
            box_open(out, "k-array", "Array", &col.len().to_string(), opts.expand);
            out.push_str("<table>");
            for (i, item) in col.iter().enumerate() {
                out.push_str("<tr><td class=\"rcf-i\">");
                out.push_str(&(i + 1).to_string());
                out.push_str("</td><td>");
                render_child(item, opts, depth, out, visited);
                out.push_str("</td></tr>");
            }
            out.push_str("</table></div>");
        }
        CfmlValue::Struct(s) => {
            let ptr = s.backing_ptr();
            let snap = s.snapshot();
            // A Java shim object (e.g. java.util.Date) is a struct carrying
            // `__java_shim`/`__java_class` markers plus `__`-prefixed data —
            // render it as a "Java <class>" box with cleaned-up field names
            // instead of leaking the internal markers as a plain struct.
            if let Some((class, entries)) = java_shim_view(&snap) {
                box_open(out, "k-comp", &format!("Java {}", esc(&class)), &entries.len().to_string(), opts.expand);
                if entries.is_empty() {
                    out.push_str("<div class=\"rcf-empty\">[no fields]</div></div>");
                    return;
                }
                out.push_str("<table>");
                for (k, v) in &entries {
                    out.push_str("<tr><td class=\"rcf-k\">");
                    out.push_str(&esc(k));
                    out.push_str("</td><td>");
                    render_child(v, opts, depth, out, visited);
                    out.push_str("</td></tr>");
                }
                out.push_str("</table></div>");
                return;
            }
            let component = component_view(&snap);
            if let Some((name, entries)) = component {
                box_open(out, "k-comp", &format!("Component {}", esc(&name)), &entries.len().to_string(), opts.expand);
                if recursion_guard(out, visited, ptr) { return; }
                if depth_exceeded(out, opts, depth) || entries.is_empty() {
                    if entries.is_empty() { out.push_str("<div class=\"rcf-empty\">[no public members]</div>"); }
                    close_box(out, visited, ptr);
                    return;
                }
                out.push_str("<table>");
                for (k, v) in &entries {
                    out.push_str("<tr><td class=\"rcf-k\">");
                    out.push_str(&esc(k));
                    out.push_str("</td><td>");
                    render_child(v, opts, depth, out, visited);
                    out.push_str("</td></tr>");
                }
                out.push_str("</table>");
                close_box(out, visited, ptr);
                return;
            }
            box_open(out, "k-struct", "Struct", &snap.len().to_string(), opts.expand);
            if recursion_guard(out, visited, ptr) { return; }
            if depth_exceeded(out, opts, depth) || snap.is_empty() {
                if snap.is_empty() { out.push_str("<div class=\"rcf-empty\">[empty]</div>"); }
                close_box(out, visited, ptr);
                return;
            }
            out.push_str("<table>");
            for (k, v) in snap.iter() {
                out.push_str("<tr><td class=\"rcf-k\">");
                out.push_str(&esc(k));
                out.push_str("</td><td>");
                render_child(v, opts, depth, out, visited);
                out.push_str("</td></tr>");
            }
            out.push_str("</table>");
            close_box(out, visited, ptr);
        }
        CfmlValue::Component(c) => {
            box_open(out, "k-comp", &format!("Component {}", esc(&c.name)), &c.properties.len().to_string(), opts.expand);
            if depth_exceeded(out, opts, depth) || c.properties.is_empty() {
                if c.properties.is_empty() { out.push_str("<div class=\"rcf-empty\">[no properties]</div>"); }
                out.push_str("</div>");
                return;
            }
            out.push_str("<table>");
            for (k, v) in c.properties.iter() {
                out.push_str("<tr><td class=\"rcf-k\">");
                out.push_str(&esc(k));
                out.push_str("</td><td>");
                render_child(v, opts, depth, out, visited);
                out.push_str("</td></tr>");
            }
            out.push_str("</table></div>");
        }
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => {
            // Render a flyweight instance as a Component box from its public
            // `this` data — the marker-struct component renders the same way via
            // the `component_view` branch in the Struct arm above.
            let g = inst.read();
            let snap = g.this_members.snapshot();
            box_open(out, "k-comp", &format!("Component {}", esc(&g.class.name)), &snap.len().to_string(), opts.expand);
            if depth_exceeded(out, opts, depth) || snap.is_empty() {
                if snap.is_empty() { out.push_str("<div class=\"rcf-empty\">[no public members]</div>"); }
                out.push_str("</div>");
                return;
            }
            out.push_str("<table>");
            for (k, v) in snap.iter() {
                out.push_str("<tr><td class=\"rcf-k\">");
                out.push_str(&esc(k));
                out.push_str("</td><td>");
                render_child(v, opts, depth, out, visited);
                out.push_str("</td></tr>");
            }
            out.push_str("</table></div>");
        }
        CfmlValue::Query(q) => {
            let data = q.with_read(|d| d.clone());
            let rows = data.row_count();
            box_open(out, "k-query", "Query", &format!("{} rows \u{00d7} {} cols", rows, data.columns.len()), opts.expand);
            if depth_exceeded(out, opts, depth) {
                out.push_str("</div>");
                return;
            }
            if rows == 0 {
                out.push_str("<div class=\"rcf-empty\">[no rows]</div>");
                // still show the header columns
            }
            out.push_str("<table><tr><th>#</th>");
            for col in &data.columns {
                out.push_str("<th>");
                out.push_str(&esc(col));
                out.push_str("</th>");
            }
            out.push_str("</tr>");
            for r in 0..rows {
                out.push_str("<tr><td class=\"rcf-i\">");
                out.push_str(&(r + 1).to_string());
                out.push_str("</td>");
                for ci in 0..data.columns.len() {
                    out.push_str("<td>");
                    if let Some(cell) = data.cell(r, ci) {
                        render_child(cell, opts, depth, out, visited);
                    }
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
            // Metadata footer (Lucee metainfo parity): record count, execution
            // time, and originating SQL when available.
            out.push_str("<div class=\"rcf-qmeta\">");
            out.push_str(&format!("Records: {}", rows));
            if let Some(ms) = data.execution_time {
                out.push_str(&format!(" \u{2022} Execution time: {} ms", ms));
            }
            if let Some(ref sql) = data.sql {
                let sql = sql.trim();
                if !sql.is_empty() {
                    out.push_str("<div class=\"rcf-qsql\">");
                    out.push_str(&esc(sql));
                    out.push_str("</div>");
                }
            }
            out.push_str("</div></div>");
        }
    }
}

/// Render a value that sits inside a parent table cell: scalars inline,
/// containers as nested boxes.
fn render_child(
    value: &CfmlValue,
    opts: &DumpOptions,
    depth: usize,
    out: &mut String,
    visited: &mut Vec<usize>,
) {
    match value {
        CfmlValue::Null
        | CfmlValue::Bool(_)
        | CfmlValue::Int(_)
        | CfmlValue::Double(_)
        | CfmlValue::String(_)
        | CfmlValue::Binary(_) => out.push_str(&scalar_html(value)),
        _ => render_html(value, opts, depth + 1, out, visited),
    }
}

/// Open a collapsible box: `<div class="rcf-dump k-KIND[ rcf-c]"><div class=rcf-h>…`.
/// Caller is responsible for the matching `</div>` (via `close_box` or inline).
fn box_open(out: &mut String, kind: &str, title: &str, meta: &str, expand: bool) {
    out.push_str("<div class=\"rcf-dump ");
    out.push_str(kind);
    if !expand {
        out.push_str(" rcf-c");
    }
    out.push_str("\"><div class=\"rcf-h\" onclick=\"rcfDt(this)\"><span class=\"rcf-tw\">\u{25be}</span>");
    out.push_str(title);
    if !meta.is_empty() {
        out.push_str(" <span class=\"rcf-meta\">(");
        out.push_str(meta);
        out.push_str(")</span>");
    }
    out.push_str("</div>");
}

/// Close a container box and pop the recursion-guard pointer.
fn close_box(out: &mut String, visited: &mut Vec<usize>, ptr: usize) {
    out.push_str("</div>");
    if let Some(pos) = visited.iter().rposition(|p| *p == ptr) {
        visited.remove(pos);
    }
}

/// Push `ptr` onto the visited stack; if already present, emit a recursive
/// marker, close the box, and return true (caller should stop).
fn recursion_guard(out: &mut String, visited: &mut Vec<usize>, ptr: usize) -> bool {
    if visited.contains(&ptr) {
        out.push_str("<div class=\"rcf-empty\">[recursive reference]</div></div>");
        return true;
    }
    visited.push(ptr);
    false
}

/// True (and emits a `…` marker) when `top` depth has been reached.
fn depth_exceeded(out: &mut String, opts: &DumpOptions, depth: usize) -> bool {
    if let Some(top) = opts.top {
        if depth >= top {
            out.push_str("<div class=\"rcf-empty\">\u{2026}</div>");
            return true;
        }
    }
    false
}

fn scalar_html(value: &CfmlValue) -> String {
    match value {
        CfmlValue::Null => "<span class=\"rcf-v-null\">[null]</span>".to_string(),
        CfmlValue::Bool(b) => format!("<span class=\"rcf-v-bool\">{}</span>", b),
        CfmlValue::Int(i) => format!("<span class=\"rcf-v-num\">{}</span>", i),
        CfmlValue::Double(d) => format!("<span class=\"rcf-v-num\">{}</span>", fmt_double(*d)),
        CfmlValue::String(s) => {
            if s.is_empty() {
                "<span class=\"rcf-v-null\">[empty string]</span>".to_string()
            } else {
                format!("<span class=\"rcf-v-str\">{}</span>", esc(s))
            }
        }
        CfmlValue::Binary(b) => {
            format!("<span class=\"rcf-v-null\">[binary, {} bytes]</span>", b.len())
        }
        _ => esc(&value_string(value)),
    }
}

// ---------------------------------------------------------------------------
// Plain-text rendering (CLI)
// ---------------------------------------------------------------------------

fn render_text(value: &CfmlValue, indent: usize, out: &mut String, visited: &mut Vec<usize>) {
    let pad = "  ".repeat(indent);
    match value {
        CfmlValue::Array(a) => {
            let ptr = a.backing_ptr();
            if visited.contains(&ptr) {
                out.push_str("[recursive array]\n");
                return;
            }
            visited.push(ptr);
            let items = a.snapshot();
            out.push_str(&format!("Array ({})\n", items.len()));
            for (i, item) in items.iter().enumerate() {
                out.push_str(&format!("{}  [{}] ", pad, i + 1));
                render_text_child(item, indent + 1, out, visited);
            }
            visited.pop();
        }
        CfmlValue::Struct(s) => {
            let ptr = s.backing_ptr();
            if visited.contains(&ptr) {
                out.push_str("[recursive struct]\n");
                return;
            }
            visited.push(ptr);
            let snap = s.snapshot();
            let (label, entries): (String, Vec<(String, CfmlValue)>) =
                if let Some((class, e)) = java_shim_view(&snap) {
                    (format!("Java {}", class), e)
                } else if let Some((name, e)) = component_view(&snap) {
                    (format!("Component {}", name), e)
                } else {
                    ("Struct".into(), snap.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                };
            out.push_str(&format!("{} ({})\n", label, entries.len()));
            for (k, v) in &entries {
                out.push_str(&format!("{}  {} = ", pad, k));
                render_text_child(v, indent + 1, out, visited);
            }
            visited.pop();
        }
        CfmlValue::Query(q) => {
            let data = q.with_read(|d| d.clone());
            let timing = data.execution_time.map(|ms| format!(", {} ms", ms)).unwrap_or_default();
            out.push_str(&format!("Query ({} rows x {} cols{}) [{}]\n", data.row_count(), data.columns.len(), timing, data.columns.join(", ")));
            for r in 0..data.row_count() {
                out.push_str(&format!("{}  row {}: ", pad, r + 1));
                let cells: Vec<String> = (0..data.columns.len())
                    .map(|ci| format!("{}={}", data.columns[ci], data.cell(r, ci).map(value_string).unwrap_or_default()))
                    .collect();
                out.push_str(&cells.join(", "));
                out.push('\n');
            }
            if let Some(ref sql) = data.sql {
                let sql = sql.trim();
                if !sql.is_empty() {
                    out.push_str(&format!("{}  SQL: {}\n", pad, sql));
                }
            }
        }
        _ => {
            out.push_str(&value_string(value));
            out.push('\n');
        }
    }
}

fn render_text_child(value: &CfmlValue, indent: usize, out: &mut String, visited: &mut Vec<usize>) {
    match value {
        CfmlValue::Array(_) | CfmlValue::Struct(_) | CfmlValue::Query(_) | CfmlValue::Component(_) => {
            render_text(value, indent, out, visited)
        }
        _ => {
            out.push_str(&value_string(value));
            out.push('\n');
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A CFC instance is materialised as a struct carrying `__variables`/`__name`
/// with its public members (data + methods) stored at the TOP LEVEL alongside
/// engine-internal `__*` markers and a self-referential `this`. Surface the
/// public members in declaration order, minus the markers. Returns the
/// component's name and its public entries, or None if not a component struct.
fn component_view(snap: &cfml_common::dynamic::ValueMap) -> Option<(String, Vec<(String, CfmlValue)>)> {
    let has_vars = snap.keys().any(|k| k.eq_ignore_ascii_case("__variables"));
    let has_name = snap.keys().any(|k| k.eq_ignore_ascii_case("__name"));
    if !has_vars || !has_name {
        return None;
    }
    let name = snap
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("__name"))
        .map(|(_, v)| value_string(v))
        .unwrap_or_default();
    let mut entries = Vec::new();
    for (k, v) in snap.iter() {
        if k.starts_with("__") || k.eq_ignore_ascii_case("this") {
            continue;
        }
        entries.push((k.clone(), v.clone()));
    }
    Some((name, entries))
}

/// A Java shim object is a struct flagged with `__java_shim`. Surface its
/// class name and its data fields with the engine-internal `__` prefix
/// stripped (e.g. `__millis` → `millis`), dropping the `__java_shim` /
/// `__java_class` markers themselves. Returns None if not a Java shim.
fn java_shim_view(snap: &cfml_common::dynamic::ValueMap) -> Option<(String, Vec<(String, CfmlValue)>)> {
    let is_shim = snap
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("__java_shim") && v.is_true());
    if !is_shim {
        return None;
    }
    let class = snap
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("__java_class"))
        .map(|(_, v)| value_string(v))
        .unwrap_or_else(|| "object".to_string());
    let mut entries = Vec::new();
    for (k, v) in snap.iter() {
        if k.eq_ignore_ascii_case("__java_shim") || k.eq_ignore_ascii_case("__java_class") {
            continue;
        }
        let field = k.trim_start_matches('_');
        entries.push((field.to_string(), v.clone()));
    }
    Some((class, entries))
}

fn fn_signature(f: &cfml_common::dynamic::CfmlFunction) -> String {
    let params: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
    params.join(", ")
}

fn fn_signature_full(f: &cfml_common::dynamic::CfmlFunction) -> String {
    let access = match f.access {
        CfmlAccess::Public => "public",
        CfmlAccess::Private => "private",
        CfmlAccess::Package => "package",
        CfmlAccess::Remote => "remote",
    };
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match &p.param_type {
            Some(t) => format!("{} {}", t, p.name),
            None => p.name.clone(),
        })
        .collect();
    let ret = f.return_type.as_deref().unwrap_or("any");
    format!("{} {} function {}({})", access, ret, f.name, params.join(", "))
}

fn fmt_double(d: f64) -> String {
    if d == d.trunc() && d.is_finite() && d.abs() < 1e15 {
        format!("{}", d as i64)
    } else {
        format!("{}", d)
    }
}

/// Plain string form of a value for text contexts / fallbacks.
fn value_string(value: &CfmlValue) -> String {
    match value {
        CfmlValue::Null => "[null]".to_string(),
        CfmlValue::Bool(b) => b.to_string(),
        CfmlValue::Int(i) => i.to_string(),
        CfmlValue::Double(d) => fmt_double(*d),
        CfmlValue::TimeSpan(d) => fmt_double(*d),
        CfmlValue::String(s) => {
            if s.is_empty() { "[empty string]".to_string() } else { s.to_string() }
        }
        CfmlValue::Binary(b) => format!("[binary, {} bytes]", b.len()),
        CfmlValue::Function(f) => format!("[function {}]", f.name),
        CfmlValue::Closure(_) => "[closure]".to_string(),
        CfmlValue::NativeObject(o) => {
            let cls = o.read().map(|g| g.class_name().to_string()).unwrap_or_else(|_| "native".into());
            format!("[native {}]", cls)
        }
        _ => "[complex]".to_string(),
    }
}

/// HTML-escape text for safe embedding in element content / attributes.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // Tab / newline / carriage return are legitimate formatting in a
            // `white-space:pre-wrap` cell — keep them.
            '\t' | '\n' | '\r' => out.push(c),
            // Any other control character (C0 0x00–0x1F, DEL 0x7F, C1
            // 0x80–0x9F) must NOT reach the HTML raw: browsers render them as
            // garbage/invisible bytes, which is exactly the "leaked bytes"
            // corruption when dumping scopes (application/session) that hold
            // strings with embedded binary — session tokens, CSRF secrets,
            // cached blobs, encryption artifacts. Show a visible, unambiguous
            // escape instead. (Only the dump display is sanitized;
            // writeOutput() of the same string is unaffected.)
            _ if c.is_control() => {
                let cp = c as u32;
                if cp <= 0xFF {
                    out.push_str(&format!("\\x{:02X}", cp));
                } else {
                    out.push_str(&format!("\\u{{{:04X}}}", cp));
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::CfmlValue;

    // Control/non-printable bytes must never reach the HTML raw — they render as
    // invisible garbage in the browser and (worse) can break HTML nesting. They
    // are shown as visible \xNN / \u{XXXX} escapes instead. Tab/newline/CR are
    // legitimate formatting and pass through.
    #[test]
    fn esc_escapes_control_chars_but_keeps_whitespace() {
        let s = format!("abc{}{}{}def{}", '\u{7}', '\u{8}', '\u{1b}', '\u{0}');
        let out = esc(&s);
        assert_eq!(out, "abc\\x07\\x08\\x1Bdef\\x00");
        // HTML metachars still escaped
        assert_eq!(esc("<a>&\"'"), "&lt;a&gt;&amp;&quot;&#39;");
        // Legitimate whitespace preserved
        assert_eq!(esc("a\tb\nc\rd"), "a\tb\nc\rd");
        // C1 control (0x80–0x9F) escaped too
        assert_eq!(esc("x\u{85}y"), "x\\x85y");
    }

    #[test]
    fn scalar_html_string_with_control_bytes_has_no_raw_controls() {
        let v = CfmlValue::string(format!("tok{}{}", '\u{1}', '\u{7f}'));
        let html = scalar_html(&v);
        assert!(!html.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')),
            "rendered dump HTML must contain no raw control bytes: {:?}", html);
        assert!(html.contains("\\x01") && html.contains("\\x7F"));
    }
}
