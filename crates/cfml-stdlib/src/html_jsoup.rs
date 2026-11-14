//! jsoup 1.10.1's HTML parse shape and serialiser, over the `HtmlDocument()` DOM.
//!
//! The `org.jsoup` adapter (`cfml-vm/src/jsoup_shim.rs`) is used by code that
//! compares the serialised HTML exactly — Preside's email style inliner and
//! click tracker among them — so "equivalent markup" is not enough: the bytes
//! must be the ones jsoup 1.10.1 (the jar Preside ships) writes. This module is
//! a port of that version's output path, not an approximation of it:
//!
//! | jsoup 1.10.1 source                         | here                          |
//! |---------------------------------------------|-------------------------------|
//! | `nodes/Node.outerHtml`, `indent`, `NodeTraversor` | [`outer_html`], [`indent`] |
//! | `nodes/Element.outerHtmlHead/Tail`, `html()`, `preserveWhitespace` | [`element_head`], [`element_tail`], [`inner_html`], [`preserve_whitespace`] |
//! | `nodes/TextNode.outerHtmlHead`              | [`text_head`]                 |
//! | `nodes/DataNode`, `Comment`, `DocumentType` | arms of [`head`]              |
//! | `nodes/Attribute.html`, `shouldCollapseAttribute` | [`attribute`]           |
//! | `nodes/Entities.escape`, `canEncode`, `EscapeMode` | [`escape`], [`BASE_ENTITIES`] |
//! | `nodes/Document.OutputSettings`             | [`OutputSettings`]            |
//! | `parser/Tag.java` tag tables                | [`tag`]                       |
//! | `helper/StringUtil.isWhitespace`, `isBlank` | [`is_ws`], [`is_blank`]       |
//!
//! Later jsoup versions rewrote pretty-printing; match 1.10.1 specifically.
//!
//! # Parse shape
//!
//! Serialisation is only byte-exact if the tree is jsoup's tree, and html5ever's
//! differs from jsoup's in two ways the output can see, so [`parse`] drives
//! html5ever through two shims:
//!
//! * **Text nodes are not merged across tokens.** jsoup's
//!   `HtmlTreeBuilder.insert(Token.Character)` appends a NEW `TextNode` for every
//!   character token, while html5ever's sink concatenates onto a preceding text
//!   sibling. The difference shows under pretty-printing, which normalises each
//!   text node on its own: the `"\n"` before `</body>` and the `"\n"` after it
//!   are two nodes in jsoup and print as two spaces, one merged node here would
//!   print as one. So a text append into a parent whose last text arrived before
//!   an intervening non-character token starts a new node.
//! * **Valueless attributes.** jsoup 1.10.1 keeps an attribute written with no
//!   value (`<video controls>`) as a `BooleanAttribute`, which writes back as the
//!   bare key whatever its name; `controls=""` is an ordinary attribute and
//!   writes back with its value. html5ever's tokens carry `""` for both, so the
//!   start tag's source text is re-read to tell them apart ([`valueless_attrs`]).
//! * **Self-closing start tags.** jsoup's `insert(Token.StartTag)` inserts a
//!   self-closing tag empty and immediately emits its end tag, so `<div/>x` is
//!   `<div></div>x`; and an *unknown* tag written self-closing (`<circle/>`)
//!   keeps that flag and is written back as `<circle />`.
//!
//! Scripting is off, as in jsoup: `<noscript>` content parses as elements.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ego_tree::{NodeId, NodeRef};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, CharacterTokens, EndTag, NullCharacterToken, ParseError, Tag,
    TagToken, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts, TokenizerResult,
};
use html5ever::tree_builder::{
    ElementFlags, NextParserState, NodeOrText, QuirksMode, TreeBuilder, TreeBuilderOpts, TreeSink,
};
use html5ever::{ns, namespace_url, Attribute, LocalName, QualName};
use scraper::{Html, HtmlTreeSink, Node};

// ---------------------------------------------------------------------------
// Output settings
// ---------------------------------------------------------------------------

/// The charset classes jsoup's `Entities.canEncode` distinguishes, plus
/// ISO-8859-1, whose encoder answers `c < 0x100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    Ascii,
    Utf,
    Latin1,
}

impl Charset {
    /// Java's `Charset.forName` for the charsets this adapter can encode to,
    /// with the alias spellings Java accepts. Anything else is an error, as it is
    /// in Java (`UnsupportedCharsetException`) — silently writing UTF-8 instead
    /// would leave characters unescaped that the caller asked to have escaped.
    pub fn for_name(name: &str) -> Option<Charset> {
        let n = name.trim().to_ascii_lowercase();
        match n.as_str() {
            "ascii" | "us-ascii" | "us_ascii" | "iso646-us" | "iso_646.irv:1983"
            | "ansi_x3.4-1968" | "ansi_x3.4-1986" | "cp367" | "ibm367" | "csascii" | "646"
            | "us" | "iso-ir-6" | "ascii7" | "default" => Some(Charset::Ascii),
            "iso-8859-1" | "iso8859-1" | "iso8859_1" | "iso_8859-1" | "iso_8859_1" | "latin1"
            | "l1" | "cp819" | "ibm819" | "ibm-819" | "819" | "8859_1" | "csisolatin1"
            | "iso-ir-100" | "iso_8859-1:1987" => Some(Charset::Latin1),
            _ if n.starts_with("utf") => Some(Charset::Utf),
            _ => None,
        }
    }

    /// The name Java's `Charset.name()` reports.
    pub fn name(self) -> &'static str {
        match self {
            Charset::Ascii => "US-ASCII",
            Charset::Utf => "UTF-8",
            Charset::Latin1 => "ISO-8859-1",
        }
    }

    fn can_encode(self, c: char) -> bool {
        match self {
            Charset::Ascii => (c as u32) < 0x80,
            Charset::Utf => true,
            Charset::Latin1 => (c as u32) < 0x100,
        }
    }
}

/// `Entities.EscapeMode`. `extended` names every character in the full HTML
/// entity table; this adapter carries only the `base` table, so `extended`
/// names the Latin-1 range and writes everything else numerically
/// (known-issues §66).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeMode {
    Base,
    Xhtml,
    Extended,
}

/// `Document.OutputSettings`, defaults as in jsoup 1.10.1.
#[derive(Debug, Clone)]
pub struct OutputSettings {
    pub pretty_print: bool,
    pub outline: bool,
    pub indent_amount: usize,
    pub charset: Charset,
    pub escape_mode: EscapeMode,
    /// `Syntax.xml`: void elements as `<br />`, no boolean-attribute collapsing,
    /// and the upper-case doctype.
    pub xml_syntax: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        OutputSettings {
            pretty_print: true,
            outline: false,
            indent_amount: 1,
            charset: Charset::Utf,
            escape_mode: EscapeMode::Base,
            xml_syntax: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Tag tables (parser/Tag.java)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct TagInfo {
    known: bool,
    format_as_block: bool,
    /// `Tag.isEmpty` — a void element.
    empty: bool,
    preserve_whitespace: bool,
}

const BLOCK_TAGS: &[&str] = &[
    "html", "head", "body", "frameset", "script", "noscript", "style", "meta", "link", "title",
    "frame", "noframes", "section", "nav", "aside", "hgroup", "header", "footer", "p", "h1", "h2",
    "h3", "h4", "h5", "h6", "ul", "ol", "pre", "div", "blockquote", "hr", "address", "figure",
    "figcaption", "form", "fieldset", "ins", "del", "s", "dl", "dt", "dd", "li", "table",
    "caption", "thead", "tfoot", "tbody", "colgroup", "col", "tr", "th", "td", "video", "audio",
    "canvas", "details", "menu", "plaintext", "template", "article", "main", "svg", "math",
];
const INLINE_TAGS: &[&str] = &[
    "object", "base", "font", "tt", "i", "b", "u", "big", "small", "em", "strong", "dfn", "code",
    "samp", "kbd", "var", "cite", "abbr", "time", "acronym", "mark", "ruby", "rt", "rp", "a",
    "img", "br", "wbr", "map", "q", "sub", "sup", "bdo", "iframe", "embed", "span", "input",
    "select", "textarea", "label", "button", "optgroup", "option", "legend", "datalist", "keygen",
    "output", "progress", "meter", "area", "param", "source", "track", "summary", "command",
    "device", "basefont", "bgsound", "menuitem", "data", "bdi",
];
const EMPTY_TAGS: &[&str] = &[
    "meta", "link", "base", "frame", "img", "br", "wbr", "embed", "hr", "input", "keygen", "col",
    "command", "device", "area", "basefont", "bgsound", "menuitem", "param", "source", "track",
];
const FORMAT_AS_INLINE_TAGS: &[&str] = &[
    "title", "a", "p", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "address", "li", "th", "td",
    "script", "style", "ins", "del", "s",
];
const PRESERVE_WHITESPACE_TAGS: &[&str] = &["pre", "plaintext", "title", "textarea"];

/// `Attribute.booleanAttributes`.
const BOOLEAN_ATTRIBUTES: &[&str] = &[
    "allowfullscreen", "async", "autofocus", "checked", "compact", "declare", "default", "defer",
    "disabled", "formnovalidate", "hidden", "inert", "ismap", "itemscope", "multiple", "muted",
    "nohref", "noresize", "noshade", "novalidate", "nowrap", "open", "readonly", "required",
    "reversed", "seamless", "selected", "sortable", "truespeed", "typemustmatch",
];

/// `Tag.valueOf(name)`. An unknown tag is inline but formatted as a block —
/// `Tag`'s field defaults, with only `isBlock` cleared. The Document itself is
/// the unknown tag `#root`.
fn tag(name: &str) -> TagInfo {
    let block = BLOCK_TAGS.contains(&name);
    let inline = INLINE_TAGS.contains(&name);
    if !block && !inline {
        return TagInfo {
            known: false,
            format_as_block: true,
            empty: false,
            preserve_whitespace: false,
        };
    }
    TagInfo {
        known: true,
        format_as_block: block && !FORMAT_AS_INLINE_TAGS.contains(&name),
        empty: EMPTY_TAGS.contains(&name),
        preserve_whitespace: PRESERVE_WHITESPACE_TAGS.contains(&name),
    }
}

/// jsoup normalises tag and attribute names to lower case at parse time
/// (`ParseSettings.htmlDefault`), including the SVG names html5ever re-cases
/// (`viewBox`, `foreignObject`).
fn tag_name(name: &QualName) -> String {
    name.local.as_ref().to_ascii_lowercase()
}

/// `true` when this unknown-to-jsoup tag name marks a self-closing element.
fn is_known(name: &str) -> bool {
    tag(name).known
}

// ---------------------------------------------------------------------------
// Entities (nodes/Entities.java, entities-base.properties)
// ---------------------------------------------------------------------------

/// `EscapeMode.base`'s codepoint → name table. Where two names share a
/// codepoint (`AMP`/`amp`) jsoup emits the later, lower-case one.
const BASE_ENTITIES: &[(u32, &str)] = &[
    (0x22, "quot"), (0x26, "amp"), (0x3c, "lt"), (0x3e, "gt"), (0xa0, "nbsp"), (0xa1, "iexcl"),
    (0xa2, "cent"), (0xa3, "pound"), (0xa4, "curren"), (0xa5, "yen"), (0xa6, "brvbar"),
    (0xa7, "sect"), (0xa8, "uml"), (0xa9, "copy"), (0xaa, "ordf"), (0xab, "laquo"), (0xac, "not"),
    (0xad, "shy"), (0xae, "reg"), (0xaf, "macr"), (0xb0, "deg"), (0xb1, "plusmn"), (0xb2, "sup2"),
    (0xb3, "sup3"), (0xb4, "acute"), (0xb5, "micro"), (0xb6, "para"), (0xb7, "middot"),
    (0xb8, "cedil"), (0xb9, "sup1"), (0xba, "ordm"), (0xbb, "raquo"), (0xbc, "frac14"),
    (0xbd, "frac12"), (0xbe, "frac34"), (0xbf, "iquest"), (0xc0, "Agrave"), (0xc1, "Aacute"),
    (0xc2, "Acirc"), (0xc3, "Atilde"), (0xc4, "Auml"), (0xc5, "Aring"), (0xc6, "AElig"),
    (0xc7, "Ccedil"), (0xc8, "Egrave"), (0xc9, "Eacute"), (0xca, "Ecirc"), (0xcb, "Euml"),
    (0xcc, "Igrave"), (0xcd, "Iacute"), (0xce, "Icirc"), (0xcf, "Iuml"), (0xd0, "ETH"),
    (0xd1, "Ntilde"), (0xd2, "Ograve"), (0xd3, "Oacute"), (0xd4, "Ocirc"), (0xd5, "Otilde"),
    (0xd6, "Ouml"), (0xd7, "times"), (0xd8, "Oslash"), (0xd9, "Ugrave"), (0xda, "Uacute"),
    (0xdb, "Ucirc"), (0xdc, "Uuml"), (0xdd, "Yacute"), (0xde, "THORN"), (0xdf, "szlig"),
    (0xe0, "agrave"), (0xe1, "aacute"), (0xe2, "acirc"), (0xe3, "atilde"), (0xe4, "auml"),
    (0xe5, "aring"), (0xe6, "aelig"), (0xe7, "ccedil"), (0xe8, "egrave"), (0xe9, "eacute"),
    (0xea, "ecirc"), (0xeb, "euml"), (0xec, "igrave"), (0xed, "iacute"), (0xee, "icirc"),
    (0xef, "iuml"), (0xf0, "eth"), (0xf1, "ntilde"), (0xf2, "ograve"), (0xf3, "oacute"),
    (0xf4, "ocirc"), (0xf5, "otilde"), (0xf6, "ouml"), (0xf7, "divide"), (0xf8, "oslash"),
    (0xf9, "ugrave"), (0xfa, "uacute"), (0xfb, "ucirc"), (0xfc, "uuml"), (0xfd, "yacute"),
    (0xfe, "thorn"), (0xff, "yuml"),
];

/// `EscapeMode.xhtml`'s table.
const XHTML_ENTITIES: &[(u32, &str)] = &[(0x22, "quot"), (0x26, "amp"), (0x3c, "lt"), (0x3e, "gt")];

/// `Entities.appendEncoded`: the mode's name if it has one, else `&#x…;`.
fn append_encoded(accum: &mut String, mode: EscapeMode, c: char) {
    let table = match mode {
        EscapeMode::Xhtml => XHTML_ENTITIES,
        EscapeMode::Base | EscapeMode::Extended => BASE_ENTITIES,
    };
    match table.binary_search_by_key(&(c as u32), |(cp, _)| *cp) {
        Ok(i) => {
            accum.push('&');
            accum.push_str(table[i].1);
            accum.push(';');
        }
        Err(_) => {
            accum.push_str(&format!("&#x{:x};", c as u32));
        }
    }
}

/// `StringUtil.isWhitespace` — note: NOT Unicode whitespace, and not U+00A0.
fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{c}' | '\r')
}

fn is_blank(s: &str) -> bool {
    s.chars().all(is_ws)
}

/// `Entities.escape(accum, string, out, inAttribute, normaliseWhite, stripLeadingWhite)`.
fn escape(
    accum: &mut String,
    s: &str,
    out: &OutputSettings,
    in_attribute: bool,
    normalise_white: bool,
    strip_leading_white: bool,
) {
    let mut last_was_white = false;
    let mut reached_non_white = false;
    let xhtml = out.escape_mode == EscapeMode::Xhtml;
    for c in s.chars() {
        if normalise_white {
            if is_ws(c) {
                if (strip_leading_white && !reached_non_white) || last_was_white {
                    continue;
                }
                accum.push(' ');
                last_was_white = true;
                continue;
            }
            last_was_white = false;
            reached_non_white = true;
        }
        match c {
            '&' => accum.push_str("&amp;"),
            '\u{a0}' => accum.push_str(if xhtml { "&#xa0;" } else { "&nbsp;" }),
            // Not needed in an HTML attribute value; required in XML.
            '<' if !in_attribute || xhtml => accum.push_str("&lt;"),
            '>' if !in_attribute => accum.push_str("&gt;"),
            '"' if in_attribute => accum.push_str("&quot;"),
            c if out.charset.can_encode(c) => accum.push(c),
            c => append_encoded(accum, out.escape_mode, c),
        }
    }
}

// ---------------------------------------------------------------------------
// Serialisation
// ---------------------------------------------------------------------------

/// What jsoup's tree records that html5ever's does not.
#[derive(Debug, Default)]
pub struct JsoupMeta {
    /// Unknown tags written `<x/>`, which jsoup writes back as `<x />`.
    pub self_closing: HashSet<NodeId>,
    /// Per element, the (lower-case) attributes written with no value.
    pub valueless: HashMap<NodeId, HashSet<String>>,
}

impl JsoupMeta {
    /// `Element.attr( key, value )` replaces a BooleanAttribute with an
    /// ordinary one, which writes its value back.
    pub fn attribute_set(&mut self, id: NodeId, key: &str) {
        if let Some(set) = self.valueless.get_mut(&id) {
            set.remove(&key.to_ascii_lowercase());
        }
    }
}

/// Everything the serialiser needs besides the node.
pub struct Ctx<'a> {
    pub out: &'a OutputSettings,
    pub meta: &'a JsoupMeta,
}

/// `Node.indent`.
fn indent(accum: &mut String, depth: usize, out: &OutputSettings) {
    accum.push('\n');
    accum.extend(std::iter::repeat_n(' ', depth * out.indent_amount));
}

/// The parent as jsoup's `parent()` sees it: an Element, or the Document, which
/// in jsoup IS an Element (tag `#root`).
enum Parent {
    Element(String),
    Document,
    None,
}

fn parent_of(node: &NodeRef<Node>) -> Parent {
    match node.parent() {
        Some(p) => match p.value() {
            Node::Element(e) => Parent::Element(tag_name(&e.name)),
            Node::Document | Node::Fragment => Parent::Document,
            _ => Parent::None,
        },
        None => Parent::None,
    }
}

fn parent_formats_as_block(node: &NodeRef<Node>) -> bool {
    match parent_of(node) {
        Parent::Element(name) => tag(&name).format_as_block,
        Parent::Document => tag("#root").format_as_block,
        Parent::None => false,
    }
}

/// `Element.preserveWhitespace(node)`: the element's own tag, or its parent's —
/// two levels only, exactly as jsoup 1.10.1 checks.
fn preserve_whitespace(node: Option<NodeRef<Node>>) -> bool {
    let Some(node) = node else { return false };
    let Node::Element(e) = node.value() else { return false };
    if tag(&tag_name(&e.name)).preserve_whitespace {
        return true;
    }
    match node.parent().as_ref().map(|p| p.value()) {
        Some(Node::Element(p)) => tag(&tag_name(&p.name)).preserve_whitespace,
        _ => false,
    }
}

/// `Attribute.html`.
/// `valueless` is jsoup's `BooleanAttribute` (written with no value), whose
/// `shouldCollapseAttribute` is overridden to always collapse.
fn attribute(accum: &mut String, name: &QualName, value: &str, valueless: bool, out: &OutputSettings) {
    let key = match &name.prefix {
        Some(p) => format!("{}:{}", p, name.local).to_ascii_lowercase(),
        None => name.local.as_ref().to_ascii_lowercase(),
    };
    accum.push_str(&key);
    let collapse = (valueless && value.is_empty())
        || ((value.is_empty() || value.eq_ignore_ascii_case(&key))
            && !out.xml_syntax
            && BOOLEAN_ATTRIBUTES.binary_search(&key.as_str()).is_ok());
    if !collapse {
        accum.push_str("=\"");
        escape(accum, value, out, true, false, false);
        accum.push('"');
    }
}

fn is_self_closing(node: &NodeRef<Node>, name: &str, ctx: &Ctx) -> bool {
    tag(name).empty || ctx.meta.self_closing.contains(&node.id())
}

/// `Element.outerHtmlHead`.
fn element_head(accum: &mut String, node: &NodeRef<Node>, depth: usize, ctx: &Ctx) {
    let Node::Element(e) = node.value() else { return };
    let out = ctx.out;
    let name = tag_name(&e.name);
    let t = tag(&name);
    if out.pretty_print
        && (t.format_as_block || parent_formats_as_block(node) || out.outline)
        && !accum.is_empty()
    {
        indent(accum, depth, out);
    }
    accum.push('<');
    accum.push_str(&name);
    let valueless = ctx.meta.valueless.get(&node.id());
    for (k, v) in e.attrs.iter() {
        accum.push(' ');
        let bare = valueless.is_some_and(|set| set.contains(&k.local.as_ref().to_ascii_lowercase()));
        attribute(accum, k, v, bare, out);
    }
    if !node.has_children() && is_self_closing(node, &name, ctx) {
        if !out.xml_syntax && t.empty {
            accum.push('>');
        } else {
            accum.push_str(" />");
        }
    } else {
        accum.push('>');
    }
}

/// `Element.outerHtmlTail`.
fn element_tail(accum: &mut String, node: &NodeRef<Node>, depth: usize, ctx: &Ctx) {
    let Node::Element(e) = node.value() else { return };
    let out = ctx.out;
    let name = tag_name(&e.name);
    if !node.has_children() && is_self_closing(node, &name, ctx) {
        return;
    }
    let children = node.children().count();
    if out.pretty_print && children > 0 {
        let only_text = children == 1
            && node.first_child().is_some_and(|c| c.value().is_text())
            && !is_data_parent(node);
        if tag(&name).format_as_block || (out.outline && (children > 1 || !only_text)) {
            indent(accum, depth, out);
        }
    }
    accum.push_str("</");
    accum.push_str(&name);
    accum.push('>');
}

/// jsoup puts character data inside `<script>` and `<style>` into DataNodes
/// (`HtmlTreeBuilder.insert(Token.Character)`), which are written raw and are
/// not `TextNode`s for the formatting rules.
fn is_data_parent(parent: &NodeRef<Node>) -> bool {
    match parent.value() {
        Node::Element(e) => matches!(tag_name(&e.name).as_str(), "script" | "style"),
        _ => false,
    }
}

/// `TextNode.outerHtmlHead` (or `DataNode`'s, for script/style content).
fn text_head(accum: &mut String, node: &NodeRef<Node>, text: &str, depth: usize, out: &OutputSettings) {
    let parent = node.parent();
    if parent.as_ref().is_some_and(is_data_parent) {
        accum.push_str(text);
        return;
    }
    let first = node.prev_sibling().is_none();
    let blank = is_blank(text);
    let parent_is_element = !matches!(parent_of(node), Parent::None);
    let siblings = parent.as_ref().map(|p| p.children().count().saturating_sub(1)).unwrap_or(0);
    if out.pretty_print
        && ((first && parent_is_element && parent_formats_as_block(node) && !blank)
            || (out.outline && siblings > 0 && !blank))
    {
        indent(accum, depth, out);
    }
    let parent_elem = parent.filter(|p| matches!(p.value(), Node::Element(_)));
    let normalise = out.pretty_print && parent_is_element && !preserve_whitespace(parent_elem);
    escape(accum, text, out, false, normalise, false);
}

/// The visitor's `head(node, depth)`.
fn head(accum: &mut String, node: &NodeRef<Node>, depth: usize, ctx: &Ctx) {
    let out = ctx.out;
    match node.value() {
        Node::Element(_) => element_head(accum, node, depth, ctx),
        Node::Text(t) => text_head(accum, node, t, depth, out),
        Node::Comment(c) => {
            if out.pretty_print {
                indent(accum, depth, out);
            }
            accum.push_str("<!--");
            accum.push_str(c);
            accum.push_str("-->");
        }
        Node::Doctype(d) => {
            let has = |s: &str| !is_blank(s);
            // "looks like a html5 doctype, go lowercase for aesthetics"
            if !out.xml_syntax && !has(&d.public_id) && !has(&d.system_id) {
                accum.push_str("<!doctype");
            } else {
                accum.push_str("<!DOCTYPE");
            }
            if has(&d.name) {
                accum.push(' ');
                accum.push_str(&d.name);
            }
            if has(&d.public_id) {
                accum.push_str(" PUBLIC \"");
                accum.push_str(&d.public_id);
                accum.push('"');
            }
            if has(&d.system_id) {
                accum.push_str(" \"");
                accum.push_str(&d.system_id);
                accum.push('"');
            }
            accum.push('>');
        }
        Node::ProcessingInstruction(pi) => {
            // jsoup's HTML parser makes `<?…>` a bogus comment; html5ever does
            // too, so this is only reachable through XML-ish input.
            accum.push_str("<?");
            accum.push_str(&pi.target);
            accum.push(' ');
            accum.push_str(&pi.data);
            accum.push_str("?>");
        }
        Node::Document | Node::Fragment => {}
    }
}

/// `Node.outerHtml(accum)`: a depth-first traversal from `node` at depth 0,
/// appending to a SHARED accumulator — the `accum.length() > 0` test in
/// `Element.outerHtmlHead` reads it, so a first block element is not indented.
pub fn outer_html(accum: &mut String, node: NodeRef<Node>, ctx: &Ctx) {
    let mut depth = 0usize;
    for edge in node.traverse() {
        match edge {
            ego_tree::iter::Edge::Open(n) => {
                head(accum, &n, depth, ctx);
                depth += 1;
            }
            ego_tree::iter::Edge::Close(n) => {
                depth -= 1;
                if let Node::Element(_) = n.value() {
                    element_tail(accum, &n, depth, ctx);
                }
            }
        }
    }
}

/// `Element.html()`: each child's outer HTML into one accumulator, trimmed
/// when pretty-printing (Java's `String.trim()`, which strips every char ≤ U+0020).
/// `Document.html()`/`toString()` are this over the document's children.
pub fn inner_html(node: NodeRef<Node>, ctx: &Ctx) -> String {
    let mut accum = String::new();
    for child in node.children() {
        outer_html(&mut accum, child, ctx);
    }
    if ctx.out.pretty_print {
        accum.trim_matches(|c: char| c <= ' ').to_string()
    } else {
        accum
    }
}

// ---------------------------------------------------------------------------
// Parsing in jsoup's tree shape
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ParseState {
    /// Bumped for every non-character token.
    epoch: Cell<u64>,
    /// Per parent, the epoch at which text was last appended to it.
    last_text: RefCell<HashMap<NodeId, u64>>,
    /// The start tag being processed: its name, whether it is self-closing,
    /// and its attributes written with no value.
    pending: RefCell<Option<(LocalName, bool, HashSet<String>)>>,
    /// The element created for it, and whether it is in the HTML namespace.
    created: Cell<Option<(NodeId, bool)>>,
    meta: RefCell<JsoupMeta>,
    /// The source and how far into it start/end tags have been matched.
    source: String,
    cursor: Cell<usize>,
    /// `parseBodyFragment`: attributes on `<html>`/`<body>` inside the fragment
    /// land on jsoup's discarded context root, not the document's elements.
    drop_merged_attrs: bool,
    /// The document's `<html>` and `<body>`, once created, and every comment
    /// node — for jsoup's placement of comments after `</body>`.
    html: Cell<Option<NodeId>>,
    body: Cell<Option<NodeId>>,
    comments: RefCell<HashSet<NodeId>>,
}

/// html5ever's tree sink for scraper, with jsoup's no-merge text insertion.
struct JsoupSink {
    inner: HtmlTreeSink,
    st: Rc<ParseState>,
}

impl TreeSink for JsoupSink {
    type Handle = NodeId;
    type Output = Html;
    type ElemName<'a>
        = <HtmlTreeSink as TreeSink>::ElemName<'a>
    where
        Self: 'a;

    fn finish(self) -> Html {
        self.inner.finish()
    }
    fn parse_error(&self, msg: std::borrow::Cow<'static, str>) {
        self.inner.parse_error(msg)
    }
    fn get_document(&self) -> NodeId {
        self.inner.get_document()
    }
    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Self::ElemName<'a> {
        self.inner.elem_name(target)
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let pending = self.st.pending.borrow();
        let matched = pending.as_ref().filter(|(p, _, _)| *p == name.local);
        let html_ns = name.ns == ns!(html);
        let unknown = !is_known(&name.local.as_ref().to_ascii_lowercase());
        let local = name.local.clone();
        let id = self.inner.create_element(name, attrs, flags);
        if html_ns && &*local == "html" && self.st.html.get().is_none() {
            self.st.html.set(Some(id));
        }
        if html_ns && &*local == "body" && self.st.body.get().is_none() {
            self.st.body.set(Some(id));
        }
        if let Some((_, self_closing, valueless)) = matched {
            self.st.created.set(Some((id, html_ns)));
            let mut meta = self.st.meta.borrow_mut();
            if *self_closing && unknown {
                meta.self_closing.insert(id);
            }
            if !valueless.is_empty() {
                meta.valueless.insert(id, valueless.clone());
            }
        }
        id
    }
    fn create_comment(&self, text: StrTendril) -> NodeId {
        let id = self.inner.create_comment(text);
        self.st.comments.borrow_mut().insert(id);
        id
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.inner.create_pi(target, data)
    }
    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        // After </body> (and after </html>) the HTML spec appends a comment to
        // <html> or the Document; jsoup 1.10.1 never pops <body>, so its
        // comment goes into the body.
        if let (NodeOrText::AppendNode(id), Some(body)) = (&child, self.st.body.get()) {
            let after_body = Some(*parent) == self.st.html.get() || *parent == self.get_document();
            if after_body && self.st.html.get().is_some() && self.st.comments.borrow().contains(id) {
                return self.inner.append(&body, child);
            }
        }
        if let NodeOrText::AppendText(_) = &child {
            let epoch = self.st.epoch.get();
            let prev = self.st.last_text.borrow_mut().insert(*parent, epoch);
            if prev.is_some_and(|p| p != epoch) {
                // A token came between this text and the parent's previous
                // text: jsoup has a fresh TextNode here. html5ever's sink would
                // concatenate onto a trailing text sibling, so separate them
                // with a placeholder that is removed again at once.
                let marker = self.inner.create_comment(StrTendril::new());
                self.inner.append(parent, NodeOrText::AppendNode(marker));
                self.inner.append(parent, child);
                self.inner.remove_from_parent(&marker);
                return;
            }
        }
        self.inner.append(parent, child)
    }
    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        self.inner.append_based_on_parent_node(element, prev_element, child)
    }
    fn append_doctype_to_document(&self, name: StrTendril, public_id: StrTendril, system_id: StrTendril) {
        self.inner.append_doctype_to_document(name, public_id, system_id)
    }
    fn mark_script_already_started(&self, node: &NodeId) {
        self.inner.mark_script_already_started(node)
    }
    fn pop(&self, node: &NodeId) {
        self.inner.pop(node)
    }
    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.inner.get_template_contents(target)
    }
    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        self.inner.same_node(x, y)
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.inner.set_quirks_mode(mode)
    }
    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        self.inner.append_before_sibling(sibling, new_node)
    }
    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        if self.st.drop_merged_attrs {
            return;
        }
        self.inner.add_attrs_if_missing(target, attrs)
    }
    fn associate_with_form(&self, target: &NodeId, form: &NodeId, nodes: (&NodeId, Option<&NodeId>)) {
        self.inner.associate_with_form(target, form, nodes)
    }
    fn remove_from_parent(&self, target: &NodeId) {
        self.inner.remove_from_parent(target)
    }
    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        self.inner.reparent_children(node, new_parent)
    }
    fn is_mathml_annotation_xml_integration_point(&self, handle: &NodeId) -> bool {
        self.inner.is_mathml_annotation_xml_integration_point(handle)
    }
    fn set_current_line(&self, line_number: u64) {
        self.inner.set_current_line(line_number)
    }
    fn complete_script(&self, node: &NodeId) -> NextParserState {
        self.inner.complete_script(node)
    }
}

/// The token stream between html5ever's tokenizer and tree builder: marks
/// token boundaries for [`JsoupSink`] and closes self-closing start tags the
/// way jsoup does.
struct JsoupTokens {
    tb: TreeBuilder<NodeId, JsoupSink>,
    st: Rc<ParseState>,
}

impl TokenSink for JsoupTokens {
    type Handle = NodeId;

    fn process_token(&self, token: Token, line_number: u64) -> TokenSinkResult<NodeId> {
        if matches!(token, CharacterTokens(_) | NullCharacterToken | ParseError(_)) {
            return self.tb.process_token(token, line_number);
        }
        self.st.epoch.set(self.st.epoch.get() + 1);
        let (name, self_closing) = match &token {
            TagToken(t) if t.kind == EndTag => {
                // Keep the source cursor in step, so a later start tag is not
                // matched against text inside a <script> or <style>.
                self.st.find_tag(&t.name, true);
                return self.tb.process_token(token, line_number);
            }
            TagToken(t) => (t.name.clone(), t.self_closing),
            _ => return self.tb.process_token(token, line_number),
        };
        let valueless = self.st.find_tag(&name, false).unwrap_or_default();
        *self.st.pending.borrow_mut() = Some((name.clone(), self_closing, valueless));
        self.st.created.set(None);
        let result = self.tb.process_token(token, line_number);
        self.st.pending.borrow_mut().take();
        if !self_closing {
            return result;
        }
        let lower = AsRef::<str>::as_ref(&name).to_ascii_lowercase();
        // jsoup's HtmlTreeBuilder.insert(StartTag): a self-closing tag is
        // inserted empty and its end tag emitted at once, returning the
        // tokenizer to the data state. The document scaffolding is left to the
        // tree builder, which merges or ignores those tags.
        let closable = !tag(&lower).empty
            && !matches!(lower.as_str(), "html" | "head" | "body" | "frameset");
        match self.st.created.take() {
            Some((_, true)) if closable => {
                self.st.epoch.set(self.st.epoch.get() + 1);
                let end = Tag { kind: EndTag, name, self_closing: false, attrs: Vec::new() };
                self.tb.process_token(TagToken(end), line_number)
            }
            _ => result,
        }
    }

    fn end(&self) {
        self.tb.end()
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.tb.adjusted_current_node_present_but_not_in_html_namespace()
    }
}

/// `Jsoup.parse(html)`: a full document in jsoup's tree shape, plus the set of
/// elements jsoup flags self-closing.
pub fn parse(html: &str, body_fragment: bool) -> (Html, JsoupMeta) {
    let st = Rc::new(ParseState {
        source: html.to_string(),
        drop_merged_attrs: body_fragment,
        ..Default::default()
    });
    let sink = JsoupSink { inner: HtmlTreeSink::new(Html::new_document()), st: st.clone() };
    let opts = TreeBuilderOpts { scripting_enabled: false, ..Default::default() };
    let tb = TreeBuilder::new(sink, opts);
    let tok = Tokenizer::new(JsoupTokens { tb, st: st.clone() }, TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    while let TokenizerResult::Script(_) = tok.feed(&input) {}
    tok.end();
    let dom = tok.sink.tb.sink.finish();
    let meta = st.meta.take();
    (dom, meta)
}

impl ParseState {
    /// Find the next `<name` (or `</name`) in the source from the cursor and
    /// move past it; for a start tag, answer the attributes it writes with no
    /// value. `None` when the tag is not in the source (one the tree builder
    /// implied, or text html5ever reached differently) — the cursor stays put.
    fn find_tag(&self, name: &LocalName, end: bool) -> Option<HashSet<String>> {
        let src = self.source.as_bytes();
        let name = name.as_ref().as_bytes();
        let open: &[u8] = if end { b"</" } else { b"<" };
        let mut i = self.cursor.get();
        while i + open.len() + name.len() <= src.len() {
            let at = i + open.len();
            if src[i..].starts_with(open)
                && src[at..at + name.len()].eq_ignore_ascii_case(name)
                && src
                    .get(at + name.len())
                    .is_none_or(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>')
            {
                let (valueless, next) = valueless_attrs(src, at + name.len());
                self.cursor.set(next);
                return Some(valueless);
            }
            i += 1;
        }
        None
    }
}

/// Scan a start tag's attributes from just after its name, as the HTML
/// tokenizer does; answer the names written without `=`, and where the tag ends.
/// A repeated attribute is dropped by the parser, so only the first counts.
fn valueless_attrs(src: &[u8], mut i: usize) -> (HashSet<String>, usize) {
    let mut seen = HashSet::new();
    let mut valueless = HashSet::new();
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
    loop {
        while i < src.len() && (ws(src[i]) || src[i] == b'/') {
            i += 1;
        }
        if i >= src.len() {
            return (valueless, i);
        }
        if src[i] == b'>' {
            return (valueless, i + 1);
        }
        let start = i;
        i += 1; // the first character may be '='
        while i < src.len() && !ws(src[i]) && !matches!(src[i], b'/' | b'>' | b'=') {
            i += 1;
        }
        let key = String::from_utf8_lossy(&src[start..i]).to_ascii_lowercase();
        while i < src.len() && ws(src[i]) {
            i += 1;
        }
        let has_value = i < src.len() && src[i] == b'=';
        if has_value {
            i += 1;
            while i < src.len() && ws(src[i]) {
                i += 1;
            }
            match src.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    i += 1;
                    while i < src.len() && src[i] != q {
                        i += 1;
                    }
                    i += 1;
                }
                _ => {
                    while i < src.len() && !ws(src[i]) && src[i] != b'>' {
                        i += 1;
                    }
                }
            }
        }
        if seen.insert(key.clone()) && !has_value {
            valueless.insert(key);
        }
    }
}
