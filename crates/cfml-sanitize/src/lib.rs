//! Policy-driven HTML sanitisation for RustCFML.
//!
//! Parses OWASP AntiSamy policy XML and applies it to HTML, backing two callers:
//! the `org.owasp.validator.html.AntiSamy` Java shim (so CFML applications that
//! reach for the real library keep working unmodified) and the `sanitizeHtml()`
//! BIF.
//!
//! ```no_run
//! # use cfml_sanitize::{Policy, sanitize};
//! let policy = Policy::from_xml_file("antisamy-preside-1.4.4.xml").unwrap();
//! assert_eq!(sanitize("<b onclick=\"x()\">hi</b>", &policy).unwrap(), "<b>hi</b>");
//! ```
//!
//! Parsed policies are cached by path in [`policy_for_file`], because callers
//! typically load a handful of policies once and then sanitise with them on
//! every request.

mod css;
mod policy;
mod sanitize;

pub use policy::{AttributeRule, CssProperty, OnInvalid, Policy, PolicyError, TagAction, TagRule};
pub use sanitize::{sanitize, SanitizeError};

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type PolicyCache = Mutex<HashMap<String, Arc<Policy>>>;

fn cache() -> &'static PolicyCache {
    static CACHE: OnceLock<PolicyCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load (and cache) the policy at `path`. Repeated calls with the same path
/// return the same parsed policy — a 2,600-line policy costs a parse once per
/// process, not once per request.
pub fn policy_for_file(path: &str) -> Result<Arc<Policy>, PolicyError> {
    if let Ok(guard) = cache().lock() {
        if let Some(found) = guard.get(path) {
            return Ok(found.clone());
        }
    }
    let parsed = Arc::new(Policy::from_xml_file(path)?);
    if let Ok(mut guard) = cache().lock() {
        guard.insert(path.to_string(), parsed.clone());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A policy shaped like the shipped ones, small enough to reason about.
    const POLICY: &str = r#"
    <anti-samy-rules>
      <directives><directive name="maxInputSize" value="200000"/></directives>
      <common-regexps>
        <regexp name="offsiteURL" value="(\s)*((ht|f)tp(s?)://|mailto:)[A-Za-z0-9]+[~a-zA-Z0-9-.@,;:_/?=&amp;#%$]*(\s)*"/>
        <regexp name="onsiteURL" value="([\p{L}\p{N}\\\.\#@\$%\+&amp;;\-_~,\?=/!]|\#(\w)+)*"/>
        <regexp name="number" value="\d+"/>
      </common-regexps>
      <common-attributes>
        <attribute name="href">
          <regexp-list><regexp name="onsiteURL"/><regexp name="offsiteURL"/></regexp-list>
        </attribute>
        <attribute name="class"><regexp-list><regexp value="[a-zA-Z0-9\s,\-_]+"/></regexp-list></attribute>
        <attribute name="alt"><regexp-list><regexp value="[\p{L}\p{N}\s\-_',:\[\]!\./\\\(\)]*"/></regexp-list></attribute>
      </common-attributes>
      <global-tag-attributes>
        <attribute name="class"/>
        <attribute name="style"/>
      </global-tag-attributes>
      <tags-to-encode><tag>g</tag></tags-to-encode>
      <tag-rules>
        <tag name="script" action="remove"/>
        <tag name="iframe" action="remove"/>
        <tag name="dd" action="truncate"/>
        <tag name="b" action="validate"/>
        <tag name="i" action="validate"/>
        <tag name="p" action="validate"/>
        <tag name="div" action="validate"/>
        <tag name="span" action="validate"/>
        <tag name="a" action="validate">
          <attribute name="href"/>
          <attribute name="target" onInvalid="remove">
            <literal-list><literal value="_blank"/></literal-list>
          </attribute>
        </tag>
        <tag name="img" action="validate">
          <attribute name="src" onInvalid="removeTag">
            <regexp-list><regexp name="offsiteURL"/></regexp-list>
          </attribute>
          <attribute name="alt"/>
          <attribute name="width"><regexp-list><regexp name="number"/></regexp-list></attribute>
        </tag>
      </tag-rules>
      <css-rules>
        <property name="color">
          <literal-list><literal value="red"/></literal-list>
        </property>
      </css-rules>
    </anti-samy-rules>"#;

    fn policy() -> Policy {
        Policy::from_xml_str(POLICY).unwrap()
    }

    fn clean(html: &str) -> String {
        sanitize(html, &policy()).unwrap()
    }

    #[test]
    fn text_without_markup_is_returned_untouched() {
        // The fast path: no `<` and no `&` means nothing can change.
        assert_eq!(clean("just a plain search term"), "just a plain search term");
        assert_eq!(clean(""), "");
    }

    #[test]
    fn allowed_markup_survives() {
        assert_eq!(clean("<b>bold</b> and <i>italic</i>"), "<b>bold</b> and <i>italic</i>");
        assert_eq!(
            clean(r#"<a href="https://ok.test/x">link</a>"#),
            r#"<a href="https://ok.test/x">link</a>"#
        );
    }

    #[test]
    fn script_elements_are_removed_with_their_contents() {
        assert_eq!(clean("<script>alert(1)</script>"), "");
        assert_eq!(clean("before<script>alert(1)</script>after"), "beforeafter");
        assert_eq!(clean("<iframe src='https://evil.test'></iframe>"), "");
    }

    #[test]
    fn event_handlers_never_survive() {
        assert_eq!(clean(r#"<b onclick="alert(1)">x</b>"#), "<b>x</b>");
        // Casing and unusual spellings must not slip past.
        assert_eq!(clean(r#"<b OnMouseOver="alert(1)">x</b>"#), "<b>x</b>");
        assert_eq!(clean(r#"<div onfocus="alert(1)">x</div>"#), "<div>x</div>");
    }

    #[test]
    fn unknown_tags_are_unwrapped_but_their_text_is_kept() {
        assert_eq!(clean("<blink>still here</blink>"), "still here");
        assert_eq!(clean("<marquee><b>bold</b></marquee>"), "<b>bold</b>");
    }

    #[test]
    fn on_invalid_remove_tag_drops_the_whole_element() {
        // An <img> whose src fails validation is meaningless, so the policy
        // asks for the element itself to go — not just the attribute.
        assert_eq!(clean(r#"<img src="javascript:alert(1)" alt="x">"#), "");
        // …while a valid src keeps it, with its attributes in SOURCE order.
        // They used to come back alphabetised — scraper sorts them unless its
        // `deterministic` feature is on, which it now is — and that reordering
        // was a (cosmetic) divergence from the Java library. Turning the feature
        // on removed it, so this asserts the input's own order.
        assert_eq!(
            clean(r#"<img src="https://ok.test/a.png" alt="x" />"#),
            r#"<img src="https://ok.test/a.png" alt="x" />"#
        );
    }

    #[test]
    fn on_invalid_remove_attribute_keeps_the_element() {
        assert_eq!(
            clean(r#"<a href="https://ok.test/" target="_evil">x</a>"#),
            r#"<a href="https://ok.test/">x</a>"#
        );
    }

    #[test]
    fn truncate_keeps_the_element_and_text_but_no_attributes() {
        assert_eq!(clean(r#"<dd class="a">text</dd>"#), "<dd>text</dd>");
    }

    #[test]
    fn tags_to_encode_are_unwrapped_like_the_real_library() {
        // Measured against AntiSamy 1.5.3: despite the section name, these are
        // unwrapped rather than encoded.
        assert_eq!(clean("<g>keep</g>"), "keep");
        assert_eq!(clean("<b><g>nested</g></b>"), "<b>nested</b>");
    }

    #[test]
    fn javascript_urls_are_rejected_in_href() {
        for vector in [
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<a href="JaVaScRiPt:alert(1)">x</a>"#,
            r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#,
        ] {
            let out = clean(vector);
            assert!(
                !out.to_lowercase().contains("javascript:") && !out.to_lowercase().contains("data:"),
                "vector {vector} produced {out}"
            );
        }
    }

    #[test]
    fn style_is_filtered_by_the_css_rules() {
        assert_eq!(
            clean(r#"<div style="color: red">x</div>"#),
            r#"<div style="color: red;">x</div>"#
        );
        // An unlisted property, and a script URL, both go. The attribute is
        // left in place but empty, which is what AntiSamy does.
        assert_eq!(
            clean(r#"<div style="position: fixed">x</div>"#),
            r#"<div style="">x</div>"#
        );
        assert_eq!(
            clean(r#"<div style="background-image: url(javascript:alert(1))">x</div>"#),
            r#"<div style="">x</div>"#
        );
    }

    #[test]
    fn comments_are_dropped() {
        assert_eq!(clean("a<!-- [if IE]><script>alert(1)</script><![endif] -->b"), "ab");
    }

    #[test]
    fn text_is_escaped_on_the_way_out() {
        assert_eq!(clean("5 < 6 & 7 > 4"), "5 &lt; 6 &amp; 7 &gt; 4");
    }

    #[test]
    fn quoting_in_attribute_values_cannot_break_out() {
        let out = clean(r#"<a href="https://ok.test/&quot;onmouseover=alert(1)">x</a>"#);
        assert!(!out.contains("onmouseover=alert"), "got {out}");
    }

    #[test]
    fn nesting_and_malformed_markup_do_not_reintroduce_script() {
        for vector in [
            "<scr<script>ipt>alert(1)</script>",
            "<<SCRIPT>alert(1);//<</SCRIPT>",
            "<img src=x onerror=alert(1)>",
            "<svg/onload=alert(1)>",
            "<body onload=alert(1)>",
            "<a href=\"jav&#x09;ascript:alert(1)\">x</a>",
            "<div><style>@import 'https://evil.test/x.css';</style></div>",
        ] {
            let out = clean(vector).to_lowercase();
            assert!(!out.contains("onerror"), "vector {vector} -> {out}");
            assert!(!out.contains("onload"), "vector {vector} -> {out}");
            assert!(!out.contains("<script"), "vector {vector} -> {out}");
            assert!(!out.contains("javascript:"), "vector {vector} -> {out}");
        }
    }

    #[test]
    fn input_over_max_input_size_is_refused_rather_than_truncated() {
        let mut small = Policy::from_xml_str(POLICY).unwrap();
        small
            .directives
            .insert("maxinputsize".to_string(), "10".to_string());
        assert!(sanitize("<b>plenty longer than ten bytes</b>", &small).is_err());
    }
}
