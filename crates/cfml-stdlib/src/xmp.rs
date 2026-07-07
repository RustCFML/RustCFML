//! `XmpParse(xmpXml)` — parse an XMP (RDF/XML) metadata packet and flatten it
//! to a `{ path: value }` struct, mirroring what Adobe XMPCore's
//! `XMPMetaFactory.parseFromString(...).iterator()` yields (each leaf property's
//! `getPath()` → `getValue()`).
//!
//! This replaces Preside's `xmpcore.jar` dependency (`XmpMetaReader.cfc`), which
//! only ever used the jar to parse-and-enumerate. It is pure Rust (built on the
//! `quick-xml` reader we already ship), so it also works on the wasm targets.
//!
//! Paths use Adobe's canonical shape for the common cases:
//! * simple property → `prefix:localName`
//! * array item (`rdf:Bag`/`Seq`/`Alt`) → `prefix:localName[N]` (1-based)
//! * struct field (`rdf:parseType="Resource"` or nested elements) →
//!   `prefix:localName/childPrefix:childName`
//!
//! Language-alternative qualifiers (`xml:lang`) are not emitted as separate
//! pseudo-properties; the `rdf:Alt` item value is emitted directly. This is a
//! deliberate, documented simplification (Preside strips the schema prefix and
//! array indices from every path anyway, so it never consumes qualifier nodes).

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use quick_xml::events::Event;
use quick_xml::Reader;

/// A minimal parsed-XML node (qualified names retained, e.g. `dc:creator`).
#[derive(Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

/// `XmpParse(xmpString)` builtin entry point.
pub fn fn_xmp_parse(args: Vec<CfmlValue>) -> CfmlResult {
    let xml = args.first().map(|v| v.as_string()).unwrap_or_default();
    let root = match parse_tree(&xml) {
        Ok(r) => r,
        Err(e) => return Err(CfmlError::runtime(format!("XmpParse: {}", e))),
    };

    let mut out = ValueMap::default();
    // XMP allows several rdf:Description blocks (one per schema). Flatten each.
    let mut descriptions = Vec::new();
    collect_descriptions(&root, &mut descriptions);
    for desc in descriptions {
        flatten_description(desc, &mut out);
    }
    Ok(CfmlValue::strukt(out))
}

// ---------------------------------------------------------------------------
// Parse into a tiny tree
// ---------------------------------------------------------------------------

fn parse_tree(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Node> = vec![Node::default()]; // synthetic root
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => stack.push(element_node(e)),
            Ok(Event::Empty(ref e)) => {
                let node = element_node(e);
                stack.last_mut().expect("root always present").children.push(node);
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let node = stack.pop().expect("checked len > 1");
                    stack.last_mut().expect("root always present").children.push(node);
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or(std::borrow::Cow::Borrowed("")).to_string();
                if !text.trim().is_empty() {
                    if let Some(cur) = stack.last_mut() {
                        cur.text.push_str(&text);
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                if let Ok(bytes) = std::str::from_utf8(e.as_ref()) {
                    if let Some(cur) = stack.last_mut() {
                        cur.text.push_str(bytes);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
    }
    Ok(stack.pop().unwrap_or_default())
}

fn element_node(e: &quick_xml::events::BytesStart) -> Node {
    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
    let mut attrs = Vec::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let val = attr
            .unescape_value()
            .map(|c| c.to_string())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
        attrs.push((key, val));
    }
    Node { name, attrs, children: Vec::new(), text: String::new() }
}

fn collect_descriptions<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    if node.name == "rdf:Description" {
        out.push(node);
    }
    for c in &node.children {
        collect_descriptions(c, out);
    }
}

// ---------------------------------------------------------------------------
// Flatten to Adobe-style path → value
// ---------------------------------------------------------------------------

fn flatten_description(desc: &Node, out: &mut ValueMap) {
    // Attribute-form simple properties (e.g. tiff:Make="Canon").
    for (k, v) in &desc.attrs {
        if is_property_name(k) {
            out.insert(k.clone(), CfmlValue::string(v.clone()));
        }
    }
    // Element-form properties.
    for child in &desc.children {
        walk_property(child, &child.name, out);
    }
}

fn walk_property(node: &Node, path: &str, out: &mut ValueMap) {
    // Array? (rdf:Bag / rdf:Seq / rdf:Alt containing rdf:li items)
    if let Some(container) = node.children.iter().find(|c| is_rdf_container(&c.name)) {
        let mut idx = 0usize;
        for li in container.children.iter().filter(|c| c.name == "rdf:li") {
            idx += 1;
            let item_path = format!("{}[{}]", path, idx);
            walk_container_item(li, &item_path, out);
        }
        return;
    }

    // Struct? (rdf:parseType="Resource", or nested property elements, or
    // property-bearing attributes)
    let struct_attrs: Vec<&(String, String)> =
        node.attrs.iter().filter(|(k, _)| is_property_name(k)).collect();
    let child_props: Vec<&Node> =
        node.children.iter().filter(|c| is_property_element(&c.name)).collect();

    if !struct_attrs.is_empty() || !child_props.is_empty() {
        for (k, v) in struct_attrs {
            out.insert(format!("{}/{}", path, k), CfmlValue::string(v.clone()));
        }
        for c in child_props {
            walk_property(c, &format!("{}/{}", path, c.name), out);
        }
        return;
    }

    // Simple leaf property.
    out.insert(path.to_string(), CfmlValue::string(node.text.trim().to_string()));
}

/// An `rdf:li` item: either a simple text value, or a nested struct.
fn walk_container_item(li: &Node, item_path: &str, out: &mut ValueMap) {
    let struct_attrs: Vec<&(String, String)> =
        li.attrs.iter().filter(|(k, _)| is_property_name(k)).collect();
    let child_props: Vec<&Node> =
        li.children.iter().filter(|c| is_property_element(&c.name)).collect();

    if struct_attrs.is_empty() && child_props.is_empty() {
        out.insert(item_path.to_string(), CfmlValue::string(li.text.trim().to_string()));
        return;
    }
    for (k, v) in struct_attrs {
        out.insert(format!("{}/{}", item_path, k), CfmlValue::string(v.clone()));
    }
    for c in child_props {
        walk_property(c, &format!("{}/{}", item_path, c.name), out);
    }
}

// ---------------------------------------------------------------------------
// Name classification
// ---------------------------------------------------------------------------

fn is_rdf_container(name: &str) -> bool {
    matches!(name, "rdf:Bag" | "rdf:Seq" | "rdf:Alt")
}

/// A namespaced attribute or element that represents an actual XMP property —
/// i.e. not RDF plumbing, an xmlns declaration, or the xml:lang qualifier.
fn is_property_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "xmlns" || lower.starts_with("xmlns:") {
        return false;
    }
    if lower.starts_with("rdf:") {
        return false; // rdf:about, rdf:parseType, rdf:resource, rdf:nodeID, …
    }
    if lower.starts_with("xml:") {
        return false; // xml:lang qualifier
    }
    true
}

fn is_property_element(name: &str) -> bool {
    // A property element carries a namespace prefix; RDF containers/li and the
    // rdf:* plumbing are excluded.
    name.contains(':') && !name.starts_with("rdf:")
}
