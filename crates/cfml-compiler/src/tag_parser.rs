//! CFML Tag Parser - Converts CFML tag syntax to script syntax
//!
//! This module preprocesses CFML tag-based code into equivalent CFScript code,
//! allowing the existing script parser to handle everything uniformly.
//!
//! Supported tags:
//! - <cfset variable = value>
//! - <cfoutput>...</cfoutput>
//! - <cfif condition>...</cfif>
//! - <cfelseif condition>
//! - <cfelse>
//! - <cfloop> (index, condition, array, list, query)
//! - <cfscript>...</cfscript>
//! - <cffunction name="..." ...>...</cffunction>
//! - <cfargument name="..." ...>
//! - <cfreturn expression>
//! - <cfinclude template="path">
//! - <cfdump var="#expression#">
//! - <cfthrow message="...">
//! - <cftry>...</cftry>
//! - <cfcatch type="...">...</cfcatch>
//! - <cfabort>
//! - <cfparam name="..." default="...">
//! - <cfcomponent>...</cfcomponent>
//! - <cfproperty name="..." ...>
//! - <cfhttp url="..." method="..." result="...">
//! - <cfquery name="..." datasource="...">SQL</cfquery>
//! - <cfheader statuscode="..." statustext="..." name="..." value="...">
//! - <cfcontent reset="..." type="..." variable="...">
//! - <cflocation url="..." statuscode="..." addtoken="...">
//! - <cfdirectory action="..." directory="..." name="..." filter="..." recurse="...">
//! - <cfinvoke component="..." method="..." returnvariable="...">

use std::cell::RefCell;
use std::collections::HashMap;

// TLD cache: prefix → (tag-name → file-name) parsed from .tld files.
thread_local! {
    static TLD_CACHE: RefCell<HashMap<String, HashMap<String, String>>> = RefCell::new(HashMap::new());
}

/// Parse a .tld file and return tag-name → file-name mapping.
fn parse_tld_file(tld_path: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let content = match std::fs::read_to_string(tld_path) {
        Ok(c) => c,
        Err(_) => return map,
    };
    // Best-effort parsing: find <tag><name>foo</name></tag> blocks
    // and optional <tag-class> or <tag-file> elements
    let lower = content.to_lowercase();
    let bytes = lower.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if let Some(tag_start) = lower[pos..].find("<tag>") {
            let abs_start = pos + tag_start;
            if let Some(tag_end) = lower[abs_start..].find("</tag>") {
                let block = &content[abs_start..abs_start + tag_end + 6];
                let block_lower = block.to_lowercase();
                // Extract <name>
                let name = extract_tld_element(&block_lower, block, "name");
                if let Some(tag_name) = name {
                    // Extract optional <tag-file>
                    let file = extract_tld_element(&block_lower, block, "tag-file")
                        .unwrap_or_else(|| format!("{}.cfm", tag_name));
                    map.insert(tag_name.to_lowercase(), file);
                }
                pos = abs_start + tag_end + 6;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    map
}

/// Extract text content of a simple XML element from a TLD block.
fn extract_tld_element(block_lower: &str, block_orig: &str, element: &str) -> Option<String> {
    let open = format!("<{}>", element);
    let close = format!("</{}>", element);
    if let Some(start) = block_lower.find(&open) {
        if let Some(end) = block_lower[start..].find(&close) {
            let text = &block_orig[start + open.len()..start + end];
            return Some(text.trim().to_string());
        }
    }
    None
}

/// Check if source contains CFML tags or CFML comments
pub fn has_cfml_tags(source: &str) -> bool {
    let lower = source.to_lowercase();
    lower.contains("<cf") || lower.contains("</cf") || source.contains("<!---")
}

/// Convert CFML tag-based source code to equivalent CFScript
pub fn tags_to_script(source: &str) -> String {
    let mut imports = std::collections::HashMap::<String, String>::new();
    tags_to_script_impl(source, &mut imports)
}

/// Internal implementation that threads cfimport prefix→taglib mappings through.
fn tags_to_script_impl(source: &str, imports: &mut std::collections::HashMap<String, String>) -> String {
    tags_to_script_inner(source, imports, false)
}

/// Inner implementation with cfoutput tracking for enableCFOutputOnly support.
/// When `in_cfoutput` is true, text and hash expressions use writeOutput() directly.
/// When false, they use __writeText() which the VM can suppress.
fn tags_to_script_inner(source: &str, imports: &mut std::collections::HashMap<String, String>, in_cfoutput: bool) -> String {
    let mut result = String::new();
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Strip CFML comments: <!--- ... --->
        if i + 4 < len && chars[i] == '<' && chars[i + 1] == '!' && chars[i + 2] == '-' && chars[i + 3] == '-' && chars[i + 4] == '-' {
            // Find closing --->
            let mut j = i + 5;
            while j + 2 < len {
                if chars[j] == '-' && chars[j + 1] == '-' && chars[j + 2] == '>' {
                    j += 3;
                    break;
                }
                j += 1;
            }
            if j + 2 >= len && !(j >= 3 && chars[j - 1] == '>' && chars[j - 2] == '-' && chars[j - 3] == '-') {
                j = len; // unclosed comment, skip to end
            }
            i = j;
            continue;
        }
        if i < len - 1 && chars[i] == '<' && is_cf_tag_start(&chars, i, len) {
            let (script, consumed) = parse_cf_tag(&chars, i, len, imports);
            result.push_str(&script);
            i += consumed;
        } else if !imports.is_empty() && chars[i] == '<' && is_import_tag_start(&chars, i, len, imports) {
            let (script, consumed) = parse_import_tag(&chars, i, len, imports);
            result.push_str(&script);
            i += consumed;
        } else if in_cfoutput && chars[i] == '#' && i + 1 < len && chars[i + 1] != '#' {
            // Hash expression inside <cfoutput> text: #expr# -> writeOutput(expr).
            // OUTSIDE <cfoutput>, `#` is literal text in CFML — falls through to
            // the plain-text branch below (e.g. `<cfinclude template="x.css">`
            // outputs CSS verbatim, including hex colors like `#f3f4f6;`).
            if let Some(end) = find_closing_hash(&chars, i + 1, len) {
                let expr: String = chars[i + 1..end].iter().collect();
                result.push_str(&format!("writeOutput({});", expr));
                i = end + 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        } else if in_cfoutput && chars[i] == '#' && i + 1 < len && chars[i + 1] == '#' {
            // Escaped hash ## inside cfoutput -> literal #
            result.push_str("writeOutput(\"##\");");
            i += 2;
        } else {
            // Plain text - collect until we hit a tag, (cfoutput-scoped) hash
            // expression, or CFML comment. Hash characters are part of plain text
            // outside <cfoutput>.
            let start = i;
            while i < len && !(chars[i] == '<' && is_cf_tag_start(&chars, i, len))
                && !(chars[i] == '<' && !imports.is_empty() && is_import_tag_start(&chars, i, len, imports))
                && !(in_cfoutput && chars[i] == '#' && i + 1 < len)
                && !(i + 4 < len && chars[i] == '<' && chars[i + 1] == '!' && chars[i + 2] == '-' && chars[i + 3] == '-' && chars[i + 4] == '-')
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            if !text.is_empty() {
                // Output ALL text including whitespace-only nodes.
                // Standard CFML outputs everything; whitespace suppression is opt-in
                // via cfprocessingdirective or cfsetting enableCFOutputOnly.
                // Outside <cfoutput>: use __writeText (suppressible by enableCFOutputOnly).
                let fn_name = if in_cfoutput { "writeOutput" } else { "__writeText" };
                let mut escaped = text.replace('\\', "\\\\").replace('"', "\"\"").replace('\n', "\\n").replace('\r', "\\r");
                // Outside <cfoutput>, `#` is literal in template text — but the
                // lexer interpolates `#expr#` inside string literals. Double the
                // hashes to CFML's literal-hash escape so e.g. `#f3f4f6;` from a
                // CSS-via-cfinclude survives untouched.
                if !in_cfoutput {
                    escaped = escaped.replace('#', "##");
                }
                result.push_str(&format!("{}(\"{}\");", fn_name, escaped));
            }
        }
    }

    result
}

/// Check if chars at pos start an import prefix tag: <prefix:tagname> or </prefix:tagname>
fn is_import_tag_start(chars: &[char], pos: usize, len: usize, imports: &std::collections::HashMap<String, String>) -> bool {
    let name_start = if pos + 1 < len && chars[pos + 1] == '/' { pos + 2 } else { pos + 1 };
    // Read prefix (alphanumeric until :)
    let mut end = name_start;
    while end < len && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if end >= len || chars[end] != ':' || end == name_start {
        return false;
    }
    // Check there's a tag name after the colon
    if end + 1 >= len || !chars[end + 1].is_alphabetic() {
        return false;
    }
    let prefix: String = chars[name_start..end].iter().collect();
    imports.contains_key(&prefix.to_lowercase())
}

/// Parse an import prefix tag: <prefix:tagname attrs> or </prefix:tagname>
fn parse_import_tag(chars: &[char], start: usize, len: usize, imports: &mut std::collections::HashMap<String, String>) -> (String, usize) {
    let is_closing = chars.get(start + 1) == Some(&'/');
    let name_start = if is_closing { start + 2 } else { start + 1 };

    // Read prefix
    let mut colon_pos = name_start;
    while colon_pos < len && chars[colon_pos] != ':' { colon_pos += 1; }
    let prefix: String = chars[name_start..colon_pos].iter().collect();

    // Read tag name after colon
    let tag_start = colon_pos + 1;
    let mut tag_name_end = tag_start;
    while tag_name_end < len && (chars[tag_name_end].is_alphanumeric() || chars[tag_name_end] == '_') {
        tag_name_end += 1;
    }
    let tag_name: String = chars[tag_start..tag_name_end].iter().collect();

    // For closing tags, just consume and return empty (opening tag handler manages execution)
    if is_closing {
        let close_end = find_tag_end(chars, tag_name_end, len);
        return (String::new(), close_end - start);
    }

    // Parse attributes
    let (attrs, quoted, tag_end) = parse_tag_attributes(chars, tag_name_end, len);

    // Look up taglib path, consulting TLD cache for file name overrides
    let prefix_lower = prefix.to_lowercase();
    let taglib = imports.get(&prefix_lower).cloned().unwrap_or_default();
    let tag_file = TLD_CACHE.with(|cache| {
        let c = cache.borrow();
        c.get(&prefix_lower)
            .and_then(|tld| tld.get(&tag_name.to_lowercase()))
            .cloned()
    }).unwrap_or_else(|| format!("{}.cfm", tag_name.to_lowercase()));
    let path = format!("{}/{}", taglib.trim_end_matches('/'), tag_file);
    let path_expr = format!("\"{}\"", path.replace('"', "\\\""));

    // Build attributes struct
    let mut attr_parts = Vec::new();
    for (k, v) in &attrs {
        attr_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k))));
    }
    let attrs_expr = format!("{{ {} }}", attr_parts.join(", "));

    // Check for body (closing </prefix:tagname>)
    let full_tag = format!("{}:{}", prefix, tag_name);
    if let Some(body_start) = find_closing_tag(chars, tag_end, len, &full_tag) {
        let body_chars = &chars[tag_end..body_start];
        let body_source: String = body_chars.iter().collect();
        let body_script = tags_to_script_impl(&body_source, imports);
        let close_end = find_tag_end(chars, body_start, len);
        let result = format!(
            "__cfcustomtag_start({}, {});\n{}\n__cfcustomtag_end();\n",
            path_expr, attrs_expr, body_script
        );
        (result, close_end - start)
    } else {
        // Self-closing
        let result = format!("__cfcustomtag({}, {});\n", path_expr, attrs_expr);
        (result, tag_end - start)
    }
}

fn is_cf_tag_start(chars: &[char], pos: usize, len: usize) -> bool {
    if pos + 3 >= len {
        return false;
    }
    let next_two: String = chars[pos + 1..pos + 3].iter().collect();
    let next_lower = next_two.to_lowercase();
    next_lower == "cf" || (chars[pos + 1] == '/' && pos + 4 < len && {
        let after_slash: String = chars[pos + 2..pos + 4].iter().collect();
        after_slash.to_lowercase() == "cf"
    })
}

fn find_closing_hash(chars: &[char], start: usize, len: usize) -> Option<usize> {
    let mut i = start;
    let mut depth = 0;
    while i < len {
        if chars[i] == '#' && depth == 0 {
            return Some(i);
        }
        if chars[i] == '(' {
            depth += 1;
        }
        if chars[i] == ')' && depth > 0 {
            depth -= 1;
        }
        i += 1;
    }
    None
}

fn parse_cf_tag(chars: &[char], start: usize, len: usize, imports: &mut std::collections::HashMap<String, String>) -> (String, usize) {
    // Determine if closing tag
    let is_closing = chars.get(start + 1) == Some(&'/');

    // Extract tag name
    let name_start = if is_closing { start + 2 } else { start + 1 };
    let mut name_end = name_start;
    while name_end < len && (chars[name_end].is_alphanumeric() || chars[name_end] == '_') {
        name_end += 1;
    }
    let tag_name: String = chars[name_start..name_end].iter().collect();
    let tag_lower = tag_name.to_lowercase();

    // For closing tags, just skip them (the opening tag handler manages scope)
    if is_closing {
        let close_end = find_tag_end(chars, name_end, len);
        // Return empty and consumed count
        match tag_lower.as_str() {
            "cfif" => return ("}\n".to_string(), close_end - start),
            "cfloop" => return ("}\n".to_string(), close_end - start),
            "cfoutput" => return (String::new(), close_end - start),
            "cffunction" => return ("}\n".to_string(), close_end - start),
            "cfcomponent" => return ("}\n".to_string(), close_end - start),
            "cfinterface" => return ("}\n".to_string(), close_end - start),
            "cftry" => return (String::new(), close_end - start), // try block closed by catch
            "cfcatch" => return ("}\n".to_string(), close_end - start),
            "cffinally" => return ("}\n".to_string(), close_end - start),
            "cfscript" => return (String::new(), close_end - start),
            "cfsavecontent" => return (String::new(), close_end - start),
            "cftransaction" => return (String::new(), close_end - start),
            "cfwhile" => return ("}\n".to_string(), close_end - start),
            "cfsilent" => return (String::new(), close_end - start),
            "cflock" => return (String::new(), close_end - start),
            "cfswitch" => return (String::new(), close_end - start),
            _ => return (String::new(), close_end - start),
        }
    }

    // Tags with freeform expression bodies (not key=value attributes) —
    // use find_tag_end directly to avoid misparsing expressions containing quotes/equals
    match tag_lower.as_str() {
        "cfset" | "cfif" | "cfelseif" | "cfreturn" => {
            let tag_end = find_tag_end(chars, name_end, len);
            let raw: String = chars[name_end..tag_end - 1].iter().collect();
            let body = raw.trim();
            let body = if body.ends_with('/') {
                body[..body.len() - 1].trim()
            } else {
                body
            };
            let body = strip_hashes(body);
            // CFML tags don't use backslash escaping, but the script parser does.
            // Escape backslashes in string literals so they survive script parsing.
            let body = escape_backslashes_in_tag_strings(&body);
            let result = match tag_lower.as_str() {
                "cfset" => format!("{};\n", body),
                "cfif" => format!("if ({}) {{\n", body),
                "cfelseif" => format!("}} else if ({}) {{\n", body),
                "cfreturn" => format!("return {};\n", body),
                _ => unreachable!(),
            };
            return (result, tag_end - start);
        }
        _ => {}
    }

    // Parse attributes for all other tags
    let (attrs, quoted, tag_end) = parse_tag_attributes(chars, name_end, len);

    match tag_lower.as_str() {
        "cfoutput" => {
            // <cfoutput> marks a region where # expressions are evaluated and
            // content is always output (even when enableCFOutputOnly is active).
            // Process body recursively with in_cfoutput=true so text uses writeOutput.
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfoutput") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_script = tags_to_script_inner(&body, imports, true);
                (body_script, close_end - start)
            } else {
                // No closing tag — treat remaining content as inside cfoutput
                let body: String = chars[tag_end..len].iter().collect();
                let body_script = tags_to_script_inner(&body, imports, true);
                (body_script, len - start)
            }
        }
        "cfelse" => {
            ("} else {\n".to_string(), tag_end - start)
        }
        "cfloop" => {
            parse_cfloop_tag(&attrs, tag_end - start)
        }
        "cfscript" => {
            // Everything between <cfscript> and </cfscript> is raw script
            // Find the closing </cfscript>
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfscript") {
                let script: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                (script, close_end - start)
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cffunction" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            let access = attrs.get("access").cloned().unwrap_or("public".to_string());
            let return_type = attrs.get("returntype").cloned().unwrap_or_default();

            // Scan ahead for <cfargument> tags to extract parameter names
            let param_names = scan_cfargument_tags(chars, tag_end, len);

            let mut sig = String::new();
            if !access.is_empty() {
                sig.push_str(&access);
                sig.push(' ');
            }
            if !return_type.is_empty() {
                sig.push_str(&return_type);
                sig.push(' ');
            }
            sig.push_str(&format!("function {}({}) {{\n", name, param_names.join(", ")));
            (sig, tag_end - start)
        }
        "cfargument" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            let default = attrs.get("default").cloned();
            if let Some(def) = default {
                let def = strip_hashes(&def);
                // Quote the default if it's not already a number, boolean, or quoted
                let def_val = quote_if_needed(&def);
                (
                    format!("if (isNull(arguments.{})) {{ arguments.{} = {}; }}\n", name, name, def_val),
                    tag_end - start,
                )
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cfinclude" => {
            let template = attrs.get("template").cloned().unwrap_or_default();
            (format!("include \"{}\";\n", template), tag_end - start)
        }
        "cfdump" => {
            let var = attrs.get("var").cloned().unwrap_or("\"\"".to_string());
            let var = strip_hashes(&var);
            (format!("writeDump({});\n", var), tag_end - start)
        }
        "cfthrow" => {
            let message = attrs.get("message").cloned().unwrap_or("An error occurred".to_string());
            let message = strip_hashes(&message);
            let escaped_msg = message.replace('\\', "\\\\").replace('"', "\\\"");

            let type_ = attrs.get("type").cloned().unwrap_or("Application".to_string());
            let escaped_type = strip_hashes(&type_).replace('\\', "\\\\").replace('"', "\\\"");

            let detail = attrs.get("detail").cloned().unwrap_or_default();
            let escaped_detail = strip_hashes(&detail).replace('\\', "\\\\").replace('"', "\\\"");

            let errorcode = attrs.get("errorcode").cloned().unwrap_or_default();
            let escaped_errorcode = strip_hashes(&errorcode).replace('\\', "\\\\").replace('"', "\\\"");

            (format!("throw(\"{}\", \"{}\", \"{}\", \"{}\");\n", escaped_msg, escaped_type, escaped_detail, escaped_errorcode), tag_end - start)
        }
        "cftry" => {
            ("try {\n".to_string(), tag_end - start)
        }
        "cfcatch" => {
            let catch_type = attrs.get("type").cloned().unwrap_or("any".to_string());
            (format!("}} catch ({} cfcatch) {{\n", catch_type), tag_end - start)
        }
        "cfabort" => {
            ("__cfabort();\n".to_string(), tag_end - start)
        }
        "cfparam" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            let default = attrs.get("default").cloned().unwrap_or("\"\"".to_string());
            // If default contains #expr#, strip hashes to get the expression.
            // Otherwise treat it as a string literal and quote it.
            let default = if default.contains('#') {
                strip_hashes(&default)
            } else {
                quote_if_needed(&default)
            };
            // Clean name - remove scope prefix quotes and strip hash expressions
            let clean_name = strip_hashes(&name.replace('"', "").replace('\'', ""));
            (
                format!("if (isNull({})) {{ {} = {}; }}\n", clean_name, clean_name, default),
                tag_end - start,
            )
        }
        "cfcomponent" => {
            let name = attrs.get("name").cloned();
            let extends = attrs.get("extends").cloned();
            let implements = attrs.get("implements").cloned();
            let mut decl = if let Some(ref n) = name {
                format!("component {} ", n)
            } else {
                "component ".to_string()
            };
            if let Some(ext) = extends {
                decl.push_str(&format!("extends=\"{}\" ", ext));
            }
            if let Some(imp) = implements {
                decl.push_str(&format!("implements=\"{}\" ", imp));
            }
            // Pass through extra attributes as metadata key="value" pairs
            for (k, v) in &attrs {
                if k != "name" && k != "extends" && k != "implements" {
                    decl.push_str(&format!("{}=\"{}\" ", k, v));
                }
            }
            decl.push_str("{\n");
            (decl, tag_end - start)
        }
        "cfinterface" => {
            let name = attrs.get("name").cloned();
            let extends = attrs.get("extends").cloned();
            let mut decl = if let Some(ref n) = name {
                format!("interface {} ", n)
            } else {
                "interface ".to_string()
            };
            if let Some(ext) = extends {
                decl.push_str(&format!("extends=\"{}\" ", ext));
            }
            // Pass through extra attributes as metadata key="value" pairs
            for (k, v) in &attrs {
                if k != "name" && k != "extends" {
                    decl.push_str(&format!("{}=\"{}\" ", k, v));
                }
            }
            decl.push_str("{\n");
            (decl, tag_end - start)
        }
        "cfproperty" => {
            // Convert <cfproperty name="x" type="y" inject="z" default="v">
            // to CFScript: property name="x" type="y" inject="z" default="v";
            let mut prop_str = String::from("property ");
            // Output name first, then all other attributes
            if let Some(name) = attrs.get("name") {
                prop_str.push_str(&format!("name=\"{}\" ", name));
            }
            for (k, v) in &attrs {
                if k != "name" {
                    prop_str.push_str(&format!("{}=\"{}\" ", k, v));
                }
            }
            prop_str.push_str(";\n");
            (prop_str, tag_end - start)
        }
        "cfhttp" => {
            let url = attrs.get("url").cloned().unwrap_or_default();
            let method = attrs.get("method").cloned().unwrap_or("GET".to_string());
            let result_var = attrs.get("result").cloned().unwrap_or("cfhttp".to_string());
            let timeout = attrs.get("timeout").cloned();
            let charset = attrs.get("charset").cloned();
            let username = attrs.get("username").cloned();
            let password = attrs.get("password").cloned();
            let useragent = attrs.get("useragent").cloned();
            let proxyserver = attrs.get("proxyserver").cloned();

            let mut opts = Vec::new();
            let url_stripped = strip_hashes(&url);
            if url != url_stripped {
                opts.push(format!("url: {}", url_stripped));
            } else {
                opts.push(format!("url: \"{}\"", url));
            }
            opts.push(format!("method: \"{}\"", method));
            if let Some(t) = timeout { opts.push(format!("timeout: {}", t)); }
            if let Some(c) = charset { opts.push(format!("charset: \"{}\"", c)); }
            if let Some(u) = username { opts.push(format!("username: \"{}\"", u)); }
            if let Some(p) = password { opts.push(format!("password: \"{}\"", p)); }
            if let Some(u) = useragent { opts.push(format!("useragent: \"{}\"", u)); }
            if let Some(p) = proxyserver { opts.push(format!("proxyserver: \"{}\"", p)); }

            // Check for body with <cfhttpparam> child tags
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfhttp") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let params = parse_cfhttpparam_tags(&body);
                if !params.is_empty() {
                    opts.push(format!("params: [{}]", params.join(", ")));
                }
                (format!("{} = cfhttp({{ {} }});\n", result_var, opts.join(", ")), close_end - start)
            } else {
                (format!("{} = cfhttp({{ {} }});\n", result_var, opts.join(", ")), tag_end - start)
            }
        }
        "cfquery" => {
            let name = attrs.get("name").cloned().unwrap_or("queryResult".to_string());
            let datasource = attrs.get("datasource").cloned();
            let return_type = attrs.get("returntype").cloned();

            // Everything between <cfquery> and </cfquery> is the SQL
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfquery") {
                let sql_raw: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                // Scan for <cfqueryparam> tags — replace with ? and collect params
                let (cleaned_sql, query_params) = scan_cfqueryparam_tags(&sql_raw);

                // Process remaining hash expressions in SQL for string interpolation
                let sql = process_sql_hashes(&cleaned_sql);

                let mut opts_parts = Vec::new();
                if let Some(ds) = &datasource {
                    let ds_val = strip_hashes(ds);
                    if ds != &ds_val {
                        // Dynamic datasource — emit as variable reference
                        opts_parts.push(format!("datasource: {}", ds_val));
                    } else {
                        opts_parts.push(format!("datasource: \"{}\"", ds));
                    }
                }
                if let Some(rt) = return_type {
                    opts_parts.push(format!("returnType: \"{}\"", rt));
                }

                let opts_str = if opts_parts.is_empty() {
                    "{}".to_string()
                } else {
                    format!("{{ {} }}", opts_parts.join(", "))
                };

                let params_str = if query_params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_strs: Vec<String> = query_params.iter().map(|p| {
                        if let Some(ac_expr) = &p.attribute_collection_expr {
                            // Build explicit-only overrides; if none, just hand the
                            // user's struct straight through.
                            let mut explicit = Vec::new();
                            if p.explicit_value || p.explicit_null {
                                explicit.push(format!("value: {}", p.value_expr));
                            }
                            if p.explicit_cfsqltype {
                                explicit.push(format!("cfsqltype: \"{}\"", p.cfsqltype));
                            }
                            if p.explicit_null {
                                explicit.push(format!("null: {}", p.null));
                            }
                            if p.explicit_list {
                                explicit.push(format!("list: {}", p.list));
                            }
                            if p.explicit_separator {
                                explicit.push(format!("separator: \"{}\"", p.separator));
                            }
                            if explicit.is_empty() {
                                ac_expr.clone()
                            } else {
                                // duplicate() so we don't mutate the user's struct;
                                // structAppend(..., ..., true) lets the explicit
                                // overrides win.
                                format!(
                                    "structAppend(duplicate({}), {{ {} }}, true)",
                                    ac_expr,
                                    explicit.join(", ")
                                )
                            }
                        } else {
                            let mut parts = Vec::new();
                            parts.push(format!("value: {}", p.value_expr));
                            parts.push(format!("cfsqltype: \"{}\"", p.cfsqltype));
                            if p.null {
                                parts.push("null: true".to_string());
                            }
                            if p.list {
                                parts.push("list: true".to_string());
                                if p.separator != "," {
                                    parts.push(format!("separator: \"{}\"", p.separator));
                                }
                            }
                            format!("{{ {} }}", parts.join(", "))
                        }
                    }).collect();
                    format!("[{}]", param_strs.join(", "))
                };

                (format!("{} = queryExecute({}, {}, {});\n", name, sql, params_str, opts_str), close_end - start)
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cfheader" => {
            // <cfheader statuscode="200" statustext="OK">
            // → __cfheader({statuscode: 200, statustext: "OK"});
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let raw = v.trim();
                if raw.starts_with('#') && raw.ends_with('#') && raw.len() > 2 {
                    // Dynamic expression: strip hashes, emit bare
                    parts.push(format!("{}: {}", k, strip_hashes(raw)));
                } else if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    parts.push(format!("{}: \"{}\"", k, raw.replace('"', "\\\"")));
                }
            }
            (format!("__cfheader({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfcontent" => {
            // <cfcontent reset="true" type="application/json">
            // → __cfcontent({reset: true, type: "application/json"});
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let val = strip_hashes(&v);
                if k == "reset" {
                    let lower = val.to_lowercase();
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else {
                        parts.push(format!("{}: false", k));
                    }
                } else if k == "variable" {
                    parts.push(format!("{}: {}", k, val));
                } else {
                    parts.push(format!("{}: \"{}\"", k, val.replace('"', "\\\"")));
                }
            }
            (format!("__cfcontent({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cflocation" => {
            // <cflocation url="/path" statuscode="302" addtoken="false">
            // → __cflocation({url: "/path", statuscode: 302, addtoken: false});
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let raw = v.trim();
                if raw.starts_with('#') && raw.ends_with('#') && raw.len() > 2 {
                    parts.push(format!("{}: {}", k, strip_hashes(raw)));
                } else if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    let lower = raw.to_lowercase();
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else if lower == "false" || lower == "no" {
                        parts.push(format!("{}: false", k));
                    } else {
                        parts.push(format!("{}: \"{}\"", k, raw.replace('"', "\\\"")));
                    }
                }
            }
            (format!("__cflocation({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfdirectory" => {
            // <cfdirectory action="list" directory="." name="qDir" recurse="true">
            // → qDir = cfdirectory({action: "list", directory: ".", recurse: true});
            let name = attrs.get("name").cloned();
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                if k == "name" {
                    continue;
                }
                let raw = v.trim();
                if raw.starts_with('#') && raw.ends_with('#') && raw.len() > 2 {
                    parts.push(format!("{}: {}", k, strip_hashes(raw)));
                } else {
                    let lower = raw.to_lowercase();
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else if lower == "false" || lower == "no" {
                        parts.push(format!("{}: false", k));
                    } else if raw.parse::<f64>().is_ok() {
                        parts.push(format!("{}: {}", k, raw));
                    } else {
                        parts.push(format!("{}: \"{}\"", k, raw.replace('"', "\\\"")));
                    }
                }
            }
            let call = format!("cfdirectory({{ {} }})", parts.join(", "));
            if let Some(n) = name {
                (format!("{} = {};\n", n, call), tag_end - start)
            } else {
                (format!("{};\n", call), tag_end - start)
            }
        }
        "cfzip" => {
            // <cfzip action="zip" file="out.zip" source="dir/" ...>
            let name = attrs.get("name").cloned();
            let variable = attrs.get("variable").cloned();
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                if k == "name" || k == "variable" { continue; }
                let raw = v.trim();
                if raw.starts_with('#') && raw.ends_with('#') && raw.len() > 2 {
                    parts.push(format!("{}: {}", k, strip_hashes(raw)));
                } else {
                    let lower = raw.to_lowercase();
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else if lower == "false" || lower == "no" {
                        parts.push(format!("{}: false", k));
                    } else if raw.parse::<f64>().is_ok() {
                        parts.push(format!("{}: {}", k, raw));
                    } else {
                        parts.push(format!("{}: \"{}\"", k, raw.replace('"', "\\\"")));
                    }
                }
            }
            let call = format!("cfzip({{ {} }})", parts.join(", "));
            if let Some(n) = name {
                (format!("{} = {};\n", n, call), tag_end - start)
            } else if let Some(v) = variable {
                (format!("{} = {};\n", v, call), tag_end - start)
            } else {
                (format!("{};\n", call), tag_end - start)
            }
        }
        "cfsavecontent" => {
            let variable = attrs.get("variable").cloned().unwrap_or("__savecontent_result".to_string());
            // Find closing tag — body between is processed by main loop
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfsavecontent") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                // Process body through main loop (handles hash expressions, nested tags, text)
                let body_script = tags_to_script_impl(&body, imports);
                (format!("__cfsavecontent_start();\n{}{} = __cfsavecontent_end();\n", body_script, variable), close_end - start)
            } else {
                (format!("__cfsavecontent_start();\n"), tag_end - start)
            }
        }
        "cftransaction" => {
            let action = attrs.get("action").cloned().unwrap_or_else(|| "begin".to_string());
            let isolation = attrs.get("isolation").cloned();
            let datasource = attrs.get("datasource").cloned();

            match action.to_lowercase().as_str() {
                "commit" => {
                    (format!("__cftransaction_commit();\n"), tag_end - start)
                }
                "rollback" => {
                    (format!("__cftransaction_rollback();\n"), tag_end - start)
                }
                _ => {
                    // "begin" (default) — wraps body in try/catch
                    if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cftransaction") {
                        let body: String = chars[tag_end..end_tag_pos].iter().collect();
                        let close_end = find_tag_end(chars, end_tag_pos, len);
                        let body_script = tags_to_script_impl(&body, imports);

                        // Build args for __cftransaction_start
                        let mut txn_args = vec!["\"begin\"".to_string()];
                        if let Some(ref iso) = isolation {
                            txn_args.push(format!("\"{}\"", iso));
                        }
                        if let Some(ref ds) = datasource {
                            let ds_val = strip_hashes(ds);
                            if ds != &ds_val {
                                txn_args.push(ds_val);
                            } else {
                                txn_args.push(format!("\"{}\"", ds));
                            }
                        } else {
                            // Try to extract datasource from the first cfquery inside
                            let ds_from_body = extract_datasource_from_body(&body);
                            if let Some(ds) = ds_from_body {
                                txn_args.push(format!("\"{}\"", ds));
                            }
                        }

                        (format!(
                            "__cftransaction_start({});\ntry {{\n{}\n__cftransaction_commit();\n}} catch(any __txn_e) {{\n__cftransaction_rollback();\nthrow __txn_e;\n}}\n",
                            txn_args.join(", "), body_script
                        ), close_end - start)
                    } else {
                        (format!("__cftransaction_start(\"begin\");\n"), tag_end - start)
                    }
                }
            }
        }
        "cfswitch" => {
            let expression = attrs.get("expression").cloned().unwrap_or_default();
            let expression = strip_hashes(&expression);
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfswitch") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let switch_body = parse_cfswitch_body(&body, imports);
                (format!("switch ({}) {{\n{}}}\n", expression, switch_body), close_end - start)
            } else {
                (format!("switch ({}) {{\n", expression), tag_end - start)
            }
        }
        "cfbreak" => {
            ("break;\n".to_string(), tag_end - start)
        }
        "cfcontinue" => {
            ("continue;\n".to_string(), tag_end - start)
        }
        "cfwhile" => {
            let condition = attrs.get("condition").cloned().unwrap_or("true".to_string());
            let condition = strip_hashes(&condition);
            (format!("while ({}) {{\n", condition), tag_end - start)
        }
        "cffinally" => {
            ("finally {\n".to_string(), tag_end - start)
        }
        "cfrethrow" => {
            ("rethrow;\n".to_string(), tag_end - start)
        }
        "cfloginuser" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            let password = attrs.get("password").cloned().unwrap_or_default();
            let roles = attrs.get("roles").cloned().unwrap_or_default();
            (format!("__cfloginuser(\"{}\", \"{}\", \"{}\");\n", name, password, roles), tag_end - start)
        }
        "cflogout" => {
            ("__cflogout();\n".to_string(), tag_end - start)
        }
        "cflog" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let val = strip_hashes(v);
                if val != *v {
                    parts.push(format!("{}: {}", k, val));
                } else {
                    parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                }
            }
            (format!("__cflog({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfprocessingdirective" => {
            let suppress_ws = attrs.get("suppresswhitespace")
                .map(|v| v.to_lowercase() == "true" || v.to_lowercase() == "yes")
                .unwrap_or(false);
            if suppress_ws {
                if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfprocessingdirective") {
                    let body: String = chars[tag_end..end_tag_pos].iter().collect();
                    let close_end = find_tag_end(chars, end_tag_pos, len);
                    let body_script = tags_to_script_impl(&body, imports);
                    // Capture output, collapse whitespace, then re-emit
                    (format!("__cfsavecontent_start();\n{}writeOutput(__cfprocessingdirective_collapse(__cfsavecontent_end()));\n", body_script), close_end - start)
                } else {
                    // Self-closing or no body — just ignore
                    (String::new(), tag_end - start)
                }
            } else {
                // suppressWhiteSpace=false or just pageEncoding — pass through body
                if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfprocessingdirective") {
                    let body: String = chars[tag_end..end_tag_pos].iter().collect();
                    let close_end = find_tag_end(chars, end_tag_pos, len);
                    let body_script = tags_to_script_impl(&body, imports);
                    (body_script, close_end - start)
                } else {
                    (String::new(), tag_end - start)
                }
            }
        }
        "cfsetting" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let lower = v.to_lowercase();
                if lower == "true" || lower == "yes" {
                    parts.push(format!("{}: true", k));
                } else if lower == "false" || lower == "no" {
                    parts.push(format!("{}: false", k));
                } else if v.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, v));
                } else {
                    parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                }
            }
            (format!("__cfsetting({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfsilent" => {
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfsilent") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_script = tags_to_script_impl(&body, imports);
                (format!("__cfsavecontent_start();\n{}__cfsavecontent_end();\n", body_script), close_end - start)
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cfcookie" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let lower = v.to_lowercase();
                if k == "secure" || k == "httponly" {
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else {
                        parts.push(format!("{}: false", k));
                    }
                } else if v.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, v));
                } else {
                    let val = strip_hashes(v);
                    if val != *v {
                        parts.push(format!("{}: {}", k, val));
                    } else {
                        parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                    }
                }
            }
            (format!("__cfcookie({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cffile" => {
            parse_cffile_tag(&attrs, tag_end - start)
        }
        "cflock" => {
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cflock") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_script = tags_to_script_impl(&body, imports);
                let mut lock_parts = Vec::new();
                for (k, v) in &attrs {
                    let lower = v.to_lowercase();
                    if v.parse::<f64>().is_ok() {
                        lock_parts.push(format!("{}: {}", k, v));
                    } else if lower == "true" || lower == "yes" {
                        lock_parts.push(format!("{}: true", k));
                    } else if lower == "false" || lower == "no" {
                        lock_parts.push(format!("{}: false", k));
                    } else {
                        lock_parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                    }
                }
                let lock_args = format!("{{ {} }}", lock_parts.join(", "));
                (format!(
                    "__cflock_start({});\ntry {{\n{}\n__cflock_end({});\n}} catch(any __lock_e) {{\n__cflock_end({});\nthrow __lock_e;\n}}\n",
                    lock_args, body_script, lock_args, lock_args
                ), close_end - start)
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cfinvoke" => {
            // <cfinvoke component="MyComp" method="greet" name="World" returnvariable="msg">
            // → msg = __cfinvoke(MyComp, "greet", {name: "World"});
            let component = attrs.get("component").cloned().unwrap_or_default();
            let method = attrs.get("method").cloned().unwrap_or_default();
            let return_var = attrs.get("returnvariable").cloned();
            let arg_collection = attrs.get("argumentcollection").cloned();

            // Component: strip hashes for dynamic (#var#), quote for static name
            let comp_expr = if component.starts_with('#') && component.ends_with('#') && component.len() > 2 {
                strip_hashes(&component)
            } else {
                format!("\"{}\"", component)
            };

            // Method: always quoted
            let method_expr = format!("\"{}\"", method);

            // Third argument: argumentcollection or struct of remaining attrs
            let third_arg = if let Some(ac) = arg_collection {
                let ac = strip_hashes(&ac);
                ac
            } else {
                let reserved = ["component", "method", "returnvariable", "argumentcollection"];
                let mut extra_parts = Vec::new();
                for (k, v) in &attrs {
                    if reserved.contains(&k.as_str()) {
                        continue;
                    }
                    extra_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k))));
                }
                format!("{{ {} }}", extra_parts.join(", "))
            };

            let call = format!("__cfinvoke({}, {}, {})", comp_expr, method_expr, third_arg);
            if let Some(rv) = return_var {
                (format!("{} = {};\n", rv, call), tag_end - start)
            } else {
                (format!("{};\n", call), tag_end - start)
            }
        }
        "cfmodule" => {
            // <cfmodule template="path.cfm" attr1="val1"> or <cfmodule name="dot.path" attr1="val1">
            let template = attrs.get("template").cloned();
            let name_attr = attrs.get("name").cloned();
            let uses_template = template.is_some();

            let path_expr = if let Some(t) = template {
                let t = strip_hashes(&t);
                format!("\"{}\"", t.replace('"', "\\\""))
            } else if let Some(n) = name_attr {
                let n = strip_hashes(&n);
                format!("\"__name:{}\"", n.replace('"', "\\\""))
            } else {
                return ("".to_string(), tag_end - start); // missing required attr
            };

            // Build attributes struct from non-reserved attrs
            // "template" is always reserved; "name" only reserved in name= form
            let mut attr_parts = Vec::new();
            for (k, v) in &attrs {
                let kl = k.to_lowercase();
                if kl == "template" {
                    continue;
                }
                if kl == "name" && !uses_template {
                    continue;
                }
                attr_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k))));
            }
            let attrs_expr = format!("{{ {} }}", attr_parts.join(", "));

            // Check for body (closing </cfmodule>)
            let tag_name_full = "cfmodule";
            if let Some(body_start) = find_closing_tag(chars, tag_end, len, tag_name_full) {
                // Body tag: emit start, recursively preprocess body, then end marker
                let body_chars = &chars[tag_end..body_start];
                let body_source: String = body_chars.iter().collect();
                let body_script = tags_to_script_impl(&body_source, imports);
                let close_end = find_tag_end(chars, body_start, len);
                let result = format!(
                    "__cfcustomtag_start({}, {});\n{}\n__cfcustomtag_end();\n",
                    path_expr, attrs_expr, body_script
                );
                (result, close_end - start)
            } else {
                // Self-closing
                let result = format!("__cfcustomtag({}, {});\n", path_expr, attrs_expr);
                (result, tag_end - start)
            }
        }
        "cfcache" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let val = strip_hashes(v);
                if val != *v {
                    parts.push(format!("{}: {}", k, val));
                } else {
                    parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                }
            }
            (format!("__cfcache({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfexecute" => {
            let name_attr = attrs.get("name").cloned().unwrap_or_default();
            let arguments = attrs.get("arguments").cloned();
            let variable = attrs.get("variable").cloned();
            let error_variable = attrs.get("errorvariable").cloned();
            let timeout = attrs.get("timeout").cloned();

            let mut opts = Vec::new();
            let name_stripped = strip_hashes(&name_attr);
            if name_attr != name_stripped {
                opts.push(format!("name: {}", name_stripped));
            } else {
                opts.push(format!("name: \"{}\"", name_attr));
            }
            if let Some(a) = &arguments {
                let a_stripped = strip_hashes(a);
                if *a != a_stripped {
                    opts.push(format!("arguments: {}", a_stripped));
                } else {
                    opts.push(format!("arguments: \"{}\"", a.replace('"', "\\\"")));
                }
            }
            if let Some(t) = &timeout { opts.push(format!("timeout: {}", t)); }
            if variable.is_some() {
                opts.push("variable: true".to_string());
            }

            // Check for body (stdin input)
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfexecute") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_trimmed = body.trim();
                if !body_trimmed.is_empty() {
                    opts.push(format!("body: \"{}\"", body_trimmed.replace('"', "\\\"").replace('\n', "\\n")));
                }
                if let Some(ref var) = variable {
                    let mut result = format!("__cfexec_tmp = __cfexecute({{ {} }});\n", opts.join(", "));
                    result.push_str(&format!("{} = __cfexec_tmp.output;\n", var));
                    if let Some(ref ev) = error_variable {
                        result.push_str(&format!("{} = __cfexec_tmp.error;\n", ev));
                    }
                    (result, close_end - start)
                } else {
                    (format!("__cfexecute({{ {} }});\n", opts.join(", ")), close_end - start)
                }
            } else {
                // Self-closing
                if let Some(ref var) = variable {
                    let mut result = format!("__cfexec_tmp = __cfexecute({{ {} }});\n", opts.join(", "));
                    result.push_str(&format!("{} = __cfexec_tmp.output;\n", var));
                    if let Some(ref ev) = error_variable {
                        result.push_str(&format!("{} = __cfexec_tmp.error;\n", ev));
                    }
                    (result, tag_end - start)
                } else {
                    (format!("__cfexecute({{ {} }});\n", opts.join(", ")), tag_end - start)
                }
            }
        }
        "cfmail" => {
            let mut opts = Vec::new();
            for (k, v) in &attrs {
                let val = strip_hashes(v);
                if val != *v {
                    opts.push(format!("{}: {}", k, val));
                } else {
                    opts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                }
            }

            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfmail") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                // Parse cfmailparam child tags
                let params = parse_cfmailparam_tags(&body);
                if !params.is_empty() {
                    opts.push(format!("params: [{}]", params.join(", ")));
                }

                // Parse cfmailpart child tags
                let (parts, remaining_body) = parse_cfmailpart_tags(&body);
                if !parts.is_empty() {
                    opts.push(format!("parts: [{}]", parts.join(", ")));
                }

                // Use remaining body (after stripping child tags) as body text
                let body_text = remaining_body.trim();
                if !body_text.is_empty() {
                    opts.push(format!("body: \"{}\"", body_text.replace('"', "\\\"").replace('\n', "\\n")));
                }

                (format!("__cfmail({{ {} }});\n", opts.join(", ")), close_end - start)
            } else {
                // Self-closing cfmail
                (format!("__cfmail({{ {} }});\n", opts.join(", ")), tag_end - start)
            }
        }
        "cfmailparam" | "cfmailpart" => {
            // These are handled as child tags of cfmail; skip if encountered standalone
            (String::new(), tag_end - start)
        }
        "cfstoredproc" => {
            let procedure = attrs.get("procedure").cloned().unwrap_or_default();
            let datasource = attrs.get("datasource").cloned();

            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfstoredproc") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                let proc_params = parse_cfprocparam_tags(&body);
                let proc_results = parse_cfprocresult_tags(&body);

                // Build param placeholders
                let placeholders: Vec<&str> = proc_params.iter().map(|_| "?").collect();
                let sql = format!("CALL {}({})", procedure, placeholders.join(","));

                // Build params array
                let params_arr: Vec<String> = proc_params.iter().map(|p| {
                    let mut parts = Vec::new();
                    if let Some(ref v) = p.value {
                        let stripped = strip_hashes(v);
                        if stripped != *v {
                            parts.push(format!("value: {}", stripped));
                        } else {
                            parts.push(format!("value: \"{}\"", v.replace('"', "\\\"")));
                        }
                    }
                    if let Some(ref t) = p.cfsqltype {
                        parts.push(format!("cfsqltype: \"{}\"", t));
                    }
                    format!("{{ {} }}", parts.join(", "))
                }).collect();

                let mut query_opts = Vec::new();
                if let Some(ds) = &datasource {
                    query_opts.push(format!("datasource: \"{}\"", ds));
                }

                let result_var = proc_results.first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| "cfresult".to_string());

                let opts_str = if query_opts.is_empty() {
                    String::new()
                } else {
                    format!(", {{ {} }}", query_opts.join(", "))
                };

                (format!("{} = queryExecute(\"{}\", [{}]{});\n",
                    result_var, sql, params_arr.join(", "), opts_str), close_end - start)
            } else {
                // Self-closing (unusual)
                (format!("queryExecute(\"CALL {}()\");\n", procedure), tag_end - start)
            }
        }
        "cfprocparam" | "cfprocresult" => {
            // These are handled as child tags of cfstoredproc; skip if encountered standalone
            (String::new(), tag_end - start)
        }
        "cfthread" => {
            let action = attrs.get("action").cloned().unwrap_or_else(|| "run".to_string()).to_lowercase();
            let thread_name = attrs.get("name").cloned().unwrap_or_else(|| "thread1".to_string());
            let thread_name_expr = format!("\"{}\"", thread_name.replace('"', "\\\""));

            match action.as_str() {
                "run" => {
                    if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfthread") {
                        let body: String = chars[tag_end..end_tag_pos].iter().collect();
                        let close_end = find_tag_end(chars, end_tag_pos, len);
                        let body_script = tags_to_script_impl(&body, imports);
                        (format!(
                            "__cfthread_run({}, function() {{\n{}\n}});\n",
                            thread_name_expr, body_script
                        ), close_end - start)
                    } else {
                        (String::new(), tag_end - start)
                    }
                }
                "join" => {
                    let timeout = attrs.get("timeout").cloned().unwrap_or_else(|| "0".to_string());
                    (format!("__cfthread_join({}, {});\n", thread_name_expr, timeout), tag_end - start)
                }
                "terminate" => {
                    (format!("__cfthread_terminate({});\n", thread_name_expr), tag_end - start)
                }
                _ => {
                    (format!("throw(\"cfthread action='{}' is not supported.\");\n", action), tag_end - start)
                }
            }
        }
        "cfimport" => {
            // Register prefix→taglib mapping for CFML custom tag libraries.
            if let (Some(taglib), Some(prefix)) = (attrs.get("taglib"), attrs.get("prefix")) {
                let prefix_lower = prefix.to_lowercase();
                imports.insert(prefix_lower.clone(), taglib.clone());
                // Check for .tld files in the taglib directory
                if let Ok(entries) = std::fs::read_dir(taglib) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |e| e == "tld") {
                            let tld_map = parse_tld_file(&path.to_string_lossy());
                            if !tld_map.is_empty() {
                                TLD_CACHE.with(|cache| {
                                    cache.borrow_mut().insert(prefix_lower.clone(), tld_map);
                                });
                            }
                        }
                    }
                }
            } else {
                // Non-taglib imports (e.g. Java class imports) are not supported
                let detail = attrs.get("prefix")
                    .or_else(|| attrs.get("name"))
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                return (
                    format!("throw(\"cfimport without taglib is not implemented ({}). Only CFML taglib imports are supported.\");\n", detail.replace('"', "\\\"" )),
                    tag_end - start,
                );
            }
            (String::new(), tag_end - start)
        }
        _ => {
            if tag_lower.starts_with("cf_") {
                // Custom tag: <cf_tagname attr1="val1">
                let custom_tag_name = &tag_lower[3..]; // strip "cf_"
                let path_expr = format!("\"__cf_:{}\"", custom_tag_name);

                // Build attributes struct from all attrs
                let mut attr_parts = Vec::new();
                for (k, v) in &attrs {
                    attr_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k))));
                }
                let attrs_expr = format!("{{ {} }}", attr_parts.join(", "));

                // Check for body (closing </cf_tagname>)
                let tag_name_full = format!("cf_{}", custom_tag_name);
                if let Some(body_start) = find_closing_tag(chars, tag_end, len, &tag_name_full) {
                    let body_chars = &chars[tag_end..body_start];
                    let body_source: String = body_chars.iter().collect();
                    let body_script = tags_to_script_impl(&body_source, imports);
                    let close_end = find_tag_end(chars, body_start, len);
                    let result = format!(
                        "__cfcustomtag_start({}, {});\n{}\n__cfcustomtag_end();\n",
                        path_expr, attrs_expr, body_script
                    );
                    (result, close_end - start)
                } else {
                    // Self-closing
                    let result = format!("__cfcustomtag({}, {});\n", path_expr, attrs_expr);
                    (result, tag_end - start)
                }
            } else {
                // Unknown/unsupported CFML tag — emit runtime error
                (
                    format!("throw(\"Tag <{}> is not implemented.\");\n", tag_name),
                    tag_end - start,
                )
            }
        }
    }
}

fn find_tag_end(chars: &[char], start: usize, len: usize) -> usize {
    let mut i = start;
    let mut in_string = false;
    let mut string_char = '"';
    while i < len {
        if !in_string && (chars[i] == '"' || chars[i] == '\'') {
            in_string = true;
            string_char = chars[i];
        } else if in_string && chars[i] == string_char {
            in_string = false;
        } else if !in_string && chars[i] == '>' {
            return i + 1;
        }
        i += 1;
    }
    len
}

fn parse_tag_attributes(
    chars: &[char],
    start: usize,
    len: usize,
) -> (std::collections::HashMap<String, String>, std::collections::HashSet<String>, usize) {
    let mut attrs = std::collections::HashMap::new();
    let mut quoted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut i = start;

    // Skip whitespace
    while i < len && chars[i].is_whitespace() {
        i += 1;
    }

    while i < len && chars[i] != '>' && !(chars[i] == '/' && i + 1 < len && chars[i + 1] == '>') {
        // Parse attribute name
        let attr_start = i;
        while i < len && chars[i] != '=' && chars[i] != '>' && !chars[i].is_whitespace() {
            i += 1;
        }
        let attr_name: String = chars[attr_start..i].iter().collect();

        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }

        if i < len && chars[i] == '=' {
            i += 1; // skip =
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }

            // Parse attribute value
            if i < len && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                let val_start = i;
                // Parse attribute value, handling:
                // - #expr# hash expressions (may contain quotes internally)
                // - Doubled-quote escaping ("" or '')
                while i < len {
                    if chars[i] == '#' && i + 1 < len && chars[i + 1] != '#' {
                        // Hash expression: skip to closing # (respecting nested strings)
                        i += 1;
                        let mut hash_depth = 0;
                        while i < len {
                            if chars[i] == '#' && hash_depth == 0 {
                                i += 1; // skip closing #
                                break;
                            }
                            if chars[i] == '"' || chars[i] == '\'' {
                                // Skip string literal inside hash expression
                                // CFML does NOT use backslash escaping — quotes are
                                // escaped by doubling ("" or '')
                                let inner_quote = chars[i];
                                i += 1;
                                while i < len {
                                    if chars[i] == inner_quote {
                                        if i + 1 < len && chars[i + 1] == inner_quote {
                                            i += 2; // doubled quote escape
                                            continue;
                                        }
                                        break; // closing quote
                                    }
                                    i += 1;
                                }
                                if i < len { i += 1; } // skip closing inner quote
                                continue;
                            }
                            if chars[i] == '(' { hash_depth += 1; }
                            if chars[i] == ')' && hash_depth > 0 { hash_depth -= 1; }
                            i += 1;
                        }
                        continue;
                    }
                    if chars[i] == '#' && i + 1 < len && chars[i + 1] == '#' {
                        i += 2; // escaped ##
                        continue;
                    }
                    if chars[i] == quote {
                        // Check for doubled quote (e.g., "" inside "...")
                        if i + 1 < len && chars[i + 1] == quote {
                            i += 2;
                            continue;
                        }
                        break; // closing quote
                    }
                    i += 1;
                }
                let val: String = chars[val_start..i].iter().collect();
                if i < len {
                    i += 1; // skip closing quote
                }
                let key = attr_name.to_lowercase();
                quoted.insert(key.clone());
                attrs.insert(key, val);
            } else {
                // Unquoted value
                let val_start = i;
                while i < len && !chars[i].is_whitespace() && chars[i] != '>' {
                    i += 1;
                }
                let val: String = chars[val_start..i].iter().collect();
                attrs.insert(attr_name.to_lowercase(), val);
            }
        } else if !attr_name.is_empty() {
            attrs.insert(attr_name.to_lowercase(), String::new());
        }

        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
    }

    // Find the actual end of the tag
    let tag_end = find_tag_end(chars, i, len);
    (attrs, quoted, tag_end)
}

/// Format an attribute value for emission inside a script struct literal.
/// If the value came from a quoted attribute (e.g. `value="caller-ok"`), emit
/// a proper script string literal, expanding `#expr#` segments to `&` concat.
/// If it came from an unquoted attribute (e.g. `value=someExpr`), fall back to
/// the legacy expression-or-literal heuristic in `quote_if_needed`.
fn format_attr_value(raw: &str, was_quoted: bool) -> String {
    if !was_quoted {
        return quote_if_needed(&strip_hashes(raw));
    }
    // Quoted attribute. Split into literal segments and `#...#` expressions.
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < len {
        if chars[i] == '#' {
            // Doubled '##' is a literal '#'
            if i + 1 < len && chars[i + 1] == '#' {
                buf.push('#');
                i += 2;
                continue;
            }
            // Flush literal, then collect expression up to closing '#'
            if !buf.is_empty() || parts.is_empty() {
                parts.push(format!("\"{}\"", escape_for_string_literal(&buf)));
                buf.clear();
            }
            i += 1;
            let expr_start = i;
            let mut depth = 0usize;
            while i < len {
                if chars[i] == '#' && depth == 0 {
                    break;
                }
                if chars[i] == '(' { depth += 1; }
                if chars[i] == ')' && depth > 0 { depth -= 1; }
                i += 1;
            }
            let expr: String = chars[expr_start..i].iter().collect();
            parts.push(format!("({})", expr));
            if i < len { i += 1; } // skip closing '#'
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() || parts.is_empty() {
        parts.push(format!("\"{}\"", escape_for_string_literal(&buf)));
    }
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        parts.join(" & ")
    }
}

fn escape_for_string_literal(s: &str) -> String {
    // CFML double-quoted strings escape `"` by doubling. We emit script-style
    // string literals into generated code that goes through the script parser;
    // the script parser supports both `\"` and `""`. Use `""` to keep it CFML-native.
    s.replace('"', "\"\"")
}

/// Quote a string value if it's not already a number, boolean, expression, or quoted
fn quote_if_needed(s: &str) -> String {
    let s = s.trim();
    // Already quoted
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return s.to_string();
    }
    // Number
    if s.parse::<f64>().is_ok() {
        return s.to_string();
    }
    // Boolean/null keywords
    let lower = s.to_lowercase();
    if lower == "true" || lower == "false" || lower == "null" || lower == "yes" || lower == "no" {
        return s.to_string();
    }
    // Contains operators or function calls - looks like an expression
    // But paths like "/foo/bar.cfm" should still be quoted.
    // Distinguish: if it starts with "/" and looks like a file path, quote it.
    let looks_like_path = s.starts_with('/') && !s.contains('(')
        && (s.contains('.') && s.split('/').all(|seg| seg.is_empty() || seg.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')));
    if looks_like_path {
        return format!("\"{}\"", s.replace('"', "\\\""));
    }
    if s.contains('(') || s.contains('+') || s.contains('-') || s.contains('*')
        || s.contains('/') || s.contains('&') || s.contains('.') || s.contains('[')
    {
        return s.to_string();
    }
    // Otherwise, quote it
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Escape backslashes inside string literals in tag body expressions.
/// CFML tag-based code doesn't use backslash escaping, but the script parser does.
/// This converts `\` to `\\` inside string literals so the script parser
/// correctly interprets them as literal backslashes.
fn escape_backslashes_in_tag_strings(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            result.push(quote); // opening quote
            i += 1;
            while i < len {
                if chars[i] == quote {
                    // Check for doubled quote (CFML escape: "" or '')
                    if i + 1 < len && chars[i + 1] == quote {
                        result.push(quote);
                        result.push(quote);
                        i += 2;
                    } else {
                        // End of string
                        break;
                    }
                } else if chars[i] == '\\' {
                    result.push('\\');
                    result.push('\\');
                    i += 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < len {
                result.push(chars[i]); // closing quote
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

fn strip_hashes(s: &str) -> String {
    let s = s.trim();
    // If the entire string is wrapped in #...#, just strip outer hashes
    if s.starts_with('#') && s.ends_with('#') && s.len() > 2 && s[1..s.len()-1].find('#').is_none() {
        return s[1..s.len() - 1].to_string();
    }
    // Handle embedded #expr# within larger expressions
    // Replace #expr# with just expr (strip the hash delimiters)
    if !s.contains('#') {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0;
    while i < len {
        if chars[i] == '#' {
            // Look for closing #
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '#') {
                let end = i + 1 + end;
                // Extract expression between hashes
                let expr: String = chars[i + 1..end].iter().collect();
                result.push_str(&expr);
                i = end + 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn find_closing_tag(chars: &[char], start: usize, len: usize, tag_name: &str) -> Option<usize> {
    let close_target = format!("</{}", tag_name);
    let close_lower = close_target.to_lowercase();
    let open_target = format!("<{}", tag_name);
    let open_lower = open_target.to_lowercase();
    let mut depth = 0;
    let mut i = start;
    while i < len {
        if chars[i] == '<' {
            if chars.get(i + 1) == Some(&'/') {
                // Potential closing tag
                let remaining: String = chars[i..].iter().take(close_target.len() + 1).collect();
                if remaining.to_lowercase().starts_with(&close_lower) {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                    i += close_target.len();
                    continue;
                }
            } else {
                // Potential opening tag (same name = nested)
                let remaining: String = chars[i..].iter().take(open_target.len() + 1).collect();
                let rem_lower = remaining.to_lowercase();
                if rem_lower.starts_with(&open_lower) {
                    // Verify it's actually a tag (next char is space, >, or /)
                    let next_char = remaining.chars().nth(open_target.len());
                    if matches!(next_char, Some(' ') | Some('>') | Some('/') | Some('\t') | Some('\n') | Some('\r') | None) {
                        depth += 1;
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Scan ahead from current position to find <cfargument> tags and extract their names
fn scan_cfargument_tags(chars: &[char], start: usize, len: usize) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = start;

    while i < len {
        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        // Check if we hit a <cfargument
        if i + 12 < len && chars[i] == '<' {
            let tag: String = chars[i..i + 12].iter().collect();
            if tag.to_lowercase() == "<cfargument " || tag.to_lowercase() == "<cfargument>" {
                // Parse the tag's attributes
                let name_start = i + 1; // skip <
                let mut j = name_start;
                while j < len && chars[j].is_alphanumeric() {
                    j += 1;
                }
                let (tag_attrs, _, _) = parse_tag_attributes(chars, j, len);
                if let Some(name) = tag_attrs.get("name") {
                    names.push(name.clone());
                }
                // Skip to end of tag
                while i < len && chars[i] != '>' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                }
                continue;
            }
            // If we hit any other CF tag (like <cfreturn>, <cfset>, etc.) or closing </cffunction>, stop scanning
            let next_chars: String = chars[i..std::cmp::min(i + 15, len)].iter().collect();
            let next_lower = next_chars.to_lowercase();
            if next_lower.starts_with("</cffunction") || next_lower.starts_with("<cfreturn")
                || next_lower.starts_with("<cfset") || next_lower.starts_with("<cfif")
                || next_lower.starts_with("<cfloop") || next_lower.starts_with("<cfoutput")
                || next_lower.starts_with("<cftry")
            {
                break;
            }
        }
        i += 1;
    }

    names
}

fn parse_cfloop_tag(
    attrs: &std::collections::HashMap<String, String>,
    consumed: usize,
) -> (String, usize) {
    // Different loop types based on attributes
    if let (Some(from), Some(to), Some(index)) = (
        attrs.get("from"),
        attrs.get("to"),
        attrs.get("index"),
    ) {
        let step = attrs.get("step").cloned().unwrap_or("1".to_string());
        let from = strip_hashes(from);
        let to = strip_hashes(to);
        let step = strip_hashes(&step);
        let comparison = if step.trim_start().starts_with('-') {
            ">="
        } else {
            "<="
        };
        (
            format!(
                "for (var {} = {}; {} {} {}; {} = {} + {}) {{\n",
                index, from, index, comparison, to, index, index, step
            ),
            consumed,
        )
    } else if let Some(condition) = attrs.get("condition") {
        let condition = strip_hashes(condition);
        (format!("while ({}) {{\n", condition), consumed)
    } else if let (Some(array), Some(index)) = (attrs.get("array"), attrs.get("index")) {
        let array = strip_hashes(array);
        let index = strip_hashes(index);
        (format!("for (var {} in {}) {{\n", index, array), consumed)
    } else if let (Some(list), Some(index)) = (attrs.get("list"), attrs.get("index")) {
        // If the list contains #expr#, strip hashes to get the expression.
        // Otherwise treat it as a string literal and quote it.
        let list = if list.contains('#') {
            strip_hashes(list)
        } else {
            quote_if_needed(list)
        };
        let index = strip_hashes(index);
        let delimiters = attrs
            .get("delimiters")
            .cloned()
            .unwrap_or(",".to_string());
        (
            format!(
                "for (var {} in listToArray({}, \"{}\")) {{\n",
                index, list, delimiters
            ),
            consumed,
        )
    } else if let Some(query) = attrs.get("query") {
        let query = strip_hashes(query);
        if let Some(index) = attrs.get("index").or(attrs.get("item")) {
            (
                format!("for (var {} in {}) {{\n", index, query),
                consumed,
            )
        } else {
            // <cfloop query="q"> without index — CFML query row loop
            // q.column resolves to the current row's column value
            (
                format!("for (var __qrow in {}) {{ {} = __qrow;\n", query, query),
                consumed,
            )
        }
    } else if let Some(collection) = attrs.get("collection") {
        let collection = strip_hashes(collection);
        let item = attrs.get("item").cloned().unwrap_or("item".to_string());
        let key = attrs.get("key");
        if let Some(key) = key {
            (
                format!("for (var {} in structKeyArray({})) {{ var {} = {}[{}];\n", key, collection, item, collection, key),
                consumed,
            )
        } else {
            (format!("for (var {} in {}) {{\n", item, collection), consumed)
        }
    } else {
        // Infinite loop? Just use while(true)
        ("while (true) {\n".to_string(), consumed)
    }
}

/// Parse the body of a <cfswitch> tag, scanning for <cfcase> and <cfdefaultcase>
fn parse_cfswitch_body(body: &str, imports: &mut std::collections::HashMap<String, String>) -> String {
    let mut result = String::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= len { break; }

        // Look for <cfcase or <cfdefaultcase
        if chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 16, len)].iter().collect();
            let ahead_lower = ahead.to_lowercase();

            if ahead_lower.starts_with("<cfdefaultcase") {
                // Find end of opening tag
                let tag_content_start = i + 1;
                let mut j = tag_content_start;
                while j < len && chars[j].is_alphanumeric() { j += 1; }
                let (_attrs, _, tag_end) = parse_tag_attributes(&chars, j, len);
                // Find closing </cfdefaultcase>
                if let Some(close_pos) = find_closing_tag(&chars, tag_end, len, "cfdefaultcase") {
                    let case_body: String = chars[tag_end..close_pos].iter().collect();
                    let case_script = tags_to_script_impl(&case_body, imports);
                    result.push_str(&format!("default: \n{}", case_script));
                    let close_end = find_tag_end(&chars, close_pos, len);
                    i = close_end;
                } else {
                    i += 1;
                }
            } else if ahead_lower.starts_with("<cfcase") {
                // Parse attributes for value
                let tag_content_start = i + 1;
                let mut j = tag_content_start;
                while j < len && chars[j].is_alphanumeric() { j += 1; }
                let (case_attrs, _, tag_end) = parse_tag_attributes(&chars, j, len);
                let value = case_attrs.get("value").cloned().unwrap_or_default();
                // Find closing </cfcase>
                if let Some(close_pos) = find_closing_tag(&chars, tag_end, len, "cfcase") {
                    let case_body: String = chars[tag_end..close_pos].iter().collect();
                    let case_script = tags_to_script_impl(&case_body, imports);
                    // Value can be comma-separated for multiple case values
                    let values: Vec<&str> = value.split(',').map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
                    let quoted_values: Vec<String> = values.iter().map(|v| {
                        let v = strip_hashes(v);
                        if v.parse::<f64>().is_ok() {
                            v
                        } else {
                            format!("\"{}\"", v.replace('"', "\\\""))
                        }
                    }).collect();
                    result.push_str(&format!("case {}: \n{}break;\n", quoted_values.join(", "), case_script));
                    let close_end = find_tag_end(&chars, close_pos, len);
                    i = close_end;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    result
}

/// Parse <cffile> tag and convert to appropriate function calls
fn parse_cffile_tag(
    attrs: &std::collections::HashMap<String, String>,
    consumed: usize,
) -> (String, usize) {
    let action = attrs.get("action").cloned().unwrap_or("read".to_string()).to_lowercase();

    match action.as_str() {
        "read" => {
            let file = attrs.get("file").cloned().unwrap_or_default();
            let file = strip_hashes(&file);
            let variable = attrs.get("variable").cloned().unwrap_or("cffile".to_string());
            let file_expr = if file.contains('.') || file.contains('(') {
                file
            } else {
                format!("\"{}\"", file.replace('"', "\\\""))
            };
            (format!("{} = fileRead({});\n", variable, file_expr), consumed)
        }
        "readbinary" => {
            let file = attrs.get("file").cloned().unwrap_or_default();
            let file = strip_hashes(&file);
            let variable = attrs.get("variable").cloned().unwrap_or("cffile".to_string());
            let file_expr = if file.contains('.') || file.contains('(') {
                file
            } else {
                format!("\"{}\"", file.replace('"', "\\\""))
            };
            (format!("{} = fileReadBinary({});\n", variable, file_expr), consumed)
        }
        "write" => {
            let file = attrs.get("file").cloned().unwrap_or_default();
            let file = strip_hashes(&file);
            let output = attrs.get("output").cloned().unwrap_or_default();
            let output = strip_hashes(&output);
            let file_expr = if file.contains('.') || file.contains('(') { file } else { format!("\"{}\"", file.replace('"', "\\\"")) };
            let output_expr = if output.contains('.') || output.contains('(') || output.contains('"') { output } else { format!("\"{}\"", output.replace('"', "\\\"")) };
            (format!("fileWrite({}, {});\n", file_expr, output_expr), consumed)
        }
        "append" => {
            let file = attrs.get("file").cloned().unwrap_or_default();
            let file = strip_hashes(&file);
            let output = attrs.get("output").cloned().unwrap_or_default();
            let output = strip_hashes(&output);
            let file_expr = if file.contains('.') || file.contains('(') { file } else { format!("\"{}\"", file.replace('"', "\\\"")) };
            let output_expr = if output.contains('.') || output.contains('(') || output.contains('"') { output } else { format!("\"{}\"", output.replace('"', "\\\"")) };
            (format!("fileAppend({}, {});\n", file_expr, output_expr), consumed)
        }
        "copy" => {
            let source = attrs.get("source").cloned().unwrap_or_default();
            let source = strip_hashes(&source);
            let dest = attrs.get("destination").cloned().unwrap_or_default();
            let dest = strip_hashes(&dest);
            let src_expr = if source.contains('.') || source.contains('(') { source } else { format!("\"{}\"", source.replace('"', "\\\"")) };
            let dst_expr = if dest.contains('.') || dest.contains('(') { dest } else { format!("\"{}\"", dest.replace('"', "\\\"")) };
            (format!("fileCopy({}, {});\n", src_expr, dst_expr), consumed)
        }
        "move" | "rename" => {
            let source = attrs.get("source").cloned().unwrap_or_default();
            let source = strip_hashes(&source);
            let dest = attrs.get("destination").cloned().unwrap_or_default();
            let dest = strip_hashes(&dest);
            let src_expr = if source.contains('.') || source.contains('(') { source } else { format!("\"{}\"", source.replace('"', "\\\"")) };
            let dst_expr = if dest.contains('.') || dest.contains('(') { dest } else { format!("\"{}\"", dest.replace('"', "\\\"")) };
            (format!("fileMove({}, {});\n", src_expr, dst_expr), consumed)
        }
        "delete" => {
            let file = attrs.get("file").cloned().unwrap_or_default();
            let file = strip_hashes(&file);
            let file_expr = if file.contains('.') || file.contains('(') { file } else { format!("\"{}\"", file.replace('"', "\\\"")) };
            (format!("fileDelete({});\n", file_expr), consumed)
        }
        "upload" | "uploadall" => {
            let func = if action == "uploadall" { "__cffile_upload" } else { "__cffile_upload" };
            let mut parts = Vec::new();
            for (k, v) in attrs {
                if k == "action" { continue; }
                let val = strip_hashes(v);
                if val != *v {
                    parts.push(format!("{}: {}", k, val));
                } else {
                    parts.push(format!("{}: \"{}\"", k, v.replace('"', "\\\"")));
                }
            }
            (format!("{}({{ {} }});\n", func, parts.join(", ")), consumed)
        }
        _ => {
            (format!("throw(\"cffile action='{}' is not implemented.\");\n", action), consumed)
        }
    }
}

/// Extract datasource from the first <cfquery> tag in a body string
fn extract_datasource_from_body(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    if let Some(pos) = lower.find("<cfquery") {
        let chars: Vec<char> = body.chars().collect();
        let len = chars.len();
        // Skip tag name
        let mut i = pos + 8; // past "<cfquery"
        while i < len && chars[i].is_alphanumeric() {
            i += 1;
        }
        let (attrs, _, _) = parse_tag_attributes(&chars, i, len);
        return attrs.get("datasource").cloned();
    }
    None
}

// -----------------------------------------------
// cfqueryparam scanning
// -----------------------------------------------

struct CfQueryParam {
    value_expr: String,  // The value expression (script-ready: variable ref or string literal)
    cfsqltype: String,
    null: bool,
    list: bool,
    separator: String,
    // Optional bare script expression supplied via `attributeCollection="#expr#"`.
    // When Some, the param item is built by overlaying the explicit attrs on top
    // of this struct so explicit `value=`/`cfsqltype=` win, per Lucee semantics.
    attribute_collection_expr: Option<String>,
    explicit_value: bool,
    explicit_cfsqltype: bool,
    explicit_null: bool,
    explicit_list: bool,
    explicit_separator: bool,
}

/// Scan SQL body for <cfqueryparam> tags, replace them with ? placeholders,
/// and collect structured parameter info.
fn scan_cfqueryparam_tags(sql_body: &str) -> (String, Vec<CfQueryParam>) {
    let mut result = String::with_capacity(sql_body.len());
    let mut params = Vec::new();
    let chars: Vec<char> = sql_body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Look for <cfqueryparam
        if i + 14 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 14, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfqueryparam") {
                // Check if followed by space or > (not a different tag)
                let next_after = chars.get(i + 13);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    // Parse the tag attributes
                    let name_end = i + 13; // after "cfqueryparam"
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    // Extract cfqueryparam attributes
                    let explicit_value = tag_attrs.contains_key("value");
                    let explicit_cfsqltype = tag_attrs.contains_key("cfsqltype");
                    let explicit_null = tag_attrs.contains_key("null");
                    let explicit_list = tag_attrs.contains_key("list");
                    let explicit_separator = tag_attrs.contains_key("separator");

                    let value_raw = tag_attrs.get("value").cloned().unwrap_or_default();
                    let cfsqltype = tag_attrs.get("cfsqltype").cloned()
                        .unwrap_or_else(|| "cf_sql_varchar".to_string());
                    let null = tag_attrs.get("null")
                        .map(|v| v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
                        .unwrap_or(false);
                    let list = tag_attrs.get("list")
                        .map(|v| v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
                        .unwrap_or(false);
                    let separator = tag_attrs.get("separator").cloned().unwrap_or_else(|| ",".to_string());

                    // attributeCollection="#expr#" — bare script expression that
                    // evaluates to a struct of param attributes.
                    let attribute_collection_expr = tag_attrs.get("attributecollection").and_then(|raw| {
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            return None;
                        }
                        let stripped = strip_hashes(trimmed);
                        if stripped != trimmed {
                            Some(stripped)
                        } else {
                            // Bare identifier (no hashes) — treat as variable reference.
                            Some(trimmed.to_string())
                        }
                    });

                    // Convert value to script expression
                    let value_expr = if null {
                        "\"\"".to_string()
                    } else {
                        let stripped = strip_hashes(&value_raw);
                        if stripped != value_raw {
                            // Had hashes — it's a variable reference
                            stripped
                        } else if value_raw.is_empty() {
                            "\"\"".to_string()
                        } else {
                            // Literal string value
                            format!("\"{}\"", value_raw.replace('"', "\\\""))
                        }
                    };

                    params.push(CfQueryParam {
                        value_expr,
                        cfsqltype,
                        null,
                        list,
                        separator,
                        attribute_collection_expr,
                        explicit_value,
                        explicit_cfsqltype,
                        explicit_null,
                        explicit_list,
                        explicit_separator,
                    });

                    // Replace with ? placeholder
                    result.push('?');

                    // Skip to end of <cfqueryparam> tag
                    while i < len && chars[i] != '>' {
                        i += 1;
                    }
                    if i < len {
                        i += 1; // skip >
                    }
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    (result, params)
}

/// Process hash expressions in SQL for cfquery.
/// Converts #var# to string concatenation: `"..." & var & "..."`
/// Returns a script expression that builds the final SQL string.
fn process_sql_hashes(sql: &str) -> String {
    let sql = sql.trim().replace('\n', " ").replace('\r', "");

    if !sql.contains('#') {
        // No hash expressions — simple string literal
        return format!("\"{}\"", sql.replace('"', "\\\""));
    }

    // Split on hash pairs and build concatenation
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut current_text = String::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '#' {
            // Look for closing #
            if let Some(end_offset) = chars[i + 1..].iter().position(|&c| c == '#') {
                let end = i + 1 + end_offset;
                // Flush current text
                if !current_text.is_empty() {
                    parts.push(format!("\"{}\"", current_text.replace('"', "\\\"")));
                    current_text = String::new();
                }
                // Extract expression
                let expr: String = chars[i + 1..end].iter().collect();
                parts.push(expr);
                i = end + 1;
                continue;
            }
        }
        current_text.push(chars[i]);
        i += 1;
    }

    // Flush remaining text
    if !current_text.is_empty() {
        parts.push(format!("\"{}\"", current_text.replace('"', "\\\"")));
    }

    if parts.len() == 1 {
        parts.into_iter().next().unwrap()
    } else {
        parts.join(" & ")
    }
}

/// Parse <cfhttpparam> tags from the body of a <cfhttp> tag.
/// Returns a vector of struct literal strings like: { type: "header", name: "X-Custom", value: "foo" }
fn parse_cfhttpparam_tags(body: &str) -> Vec<String> {
    let mut params = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 13 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 13, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfhttpparam") {
                let next_after = chars.get(i + 12);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    let name_end = i + 12;
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    let mut parts = Vec::new();
                    if let Some(t) = tag_attrs.get("type") {
                        parts.push(format!("type: \"{}\"", t.to_lowercase()));
                    }
                    if let Some(n) = tag_attrs.get("name") {
                        let stripped = strip_hashes(n);
                        if stripped != *n {
                            parts.push(format!("name: {}", stripped));
                        } else {
                            parts.push(format!("name: \"{}\"", n));
                        }
                    }
                    if let Some(v) = tag_attrs.get("value") {
                        let stripped = strip_hashes(v);
                        if stripped != *v {
                            parts.push(format!("value: {}", stripped));
                        } else {
                            parts.push(format!("value: \"{}\"", v));
                        }
                    }
                    if let Some(f) = tag_attrs.get("file") {
                        let stripped = strip_hashes(f);
                        if stripped != *f {
                            parts.push(format!("file: {}", stripped));
                        } else {
                            parts.push(format!("file: \"{}\"", f));
                        }
                    }
                    if let Some(e) = tag_attrs.get("encoded") {
                        parts.push(format!("encoded: \"{}\"", e));
                    }
                    if let Some(m) = tag_attrs.get("mimetype") {
                        parts.push(format!("mimeType: \"{}\"", m));
                    }

                    params.push(format!("{{ {} }}", parts.join(", ")));

                    // Skip to end of tag
                    while i < len && chars[i] != '>' {
                        i += 1;
                    }
                    if i < len { i += 1; }
                    continue;
                }
            }
        }
        i += 1;
    }

    params
}

/// Parse <cfmailparam> tags from the body of a <cfmail> tag.
/// Returns a vector of struct literal strings like: { name: "X-Header", value: "foo" }
fn parse_cfmailparam_tags(body: &str) -> Vec<String> {
    let mut params = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 12 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 13, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfmailparam") {
                let next_after = chars.get(i + 12);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    let name_end = i + 12;
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    let mut parts = Vec::new();
                    if let Some(n) = tag_attrs.get("name") {
                        let stripped = strip_hashes(n);
                        if stripped != *n {
                            parts.push(format!("name: {}", stripped));
                        } else {
                            parts.push(format!("name: \"{}\"", n));
                        }
                    }
                    if let Some(v) = tag_attrs.get("value") {
                        let stripped = strip_hashes(v);
                        if stripped != *v {
                            parts.push(format!("value: {}", stripped));
                        } else {
                            parts.push(format!("value: \"{}\"", v));
                        }
                    }
                    if let Some(f) = tag_attrs.get("file") {
                        let stripped = strip_hashes(f);
                        if stripped != *f {
                            parts.push(format!("file: {}", stripped));
                        } else {
                            parts.push(format!("file: \"{}\"", f));
                        }
                    }
                    if let Some(t) = tag_attrs.get("type") {
                        parts.push(format!("type: \"{}\"", t.to_lowercase()));
                    }
                    if let Some(d) = tag_attrs.get("disposition") {
                        parts.push(format!("disposition: \"{}\"", d));
                    }

                    params.push(format!("{{ {} }}", parts.join(", ")));

                    while i < len && chars[i] != '>' { i += 1; }
                    if i < len { i += 1; }
                    continue;
                }
            }
        }
        i += 1;
    }

    params
}

/// Parse <cfmailpart> tags from the body of a <cfmail> tag.
/// Returns (parts_vec, remaining_body_text) where remaining_body_text has child tags stripped.
fn parse_cfmailpart_tags(body: &str) -> (Vec<String>, String) {
    let mut parts = Vec::new();
    let mut remaining = body.to_string();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut remove_ranges: Vec<(usize, usize)> = Vec::new();

    while i < len {
        if i + 11 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 12, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfmailpart") {
                let next_after = chars.get(i + 11);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    let tag_start = i;
                    let name_end = i + 11;
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    // Find > to get start of body content
                    while i < len && chars[i] != '>' { i += 1; }
                    if i < len { i += 1; }
                    let content_start = i;

                    // Find closing </cfmailpart>
                    let close_tag = "</cfmailpart>";
                    let mut found_close = false;
                    let close_len = close_tag.len();
                    while i + close_len <= len {
                        let slice: String = chars[i..i + close_len].iter().collect();
                        if slice.to_lowercase() == close_tag {
                            let content: String = chars[content_start..i].iter().collect();
                            let mut part_parts = Vec::new();
                            if let Some(t) = tag_attrs.get("type") {
                                part_parts.push(format!("type: \"{}\"", t.to_lowercase()));
                            }
                            if let Some(c) = tag_attrs.get("charset") {
                                part_parts.push(format!("charset: \"{}\"", c));
                            }
                            let content_trimmed = content.trim();
                            part_parts.push(format!("body: \"{}\"", content_trimmed.replace('"', "\\\"").replace('\n', "\\n")));
                            parts.push(format!("{{ {} }}", part_parts.join(", ")));

                            i += close_len;
                            // Skip past >
                            while i < len && chars[i] != '>' { i += 1; }
                            if i < len { i += 1; }
                            remove_ranges.push((tag_start, i));
                            found_close = true;
                            break;
                        }
                        i += 1;
                    }
                    if found_close { continue; }
                }
            }
            // Also remove <cfmailparam> tags from remaining body
            if ahead.to_lowercase().starts_with("<cfmailparam") {
                let tag_start = i;
                while i < len && chars[i] != '>' { i += 1; }
                if i < len { i += 1; }
                remove_ranges.push((tag_start, i));
                continue;
            }
        }
        i += 1;
    }

    // Remove child tag ranges from remaining body (reverse order to preserve indices)
    for (start, end) in remove_ranges.iter().rev() {
        let start_byte = chars[..*start].iter().collect::<String>().len();
        let end_byte = chars[..*end].iter().collect::<String>().len();
        remaining.replace_range(start_byte..end_byte, "");
    }

    (parts, remaining)
}

/// Represents a stored procedure parameter
struct ProcParam {
    value: Option<String>,
    cfsqltype: Option<String>,
}

/// Parse <cfprocparam> tags from the body of a <cfstoredproc> tag.
fn parse_cfprocparam_tags(body: &str) -> Vec<ProcParam> {
    let mut params = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 12 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 13, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfprocparam") {
                let next_after = chars.get(i + 12);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    let name_end = i + 12;
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    params.push(ProcParam {
                        value: tag_attrs.get("value").cloned(),
                        cfsqltype: tag_attrs.get("cfsqltype").cloned(),
                    });

                    while i < len && chars[i] != '>' { i += 1; }
                    if i < len { i += 1; }
                    continue;
                }
            }
        }
        i += 1;
    }

    params
}

/// Parse <cfprocresult> tags from the body of a <cfstoredproc> tag.
/// Returns Vec<(result_variable_name, resultset_number)>
fn parse_cfprocresult_tags(body: &str) -> Vec<(String, usize)> {
    let mut results = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 13 < len && chars[i] == '<' {
            let ahead: String = chars[i..std::cmp::min(i + 14, len)].iter().collect();
            if ahead.to_lowercase().starts_with("<cfprocresult") {
                let next_after = chars.get(i + 13);
                if next_after == Some(&' ') || next_after == Some(&'>') || next_after == Some(&'/') || next_after == Some(&'\t') || next_after == Some(&'\n') {
                    let name_end = i + 13;
                    let (tag_attrs, _, _) = parse_tag_attributes(&chars, name_end, len);

                    let name = tag_attrs.get("name").cloned().unwrap_or_else(|| "cfresult".to_string());
                    let resultset: usize = tag_attrs.get("resultset")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    results.push((name, resultset));

                    while i < len && chars[i] != '>' { i += 1; }
                    if i < len { i += 1; }
                    continue;
                }
            }
        }
        i += 1;
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfset() {
        let input = "<cfset x = 5>";
        assert!(has_cfml_tags(input));
        let result = tags_to_script(input);
        assert!(result.contains("x = 5"));
    }

    #[test]
    fn test_cfif() {
        let input = "<cfif x GT 5>yes</cfif>";
        let result = tags_to_script(input);
        assert!(result.contains("if (x GT 5)"));
    }

    #[test]
    fn test_cfoutput_hash() {
        let input = "<cfoutput>#name#</cfoutput>";
        let result = tags_to_script(input);
        assert!(result.contains("writeOutput(name)"));
    }

    #[test]
    fn test_cfloop_index() {
        let input = r#"<cfloop from="1" to="10" index="i">body</cfloop>"#;
        let result = tags_to_script(input);
        assert!(result.contains("for (var i = 1; i <= 10; i = i + 1)"));
    }
}
