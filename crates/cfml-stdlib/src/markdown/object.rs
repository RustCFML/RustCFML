//! `MarkdownDocument()` — a markdown document as an editable tree.
//!
//! A shared, reference-typed native object like `Spreadsheet()`: builder
//! verbs and edits return the object so calls chain; terminals (`toHtml`,
//! `toMarkdown`, `toStruct`, …) return their value. Every edit runs on a copy
//! of the tree and is committed only if the result validates, so a failed
//! call leaves the document exactly as it was.

use std::sync::{Arc, RwLock, Weak};

use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};

use super::cfml::{doc_from_struct, doc_to_struct, validate_doc, Loader};
use super::options::{defaults, md_err, MdOptions};
use super::parse::parse_markdown;
use super::render::{render_html, render_markdown};
use super::table::{build_table, TableArgs};
use super::tree::{Doc, Kind, Node};
use super::{html_in, ops};

pub struct MarkdownDocument {
    pub doc: Doc,
    /// The options the document was made with: the defaults for its
    /// renderers and for parsing markdown strings given to it.
    pub opts: MdOptions,
    last_ids: Vec<String>,
    self_ref: Option<Weak<RwLock<MarkdownDocument>>>,
}

impl std::fmt::Debug for MarkdownDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownDocument").field("blocks", &self.doc.children.len()).finish()
    }
}

impl MarkdownDocument {
    pub fn new(doc: Doc, opts: MdOptions) -> MarkdownDocument {
        MarkdownDocument { doc, opts, last_ids: Vec::new(), self_ref: None }
    }

    pub fn into_value(mut self) -> CfmlValue {
        let arc: Arc<RwLock<MarkdownDocument>> = Arc::new_cyclic(|weak| {
            self.self_ref = Some(weak.clone());
            RwLock::new(self)
        });
        CfmlValue::NativeObject(arc)
    }

    fn this(&self) -> CfmlValue {
        match self.self_ref.as_ref().and_then(|w| w.upgrade()) {
            Some(arc) => CfmlValue::NativeObject(arc),
            None => CfmlValue::Null,
        }
    }

    fn copy(&self) -> CfmlValue {
        MarkdownDocument::new(self.doc.clone(), self.opts.clone()).into_value()
    }

    /// An edit that is all-or-nothing by construction: it resolves and
    /// validates everything before it touches the tree (see `ops::graft`).
    /// No copy, so building a large document a block at a time stays linear.
    fn edit(&mut self, f: impl FnOnce(&mut Doc, &MdOptions) -> Result<Vec<String>, CfmlError>) -> CfmlResult {
        let ids = f(&mut self.doc, &self.opts)?;
        self.last_ids = ids;
        Ok(self.this())
    }

    /// An edit that can't check everything up front (moves, field changes):
    /// run it on a copy of the tree and commit only if the result validates.
    fn edit_copy(&mut self, f: impl FnOnce(&mut Doc, &MdOptions) -> Result<Vec<String>, CfmlError>) -> CfmlResult {
        let mut work = self.doc.clone();
        let ids = f(&mut work, &self.opts)?;
        work.tidy();
        validate_doc(&work)?;
        self.doc = work;
        self.last_ids = ids;
        Ok(self.this())
    }

    /// A builder verb: make blocks with the loader, put them at the end of
    /// the document or of the `into` container.
    fn add(&mut self, into: Option<&CfmlValue>, make: impl FnOnce(&mut Loader) -> Result<Vec<Node>, CfmlError>) -> CfmlResult {
        let into = into.cloned();
        self.edit(|doc, opts| {
            let c = ops::resolve_parent(doc, into.as_ref())?;
            ops::build_and_graft(doc, opts, &c, make)
        })
    }

    fn stats(&self) -> CfmlValue {
        let text = {
            let mut parts = Vec::new();
            for n in &self.doc.children {
                parts.push(n.plain_text());
            }
            parts.join("\n")
        };
        let (mut headings, mut code, mut tables, mut links, mut images, mut paragraphs, mut lists) = (0, 0, 0, 0, 0, 0, 0);
        self.doc.walk(&mut |n| match n.kind {
            Kind::Heading { .. } => headings += 1,
            Kind::Code { .. } => code += 1,
            Kind::Table { .. } => tables += 1,
            Kind::Link { .. } => links += 1,
            Kind::Image { .. } => images += 1,
            Kind::Paragraph => paragraphs += 1,
            Kind::List { .. } => lists += 1,
            _ => {}
        });
        let md = render_markdown(&self.doc, &self.opts);
        let mut m = ValueMap::default();
        m.insert("chars", CfmlValue::Int(text.chars().count() as i64));
        m.insert("words", CfmlValue::Int(text.split_whitespace().count() as i64));
        m.insert("lines", CfmlValue::Int(md.lines().count() as i64));
        m.insert("blocks", CfmlValue::Int(self.doc.children.len() as i64));
        m.insert("headings", CfmlValue::Int(headings));
        m.insert("paragraphs", CfmlValue::Int(paragraphs));
        m.insert("lists", CfmlValue::Int(lists));
        m.insert("codeBlocks", CfmlValue::Int(code));
        m.insert("tables", CfmlValue::Int(tables));
        m.insert("links", CfmlValue::Int(links));
        m.insert("images", CfmlValue::Int(images));
        m.insert("footnotes", CfmlValue::Int(self.doc.definitions.len() as i64));
        CfmlValue::strukt(m)
    }

    fn front_matter(&self) -> CfmlResult {
        let Some(fm) = &self.doc.front_matter else {
            return Ok(CfmlValue::strukt(ValueMap::default()));
        };
        #[cfg(feature = "yaml")]
        {
            crate::builtins::fn_yaml_deserialize(vec![CfmlValue::string(fm.clone())])
                .map_err(|e| md_err(format!("The front matter is not valid YAML: {}", e.message)))
        }
        #[cfg(not(feature = "yaml"))]
        {
            Ok(CfmlValue::string(fm.clone()))
        }
    }

    fn set_front_matter(&mut self, v: Option<&CfmlValue>) -> CfmlResult {
        self.doc.front_matter = match v {
            None | Some(CfmlValue::Null) => None,
            Some(CfmlValue::String(s)) if s.trim().is_empty() => None,
            Some(CfmlValue::String(s)) => Some(s.trim_end().to_string()),
            #[cfg(feature = "yaml")]
            Some(other @ (CfmlValue::Struct(_) | CfmlValue::Array(_))) => {
                Some(crate::builtins::fn_yaml_serialize(vec![other.clone()])?.as_string().trim_end().to_string())
            }
            Some(other) => {
                return Err(md_err(format!(
                    "Front matter is a YAML string or a struct, not a {}.",
                    other.type_name()
                )))
            }
        };
        Ok(self.this())
    }
}

fn arg(a: &[CfmlValue], i: usize) -> Option<&CfmlValue> {
    a.get(i).filter(|v| !matches!(v, CfmlValue::Null))
}

fn arg_str(a: &[CfmlValue], i: usize) -> Option<String> {
    arg(a, i).map(|v| v.as_string())
}

fn arg_bool(a: &[CfmlValue], i: usize, default: bool) -> bool {
    arg(a, i).map(|v| v.is_true()).unwrap_or(default)
}

fn required<'a>(a: &'a [CfmlValue], i: usize, method: &str, name: &str) -> Result<&'a CfmlValue, CfmlError> {
    arg(a, i).ok_or_else(|| md_err(format!("{}() needs [{}].", method, name)))
}

const METHODS: &str = "heading, paragraph, list, taskList, quote, code, table, image, rule, html, markdown, \
                       insertAt, insertBefore, insertAfter, append, prepend, replace, remove, move, moveBefore, \
                       moveAfter, set, setText, get, find, lastIds, outline, section, appendToSection, replaceSection, \
                       removeSection, moveSection, moveSectionBefore, moveSectionAfter, toHtml, toMarkdown, toStruct, \
                       toJSON, toText, stats, frontMatter, setFrontMatter, len, copy, toString";

impl CfmlNative for MarkdownDocument {
    fn class_name(&self) -> &str {
        "MarkdownDocument"
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn duplicate(&self) -> Option<CfmlValue> {
        Some(self.copy())
    }

    fn visit_values(&self, f: &mut dyn FnMut(&CfmlValue)) {
        for (_, v) in &self.doc.extra {
            f(v);
        }
        self.doc.walk(&mut |n| {
            for (_, v) in &n.extra {
                f(v);
            }
        });
    }

    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_ascii_lowercase().as_str() {
            "heading" | "h" => &["text", "level", "id", "into", "depth"][..],
            "paragraph" | "p" | "text" => &["text", "id", "into"][..],
            "list" => &["items", "ordered", "start", "id", "into", "spread"][..],
            "tasklist" => &["items", "id", "into"][..],
            "quote" | "blockquote" => &["content", "id", "into", "children"][..],
            "code" | "pre" => &["text", "language", "meta", "id", "into", "lang"][..],
            "table" => &["data", "columnList", "headers", "align", "cellFormat", "columnFormats", "id", "into"][..],
            "image" => &["url", "alt", "title", "id", "into"][..],
            "rule" | "hr" | "thematicbreak" => &["id", "into"][..],
            "html" => &["markup", "id", "into"][..],
            "markdown" => &["source", "into"][..],
            "insertat" => &["position", "content", "parent"][..],
            "insertbefore" | "insertafter" => &["target", "content", "parent"][..],
            "append" | "prepend" => &["parent", "content"][..],
            "replace" => &["target", "content", "parent"][..],
            "remove" | "get" => &["target", "parent"][..],
            "move" => &["target", "position", "parent"][..],
            "movebefore" | "moveafter" => &["target", "other"][..],
            "set" => &["target", "field", "value"][..],
            "settext" => &["target", "content"][..],
            "find" => &["type", "text", "level"][..],
            "section" | "removesection" => &["heading"][..],
            "appendtosection" => &["heading", "content"][..],
            "replacesection" => &["heading", "content", "keepHeading"][..],
            "movesection" => &["heading", "position"][..],
            "movesectionbefore" | "movesectionafter" => &["heading", "other"][..],
            "tohtml" | "tomarkdown" => &["options"][..],
            "setfrontmatter" => &["value"][..],
            "outline" | "lastids" | "tostruct" | "tojson" | "totext" | "stats" | "frontmatter" | "len"
            | "copy" | "tostring" => &[][..],
            _ => return None,
        })
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let a = args.as_slice();
        let lc = name.to_ascii_lowercase();
        match lc.as_str() {
            // ---- builder verbs ------------------------------------------------
            "heading" | "h" => {
                let text = a.first().cloned().unwrap_or(CfmlValue::Null);
                let level = arg(a, 1).or_else(|| arg(a, 4)).map(|v| v.as_string());
                let depth = match level {
                    Some(l) => {
                        let n = l.trim().parse::<f64>().ok().filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= 6.0);
                        n.ok_or_else(|| md_err(format!("Heading level must be 1 to 6, not [{}].", l)))? as u8
                    }
                    None => 1,
                };
                let id = arg_str(a, 2);
                self.add(arg(a, 3), move |l| {
                    let mut n = Node::with_children(Kind::Heading { depth }, l.single_line_inlines(&text)?);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "paragraph" | "p" | "text" => {
                let text = a.first().cloned().unwrap_or(CfmlValue::Null);
                let id = arg_str(a, 1);
                self.add(arg(a, 2), move |l| {
                    let mut n = Node::with_children(Kind::Paragraph, l.inlines(&text)?);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "list" | "tasklist" => {
                let task = lc == "tasklist";
                let items = a.first().cloned().unwrap_or(CfmlValue::Null);
                let (ordered, start, id, into, spread) = if task {
                    (false, 1, arg_str(a, 1), arg(a, 2), false)
                } else {
                    let start = match arg(a, 2) {
                        Some(s) => {
                            let t = s.as_string();
                            t.trim().parse::<f64>().ok().filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| n as u64).ok_or_else(|| {
                                md_err(format!("List start must be a whole number, not [{}].", t))
                            })?
                        }
                        None => 1,
                    };
                    (arg_bool(a, 1, false), start, arg_str(a, 3), arg(a, 4), arg_bool(a, 5, false))
                };
                self.add(into, move |l| {
                    let mut kids = l.items(&items)?;
                    if task {
                        for k in kids.iter_mut() {
                            if let Kind::ListItem { checked: c @ None } = &mut k.kind {
                                *c = Some(false);
                            }
                        }
                    }
                    let mut n = Node::with_children(Kind::List { ordered, start, spread, marker: None }, kids);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "quote" | "blockquote" => {
                let content = a.first().cloned().unwrap_or(CfmlValue::Null);
                let children = arg(a, 3).cloned();
                let id = arg_str(a, 1);
                self.add(arg(a, 2), move |l| {
                    let mut kids = Vec::new();
                    if !matches!(content, CfmlValue::Null) {
                        let inl = l.inlines(&content)?;
                        if !inl.is_empty() {
                            kids.push(Node::with_children(Kind::Paragraph, inl));
                        }
                    }
                    if let Some(c) = &children {
                        kids.extend(l.blocks(c)?);
                    }
                    let mut n = Node::with_children(Kind::Blockquote, kids);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "code" | "pre" => {
                let text = a.first().map(|v| v.as_string()).unwrap_or_default();
                let lang = arg_str(a, 1).or_else(|| arg_str(a, 5)).filter(|s| !s.trim().is_empty());
                let meta = arg_str(a, 2).filter(|s| !s.trim().is_empty());
                let id = arg_str(a, 3);
                self.add(arg(a, 4), move |_| {
                    let mut n = Node::new(Kind::Code { lang, meta, value: text.trim_end_matches('\n').to_string() });
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "table" => {
                let data = required(a, 0, "table", "data")?.clone();
                let rest: Vec<Option<CfmlValue>> = (1..=5).map(|i| arg(a, i).cloned()).collect();
                let id = arg_str(a, 6);
                self.add(arg(a, 7), move |l| {
                    let mut n = build_table(
                        l,
                        TableArgs {
                            data: &data,
                            column_list: rest[0].as_ref(),
                            headers: rest[1].as_ref(),
                            align: rest[2].as_ref(),
                            cell_format: rest[3].as_ref(),
                            column_formats: rest[4].as_ref(),
                        },
                    )?;
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "image" => {
                let url = required(a, 0, "image", "url")?.as_string();
                let alt = arg_str(a, 1).unwrap_or_default();
                let title = arg_str(a, 2).filter(|s| !s.is_empty());
                let id = arg_str(a, 3);
                self.add(arg(a, 4), move |_| {
                    let mut n = Node::with_children(Kind::Paragraph, vec![Node::new(Kind::Image { url, alt, title })]);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "rule" | "hr" | "thematicbreak" => {
                let id = arg_str(a, 0);
                self.add(arg(a, 1), move |_| {
                    let mut n = Node::new(Kind::ThematicBreak);
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "html" => {
                let markup = a.first().map(|v| v.as_string()).unwrap_or_default();
                let id = arg_str(a, 1);
                self.add(arg(a, 2), move |_| {
                    let mut n = Node::new(Kind::Html { value: markup.trim_end_matches('\n').to_string() });
                    n.id = id;
                    Ok(vec![n])
                })
            }
            "markdown" => {
                let src = a.first().cloned().unwrap_or(CfmlValue::Null);
                self.add(arg(a, 1), move |l| l.blocks(&src))
            }

            // ---- structural edits ---------------------------------------------
            "insertat" => {
                let p = required(a, 0, "insertAt", "position")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                let parent = arg(a, 2).cloned();
                self.edit(|d, o| ops::insert_at(d, o, &p, &c, parent.as_ref()))
            }
            "insertbefore" | "insertafter" => {
                let before = lc == "insertbefore";
                let t = required(a, 0, name, "target")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                let parent = arg(a, 2).cloned();
                self.edit(|d, o| ops::insert_rel(d, o, &t, &c, parent.as_ref(), before))
            }
            "append" | "prepend" => {
                let front = lc == "prepend";
                let p = required(a, 0, name, "parent")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                self.edit(|d, o| ops::append(d, o, &p, &c, front))
            }
            "replace" => {
                let t = required(a, 0, "replace", "target")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                let parent = arg(a, 2).cloned();
                self.edit(|d, o| ops::replace(d, o, &t, &c, parent.as_ref()))
            }
            "remove" => {
                let t = required(a, 0, "remove", "target")?.clone();
                let parent = arg(a, 1).cloned();
                self.edit(|d, _| ops::remove(d, &t, parent.as_ref()).map(|_| Vec::new()))
            }
            "move" => {
                let t = required(a, 0, "move", "target")?.clone();
                let p = required(a, 1, "move", "position")?.clone();
                let parent = arg(a, 2).cloned();
                self.edit_copy(|d, _| ops::move_to(d, &t, &p, parent.as_ref()).map(|_| Vec::new()))
            }
            "movebefore" | "moveafter" => {
                let before = lc == "movebefore";
                let t = required(a, 0, name, "target")?.clone();
                let other = required(a, 1, name, "other")?.clone();
                self.edit_copy(|d, _| ops::move_rel(d, &t, &other, before).map(|_| Vec::new()))
            }
            "set" => {
                let t = required(a, 0, "set", "target")?.clone();
                let field = required(a, 1, "set", "field")?.clone();
                let value = a.get(2).cloned().unwrap_or(CfmlValue::Null);
                self.edit_copy(|d, _| {
                    match &field {
                        CfmlValue::Struct(_) => ops::set_fields(d, &t, &field)?,
                        f => ops::set_field(d, &t, &f.as_string(), &value)?,
                    }
                    Ok(Vec::new())
                })
            }
            "settext" => {
                let t = required(a, 0, "setText", "target")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                self.edit_copy(|d, o| ops::set_text(d, o, &t, &c).map(|_| Vec::new()))
            }
            "get" => {
                let t = required(a, 0, "get", "target")?;
                ops::get(&self.doc, t, arg(a, 1))
            }
            "find" => {
                let level = match arg(a, 2) {
                    Some(v) => Some(
                        v.as_string()
                            .trim()
                            .parse::<u8>()
                            .map_err(|_| md_err(format!("find() level must be 1 to 6, not [{}].", v.as_string())))?,
                    ),
                    None => None,
                };
                Ok(ops::find(&self.doc, arg_str(a, 0).as_deref(), arg_str(a, 1).as_deref(), level))
            }
            "lastids" => Ok(CfmlValue::array(self.last_ids.iter().cloned().map(CfmlValue::string).collect())),

            // ---- sections -----------------------------------------------------
            "outline" => Ok(ops::outline(&self.doc)),
            "section" => ops::section_info(&self.doc, required(a, 0, "section", "heading")?),
            "appendtosection" => {
                let h = required(a, 0, "appendToSection", "heading")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                self.edit(|d, o| ops::append_to_section(d, o, &h, &c))
            }
            "replacesection" => {
                let h = required(a, 0, "replaceSection", "heading")?.clone();
                let c = a.get(1).cloned().unwrap_or(CfmlValue::Null);
                let keep = arg_bool(a, 2, true);
                self.edit(|d, o| ops::replace_section(d, o, &h, &c, keep))
            }
            "removesection" => {
                let h = required(a, 0, "removeSection", "heading")?.clone();
                self.edit(|d, _| ops::remove_section(d, &h).map(|_| Vec::new()))
            }
            "movesection" => {
                let h = required(a, 0, "moveSection", "heading")?.clone();
                let p = required(a, 1, "moveSection", "position")?.clone();
                self.edit_copy(|d, _| ops::move_section(d, &h, &p).map(|_| Vec::new()))
            }
            "movesectionbefore" | "movesectionafter" => {
                let before = lc == "movesectionbefore";
                let h = required(a, 0, name, "heading")?.clone();
                let other = required(a, 1, name, "other")?.clone();
                self.edit_copy(|d, _| ops::move_section_rel(d, &h, &other, before).map(|_| Vec::new()))
            }

            // ---- terminals ----------------------------------------------------
            "tohtml" => {
                let o = MdOptions::overlay(&self.opts, arg(a, 0))?;
                Ok(CfmlValue::string(render_html(&self.doc, &o)))
            }
            "tomarkdown" | "tostring" => {
                let o = MdOptions::overlay(&self.opts, arg(a, 0))?;
                Ok(CfmlValue::string(render_markdown(&self.doc, &o)))
            }
            "tostruct" => Ok(doc_to_struct(&self.doc)),
            "tojson" => crate::builtins::fn_serialize_json(vec![doc_to_struct(&self.doc)]),
            "totext" => Ok(CfmlValue::string(
                self.doc.children.iter().map(|n| n.plain_text()).collect::<Vec<_>>().join("\n\n"),
            )),
            "stats" => Ok(self.stats()),
            "frontmatter" => self.front_matter(),
            "setfrontmatter" => self.set_front_matter(arg(a, 0)),
            "len" => Ok(CfmlValue::Int(self.doc.children.len() as i64)),
            "copy" | "duplicate" | "clone" => Ok(self.copy()),
            other => Err(md_err(format!("MarkdownDocument has no method [{}]. Available: {}.", other, METHODS))),
        }
    }
}

pub fn is_markdown_document(v: &CfmlValue) -> bool {
    let CfmlValue::NativeObject(o) = v else { return false };
    match o.try_read() {
        Ok(g) => g.as_any().map(|a| a.is::<MarkdownDocument>()).unwrap_or(false),
        // Locked by a running method of its own: it's an object of ours only
        // if the class name says so.
        Err(_) => false,
    }
}

/// `MarkdownDocument( [source] [, options] [, html] )`.
pub fn construct(args: Vec<CfmlValue>) -> CfmlResult {
    let opts = MdOptions::overlay(&defaults(), arg(&args, 1))?;
    if let Some(html) = arg(&args, 2) {
        let mut doc = html_in::html_to_doc(&html.as_string(), &opts);
        doc.assign_all_ids();
        validate_doc(&doc)?;
        return Ok(MarkdownDocument::new(doc, opts).into_value());
    }
    let doc = match arg(&args, 0) {
        None => Doc::new(opts.gfm, opts.footnotes),
        Some(CfmlValue::String(s)) => parse_markdown(s, &opts)?,
        Some(v @ CfmlValue::Struct(_)) => doc_from_struct(v, &opts)?,
        Some(CfmlValue::NativeObject(o)) => {
            let g = o.try_read().map_err(|_| md_err("That object is busy."))?;
            let Some(md) = g.as_any().and_then(|x| x.downcast_ref::<MarkdownDocument>()) else {
                return Err(md_err(format!("MarkdownDocument() can't be made from a {} object.", g.class_name())));
            };
            // A copy; it keeps the original's options unless new ones are given.
            let o2 = if arg(&args, 1).is_some() { opts.clone() } else { md.opts.clone() };
            return Ok(MarkdownDocument::new(md.doc.clone(), o2).into_value());
        }
        Some(other) => {
            return Err(md_err(format!(
                "MarkdownDocument() takes markdown text, a document struct, or another MarkdownDocument; not a {}.",
                other.type_name()
            )))
        }
    };
    Ok(MarkdownDocument::new(doc, opts).into_value())
}
