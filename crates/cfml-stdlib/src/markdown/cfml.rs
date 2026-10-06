//! CFML values ↔ the markdown tree.
//!
//! **The mixing rule.** A string is markdown. `{ text = value }` is literal
//! text. A struct with a `type` is a tree node. An array is a sequence of any
//! of these. What a string is parsed *as* depends on the slot it lands in: a
//! block slot (any blocks) takes block markdown, an inline slot (one specific
//! block's content) takes inline markdown only — see [`super::inline`].
//!
//! The same rule applies to `MarkdownDocument( struct )`: a `children` array
//! may hold markdown strings, parsed by slot. `toStruct()` always returns
//! canonical nodes; strings are an input convenience and are never stored.

use std::collections::HashSet;

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;

use super::inline::parse_inline;
use super::options::{md_err, MdOptions};
use super::parse::parse_unnumbered;
use super::tree::{normalise_inlines, Align, Content, Doc, Kind, Node};

// ---------------------------------------------------------------------------
// struct field access
// ---------------------------------------------------------------------------

/// A struct's entries with "was this key read" bookkeeping, so whatever the
/// loader did not understand can be preserved as extra fields.
struct Fields {
    entries: Vec<(String, CfmlValue)>,
    used: Vec<bool>,
}

impl Fields {
    fn new(map: &ValueMap) -> Fields {
        let entries: Vec<(String, CfmlValue)> =
            map.iter().map(|(k, v)| (k.as_str().to_string(), v.clone())).collect();
        let used = vec![false; entries.len()];
        Fields { entries, used }
    }

    /// The first present (non-null) value among `names`, marking every
    /// spelling as read.
    fn get(&mut self, names: &[&str]) -> Option<CfmlValue> {
        let mut found = None;
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if names.iter().any(|n| k.eq_ignore_ascii_case(n)) {
                self.used[i] = true;
                if found.is_none() && !matches!(v, CfmlValue::Null) {
                    found = Some(v.clone());
                }
            }
        }
        found
    }

    fn rest(self) -> Vec<(String, CfmlValue)> {
        self.entries
            .into_iter()
            .zip(self.used)
            .filter(|((_, v), used)| !used && !matches!(v, CfmlValue::Null))
            .map(|(e, _)| e)
            .collect()
    }

    fn unused_keys(&self) -> Vec<String> {
        self.entries
            .iter()
            .zip(&self.used)
            .filter(|((_, v), used)| !**used && !matches!(v, CfmlValue::Null))
            .map(|((k, _), _)| k.clone())
            .collect()
    }
}

fn as_u64(v: &CfmlValue, what: &str) -> Result<u64, CfmlError> {
    let s = v.as_string();
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as u64)
        .ok_or_else(|| md_err(format!("[{}] must be a whole number, not [{}].", what, s)))
}

fn opt_string(v: Option<CfmlValue>) -> Option<String> {
    v.map(|v| v.as_string()).filter(|s| !s.is_empty())
}

/// A list given as an array or a comma-delimited string.
pub fn list_values(v: &CfmlValue) -> Vec<CfmlValue> {
    match v {
        CfmlValue::Array(a) => a.snapshot(),
        CfmlValue::Null => Vec::new(),
        other => other.as_string().split(',').map(|s| CfmlValue::string(s.trim())).collect(),
    }
}

pub fn parse_align_list(v: &CfmlValue) -> Result<Vec<Align>, CfmlError> {
    list_values(v)
        .iter()
        .map(|a| {
            if matches!(a, CfmlValue::Null) {
                return Ok(Align::None);
            }
            let s = a.as_string();
            Align::parse(&s).ok_or_else(|| {
                md_err(format!("Table alignment [{}] is not one of left, center, right or none.", s))
            })
        })
        .collect()
}

fn type_label(v: &CfmlValue) -> &'static str {
    match v {
        CfmlValue::NativeObject(_) => "object",
        other => other.type_name(),
    }
}

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

/// Builds tree nodes from CFML values for one document. Nodes come back
/// without ids (unless the caller supplied them); the document mints ids when
/// it grafts them in. Footnote definitions created along the way go straight
/// into the document.
pub struct Loader<'d> {
    pub doc: &'d mut Doc,
    pub base: MdOptions,
}

const RUN_KEYS: &str = "text, markdown, bold/strong, italic/emphasis, strike/strikethrough, code, \
                        link, title, image, alt, linebreak/break, footnote";

impl<'d> Loader<'d> {
    pub fn new(doc: &'d mut Doc, base: &MdOptions) -> Loader<'d> {
        Loader { doc, base: base.clone() }
    }

    fn content_opts(&self) -> MdOptions {
        let mut o = self.base.clone();
        o.gfm = self.doc.gfm;
        o.footnotes = self.doc.footnotes;
        o.front_matter = false;
        o
    }

    /// Content for a container whose children are `slot`.
    pub fn content(&mut self, slot: Content, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        match slot {
            Content::Flow => self.blocks(v),
            Content::Phrasing => self.inlines(v),
            Content::ListItems => self.items(v),
            Content::TableRows => self.rows(v),
            Content::TableCells => self.cells(v),
            Content::None => Err(md_err("That node can't hold content.")),
        }
    }

    /// Parse a string of block markdown into nodes, moving any footnote
    /// definitions it carries into the document.
    pub fn block_markdown(&mut self, src: &str) -> Result<Vec<Node>, CfmlError> {
        let mut o = self.content_opts();
        o.dedent = true;
        let parsed = parse_unnumbered(src, &o)?;
        for d in parsed.definitions {
            self.add_definition(d)?;
        }
        Ok(parsed.children)
    }

    pub fn add_definition(&mut self, d: Node) -> Result<(), CfmlError> {
        if let Kind::FootnoteDefinition { identifier } = &d.kind {
            let exists = self.doc.definitions.iter().any(|e| {
                matches!(&e.kind, Kind::FootnoteDefinition { identifier: x } if x.eq_ignore_ascii_case(identifier))
            });
            if exists {
                return Err(md_err(format!("Footnote [^{}] is already defined in this document.", identifier)));
            }
        }
        let mut d = d;
        let mut kids = std::mem::take(&mut d.children);
        self.doc.assign_ids(&mut kids, Content::Flow);
        d.children = kids;
        if d.id.is_none() {
            d.id = Some(self.doc.mint_id());
        }
        self.doc.definitions.push(d);
        Ok(())
    }

    /// Block slot.
    pub fn blocks(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        match v {
            CfmlValue::Null => Ok(Vec::new()),
            CfmlValue::String(s) => self.block_markdown(s),
            CfmlValue::Array(a) => {
                let mut out = Vec::new();
                for e in a.snapshot() {
                    match &e {
                        // A nested array is a paragraph of runs.
                        CfmlValue::Array(_) => {
                            let inl = self.inlines(&e)?;
                            out.push(Node::with_children(Kind::Paragraph, inl));
                        }
                        _ => out.extend(self.blocks(&e)?),
                    }
                }
                Ok(out)
            }
            CfmlValue::Struct(_) => {
                let map = v.as_struct().unwrap_or_default();
                if has_key(&map, "type") {
                    let n = self.node_from_struct(&map, Content::Flow)?;
                    // A definition given in the flow belongs on the document.
                    if let Kind::FootnoteDefinition { .. } = n.kind {
                        self.add_definition(n)?;
                        return Ok(Vec::new());
                    }
                    return Ok(vec![n]);
                }
                // `{ markdown = "…" }` alone is block markdown; anything else
                // is a run, and a run in a block slot is a paragraph.
                let only_markdown = map.iter().all(|(k, val)| {
                    k.as_str().eq_ignore_ascii_case("markdown") || matches!(val, CfmlValue::Null)
                }) && has_key(&map, "markdown");
                if only_markdown {
                    let src = map.get("markdown").map(|m| m.as_string()).unwrap_or_default();
                    return self.block_markdown(&src);
                }
                let inl = self.run(&map)?;
                Ok(vec![Node::with_children(Kind::Paragraph, inl)])
            }
            CfmlValue::NativeObject(_) => {
                let (blocks, defs) = other_document(v)?;
                for d in defs {
                    self.add_definition(d)?;
                }
                Ok(blocks)
            }
            CfmlValue::Query(_) => Err(md_err("A query can't be markdown content; use .table( data = query ).")),
            CfmlValue::Function(_) | CfmlValue::Closure(_) | CfmlValue::Component(_) | CfmlValue::Binary(_) => {
                Err(md_err(format!("A {} can't be markdown content.", type_label(v))))
            }
            other => Ok(vec![Node::with_children(Kind::Paragraph, vec![Node::text(other.as_string())])]),
        }
    }

    /// Inline slot.
    pub fn inlines(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        let mut out = match v {
            CfmlValue::Null => Vec::new(),
            CfmlValue::String(s) => parse_inline(s, &self.content_opts())?,
            CfmlValue::Array(a) => {
                let mut out = Vec::new();
                for e in a.snapshot() {
                    out.extend(self.inlines(&e)?);
                }
                out
            }
            CfmlValue::Struct(_) => {
                let map = v.as_struct().unwrap_or_default();
                if has_key(&map, "type") {
                    vec![self.node_from_struct(&map, Content::Phrasing)?]
                } else {
                    self.run(&map)?
                }
            }
            CfmlValue::NativeObject(_)
            | CfmlValue::Query(_)
            | CfmlValue::Function(_)
            | CfmlValue::Closure(_)
            | CfmlValue::Component(_)
            | CfmlValue::Binary(_) => {
                return Err(md_err(format!("A {} can't be inline markdown content.", type_label(v))))
            }
            other => vec![Node::text(other.as_string())],
        };
        normalise_inlines(&mut out);
        Ok(out)
    }

    /// Inline content for a heading or table cell: neither can hold a line
    /// break, so a newline in builder input becomes a space.
    pub fn single_line_inlines(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        let mut nodes = self.inlines(v)?;
        breaks_to_spaces(&mut nodes);
        normalise_inlines(&mut nodes);
        Ok(nodes)
    }

    /// A run: a struct of the keys the Typst `Document()` builder accepts.
    pub fn run(&mut self, map: &ValueMap) -> Result<Vec<Node>, CfmlError> {
        let mut f = Fields::new(map);
        let text = f.get(&["text"]);
        let markdown = f.get(&["markdown"]);
        let bold = f.get(&["bold", "strong"]).map(|v| v.is_true()).unwrap_or(false);
        let italic = f.get(&["italic", "emphasis", "em"]).map(|v| v.is_true()).unwrap_or(false);
        let strike = f.get(&["strike", "strikethrough", "delete"]).map(|v| v.is_true()).unwrap_or(false);
        let code = f.get(&["code"]).map(|v| v.is_true()).unwrap_or(false);
        let link = f.get(&["link", "href"]).map(|v| v.as_string());
        let title = opt_string(f.get(&["title"]));
        let image = f.get(&["image", "src"]).map(|v| v.as_string());
        let alt = f.get(&["alt"]).map(|v| v.as_string());
        let linebreak = f.get(&["linebreak", "break", "br"]).map(|v| v.is_true()).unwrap_or(false);
        let footnote = f.get(&["footnote"]);
        // Presentation the Typst builder understands and markdown cannot say:
        // ignored, so one runs array can feed both builders.
        f.get(&[
            "size", "fontsize", "color", "fontcolor", "font", "fontname", "family", "align", "alignment",
            "underline", "smallcaps", "bgcolor", "backgroundcolor", "fill", "format", "unit",
        ]);
        let unknown = f.unused_keys();
        if !unknown.is_empty() {
            return Err(md_err(format!(
                "Unknown key{} [{}] in a markdown text run. Valid keys: {}.",
                if unknown.len() > 1 { "s" } else { "" },
                unknown.join(", "),
                RUN_KEYS
            )));
        }

        if linebreak {
            return Ok(vec![Node::new(Kind::HardBreak)]);
        }

        let mut content: Vec<Node> = if let Some(url) = image {
            let alt = alt.or_else(|| text.as_ref().map(|t| t.as_string())).unwrap_or_default();
            vec![Node::new(Kind::Image { url, alt, title: if link.is_some() { None } else { title.clone() } })]
        } else if let Some(m) = markdown {
            self.inlines(&CfmlValue::string(m.as_string()))?
        } else if let Some(t) = &text {
            match t {
                CfmlValue::Array(_) | CfmlValue::Struct(_) => self.inlines(t)?,
                other => vec![Node::text(other.as_string())],
            }
        } else {
            Vec::new()
        };

        if code {
            let value: String = content.iter().map(|n| n.plain_text()).collect();
            content = vec![Node::new(Kind::InlineCode { value })];
        }
        if strike {
            content = vec![Node::with_children(Kind::Delete, content)];
        }
        if italic {
            content = vec![Node::with_children(Kind::Emphasis, content)];
        }
        if bold {
            content = vec![Node::with_children(Kind::Strong, content)];
        }
        if let Some(url) = link {
            content = vec![Node::with_children(Kind::Link { url, title }, content)];
        }

        if let Some(fv) = footnote {
            self.doc.footnotes = true;
            let identifier = self.next_footnote_id();
            let body: Vec<Node> = match &fv {
                CfmlValue::Bool(true) => std::mem::take(&mut content),
                CfmlValue::Bool(false) => Vec::new(),
                other if other.as_string().eq_ignore_ascii_case("true") || other.as_string().eq_ignore_ascii_case("yes") => {
                    std::mem::take(&mut content)
                }
                other => self.inlines(other)?,
            };
            if !body.is_empty() {
                let def = Node::with_children(
                    Kind::FootnoteDefinition { identifier: identifier.clone() },
                    vec![Node::with_children(Kind::Paragraph, body)],
                );
                self.add_definition(def)?;
                content.push(Node::new(Kind::FootnoteReference { identifier }));
            }
        }
        Ok(content)
    }

    fn next_footnote_id(&self) -> String {
        let mut n = self.doc.definitions.len() + 1;
        loop {
            let candidate = n.to_string();
            let taken = self.doc.definitions.iter().any(|d| {
                matches!(&d.kind, Kind::FootnoteDefinition { identifier } if identifier.eq_ignore_ascii_case(&candidate))
            });
            if !taken {
                return candidate;
            }
            n += 1;
        }
    }

    /// List items: an array of items, or one.
    pub fn items(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        match v {
            CfmlValue::Null => Ok(Vec::new()),
            CfmlValue::Array(a) => a.snapshot().iter().map(|e| self.item(e)).collect(),
            other => Ok(vec![self.item(other)?]),
        }
    }

    /// One list item. A string or runs array is the item's paragraph; a
    /// struct without `type` is `{ text | markdown | <run keys>, checked,
    /// children, id }`.
    pub fn item(&mut self, v: &CfmlValue) -> Result<Node, CfmlError> {
        match v {
            CfmlValue::Struct(_) => {
                let map = v.as_struct().unwrap_or_default();
                if has_key(&map, "type") {
                    return self.node_from_struct(&map, Content::ListItems);
                }
                let mut f = Fields::new(&map);
                let checked = f.get(&["checked"]).map(|c| c.is_true());
                let children = f.get(&["children"]);
                let id = f.get(&["id"]).map(|i| i.as_string());
                let rest: ValueMap = {
                    let mut m = ValueMap::default();
                    for (k, val) in f.rest() {
                        m.insert(k, val);
                    }
                    m
                };
                let mut kids = Vec::new();
                if !rest.is_empty() {
                    let inl = self.run(&rest)?;
                    if !inl.is_empty() {
                        kids.push(Node::with_children(Kind::Paragraph, inl));
                    }
                }
                if let Some(c) = children {
                    kids.extend(self.blocks(&c)?);
                }
                let mut n = Node::with_children(Kind::ListItem { checked }, kids);
                n.id = id;
                Ok(n)
            }
            CfmlValue::Null => Ok(Node::new(Kind::ListItem { checked: None })),
            other => {
                let inl = self.inlines(other)?;
                let kids = if inl.is_empty() { Vec::new() } else { vec![Node::with_children(Kind::Paragraph, inl)] };
                Ok(Node::with_children(Kind::ListItem { checked: None }, kids))
            }
        }
    }

    /// Table rows: an array of rows (each an array of cells, or a `tableRow`
    /// node), or a single row.
    pub fn rows(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        if let CfmlValue::Array(a) = v {
            let elems = a.snapshot();
            let is_rows = !elems.is_empty()
                && elems.iter().all(|e| match e {
                    CfmlValue::Array(_) => true,
                    CfmlValue::Struct(_) => {
                        let m = e.as_struct().unwrap_or_default();
                        m.get("type").map(|t| t.as_string().eq_ignore_ascii_case("tableRow")).unwrap_or(false)
                    }
                    _ => false,
                });
            if is_rows {
                return elems.iter().map(|e| self.row(e)).collect();
            }
        }
        Ok(vec![self.row(v)?])
    }

    fn row(&mut self, v: &CfmlValue) -> Result<Node, CfmlError> {
        if let CfmlValue::Struct(_) = v {
            let map = v.as_struct().unwrap_or_default();
            if has_key(&map, "type") {
                return self.node_from_struct(&map, Content::TableRows);
            }
        }
        Ok(Node::with_children(Kind::TableRow, self.cells(v)?))
    }

    pub fn cells(&mut self, v: &CfmlValue) -> Result<Vec<Node>, CfmlError> {
        match v {
            CfmlValue::Array(a) => a.snapshot().iter().map(|e| self.cell(e)).collect(),
            other => Ok(vec![self.cell(other)?]),
        }
    }

    fn cell(&mut self, v: &CfmlValue) -> Result<Node, CfmlError> {
        if let CfmlValue::Struct(_) = v {
            let map = v.as_struct().unwrap_or_default();
            if has_key(&map, "type") {
                return self.node_from_struct(&map, Content::TableCells);
            }
        }
        Ok(Node::with_children(Kind::TableCell, self.single_line_inlines(v)?))
    }

    /// A struct with a `type`: a canonical tree node (mdast field names, with
    /// the builder's names accepted as aliases). Keys this version does not
    /// know are preserved.
    pub fn node_from_struct(&mut self, map: &ValueMap, slot: Content) -> Result<Node, CfmlError> {
        let mut f = Fields::new(map);
        let ty = f.get(&["type"]).map(|t| t.as_string()).unwrap_or_default();
        let id = f.get(&["id"]).map(|i| i.as_string());
        let lc = ty.to_ascii_lowercase();
        let kind = match lc.as_str() {
            "paragraph" => Kind::Paragraph,
            "heading" => {
                let depth = match f.get(&["depth", "level"]) {
                    Some(d) => as_u64(&d, "heading depth")?,
                    None => 1,
                };
                if !(1..=6).contains(&depth) {
                    return Err(md_err(format!("Heading depth must be 1 to 6, not {}.", depth)));
                }
                Kind::Heading { depth: depth as u8 }
            }
            "thematicbreak" => Kind::ThematicBreak,
            "code" => Kind::Code {
                lang: opt_string(f.get(&["lang", "language"])),
                meta: opt_string(f.get(&["meta"])),
                value: f.get(&["value", "text", "code"]).map(|v| v.as_string()).unwrap_or_default(),
            },
            "html" => Kind::Html { value: f.get(&["value", "markup"]).map(|v| v.as_string()).unwrap_or_default() },
            "blockquote" => Kind::Blockquote,
            "list" => Kind::List {
                ordered: f.get(&["ordered"]).map(|v| v.is_true()).unwrap_or(false),
                start: match f.get(&["start"]) {
                    Some(s) => as_u64(&s, "list start")?,
                    None => 1,
                },
                spread: f.get(&["spread"]).map(|v| v.is_true()).unwrap_or(false),
                marker: match f.get(&["marker"]) {
                    Some(m) => {
                        let s = m.as_string();
                        let c = s.trim().chars().next();
                        match c {
                            Some('-' | '*' | '+' | '.' | ')') if s.trim().len() == 1 => c,
                            _ => {
                                return Err(md_err(format!(
                                    "List marker must be one of - * + . ), not [{}].",
                                    s
                                )))
                            }
                        }
                    }
                    None => None,
                },
            },
            "listitem" => Kind::ListItem { checked: f.get(&["checked"]).map(|v| v.is_true()) },
            "table" => Kind::Table {
                align: match f.get(&["align"]) {
                    Some(a) => parse_align_list(&a)?,
                    None => Vec::new(),
                },
            },
            "tablerow" => Kind::TableRow,
            "tablecell" => Kind::TableCell,
            "footnotedefinition" => Kind::FootnoteDefinition {
                identifier: f
                    .get(&["identifier", "label"])
                    .map(|v| v.as_string())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| md_err("A footnoteDefinition needs an identifier."))?,
            },
            "text" => Kind::Text { value: f.get(&["value", "text"]).map(|v| v.as_string()).unwrap_or_default() },
            "softbreak" => Kind::SoftBreak,
            "hardbreak" => Kind::HardBreak,
            "inlinecode" => {
                Kind::InlineCode { value: f.get(&["value", "text", "code"]).map(|v| v.as_string()).unwrap_or_default() }
            }
            "emphasis" => Kind::Emphasis,
            "strong" => Kind::Strong,
            "delete" => Kind::Delete,
            "link" => Kind::Link {
                url: f
                    .get(&["url", "href"])
                    .map(|v| v.as_string())
                    .ok_or_else(|| md_err("A link needs a url."))?,
                title: opt_string(f.get(&["title"])),
            },
            "image" => Kind::Image {
                url: f
                    .get(&["url", "src"])
                    .map(|v| v.as_string())
                    .ok_or_else(|| md_err("An image needs a url."))?,
                alt: f.get(&["alt"]).map(|v| v.as_string()).unwrap_or_default(),
                title: opt_string(f.get(&["title"])),
            },
            "footnotereference" => Kind::FootnoteReference {
                identifier: f
                    .get(&["identifier", "label"])
                    .map(|v| v.as_string())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| md_err("A footnoteReference needs an identifier."))?,
            },
            "" => return Err(md_err("A markdown node needs a type.")),
            other => {
                return Err(md_err(format!(
                    "Unknown markdown node type [{}]. Valid types: paragraph, heading, thematicBreak, code, html, \
                     blockquote, list, listItem, table, tableRow, tableCell, footnoteDefinition, text, softBreak, \
                     hardBreak, inlineCode, emphasis, strong, delete, link, image, footnoteReference.",
                    other
                )))
            }
        };
        let slot_ok = kind.fits(slot) || (slot == Content::Flow && matches!(kind, Kind::FootnoteDefinition { .. }));
        if !slot_ok {
            return Err(md_err(format!(
                "A {} can't go where {} are expected.",
                kind.type_name(),
                slot.describe()
            )));
        }
        let inner = kind.content();
        let children = match (inner, f.get(&["children"])) {
            (Content::None, Some(_)) => {
                return Err(md_err(format!("A {} has no children.", kind.type_name())));
            }
            (Content::None, None) => Vec::new(),
            (Content::TableCells, Some(c)) => {
                let mut cells = self.cells(&c)?;
                for cell in &mut cells {
                    breaks_to_spaces(&mut cell.children);
                }
                cells
            }
            (c, Some(v)) => self.content(c, &v)?,
            (_, None) => {
                // `text` on a link / emphasis / … is shorthand for its content.
                match (inner, f.get(&["text"])) {
                    (Content::Phrasing, Some(t)) if !matches!(kind, Kind::Text { .. }) => {
                        vec![Node::text(t.as_string())]
                    }
                    _ => Vec::new(),
                }
            }
        };
        let mut n = Node::with_children(kind, children);
        if let Kind::Table { align } = &mut n.kind {
            let width = n.children.first().map(|r| r.children.len()).unwrap_or(0);
            if align.is_empty() {
                *align = vec![Align::None; width];
            }
        }
        n.id = id;
        n.extra = f.rest();
        Ok(n)
    }
}

fn has_key(map: &ValueMap, key: &str) -> bool {
    map.get(key).map(|v| !matches!(v, CfmlValue::Null)).unwrap_or(false)
}

pub fn breaks_to_spaces(nodes: &mut Vec<Node>) {
    for n in nodes.iter_mut() {
        if matches!(n.kind, Kind::SoftBreak | Kind::HardBreak) {
            n.kind = Kind::Text { value: " ".to_string() };
        }
        breaks_to_spaces(&mut n.children);
    }
}

/// Blocks and footnote definitions of another `MarkdownDocument`, ids
/// stripped so the receiving document mints its own.
fn other_document(v: &CfmlValue) -> Result<(Vec<Node>, Vec<Node>), CfmlError> {
    let CfmlValue::NativeObject(o) = v else { return Err(md_err("Not a MarkdownDocument.")) };
    let guard = o
        .try_read()
        .map_err(|_| md_err("A MarkdownDocument can't be inserted into itself; use .copy() first."))?;
    let Some(md) = guard.as_any().and_then(|a| a.downcast_ref::<super::object::MarkdownDocument>()) else {
        return Err(md_err(format!("A {} object can't be markdown content.", guard.class_name())));
    };
    let mut blocks = md.doc.children.clone();
    let mut defs = md.doc.definitions.clone();
    Doc::strip_ids(&mut blocks);
    Doc::strip_ids(&mut defs);
    Ok((blocks, defs))
}

// ---------------------------------------------------------------------------
// whole documents
// ---------------------------------------------------------------------------

/// `MarkdownDocument( struct )`.
pub fn doc_from_struct(v: &CfmlValue, opts: &MdOptions) -> Result<Doc, CfmlError> {
    let map = v.as_struct().unwrap_or_default();
    let mut f = Fields::new(&map);
    if let Some(t) = f.get(&["type"]) {
        if !t.as_string().eq_ignore_ascii_case("document") {
            return Err(md_err(format!(
                "A markdown document struct has type \"document\", not [{}].",
                t.as_string()
            )));
        }
    }
    let (mut gfm, mut footnotes) = (opts.gfm, opts.footnotes);
    if let Some(p) = f.get(&["profile"]) {
        gfm = false;
        footnotes = false;
        for e in list_values(&p) {
            match e.as_string().trim().to_ascii_lowercase().as_str() {
                "gfm" => gfm = true,
                "footnotes" => footnotes = true,
                "commonmark" | "" => {}
                other => {
                    return Err(md_err(format!(
                        "Unknown markdown profile [{}]. Valid: commonmark, gfm, footnotes.",
                        other
                    )))
                }
            }
        }
    }
    let mut doc = Doc::new(gfm, footnotes);
    doc.front_matter = f.get(&["frontMatter"]).map(|v| v.as_string());
    let children = f.get(&["children"]);
    let definitions = f.get(&["definitions"]);
    doc.extra = f.rest();
    // Ids the caller supplied must survive and must not collide with minted
    // ones: note them all before minting anything.
    note_ids_in_value(&mut doc, children.as_ref());
    note_ids_in_value(&mut doc, definitions.as_ref());
    let mut loader = Loader::new(&mut doc, opts);
    let mut kids = match &children {
        Some(c) => loader.blocks(c)?,
        None => Vec::new(),
    };
    if let Some(defs) = &definitions {
        for d in list_values(defs) {
            let m = d.as_struct().ok_or_else(|| md_err("Each entry of definitions must be a footnoteDefinition struct."))?;
            let n = loader.node_from_struct(&m, Content::Flow)?;
            if !matches!(n.kind, Kind::FootnoteDefinition { .. }) {
                return Err(md_err("Each entry of definitions must be a footnoteDefinition."));
            }
            loader.add_definition(n)?;
        }
    }
    doc.assign_ids(&mut kids, Content::Flow);
    doc.children = kids;
    doc.tidy();
    validate_doc(&doc)?;
    Ok(doc)
}

fn note_ids_in_value(doc: &mut Doc, v: Option<&CfmlValue>) {
    fn walk(doc: &mut Doc, v: &CfmlValue) {
        match v {
            CfmlValue::Array(a) => {
                for e in a.snapshot() {
                    walk(doc, &e);
                }
            }
            CfmlValue::Struct(s) => {
                if let Some(id) = s.get_ci("id") {
                    doc.note_id(&id.as_string());
                }
                if let Some(c) = s.get_ci("children") {
                    walk(doc, &c);
                }
            }
            _ => {}
        }
    }
    if let Some(v) = v {
        walk(doc, v);
    }
}

pub fn doc_to_struct(doc: &Doc) -> CfmlValue {
    let mut m = ValueMap::default();
    m.insert("type", CfmlValue::string("document"));
    let mut profile = vec![CfmlValue::string("commonmark")];
    if doc.gfm {
        profile.push(CfmlValue::string("gfm"));
    }
    if doc.footnotes {
        profile.push(CfmlValue::string("footnotes"));
    }
    m.insert("profile", CfmlValue::array(profile));
    if let Some(fm) = &doc.front_matter {
        m.insert("frontMatter", CfmlValue::string(fm.clone()));
    }
    m.insert("children", CfmlValue::array(doc.children.iter().map(node_to_value).collect()));
    m.insert("definitions", CfmlValue::array(doc.definitions.iter().map(node_to_value).collect()));
    for (k, v) in &doc.extra {
        m.insert(k.clone(), v.clone());
    }
    CfmlValue::strukt(m)
}

fn opt(m: &mut ValueMap, key: &str, v: &Option<String>) {
    if let Some(s) = v {
        m.insert(key, CfmlValue::string(s.clone()));
    }
}

pub fn node_to_value(n: &Node) -> CfmlValue {
    let mut m = ValueMap::default();
    if let Some(id) = &n.id {
        m.insert("id", CfmlValue::string(id.clone()));
    }
    m.insert("type", CfmlValue::string(n.kind.type_name()));
    match &n.kind {
        Kind::Heading { depth } => {
            m.insert("depth", CfmlValue::Int(*depth as i64));
        }
        Kind::Code { lang, meta, value } => {
            opt(&mut m, "lang", lang);
            opt(&mut m, "meta", meta);
            m.insert("value", CfmlValue::string(value.clone()));
        }
        Kind::Html { value } | Kind::Text { value } | Kind::InlineCode { value } => {
            m.insert("value", CfmlValue::string(value.clone()));
        }
        Kind::List { ordered, start, spread, marker } => {
            m.insert("ordered", CfmlValue::Bool(*ordered));
            if *ordered {
                m.insert("start", CfmlValue::Int(*start as i64));
            }
            m.insert("spread", CfmlValue::Bool(*spread));
            if let Some(c) = marker {
                m.insert("marker", CfmlValue::string(c.to_string()));
            }
        }
        Kind::ListItem { checked: Some(c) } => {
            m.insert("checked", CfmlValue::Bool(*c));
        }
        Kind::Table { align } => {
            m.insert(
                "align",
                CfmlValue::array(
                    align.iter().map(|a| a.name().map(CfmlValue::string).unwrap_or(CfmlValue::Null)).collect(),
                ),
            );
        }
        Kind::Link { url, title } => {
            m.insert("url", CfmlValue::string(url.clone()));
            opt(&mut m, "title", title);
        }
        Kind::Image { url, alt, title } => {
            m.insert("url", CfmlValue::string(url.clone()));
            m.insert("alt", CfmlValue::string(alt.clone()));
            opt(&mut m, "title", title);
        }
        Kind::FootnoteDefinition { identifier } | Kind::FootnoteReference { identifier } => {
            m.insert("identifier", CfmlValue::string(identifier.clone()));
        }
        _ => {}
    }
    if n.kind.content() != Content::None {
        m.insert("children", CfmlValue::array(n.children.iter().map(node_to_value).collect()));
    }
    for (k, v) in &n.extra {
        m.insert(k.clone(), v.clone());
    }
    CfmlValue::strukt(m)
}

// ---------------------------------------------------------------------------
// validation
// ---------------------------------------------------------------------------

/// Everything the loader rejects rather than drops. Run on load and after
/// every mutation, so a document can never hold a tree that will not render
/// or will not survive `toMarkdown()` → parse.
pub fn validate_doc(doc: &Doc) -> Result<(), CfmlError> {
    let v = Validator { gfm: doc.gfm, footnotes: doc.footnotes };
    v.nodes(&doc.children, Content::Flow, Ctx::default())?;
    let mut seen_defs: HashSet<String> = HashSet::new();
    for d in &doc.definitions {
        let Kind::FootnoteDefinition { identifier } = &d.kind else {
            return Err(md_err(format!("[definitions] holds footnoteDefinition nodes, not {}.", d.kind.type_name())));
        };
        if !doc.footnotes {
            return Err(md_err("Footnotes are off for this document (profile has no \"footnotes\")."));
        }
        if !seen_defs.insert(identifier.to_lowercase()) {
            return Err(md_err(format!("Footnote [^{}] is defined twice.", identifier)));
        }
        v.nodes(&d.children, Content::Flow, Ctx::default())?;
    }
    check_ids(doc)
}

/// Validate nodes about to be grafted into a container, without walking the
/// rest of the document: `ctx` carries what the container's ancestry implies
/// (inside a heading or cell: one line; inside a link: no nested links).
pub fn validate_nodes(doc: &Doc, nodes: &[Node], slot: Content, ctx: Ctx) -> Result<(), CfmlError> {
    Validator { gfm: doc.gfm, footnotes: doc.footnotes }.nodes(nodes, slot, ctx)
}

/// Ids the caller supplied on new nodes: not empty, not numeric, not used
/// twice, not already in the document. Minted ids need no check (the counter
/// is always past every id in the tree), so the document is only walked when
/// someone named a node.
pub fn check_new_ids(doc: &mut Doc, nodes: &[Node]) -> Result<(), CfmlError> {
    let mut supplied: Vec<String> = Vec::new();
    for n in nodes {
        n.walk(&mut |m| {
            if let Some(id) = &m.id {
                supplied.push(id.clone());
            }
        });
    }
    if supplied.is_empty() {
        return Ok(());
    }
    let mut seen: HashSet<String> = HashSet::new();
    doc.walk(&mut |m| {
        if let Some(id) = &m.id {
            seen.insert(id.to_lowercase());
        }
    });
    for id in &supplied {
        if id.trim().is_empty() {
            return Err(md_err("A node id can't be empty."));
        }
        if is_numeric_id(id) {
            return Err(md_err(format!(
                "Node id [{}] is a number; ids can't be numbers, because a number means a position.",
                id
            )));
        }
        if !seen.insert(id.to_lowercase()) {
            return Err(md_err(format!("Node id [{}] is used twice.", id)));
        }
    }
    for id in &supplied {
        doc.note_id(id);
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
pub struct Ctx {
    pub single_line: bool,
    pub in_cell: bool,
    pub in_link: bool,
}

struct Validator {
    gfm: bool,
    footnotes: bool,
}

impl Validator {
    fn nodes(&self, nodes: &[Node], slot: Content, ctx: Ctx) -> Result<(), CfmlError> {
        for n in nodes {
            self.node(n, slot, ctx)?;
        }
        Ok(())
    }

    fn node(&self, n: &Node, slot: Content, ctx: Ctx) -> Result<(), CfmlError> {
        if !n.kind.fits(slot) {
            return Err(md_err(format!("A {} can't go where {} are expected.", n.kind.type_name(), slot.describe())));
        }
        let gfm_only = |what: &str| -> Result<(), CfmlError> {
            if self.gfm {
                Ok(())
            } else {
                Err(md_err(format!("{} need GitHub Flavored Markdown, which is off for this document.", what)))
            }
        };
        let mut inner = ctx;
        match &n.kind {
            Kind::Heading { depth } => {
                if !(1..=6).contains(depth) {
                    return Err(md_err(format!("Heading depth must be 1 to 6, not {}.", depth)));
                }
                inner.single_line = true;
            }
            Kind::Table { align } => {
                gfm_only("Tables")?;
                if n.children.is_empty() {
                    return Err(md_err("A table needs at least a header row."));
                }
                for (i, row) in n.children.iter().enumerate() {
                    if row.children.len() != align.len() {
                        return Err(md_err(format!(
                            "Table row {} has {} cell{}; the table has {} column{}. Every row needs one cell per column.",
                            i + 1,
                            row.children.len(),
                            if row.children.len() == 1 { "" } else { "s" },
                            align.len(),
                            if align.len() == 1 { "" } else { "s" }
                        )));
                    }
                }
                if align.is_empty() {
                    return Err(md_err("A table needs at least one column."));
                }
            }
            Kind::TableCell => {
                inner.single_line = true;
                inner.in_cell = true;
            }
            Kind::Delete => gfm_only("Strikethrough")?,
            Kind::ListItem { checked: Some(_) } => gfm_only("Task list items")?,
            Kind::FootnoteReference { .. } | Kind::FootnoteDefinition { .. } => {
                if !self.footnotes {
                    return Err(md_err("Footnotes are off for this document (profile has no \"footnotes\")."));
                }
            }
            // A soft break in a heading is what a multi-line setext heading
            // parses to; it renders as a space. A table cell has neither.
            Kind::HardBreak if ctx.single_line => {
                return Err(md_err("A heading or table cell can't contain a line break."));
            }
            Kind::SoftBreak if ctx.in_cell => {
                return Err(md_err("A table cell can't contain a line break."));
            }
            Kind::Link { .. } => {
                if ctx.in_link {
                    return Err(md_err("A link can't contain another link."));
                }
                inner.in_link = true;
            }
            Kind::Code { value: _, lang, .. } => {
                if lang.as_deref().map(|l| l.contains('`') || l.contains('\n')).unwrap_or(false) {
                    return Err(md_err("A code block's language can't contain a backtick or a newline."));
                }
            }
            _ => {}
        }
        self.nodes(&n.children, n.kind.content(), inner)
    }
}

fn is_numeric_id(id: &str) -> bool {
    let t = id.trim();
    !t.is_empty() && t.parse::<f64>().is_ok()
}

pub fn check_ids(doc: &Doc) -> Result<(), CfmlError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut err: Option<CfmlError> = None;
    doc.walk(&mut |n| {
        if err.is_some() {
            return;
        }
        if let Some(id) = &n.id {
            if id.trim().is_empty() {
                err = Some(md_err("A node id can't be empty."));
            } else if is_numeric_id(id) {
                err = Some(md_err(format!(
                    "Node id [{}] is a number; ids can't be numbers, because a number means a position.",
                    id
                )));
            } else if !seen.insert(id.to_lowercase()) {
                err = Some(md_err(format!("Node id [{}] is used twice.", id)));
            }
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
