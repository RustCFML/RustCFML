//! The markdown tree that `MarkdownDocument` holds and `toStruct()` exposes.
//!
//! This is our own type rather than comrak's AST for two reasons. comrak's
//! nodes live in an arena with a lifetime, so they cannot sit inside a
//! long-lived native object. And the tree has to carry things comrak has no
//! slot for: block ids, and fields a newer writer added that this version does
//! not understand (kept and handed back, so a round trip loses nothing).
//!
//! Shape follows mdast where that does not fight a closed schema: `depth`,
//! `lang`, `spread`, `checked`, one `align` array per table.

use cfml_common::dynamic::CfmlValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "null" | "default" => Some(Align::None),
            "left" | "l" => Some(Align::Left),
            "center" | "centre" | "c" | "middle" => Some(Align::Center),
            "right" | "r" => Some(Align::Right),
            _ => None,
        }
    }

    pub fn name(self) -> Option<&'static str> {
        match self {
            Align::None => None,
            Align::Left => Some("left"),
            Align::Center => Some("center"),
            Align::Right => Some("right"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    // ---- flow ----
    Paragraph,
    Heading { depth: u8 },
    ThematicBreak,
    Code { lang: Option<String>, meta: Option<String>, value: String },
    /// Raw HTML. Block or inline is decided by where it sits, as in mdast.
    Html { value: String },
    Blockquote,
    List { ordered: bool, start: u64, spread: bool, marker: Option<char> },
    ListItem { checked: Option<bool> },
    Table { align: Vec<Align> },
    TableRow,
    TableCell,
    FootnoteDefinition { identifier: String },
    // ---- phrasing ----
    Text { value: String },
    SoftBreak,
    HardBreak,
    InlineCode { value: String },
    Emphasis,
    Strong,
    Delete,
    Link { url: String, title: Option<String> },
    Image { url: String, alt: String, title: Option<String> },
    FootnoteReference { identifier: String },
}

/// What a node's `children` hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    None,
    Flow,
    Phrasing,
    ListItems,
    TableRows,
    TableCells,
}

impl Content {
    pub fn describe(self) -> &'static str {
        match self {
            Content::None => "no children",
            Content::Flow => "blocks",
            Content::Phrasing => "inline content",
            Content::ListItems => "list items",
            Content::TableRows => "table rows",
            Content::TableCells => "table cells",
        }
    }
}

impl Kind {
    pub fn type_name(&self) -> &'static str {
        match self {
            Kind::Paragraph => "paragraph",
            Kind::Heading { .. } => "heading",
            Kind::ThematicBreak => "thematicBreak",
            Kind::Code { .. } => "code",
            Kind::Html { .. } => "html",
            Kind::Blockquote => "blockquote",
            Kind::List { .. } => "list",
            Kind::ListItem { .. } => "listItem",
            Kind::Table { .. } => "table",
            Kind::TableRow => "tableRow",
            Kind::TableCell => "tableCell",
            Kind::FootnoteDefinition { .. } => "footnoteDefinition",
            Kind::Text { .. } => "text",
            Kind::SoftBreak => "softBreak",
            Kind::HardBreak => "hardBreak",
            Kind::InlineCode { .. } => "inlineCode",
            Kind::Emphasis => "emphasis",
            Kind::Strong => "strong",
            Kind::Delete => "delete",
            Kind::Link { .. } => "link",
            Kind::Image { .. } => "image",
            Kind::FootnoteReference { .. } => "footnoteReference",
        }
    }

    pub fn content(&self) -> Content {
        match self {
            Kind::Paragraph | Kind::Heading { .. } | Kind::TableCell => Content::Phrasing,
            Kind::Emphasis | Kind::Strong | Kind::Delete | Kind::Link { .. } => Content::Phrasing,
            Kind::Blockquote | Kind::ListItem { .. } | Kind::FootnoteDefinition { .. } => Content::Flow,
            Kind::List { .. } => Content::ListItems,
            Kind::Table { .. } => Content::TableRows,
            Kind::TableRow => Content::TableCells,
            _ => Content::None,
        }
    }

    /// Whether this kind may appear where `slot` content is expected.
    pub fn fits(&self, slot: Content) -> bool {
        match slot {
            Content::None => false,
            Content::Flow => matches!(
                self,
                Kind::Paragraph
                    | Kind::Heading { .. }
                    | Kind::ThematicBreak
                    | Kind::Code { .. }
                    | Kind::Html { .. }
                    | Kind::Blockquote
                    | Kind::List { .. }
                    | Kind::Table { .. }
            ),
            Content::Phrasing => matches!(
                self,
                Kind::Text { .. }
                    | Kind::SoftBreak
                    | Kind::HardBreak
                    | Kind::InlineCode { .. }
                    | Kind::Emphasis
                    | Kind::Strong
                    | Kind::Delete
                    | Kind::Link { .. }
                    | Kind::Image { .. }
                    | Kind::Html { .. }
                    | Kind::FootnoteReference { .. }
            ),
            Content::ListItems => matches!(self, Kind::ListItem { .. }),
            Content::TableRows => matches!(self, Kind::TableRow),
            Content::TableCells => matches!(self, Kind::TableCell),
        }
    }

    /// Does a node of this kind, sitting in a `slot`, carry an id?
    ///
    /// Every block-level node does, so any of them can be removed or moved by
    /// id. Inline nodes do not, except links and images: they have scalar
    /// fields (`url`) worth changing in place. Other inline edits go through
    /// `setText` on the enclosing block.
    pub fn takes_id(&self, slot: Content) -> bool {
        match slot {
            Content::Phrasing => matches!(self, Kind::Link { .. } | Kind::Image { .. }),
            _ => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: Option<String>,
    pub kind: Kind,
    pub children: Vec<Node>,
    /// Fields this version does not understand, in the order they arrived.
    /// Preserved through `toStruct()`, ignored by the renderers.
    pub extra: Vec<(String, CfmlValue)>,
}

impl Node {
    pub fn new(kind: Kind) -> Node {
        Node { id: None, kind, children: Vec::new(), extra: Vec::new() }
    }

    pub fn with_children(kind: Kind, children: Vec<Node>) -> Node {
        Node { id: None, kind, children, extra: Vec::new() }
    }

    pub fn text(value: impl Into<String>) -> Node {
        Node::new(Kind::Text { value: value.into() })
    }

    /// The node's text with all markup removed.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        self.push_plain_text(&mut out);
        out
    }

    fn push_plain_text(&self, out: &mut String) {
        match &self.kind {
            Kind::Text { value } | Kind::InlineCode { value } => out.push_str(value),
            Kind::Code { value, .. } => out.push_str(value),
            Kind::Image { alt, .. } => out.push_str(alt),
            Kind::SoftBreak | Kind::HardBreak => out.push(' '),
            _ => {}
        }
        let block_children = matches!(self.kind.content(), Content::Flow | Content::ListItems | Content::TableRows);
        for (i, c) in self.children.iter().enumerate() {
            if i > 0 {
                if block_children {
                    out.push('\n');
                } else if matches!(self.kind, Kind::TableRow) {
                    out.push('\t');
                }
            }
            c.push_plain_text(out);
        }
    }

    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }

    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Node)) {
        f(self);
        for c in &mut self.children {
            c.walk_mut(f);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Doc {
    /// The GitHub Flavored Markdown extensions: tables, strikethrough, task
    /// list items, autolinks.
    pub gfm: bool,
    pub footnotes: bool,
    /// Front matter without its `---` delimiters. Opaque: the renderer writes
    /// it back verbatim and nothing in the tree depends on it.
    pub front_matter: Option<String>,
    pub children: Vec<Node>,
    /// Footnote definitions. Kept off the flow because they do not render
    /// where they are written: they collect at the end of the document.
    pub definitions: Vec<Node>,
    pub extra: Vec<(String, CfmlValue)>,
    /// The highest `bN` id handed out (or loaded).
    pub next_id: u64,
}

impl Doc {
    pub fn new(gfm: bool, footnotes: bool) -> Doc {
        Doc {
            gfm,
            footnotes,
            front_matter: None,
            children: Vec::new(),
            definitions: Vec::new(),
            extra: Vec::new(),
            next_id: 0,
        }
    }

    pub fn mint_id(&mut self) -> String {
        self.next_id += 1;
        format!("b{}", self.next_id)
    }

    /// Bump the id counter past any `bN` id already in the tree, so a minted
    /// id can never collide with a loaded one.
    pub fn note_id(&mut self, id: &str) {
        if let Some(n) = id.strip_prefix('b').and_then(|d| d.parse::<u64>().ok()) {
            if n > self.next_id {
                self.next_id = n;
            }
        }
    }

    /// Give an id to every node that takes one and has none.
    pub fn assign_ids(&mut self, nodes: &mut [Node], slot: Content) {
        for n in nodes.iter_mut() {
            if n.id.is_none() && n.kind.takes_id(slot) {
                n.id = Some(self.mint_id());
            }
            let inner = n.kind.content();
            self.assign_ids(&mut n.children, inner);
        }
    }

    pub fn assign_all_ids(&mut self) {
        let mut children = std::mem::take(&mut self.children);
        self.assign_ids(&mut children, Content::Flow);
        self.children = children;
        let mut defs = std::mem::take(&mut self.definitions);
        for d in defs.iter_mut() {
            if d.id.is_none() {
                d.id = Some(self.mint_id());
            }
            let mut kids = std::mem::take(&mut d.children);
            self.assign_ids(&mut kids, Content::Flow);
            d.children = kids;
        }
        self.definitions = defs;
    }

    /// Every node, flow first then definitions.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node)) {
        for n in &self.children {
            n.walk(f);
        }
        for n in &self.definitions {
            n.walk(f);
        }
    }

    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Node)) {
        for n in &mut self.children {
            n.walk_mut(f);
        }
        for n in &mut self.definitions {
            n.walk_mut(f);
        }
    }

    /// Drop every id, so a subtree can be grafted into another document and
    /// re-minted there without colliding.
    pub fn strip_ids(nodes: &mut [Node]) {
        for n in nodes {
            n.walk_mut(&mut |m| m.id = None);
        }
    }
}

/// Merge adjacent text nodes and drop empty ones, recursively. The parser
/// can split text (around an escape, an entity, a bracket that did not become
/// a link) and builder input can produce runs of literal text; neither is a
/// difference anyone can see.
pub fn normalise_inlines(nodes: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for mut n in nodes.drain(..) {
        normalise_inlines(&mut n.children);
        if let Kind::Text { value } = &n.kind {
            if value.is_empty() && n.extra.is_empty() {
                continue;
            }
            if let Some(Node { kind: Kind::Text { value: prev }, extra, .. }) = out.last_mut() {
                if extra.is_empty() && n.extra.is_empty() {
                    prev.push_str(value);
                    continue;
                }
            }
        }
        out.push(n);
    }
    *nodes = out;
}

/// Trim the edges of every block's inline content. A paragraph, heading or
/// cell can't start or end with whitespace in markdown, so builder input that
/// does (a run array's `"Total: "`) would otherwise not survive a round trip.
pub fn trim_block_edges(nodes: &mut [Node]) {
    for n in nodes.iter_mut() {
        if matches!(n.kind, Kind::Paragraph | Kind::Heading { .. } | Kind::TableCell) {
            normalise_inlines(&mut n.children);
            // Spaces only: a leading tab or no-break space that came from
            // `&#9;` / `&nbsp;` is content.
            if let Some(Node { kind: Kind::Text { value }, .. }) = n.children.first_mut() {
                let t = value.trim_start_matches(' ').to_string();
                *value = t;
            }
            if let Some(Node { kind: Kind::Text { value }, .. }) = n.children.last_mut() {
                let t = value.trim_end_matches(' ').to_string();
                *value = t;
            }
            normalise_inlines(&mut n.children);
        } else {
            trim_block_edges(&mut n.children);
        }
    }
}

impl Doc {
    pub fn tidy(&mut self) {
        trim_block_edges(&mut self.children);
        trim_block_edges(&mut self.definitions);
    }
}
