//! [`Doc`] → HTML or CommonMark, through a throwaway comrak arena.
//!
//! Building comrak's own tree and handing it to comrak's formatters means we
//! inherit all of its escaping: fence lengths, inline-code padding, table
//! pipes, line-start characters that would otherwise start a block, the
//! `<!-- end list -->` separator between adjacent lists. We do not write a
//! markdown renderer.

use std::collections::HashMap;

use comrak::nodes::{
    AstNode, ListDelimType, ListType, NodeCode, NodeCodeBlock, NodeFootnoteDefinition,
    NodeFootnoteReference, NodeHeading, NodeHtmlBlock, NodeLink, NodeList, NodeTable,
    NodeTaskItem, NodeValue, TableAlignment,
};
use comrak::{format_commonmark, format_html, Arena};

use super::options::MdOptions;
use super::tree::{Align, Doc, Kind, Node};

pub fn render_html(doc: &Doc, opts: &MdOptions) -> String {
    let mut o = opts.clone();
    o.gfm = doc.gfm;
    o.footnotes = doc.footnotes;
    let copts = o.comrak();
    let arena = Arena::new();
    let root = build(&arena, doc);
    let mut out = String::new();
    let _ = format_html(root, &copts, &mut out);
    apply_table_class(out, &opts.table_class)
}

pub fn render_markdown(doc: &Doc, opts: &MdOptions) -> String {
    let mut o = opts.clone();
    o.gfm = doc.gfm;
    o.footnotes = doc.footnotes;
    let copts = o.comrak();
    let arena = Arena::new();
    let root = build(&arena, doc);
    let mut out = String::new();
    let _ = format_commonmark(root, &copts, &mut out);
    out
}

/// comrak has no option for a table class; add it to the `<table>` tags the
/// renderer wrote. (Raw HTML tables only exist with `unsafe` on, and are
/// written with whatever attributes their author gave them, so an exact
/// `<table>` is ours.)
pub fn apply_table_class(html: String, class: &str) -> String {
    if class.trim().is_empty() || !html.contains("<table>") {
        return html;
    }
    let escaped = class.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    html.replace("<table>", &format!("<table class=\"{}\">", escaped))
}

fn alloc<'a>(arena: &'a Arena<'a>, v: NodeValue) -> &'a AstNode<'a> {
    arena.alloc(AstNode::from(v))
}

/// Footnote bookkeeping comrak's parser would have done: number definitions
/// by first reference, count references, drop unreferenced definitions.
struct Footnotes {
    /// folded name → (ix, references so far, definition's own name)
    map: HashMap<String, (u32, u32, String)>,
    defined: HashMap<String, String>,
    next_ix: u32,
}

fn fold(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn build<'a>(arena: &'a Arena<'a>, doc: &Doc) -> &'a AstNode<'a> {
    let root = alloc(arena, NodeValue::Document);
    if let Some(fm) = &doc.front_matter {
        let mut s = String::from("---\n");
        s.push_str(fm);
        if !fm.is_empty() && !fm.ends_with('\n') {
            s.push('\n');
        }
        s.push_str("---\n\n");
        root.append(alloc(arena, NodeValue::FrontMatter(s)));
    }
    let mut fns = Footnotes { map: HashMap::new(), defined: HashMap::new(), next_ix: 0 };
    if doc.footnotes {
        for d in &doc.definitions {
            if let Kind::FootnoteDefinition { identifier } = &d.kind {
                fns.defined.insert(fold(identifier), identifier.clone());
            }
        }
    }
    for n in &doc.children {
        root.append(block(arena, n, &mut fns, doc.footnotes));
    }
    if doc.footnotes {
        // Definitions referenced from other definitions get numbered as they
        // are rendered, so walk until no new definition is reached.
        let mut emitted: Vec<String> = Vec::new();
        loop {
            let mut pending: Vec<(u32, String)> = fns
                .map
                .iter()
                .filter(|(k, _)| !emitted.contains(k))
                .map(|(k, (ix, _, _))| (*ix, k.clone()))
                .collect();
            if pending.is_empty() {
                break;
            }
            pending.sort();
            let mut nodes = Vec::new();
            for (_, key) in &pending {
                emitted.push(key.clone());
                if let Some(def) = doc.definitions.iter().find(|d| matches!(&d.kind, Kind::FootnoteDefinition { identifier } if fold(identifier) == *key)) {
                    let name = fns.map.get(key).map(|e| e.2.clone()).unwrap_or_default();
                    let node = alloc(arena, NodeValue::FootnoteDefinition(NodeFootnoteDefinition { name, total_references: 0 }));
                    for c in &def.children {
                        node.append(block(arena, c, &mut fns, true));
                    }
                    nodes.push((key.clone(), node));
                }
            }
            for (_, node) in nodes {
                root.append(node);
            }
        }
        // Totals are only known once every reference has been seen.
        for c in root.children() {
            let mut data = c.data_mut();
            if let NodeValue::FootnoteDefinition(ref mut nfd) = data.value {
                if let Some((_, total, _)) = fns.map.get(&fold(&nfd.name)) {
                    nfd.total_references = *total;
                }
            }
        }
    }
    root
}

fn block<'a>(arena: &'a Arena<'a>, n: &Node, fns: &mut Footnotes, footnotes: bool) -> &'a AstNode<'a> {
    match &n.kind {
        Kind::Paragraph => {
            let p = alloc(arena, NodeValue::Paragraph);
            inlines(arena, p, &n.children, fns, footnotes);
            p
        }
        Kind::Heading { depth } => {
            let h = alloc(arena, NodeValue::Heading(NodeHeading { level: *depth, setext: false, closed: false }));
            inlines(arena, h, &n.children, fns, footnotes);
            h
        }
        Kind::ThematicBreak => alloc(arena, NodeValue::ThematicBreak),
        Kind::Code { lang, meta, value } => {
            let mut info = lang.clone().unwrap_or_default();
            if let Some(m) = meta {
                if !info.is_empty() {
                    info.push(' ');
                }
                info.push_str(m);
            }
            let mut literal = value.clone();
            if !literal.is_empty() {
                literal.push('\n');
            }
            alloc(
                arena,
                NodeValue::CodeBlock(Box::new(NodeCodeBlock {
                    fenced: true,
                    fence_char: b'`',
                    fence_length: 3,
                    fence_offset: 0,
                    info,
                    literal,
                    closed: true,
                })),
            )
        }
        Kind::Html { value } => {
            let mut literal = value.clone();
            literal.push('\n');
            alloc(arena, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 6, literal }))
        }
        Kind::Blockquote => {
            let q = alloc(arena, NodeValue::BlockQuote);
            for c in &n.children {
                q.append(block(arena, c, fns, footnotes));
            }
            q
        }
        Kind::List { ordered, start, spread, .. } => {
            let is_task_list = n.children.iter().any(|i| matches!(i.kind, Kind::ListItem { checked: Some(_) }));
            let nl = NodeList {
                list_type: if *ordered { ListType::Ordered } else { ListType::Bullet },
                marker_offset: 0,
                padding: 2,
                start: *start as usize,
                delimiter: ListDelimType::Period,
                bullet_char: b'-',
                tight: !*spread,
                is_task_list,
            };
            let l = alloc(arena, NodeValue::List(nl));
            for item in &n.children {
                let v = match item.kind {
                    Kind::ListItem { checked: Some(c) } => NodeValue::TaskItem(NodeTaskItem {
                        symbol: if c { Some('x') } else { None },
                        symbol_sourcepos: (0, 0, 0, 0).into(),
                    }),
                    _ => NodeValue::Item(nl),
                };
                let li = alloc(arena, v);
                for c in &item.children {
                    li.append(block(arena, c, fns, footnotes));
                }
                l.append(li);
            }
            l
        }
        Kind::Table { align } => {
            let alignments: Vec<TableAlignment> = align
                .iter()
                .map(|a| match a {
                    Align::None => TableAlignment::None,
                    Align::Left => TableAlignment::Left,
                    Align::Center => TableAlignment::Center,
                    Align::Right => TableAlignment::Right,
                })
                .collect();
            let t = alloc(
                arena,
                NodeValue::Table(Box::new(NodeTable {
                    num_columns: alignments.len(),
                    alignments,
                    num_rows: n.children.len(),
                    num_nonempty_cells: 0,
                })),
            );
            for (i, row) in n.children.iter().enumerate() {
                let r = alloc(arena, NodeValue::TableRow(i == 0));
                for cell in &row.children {
                    let c = alloc(arena, NodeValue::TableCell);
                    inlines(arena, c, &cell.children, fns, footnotes);
                    r.append(c);
                }
                t.append(r);
            }
            t
        }
        // Validation keeps these out of the flow; render them harmlessly if
        // they ever arrive (a struct written by hand, say).
        Kind::ListItem { .. } | Kind::TableRow | Kind::TableCell | Kind::FootnoteDefinition { .. } => {
            let q = alloc(arena, NodeValue::BlockQuote);
            for c in &n.children {
                q.append(block(arena, c, fns, footnotes));
            }
            q
        }
        _ => {
            let p = alloc(arena, NodeValue::Paragraph);
            inlines(arena, p, std::slice::from_ref(n), fns, footnotes);
            p
        }
    }
}

fn inlines<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, nodes: &[Node], fns: &mut Footnotes, footnotes: bool) {
    for n in nodes {
        parent.append(inline(arena, n, fns, footnotes));
    }
}

fn inline<'a>(arena: &'a Arena<'a>, n: &Node, fns: &mut Footnotes, footnotes: bool) -> &'a AstNode<'a> {
    let container = |v: NodeValue, fns: &mut Footnotes| {
        let c = alloc(arena, v);
        inlines(arena, c, &n.children, fns, footnotes);
        c
    };
    match &n.kind {
        Kind::Text { value } => alloc(arena, NodeValue::Text(value.clone().into())),
        Kind::SoftBreak => alloc(arena, NodeValue::SoftBreak),
        Kind::HardBreak => alloc(arena, NodeValue::LineBreak),
        Kind::InlineCode { value } => {
            alloc(arena, NodeValue::Code(NodeCode { num_backticks: 1, literal: value.clone() }))
        }
        Kind::Html { value } => alloc(arena, NodeValue::HtmlInline(value.clone())),
        Kind::Emphasis => container(NodeValue::Emph, fns),
        Kind::Strong => container(NodeValue::Strong, fns),
        Kind::Delete => container(NodeValue::Strikethrough, fns),
        Kind::Link { url, title } => container(
            NodeValue::Link(Box::new(NodeLink { url: url.clone(), title: title.clone().unwrap_or_default() })),
            fns,
        ),
        Kind::Image { url, alt, title } => {
            let img = alloc(
                arena,
                NodeValue::Image(Box::new(NodeLink { url: url.clone(), title: title.clone().unwrap_or_default() })),
            );
            if !alt.is_empty() {
                img.append(alloc(arena, NodeValue::Text(alt.clone().into())));
            }
            img
        }
        Kind::FootnoteReference { identifier } => {
            let key = fold(identifier);
            if footnotes {
                if let Some(def_name) = fns.defined.get(&key).cloned() {
                    let next = fns.next_ix + 1;
                    let entry = fns.map.entry(key).or_insert((next, 0, def_name));
                    if entry.0 == next {
                        fns.next_ix = next;
                    }
                    entry.1 += 1;
                    return alloc(
                        arena,
                        NodeValue::FootnoteReference(Box::new(NodeFootnoteReference {
                            name: entry.2.clone(),
                            texts: Vec::new(),
                            ref_num: entry.1,
                            ix: entry.0,
                        })),
                    );
                }
            }
            // No definition: comrak's parser leaves the source text.
            alloc(arena, NodeValue::Text(format!("[^{}]", identifier).into()))
        }
        // Block kinds never reach here after validation.
        _ => alloc(arena, NodeValue::Text(n.plain_text().into())),
    }
}
