//! `style` attribute filtering against the policy's `<css-rules>`.
//!
//! The policies declare `<attribute name="style"/>` with **no rules of its own**
//! and a comment saying it "will be validated by an inline stylesheet scanner" —
//! so all of `style=` validation lives here. Each declaration is looked up by
//! property name and kept only if its value matches one of that property's
//! literals or regexps; everything else is dropped.
//!
//! Two rules that are not negotiable:
//!
//! * **Never fetch anything.** `@import` and remote `url()` are rejected, never
//!   resolved — the policy's `embedStyleSheets=false` says so, and a sanitiser
//!   that made network calls would be a request-forgery primitive.
//! * **Reject any `url()` that is not a plain http(s)/relative reference**, so
//!   `url(javascript:…)` and `url(data:…)` cannot smuggle script past a
//!   property whose regexp was written loosely.

use crate::policy::Policy;

/// Filter the CONTENT of a `<style>` element — a stylesheet, not the bare
/// declaration list a `style=` attribute holds.
///
/// The policies mark `<style>` as `action="validate"`, so the element survives;
/// without this its text would reach the browser untouched, and `@import` or
/// `expression()` inside it would be live. (That is not hypothetical: it is what
/// this sanitiser did until the OWASP vector corpus caught it.)
///
/// At-rules are dropped wholesale rather than parsed. `@import` must never be
/// honoured, and the rest (`@media`, `@supports`, …) nest further rule blocks
/// that would need a real stylesheet parser to filter safely — dropping them
/// loses some legitimate styling, which is recorded in `docs/known-issues.md`,
/// and is the conservative direction.
pub fn filter_stylesheet(css: &str, policy: &Policy) -> String {
    let css = strip_comments(css);
    let mut out: Vec<String> = Vec::new();
    let mut rest = css.as_str();

    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim();
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let body = &after[..close];
        rest = &after[close + 1..];

        if selector.is_empty() || !selector_is_safe(selector) {
            continue;
        }
        let declarations = filter_declarations(body, policy);
        if !declarations.is_empty() {
            out.push(format!("{} {{ {} }}", selector, declarations));
        }
    }
    // AntiSamy wraps the scanned stylesheet in a CDATA section (the policies set
    // useXHTML=true) and emits an empty comment when nothing survived, so an
    // emptied <style> is still well-formed. Matched here so a cross-engine diff
    // of the two sanitisers stays quiet.
    let body = if out.is_empty() { "/* */".to_string() } else { out.join("\n") };
    let result = format!("<![CDATA[{}]]>", body);
    // `<style>` content is a raw-text element: the serialiser must NOT escape it
    // (that would corrupt the CSS), so a stray `<` in the FILTERED CSS would
    // re-enter HTML parsing as markup. The CDATA wrapper above is a fixed
    // literal we control; the filtered body is allowlisted and cannot contain
    // one — this refuses the whole stylesheet if that ever stops being true.
    if body.contains('<') {
        return "<![CDATA[/* */]]>".to_string();
    }
    result
}

/// A selector cannot execute anything by itself, but it can carry markup that
/// re-enters HTML parsing, or an at-rule that would change the meaning of the
/// block. Anything but a plain selector is refused.
fn selector_is_safe(selector: &str) -> bool {
    let lowered = selector.to_ascii_lowercase();
    !lowered.contains('<')
        && !lowered.contains('>')
        && !lowered.contains('@')
        && !lowered.contains("expression(")
        && !lowered.contains("javascript:")
        && !lowered.contains("url(")
}

/// Remove `/* … */` comments before any other pass, so they cannot be used to
/// split a keyword the later checks look for.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            // Unterminated comment: everything after it is commented out.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Filter a `style` attribute value, returning only the declarations the policy
/// accepts. An empty result means the attribute should be removed entirely.
pub fn filter_declarations(style: &str, policy: &Policy) -> String {
    let mut kept: Vec<String> = Vec::new();

    for declaration in split_declarations(style) {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            continue;
        }
        // A CSS comment inside a declaration is a classic filter-evasion trick
        // (`col/*x*/or`), and a policy lookup on the commented name would miss.
        if property.contains("/*") || value.contains("/*") || value.contains("*/") {
            continue;
        }
        if !url_tokens_are_safe(value) {
            continue;
        }
        if accepts(&property, value, policy) {
            kept.push(format!("{}: {};", property, value));
        }
    }

    // AntiSamy terminates every declaration, including the last — matched so a
    // cross-engine diff of the two sanitisers stays quiet.
    kept.join(" ")
}

/// Does the policy accept `value` for `property`? A property the policy never
/// names is rejected — an allowlist, not a denylist, so a CSS feature invented
/// after the policy was written cannot slip through unvalidated.
fn accepts(property: &str, value: &str, policy: &Policy) -> bool {
    let Some(rule) = policy.css_properties.get(property) else {
        return false;
    };
    if rule
        .literals
        .iter()
        .any(|l| l.eq_ignore_ascii_case(value))
    {
        return true;
    }
    if rule.regexps.iter().any(|r| {
        r.find(value)
            .map(|m| m.start() == 0 && m.end() == value.len())
            .unwrap_or(false)
    }) {
        return true;
    }
    // A shorthand (`background`, `font`, …) accepts a space-separated list whose
    // every component is valid for one of the properties it expands into.
    if !rule.shorthands.is_empty() {
        let parts: Vec<&str> = value.split_whitespace().collect();
        if !parts.is_empty()
            && parts.iter().all(|part| {
                rule.shorthands
                    .iter()
                    .any(|target| accepts(target, part, policy))
            })
        {
            return true;
        }
    }
    false
}

/// Every `url(...)` in the value must reference a plain http(s) or relative
/// target. `javascript:`, `data:`, `vbscript:` and anything else are refused
/// outright rather than left to a property regexp to catch.
fn url_tokens_are_safe(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    if lowered.contains("@import") || lowered.contains("expression(") {
        return false;
    }
    let mut rest = lowered.as_str();
    while let Some(start) = rest.find("url(") {
        let after = &rest[start + 4..];
        let Some(end) = after.find(')') else {
            // An unterminated url( is malformed; refuse rather than guess.
            return false;
        };
        let target = after[..end].trim().trim_matches(|c| c == '"' || c == '\'');
        // A scheme is anything before the first ':' that looks like a scheme.
        if let Some((scheme, _)) = target.split_once(':') {
            let scheme = scheme.trim();
            let scheme_like = !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
            if scheme_like && !matches!(scheme, "http" | "https") {
                return false;
            }
        }
        rest = &after[end + 1..];
    }
    true
}

/// Split on `;`, ignoring semicolons inside quotes or `url(...)`.
fn split_declarations(style: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;

    for ch in style.chars() {
        match ch {
            '\'' | '"' => {
                match quote {
                    Some(q) if q == ch => quote = None,
                    None => quote = Some(ch),
                    _ => {}
                }
                current.push(ch);
            }
            '(' if quote.is_none() => {
                depth += 1;
                current.push(ch);
            }
            ')' if quote.is_none() => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ';' if quote.is_none() && depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Policy;

    const CSS_POLICY: &str = r##"
    <anti-samy-rules>
      <tag-rules><tag name="div" action="validate"/></tag-rules>
      <css-rules>
        <property name="color">
          <literal-list><literal value="red"/><literal value="blue"/></literal-list>
          <regexp-list><regexp value="#[0-9a-fA-F]{6}"/></regexp-list>
        </property>
        <property name="background-image">
          <regexp-list><regexp value="url\(.*\)"/></regexp-list>
        </property>
        <property name="background-color">
          <literal-list><literal value="red"/></literal-list>
        </property>
        <property name="background">
          <shorthand-list>
            <shorthand name="background-color"/>
            <shorthand name="background-image"/>
          </shorthand-list>
        </property>
      </css-rules>
    </anti-samy-rules>"##;

    fn policy() -> Policy {
        Policy::from_xml_str(CSS_POLICY).unwrap()
    }

    #[test]
    fn keeps_declarations_the_policy_allows() {
        let p = policy();
        assert_eq!(filter_declarations("color: red", &p), "color: red;");
        assert_eq!(filter_declarations("color: #AABBCC", &p), "color: #AABBCC;");
    }

    #[test]
    fn drops_unknown_properties_and_values() {
        let p = policy();
        assert_eq!(filter_declarations("position: fixed", &p), "");
        assert_eq!(filter_declarations("color: chartreuse", &p), "");
    }

    #[test]
    fn keeps_the_valid_half_of_a_mixed_declaration_list() {
        let p = policy();
        assert_eq!(
            filter_declarations("color: red; position: fixed; color: blue", &p),
            "color: red; color: blue;"
        );
    }

    #[test]
    fn rejects_script_urls_however_they_are_spelled() {
        let p = policy();
        for vector in [
            "background-image: url(javascript:alert(1))",
            "background-image: url('javascript:alert(1)')",
            "background-image: url(\"data:text/html;base64,PHNjcmlwdD4=\")",
            "background-image: url(vbscript:msgbox)",
        ] {
            assert_eq!(filter_declarations(vector, &p), "", "vector: {}", vector);
        }
        // …while a plain remote image still passes the property's own regexp.
        assert_eq!(
            filter_declarations("background-image: url(https://ok.test/a.png)", &p),
            "background-image: url(https://ok.test/a.png);"
        );
    }

    #[test]
    fn rejects_comment_obfuscation_and_expressions() {
        let p = policy();
        assert_eq!(filter_declarations("col/*x*/or: red", &p), "");
        assert_eq!(filter_declarations("width: expression(alert(1))", &p), "");
        assert_eq!(filter_declarations("color: red/*", &p), "");
    }

    #[test]
    fn never_resolves_imports() {
        let p = policy();
        assert_eq!(
            filter_declarations("background-image: @import url(https://evil.test/x.css)", &p),
            ""
        );
    }

    #[test]
    fn shorthand_accepts_only_component_valid_values() {
        let p = policy();
        assert_eq!(
            filter_declarations("background: red", &p),
            "background: red;"
        );
        assert_eq!(filter_declarations("background: chartreuse", &p), "");
    }

    #[test]
    fn semicolons_inside_url_do_not_split_declarations() {
        let p = policy();
        // The `;` inside the data URI must not create a second declaration —
        // and the whole thing must still be rejected.
        assert_eq!(
            filter_declarations("background-image: url(data:text/html;base64,x); color: red", &p),
            "color: red;"
        );
    }
}
