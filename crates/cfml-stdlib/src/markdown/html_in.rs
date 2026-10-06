//! HTML → markdown tree, for `htmlToMarkdown()` and
//! `MarkdownDocument( html = … )`.
//!
//! Walks the DOM the `html` feature's `scraper` parser builds, so there is no
//! second HTML parser in the binary, and renders through the same path as
//! everything else, so HTML-sourced markdown follows the same round-trip
//! rules. Elements markdown has no word for contribute their content;
//! `script`, `style` and friends are dropped.

use ego_tree::NodeRef;
use scraper::{Html, Node as HNode};

use super::options::MdOptions;
use super::tree::{normalise_inlines, Align, Doc, Kind, Node};

const DROPPED: &[&str] = &[
    "script", "style", "head", "title", "meta", "link", "template", "noscript", "iframe", "object", "embed",
    "svg", "canvas", "button", "select", "option", "textarea", "input",
];

const BLOCKS: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "table", "thead", "tbody",
    "tfoot", "tr", "th", "td", "hr", "div", "section", "article", "header", "footer", "main", "nav", "aside",
    "figure", "figcaption", "form", "fieldset", "details", "summary", "dl", "dt", "dd", "address", "center",
    "body", "html", "caption", "legend", "hgroup",
];

pub fn html_to_doc(html: &str, opts: &MdOptions) -> Doc {
    let dom = if crate::html_dom::CfmlHtmlDocument::looks_like_document(html) {
        Html::parse_document(html)
    } else {
        Html::parse_fragment(html)
    };
    let mut doc = Doc::new(opts.gfm, false);
    let root = dom.root_element();
    let body = root
        .descendants()
        .find(|n| matches!(n.value(), HNode::Element(e) if e.name() == "body"))
        .unwrap_or(*root);
    let conv = Conv { gfm: opts.gfm };
    doc.children = conv.flow(body.children());
    doc
}

fn tag<'a>(n: &NodeRef<'a, HNode>) -> Option<&'a str> {
    match n.value() {
        HNode::Element(e) => Some(e.name()),
        _ => None,
    }
}

fn attr<'a>(n: &NodeRef<'a, HNode>, name: &str) -> Option<&'a str> {
    match n.value() {
        HNode::Element(e) => e.attr(name),
        _ => None,
    }
}

fn is_block(n: &NodeRef<HNode>) -> bool {
    tag(n).map(|t| BLOCKS.contains(&t) || DROPPED.contains(&t)).unwrap_or(false)
}

fn text_content(n: &NodeRef<HNode>) -> String {
    let mut s = String::new();
    for d in n.descendants() {
        if let HNode::Text(t) = d.value() {
            s.push_str(t);
        }
    }
    s
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if c.is_ascii_whitespace() {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

struct Conv {
    gfm: bool,
}

impl Conv {
    fn flow<'a>(&self, nodes: impl Iterator<Item = NodeRef<'a, HNode>>) -> Vec<Node> {
        let mut out = Vec::new();
        let mut pending: Vec<Node> = Vec::new();
        for n in nodes {
            if is_block(&n) {
                flush(&mut pending, &mut out);
                out.extend(self.block(&n));
            } else {
                pending.extend(self.inline(&n, false));
            }
        }
        flush(&mut pending, &mut out);
        out
    }

    fn inlines<'a>(&self, nodes: impl Iterator<Item = NodeRef<'a, HNode>>, in_link: bool) -> Vec<Node> {
        let mut out = Vec::new();
        for n in nodes {
            if is_block(&n) && !DROPPED.contains(&tag(&n).unwrap_or("")) {
                // A block inside inline content: keep its words.
                if !out.is_empty() {
                    out.push(Node::text(" "));
                }
                out.extend(self.inlines(n.children(), in_link));
                out.push(Node::text(" "));
            } else {
                out.extend(self.inline(&n, in_link));
            }
        }
        out
    }

    fn block(&self, n: &NodeRef<HNode>) -> Vec<Node> {
        let t = tag(n).unwrap_or("");
        match t {
            _ if DROPPED.contains(&t) => Vec::new(),
            "p" => para(self.inlines(n.children(), false)),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let depth = t[1..].parse::<u8>().unwrap_or(1);
                let mut inl = self.inlines(n.children(), false);
                super::cfml::breaks_to_spaces(&mut inl);
                let inl = tidy(inl);
                if inl.is_empty() {
                    Vec::new()
                } else {
                    vec![Node::with_children(Kind::Heading { depth }, inl)]
                }
            }
            "ul" | "ol" => self.list(n, t == "ol"),
            "li" => self.list_from_stray_item(n),
            "blockquote" => vec![Node::with_children(Kind::Blockquote, self.flow(n.children()))],
            "pre" => {
                let code = n.children().find(|c| tag(c) == Some("code"));
                let class = code.as_ref().and_then(|c| attr(c, "class")).or_else(|| attr(n, "class")).unwrap_or("");
                let lang = class
                    .split_whitespace()
                    .find_map(|c| c.strip_prefix("language-").or_else(|| c.strip_prefix("lang-")))
                    .map(|s| s.to_string());
                let mut value = text_content(n);
                if value.ends_with('\n') {
                    value.pop();
                }
                vec![Node::new(Kind::Code { lang, meta: None, value })]
            }
            "hr" => vec![Node::new(Kind::ThematicBreak)],
            "table" => self.table(n),
            "dt" => {
                let inl = tidy(self.inlines(n.children(), false));
                if inl.is_empty() {
                    Vec::new()
                } else {
                    vec![Node::with_children(Kind::Paragraph, vec![Node::with_children(Kind::Strong, inl)])]
                }
            }
            "tr" | "thead" | "tbody" | "tfoot" | "th" | "td" | "caption" => para(self.inlines(n.children(), false)),
            _ => self.flow(n.children()),
        }
    }

    fn list(&self, n: &NodeRef<HNode>, ordered: bool) -> Vec<Node> {
        let start = if ordered { attr(n, "start").and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(1) } else { 1 };
        let mut items = Vec::new();
        let mut spread = false;
        for c in n.children() {
            match tag(&c) {
                Some("li") => {
                    if c.children().any(|g| tag(&g) == Some("p")) {
                        spread = true;
                    }
                    items.push(self.item(&c));
                }
                Some(_) => {
                    let blocks = self.flow(std::iter::once(c));
                    if !blocks.is_empty() {
                        items.push(Node::with_children(Kind::ListItem { checked: None }, blocks));
                    }
                }
                None => {
                    if !text_content(&c).trim().is_empty() {
                        let blocks = self.flow(std::iter::once(c));
                        items.push(Node::with_children(Kind::ListItem { checked: None }, blocks));
                    }
                }
            }
        }
        if items.is_empty() {
            return Vec::new();
        }
        vec![Node::with_children(Kind::List { ordered, start, spread, marker: None }, items)]
    }

    fn list_from_stray_item(&self, n: &NodeRef<HNode>) -> Vec<Node> {
        vec![Node::with_children(
            Kind::List { ordered: false, start: 1, spread: false, marker: None },
            vec![self.item(n)],
        )]
    }

    fn item(&self, li: &NodeRef<HNode>) -> Node {
        // A task list item: a checkbox before any text.
        let mut checked = None;
        if self.gfm {
            for d in li.descendants() {
                match d.value() {
                    HNode::Text(t) if !t.trim().is_empty() => break,
                    HNode::Element(e) if e.name() == "input" => {
                        if e.attr("type").map(|t| t.eq_ignore_ascii_case("checkbox")).unwrap_or(false) {
                            checked = Some(e.attr("checked").is_some());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
        Node::with_children(Kind::ListItem { checked }, self.flow(li.children()))
    }

    fn table(&self, n: &NodeRef<HNode>) -> Vec<Node> {
        if !self.gfm {
            return self.flow(n.children());
        }
        let mut rows: Vec<NodeRef<HNode>> = Vec::new();
        for c in n.children() {
            match tag(&c) {
                Some("tr") => rows.push(c),
                Some("thead") | Some("tbody") | Some("tfoot") => {
                    rows.extend(c.children().filter(|r| tag(r) == Some("tr")));
                }
                _ => {}
            }
        }
        if rows.is_empty() {
            return Vec::new();
        }
        let mut grid: Vec<Vec<Node>> = Vec::new();
        let mut align: Vec<Align> = Vec::new();
        for (ri, r) in rows.iter().enumerate() {
            let mut cells = Vec::new();
            for cell in r.children().filter(|c| matches!(tag(c), Some("th") | Some("td"))) {
                if ri == 0 {
                    let a = attr(&cell, "align")
                        .and_then(Align::parse)
                        .or_else(|| {
                            attr(&cell, "style").and_then(|s| {
                                s.split(';').find_map(|decl| {
                                    let (k, v) = decl.split_once(':')?;
                                    if k.trim().eq_ignore_ascii_case("text-align") {
                                        Align::parse(v)
                                    } else {
                                        None
                                    }
                                })
                            })
                        })
                        .unwrap_or(Align::None);
                    align.push(a);
                }
                let mut inl = self.inlines(cell.children(), false);
                super::cfml::breaks_to_spaces(&mut inl);
                cells.push(Node::with_children(Kind::TableCell, tidy(inl)));
            }
            grid.push(cells);
        }
        let width = grid.iter().map(|r| r.len()).max().unwrap_or(0);
        if width == 0 {
            return Vec::new();
        }
        align.resize(width, Align::None);
        let rows = grid
            .into_iter()
            .map(|mut r| {
                while r.len() < width {
                    r.push(Node::new(Kind::TableCell));
                }
                Node::with_children(Kind::TableRow, r)
            })
            .collect();
        vec![Node::with_children(Kind::Table { align }, rows)]
    }

    fn inline(&self, n: &NodeRef<HNode>, in_link: bool) -> Vec<Node> {
        match n.value() {
            HNode::Text(t) => vec![Node::text(collapse_ws(t))],
            HNode::Element(e) => {
                let t = e.name();
                let wrap = |k: Kind| {
                    let inner = self.inlines(n.children(), in_link);
                    if inner.iter().all(|c| matches!(&c.kind, Kind::Text { value } if value.trim().is_empty())) {
                        inner
                    } else {
                        vec![Node::with_children(k, inner)]
                    }
                };
                match t {
                    _ if DROPPED.contains(&t) => Vec::new(),
                    "br" => vec![Node::new(Kind::HardBreak)],
                    "em" | "i" | "cite" | "dfn" | "var" => wrap(Kind::Emphasis),
                    "strong" | "b" => wrap(Kind::Strong),
                    "del" | "s" | "strike" if self.gfm => wrap(Kind::Delete),
                    "code" | "kbd" | "samp" | "tt" => {
                        let v = collapse_ws(&text_content(n));
                        if v.is_empty() {
                            Vec::new()
                        } else {
                            vec![Node::new(Kind::InlineCode { value: v })]
                        }
                    }
                    "a" => match e.attr("href") {
                        Some(href) if !in_link => {
                            let inner = self.inlines(n.children(), true);
                            let empty = inner.iter().all(|c| matches!(&c.kind, Kind::Text { value } if value.trim().is_empty()));
                            if empty {
                                Vec::new()
                            } else {
                                vec![Node::with_children(
                                    Kind::Link {
                                        url: href.to_string(),
                                        title: e.attr("title").map(|s| s.to_string()).filter(|s| !s.is_empty()),
                                    },
                                    inner,
                                )]
                            }
                        }
                        _ => self.inlines(n.children(), in_link),
                    },
                    "img" => match e.attr("src") {
                        Some(src) => vec![Node::new(Kind::Image {
                            url: src.to_string(),
                            alt: e.attr("alt").unwrap_or("").to_string(),
                            title: e.attr("title").map(|s| s.to_string()).filter(|s| !s.is_empty()),
                        })],
                        None => Vec::new(),
                    },
                    _ => self.inlines(n.children(), in_link),
                }
            }
            _ => Vec::new(),
        }
    }
}

fn para(inl: Vec<Node>) -> Vec<Node> {
    let inl = tidy(inl);
    if inl.is_empty() {
        Vec::new()
    } else {
        vec![Node::with_children(Kind::Paragraph, inl)]
    }
}

fn flush(pending: &mut Vec<Node>, out: &mut Vec<Node>) {
    if pending.is_empty() {
        return;
    }
    out.extend(para(std::mem::take(pending)));
}

/// Merge text, move edge whitespace out of emphasis/strong/delete/link
/// (markdown can't open emphasis with a space), trim the ends, drop spaces
/// around hard breaks.
fn tidy(mut nodes: Vec<Node>) -> Vec<Node> {
    normalise_inlines(&mut nodes);
    nodes = hoist_spaces(nodes);
    normalise_inlines(&mut nodes);
    // collapse double spaces created by hoisting
    for n in nodes.iter_mut() {
        if let Kind::Text { value } = &mut n.kind {
            *value = collapse_ws(value);
        }
    }
    trim_edges(&mut nodes);
    // spaces next to a hard break
    for i in 0..nodes.len() {
        if matches!(nodes[i].kind, Kind::HardBreak) {
            if i > 0 {
                if let Kind::Text { value } = &mut nodes[i - 1].kind {
                    *value = value.trim_end().to_string();
                }
            }
            if i + 1 < nodes.len() {
                if let Kind::Text { value } = &mut nodes[i + 1].kind {
                    *value = value.trim_start().to_string();
                }
            }
        }
    }
    // A hard break can't end a paragraph.
    while matches!(nodes.last().map(|n| &n.kind), Some(Kind::HardBreak)) {
        nodes.pop();
    }
    while matches!(nodes.first().map(|n| &n.kind), Some(Kind::HardBreak)) {
        nodes.remove(0);
    }
    normalise_inlines(&mut nodes);
    nodes
}

fn trim_edges(nodes: &mut Vec<Node>) {
    if let Some(Node { kind: Kind::Text { value }, .. }) = nodes.first_mut() {
        *value = value.trim_start().to_string();
    }
    if let Some(Node { kind: Kind::Text { value }, .. }) = nodes.last_mut() {
        *value = value.trim_end().to_string();
    }
    normalise_inlines(nodes);
}

fn hoist_spaces(nodes: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for mut n in nodes {
        if matches!(n.kind, Kind::Emphasis | Kind::Strong | Kind::Delete | Kind::Link { .. }) {
            n.children = hoist_spaces(std::mem::take(&mut n.children));
            normalise_inlines(&mut n.children);
            let lead = match n.children.first_mut() {
                Some(Node { kind: Kind::Text { value }, .. }) if value.starts_with(' ') => {
                    *value = value.trim_start().to_string();
                    true
                }
                _ => false,
            };
            let trail = match n.children.last_mut() {
                Some(Node { kind: Kind::Text { value }, .. }) if value.ends_with(' ') => {
                    *value = value.trim_end().to_string();
                    true
                }
                _ => false,
            };
            normalise_inlines(&mut n.children);
            if lead {
                out.push(Node::text(" "));
            }
            if !n.children.is_empty() || matches!(n.kind, Kind::Link { .. }) {
                out.push(n);
            }
            if trail {
                out.push(Node::text(" "));
            }
        } else {
            out.push(n);
        }
    }
    out
}
