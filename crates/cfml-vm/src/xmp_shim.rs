//! `com.adobe.xmp.*` — Adobe XMPCore (`xmpcore.jar`).
//!
//! Preside's `XmpMetaReader.cfc` reads embedded XMP metadata out of an uploaded
//! image with exactly this shape, and nothing else:
//!
//! ```cfml
//! factory  = CreateObject( "java", "com.adobe.xmp.XMPMetaFactory", [ xmpcore.jar ] );
//! meta     = factory.parseFromString( Trim( xmp ) );
//! iterator = meta.iterator();
//! while( iterator.hasNext() ) {
//!     prop = iterator.next();
//!     path = prop.getPath();  value = prop.getValue();
//! }
//! ```
//!
//! The parsing itself is the `xmpParse()` builtin (`cfml-stdlib::xmp`), which was
//! written to mirror XMPCore's `parseFromString(...).iterator()` output: one entry
//! per *leaf* property, keyed by the canonical Adobe path
//! (`prefix:name`, `prefix:name[N]`, `prefix:name/child:name`), in document order.
//! This module is a thin adapter that re-presents that ordered map through the
//! Java iterator protocol, so the CFC runs unmodified — no `try`/`catch`, no
//! availability probe, no `RUSTCFML-NOOP` branch.
//!
//! The jar-path argument is ignored, as every other third-party shim does: there
//! is no JVM to load a jar into and the parser is native.
//!
//! **Where this is a subset of XMPCore.** A real `XMPIterator` also emits the
//! *schema* nodes (one per namespace, with an empty path and a null value) ahead
//! of that namespace's leaves, and can be steered with `skipSubtree()`. We emit
//! leaves only. That is a faithful superset for every consumer that filters on
//! "path and value are both non-empty" — which is what XMPCore's own examples and
//! Preside both do — and `skipSubtree()`/`skipSiblings()` are accepted as no-ops
//! because with no schema nodes there is no subtree left to skip. Anything beyond
//! read-then-iterate (`setProperty`, `serializeToString`, the schema registry)
//! **throws** rather than answering wrongly; see `unsupported()`.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const FACTORY_CLASS: &str = "com.adobe.xmp.xmpmetafactory";
pub const META_CLASS: &str = "com.adobe.xmp.impl.xmpmetaimpl";
pub const ITERATOR_CLASS: &str = "com.adobe.xmp.impl.xmpiteratorimpl";
pub const PROPERTY_INFO_CLASS: &str = "com.adobe.xmp.properties.xmppropertyinfo";

/// The class names a caller can hand to `createObject("java", …)`. `XMPMeta`
/// and the iterator are only ever produced *by* the factory, but callers do
/// sometimes name `XMPConst`, so accept the whole `com.adobe.xmp.` prefix at the
/// construction site and let the method dispatch decide what is actually usable.
pub fn is_xmp_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        FACTORY_CLASS | META_CLASS | ITERATOR_CLASS | PROPERTY_INFO_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

/// `com.adobe.xmp.XMPException` — what XMPCore raises for a malformed packet,
/// and the type Preside-style callers catch.
fn xmp_exception(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("com.adobe.xmp.XMPException: {}", message),
        CfmlErrorType::Custom("com.adobe.xmp.XMPException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "com.adobe.xmp.{}.{}() is not supported by RustCFML's XMPCore shim, which \
             covers parse-and-iterate only (parseFromString → iterator → getPath/getValue)",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// The standard XMP schema namespaces, by prefix. `XMPPropertyInfo.getNamespace()`
/// returns the namespace **URI**, and the flattened paths carry only the prefix, so
/// this table maps back. Unknown prefixes yield `""` rather than a guessed URI — a
/// wrong namespace is worse than an absent one.
fn namespace_uri(prefix: &str) -> &'static str {
    match prefix {
        "dc" => "http://purl.org/dc/elements/1.1/",
        "xmp" => "http://ns.adobe.com/xap/1.0/",
        "xmpidq" => "http://ns.adobe.com/xmp/Identifier/qual/1.0/",
        "xmpRights" => "http://ns.adobe.com/xap/1.0/rights/",
        "xmpMM" => "http://ns.adobe.com/xap/1.0/mm/",
        "xmpBJ" => "http://ns.adobe.com/xap/1.0/bj/",
        "xmpTPg" => "http://ns.adobe.com/xap/1.0/t/pg/",
        "xmpDM" => "http://ns.adobe.com/xmp/1.0/DynamicMedia/",
        "pdf" => "http://ns.adobe.com/pdf/1.3/",
        "photoshop" => "http://ns.adobe.com/photoshop/1.0/",
        "crs" => "http://ns.adobe.com/camera-raw-settings/1.0/",
        "tiff" => "http://ns.adobe.com/tiff/1.0/",
        "exif" => "http://ns.adobe.com/exif/1.0/",
        "aux" => "http://ns.adobe.com/exif/1.0/aux/",
        "Iptc4xmpCore" => "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/",
        "Iptc4xmpExt" => "http://iptc.org/std/Iptc4xmpExt/2008-02-29/",
        "plus" => "http://ns.useplus.org/ldf/xmp/1.0/",
        "stEvt" => "http://ns.adobe.com/xap/1.0/sType/ResourceEvent#",
        "stRef" => "http://ns.adobe.com/xap/1.0/sType/ResourceRef#",
        "rdf" => "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
        "x" => "adobe:ns:meta/",
        _ => "",
    }
}

/// Reduce a flattened path (or a caller's property name) to the comparable form
/// `name` / `name/child:name`: drop any leading `prefix:` on the FIRST segment and
/// every `[N]` array index, and fold case. `dc:title[1]` and `title` both become
/// `title`.
fn normalize_prop_name(path: &str) -> String {
    let base = path.split_once(':').map(|(_, rest)| rest).unwrap_or(path);
    let mut out = String::with_capacity(base.len());
    let mut depth = 0usize;
    for c in base.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c.to_ascii_lowercase()),
            _ => {}
        }
    }
    out
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

/// Turn the `{ path: value }` map `xmpParse()` produces into the ordered
/// `[ { path, value } ]` list an `XMPIterator` walks. Order is the map's
/// insertion order, i.e. document order — `ValueMap` is an `IndexMap`.
fn props_from_parsed(parsed: CfmlValue) -> CfmlValue {
    let mut out = Vec::new();
    if let CfmlValue::Struct(s) = parsed {
        for (path, value) in s.iter() {
            let path = path.as_str().to_string();
            let mut p = shim(PROPERTY_INFO_CLASS);
            let prefix = path.split(':').next().unwrap_or("").to_string();
            p.insert("__path".to_string(), CfmlValue::string(path));
            p.insert("__value".to_string(), CfmlValue::string(value.as_string()));
            p.insert(
                "__namespace".to_string(),
                CfmlValue::string(namespace_uri(&prefix).to_string()),
            );
            out.push(CfmlValue::strukt(p));
        }
    }
    CfmlValue::array(out)
}

/// `XMPMetaFactory`. `parseFromString` is the only method Preside calls; the
/// factory is otherwise a static holder, so `init()` just re-marks it.
///
/// `parse_xmp` is the `xmpParse()` builtin, threaded in by the caller so this
/// module stays free of a `cfml-stdlib` dependency (that crate is an optional
/// dep of `cfml-vm`).
pub fn handle_factory(
    method: &str,
    args: Vec<CfmlValue>,
    parse_xmp: impl FnOnce(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        "init" => Ok(CfmlValue::strukt(shim(FACTORY_CLASS))),
        "parsefromstring" | "parse" => {
            let xml = args.first().map(|v| v.as_string()).unwrap_or_default();
            // XMPCore rejects an empty or non-XML packet outright; so does the
            // builtin, whose error we retype as the XMPException callers catch.
            let parsed = parse_xmp(vec![CfmlValue::string(xml)])
                .map_err(|e| xmp_exception(e.message))?;
            let mut m = shim(META_CLASS);
            m.insert("__props".to_string(), props_from_parsed(parsed));
            Ok(CfmlValue::strukt(m))
        }
        // `create()` yields an empty packet — harmless and exactly representable.
        "create" => {
            let mut m = shim(META_CLASS);
            m.insert("__props".to_string(), CfmlValue::array(Vec::new()));
            Ok(CfmlValue::strukt(m))
        }
        other => Err(unsupported("XMPMetaFactory", other)),
    }
}

pub fn handle_meta(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let props = || field(object, "__props").unwrap_or_else(|| CfmlValue::array(Vec::new()));
    match method {
        // `iterator()` and `iterator(schemaNS, propPath, options)` — the
        // filtered form is not distinguishable from the flat form here, so a
        // filtered call is refused rather than silently returning everything.
        "iterator" if args.is_empty() => {
            let mut it = shim(ITERATOR_CLASS);
            it.insert("__props".to_string(), props());
            it.insert("__pos".to_string(), CfmlValue::Int(0));
            Ok(CfmlValue::strukt(it))
        }
        "getpropertystring" | "getproperty" => {
            // (schemaNS, propName). The flattened paths carry `prefix:name` and,
            // for an array or lang-alt, a `[N]` item index; XMPCore's own
            // getProperty(ns, "title") answers with the first item, so strip both
            // the prefix and the index before comparing and take the first hit.
            let want = args.get(1).map(|v| v.as_string()).unwrap_or_default();
            let want = normalize_prop_name(&want);
            if let CfmlValue::Array(a) = props() {
                for p in a.snapshot() {
                    let path = field(&p, "__path").map(|v| v.as_string()).unwrap_or_default();
                    if normalize_prop_name(&path) == want {
                        return Ok(field(&p, "__value").unwrap_or(CfmlValue::Null));
                    }
                }
            }
            Ok(CfmlValue::Null)
        }
        "doespropertyexist" => {
            let existing = handle_meta("getpropertystring", args, object)?;
            Ok(CfmlValue::Bool(!matches!(existing, CfmlValue::Null)))
        }
        other => Err(unsupported("XMPMeta", other)),
    }
}

pub fn handle_iterator(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let pos = match field(object, "__pos") {
        Some(CfmlValue::Int(n)) => n.max(0) as usize,
        _ => 0,
    };
    let items = match field(object, "__props") {
        Some(CfmlValue::Array(a)) => a.snapshot(),
        _ => Vec::new(),
    };
    match method {
        "hasnext" => Ok(CfmlValue::Bool(pos < items.len())),
        "next" => {
            if pos >= items.len() {
                // java.util.Iterator's contract for next()-past-the-end.
                return Err(CfmlError::new(
                    "java.util.NoSuchElementException: XMPIterator is exhausted".to_string(),
                    CfmlErrorType::Custom("java.util.NoSuchElementException".to_string()),
                ));
            }
            // Advance in place, so the handle the caller is looping on sees it
            // (same shared-struct write-through the ByteBuffer shim relies on).
            if let CfmlValue::Struct(s) = object {
                s.insert("__pos".to_string(), CfmlValue::Int(pos as i64 + 1));
            }
            Ok(items[pos].clone())
        }
        // With schema nodes flattened away there is no subtree or sibling group
        // left to skip, so these are genuine no-ops rather than swallowed calls.
        "skipsubtree" | "skipsiblings" => Ok(CfmlValue::Null),
        "remove" => Err(unsupported("XMPIterator", "remove")),
        other => Err(unsupported("XMPIterator", other)),
    }
}

pub fn handle_property_info(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let get = |k: &str| CfmlValue::string(field(object, k).map(|v| v.as_string()).unwrap_or_default());
    match method {
        "getpath" => Ok(get("__path")),
        "getvalue" => Ok(get("__value")),
        "getnamespace" => Ok(get("__namespace")),
        // XMPCore reports per-property option flags (array item, struct, lang
        // alternative, …). We flatten those distinctions away during parsing, so
        // there is nothing truthful to report; 0 ("no options") would be a lie
        // for array items in particular.
        "getoptions" => Err(unsupported("XMPPropertyInfo", "getOptions")),
        other => Err(unsupported("XMPPropertyInfo", other)),
    }
}
