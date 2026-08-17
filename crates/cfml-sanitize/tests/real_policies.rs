//! The parser and sanitiser against the REAL shipped AntiSamy policies.
//!
//! The unit tests use hand-written miniature policies, which cannot catch "the
//! production file has a construct we silently skip". These run the six 1.4.4
//! policies Preside vendors (also shipped in Lucee's test tree) through the
//! parser and assert both that they parse into something substantial and that
//! the resulting policy actually blocks the classic vectors.
//!
//! The files are located at run time and the tests skip (rather than fail) when
//! no checkout is present, so the suite stays green on a machine that has only
//! this repo. `POLICY_DIR` overrides the search.

use cfml_sanitize::{sanitize, Policy};

const POLICY_NAMES: &[&str] = &[
    "antisamy-preside-1.4.4.xml",
    "antisamy-tinymce-1.4.4.xml",
    "antisamy-slashdot-1.4.4.xml",
    "antisamy-ebay-1.4.4.xml",
    "antisamy-myspace-1.4.4.xml",
    "antisamy-anythinggoes-1.4.4.xml",
];

/// Candidate directories, in preference order. A sibling checkout is the normal
/// case on a development machine.
fn policy_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("POLICY_DIR") {
        let path = std::path::PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        format!("{home}/Repos/opensource/Preside-CMS/system/services/security/antisamylib"),
        format!("{home}/Repos/opensource/CFMLs/Lucee/test/tickets/LDEV5085/antisamylib"),
    ];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_dir())
}

fn load(name: &str) -> Option<Policy> {
    let dir = policy_dir()?;
    let path = dir.join(name);
    if !path.exists() {
        return None;
    }
    Some(
        Policy::from_xml_file(path.to_str().unwrap())
            .unwrap_or_else(|e| panic!("{name} failed to parse: {e}")),
    )
}

#[test]
fn every_shipped_policy_parses_into_something_substantial() {
    let Some(dir) = policy_dir() else {
        eprintln!("skipped: no AntiSamy policy checkout found");
        return;
    };
    for name in POLICY_NAMES {
        let Some(policy) = load(name) else {
            eprintln!("skipped {name}: not present in {}", dir.display());
            continue;
        };
        assert!(
            !policy.tags.is_empty(),
            "{name} parsed with no tag rules at all"
        );
        // A policy that parsed but lost its attribute or CSS rules would
        // sanitise far too aggressively while still looking "successful".
        assert!(
            !policy.common_attributes.is_empty(),
            "{name} parsed with no common attributes"
        );
    }
}

#[test]
fn the_preside_policy_has_the_shape_the_file_declares() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    // Counted from the file itself; these are the numbers the plan documented.
    assert_eq!(policy.max_input_size(), Some(200_000));
    assert!(
        policy.tags.len() >= 60,
        "expected ~63 tag rules, got {}",
        policy.tags.len()
    );
    assert!(
        policy.css_properties.len() >= 100,
        "expected ~119 css properties, got {}",
        policy.css_properties.len()
    );
    // 42, not the 184 a naive grep of `<attribute ` reports: that counts every
    // section of the file, and one declaration inside <common-attributes> is
    // commented out (tabindex).
    assert!(
        policy.common_attributes.len() >= 40,
        "expected ~42 common attributes, got {}",
        policy.common_attributes.len()
    );

    // The specific rules the plan calls out.
    use cfml_sanitize::{OnInvalid, TagAction};
    assert_eq!(policy.tags.get("script").map(|t| t.action), Some(TagAction::Remove));
    assert_eq!(policy.tags.get("iframe").map(|t| t.action), Some(TagAction::Remove));
    assert_eq!(policy.tags.get("dd").map(|t| t.action), Some(TagAction::Truncate));
    assert_eq!(policy.tags.get("g").map(|t| t.action), Some(TagAction::Encode));
    assert_eq!(
        policy.attribute_rule("img", "src").map(|r| r.on_invalid),
        Some(OnInvalid::RemoveTag)
    );
}

/// Every regexp in every shipped policy must have compiled. A pattern the
/// `regex` crate rejected would be silently dropped, leaving its rule unable to
/// match — safe, but it would reject legitimate content, so it must not happen
/// silently. Proxy check: known-good values for well-known attributes are
/// accepted, which they cannot be if their pattern was dropped.
#[test]
fn the_policies_regexps_all_compiled() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    let href = policy.attribute_rule("a", "href").expect("a@href rule");
    assert!(href.accepts("https://example.test/page?a=1"), "offsiteURL dropped");
    assert!(href.accepts("/relative/path"), "onsiteURL dropped");

    let width = policy.attribute_rule("img", "width").expect("img@width rule");
    assert!(width.accepts("120"), "number regexp dropped");
}

/// The OWASP filter-evasion classics, under the real Preside policy. None may
/// survive in an executable form.
#[test]
fn the_real_policy_blocks_the_classic_vectors() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    let vectors = [
        r#"<script>alert(1)</script>"#,
        r#"<SCRIPT SRC=https://evil.test/xss.js></SCRIPT>"#,
        r#"<IMG SRC="javascript:alert('XSS');">"#,
        r#"<IMG SRC=javascript:alert('XSS')>"#,
        r#"<IMG SRC=JaVaScRiPt:alert('XSS')>"#,
        r#"<IMG SRC=`javascript:alert("XSS")`>"#,
        r#"<IMG SRC=# onmouseover="alert('xxs')">"#,
        r#"<IMG SRC=/ onerror="alert(String.fromCharCode(88,83,83))"></img>"#,
        r#"<img src=x onerror=alert(1)//>"#,
        r#"<IMG SRC="jav&#x09;ascript:alert('XSS');">"#,
        r#"<IMG SRC="  javascript:alert('XSS');">"#,
        r#"<BODY ONLOAD=alert('XSS')>"#,
        r#"<BODY BACKGROUND="javascript:alert('XSS')">"#,
        r#"<svg/onload=alert(1)>"#,
        r#"<svg><script>alert(1)</script></svg>"#,
        r#"<iframe src="javascript:alert('XSS');"></iframe>"#,
        r#"<object type="text/x-scriptlet" data="https://evil.test/x.html"></object>"#,
        r#"<EMBED SRC="https://evil.test/x.swf" AllowScriptAccess="always"></EMBED>"#,
        r#"<a href="javascript:alert(1)">x</a>"#,
        r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">x</a>"#,
        r#"<div style="background-image: url(javascript:alert('XSS'))">x</div>"#,
        r#"<div style="width: expression(alert('XSS'))">x</div>"#,
        r#"<STYLE>@import'https://evil.test/xss.css';</STYLE>"#,
        r#"<STYLE>li {list-style-image: url("javascript:alert('XSS')");}</STYLE>"#,
        r#"<META HTTP-EQUIV="refresh" CONTENT="0;url=javascript:alert('XSS');">"#,
        r#"<TABLE BACKGROUND="javascript:alert('XSS')"><tr><td>x</td></tr></TABLE>"#,
        r#"<DIV STYLE="background-image: url(&#1;javascript:alert('XSS'))">x</DIV>"#,
        r#"<base href="javascript:alert(1)//">"#,
        r#"<form action="javascript:alert(1)"><input type=submit></form>"#,
        r#"<isindex type=image src=1 onerror=alert(1)>"#,
        r#"<xss onafterscriptexecute=alert(1)><script>1</script>"#,
        r#"<scr<script>ipt>alert(1)</scr</script>ipt>"#,
        r#"<noscript><p title="</noscript><script>alert(1)</script>">"#,
    ];

    for vector in vectors {
        let out = sanitize(vector, &policy)
            .unwrap_or_else(|e| panic!("vector {vector} errored: {e}"));
        let lowered = out.to_lowercase();
        // Script-execution vectors, and the elements this policy removes
        // outright. NOT in this list: `<form>`, `<input>`, `<base>` and
        // `<meta>` — the Preside policy explicitly allows some of those with
        // validated attributes, and the real AntiSamy keeps them too. What
        // matters is that their dangerous ATTRIBUTES are gone, which the
        // `javascript:`/`vbscript:` entries below assert directly.
        for forbidden in [
            "<script", "javascript:", "vbscript:", "onerror", "onload", "onmouseover",
            "onafterscriptexecute", "expression(", "@import", "<iframe", "<object", "<embed",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "vector `{vector}` left `{forbidden}` in output: {out}"
            );
        }
        // Nothing may survive with an inline event handler of any name.
        assert!(
            !lowered.contains(" on") || !lowered.contains("=alert"),
            "vector `{vector}` left an event handler in output: {out}"
        );
    }
}

/// The plan's T1: `_removeUnwantedCleanses` in Preside masks `&quot;` as
/// `&~~~quot;`, scans, then replaces whatever the sanitiser turned a bare `&`
/// into. That round-trip only works if the serialiser escapes a bare `&`
/// consistently — this pins it.
#[test]
fn ampersand_escaping_is_self_consistent() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    // The probe Preside makes at runtime to calibrate itself.
    assert_eq!(sanitize("&", &policy).unwrap(), "&amp;");

    // The masked form must survive with its marker intact, so the unmask step
    // can restore the original entity.
    let masked = "He said &~~~quot;hi&~~~quot;";
    let scanned = sanitize(masked, &policy).unwrap();
    assert_eq!(scanned, "He said &amp;~~~quot;hi&amp;~~~quot;");
    let unmasked = scanned.replace("&amp;", "&").replace("&~~~quot;", "&quot;");
    assert_eq!(unmasked, "He said &quot;hi&quot;");
}

/// The `_xssProtect` shape: short scalar request values with no markup at all.
/// These must take the fast path and come back byte-identical — this is the
/// hottest path in the application the shim exists for.
#[test]
fn plain_request_values_are_returned_unchanged() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    for value in [
        "",
        "42",
        "some search term",
        "2026-08-17",
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "/admin/datamanager/object/page",
        "sort=label&direction=asc".trim_end_matches("&direction=asc"),
    ] {
        assert_eq!(sanitize(value, &policy).unwrap(), value, "changed: {value}");
    }
}

/// Legitimate rich content must survive — a sanitiser that ate valid markup
/// would "pass" every security test while being useless.
#[test]
fn legitimate_rich_content_survives() {
    let Some(policy) = load("antisamy-preside-1.4.4.xml") else {
        eprintln!("skipped: preside policy not found");
        return;
    };
    let out = sanitize(
        r#"<p>Hello <b>world</b>, see <a href="https://example.test/x">this</a>.</p>"#,
        &policy,
    )
    .unwrap();
    assert!(out.contains("<b>world</b>"), "lost bold: {out}");
    assert!(out.contains("href=\"https://example.test/x\""), "lost link: {out}");
    assert!(out.contains("<p>"), "lost paragraph: {out}");

    let table = sanitize(
        r#"<table><tr><td>a</td><td>b</td></tr></table>"#,
        &policy,
    )
    .unwrap();
    assert!(table.contains("<td>a</td>"), "lost table cell: {table}");
}
