//! Markdown text → [`Doc`], via comrak's parser.
//!
//! comrak resolves reference links while parsing (a link node carries its
//! final url), so the tree stores links resolved: `[x][ref]` and `[x](url)`
//! are the same node. Footnote definitions are collected into
//! `Doc::definitions`; front matter is kept opaque.

use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{parse_document, Arena};

use super::options::{md_err, MdOptions};
use super::tree::{normalise_inlines, Align, Doc, Kind, Node};
use cfml_common::vm::CfmlError;

pub fn parse_markdown(src: &str, opts: &MdOptions) -> Result<Doc, CfmlError> {
    let mut doc = parse_unnumbered(src, opts)?;
    doc.assign_all_ids();
    Ok(doc)
}

/// Parse without minting ids — for a fragment that is about to be grafted
/// into another document, which mints its own.
pub fn parse_unnumbered(src: &str, opts: &MdOptions) -> Result<Doc, CfmlError> {
    let owned;
    let src = if opts.dedent {
        owned = super::options::dedent(src);
        owned.as_str()
    } else {
        src
    };
    let arena = Arena::new();
    let copts = opts.comrak();
    let root = parse_document(&arena, src, &copts);
    let mut doc = Doc::new(opts.gfm, opts.footnotes);
    for child in root.children() {
        let value = child.data().value.clone();
        match value {
            NodeValue::FrontMatter(fm) => doc.front_matter = Some(strip_front_matter(&fm)),
            _ => {
                if let Some(n) = block(child, &mut doc)? {
                    doc.children.push(n);
                }
            }
        }
    }
    Ok(doc)
}

/// comrak keeps the delimiters and the blank line after them; the tree keeps
/// only what is between.
fn strip_front_matter(fm: &str) -> String {
    let mut lines: Vec<&str> = fm.lines().collect();
    if lines.first().map(|l| l.trim() == "---").unwrap_or(false) {
        lines.remove(0);
    }
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if lines.last().map(|l| l.trim() == "---").unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

fn strip_one_newline(s: &str) -> String {
    s.strip_suffix('\n').unwrap_or(s).to_string()
}

fn block<'a>(node: &'a AstNode<'a>, doc: &mut Doc) -> Result<Option<Node>, CfmlError> {
    let value = node.data().value.clone();
    let n = match value {
        NodeValue::Paragraph => Node::with_children(Kind::Paragraph, inlines(node)?),
        NodeValue::Heading(h) => Node::with_children(Kind::Heading { depth: h.level }, inlines(node)?),
        NodeValue::ThematicBreak => Node::new(Kind::ThematicBreak),
        NodeValue::CodeBlock(cb) => {
            let info = cb.info.trim();
            let (lang, meta) = match info.split_once(|c: char| c.is_whitespace()) {
                Some((l, m)) => (l.to_string(), m.trim().to_string()),
                None => (info.to_string(), String::new()),
            };
            Node::new(Kind::Code {
                lang: (!lang.is_empty()).then_some(lang),
                meta: (!meta.is_empty()).then_some(meta),
                value: strip_one_newline(&cb.literal),
            })
        }
        NodeValue::HtmlBlock(hb) => {
            // The separator comrak (and cmark) write between two adjacent
            // lists, or a list and a code block, so they don't merge. It is
            // the renderer's punctuation, not content.
            if hb.literal.trim() == "<!-- end list -->" {
                return Ok(None);
            }
            Node::new(Kind::Html { value: strip_one_newline(&hb.literal) })
        }
        NodeValue::BlockQuote => Node::with_children(Kind::Blockquote, blocks(node, doc)?),
        NodeValue::List(l) => {
            let mut items = Vec::new();
            for item in node.children() {
                let checked = match &item.data().value {
                    NodeValue::Item(_) => None,
                    NodeValue::TaskItem(t) => Some(t.symbol.is_some()),
                    other => return Err(unexpected(other)),
                };
                items.push(Node::with_children(Kind::ListItem { checked }, blocks(item, doc)?));
            }
            Node::with_children(
                Kind::List {
                    ordered: l.list_type == ListType::Ordered,
                    start: if l.list_type == ListType::Ordered { l.start as u64 } else { 1 },
                    spread: !l.tight,
                    marker: None,
                },
                items,
            )
        }
        NodeValue::Table(t) => {
            let align = t
                .alignments
                .iter()
                .map(|a| match a {
                    comrak::nodes::TableAlignment::None => Align::None,
                    comrak::nodes::TableAlignment::Left => Align::Left,
                    comrak::nodes::TableAlignment::Center => Align::Center,
                    comrak::nodes::TableAlignment::Right => Align::Right,
                })
                .collect();
            let mut rows = Vec::new();
            for row in node.children() {
                let mut cells = Vec::new();
                for cell in row.children() {
                    cells.push(Node::with_children(Kind::TableCell, inlines(cell)?));
                }
                rows.push(Node::with_children(Kind::TableRow, cells));
            }
            Node::with_children(Kind::Table { align }, rows)
        }
        NodeValue::FootnoteDefinition(fd) => {
            let def = Node::with_children(Kind::FootnoteDefinition { identifier: fd.name.clone() }, blocks(node, doc)?);
            doc.definitions.push(def);
            return Ok(None);
        }
        other => return Err(unexpected(&other)),
    };
    Ok(Some(n))
}

fn blocks<'a>(node: &'a AstNode<'a>, doc: &mut Doc) -> Result<Vec<Node>, CfmlError> {
    let mut out = Vec::new();
    for c in node.children() {
        if let Some(n) = block(c, doc)? {
            out.push(n);
        }
    }
    Ok(out)
}

fn inlines<'a>(node: &'a AstNode<'a>) -> Result<Vec<Node>, CfmlError> {
    let mut out = Vec::new();
    for c in node.children() {
        out.push(inline(c)?);
    }
    normalise_inlines(&mut out);
    Ok(out)
}

fn inline<'a>(node: &'a AstNode<'a>) -> Result<Node, CfmlError> {
    let value = node.data().value.clone();
    Ok(match value {
        NodeValue::Text(t) => Node::text(t.to_string()),
        NodeValue::SoftBreak => Node::new(Kind::SoftBreak),
        NodeValue::LineBreak => Node::new(Kind::HardBreak),
        NodeValue::Code(c) => Node::new(Kind::InlineCode { value: c.literal }),
        NodeValue::HtmlInline(h) => Node::new(Kind::Html { value: h }),
        NodeValue::Emph => Node::with_children(Kind::Emphasis, inlines(node)?),
        NodeValue::Strong => Node::with_children(Kind::Strong, inlines(node)?),
        NodeValue::Strikethrough => Node::with_children(Kind::Delete, inlines(node)?),
        NodeValue::Link(l) => Node::with_children(
            Kind::Link { url: l.url.clone(), title: (!l.title.is_empty()).then(|| l.title.clone()) },
            inlines(node)?,
        ),
        NodeValue::Image(l) => {
            let mut alt = String::new();
            alt_text(node, &mut alt);
            Node::new(Kind::Image {
                url: l.url.clone(),
                alt,
                title: (!l.title.is_empty()).then(|| l.title.clone()),
            })
        }
        NodeValue::FootnoteReference(r) => Node::new(Kind::FootnoteReference { identifier: r.name.clone() }),
        other => return Err(unexpected(&other)),
    })
}

/// An image's alt text, exactly as comrak's HTML renderer flattens it.
fn alt_text<'a>(node: &'a AstNode<'a>, out: &mut String) {
    for c in node.children() {
        match &c.data().value {
            NodeValue::Text(t) => out.push_str(t),
            NodeValue::Code(code) => out.push_str(&code.literal),
            NodeValue::HtmlInline(h) => out.push_str(h),
            NodeValue::LineBreak | NodeValue::SoftBreak => out.push(' '),
            _ => {}
        }
        alt_text(c, out);
    }
}

fn unexpected(v: &NodeValue) -> CfmlError {
    md_err(format!("Markdown construct [{}] is not supported by this engine.", v.xml_node_name()))
}
