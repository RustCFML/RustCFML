//! A **mutable** HTML document — parse, CSS-select, read and change attributes,
//! serialise back out.
//!
//! CFML can already *read* HTML (`htmlParse()` hands back an immutable
//! XML-shaped struct) but has never been able to change one and write it back.
//! That is what every "rewrite the links in this email" or "inline the CSS
//! before sending" job needs, and it is why CFML applications reach for jsoup
//! through `createObject( "java", "org.jsoup.Jsoup" )`. This is that capability
//! under a CFML name; the jsoup shim is a thin adapter over it.
//!
//! `HtmlDocument( html )` returns a native object:
//!
//! ```cfml
//! doc   = HtmlDocument( emailHtml );
//! links = doc.select( "a" );                       // node handles
//! for( link in links ) {
//!     href = doc.attr( link, "href" );
//!     doc.setAttr( link, "href", track( href ) );  // mutates the document
//! }
//! rewritten = doc.toString();
//! ```
//!
//! # Node handles
//!
//! `select()` returns **integer handles**, not sub-objects. A handle is an
//! `ego_tree` node id, so it stays valid across mutations and costs nothing to
//! pass around, and — importantly — every operation goes back through the one
//! document that owns the tree. Sub-objects would each need their own borrow of
//! a tree that is being mutated underneath them.
//!
//! # Parsing
//!
//! Full documents go through html5ever's document parser, which supplies the
//! `html`/`head`/`body` scaffolding a browser would. A *fragment* (the common
//! case for an email partial or a widget) would otherwise be silently wrapped in
//! that scaffolding and come back out bigger than it went in, so
//! `HtmlDocument( html, "fragment" )` parses in fragment mode instead. The
//! default sniffs: input containing `<html` or `<body` is a document, anything
//! else is a fragment.

use std::sync::{Arc, Mutex, RwLock, Weak};

use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};

use ego_tree::NodeId;
use html5ever::{namespace_url, ns, LocalName, QualName};
use scraper::{Html, Node, Selector};

pub struct CfmlHtmlDocument {
    /// `Mutex`, not a bare field: `scraper::Element` carries `OnceCell`s for its
    /// lazily-derived id/class lists, and `OnceCell` is `Send` but never `Sync`.
    /// A `CfmlNative` must be both. `Mutex<T>` is `Sync` whenever `T: Send`, so
    /// this is what makes a parsed document holdable across calls at all.
    /// (`scraper`'s `atomic` feature supplies the `Send` half, by swapping
    /// html5ever's `Cell`-refcounted tendrils for atomic ones.)
    dom: Mutex<Html>,
    /// Whether this was parsed as a fragment — `toString()` then emits the
    /// fragment back, without the scaffolding the parser added.
    fragment: bool,
    self_ref: Option<Weak<RwLock<CfmlHtmlDocument>>>,
}

impl std::fmt::Debug for CfmlHtmlDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlDocument")
            .field("fragment", &self.fragment)
            .field("nodes", &self.with_dom(|d| d.tree.values().count()))
            .finish()
    }
}

impl CfmlHtmlDocument {
    pub fn parse(html: &str, fragment: bool) -> CfmlHtmlDocument {
        let dom = if fragment {
            Html::parse_fragment(html)
        } else {
            Html::parse_document(html)
        };
        CfmlHtmlDocument { dom: Mutex::new(dom), fragment, self_ref: None }
    }

    /// `<html`/`<body` means the caller handed us a whole document; anything
    /// else is a fragment and must not come back wrapped in scaffolding.
    pub fn looks_like_document(html: &str) -> bool {
        let lower = html.to_ascii_lowercase();
        lower.contains("<html") || lower.contains("<body") || lower.contains("<!doctype")
    }

    pub fn into_value(mut self) -> CfmlValue {
        let arc: Arc<RwLock<CfmlHtmlDocument>> = Arc::new_cyclic(|weak| {
            self.self_ref = Some(weak.clone());
            RwLock::new(self)
        });
        CfmlValue::NativeObject(arc)
    }

    /// Read the parsed document under the lock. Poisoning is recovered from
    /// rather than propagated: a panic in another thread must not make this
    /// document permanently unusable.
    fn with_dom<R>(&self, f: impl FnOnce(&Html) -> R) -> R {
        f(&self.dom.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn with_dom_mut<R>(&self, f: impl FnOnce(&mut Html) -> R) -> R {
        f(&mut self.dom.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn this(&self) -> CfmlValue {
        match self.self_ref.as_ref().and_then(|w| w.upgrade()) {
            Some(arc) => CfmlValue::NativeObject(arc),
            None => CfmlValue::Null,
        }
    }

    /// Turn a caller's integer handle back into a live node id.
    ///
    /// `ego_tree::NodeId` cannot be constructed from an integer, so handles are
    /// **positions in the tree's node order**, which is stable: `ego_tree` never
    /// removes a node's slot, so an id keeps its position for the life of the
    /// tree even as values are mutated.
    fn resolve_in(dom: &Html, handle: i64) -> Result<NodeId, CfmlError> {
        if handle < 0 {
            return Err(CfmlError::runtime(format!(
                "HtmlDocument: '{}' is not a node handle (handles come from select())",
                handle
            )));
        }
        dom.tree
            .nodes()
            .nth(handle as usize)
            .map(|n| n.id())
            .ok_or_else(|| {
                CfmlError::runtime(format!(
                    "HtmlDocument: node handle {} is not in this document",
                    handle
                ))
            })
    }

    fn handle_of(dom: &Html, id: NodeId) -> i64 {
        dom.tree
            .nodes()
            .position(|n| n.id() == id)
            .map(|p| p as i64)
            .unwrap_or(-1)
    }

    fn selector(css: &str) -> Result<Selector, CfmlError> {
        Selector::parse(css).map_err(|e| {
            CfmlError::runtime(format!("HtmlDocument: invalid CSS selector '{}': {}", css, e))
        })
    }

    fn select(&self, css: &str) -> Result<Vec<CfmlValue>, CfmlError> {
        let sel = Self::selector(css)?;
        self.with_dom(|dom| {
            Ok(dom
                .select(&sel)
                .map(|el| CfmlValue::Int(Self::handle_of(dom, el.id())))
                .collect())
        })
    }

    /// `select`, restricted to descendants of `root` — jsoup's
    /// `Element.select()`, as distinct from `Document.select()`.
    fn select_within(&self, root: i64, css: &str) -> Result<Vec<CfmlValue>, CfmlError> {
        let sel = Self::selector(css)?;
        self.with_dom(|dom| {
            let root_id = Self::resolve_in(dom, root)?;
            let inside: std::collections::HashSet<NodeId> = match dom.tree.get(root_id) {
                Some(n) => n.descendants().map(|d| d.id()).collect(),
                None => Default::default(),
            };
            Ok(dom
                .select(&sel)
                .filter(|el| inside.contains(&el.id()))
                .map(|el| CfmlValue::Int(Self::handle_of(dom, el.id())))
                .collect())
        })
    }

    /// `[ self, …descendants ]` as element handles — jsoup's
    /// `Element.getAllElements()`, whose first entry is the element itself.
    fn all_elements(&self, root: i64) -> Result<Vec<CfmlValue>, CfmlError> {
        self.with_dom(|dom| {
            let root_id = Self::resolve_in(dom, root)?;
            let Some(node) = dom.tree.get(root_id) else {
                return Ok(Vec::new());
            };
            Ok(node
                .descendants()
                .filter(|d| d.value().is_element())
                .map(|d| CfmlValue::Int(Self::handle_of(dom, d.id())))
                .collect())
        })
    }

    fn attr(&self, handle: i64, name: &str) -> Result<CfmlValue, CfmlError> {
        self.with_dom(|dom| {
        let id = Self::resolve_in(dom, handle)?;
        Ok(match dom.tree.get(id).and_then(|n| n.value().as_element()) {
            // jsoup's attr() answers "" for a missing attribute, not null, and
            // callers Trim() the result — so an absent attribute must not be a
            // null that blows up the next string operation.
            Some(el) => CfmlValue::string(el.attr(name).unwrap_or("").to_string()),
            None => CfmlValue::string(String::new()),
        })
        })
    }

    fn set_attr(&self, handle: i64, name: &str, value: &str) -> Result<(), CfmlError> {
        self.with_dom_mut(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            let mut node = dom.tree.get_mut(id).ok_or_else(|| {
                CfmlError::runtime(format!("HtmlDocument: node handle {} is gone", handle))
            })?;
            match node.value() {
                Node::Element(el) => {
                    // Attribute names are lowercased: HTML attributes are
                    // case-insensitive, and the parser has already lowercased the
                    // ones it read, so a caller writing "HREF" must land on the
                    // same key rather than adding a second one.
                    let qname =
                        QualName::new(None, ns!(), LocalName::from(name.to_ascii_lowercase()));
                    el.attrs.insert(qname, value.into());
                    Ok(())
                }
                _ => Err(CfmlError::runtime(format!(
                    "HtmlDocument: node handle {} is not an element, so it has no attributes",
                    handle
                ))),
            }
        })
    }

    fn remove_attr(&self, handle: i64, name: &str) -> Result<(), CfmlError> {
        let lower = name.to_ascii_lowercase();
        self.with_dom_mut(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            if let Some(mut node) = dom.tree.get_mut(id) {
                if let Node::Element(el) = node.value() {
                    el.attrs.retain(|k, _| k.local.as_ref() != lower);
                }
            }
            Ok(())
        })
    }

    fn attributes(&self, handle: i64) -> Result<CfmlValue, CfmlError> {
        self.with_dom(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            let mut out = ValueMap::default();
            if let Some(el) = dom.tree.get(id).and_then(|n| n.value().as_element()) {
                for (k, v) in el.attrs() {
                    out.insert(k.to_string(), CfmlValue::string(v.to_string()));
                }
            }
            Ok(CfmlValue::strukt(out))
        })
    }

    /// All descendant text, whitespace-collapsed — jsoup's `Element.text()`.
    fn text(&self, handle: i64) -> Result<String, CfmlError> {
        let raw = self.data(handle)?;
        Ok(raw.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// Raw text content, NOT collapsed and NOT escaped — jsoup's
    /// `DataNode.getWholeData()`, i.e. what is inside a `<style>` or `<script>`.
    /// CSS must come back byte-for-byte or the rules change meaning.
    fn data(&self, handle: i64) -> Result<String, CfmlError> {
        self.with_dom(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            let Some(node) = dom.tree.get(id) else {
                return Ok(String::new());
            };
            let mut out = String::new();
            for d in node.descendants() {
                if let Node::Text(t) = d.value() {
                    out.push_str(t);
                }
            }
            Ok(out)
        })
    }

    fn serialise_node(&self, handle: i64, outer: bool) -> Result<String, CfmlError> {
        self.with_dom(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            let node = dom.tree.get(id).ok_or_else(|| {
                CfmlError::runtime(format!("HtmlDocument: node handle {} is gone", handle))
            })?;
            Ok(match scraper::ElementRef::wrap(node) {
                Some(el) => {
                    if outer {
                        el.html()
                    } else {
                        el.inner_html()
                    }
                }
                None => String::new(),
            })
        })
    }

    fn tag_name(&self, handle: i64) -> Result<String, CfmlError> {
        self.with_dom(|dom| {
            let id = Self::resolve_in(dom, handle)?;
            Ok(dom
                .tree
                .get(id)
                .and_then(|n| n.value().as_element())
                .map(|el| el.name().to_string())
                .unwrap_or_default())
        })
    }

    /// The whole document. A fragment comes back WITHOUT the html/head/body the
    /// parser added, so a round trip through this object does not grow the input.
    fn to_html(&self) -> String {
        self.with_dom(|dom| {
            if !self.fragment {
                return dom.html();
            }
            // A fragment parse puts the content under a synthetic <html> root.
            match dom.tree.root().first_child().and_then(scraper::ElementRef::wrap) {
                Some(root) => root.inner_html(),
                None => dom.html(),
            }
        })
    }
}

impl CfmlNative for CfmlHtmlDocument {
    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_ascii_lowercase().as_str() {
            "select" => &["selector"][..],
            "selectwithin" => &["element", "selector"][..],
            "attr" | "removeattr" | "hasattr" => &["element", "name"][..],
            "setattr" => &["element", "name", "value"][..],
            "allelements" | "attributes" | "text" | "data" | "html" | "innerhtml"
            | "outerhtml" | "tagname" => &["element"][..],
            "tostring" | "documenthtml" => &[][..],
            _ => return None,
        })
    }

    fn class_name(&self) -> &str {
        "HtmlDocument"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let a = args.as_slice();
        let str_arg = |i: usize| a.get(i).map(|v| v.as_string()).unwrap_or_default();
        let handle = |i: usize| -> i64 {
            match a.get(i) {
                Some(CfmlValue::Int(n)) => *n,
                Some(CfmlValue::Double(d)) => *d as i64,
                Some(other) => other.as_string().trim().parse().unwrap_or(-1),
                None => -1,
            }
        };

        match name.to_ascii_lowercase().as_str() {
            "select" => Ok(CfmlValue::array(self.select(&str_arg(0))?)),
            "selectwithin" => Ok(CfmlValue::array(self.select_within(handle(0), &str_arg(1))?)),
            "allelements" => Ok(CfmlValue::array(self.all_elements(handle(0))?)),
            "attr" => self.attr(handle(0), &str_arg(1)),
            "setattr" => {
                self.set_attr(handle(0), &str_arg(1), &str_arg(2))?;
                Ok(self.this())
            }
            "removeattr" => {
                self.remove_attr(handle(0), &str_arg(1))?;
                Ok(self.this())
            }
            "attributes" => self.attributes(handle(0)),
            "hasattr" => {
                let v = self.attr(handle(0), &str_arg(1))?;
                Ok(CfmlValue::Bool(!v.as_string().is_empty()))
            }
            "text" => Ok(CfmlValue::string(self.text(handle(0))?)),
            "data" => Ok(CfmlValue::string(self.data(handle(0))?)),
            "html" | "innerhtml" => Ok(CfmlValue::string(self.serialise_node(handle(0), false)?)),
            "outerhtml" => Ok(CfmlValue::string(self.serialise_node(handle(0), true)?)),
            "tagname" => Ok(CfmlValue::string(self.tag_name(handle(0))?)),
            "tostring" | "documenthtml" => Ok(CfmlValue::string(self.to_html())),
            other => Err(CfmlError::runtime(format!(
                "HtmlDocument has no method [{}]. Available: select, attr, setAttr, \
                 removeAttr, hasAttr, attributes, text, data, html, outerHtml, tagName, \
                 selectWithin, allElements, toString.",
                other
            ))),
        }
    }
}

/// `HtmlDocument( html [, mode ] )` — parse HTML into a mutable document.
///
/// `mode` is `"document"` or `"fragment"`; omitted, it is sniffed from the input
/// (see [`CfmlHtmlDocument::looks_like_document`]). The distinction matters
/// because a document parse adds `html`/`head`/`body` scaffolding, which would
/// otherwise appear in the output of a round trip over an HTML *snippet*.
pub fn fn_html_document(args: Vec<CfmlValue>) -> CfmlResult {
    let html = args.first().map(|v| v.as_string()).unwrap_or_default();
    let fragment = match args.get(1).map(|v| v.as_string()).unwrap_or_default() {
        m if m.eq_ignore_ascii_case("fragment") => true,
        m if m.eq_ignore_ascii_case("document") => false,
        _ => !CfmlHtmlDocument::looks_like_document(&html),
    };
    Ok(CfmlHtmlDocument::parse(&html, fragment).into_value())
}
