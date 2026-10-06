//! The markdown tree's guarantees, checked over every CommonMark 0.31.2 spec
//! example and every cmark-gfm extension example.
//!
//! 1. **Lossless.** Parsing into our tree and rendering it gives exactly the
//!    HTML comrak gives straight from the source: the tree drops nothing HTML
//!    can see.
//! 2. **Round trip.** `parse( toMarkdown( tree ) )` is the same tree (ids and
//!    renderer-chosen markers aside).
//! 3. **Struct round trip.** `toStruct()` → `MarkdownDocument( struct )` is
//!    exact, ids and unknown fields included.
//!
//! An example that cannot meet a guarantee is listed below with the reason,
//! never skipped silently.
#![cfg(feature = "markdown")]

use cfml_stdlib::markdown::cfml::{doc_from_struct, doc_to_struct};
use cfml_stdlib::markdown::parse::parse_markdown;
use cfml_stdlib::markdown::render::{render_html, render_markdown};
use cfml_stdlib::markdown::tree::{normalise_inlines, Doc, Kind, Node};
use cfml_stdlib::markdown::{markdown_to_html, MdOptions};

fn corpus() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/markdown_spec");
    let mut out = Vec::new();
    let cm: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{}/commonmark-0.31.2.json", dir)).unwrap()).unwrap();
    for ex in cm.as_array().unwrap() {
        out.push((
            format!("commonmark #{} ({})", ex["example"], ex["section"].as_str().unwrap_or("")),
            ex["markdown"].as_str().unwrap().to_string(),
        ));
    }
    let gfm = std::fs::read_to_string(format!("{}/gfm-extensions.txt", dir)).unwrap();
    let fence = "```````````````````````````````` example";
    let mut n = 0;
    for chunk in gfm.split(fence).skip(1) {
        n += 1;
        let body = chunk.split("\n````````````````````````````````").next().unwrap();
        let body = body.strip_prefix('\n').unwrap_or(body);
        let md = body.split("\n.\n").next().unwrap();
        let md = if md == "." { "" } else { md };
        out.push((format!("gfm-extensions #{}", n), format!("{}\n", md).replace('→', "\t")));
    }
    for (_, md) in out.iter_mut() {
        *md = md.replace('→', "\t");
    }
    out
}

fn opts() -> MdOptions {
    MdOptions { gfm: true, footnotes: true, unsafe_html: true, ..MdOptions::default() }
}

#[test]
fn tree_is_lossless_against_comrak_html() {
    let o = opts();
    let mut failures = Vec::new();
    let all = corpus();
    for (name, md) in &all {
        let direct = markdown_to_html(md, &o);
        let doc = match parse_markdown(md, &o) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: parse error {}", name, e.message));
                continue;
            }
        };
        let via_tree = render_html(&doc, &o);
        if direct != via_tree {
            failures.push(format!("{}\n--- input\n{}--- comrak\n{}--- tree\n{}", name, md, direct, via_tree));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} examples render differently through the tree:\n\n{}",
        failures.len(),
        all.len(),
        failures.join("\n=====\n")
    );
}

/// Differences that render to the same HTML, or that only pathological
/// spec examples produce, normalised away before comparing:
///
/// * a soft break in a heading is a space (ATX headings are one line);
/// * bold directly inside bold is bold (comrak's writer can't express
///   `<strong><strong>`, and nobody means it);
/// * a space in a url is `%20` (the writer encodes it; the href is the same).
fn canon(nodes: &[Node], in_heading: bool) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    for n in nodes {
        let mut n = n.clone();
        if in_heading && matches!(n.kind, Kind::SoftBreak) {
            n.kind = Kind::Text { value: " ".into() };
        }
        if let Kind::Link { url, .. } | Kind::Image { url, .. } = &mut n.kind {
            *url = url.replace(' ', "%20");
        }
        let heading = in_heading || matches!(n.kind, Kind::Heading { .. });
        n.children = canon(&n.children, heading);
        if matches!(n.kind, Kind::Strong) {
            let mut flat = Vec::new();
            for c in n.children.drain(..) {
                if matches!(c.kind, Kind::Strong) {
                    flat.extend(c.children);
                } else {
                    flat.push(c);
                }
            }
            n.children = flat;
        }
        out.push(n);
    }
    normalise_inlines(&mut out);
    out
}

/// Structural equality, ignoring ids and renderer-chosen markers.
fn same(a: &[Node], b: &[Node]) -> bool {
    same_raw(&canon(a, false), &canon(b, false))
}

fn same_raw(a: &[Node], b: &[Node]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            let kinds = match (&x.kind, &y.kind) {
                (Kind::List { ordered: o1, start: s1, spread: p1, .. }, Kind::List { ordered: o2, start: s2, spread: p2, .. }) => {
                    o1 == o2 && p1 == p2 && (!o1 || s1 == s2)
                }
                (k1, k2) => k1 == k2,
            };
            kinds && same_raw(&x.children, &y.children)
        })
}

fn same_doc(a: &Doc, b: &Doc) -> bool {
    same(&a.children, &b.children) && same(&a.definitions, &b.definitions) && a.front_matter == b.front_matter
}

fn dump(nodes: &[Node], depth: usize, out: &mut String) {
    for n in nodes {
        out.push_str(&format!("{}{:?}\n", "  ".repeat(depth), n.kind));
        dump(&n.children, depth + 1, out);
    }
}

/// Examples whose tree legitimately cannot survive a markdown round trip.
/// Each is a spec example that exists to show a parse quirk, not a document
/// anyone builds; the reason is given.
const ROUND_TRIP_EXCEPTIONS: &[(&str, &str)] = &[];

#[test]
fn markdown_round_trip_is_stable() {
    let o = opts();
    let mut failures = Vec::new();
    let all = corpus();
    for (name, md) in &all {
        if ROUND_TRIP_EXCEPTIONS.iter().any(|(n, _)| name.starts_with(n)) {
            continue;
        }
        let Ok(t1) = parse_markdown(md, &o) else { continue };
        let out = render_markdown(&t1, &o);
        let t2 = match parse_markdown(&out, &o) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: reparse error {}", name, e.message));
                continue;
            }
        };
        if !same_doc(&t1, &t2) {
            let (mut d1, mut d2) = (String::new(), String::new());
            dump(&t1.children, 0, &mut d1);
            dump(&t2.children, 0, &mut d2);
            failures.push(format!(
                "{}\n--- input\n{}--- rendered\n{}--- tree\n{}--- reparsed\n{}",
                name, md, out, d1, d2
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} examples change across a markdown round trip:\n\n{}",
        failures.len(),
        all.len(),
        failures.join("\n=====\n")
    );
}

#[test]
fn struct_round_trip_is_exact() {
    let o = opts();
    let mut failures = Vec::new();
    for (name, md) in corpus() {
        let Ok(t1) = parse_markdown(&md, &o) else { continue };
        let v = doc_to_struct(&t1);
        let t2 = match doc_from_struct(&v, &o) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: reload error {}", name, e.message));
                continue;
            }
        };
        let ids = |d: &Doc| {
            let mut v = Vec::new();
            d.walk(&mut |n| v.push(n.id.clone()));
            v
        };
        if !same_doc(&t1, &t2) || ids(&t1) != ids(&t2) {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "struct round trip changed: {:?}", failures);
}

// ---------------------------------------------------------------------------
// inline slots, markdownEscape, safe mode
// ---------------------------------------------------------------------------

fn inline_text(s: &str) -> (String, Vec<&'static str>) {
    let nodes = cfml_stdlib::markdown::inline::parse_inline(s, &opts()).unwrap();
    let text: String = nodes.iter().map(|n| n.plain_text()).collect();
    (text, nodes.iter().map(|n| n.kind.type_name()).collect())
}

#[test]
fn inline_slot_keeps_line_start_block_syntax_literal() {
    for s in [
        "1. Introduction", "10) Ten", "# hashtag", "## two", "- dash", "+ plus", "* star", "> quote", "| a | b |",
        "---", "***", "___", "===", "~~~ fence", "``` fence", ": colon", "2024. A year",
    ] {
        let (text, kinds) = inline_text(s);
        assert_eq!(text, s, "inline slot changed [{}] into {:?}", s, kinds);
        assert_eq!(kinds, vec!["text"], "[{}] should be plain text, got {:?}", s, kinds);
    }
    // Leading indentation is not a code block in an inline slot. (The edge
    // space survives as a run separator; the block trims it.)
    let (text, kinds) = inline_text("    indented");
    assert_eq!((text.trim(), kinds), ("indented", vec!["text"]));
    // Multi-line: block syntax on a later line is literal too.
    let (text, _) = inline_text("first\n- second\n1. third");
    assert_eq!(text, "first - second 1. third");
}

#[test]
fn inline_slot_still_parses_inline_markdown() {
    let (_, kinds) = inline_text("**bold** and `code` and [link](https://x.test) and ~~gone~~");
    assert_eq!(kinds, vec!["strong", "text", "inlineCode", "text", "link", "text", "delete"]);
    // Emphasis at the start of a line is inline, so it is left alone.
    assert_eq!(inline_text("*em* first").1, vec!["emphasis", "text"]);
    assert_eq!(inline_text("`code` first").1, vec!["inlineCode", "text"]);
    assert_eq!(inline_text("```code``` first").1, vec!["inlineCode", "text"]);
}

#[test]
fn inline_slot_rejects_a_blank_line() {
    assert!(cfml_stdlib::markdown::inline::parse_inline("one\n\ntwo", &opts()).is_err());
}

#[test]
fn markdown_escape_reads_back_literally() {
    let samples = [
        "*a* _b_ `c` [d](e) <f> &amp; ~~x~~ \\ ! | #",
        "1. not a list",
        "- not a list",
        "# not a heading",
        "> not a quote",
        "www.example.com http://example.com a@b.test",
        "C* and __init__ and 2*3*4",
        "=== --- ***",
        "a\n- b\n1) c",
        "<script>alert(1)</script>",
    ];
    for s in samples {
        let doc = parse_markdown(&cfml_stdlib::markdown::inline::escape_literal(s), &opts()).unwrap();
        let mut kinds = Vec::new();
        doc.walk(&mut |n| kinds.push(n.kind.type_name()));
        assert!(
            kinds.iter().all(|k| matches!(*k, "paragraph" | "text" | "softBreak")),
            "markdownEscape([{}]) still has markup: {:?}",
            s,
            kinds
        );
        let text: Vec<String> = doc.children.iter().map(|n| n.plain_text()).collect();
        let want: String = s.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert_eq!(text.join(" "), want, "markdownEscape([{}]) changed the text", s);
    }
}

#[test]
fn safe_mode_blanks_dangerous_urls_and_drops_raw_html() {
    let safe = MdOptions::default();
    let html = markdown_to_html("[x](javascript:alert(1)) <b>raw</b> ![i](data:text/html,x)", &safe);
    assert!(html.contains("<a href=\"\">x</a>"), "{}", html);
    assert!(!html.contains("<b>"), "{}", html);
    assert!(!html.contains("data:text/html"), "{}", html);
    let unsafe_o = MdOptions { unsafe_html: true, ..MdOptions::default() };
    let html = markdown_to_html("<b>raw</b>", &unsafe_o);
    assert!(html.contains("<b>raw</b>"), "{}", html);
    let esc = MdOptions { escape_html: true, ..MdOptions::default() };
    let html = markdown_to_html("<b>raw</b>", &esc);
    assert!(html.contains("&lt;b&gt;raw&lt;/b&gt;"), "{}", html);
}
