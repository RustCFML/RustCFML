//! Tuckey-compatible urlrewrite.xml parser and rewrite engine.
//!
//! Parses `urlrewrite.xml` files and applies URL rewrite rules to incoming
//! requests in `--serve` mode. Supports regex and wildcard matching, conditions
//! on method/port/headers, and forward/redirect/permanent-redirect actions.

use regex::Regex;
use std::collections::HashMap;

/// Max distinct patterns held by [`cached_rule_regex`].
const REWRITE_REGEX_CACHE_CAP: usize = 1024;

/// Compile `pattern`, memoized process-wide.
///
/// Every rule (and every regex-valued condition) used to be recompiled on
/// EVERY request — a site with 7 rules in `urlrewrite.xml` paid up to 7 regex
/// compilations per hit. On a warm Preside profile that made
/// `apply_rewrite_rules` ~11.7% of all allocation, second only to the VM's
/// own `execute_function_body`.
///
/// Pure memoization: same pattern string in, same `Regex` out. Compile errors
/// stay uncached and keep their existing "warn and skip the rule" behavior.
/// Bounded like the other regex caches in the tree — cleared wholesale past the
/// cap so it can't grow without limit. Rule patterns come from a config file
/// and are effectively a fixed small set, so the cap should never be reached.
fn cached_rule_regex(pattern: &str) -> Result<std::sync::Arc<Regex>, regex::Error> {
    static CACHE: std::sync::OnceLock<
        std::sync::RwLock<HashMap<String, std::sync::Arc<Regex>>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::new()));

    if let Some(re) = cache.read().unwrap_or_else(|e| e.into_inner()).get(pattern) {
        return Ok(std::sync::Arc::clone(re)); // refcount bump, not a recompile
    }
    let re = std::sync::Arc::new(Regex::new(pattern)?);
    let mut w = cache.write().unwrap_or_else(|e| e.into_inner());
    if w.len() >= REWRITE_REGEX_CACHE_CAP {
        w.clear();
    }
    w.insert(pattern.to_string(), std::sync::Arc::clone(&re));
    Ok(re)
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum MatchType {
    Regex,
    Wildcard,
}

#[derive(Debug, Clone)]
pub enum RewriteType {
    Forward,
    Redirect,
    PermanentRedirect,
}

#[derive(Debug, Clone)]
enum ConditionType {
    Method,
    Port,
    Header(String),
    /// The raw request query string (without the leading `?`).
    QueryString,
    /// The client's remote IP address.
    RemoteAddr,
    /// The request URI (the URL path, without scheme/host/query-string). This
    /// is the condition type every Wheels app's `urlrewrite.xml` ships — it
    /// guards the clean-URL rule against rewriting `/index.cfm`, `/images`, etc.
    RequestUri,
    /// A condition type we don't understand. A rule carrying one of these can
    /// never be safely evaluated, so it must NOT match — otherwise dropping the
    /// condition would silently make the rule fire on every request (this is
    /// exactly what produced an infinite 301 loop with Preside's
    /// query-string/remote-addr guard rules).
    Unsupported,
}

#[derive(Debug, Clone)]
enum ConditionOp {
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

#[derive(Debug, Clone)]
struct RewriteCondition {
    cond_type: ConditionType,
    operator: ConditionOp,
    value: String,
    case_sensitive: bool,
    /// `next="or"`: OR this condition's result with the next one instead of
    /// ANDing (tuckey evaluates a rule's conditions left to right).
    next_or: bool,
}

#[derive(Debug, Clone)]
pub struct RewriteRule {
    #[allow(dead_code)]
    name: Option<String>,
    enabled: bool,
    match_type: MatchType,
    case_sensitive: bool,
    from: String,
    to: Option<String>,
    to_type: RewriteType,
    to_last: bool,
    conditions: Vec<RewriteCondition>,
    /// `<set type="status">N</set>`: the response status for a request this
    /// rule matches.
    set_status: Option<u16>,
}

pub struct RewriteResult {
    pub new_path: String,
    pub rewrite_type: RewriteType,
    /// The status set by the last matching rule with `<set type="status">`.
    pub status: Option<u16>,
}

// ---------------------------------------------------------------------------
// XML parser
// ---------------------------------------------------------------------------

/// Parse the contents of a urlrewrite.xml document into a list of rewrite rules.
///
/// Callers should supply content read through the VFS so an embedded
/// `urlrewrite.xml` in a self-contained binary is honoured — reading the real
/// filesystem would look for an absolute path that does not exist on the
/// deployment machine.
pub fn parse_urlrewrite_xml_content(content: &str) -> Vec<RewriteRule> {
    let mut rules = Vec::new();
    let mut pos = 0;
    let bytes = content.as_bytes();

    while pos < bytes.len() {
        // Skip to next '<'
        match content[pos..].find('<') {
            Some(i) => pos += i,
            None => break,
        }

        // Skip XML comments
        if content[pos..].starts_with("<!--") {
            match content[pos..].find("-->") {
                Some(i) => {
                    pos += i + 3;
                    continue;
                }
                None => break,
            }
        }

        // Skip processing instructions
        if content[pos..].starts_with("<?") {
            match content[pos..].find("?>") {
                Some(i) => {
                    pos += i + 2;
                    continue;
                }
                None => break,
            }
        }

        // Look for <rule> opening tag
        if content[pos..].starts_with("<rule") {
            let rule_start = pos;
            // Find closing </rule>
            match content[pos..].find("</rule>") {
                Some(i) => {
                    let rule_end = pos + i + 7;
                    let rule_block = &content[rule_start..rule_end];
                    if let Some(rule) = parse_rule_block(rule_block) {
                        rules.push(rule);
                    }
                    pos = rule_end;
                }
                None => {
                    pos += 1;
                }
            }
        } else {
            pos += 1;
        }
    }

    rules
}

/// Parse a single <rule>...</rule> block.
fn parse_rule_block(block: &str) -> Option<RewriteRule> {
    // Extract <rule> tag attributes
    let rule_tag_end = block.find('>')?;
    let rule_tag = &block[..rule_tag_end + 1];
    let enabled = get_xml_attr(rule_tag, "enabled").map_or(true, |v| v != "false");
    let match_type = match get_xml_attr(rule_tag, "match-type").as_deref() {
        Some("wildcard") => MatchType::Wildcard,
        _ => MatchType::Regex,
    };

    let name = extract_element_text(block, "name");
    let from = match extract_element_text(block, "from") {
        Some(f) => f,
        None => return None, // <from> is required
    };

    // Parse <to> element with attributes
    let (to_text, to_type, to_last, case_sensitive) = parse_to_element(block);

    // Parse <condition> elements
    let conditions = parse_conditions(block);
    for c in &conditions {
        if let ConditionType::Unsupported = c.cond_type {
            log::warn!(
                "urlrewrite: rule {} has a condition type this engine does not support; the rule will never match",
                name.as_deref().map(|n| format!("'{}'", n)).unwrap_or_else(|| format!("from '{}'", from))
            );
        }
    }

    let set_status = parse_set_status(block);

    Some(RewriteRule {
        name,
        enabled,
        match_type,
        case_sensitive,
        from,
        to: to_text,
        to_type,
        to_last,
        conditions,
        set_status,
    })
}

/// The status of a `<set type="status">N</set>` element, if the rule has one.
/// Other `<set>` types (cookie, request attributes, …) are not modelled.
fn parse_set_status(block: &str) -> Option<u16> {
    let mut from = 0;
    while let Some(i) = block[from..].find("<set") {
        let start = from + i;
        let tag_end = start + block[start..].find('>')?;
        let tag = &block[start..=tag_end];
        from = tag_end + 1;
        if get_xml_attr(tag, "type").as_deref() != Some("status") || tag.ends_with("/>") {
            continue;
        }
        let close = block[from..].find("</set>")?;
        if let Ok(n) = block[from..from + close].trim().parse::<u16>() {
            return Some(n);
        }
    }
    None
}

/// Extract an XML attribute value from an opening tag string.
fn get_xml_attr(tag: &str, attr_name: &str) -> Option<String> {
    let pattern_dq = format!("{}=\"", attr_name);
    let pattern_sq = format!("{}='", attr_name);

    if let Some(start) = tag.find(&pattern_dq) {
        let value_start = start + pattern_dq.len();
        if let Some(end) = tag[value_start..].find('"') {
            return Some(tag[value_start..value_start + end].to_string());
        }
    } else if let Some(start) = tag.find(&pattern_sq) {
        let value_start = start + pattern_sq.len();
        if let Some(end) = tag[value_start..].find('\'') {
            return Some(tag[value_start..value_start + end].to_string());
        }
    }
    None
}

/// Extract text content of a simple XML element like `<name>text</name>`.
fn extract_element_text(block: &str, element: &str) -> Option<String> {
    let open = format!("<{}", element);
    let close = format!("</{}>", element);

    let start = block.find(&open)?;
    let tag_end = block[start..].find('>')? + start + 1;
    let end = block[tag_end..].find(&close)? + tag_end;
    Some(block[tag_end..end].trim().to_string())
}

/// Parse the `<to>` element, returning (text, type, last, casesensitive).
fn parse_to_element(block: &str) -> (Option<String>, RewriteType, bool, bool) {
    let open = "<to";
    let close = "</to>";

    let start = match block.find(open) {
        Some(s) => s,
        None => return (None, RewriteType::Forward, true, false),
    };

    let tag_end = match block[start..].find('>') {
        Some(e) => start + e,
        None => return (None, RewriteType::Forward, true, false),
    };
    let tag = &block[start..tag_end + 1];

    let is_self_closing = tag.ends_with("/>");

    let to_type = match get_xml_attr(tag, "type").as_deref() {
        Some("redirect") => RewriteType::Redirect,
        Some("permanent-redirect") => RewriteType::PermanentRedirect,
        Some("temporary-redirect") => RewriteType::Redirect,
        _ => RewriteType::Forward,
    };

    let to_last = get_xml_attr(tag, "last").map_or(true, |v| v != "false");
    let case_sensitive = get_xml_attr(tag, "casesensitive").map_or(false, |v| v == "true");

    if is_self_closing {
        return (None, to_type, to_last, case_sensitive);
    }

    let text_start = tag_end + 1;
    let text_end = match block[text_start..].find(close) {
        Some(e) => text_start + e,
        None => return (None, to_type, to_last, case_sensitive),
    };

    let text = block[text_start..text_end].trim().to_string();
    let text = if text.is_empty() { None } else { Some(text) };

    (text, to_type, to_last, case_sensitive)
}

/// Parse all `<condition>` elements in a rule block.
fn parse_conditions(block: &str) -> Vec<RewriteCondition> {
    let mut conditions = Vec::new();
    let mut search_from = 0;

    while let Some(start) = block[search_from..].find("<condition") {
        let abs_start = search_from + start;
        let tag_end = match block[abs_start..].find('>') {
            Some(e) => abs_start + e,
            None => break,
        };
        let tag = &block[abs_start..tag_end + 1];

        // tuckey defaults a condition with no `type` to `header`.
        let cond_type = match get_xml_attr(tag, "type").as_deref() {
            Some("method") => ConditionType::Method,
            Some("port") => ConditionType::Port,
            Some("header") | None => {
                let header_name = get_xml_attr(tag, "name").unwrap_or_default();
                ConditionType::Header(header_name)
            }
            Some("query-string") => ConditionType::QueryString,
            Some("remote-addr") => ConditionType::RemoteAddr,
            Some("request-uri") => ConditionType::RequestUri,
            _ => ConditionType::Unsupported,
        };

        let operator = match get_xml_attr(tag, "operator").as_deref() {
            Some("equal") => ConditionOp::Equal,
            Some("notequal") => ConditionOp::NotEqual,
            Some("greater") => ConditionOp::Greater,
            Some("less") => ConditionOp::Less,
            Some("greaterorequal") => ConditionOp::GreaterOrEqual,
            Some("lessorequal") => ConditionOp::LessOrEqual,
            _ => ConditionOp::Equal,
        };

        let case_sensitive =
            get_xml_attr(tag, "casesensitive").map_or(false, |v| v == "true");
        let next_or = get_xml_attr(tag, "next").map_or(false, |v| v.eq_ignore_ascii_case("or"));

        let is_self_closing = tag.ends_with("/>");
        let value = if is_self_closing {
            get_xml_attr(tag, "value").unwrap_or_default()
        } else {
            match block[tag_end + 1..].find("</condition>") {
                Some(e) => block[tag_end + 1..tag_end + 1 + e].trim().to_string(),
                None => String::new(),
            }
        };

        conditions.push(RewriteCondition {
            cond_type,
            operator,
            value,
            case_sensitive,
            next_or,
        });

        search_from = tag_end + 1;
    }

    conditions
}

// ---------------------------------------------------------------------------
// Wildcard-to-regex converter
// ---------------------------------------------------------------------------

/// Convert a wildcard pattern to a regex string.
/// `*` matches a single path segment, `**` matches across segments.
fn wildcard_to_regex(pattern: &str) -> String {
    let mut result = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            result.push_str("(.*)");
            i += 2;
        } else if chars[i] == '*' {
            result.push_str("([^/]*)");
            i += 1;
        } else {
            let c = chars[i];
            if ".+?^${}()|[]\\".contains(c) {
                result.push('\\');
            }
            result.push(c);
            i += 1;
        }
    }

    result.push('$');
    result
}

// ---------------------------------------------------------------------------
// Condition evaluator
// ---------------------------------------------------------------------------

fn check_condition(
    cond: &RewriteCondition,
    method: &str,
    port: u16,
    headers: &HashMap<String, String>,
    query_string: &str,
    remote_addr: &str,
    request_uri: &str,
) -> bool {
    let actual: String = match &cond.cond_type {
        // A port compares numerically, so `greater 9000` is not a string
        // comparison ("9000" > "10000" lexicographically).
        ConditionType::Port => {
            let want = match cond.value.trim().parse::<i64>() {
                Ok(n) => n,
                Err(_) => return false,
            };
            let have = port as i64;
            return match cond.operator {
                ConditionOp::Equal => have == want,
                ConditionOp::NotEqual => have != want,
                ConditionOp::Greater => have > want,
                ConditionOp::Less => have < want,
                ConditionOp::GreaterOrEqual => have >= want,
                ConditionOp::LessOrEqual => have <= want,
            };
        }
        ConditionType::Method => method.to_string(),
        ConditionType::Header(name) => headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default(),
        ConditionType::QueryString => query_string.to_string(),
        ConditionType::RemoteAddr => remote_addr.to_string(),
        ConditionType::RequestUri => request_uri.to_string(),
        // Never match — see ConditionType::Unsupported.
        ConditionType::Unsupported => return false,
    };

    // Every other type follows tuckey: the value is a regular expression
    // matched against the actual value. `equal` means the pattern is found,
    // `notequal` that it is not. (header and method used to compare as plain
    // strings, so `.+` never matched a header value.)
    let pattern = if cond.case_sensitive {
        cached_rule_regex(&cond.value)
    } else {
        cached_rule_regex(&format!("(?i){}", cond.value))
    };
    let found = pattern.map(|re| re.is_match(&actual)).unwrap_or(false);
    match cond.operator {
        ConditionOp::NotEqual => !found,
        // equal (and any ordering operator, which tuckey does not define for
        // regex conditions) means "the pattern matched".
        _ => found,
    }
}

/// Combine a rule's conditions as tuckey does: left to right, each result
/// ANDed into the running total, or ORed when the PREVIOUS condition carried
/// `next="or"`. No conditions means the rule applies.
fn conditions_pass(
    conditions: &[RewriteCondition],
    method: &str,
    port: u16,
    headers: &HashMap<String, String>,
    query_string: &str,
    remote_addr: &str,
    request_uri: &str,
) -> bool {
    let mut passing = true;
    let mut or_with_next = false;
    for (i, c) in conditions.iter().enumerate() {
        let hit = check_condition(c, method, port, headers, query_string, remote_addr, request_uri);
        passing = if i == 0 {
            hit
        } else if or_with_next {
            passing || hit
        } else {
            passing && hit
        };
        or_with_next = c.next_or;
    }
    passing
}

// ---------------------------------------------------------------------------
// Rewrite engine
// ---------------------------------------------------------------------------

/// Apply rewrite rules to a URL path. Returns a `RewriteResult` if any rule matched.
pub fn apply_rewrite_rules(
    rules: &[RewriteRule],
    url_path: &str,
    method: &str,
    port: u16,
    headers: &HashMap<String, String>,
    query_string: &str,
    remote_addr: &str,
) -> Option<RewriteResult> {
    let mut current_path = url_path.to_string();
    let mut last_result: Option<RewriteResult> = None;
    let mut status: Option<u16> = None;

    for rule in rules {
        if !rule.enabled {
            continue;
        }

        if !conditions_pass(
            &rule.conditions,
            method,
            port,
            headers,
            query_string,
            remote_addr,
            &current_path,
        ) {
            continue;
        }

        // Build regex pattern
        let pattern_str = match rule.match_type {
            MatchType::Wildcard => wildcard_to_regex(&rule.from),
            MatchType::Regex => rule.from.clone(),
        };

        let regex = if rule.case_sensitive {
            cached_rule_regex(&pattern_str)
        } else {
            cached_rule_regex(&format!("(?i){}", pattern_str))
        };

        let regex = match regex {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "Warning: Invalid rewrite pattern '{}': {}",
                    rule.from, e
                );
                continue;
            }
        };

        if let Some(captures) = regex.captures(&current_path) {
            if rule.set_status.is_some() {
                status = rule.set_status;
            }
            if let Some(ref to) = rule.to {
                // Substitute backreferences $1, $2, etc.
                let mut new_path = to.clone();
                for i in 1..captures.len() {
                    if let Some(m) = captures.get(i) {
                        new_path = new_path.replace(&format!("${}", i), m.as_str());
                    }
                }
                // Tuckey built-in variable: `%{context-path}` is the servlet
                // context path. A root-deployed CFML app (the only deployment
                // model here) has an empty context path, so the token resolves
                // to "". Without this, Preside's `%{context-path}/index.cfm`
                // rewrite produced a literal, unservable path.
                new_path = new_path.replace("%{context-path}", "");

                last_result = Some(RewriteResult {
                    new_path: new_path.clone(),
                    rewrite_type: rule.to_type.clone(),
                    status: None,
                });
                current_path = new_path;
            } else {
                // No <to> — matched but pass-through
                last_result = Some(RewriteResult {
                    new_path: current_path.clone(),
                    rewrite_type: RewriteType::Forward,
                    status: None,
                });
            }

            if rule.to_last {
                break;
            }
        }
    }

    if let Some(r) = last_result.as_mut() {
        r.status = status;
    }
    last_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_from_content() {
        // Content is parsed straight from a string (as a VFS read would
        // supply), not the real filesystem.
        let xml = r#"<?xml version="1.0"?>
            <urlrewrite>
                <rule>
                    <from>^/foo/(.*)$</from>
                    <to>/index.cfm/$1</to>
                </rule>
            </urlrewrite>"#;
        let rules = parse_urlrewrite_xml_content(xml);
        assert_eq!(rules.len(), 1);

        let headers = HashMap::new();
        let result =
            apply_rewrite_rules(&rules, "/foo/bar", "GET", 8500, &headers, "", "127.0.0.1");
        let result = result.expect("rule should match");
        assert_eq!(result.new_path, "/index.cfm/bar");
    }

    // Regression for GH #194: a rule with a `request-uri` condition (the shape
    // every Wheels app's urlrewrite.xml ships) must fire for a clean URL and be
    // skipped for the excluded paths. Before the `request-uri` type was
    // implemented it fell to `Unsupported` and the rule never matched (clean
    // URLs 404'd from v0.227.0).
    #[test]
    fn request_uri_condition_gates_clean_url_rewrite() {
        let xml = r#"<urlrewrite>
            <rule enabled="true">
                <condition type="request-uri" operator="notequal">^/(index.cfm|images|files)</condition>
                <from>^/(.*)$</from>
                <to type="passthrough">/index.cfm/$1</to>
            </rule>
        </urlrewrite>"#;
        let rules = parse_urlrewrite_xml_content(xml);
        assert_eq!(rules.len(), 1);
        let headers = HashMap::new();

        // Clean URL: request-uri does NOT match the exclusion → condition passes → rewrites.
        let hit = apply_rewrite_rules(&rules, "/hello", "GET", 8500, &headers, "", "127.0.0.1")
            .expect("clean URL should rewrite");
        assert_eq!(hit.new_path, "/index.cfm/hello");

        // Already-rewritten path: request-uri matches the exclusion → notequal
        // fails → condition fails → rule skipped, no rewrite.
        let miss =
            apply_rewrite_rules(&rules, "/index.cfm/posts", "GET", 8500, &headers, "", "127.0.0.1");
        assert!(miss.is_none(), "excluded path must not be rewritten");
    }

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn rules(body: &str) -> Vec<RewriteRule> {
        parse_urlrewrite_xml_content(&format!("<urlrewrite>{}</urlrewrite>", body))
    }

    fn hit(rules: &[RewriteRule], path: &str, method: &str, port: u16, h: &HashMap<String, String>, addr: &str) -> Option<RewriteResult> {
        apply_rewrite_rules(rules, path, method, port, h, "", addr)
    }

    // GH #456: header and method values are regular expressions, as on tuckey.
    #[test]
    fn header_condition_is_a_regex() {
        let r = rules(r#"<rule>
            <condition type="header" name="X-Forwarded-For" operator="equal">.+</condition>
            <from>^/internal/.*$</from>
            <set type="status">404</set>
            <to last="true">/404.html</to>
        </rule>"#);
        let via_proxy = hit(&r, "/internal/x", "GET", 8500, &headers(&[("x-forwarded-for", "203.0.113.9")]), "10.0.0.1")
            .expect("header present: rule fires");
        assert_eq!(via_proxy.status, Some(404));
        assert_eq!(via_proxy.new_path, "/404.html");
        assert!(hit(&r, "/internal/x", "GET", 8500, &headers(&[]), "10.0.0.1").is_none(), "header absent: rule skipped");

        let r = rules(r#"<rule>
            <condition type="header" name="X-Forwarded-For">^10\.</condition>
            <from>^/a$</from><to>/b</to>
        </rule>"#);
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[("X-Forwarded-For", "10.1.2.3")]), "").is_some());
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[("X-Forwarded-For", "110.1.2.3")]), "").is_none());
    }

    #[test]
    fn method_condition_is_a_regex() {
        let r = rules(r#"<rule>
            <condition type="method">^(POST|PUT)$</condition>
            <from>^/a$</from><to>/b</to>
        </rule>"#);
        assert!(hit(&r, "/a", "PUT", 8500, &headers(&[]), "").is_some());
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[]), "").is_none());
    }

    #[test]
    fn port_compares_numerically() {
        let r = rules(r#"<rule>
            <condition type="port" operator="greater">9000</condition>
            <from>^/a$</from><to>/b</to>
        </rule>"#);
        assert!(hit(&r, "/a", "GET", 10000, &headers(&[]), "").is_some(), "10000 > 9000");
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[]), "").is_none());
    }

    #[test]
    fn missing_type_is_header() {
        let r = rules(r#"<rule>
            <condition name="X-Probe">yes</condition>
            <from>^/a$</from><to>/b</to>
        </rule>"#);
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[("X-Probe", "yes")]), "").is_some());
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[]), "").is_none());
    }

    #[test]
    fn next_or_combines_left_to_right() {
        // Block /admin unless BOTH the client address and Host are local.
        let r = rules(r#"<rule>
            <condition type="remote-addr" operator="notequal" next="or">^127\.0\.0\.1$</condition>
            <condition type="header" name="host" operator="notequal">^127\.0\.0\.1</condition>
            <from>^/admin.*$</from>
            <set type="status">404</set>
            <to last="true">/404.html</to>
        </rule>"#);
        let local = headers(&[("host", "127.0.0.1:8500")]);
        let spoofed = headers(&[("host", "127.0.0.1")]);
        let public = headers(&[("host", "example.com")]);
        assert!(hit(&r, "/admin", "GET", 8500, &local, "127.0.0.1").is_none(), "local client, local host: allowed");
        assert!(hit(&r, "/admin", "GET", 8500, &spoofed, "203.0.113.9").is_some(), "remote client spoofing Host: refused");
        assert!(hit(&r, "/admin", "GET", 8500, &public, "127.0.0.1").is_some(), "public host: refused");
    }

    #[test]
    fn unknown_condition_type_never_matches() {
        let r = rules(r#"<rule>
            <condition type="session-attribute" name="x">.*</condition>
            <from>^/a$</from><to>/b</to>
        </rule>"#);
        assert!(hit(&r, "/a", "GET", 8500, &headers(&[]), "").is_none());
    }

    #[test]
    fn set_status_without_to_passes_through() {
        let r = rules(r#"<rule><from>^/gone$</from><set type="status">410</set></rule>"#);
        let res = hit(&r, "/gone", "GET", 8500, &headers(&[]), "").expect("matches");
        assert_eq!(res.status, Some(410));
        assert_eq!(res.new_path, "/gone");
    }
}
