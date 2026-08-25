//! `org.jsoup.*` — the HTML parser CFML applications reach for when they need to
//! *change* a document rather than just read one.
//!
//! Preside uses it in two places, both on the email path:
//!
//! * `EmailLoggingService.insertClickTrackingLinks` — parse the message, select
//!   every `A`, rewrite each `href` to a tracking URL, serialise back;
//! * `EmailStyleInliner.inlineStyles` — parse, read the `<style>` blocks, match
//!   each CSS rule with `select()`, and write the resulting declarations onto
//!   each matched element's `style` attribute.
//!
//! Both are *mutate-then-serialise*, which is exactly what the `HtmlDocument()`
//! builtin does. This module is the jsoup-shaped adapter over it.
//!
//! # Handles, not sub-objects
//!
//! A jsoup `Element` here is a shim struct carrying the document (the native
//! `HtmlDocument` handle, which is shared, so every element addresses the same
//! mutable tree) plus an integer node handle. Nothing is copied, and a mutation
//! through one element is visible through every other and in the document's
//! `toString()`.
//!
//! `hashCode()` is the node handle. jsoup's is an identity hash, and callers —
//! `_getElementsWithStylesToApply` among them — use it as a struct key to group
//! per element, so it must be stable for one element and distinct between two.
//!
//! # `Elements` is a plain CFML array
//!
//! `select()` returns an array, because that is what makes `for( el in els )`,
//! `ArrayLen( els )` and `els[ 1 ]` all work the way the calling code expects of
//! a `java.util.List` on Lucee. The consequence is that `els.toString()` renders
//! the array's *elements*, so each element struct carries `__html` — its
//! outerHTML at selection time — to keep that content-bearing. `readStyles`
//! hashes exactly that (`Hash( styleElements.toString() )`) for its style cache
//! key, and a key that did not vary with the CSS would serve one email's styles
//! to another. The cost is one serialisation per selected element; the
//! correctness is not optional.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const JSOUP_CLASS: &str = "org.jsoup.jsoup";
pub const DOCUMENT_CLASS: &str = "org.jsoup.nodes.document";
pub const ELEMENT_CLASS: &str = "org.jsoup.nodes.element";
pub const ATTRIBUTES_CLASS: &str = "org.jsoup.nodes.attributes";
pub const OUTPUT_SETTINGS_CLASS: &str = "org.jsoup.nodes.document$outputsettings";

pub fn is_jsoup_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        JSOUP_CLASS | DOCUMENT_CLASS | ELEMENT_CLASS | ATTRIBUTES_CLASS | OUTPUT_SETTINGS_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn get(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn get_int(object: &CfmlValue, key: &str) -> i64 {
    match get(object, key) {
        Some(CfmlValue::Int(n)) => n,
        Some(CfmlValue::Double(d)) => d as i64,
        Some(other) => other.as_string().trim().parse().unwrap_or(-1),
        None => -1,
    }
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's jsoup adapter, which covers \
             parse / select / attributes / text / html / serialise. Anything beyond that is \
             refused rather than quietly returning an empty document.",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// Build an Element (or Document) handle onto `doc`.
///
/// An Element carries its outerHTML under `__value`, the engine's existing
/// convention for "what this Java object coerces to as a string" — and for a
/// jsoup Element that is exactly right, since `Element.toString()` IS its outer
/// HTML. It is what makes an ARRAY of elements stringify to something
/// content-bearing, which `readStyles` depends on
/// (`Hash( styleElements.toString() )` as a cache key).
///
/// It is a SNAPSHOT, taken at selection time. Every live read goes through a
/// method (`toString()`, `html()`, `attr()`), which re-reads the document, so
/// the snapshot is only ever seen by string coercion of the handle itself. A
/// Document deliberately gets no `__value`: it is the mutable thing, so a
/// snapshot of it would be the one most likely to go stale, and coercing a
/// Document to a string is not something callers do — they call `.html()`.
fn element(
    doc: &CfmlValue,
    node: i64,
    class: &str,
    call: &dyn Fn(&CfmlValue, &str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let mut m = shim(class);
    m.insert("__doc".to_string(), doc.clone());
    m.insert("__node".to_string(), CfmlValue::Int(node));
    if class == ELEMENT_CLASS {
        let html = call(doc, "outerHtml", vec![CfmlValue::Int(node)])?;
        m.insert("__value".to_string(), html);
    }
    Ok(CfmlValue::strukt(m))
}

/// The `HtmlDocument` native object behind a Document/Element handle.
fn doc_of(object: &CfmlValue) -> Result<CfmlValue, CfmlError> {
    get(object, "__doc").ok_or_else(|| {
        CfmlError::new(
            "org.jsoup: this object is not attached to a parsed document".to_string(),
            CfmlErrorType::Custom("java.lang.IllegalStateException".to_string()),
        )
    })
}

/// `parse` is the `HtmlDocument()` builtin; `call` invokes a method on the
/// native document object it returns.
pub fn dispatch(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    parse: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
    call: &dyn Fn(&CfmlValue, &str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match class_lower {
        // ---- org.jsoup.Jsoup (static entry points) --------------------------
        JSOUP_CLASS => match method {
            "init" => Ok(CfmlValue::strukt(shim(JSOUP_CLASS))),
            // parse( html ) / parse( html, baseUri ) — the base URI is only used
            // for resolving relative links, which this adapter does not do.
            "parse" | "parsebodyfragment" => {
                let html = args.first().map(|v| v.as_string()).unwrap_or_default();
                let mode = if method == "parsebodyfragment" { "fragment" } else { "" };
                let doc = parse(vec![
                    CfmlValue::string(html),
                    CfmlValue::string(mode.to_string()),
                ])?;
                let mut m = shim(DOCUMENT_CLASS);
                m.insert("__doc".to_string(), doc);
                m.insert("__node".to_string(), CfmlValue::Int(0));
                Ok(CfmlValue::strukt(m))
            }
            // Jsoup.clean( html, whitelist ) is a sanitiser, and CFML has a real
            // one — but its policy model is AntiSamy's, not jsoup's whitelist, so
            // silently substituting it would apply rules the caller did not ask
            // for. Point at it instead.
            "clean" => Err(CfmlError::new(
                "org.jsoup.Jsoup.clean() is not supported: its Whitelist policy model has no \
                 equivalent here. Use sanitizeHtml( html, policyPath ) or the \
                 org.owasp.validator.html.AntiSamy shim, both of which take an AntiSamy policy."
                    .to_string(),
                CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
            )),
            other => Err(unsupported("org.jsoup.Jsoup", other)),
        },

        // ---- Document and Element share almost everything -------------------
        DOCUMENT_CLASS | ELEMENT_CLASS => {
            let doc = doc_of(object)?;
            let node = get_int(object, "__node");
            let is_doc = class_lower == DOCUMENT_CLASS;

            match method {
                "select" => {
                    // Document.select searches everything; Element.select is
                    // scoped to descendants.
                    let css = args.first().map(|v| v.as_string()).unwrap_or_default();
                    let handles = if is_doc {
                        call(&doc, "select", vec![CfmlValue::string(css)])?
                    } else {
                        call(
                            &doc,
                            "selectWithin",
                            vec![CfmlValue::Int(node), CfmlValue::string(css)],
                        )?
                    };
                    to_elements(&doc, handles, call)
                }
                // [ self, …descendant elements ]; callers take .get(0) to mean
                // "this element".
                "getallelements" => {
                    let handles = call(&doc, "allElements", vec![CfmlValue::Int(node)])?;
                    to_elements(&doc, handles, call)
                }
                "getelementsbytag" => {
                    let tag = args.first().map(|v| v.as_string()).unwrap_or_default();
                    let handles = call(&doc, "select", vec![CfmlValue::string(tag)])?;
                    to_elements(&doc, handles, call)
                }
                "getelementbyid" => {
                    let id = args.first().map(|v| v.as_string()).unwrap_or_default();
                    let handles =
                        call(&doc, "select", vec![CfmlValue::string(format!("#{}", id))])?;
                    match handles {
                        CfmlValue::Array(a) if !a.is_empty() => {
                            let h = a.get(0).map(|v| v.as_string()).unwrap_or_default();
                            element(
                                &doc,
                                h.trim().parse().unwrap_or(-1),
                                ELEMENT_CLASS,
                                call,
                            )
                        }
                        // jsoup returns null when there is no such id.
                        _ => Ok(CfmlValue::Null),
                    }
                }
                // attr( name ) reads; attr( name, value ) writes and returns the
                // element for chaining.
                "attr" => {
                    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                    if args.len() >= 2 {
                        call(
                            &doc,
                            "setAttr",
                            vec![
                                CfmlValue::Int(node),
                                CfmlValue::string(name),
                                CfmlValue::string(args[1].as_string()),
                            ],
                        )?;
                        return Ok(object.clone());
                    }
                    call(&doc, "attr", vec![CfmlValue::Int(node), CfmlValue::string(name)])
                }
                "hasattr" => {
                    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                    let v = call(
                        &doc,
                        "attr",
                        vec![CfmlValue::Int(node), CfmlValue::string(name)],
                    )?;
                    Ok(CfmlValue::Bool(!v.as_string().is_empty()))
                }
                "removeattr" => {
                    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                    call(
                        &doc,
                        "removeAttr",
                        vec![CfmlValue::Int(node), CfmlValue::string(name)],
                    )?;
                    Ok(object.clone())
                }
                "attributes" => {
                    let map = call(&doc, "attributes", vec![CfmlValue::Int(node)])?;
                    let mut m = shim(ATTRIBUTES_CLASS);
                    m.insert("__attrs".to_string(), map);
                    Ok(CfmlValue::strukt(m))
                }
                "text" | "wholetext" => {
                    call(&doc, "text", vec![CfmlValue::Int(node)])
                }
                // A DataNode's raw content — the CSS inside a <style>. NOT
                // whitespace-collapsed and NOT escaped: collapsing it would
                // change what the stylesheet means.
                "data" | "getwholedata" => call(&doc, "data", vec![CfmlValue::Int(node)]),
                // Document.html() is the whole document; Element.html() is the
                // element's INNER html. jsoup draws exactly that distinction.
                "html" => {
                    if is_doc {
                        call(&doc, "toString", vec![])
                    } else {
                        call(&doc, "html", vec![CfmlValue::Int(node)])
                    }
                }
                "outerhtml" => call(&doc, "outerHtml", vec![CfmlValue::Int(node)]),
                "tostring" => {
                    if is_doc {
                        call(&doc, "toString", vec![])
                    } else {
                        call(&doc, "outerHtml", vec![CfmlValue::Int(node)])
                    }
                }
                "tagname" | "nodename" => call(&doc, "tagName", vec![CfmlValue::Int(node)]),
                // Identity: stable for one element, distinct between two — the
                // node handle is exactly that. Callers group by it.
                "hashcode" => Ok(CfmlValue::Int(node)),
                "equals" => {
                    let other = args.first().cloned().unwrap_or(CfmlValue::Null);
                    Ok(CfmlValue::Bool(get_int(&other, "__node") == node))
                }
                // Output settings are presentation knobs (charset, pretty-print,
                // escape mode) over a serialiser that has none of them: this
                // adapter emits the document as parsed, in UTF-8. Accepted and
                // ignored, fluently, because callers chain off them — and
                // recorded in docs/known-issues.md rather than left silent.
                "outputsettings" => Ok(CfmlValue::strukt(shim(OUTPUT_SETTINGS_CLASS))),
                "body" | "head" | "root" => {
                    let tag = if method == "head" { "head" } else { "body" };
                    let handles = call(&doc, "select", vec![CfmlValue::string(tag.to_string())])?;
                    match handles {
                        CfmlValue::Array(ref a) if !a.is_empty() => element(
                            &doc,
                            a.get(0).map(|v| v.as_string()).unwrap_or_default().trim().parse().unwrap_or(-1),
                            ELEMENT_CLASS,
                            call,
                        ),
                        // A fragment has no <body>; jsoup synthesises one, but
                        // here the document root is the honest answer.
                        _ => Ok(object.clone()),
                    }
                }
                other => Err(unsupported(
                    if is_doc { "org.jsoup.nodes.Document" } else { "org.jsoup.nodes.Element" },
                    other,
                )),
            }
        }

        // ---- org.jsoup.nodes.Attributes -------------------------------------
        ATTRIBUTES_CLASS => {
            let attrs = get(object, "__attrs").unwrap_or(CfmlValue::strukt(ValueMap::default()));
            match method {
                // jsoup's Attributes.get() answers "" for an absent key, and
                // callers Trim() it — a null here would break the next call.
                "get" | "getignorecase" => {
                    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                    Ok(match &attrs {
                        CfmlValue::Struct(s) => match s.get_ci(&name) {
                            Some(v) => v,
                            None => CfmlValue::string(String::new()),
                        },
                        _ => CfmlValue::string(String::new()),
                    })
                }
                "haskey" | "haskeyignorecase" => {
                    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                    Ok(CfmlValue::Bool(matches!(
                        &attrs, CfmlValue::Struct(s) if s.get_ci(&name).is_some()
                    )))
                }
                "size" => Ok(CfmlValue::Int(match &attrs {
                    CfmlValue::Struct(s) => s.iter().count() as i64,
                    _ => 0,
                })),
                "aslist" | "asmap" | "dataset" => Ok(attrs),
                other => Err(unsupported("org.jsoup.nodes.Attributes", other)),
            }
        }

        // ---- Document.OutputSettings (fluent, inert) ------------------------
        OUTPUT_SETTINGS_CLASS => match method {
            "charset" | "prettyprint" | "escapemode" | "indentamount" | "syntax"
            | "outline" => Ok(object.clone()),
            other => Err(unsupported("org.jsoup.nodes.Document.OutputSettings", other)),
        },

        other => Err(unsupported(other, method)),
    }
}

/// Turn an array of node handles into an array of Element shims.
fn to_elements(
    doc: &CfmlValue,
    handles: CfmlValue,
    call: &dyn Fn(&CfmlValue, &str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let CfmlValue::Array(a) = handles else {
        return Ok(CfmlValue::array(Vec::new()));
    };
    let mut out = Vec::new();
    for h in a.snapshot() {
        let node = match h {
            CfmlValue::Int(n) => n,
            other => other.as_string().trim().parse().unwrap_or(-1),
        };
        out.push(element(doc, node, ELEMENT_CLASS, call)?);
    }
    Ok(CfmlValue::array(out))
}
