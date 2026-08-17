//! OWASP AntiSamy policy XML → a compiled [`Policy`].
//!
//! The format is the 1.4.4 schema every shipped policy file uses (Preside
//! vendors six of them; `antisamy-preside-1.4.4.xml` is ~2,600 lines). The
//! sections that carry meaning for sanitisation:
//!
//! | Section | Meaning |
//! |---|---|
//! | `<directives>` | output/parse switches — only `maxInputSize` is enforced here |
//! | `<common-regexps>` | named regexps referenced by attribute rules |
//! | `<common-attributes>` | the default rule for an attribute of a given name |
//! | `<global-tag-attributes>` | attributes allowed on *any* validated tag |
//! | `<tags-to-encode>` | tags rendered as literal text rather than markup |
//! | `<tag-rules>` | per-tag action plus tag-specific attribute overrides |
//! | `<css-rules>` | per-property validation for `style` attributes/elements |
//!
//! Parsing is a once-per-policy cost (callers cache the handle), so this favours
//! clarity over speed; the hot path is [`crate::sanitize`].

use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// What to do when an attribute's value fails validation.
///
/// AntiSamy's default is `removeAttribute` — drop the attribute, keep the
/// element. The other two are declared per-attribute in the policy and matter:
/// an `<img>` whose `src` is invalid is meaningless, so the policy asks for the
/// whole element to go rather than leaving a bare `<img>` behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OnInvalid {
    #[default]
    RemoveAttribute,
    /// Drop the element **and its contents**.
    RemoveTag,
    /// Drop the element but keep its children (unwrap it).
    FilterTag,
}

impl OnInvalid {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "removetag" => OnInvalid::RemoveTag,
            "filtertag" => OnInvalid::FilterTag,
            // "remove" and "removeAttribute" are the same instruction; anything
            // unrecognised falls back to the least destructive action.
            _ => OnInvalid::RemoveAttribute,
        }
    }
}

/// What to do with an element the policy names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagAction {
    /// Keep the element, validate its attributes.
    Validate,
    /// Drop the element, keep its children.
    Filter,
    /// Drop the element **and its contents** — `script`, `iframe`, `frameset`.
    Remove,
    /// Keep the element and its text, drop every attribute.
    Truncate,
    /// Render the element as literal text (`<tags-to-encode>`).
    Encode,
}

impl TagAction {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "remove" => TagAction::Remove,
            "truncate" => TagAction::Truncate,
            "filter" => TagAction::Filter,
            _ => TagAction::Validate,
        }
    }
}

/// One attribute's validation rule.
///
/// The distinction that matters for safety is [`Self::constrained`]: a rule that
/// declares NO lists at all is how the policy spells an unconstrained attribute
/// (`<attribute name="align"/>`) and accepts anything, whereas a rule that
/// declared lists which then resolved to nothing accepts NOTHING. Collapsing
/// those two into "empty lists" makes an unresolvable rule fail OPEN — which is
/// exactly how `javascript:` URLs survived under the tinymce policy until a diff
/// against the real AntiSamy caught it.
#[derive(Clone, Debug, Default)]
pub struct AttributeRule {
    pub on_invalid: OnInvalid,
    pub regexps: Vec<Regex>,
    /// Compared case-insensitively, as AntiSamy does.
    pub literals: Vec<String>,
    /// True when the policy declared a `<regexp-list>` or `<literal-list>` for
    /// this attribute, whether or not anything in it resolved.
    pub constrained: bool,
}

impl AttributeRule {
    /// A rule that declares no lists accepts anything; one that declares lists
    /// accepts only what they match.
    pub fn accepts(&self, value: &str) -> bool {
        if !self.constrained {
            return true;
        }
        if self
            .literals
            .iter()
            .any(|l| l.eq_ignore_ascii_case(value.trim()))
        {
            return true;
        }
        // AntiSamy anchors its attribute regexps at both ends — a pattern that
        // merely matched *somewhere* in the value would accept
        // `javascript:alert(1)#https://ok.example` for an offsiteURL rule.
        self.regexps.iter().any(|r| full_match(r, value))
    }
}

/// True when `re` matches the WHOLE of `value`. The policy's patterns are
/// written unanchored, and Java's `Matcher.matches()` — which AntiSamy uses —
/// requires a full match, so anchoring here is what preserves their meaning.
fn full_match(re: &Regex, value: &str) -> bool {
    match re.find(value) {
        Some(m) => m.start() == 0 && m.end() == value.len(),
        None => false,
    }
}

#[derive(Clone, Debug)]
pub struct TagRule {
    pub action: TagAction,
    /// Tag-specific attribute rules, keyed lowercase. A tag-level
    /// `<attribute name="href"/>` with no lists of its own means "allow href
    /// here, validated by the common-attributes rule".
    pub attributes: HashMap<String, Option<AttributeRule>>,
}

/// One `<css-rules>` property: which literal keywords and which regexps a
/// declaration's value may match, plus any shorthand it expands into.
#[derive(Clone, Debug, Default)]
pub struct CssProperty {
    pub literals: Vec<String>,
    pub regexps: Vec<Regex>,
    pub shorthands: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub directives: HashMap<String, String>,
    pub common_attributes: HashMap<String, AttributeRule>,
    pub global_attributes: HashSet<String>,
    pub tags: HashMap<String, TagRule>,
    pub css_properties: HashMap<String, CssProperty>,
}

#[derive(Debug)]
pub struct PolicyError(pub String);

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PolicyError {}

impl Policy {
    /// `maxInputSize` from `<directives>`, if the policy sets one. AntiSamy
    /// refuses oversized input rather than truncating it.
    pub fn max_input_size(&self) -> Option<usize> {
        self.directives
            .get("maxinputsize")
            .and_then(|v| v.trim().parse::<usize>().ok())
    }

    /// The rule that governs `attribute` on `tag`: the tag's own rule if it has
    /// one, otherwise the common-attributes default, and `None` when the
    /// attribute is not allowed on this tag at all.
    ///
    /// A tag-level entry that carries its own lists overrides the common one
    /// wholesale (including `onInvalid`); a bare `<attribute name="x"/>` under a
    /// tag defers to the common definition, and is unconstrained if there is
    /// none.
    pub fn attribute_rule(&self, tag: &str, attribute: &str) -> Option<&AttributeRule> {
        static ALLOW_ANY: std::sync::OnceLock<AttributeRule> = std::sync::OnceLock::new();
        let allow_any = || ALLOW_ANY.get_or_init(AttributeRule::default);

        if let Some(tag_rule) = self.tags.get(tag) {
            if let Some(entry) = tag_rule.attributes.get(attribute) {
                return match entry {
                    Some(rule) => Some(rule),
                    None => Some(
                        self.common_attributes
                            .get(attribute)
                            .unwrap_or_else(|| allow_any()),
                    ),
                };
            }
        }
        if self.global_attributes.contains(attribute) {
            return Some(
                self.common_attributes
                    .get(attribute)
                    .unwrap_or_else(|| allow_any()),
            );
        }
        None
    }

    pub fn from_xml_str(xml: &str) -> Result<Policy, PolicyError> {
        parse(xml)
    }

    pub fn from_xml_file(path: &str) -> Result<Policy, PolicyError> {
        let xml = std::fs::read_to_string(path)
            .map_err(|e| PolicyError(format!("cannot read policy file [{}]: {}", path, e)))?;
        parse(&xml)
    }
}

/// Where in the document the reader currently is. The same `<attribute>` and
/// `<regexp>` element names appear under several parents with different
/// meanings, so the parser tracks its section rather than matching on tag names
/// alone.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Section {
    None,
    Directives,
    CommonRegexps,
    CommonAttributes,
    GlobalTagAttributes,
    TagsToEncode,
    TagRules,
    CssRules,
}

/// Compile one policy regexp. AntiSamy's patterns are Java-flavour, but none of
/// the six shipped policies uses a construct the `regex` crate lacks (no
/// lookaround, no backreferences — audited across all six). A pattern that
/// still fails to compile is dropped with its rule left unable to match, which
/// fails CLOSED: the attribute is rejected rather than waved through.
fn compile(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

fn attr_value(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref().eq_ignore_ascii_case(key.as_bytes()) {
            String::from_utf8(a.value.into_owned()).ok()
        } else {
            None
        }
    })
}

fn parse(xml: &str) -> Result<Policy, PolicyError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut policy = Policy::default();
    let mut named_regexps: HashMap<String, Regex> = HashMap::new();

    let mut section = Section::None;
    // The attribute/tag/property currently being filled in. Each is Some only
    // between its start and end event.
    let mut current_attribute: Option<(String, AttributeRule)> = None;
    let mut current_tag: Option<(String, TagRule)> = None;
    let mut current_tag_attribute: Option<(String, AttributeRule, bool)> = None;
    let mut current_property: Option<(String, CssProperty)> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| PolicyError(format!("malformed policy XML: {}", e)))?;
        match event {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                match name.as_str() {
                    "directives" => section = Section::Directives,
                    "common-regexps" => section = Section::CommonRegexps,
                    "common-attributes" => section = Section::CommonAttributes,
                    "global-tag-attributes" => section = Section::GlobalTagAttributes,
                    "tags-to-encode" => section = Section::TagsToEncode,
                    "tag-rules" => section = Section::TagRules,
                    "css-rules" => section = Section::CssRules,

                    "directive" if section == Section::Directives => {
                        if let (Some(n), Some(v)) = (attr_value(e, "name"), attr_value(e, "value")) {
                            policy.directives.insert(n.to_ascii_lowercase(), v);
                        }
                    }

                    "regexp" => match section {
                        Section::CommonRegexps => {
                            if let (Some(n), Some(v)) =
                                (attr_value(e, "name"), attr_value(e, "value"))
                            {
                                if let Some(re) = compile(&v) {
                                    named_regexps.insert(n.to_ascii_lowercase(), re);
                                }
                            }
                        }
                        // Inside an attribute or CSS property: either an inline
                        // `value=` pattern or a `name=` reference to a
                        // common-regexp compiled above.
                        _ => {
                            let resolved = match (attr_value(e, "value"), attr_value(e, "name")) {
                                (Some(v), _) => compile(&v),
                                (None, Some(n)) => named_regexps.get(&n.to_ascii_lowercase()).cloned(),
                                _ => None,
                            };
                            // The rule counts as constrained the moment the
                            // policy declares an entry, even if the pattern
                            // failed to resolve — otherwise an unresolvable
                            // reference would turn the rule into allow-all.
                            if let Some((_, rule, _)) = current_tag_attribute.as_mut() {
                                rule.constrained = true;
                                rule.regexps.extend(resolved);
                            } else if let Some((_, rule)) = current_attribute.as_mut() {
                                rule.constrained = true;
                                rule.regexps.extend(resolved);
                            } else if let Some((_, prop)) = current_property.as_mut() {
                                prop.regexps.extend(resolved);
                            }
                        }
                    },

                    "literal" => {
                        if let Some(v) = attr_value(e, "value") {
                            if let Some((_, rule, _)) = current_tag_attribute.as_mut() {
                                rule.constrained = true;
                                rule.literals.push(v);
                            } else if let Some((_, rule)) = current_attribute.as_mut() {
                                rule.constrained = true;
                                rule.literals.push(v);
                            } else if let Some((_, prop)) = current_property.as_mut() {
                                prop.literals.push(v);
                            }
                        }
                    }

                    "shorthand" => {
                        if let (Some(n), Some((_, prop))) =
                            (attr_value(e, "name"), current_property.as_mut())
                        {
                            prop.shorthands.push(n.to_ascii_lowercase());
                        }
                    }

                    "attribute" => match section {
                        Section::CommonAttributes => {
                            let Some(n) = attr_value(e, "name") else { continue };
                            let rule = AttributeRule {
                                on_invalid: attr_value(e, "onInvalid")
                                    .map(|v| OnInvalid::parse(&v))
                                    .unwrap_or_default(),
                                ..Default::default()
                            };
                            if empty {
                                policy.common_attributes.insert(n.to_ascii_lowercase(), rule);
                            } else {
                                current_attribute = Some((n.to_ascii_lowercase(), rule));
                            }
                        }
                        Section::GlobalTagAttributes => {
                            if let Some(n) = attr_value(e, "name") {
                                policy.global_attributes.insert(n.to_ascii_lowercase());
                            }
                        }
                        Section::TagRules => {
                            let Some(n) = attr_value(e, "name") else { continue };
                            let declared_on_invalid = attr_value(e, "onInvalid");
                            let rule = AttributeRule {
                                on_invalid: declared_on_invalid
                                    .as_deref()
                                    .map(OnInvalid::parse)
                                    .unwrap_or_default(),
                                ..Default::default()
                            };
                            if empty {
                                // `<attribute name="href"/>` — defer to the
                                // common definition unless this one declares its
                                // own onInvalid, which must survive the deferral.
                                let entry = match declared_on_invalid {
                                    Some(_) => Some(rule),
                                    None => None,
                                };
                                if let Some((_, tag)) = current_tag.as_mut() {
                                    tag.attributes.insert(n.to_ascii_lowercase(), entry);
                                }
                            } else {
                                current_tag_attribute =
                                    Some((n.to_ascii_lowercase(), rule, declared_on_invalid.is_some()));
                            }
                        }
                        _ => {}
                    },

                    "tag" if section == Section::TagRules => {
                        let Some(n) = attr_value(e, "name") else { continue };
                        let action = attr_value(e, "action")
                            .map(|a| TagAction::parse(&a))
                            .unwrap_or(TagAction::Validate);
                        let rule = TagRule {
                            action,
                            attributes: HashMap::new(),
                        };
                        if empty {
                            policy.tags.insert(n.to_ascii_lowercase(), rule);
                        } else {
                            current_tag = Some((n.to_ascii_lowercase(), rule));
                        }
                    }

                    "tag" if section == Section::TagsToEncode => {
                        // `<tags-to-encode>` holds bare `<tag>g</tag>` text
                        // nodes, handled by the Text arm below.
                    }

                    "property" if section == Section::CssRules => {
                        let Some(n) = attr_value(e, "name") else { continue };
                        let prop = CssProperty::default();
                        if empty {
                            policy.css_properties.insert(n.to_ascii_lowercase(), prop);
                        } else {
                            current_property = Some((n.to_ascii_lowercase(), prop));
                        }
                    }

                    _ => {}
                }
            }

            Event::Text(ref t) if section == Section::TagsToEncode => {
                let text = String::from_utf8_lossy(t.as_ref()).trim().to_ascii_lowercase();
                if !text.is_empty() {
                    policy.tags.insert(
                        text,
                        TagRule {
                            action: TagAction::Encode,
                            attributes: HashMap::new(),
                        },
                    );
                }
            }

            Event::End(ref e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                match name.as_str() {
                    "directives" | "common-regexps" | "common-attributes"
                    | "global-tag-attributes" | "tags-to-encode" | "tag-rules" | "css-rules" => {
                        section = Section::None;
                    }
                    "attribute" => {
                        if let Some((n, rule, _)) = current_tag_attribute.take() {
                            if let Some((_, tag)) = current_tag.as_mut() {
                                tag.attributes.insert(n, Some(rule));
                            }
                        } else if let Some((n, rule)) = current_attribute.take() {
                            policy.common_attributes.insert(n, rule);
                        }
                    }
                    "tag" => {
                        if let Some((n, rule)) = current_tag.take() {
                            policy.tags.insert(n, rule);
                        }
                    }
                    "property" => {
                        if let Some((n, prop)) = current_property.take() {
                            policy.css_properties.insert(n, prop);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // A tag-level attribute that declared no lists of its own must inherit the
    // common-attributes lists, keeping only its own `onInvalid`. Without this,
    // `<attribute name="href" onInvalid="filterTag"/>` (the tinymce policy's
    // spelling) became an unconstrained rule that accepted `javascript:` URLs.
    // Done as a pass at the end so section order in the file cannot matter.
    let common = policy.common_attributes.clone();
    for tag_rule in policy.tags.values_mut() {
        for (name, entry) in tag_rule.attributes.iter_mut() {
            let Some(rule) = entry.as_mut() else { continue };
            if rule.constrained {
                continue;
            }
            if let Some(inherited) = common.get(name) {
                rule.regexps = inherited.regexps.clone();
                rule.literals = inherited.literals.clone();
                rule.constrained = inherited.constrained;
            }
        }
    }

    if policy.tags.is_empty() {
        return Err(PolicyError(
            "policy defines no <tag-rules> — refusing to treat it as a sanitisation policy"
                .to_string(),
        ));
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
    <anti-samy-rules>
      <directives>
        <directive name="maxInputSize" value="100"/>
      </directives>
      <common-regexps>
        <regexp name="number" value="\d+"/>
      </common-regexps>
      <common-attributes>
        <attribute name="href">
          <regexp-list><regexp value="https?://.*"/></regexp-list>
        </attribute>
      </common-attributes>
      <global-tag-attributes>
        <attribute name="class"/>
      </global-tag-attributes>
      <tags-to-encode><tag>g</tag></tags-to-encode>
      <tag-rules>
        <tag name="script" action="remove"/>
        <tag name="dd" action="truncate"/>
        <tag name="a" action="validate">
          <attribute name="href"/>
          <attribute name="target" onInvalid="remove">
            <literal-list><literal value="_blank"/></literal-list>
          </attribute>
        </tag>
        <tag name="img" action="validate">
          <attribute name="src" onInvalid="removeTag">
            <regexp-list><regexp value="https?://.*"/></regexp-list>
          </attribute>
          <attribute name="width"><regexp-list><regexp name="number"/></regexp-list></attribute>
        </tag>
      </tag-rules>
      <css-rules>
        <property name="color">
          <literal-list><literal value="red"/></literal-list>
        </property>
      </css-rules>
    </anti-samy-rules>"#;

    fn minimal() -> Policy {
        Policy::from_xml_str(MINIMAL).unwrap()
    }

    #[test]
    fn parses_every_section() {
        let p = minimal();
        assert_eq!(p.max_input_size(), Some(100));
        assert_eq!(p.tags.get("script").unwrap().action, TagAction::Remove);
        assert_eq!(p.tags.get("dd").unwrap().action, TagAction::Truncate);
        assert_eq!(p.tags.get("g").unwrap().action, TagAction::Encode);
        assert!(p.global_attributes.contains("class"));
        assert!(p.css_properties.contains_key("color"));
    }

    #[test]
    fn tag_level_bare_attribute_defers_to_the_common_rule() {
        let p = minimal();
        // `<attribute name="href"/>` under <tag name="a"> carries no lists, so
        // the common-attributes href rule (https? only) must govern it.
        let rule = p.attribute_rule("a", "href").unwrap();
        assert!(rule.accepts("https://example.test/x"));
        assert!(!rule.accepts("javascript:alert(1)"));
    }

    #[test]
    fn named_regexp_references_resolve() {
        let p = minimal();
        let rule = p.attribute_rule("img", "width").unwrap();
        assert!(rule.accepts("42"));
        assert!(!rule.accepts("42px"));
    }

    #[test]
    fn on_invalid_is_read_from_the_tag_level_attribute() {
        let p = minimal();
        assert_eq!(
            p.attribute_rule("img", "src").unwrap().on_invalid,
            OnInvalid::RemoveTag
        );
        assert_eq!(
            p.attribute_rule("a", "target").unwrap().on_invalid,
            OnInvalid::RemoveAttribute
        );
    }

    #[test]
    fn global_attributes_apply_to_any_validated_tag() {
        let p = minimal();
        assert!(p.attribute_rule("a", "class").is_some());
        assert!(p.attribute_rule("img", "class").is_some());
        // …but an attribute nobody declared is not allowed anywhere.
        assert!(p.attribute_rule("a", "onclick").is_none());
    }

    #[test]
    fn regexps_must_match_the_whole_value() {
        let p = minimal();
        let rule = p.attribute_rule("a", "href").unwrap();
        // A trailing-fragment attack: the pattern matches a SUBSTRING, so an
        // unanchored match would accept this.
        assert!(!rule.accepts("javascript:alert(1)#https://ok.test"));
    }

    #[test]
    fn literal_lists_compare_case_insensitively() {
        let p = minimal();
        let rule = p.attribute_rule("a", "target").unwrap();
        assert!(rule.accepts("_BLANK"));
        assert!(!rule.accepts("_other"));
    }

    #[test]
    fn a_policy_with_no_tag_rules_is_rejected() {
        assert!(Policy::from_xml_str("<anti-samy-rules></anti-samy-rules>").is_err());
    }
}
