//! Apply a [`Policy`] to an HTML fragment.
//!
//! Parsing and serialisation are html5ever's (via `scraper`) — the same
//! battle-tested core the mainstream Rust sanitisers use, and the part that has
//! to be right for mutation-XSS resistance. What this module adds is the policy
//! layer: the four tag actions, per-attribute validation with `onInvalid`
//! semantics, and CSS filtering of `style`.
//!
//! ## Why a tree walk rather than an allowlist builder
//!
//! An allowlist-style sanitiser can drop a disallowed attribute, but it cannot
//! drop the *element* because one of its attributes failed a regexp — and
//! `onInvalid="removeTag"` (which the Preside policy puts on `img@src` and
//! `link@type`) requires exactly that. `filterTag` and `truncate` are likewise
//! inexpressible. Walking the tree ourselves is the only way all four actions
//! mean what the policy says they mean.

use crate::css;
use crate::policy::{OnInvalid, Policy, TagAction};
use ego_tree::NodeId;
use scraper::node::Node;
use scraper::Html;

/// Elements dropped WITH their contents when the policy does not name them at
/// all. An unnamed tag is otherwise unwrapped (children kept), which for
/// `<script>` would turn code into text in a context an attacker may control.
///
/// This is a fallback, not an override: a policy that explicitly names one of
/// these wins. The shipped policies allow `<form>`, `<input>` and `<base>` with
/// validated attributes, and overriding them here silently deleted legitimate
/// content — caught by diffing against the real AntiSamy.
const REMOVE_IF_UNNAMED: &[&str] = &[
    "script", "iframe", "frame", "frameset", "object", "embed", "applet", "noscript", "noembed",
    "template", "base", "form",
];

/// Attributes that are never allowed regardless of policy: every `on*` event
/// handler. A policy that allow-lists an attribute name generously (or a
/// `<global-tag-attributes>` entry) must not be able to reintroduce script
/// execution.
fn is_event_handler(name: &str) -> bool {
    name.len() > 2 && name.as_bytes()[0].eq_ignore_ascii_case(&b'o')
        && name.as_bytes()[1].eq_ignore_ascii_case(&b'n')
}

#[derive(Debug)]
pub struct SanitizeError(pub String);

impl std::fmt::Display for SanitizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SanitizeError {}

/// What the walk decided to do with one element.
enum Disposition {
    /// Keep the element; its attributes have already been rewritten.
    Keep,
    /// Drop the element and everything under it.
    Drop,
    /// Drop the element, keep its children in its place.
    Unwrap,
}

/// Sanitise `html` under `policy`.
///
/// The fast path matters more than anything else here: this runs on every
/// request parameter of every request in the application that motivated it, and
/// the overwhelming majority of those values contain no markup at all. Input
/// with none of `<`, `&` or `"` cannot change under sanitisation, so it is
/// returned untouched without ever building a parse tree. (`"` belongs in that
/// set because text output escapes it — leaving it out made a bare quote come
/// back unescaped while the slow path escaped it, a divergence the AntiSamy
/// corpus diff caught.)
pub fn sanitize(html: &str, policy: &Policy) -> Result<String, SanitizeError> {
    if !html
        .as_bytes()
        .iter()
        .any(|b| *b == b'<' || *b == b'&' || *b == b'"')
    {
        return Ok(html.to_string());
    }
    if let Some(max) = policy.max_input_size() {
        if html.len() > max {
            return Err(SanitizeError(format!(
                "input is {} bytes, which exceeds the policy's maxInputSize of {}",
                html.len(),
                max
            )));
        }
    }

    let mut document = Html::parse_fragment(html);

    // Collect decisions first, then apply them: mutating the tree while
    // iterating it would invalidate the traversal.
    let root = document.tree.root().id();
    let mut to_drop: Vec<NodeId> = Vec::new();
    // Child ids are collected here, during the read pass: `NodeMut` cannot
    // enumerate children, and by the time the mutation pass runs the tree is
    // already changing under it.
    let mut to_unwrap: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
    let mut attribute_edits: Vec<(NodeId, Vec<(String, Option<String>)>)> = Vec::new();
    // Text children of a surviving `<style>`, rewritten through the stylesheet
    // filter. Without this the element's content reaches the browser verbatim.
    let mut style_bodies: Vec<(NodeId, String)> = Vec::new();

    for node in document.tree.get(root).unwrap().descendants() {
        match node.value() {
            Node::Element(_) => {}
            // Comments can carry conditional-comment script in old engines and
            // are never meaningful content — drop them wholesale, as AntiSamy
            // and every mainstream sanitiser do.
            Node::Comment(_) | Node::ProcessingInstruction(_) | Node::Doctype(_) => {
                to_drop.push(node.id());
                continue;
            }
            _ => continue,
        }
        let element = node.value().as_element().unwrap();
        let tag = element.name().to_ascii_lowercase();

        let (disposition, edits) = decide(&tag, element, policy);
        match disposition {
            Disposition::Keep => {
                if !edits.is_empty() {
                    attribute_edits.push((node.id(), edits));
                }
                if tag == "style" {
                    for child in node.children() {
                        if let Node::Text(text) = child.value() {
                            style_bodies
                                .push((child.id(), css::filter_stylesheet(&text.text, policy)));
                        }
                    }
                }
            }
            Disposition::Drop => to_drop.push(node.id()),
            Disposition::Unwrap => {
                to_unwrap.push((node.id(), node.children().map(|c| c.id()).collect()))
            }
        }
    }

    for (id, edits) in attribute_edits {
        let Some(mut node) = document.tree.get_mut(id) else { continue };
        if let Node::Element(el) = node.value() {
            for (name, replacement) in edits {
                let position = el
                    .attrs
                    .iter()
                    .position(|(k, _)| k.local.as_ref().eq_ignore_ascii_case(&name));
                let Some(position) = position else { continue };
                match replacement {
                    Some(value) => el.attrs[position].1 = value.into(),
                    // `remove`, not `swap_remove`: dropping one attribute must
                    // not reshuffle the rest.
                    None => {
                        el.attrs.remove(position);
                    }
                }
            }
        }
    }

    for (id, filtered) in style_bodies {
        if let Some(mut node) = document.tree.get_mut(id) {
            if let Node::Text(text) = node.value() {
                text.text = filtered.into();
            }
        }
    }

    for (id, children) in to_unwrap {
        let Some(mut node) = document.tree.get_mut(id) else { continue };
        // Reparenting the children before the element itself goes keeps their
        // order and their own subtrees intact.
        for child in children {
            node.insert_id_before(child);
        }
        node.detach();
    }

    for id in to_drop {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }

    Ok(serialize_children(&document))
}

/// Decide one element's fate and which of its attributes need rewriting.
fn decide(
    tag: &str,
    element: &scraper::node::Element,
    policy: &Policy,
) -> (Disposition, Vec<(String, Option<String>)>) {
    let action = match policy.tags.get(tag).map(|r| r.action) {
        Some(action) => action,
        // A tag the policy never names is dropped but its contents kept, which
        // is AntiSamy's "filter" default for unknown elements — unless it is
        // one whose contents must not survive either.
        None if REMOVE_IF_UNNAMED.contains(&tag) => TagAction::Remove,
        None => TagAction::Filter,
    };

    match action {
        TagAction::Remove => return (Disposition::Drop, Vec::new()),
        TagAction::Filter => return (Disposition::Unwrap, Vec::new()),
        // `<tags-to-encode>` reads as "render the tag as literal text", and
        // that is what AntiSamy 1.4.x did. The 1.5.3 jar Preside ships instead
        // UNWRAPS them — `<g>encoded</g>` comes back as `encoded`, measured
        // against the real library across `<g>a</g>b`, `x<g>y`, `<g/>` and a
        // nested case. Matching the jar, not the name.
        TagAction::Encode => return (Disposition::Unwrap, Vec::new()),
        TagAction::Truncate => {
            // Keep the element and its text; strip every attribute.
            let edits = element
                .attrs()
                .map(|(k, _)| (k.to_string(), None))
                .collect();
            return (Disposition::Keep, edits);
        }
        TagAction::Validate => {}
    }

    let mut edits: Vec<(String, Option<String>)> = Vec::new();
    for (raw_name, raw_value) in element.attrs() {
        let name = raw_name.to_ascii_lowercase();

        if is_event_handler(&name) {
            edits.push((raw_name.to_string(), None));
            continue;
        }

        // `style` is validated declaration-by-declaration by the CSS rules, not
        // by an attribute regexp — the policies deliberately declare it with no
        // rules of its own and delegate to <css-rules>. It still has to be an
        // attribute the policy ALLOWS here: the slashdot policy permits no
        // `style` at all, and filtering one it never allowed left an empty
        // `style=""` behind where AntiSamy removes the attribute outright.
        if name == "style" && policy.attribute_rule(tag, "style").is_some() {
            // An emptied `style` is left in place as `style=""` rather than
            // removed: that is what AntiSamy does, and the difference is
            // visible to anything diffing the two engines' output.
            edits.push((
                raw_name.to_string(),
                Some(css::filter_declarations(raw_value, policy)),
            ));
            continue;
        }

        let Some(rule) = policy.attribute_rule(tag, &name) else {
            edits.push((raw_name.to_string(), None));
            continue;
        };
        if rule.accepts(raw_value) {
            continue;
        }
        match rule.on_invalid {
            OnInvalid::RemoveTag => return (Disposition::Drop, Vec::new()),
            OnInvalid::FilterTag => return (Disposition::Unwrap, Vec::new()),
            OnInvalid::RemoveAttribute => edits.push((raw_name.to_string(), None)),
        }
    }
    (Disposition::Keep, edits)
}

/// Serialise the fragment's children — `parse_fragment` wraps everything in an
/// `<html>` element that must not appear in the output.
fn serialize_children(document: &Html) -> String {
    let root = document.tree.root();
    let mut out = String::new();
    // The fragment root's single child is the synthetic <html>; its children
    // are the caller's actual nodes.
    for child in root.children() {
        match child.value() {
            Node::Element(el) if el.name() == "html" => {
                for inner in child.children() {
                    out.push_str(&serialize_node(&inner));
                }
            }
            _ => out.push_str(&serialize_node(&child)),
        }
    }
    out
}

fn serialize_node(node: &ego_tree::NodeRef<'_, Node>) -> String {
    match node.value() {
        Node::Text(t) => escape_text(t),
        Node::Element(el) => {
            let mut out = String::new();
            out.push('<');
            out.push_str(el.name());
            for (k, v) in el.attrs() {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&escape_attribute(v));
                out.push('"');
            }
            if is_void(el.name()) {
                out.push_str(" />");
                return out;
            }
            out.push('>');
            // `<style>` is a raw-text element: escaping its content would
            // corrupt the CSS. Safe because the stylesheet filter allowlists
            // what it emits and refuses anything containing `<`.
            let raw_text = el.name() == "style";
            for child in node.children() {
                match child.value() {
                    Node::Text(t) if raw_text => out.push_str(&t.text),
                    _ => out.push_str(&serialize_node(&child)),
                }
            }
            out.push_str("</");
            out.push_str(el.name());
            out.push('>');
            out
        }
        _ => String::new(),
    }
}

/// Text-node escaping. `&` goes first — escaping it after `<` would double-
/// escape the entities the earlier passes just introduced.
///
/// `"` is escaped in text too, which is not strictly required there. It is what
/// makes `He said &quot;hi&quot;` come back out as it went in: the parser
/// decodes the entity to a bare quote, and without re-encoding it the value
/// would silently change shape. AntiSamy preserves the entity, and matching it
/// keeps a cross-engine corpus diff clean.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value)
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
