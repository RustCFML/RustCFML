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
    // Scan for a real CFML tag — `<cf...>`, `</cf...>`, or a `<!--- --->`
    // comment — while skipping CFScript comments (`/* */`, `//`) and quoted
    // strings. A `<cf...>` that appears ONLY inside a script comment or string
    // (e.g. a doc comment in a script-component body, issue #69) must NOT force
    // the whole file through the template tag preprocessor, which would mangle
    // the component into __writeText() echoes. Template extensions (.cfm/.css/
    // …) are tag-parsed unconditionally by callers regardless of this result,
    // so skipping script comments/strings here only affects script files (.cfc).
    let b = source.as_bytes();
    let n = b.len();
    // `<cf` / `</cf` at byte i, case-insensitive on the c/f.
    let is_cf_at = |i: usize| -> bool {
        if i + 2 >= n || b[i] != b'<' {
            return false;
        }
        let (c1, c2) = if b[i + 1] == b'/' {
            if i + 3 >= n {
                return false;
            }
            (b[i + 2], b[i + 3])
        } else {
            (b[i + 1], b[i + 2])
        };
        c1.eq_ignore_ascii_case(&b'c') && c2.eq_ignore_ascii_case(&b'f')
    };
    let mut i = 0;
    while i < n {
        let c = b[i];
        // Block comment /* ... */
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Line comment // ... to end of line
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            i += 2;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Quoted string ("" / '' escape by doubling the quote)
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < n {
                if b[i] == c {
                    if i + 1 < n && b[i + 1] == c {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // A `<!--- --->` CFML comment must NOT force tag mode. It is a valid
        // comment inside a script-based `.cfc` body (Preside preside-objects
        // use `<!--- properties --->` markers), and the CFScript lexer strips
        // it. Skip the whole comment so neither it, nor a `<cf...>` tag that is
        // commented out *inside* it, triggers tag preprocessing.
        if c == b'<'
            && i + 4 < n
            && b[i + 1] == b'!'
            && b[i + 2] == b'-'
            && b[i + 3] == b'-'
            && b[i + 4] == b'-'
        {
            // CFML comments nest — track depth so an inner `<!--- … --->` does
            // not end the outer comment early (which could expose a commented-out
            // `<cf…>` and wrongly force tag mode).
            i += 5;
            let mut depth = 1usize;
            while i < n && depth > 0 {
                if i + 4 < n
                    && b[i] == b'<'
                    && b[i + 1] == b'!'
                    && b[i + 2] == b'-'
                    && b[i + 3] == b'-'
                    && b[i + 4] == b'-'
                {
                    depth += 1;
                    i += 5;
                } else if i + 2 < n && b[i] == b'-' && b[i + 1] == b'-' && b[i + 2] == b'>' {
                    depth -= 1;
                    i += 3;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'<' && is_cf_at(i) {
            return true;
        }
        i += 1;
    }
    false
}

thread_local! {
    /// Structural error recorded during the most recent preprocess pass — e.g.
    /// an unclosed `<cfscript>` (no `</cfscript>` before EOF). Lucee/ACF reject
    /// these at compile time; we record the first one here so the strict entry
    /// point ([`tags_to_script_checked`]) can surface it as a compile error
    /// instead of silently echoing the unterminated body as literal output.
    static PREPROCESS_ERROR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Active `<cfoutput encodeFor="...">` encodings, innermost last.
    ///
    /// `encodeFor` is an XSS control: every `#expr#` interpolated inside the tag
    /// body is passed through `encodeFor(type, value)`. It was parsed and then
    /// dropped — the attribute appeared nowhere in the workspace — so pages that
    /// asked for auto-encoding emitted raw, unescaped markup and the protection
    /// silently did not exist.
    ///
    /// A stack rather than a single value so nested tags inherit the enclosing
    /// encoding (verified on Lucee 7.0.4: `#x#` inside a `<cfif>` nested in an
    /// `encodeFor` body IS encoded), and so an inner `<cfoutput>` restores the
    /// outer encoding when it closes.
    static ENCODE_FOR: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The innermost active `encodeFor` type, if any.
fn current_encode_for() -> Option<String> {
    ENCODE_FOR.with(|e| e.borrow().last().cloned())
}

/// Pops the `ENCODE_FOR` stack when a `<cfoutput encodeFor=…>` arm goes out of
/// scope. A guard rather than an explicit pop because that arm has several
/// early returns (the `query=` and `group=` forms), and a missed pop would leak
/// the encoding onto sibling output later in the same file.
struct EncodeForGuard {
    active: bool,
}

impl Drop for EncodeForGuard {
    fn drop(&mut self) {
        if self.active {
            ENCODE_FOR.with(|e| {
                e.borrow_mut().pop();
            });
        }
    }
}

/// Record the first structural preprocess error of the current pass.
fn record_preprocess_error(msg: impl Into<String>) {
    PREPROCESS_ERROR.with(|e| {
        let mut slot = e.borrow_mut();
        if slot.is_none() {
            *slot = Some(msg.into());
        }
    });
}

/// Convert CFML tag-based source code to equivalent CFScript.
///
/// Tolerant: structural errors (e.g. an unclosed `<cfscript>`) degrade rather
/// than fail. Prefer [`tags_to_script_checked`] on compile paths so those
/// errors surface the way Lucee/ACF report them.
pub fn tags_to_script(source: &str) -> String {
    // Strip a leading UTF-8 BOM. Without this it is treated as literal text
    // before the first tag and emitted into page output (Lucee/ACF strip it).
    // Preside/ColdBox .cfc files ship with a BOM, so the marker would otherwise
    // surface in every rendered response.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut imports = std::collections::HashMap::<String, String>::new();
    PREPROCESS_ERROR.with(|e| *e.borrow_mut() = None);
    tags_to_script_impl(source, &mut imports)
}

/// Strict variant of [`tags_to_script`]: returns `Err(message)` when the source
/// has a structural tag error (an unterminated body tag such as a `<cfscript>`
/// with no `</cfscript>`). Compile paths should use this so a missing close tag
/// is a clear compile error rather than the body being emitted as literal text.
pub fn tags_to_script_checked(source: &str) -> Result<String, String> {
    let out = tags_to_script(source);
    match PREPROCESS_ERROR.with(|e| e.borrow_mut().take()) {
        Some(msg) => Err(msg),
        None => Ok(out),
    }
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
            // Find the closing `--->`. CFML comments NEST (unlike HTML): the
            // comment `<!--- a <!--- b ---> c --->` is a SINGLE comment. Track
            // depth so an inner `<!--- … --->` doesn't terminate the outer one
            // early. Mura/Masa comment out whole `<cftry>/<cfcatch>` blocks that
            // themselves contain `<!--- … --->` notes; a non-nesting scan ended
            // the comment at the inner `--->` and left the tail (`</cftry>` etc.)
            // as stray active code, corrupting the surrounding cflock/brace
            // structure ("Expected RParen, found __lock_e").
            let mut j = i + 5;
            let mut depth = 1usize;
            while j < len {
                if j + 4 < len
                    && chars[j] == '<'
                    && chars[j + 1] == '!'
                    && chars[j + 2] == '-'
                    && chars[j + 3] == '-'
                    && chars[j + 4] == '-'
                {
                    depth += 1;
                    j += 5;
                    continue;
                }
                if j + 2 < len && chars[j] == '-' && chars[j + 1] == '-' && chars[j + 2] == '>' {
                    depth -= 1;
                    j += 3;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                j += 1;
            }
            if depth > 0 {
                j = len; // unclosed comment, skip to end
            }
            // Preserve the newlines that lived inside the comment so downstream
            // line numbers still line up with the source. Dropping the comment
            // outright (a multi-line `<!----- … ----->` banner is common) shifted
            // every subsequent tag up by the comment's height, so parse errors
            // were reported on the wrong line (GitHub #226).
            for c in &chars[i..j] {
                if *c == '\n' {
                    result.push('\n');
                }
            }
            i = j;
            continue;
        }
        if i < len - 1 && chars[i] == '<' && is_cf_tag_start(&chars, i, len) {
            let (script, consumed) = parse_cf_tag(&chars, i, len, imports, in_cfoutput);
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
                // Under `<cfoutput encodeFor="…">` every interpolated value is
                // encoded; literal body text is NOT (matching Lucee — literal
                // `<b>` in the body stays markup, only `#expr#` is escaped).
                match current_encode_for() {
                    Some(kind) => result.push_str(&format!(
                        "writeOutput(encodeFor(\"{}\", {}));",
                        kind, expr
                    )),
                    None => result.push_str(&format!("writeOutput({});", expr)),
                }
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
            // Structural gap suppression: whitespace between </cfcatch> (or the
            // try body) and <cffinally>/<cfcatch> would otherwise be emitted as
            // __writeText(...) right at the `} finally {` / `} catch {` junction,
            // producing "} __writeText(...); finally {" or "} __writeText(...);
            // catch {" — a parse error, because a statement can't sit between a
            // try/catch and its following catch/finally clause (the clause is
            // orphaned: "Expected RParen, found Identifier cfcatch"). CFML treats
            // this inter-tag whitespace as structural, not output, so drop a
            // whitespace-only node when the next tag is <cffinally> or <cfcatch>.
            // (Masa CMS core/setup/inc/_process.cfm chains two <cfcatch> blocks.)
            let next_is_catch_junction = {
                let peek: String = chars[i..std::cmp::min(i + 10, len)].iter().collect();
                let peek = peek.to_lowercase();
                peek.starts_with("<cffinally") || peek.starts_with("<cfcatch")
            };
            if next_is_catch_junction && text.trim().is_empty() {
                // suppress
            } else if !text.is_empty() {
                // Output ALL text including whitespace-only nodes.
                // Standard CFML outputs everything; whitespace suppression is opt-in
                // via cfprocessingdirective or cfsetting enableCFOutputOnly.
                // Outside <cfoutput>: use __writeText (suppressible by enableCFOutputOnly).
                let fn_name = if in_cfoutput { "writeOutput" } else { "__writeText" };
                // CFML string literals don't use backslash escapes and may span
                // multiple lines, so only the double-quote needs escaping (by
                // doubling). Backslashes and real newlines pass through verbatim;
                // the script lexer reads them literally.
                let mut escaped = text.replace('"', "\"\"");
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
    let path_expr = format!("\"{}\"", escape_for_string_literal(&path));

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
        // XML-style self-closing custom tags still run the end phase.
        let run_end = is_self_closing_tag(chars, tag_end);
        let result = format!("__cfcustomtag({}, {}, {});\n", path_expr, attrs_expr, run_end);
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
        // Skip nested string literals so that `#` characters living inside a
        // quoted string (which are themselves nested interpolations, e.g.
        // `#"line #tc.line#"#`) are not mistaken for the closing delimiter.
        if chars[i] == '"' || chars[i] == '\'' {
            i = skip_cfml_string(chars, i, len);
            continue;
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

fn parse_cf_tag(chars: &[char], start: usize, len: usize, imports: &mut std::collections::HashMap<String, String>, in_cfoutput: bool) -> (String, usize) {
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
            // Lucee ignores CFML comments (`<!--- ... --->`) embedded inside a
            // tag-mode expression body — e.g. section comments inside a
            // multi-line array/struct literal. Strip them before the script
            // parser sees the body, or the leftover `<!---` mis-parses.
            let raw = strip_cfml_comments(&raw);
            let body = raw.trim();
            let body = if body.ends_with('/') {
                body[..body.len() - 1].trim()
            } else {
                body
            };
            let body = strip_hashes(body);
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
    let (attrs, quoted, attr_order, tag_end) =
        parse_tag_attributes_ordered(chars, name_end, len);

    match tag_lower.as_str() {
        "cfoutput" => {
            // <cfoutput> marks a region where # expressions are evaluated and
            // content is always output (even when enableCFOutputOnly is active).
            // Process body recursively with in_cfoutput=true so text uses writeOutput.
            let (body, consumed) = if let Some(end_tag_pos) =
                find_closing_tag(chars, tag_end, len, "cfoutput")
            {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                (body, close_end - start)
            } else {
                // No closing tag — treat remaining content as inside cfoutput
                let body: String = chars[tag_end..len].iter().collect();
                (body, len - start)
            };
            // `encodeFor="html|javascript|url|css|xml|htmlattribute|xmlattribute"`
            // auto-encodes every `#expr#` in the body. Push it for the duration
            // of the body walk so nested tags inherit it and the previous
            // encoding (if any) is restored on the way out.
            let encode_for = attrs
                .get("encodefor")
                .map(|v| strip_hashes(v).trim_matches(|c| c == '"' || c == '\'').to_lowercase())
                .filter(|v| !v.is_empty());
            if let Some(ref kind) = encode_for {
                ENCODE_FOR.with(|e| e.borrow_mut().push(kind.clone()));
            }
            let _encode_for_guard = EncodeForGuard {
                active: encode_for.is_some(),
            };
            match attrs.get("query") {
                None => (tags_to_script_inner(&body, imports, true), consumed),
                Some(query_raw) => {
                    let query = strip_hashes(query_raw);
                    if attrs.contains_key("group") {
                        // Grouped (control-break) output. The query rows are
                        // materialised into an array once, then iterated in
                        // group breaks: per-group body runs once, and a nested
                        // <cfoutput> (the detail block) iterates that group's
                        // rows. Nesting recurses for multi-level grouping.
                        let group_col = strip_hashes(attrs.get("group").unwrap());
                        let case_sensitive = group_case_sensitive(&attrs);
                        let q = format!("__cfg_q_{}", start);
                        let rows = format!("__cfg_rows_{}", start);
                        let rc = format!("__cfg_rc_{}", start);
                        let cl = format!("__cfg_cl_{}", start);
                        let ci = format!("__cfg_ci_{}", start);
                        let r = format!("__cfg_r_{}", start);
                        let group_loop = emit_grouped_query_body(
                            &rows, &query, &body, &group_col, case_sensitive, "cfoutput", true,
                            imports, start, 0,
                        );
                        (
                            format!(
                                "var {q} = {query};\nvar {rows} = [];\nvar {rc} = {q}.recordcount;\nvar {cl} = {q}.columnlist;\nvar {ci} = 0;\nfor (var {r} in {q}) {{\n{ci} = {ci} + 1;\n{r}.currentRow = {ci};\n{r}.recordCount = {rc};\n{r}.columnList = {cl};\narrayAppend({rows}, {r});\n}}\n{loop}{query} = {q};\n",
                                q = q,
                                query = query,
                                rows = rows,
                                rc = rc,
                                cl = cl,
                                ci = ci,
                                r = r,
                                loop = group_loop,
                            ),
                            consumed,
                        )
                    } else {
                        // <cfoutput query="q"> iterates the query's rows. Within
                        // the body, bare column refs (#name#) resolve to the
                        // current row (we merge the row struct into `variables`)
                        // and `#q.col#` resolves to the row scalar (we reassign
                        // the query var to the row, restoring it after the loop).
                        // `maxrows`/`startrow` bound the iteration.
                        let body_script = tags_to_script_inner(&body, imports, true);
                        let startrow = attrs
                            .get("startrow")
                            .map(|s| strip_hashes(s))
                            .unwrap_or_else(|| "1".to_string());
                        let maxrows = attrs
                            .get("maxrows")
                            .map(|s| strip_hashes(s))
                            .unwrap_or_else(|| "-1".to_string());
                        // Unique temp names per tag occurrence (mirrors cfloop):
                        // a leading bare `{` block is parsed as a struct literal,
                        // so we declare the temps inline rather than wrapping.
                        let q = format!("__cfoq_q_{}", start);
                        let rc = format!("__cfoq_rc_{}", start);
                        let cl = format!("__cfoq_cl_{}", start);
                        let sr = format!("__cfoq_sr_{}", start);
                        let mr = format!("__cfoq_mr_{}", start);
                        let i = format!("__cfoq_i_{}", start);
                        let row = format!("__cfoq_row_{}", start);
                        (
                            format!(
                                "var {q} = {query};\nvar {rc} = {q}.recordcount;\nvar {cl} = {q}.columnlist;\nvar {sr} = {startrow};\nvar {mr} = {maxrows};\nvar {i} = 0;\nfor (var {row} in {q}) {{\n{i} = {i} + 1;\nif ({i} < {sr}) {{ continue; }}\nif ({mr} >= 0 && {i} >= {sr} + {mr}) {{ break; }}\n{row}.currentRow = {i};\n{row}.recordCount = {rc};\n{row}.columnList = {cl};\nstructAppend(variables, {row}, true);\n{query} = {row};\n{body}\n}}\n{query} = {q};\n",
                                q = q,
                                rc = rc,
                                cl = cl,
                                sr = sr,
                                mr = mr,
                                i = i,
                                row = row,
                                query = query,
                                startrow = startrow,
                                maxrows = maxrows,
                                body = body_script,
                            ),
                            consumed,
                        )
                    }
                }
            }
        }
        "cfelse" => {
            ("} else {\n".to_string(), tag_end - start)
        }
        "cfloop" => {
            // A `<cfloop query="q">` with no index/item binding needs its whole
            // body captured (not just the opener) so the query variable can be
            // RESTORED after the loop. The loop reassigns `q` to the current row
            // each iteration (so `q.col` and the pseudo-columns work); without a
            // restore, `q` is left as the last row struct, which corrupts a
            // re-entrant/nested loop over the same variable (`for (row in <row
            // struct>)` iterates keys → non-struct rows). Other cfloop forms keep
            // the lightweight opener-only path (`</cfloop>` emits the closing `}`).
            let is_query_row_loop = attrs.contains_key("query")
                && !attrs.contains_key("index")
                && !attrs.contains_key("item");
            if is_query_row_loop {
                let query = strip_hashes(attrs.get("query").unwrap());
                let (body, consumed) = if let Some(end_tag_pos) =
                    find_closing_tag(chars, tag_end, len, "cfloop")
                {
                    let body: String = chars[tag_end..end_tag_pos].iter().collect();
                    let close_end = find_tag_end(chars, end_tag_pos, len);
                    (body, close_end - start)
                } else {
                    let body: String = chars[tag_end..len].iter().collect();
                    (body, len - start)
                };
                // Row-window attributes. `startrow`/`endrow` are cfloop's own;
                // Lucee also honours `maxrows` here (probed on 7.0.4), and an
                // out-of-range window simply yields no iterations rather than an
                // error. All three used to be discarded by this lowering, so
                // EVERY row was iterated — silently (docs known-issues §27).
                let startrow = attrs
                    .get("startrow")
                    .map(|s| strip_hashes(s))
                    .unwrap_or_else(|| "1".to_string());
                let endrow = attrs
                    .get("endrow")
                    .map(|s| strip_hashes(s))
                    .unwrap_or_else(|| "-1".to_string());
                let maxrows = attrs
                    .get("maxrows")
                    .map(|s| strip_hashes(s))
                    .unwrap_or_else(|| "-1".to_string());
                let sr = format!("__cfloopq_sr_{}", start);
                let er = format!("__cfloopq_er_{}", start);
                let q = format!("__cfloopq_q_{}", start);
                let rc = format!("__cfloopq_rc_{}", start);
                let i = format!("__cfloopq_i_{}", start);
                // The window preamble is shared by the plain and grouped forms:
                // `sr`..`er` is the 1-based row range to visit, clamped to the
                // recordset. Grouping applies AFTER the window (Lucee-probed:
                // `group="dept" startrow="3"` groups only rows 3+).
                let window = format!(
                    "var {q} = {query};\nvar {rc} = {q}.recordcount;\nvar {sr} = {startrow};\nif ({sr} < 1) {{ {sr} = 1; }}\nvar {er} = {endrow};\nif ({er} < 0 || {er} > {rc}) {{ {er} = {rc}; }}\nvar {mrv} = {maxrows};\nif ({mrv} >= 0 && {sr} + {mrv} - 1 < {er}) {{ {er} = {sr} + {mrv} - 1; }}\n",
                    q = q,
                    query = query,
                    rc = rc,
                    sr = sr,
                    er = er,
                    startrow = startrow,
                    endrow = endrow,
                    mrv = format!("__cfloopq_mr_{}", start),
                    maxrows = maxrows,
                );
                if let Some(group_raw) = attrs.get("group") {
                    // Grouped (control-break) iteration, the same model the
                    // grouped `<cfoutput query>` uses: materialise the windowed
                    // rows, then break on the group column. A BARE nested
                    // `<cfloop>` is the detail block over the current group.
                    let group_col = strip_hashes(group_raw);
                    let case_sensitive = group_case_sensitive(&attrs);
                    let rows = format!("__cfloopq_rows_{}", start);
                    let cl = format!("__cfloopq_cl_{}", start);
                    let r = format!("__cfloopq_r_{}", start);
                    let group_loop = emit_grouped_query_body(
                        &rows, &query, &body, &group_col, case_sensitive, "cfloop", in_cfoutput,
                        imports, start, 0,
                    );
                    return (
                        format!(
                            "{window}var {cl} = {q}.columnlist;\nvar {rows} = [];\nvar {i} = 0;\nfor (var {r} in {q}) {{\n{i} = {i} + 1;\nif ({i} < {sr}) {{ continue; }}\nif ({i} > {er}) {{ break; }}\n{r}.currentRow = {i};\n{r}.recordCount = {rc};\n{r}.columnList = {cl};\narrayAppend({rows}, {r});\n}}\n{loop}{query} = {q};\n",
                            window = window,
                            cl = cl,
                            q = q,
                            rows = rows,
                            i = i,
                            r = r,
                            sr = sr,
                            er = er,
                            rc = rc,
                            loop = group_loop,
                            query = query,
                        ),
                        consumed,
                    );
                }
                let body_script = tags_to_script_inner(&body, imports, in_cfoutput);
                // Cursor model (Lucee/ACF): the query variable STAYS the query;
                // each iteration moves the query's current-row cursor via the
                // internal __querySetRow helper, so `q.col` reads the current
                // row, `q.currentRow` reports the position, and `q` can still be
                // passed to functions that expect the whole query. The cursor is
                // reset to row 1 after the loop.
                (
                    format!(
                        "{window}for (var {i} = {sr}; {i} <= {er}; {i} = {i} + 1) {{\n__querySetRow({q}, {i});\n{query} = {q};\n{body}\n}}\n__querySetRow({q}, 1);\n{query} = {q};\n",
                        window = window,
                        q = q,
                        sr = sr,
                        er = er,
                        i = i,
                        query = query,
                        body = body_script,
                    ),
                    consumed,
                )
            } else {
                parse_cfloop_tag(&attrs, tag_end - start, start)
            }
        }
        "cfscript" => {
            // Everything between <cfscript> and </cfscript> is raw script
            // Find the closing </cfscript>
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfscript") {
                let script: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                (script, close_end - start)
            } else {
                // No </cfscript> before EOF. Lucee/ACF reject this at compile time;
                // record it so the strict entry point surfaces a compile error
                // rather than silently emitting the unterminated body as text.
                record_preprocess_error("Unclosed <cfscript> tag: missing </cfscript>");
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
            sig.push_str(&format!("function {}({})", name, param_names.join(", ")));
            // Carry the `output` attribute through as a script-function attribute so
            // the parser records it in the function metadata. `output="false"` must
            // suppress the body's output (whitespace/text/cfoutput) at runtime — a
            // tag `<cffunction output="false">` otherwise leaked its inter-tag
            // whitespace into the caller (breaking Masa CMS admin inline JS). No
            // attribute (or output="true") is left off → body text still emits,
            // matching Lucee.
            if let Some(output) = attrs.get("output") {
                sig.push_str(&format!(" output=\"{}\"", output));
            }
            sig.push_str(" {\n");
            (sig, tag_end - start)
        }
        "cfargument" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            if let Some(raw) = attrs.get("default") {
                // Mirror cfparam: a quoted default interpolates `#...#` segments
                // while preserving literal text; a bare `#expr#` keeps its native
                // type; an unquoted default falls back to the expr-or-literal
                // heuristic.
                let def_val = if quoted.contains("default") {
                    match single_hash_expr(raw) {
                        Some(expr) => format!("({})", expr),
                        None => format_attr_value(raw, true),
                    }
                } else {
                    format_attr_value(raw, false)
                };
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
            // Forward var plus the common option attributes (label/expand/top)
            // as named args so writeDump's intercept honours them — matches the
            // script form writeDump(var=…, label=…, expand=…, top=…).
            let attr_expr = |key: &str| -> Option<String> {
                attrs.get(key).map(|raw| {
                    if quoted.contains(key) {
                        match single_hash_expr(raw) {
                            Some(expr) => format!("({})", expr),
                            None => format_attr_value(raw, true),
                        }
                    } else {
                        format_attr_value(raw, false)
                    }
                })
            };
            // `var` must go through the same attribute-value handling as the
            // other attrs: a pure `#expr#` keeps the expression's native type,
            // but a literal string that merely CONTAINS interpolation
            // (`var="An error happened: #x#"`, as Mura/Masa's settingsBundleBean
            // cfdump uses) must be emitted as a proper quoted/concatenated
            // string. `strip_hashes` produced a bare unquoted `An error ... x`
            // that failed to parse.
            let var = attr_expr("var").unwrap_or_else(|| "\"\"".to_string());
            let mut call_args = format!("var={}", var);
            // Every attribute other than `var` rides through as a named arg.
            // This was a three-key list (label/expand/top), so `output=` (console
            // / a file path) and `abort=` were discarded at compile time —
            // `<cfdump output="console">` wrote into the HTTP response anyway and
            // `abort="true"` never stopped the request (docs known-issues §27).
            // Source order keeps codegen deterministic (`attrs` is a HashMap).
            for key in &attr_order {
                if key == "var" {
                    continue;
                }
                if let Some(expr) = attr_expr(key) {
                    call_args.push_str(&format!(", {}={}", key, expr));
                }
            }
            (format!("writeDump({});\n", call_args), tag_end - start)
        }
        "cfthrow" => {
            // Route each attribute value through format_attr_value so `#...#`
            // segments interpolate while literal text before/between/after is
            // preserved — uniform with cfparam (Lucee parity). A quoted value
            // that is exactly one `#expr#` keeps the expression's native type.
            let attr_expr = |key: &str, default_lit: &str| -> String {
                match attrs.get(key) {
                    Some(raw) if quoted.contains(key) => match single_hash_expr(raw) {
                        Some(expr) => format!("({})", expr),
                        None => format_attr_value(raw, true),
                    },
                    Some(raw) => format_attr_value(raw, false),
                    None => format!("\"{}\"", default_lit),
                }
            };
            // `<cfthrow object="#e#">` re-throws an existing exception object and
            // `extendedInfo` is a first-class attribute — both were previously
            // dropped (only message/type/detail/errorcode were forwarded), so
            // `cfthrow object=` lost the original error (GitHub issue #158). When
            // no message is given default to empty (not "An error occurred") so an
            // object/extendedInfo-only throw doesn't fabricate a message.
            // When re-throwing an object, unspecified attributes must NOT inject
            // literal defaults (e.g. type="Application") — that would clobber the
            // object's own fields. Default to empty so the object's values survive;
            // an explicitly supplied attribute still overrides.
            let has_object = attrs.contains_key("object");
            let dflt_msg = if has_object { "" } else { "An error occurred" };
            let dflt_type = if has_object { "" } else { "Application" };
            let message = attr_expr("message", dflt_msg);
            let type_ = attr_expr("type", dflt_type);
            let detail = attr_expr("detail", "");
            let errorcode = attr_expr("errorcode", "");
            let extendedinfo = attr_expr("extendedinfo", "");
            let object = if has_object { attr_expr("object", "") } else { "\"\"".to_string() };

            (
                format!(
                    "throw({}, {}, {}, {}, {}, {});\n",
                    message, type_, detail, errorcode, extendedinfo, object
                ),
                tag_end - start,
            )
        }
        "cftry" => {
            ("try {\n".to_string(), tag_end - start)
        }
        "cfcatch" => {
            let catch_type = attrs.get("type").cloned().unwrap_or("any".to_string());
            // A `<cfcatch>` opens `catch (T cfcatch) {`, but the preceding block
            // must be closed first. For the FIRST catch that block is the `try {`
            // body, so we emit the leading `}`. For a CHAINED catch (multiple
            // `<cfcatch>` on one `<cftry>`, e.g. `type="database"` then
            // `type="any"`) the preceding `</cfcatch>` already emitted its own
            // closing `}`, so a leading `}` here would DOUBLE-close and orphan the
            // `catch` (parser: "Expected RParen, found Identifier cfcatch").
            // Mirror the `<cffinally>` look-back: scan back over whitespace for a
            // preceding `</cfcatch>` and drop the leading brace in that case.
            // (Masa CMS core/setup/inc/_process.cfm has a two-catch try.)
            let mut j = start;
            while j > 0 && chars[j - 1].is_whitespace() {
                j -= 1;
            }
            let preceded_by_catch = j > 0 && chars[j - 1] == '>' && {
                let mut k = j - 1;
                while k > 0 && chars[k - 1] != '<' {
                    k -= 1;
                }
                let tag: String = chars[k.saturating_sub(1)..j]
                    .iter()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                tag.eq_ignore_ascii_case("</cfcatch>")
            };
            let prefix = if preceded_by_catch { "" } else { "}\n" };
            (format!("{}catch ({} cfcatch) {{\n", prefix, catch_type), tag_end - start)
        }
        "cfabort" => {
            // `<cfabort showError="msg">` raises a CATCHABLE error that is routed
            // through Application.cfc::onError — NOT onAbort (Adobe/Lucee parity).
            // Plain `<cfabort>` unwinds silently and fires onAbort instead.
            match attrs.get("showerror") {
                Some(raw) => (
                    format!("__cfabort({});\n", format_attr_value(raw, quoted.contains("showerror"))),
                    tag_end - start,
                ),
                None => ("__cfabort();\n".to_string(), tag_end - start),
            }
        }
        "cfparam" => {
            let name = attrs.get("name").cloned().unwrap_or_default();
            // A quoted default is ALWAYS a literal string in CFML (Lucee parity):
            // default="px-5 py-5", default="1+1", default="now()" all stay literal
            // strings; only #expr# segments interpolate. Unquoted defaults fall back
            // to the expression-or-literal heuristic. format_attr_value handles both,
            // using the quote flag the attribute parser already tracked. The one
            // exception: a value that is EXACTLY a single #expr# (e.g. "#[]#") keeps
            // the expression's native type (array/struct/number), so emit it bare
            // rather than as a string concatenation.
            let default = match attrs.get("default") {
                Some(raw) if quoted.contains("default") => match single_hash_expr(raw) {
                    Some(expr) => format!("({})", expr),
                    None => format_attr_value(raw, true),
                },
                Some(raw) => format_attr_value(raw, false),
                None => "\"\"".to_string(),
            };
            // Clean name - remove scope prefix quotes and strip hash expressions
            let clean_name = strip_hashes(&name.replace('"', "").replace('\'', ""));
            let mut script =
                format!("if (isNull({})) {{ {} = {}; }}\n", clean_name, clean_name, default);
            // Enforce the `type` attribute (with optional min/max/pattern).
            // Previously these were parsed and silently dropped.
            if let Some(type_attr) = attrs.get("type") {
                let ty = type_attr.to_lowercase();
                if !ty.is_empty() && ty != "any" {
                    let min = attrs
                        .get("min")
                        .map(|s| strip_hashes(s))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "\"\"".to_string());
                    let max = attrs
                        .get("max")
                        .map(|s| strip_hashes(s))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "\"\"".to_string());
                    // CFML strings don't treat backslash as an escape; only the
                    // double-quote needs doubling for a quoted literal.
                    let pattern = attrs
                        .get("pattern")
                        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
                        .unwrap_or_else(|| "\"\"".to_string());
                    script.push_str(&format!(
                        "__cfparam_validate({val}, \"{ty}\", \"{nm}\", {min}, {max}, {pat});\n",
                        val = clean_name,
                        ty = ty,
                        nm = clean_name,
                        min = min,
                        max = max,
                        pat = pattern,
                    ));
                }
            }
            (script, tag_end - start)
        }
        "cfcomponent" => {
            let extends = attrs.get("extends").cloned();
            let implements = attrs.get("implements").cloned();
            // Do NOT emit the `name` attribute. Lucee/ACF derive a component's name
            // from its FILENAME and ignore any `<cfcomponent name="…">` attribute
            // (verified: `getMetaData().name` returns the filename, not the attr).
            // Emitting it as a bareword (`component i18n …`) stranded the name
            // before the `{` when a keyword attribute like `output="false"`
            // followed (Preside cbi18n `<cfcomponent name="i18n" … singleton>` →
            // "Expected LBrace"); emitting it as `name="…"` wrongly overrode the
            // filename-derived __name and broke path resolution. Dropping it is the
            // Lucee-faithful behaviour — the `name` key is skipped in the
            // pass-through loop below too.
            let mut decl = "component ".to_string();
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
            let extends = attrs.get("extends").cloned();
            // Drop the `name` attribute — name is filename-derived; see cfcomponent.
            let mut decl = "interface ".to_string();
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
            // Output name first, then all other attributes in SOURCE ORDER.
            // Order matters: cfproperty attributes are preserved into component
            // metadata and feed identity hashes (Preside FK constraint names are
            // Hash(SerializeJson(property))), so HashMap iteration order would
            // make those hashes non-deterministic and diverge from Lucee.
            if let Some(name) = attrs.get("name") {
                prop_str.push_str(&format!("name=\"{}\" ", name));
            }
            for k in &attr_order {
                if k != "name" {
                    if let Some(v) = attrs.get(k) {
                        prop_str.push_str(&format!("{}=\"{}\" ", k, v));
                    }
                }
            }
            prop_str.push_str(";\n");
            (prop_str, tag_end - start)
        }
        "cfspreadsheet" => {
            // Action-dispatched spreadsheet tag → the Spreadsheet* BIFs.
            //   read   → NAME = spreadsheetRead(src)[.setActiveSheet…][.toQuery()/.toCsv()]
            //   write  → spreadsheetWrite( <workbook | query→workbook>, filename, overwrite )
            //   update → treated as write (writes the file; POI-style in-place
            //            merge into an existing file is not yet modelled).
            let action = attrs
                .get("action")
                .map(|s| s.to_lowercase())
                .unwrap_or_else(|| "read".to_string());
            let overwrite = attrs
                .get("overwrite")
                .map(|v| strip_hashes(v))
                .unwrap_or_else(|| "false".to_string());
            let code = match action.as_str() {
                "write" | "update" => {
                    let filename = attrs
                        .get("filename")
                        .or_else(|| attrs.get("src"))
                        .map(|v| format_attr_value(v, quoted.contains("filename") || quoted.contains("src")))
                        .unwrap_or_else(|| "\"\"".to_string());
                    if let Some(qv) = attrs.get("query") {
                        format!(
                            "spreadsheetWrite( spreadsheet(\"xlsx\").addRows({}, 1, 1, true), {}, {} );\n",
                            strip_hashes(qv), filename, overwrite
                        )
                    } else if let Some(nm) = attrs.get("name") {
                        format!("spreadsheetWrite({}, {}, {});\n", strip_hashes(nm), filename, overwrite)
                    } else {
                        String::new()
                    }
                }
                _ => {
                    // read
                    let src = attrs
                        .get("src")
                        .or_else(|| attrs.get("filename"))
                        .map(|v| format_attr_value(v, quoted.contains("src") || quoted.contains("filename")))
                        .unwrap_or_else(|| "\"\"".to_string());
                    let mut expr = format!("spreadsheetRead({})", src);
                    if let Some(sheet) = attrs.get("sheet") {
                        expr = format!("{}.setActiveSheetNumber({})", expr, strip_hashes(sheet));
                    } else if let Some(sn) = attrs.get("sheetname") {
                        expr = format!("{}.setActiveSheet({})", expr, format_attr_value(sn, quoted.contains("sheetname")));
                    }
                    // A `query` attribute → return a query; `name` → object (or a
                    // CSV/HTML string when format says so).
                    let (target, suffix) = if let Some(qv) = attrs.get("query") {
                        (strip_hashes(qv), ".toQuery()".to_string())
                    } else {
                        let nm = attrs
                            .get("name")
                            .map(|v| strip_hashes(v))
                            .unwrap_or_else(|| "cfspreadsheet".to_string());
                        let suf = match attrs.get("format").map(|f| f.to_lowercase()).as_deref() {
                            Some("csv") => ".toCsv()".to_string(),
                            _ => String::new(),
                        };
                        (nm, suf)
                    };
                    format!("{} = {}{};\n", target, expr, suffix)
                }
            };
            (code, tag_end - start)
        }
        "cfhttp" => {
            let result_var = attrs.get("result").cloned().unwrap_or("cfhttp".to_string());

            let mut opts = Vec::new();
            // attributeCollection="#struct#" — a struct of cfhttp attributes
            // (Lucee/BoxLang). fn_cfhttp merges it into the options struct with
            // explicitly-supplied attributes winning. Pass it through verbatim;
            // format_attr_value preserves the native struct type.
            if let Some(ac) = attrs.get("attributecollection") {
                opts.push(format!("attributeCollection: {}", format_attr_value(ac, quoted.contains("attributecollection"))));
            }
            // Emit only the attributes the author actually supplied so an
            // attributeCollection can provide the rest (an unconditional
            // `url: ""` would otherwise win the merge and blank the URL).
            // String attributes may carry #expr# interpolation inside an
            // otherwise-literal quoted value (e.g. url="#baseUrl##path#").
            // format_attr_value emits literal segments quoted and only evaluates
            // the #...# parts; strip_hashes would collapse the whole value into a
            // bare (mis-parsed) expression like `baseUrlpath`.
            //
            // EVERY attribute is forwarded, not a whitelist — same lesson as
            // cfquery (GH #294) and cflock (v0.553.0): a fixed list drops
            // whatever it doesn't know about at COMPILE time, so the attribute
            // never reaches the runtime and there is no "unknown option" either.
            // The ten-key list here silently discarded `name` (response never
            // parsed into a query, variable never created), `file`/`path` (body
            // never written to disk) and `throwOnError`/`redirect`/`port`/
            // `proxyPort`/`encodeURL` — the last five already implemented in
            // fn_cfhttp and lost purely in the lowering. `result` is excluded
            // because it is the assignment target below, not an option.
            // `attr_order` (source order) drives emission so codegen stays
            // deterministic — `attrs` is a HashMap.
            for key in &attr_order {
                if key == "result" || key == "attributecollection" {
                    continue;
                }
                let Some(v) = attrs.get(key) else { continue };
                opts.push(format!("{}: {}", key, format_attr_value(v, quoted.contains(key))));
            }

            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfhttp") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                // Runtime body assembly when the body contains control-flow
                // tags (issue #55): <cfhttpparam> inside <cfif>/<cfloop> must
                // be evaluated at runtime, not collected by a static body scan.
                if body_has_control_flow_except(&body, &["cfhttpparam"]) {
                    let body_script = tags_to_script_impl(&body, imports);
                    opts.push("params: __cfhttp_params".to_string());
                    let code = format!(
                        "__cfhttp_params = [];\n{}{} = cfhttp({{ {} }});\n",
                        body_script, result_var, opts.join(", ")
                    );
                    return (code, close_end - start);
                }
                let params = parse_cfhttpparam_tags(&body);
                if !params.is_empty() {
                    opts.push(format!("params: [{}]", params.join(", ")));
                }
                (format!("{} = cfhttp({{ {} }});\n", result_var, opts.join(", ")), close_end - start)
            } else {
                (format!("{} = cfhttp({{ {} }});\n", result_var, opts.join(", ")), tag_end - start)
            }
        }
        "cfhttpparam" => {
            // Reached only when a cfhttp body is built at runtime (it contains
            // control-flow tags). Append the param struct to the runtime list;
            // outside a runtime cfhttp body __cfhttp_params is undefined — which
            // is correct since cfhttpparam is invalid there anyway.
            let lit = cfhttpparam_attrs_to_literal(&attrs, &quoted);
            (format!("arrayAppend(__cfhttp_params, {});\n", lit), tag_end - start)
        }
        "cfqueryparam" => {
            // Reached only when a cfquery body is built at runtime (it contains
            // control-flow tags). Emit a "?" into the SQL buffer and append the
            // param struct to the runtime params array. Outside a cfquery this
            // would reference an undefined __cfquery_params — which is correct,
            // since cfqueryparam is invalid there anyway.
            let p = parse_cfqueryparam_attrs(&attrs);
            let lit = cfqueryparam_to_literal(&p);
            (
                format!("writeOutput(\"?\");arrayAppend(__cfquery_params, {});\n", lit),
                tag_end - start,
            )
        }
        "cfquery" => {
            // Everything between <cfquery> and </cfquery> is the SQL
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfquery") {
                let sql_raw: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                // Scan for <cfqueryparam> tags — replace with ? and collect params
                let (cleaned_sql, query_params) = scan_cfqueryparam_tags(&sql_raw);

                // Process remaining hash expressions in SQL for string interpolation
                let sql = process_sql_hashes(&cleaned_sql);

                // All attributes ride in queryExecute's options struct. The
                // VM intercept expands attributeCollection and delivers
                // `name`/`result` (possibly dotted, e.g. "local.wheels.result")
                // into the calling scope at runtime — `name` only when a
                // resultset came back, matching Lucee (an INSERT leaves the
                // name variable untouched and `result` gets the metadata).
                //
                // EVERY attribute is forwarded, not a whitelist: the tag must
                // honour whatever queryExecute honours (columnKey/keyColumn,
                // maxrows, timeout, …) — GH #294, where a whitelist of six
                // silently dropped `columnkey`/`maxrows` so
                // returntype="struct" fell into the single-row-map branch and
                // came back keyed by column names. Options the engine doesn't
                // recognise are simply ignored downstream, as in Lucee.
                // `attr_order` (source order) drives emission so codegen stays
                // deterministic — `attrs` is a HashMap.
                let mut opts_parts = Vec::new();
                for key in &attr_order {
                    let Some(raw) = attrs.get(key) else { continue };
                    // Canonical spellings for the keys the VM intercept and the
                    // builtin look up; everything else rides through as written
                    // (both sides match case-insensitively anyway).
                    let target = match key.as_str() {
                        "returntype" => "returnType",
                        "attributecollection" => "attributeCollection",
                        "columnkey" => "columnKey",
                        "keycolumn" => "keyColumn",
                        other => other,
                    };
                    // A key that isn't a bare identifier (hyphens etc.) has to
                    // be quoted to stay a legal struct literal.
                    let target = if !target.is_empty()
                        && target.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !target.starts_with(|c: char| c.is_ascii_digit())
                    {
                        target.to_string()
                    } else {
                        format!("\"{}\"", target.replace('"', "\"\""))
                    };
                    let val = strip_hashes(raw);
                    if raw != &val {
                        // Dynamic #expr# value — emit as expression
                        opts_parts.push(format!("{}: {}", target, val));
                    } else {
                        opts_parts.push(format!("{}: \"{}\"", target, raw));
                    }
                }
                if !attrs.contains_key("name") && !attrs.contains_key("attributecollection") {
                    // Pre-#90 leniency: nameless cfquery still populates
                    // queryResult.
                    opts_parts.push("name: \"queryResult\"".to_string());
                }
                opts_parts.push("__cfquery_tag: true".to_string());
                let opts_str = format!("{{ {} }}", opts_parts.join(", "));

                let params_str = if query_params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_strs: Vec<String> = query_params.iter()
                        .map(cfqueryparam_to_literal)
                        .collect();
                    format!("[{}]", param_strs.join(", "))
                };

                // If the body contains control-flow tags (cfif, cfloop, cfset…),
                // the SQL and the set of bound params can vary at runtime, so the
                // compile-time string/array above is wrong. Build both at runtime
                // by executing the body savecontent-style: text and #expr# append
                // to the SQL buffer, and each cfqueryparam appends "?" plus its
                // struct to the params array.
                if body_has_control_flow(&sql_raw) {
                    let body_script = tags_to_script_inner(&sql_raw, imports, true);
                    let code = format!(
                        "__cfquery_params = [];\n__cfsavecontent_start();\n{}queryExecute(__cfsavecontent_end(), __cfquery_params, {});\n",
                        body_script, opts_str
                    );
                    return (code, close_end - start);
                }

                (format!("queryExecute({}, {}, {});\n", sql, params_str, opts_str), close_end - start)
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
                if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    // format_attr_value handles whole-value #expr#, mixed
                    // literal+#expr# (e.g. statustext="Error #code#"), and embedded
                    // quotes (doubled, not backslash-escaped).
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
                    if v.contains('#') {
                        // reset="#expr#" — evaluate at runtime (Lucee parity),
                        // don't literal-match the raw "#...#" text.
                        parts.push(format!(
                            "{}: {}",
                            k,
                            format_attr_value(v, quoted.contains(k.as_str()))
                        ));
                    } else {
                        let lower = val.to_lowercase();
                        if lower == "true" || lower == "yes" {
                            parts.push(format!("{}: true", k));
                        } else {
                            parts.push(format!("{}: false", k));
                        }
                    }
                } else if k == "variable" {
                    parts.push(format!("{}: {}", k, val));
                } else {
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
                if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    let lower = raw.to_lowercase();
                    if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else if lower == "false" || lower == "no" {
                        parts.push(format!("{}: false", k));
                    } else {
                        // url="/page?id=#id#" and other mixed literal+#expr#
                        // values interpolate; embedded quotes are doubled.
                        parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
                    }
                }
            }
            (format!("__cflocation({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cfdbinfo" => {
            // <cfdbinfo type="columns" name="cols" table="t" datasource="ds">
            // → cfdbinfo({ type: "columns", name: "cols", ... });
            // The VM intercept runs the metadata query and delivers it to the
            // (possibly dotted) `name` variable — no compile-time assignment.
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
            }
            (format!("cfdbinfo({{ {} }});\n", parts.join(", ")), tag_end - start)
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
                let lower = raw.to_lowercase();
                if lower == "true" || lower == "yes" {
                    parts.push(format!("{}: true", k));
                } else if lower == "false" || lower == "no" {
                    parts.push(format!("{}: false", k));
                } else if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
                let lower = raw.to_lowercase();
                if lower == "true" || lower == "yes" {
                    parts.push(format!("{}: true", k));
                } else if lower == "false" || lower == "no" {
                    parts.push(format!("{}: false", k));
                } else if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
                // Process body preserving the enclosing cfoutput context. When the
                // <cfsavecontent> is nested inside <cfoutput>, its body's #expr#
                // must still interpolate (Lucee/ACF). Using tags_to_script_impl
                // here reset in_cfoutput to false, so Preside's form.cfm captured
                // literal `#formId#`/`#validationJs#` inside the validation-JS
                // savecontent. Same class as the v0.443.0 cfswitch/cfcase fix.
                let body_script = tags_to_script_inner(&body, imports, in_cfoutput);
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

                        // Build args for __cftransaction_start. All THREE
                        // positions are always emitted — action, isolation,
                        // datasource — with an empty string standing in for an
                        // absent attribute. Omitting them shifted the arguments
                        // left, so `<cftransaction isolation="serializable">`
                        // with no datasource delivered "serializable" INTO the
                        // datasource slot and the block tried to open a
                        // connection to a datasource by that name.
                        let iso_arg = match isolation {
                            Some(ref iso) => format!("\"{}\"", iso),
                            None => "\"\"".to_string(),
                        };
                        let ds_arg = match datasource {
                            Some(ref ds) => {
                                let ds_val = strip_hashes(ds);
                                if ds != &ds_val {
                                    ds_val
                                } else {
                                    format!("\"{}\"", ds)
                                }
                            }
                            // No datasource attribute: fall back to the one the
                            // first <cfquery> in the body names, as before.
                            None => match extract_datasource_from_body(&body) {
                                Some(ds) => format!("\"{}\"", ds),
                                None => "\"\"".to_string(),
                            },
                        };
                        let txn_args = vec!["\"begin\"".to_string(), iso_arg, ds_arg];

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
                let switch_body = parse_cfswitch_body(&body, imports, in_cfoutput);
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
            // `<cffinally>` opens a `finally {` block, but the preceding `try {`
            // (or catch) must be closed first. In the catch+finally form the
            // preceding `</cfcatch>` already emitted that closing `}`, so we emit
            // only `finally {`. In the CATCHLESS form (`<cftry>…<cffinally>`)
            // nothing has closed the `try {` yet, so we must emit `} finally {`.
            // Detect by scanning back over whitespace for a preceding
            // `</cfcatch>`. (PR #162 — Lucee accepts catchless try/finally.)
            let mut j = start;
            while j > 0 && chars[j - 1].is_whitespace() {
                j -= 1;
            }
            let preceded_by_catch = j > 0 && chars[j - 1] == '>' && {
                // Walk back to the matching `<` and normalise the tag text.
                let mut k = j - 1;
                while k > 0 && chars[k - 1] != '<' {
                    k -= 1;
                }
                let tag: String = chars[k.saturating_sub(1)..j]
                    .iter()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                tag.eq_ignore_ascii_case("</cfcatch>")
            };
            let prefix = if preceded_by_catch { "" } else { "}\n" };
            (format!("{}finally {{\n", prefix), tag_end - start)
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
            // Route through format_attr_value so a mixed literal+#expr# value
            // (e.g. text="took #n#ms") interpolates correctly. strip_hashes only
            // handled a value that is entirely one #expr#; a mixed value emitted
            // malformed script and failed to parse.
            for (k, v) in &attrs {
                parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
                    // Preserve the enclosing cfoutput context so `#expr#` in the
                    // body still interpolates when the directive is nested inside
                    // <cfoutput> (Masa's admin layouts wrap the whole body in
                    // `<cfoutput><cfprocessingdirective suppresswhitespace>`).
                    let body_script = tags_to_script_inner(&body, imports, in_cfoutput);
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
                    let body_script = tags_to_script_inner(&body, imports, in_cfoutput);
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
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
        "cfstatic" => {
            // Static initialization block (tag form). Convert the body's tags to
            // script and wrap it in a `static { ... }` block, which the parser
            // collects into the component's static_body.
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfstatic") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_script = tags_to_script_impl(&body, imports);
                (format!("static {{\n{}}}\n", body_script), close_end - start)
            } else {
                (String::new(), tag_end - start)
            }
        }
        "cfcookie" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let lower = v.to_lowercase();
                if k == "secure" || k == "httponly" {
                    if v.contains('#') {
                        // Expression like secure="#isHttps#" — evaluate at runtime
                        // (Lucee parity), don't literal-match the raw "#...#" text.
                        parts.push(format!(
                            "{}: {}",
                            k,
                            format_attr_value(v, quoted.contains(k.as_str()))
                        ));
                    } else if lower == "true" || lower == "yes" {
                        parts.push(format!("{}: true", k));
                    } else {
                        parts.push(format!("{}: false", k));
                    }
                } else if v.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, v));
                } else {
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
                }
            }
            (format!("__cfcookie({{ {} }});\n", parts.join(", ")), tag_end - start)
        }
        "cffile" => {
            parse_cffile_tag(&attrs, &quoted, tag_end - start)
        }
        "cfimage" => {
            // <cfimage action="resize" source="in.png" name="img" width="100">
            // → img = cfimage({ action:"resize", source:in.png, name:"img", ... });
            // The `writeToBrowser` action needs the output buffer, so it is
            // lowered directly to a writeOutput() of the base64 <img> markup.
            let action = attrs.get("action").map(|s| s.to_lowercase()).unwrap_or_else(|| "read".to_string());
            let name = attrs.get("name").cloned();
            let struct_name = attrs.get("structname").cloned();

            let mut parts = Vec::new();
            for (k, v) in &attrs {
                let raw = v.trim();
                let lower = raw.to_lowercase();
                if lower == "true" || lower == "yes" {
                    parts.push(format!("{}: true", k));
                } else if lower == "false" || lower == "no" {
                    parts.push(format!("{}: false", k));
                } else if raw.parse::<f64>().is_ok() {
                    parts.push(format!("{}: {}", k, raw));
                } else {
                    parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
                }
            }
            let call = format!("cfimage({{ {} }})", parts.join(", "));

            if action == "writetobrowser" {
                // source may be an image var (#img#) or a quoted path.
                let src = attrs.get("source").cloned().unwrap_or_default();
                let src_expr = format_attr_value(&src, quoted.contains("source"));
                let fmt = attrs.get("format").cloned().unwrap_or_else(|| "png".to_string()).to_lowercase();
                (format!(
                    "writeOutput('<img src=\"data:image/{f};base64,' & toBase64(imageGetBlob({src}, \"{f}\")) & '\" />');\n",
                    f = fmt, src = src_expr
                ), tag_end - start)
            } else if let Some(sn) = struct_name.filter(|_| action == "info") {
                (format!("{} = {};\n", sn, call), tag_end - start)
            } else if let Some(n) = name {
                (format!("{} = {};\n", n, call), tag_end - start)
            } else {
                (format!("{};\n", call), tag_end - start)
            }
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
                        // name="lock_#id#" and other interpolated values.
                        lock_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
                    }
                }
                let lock_args = format!("{{ {} }}", lock_parts.join(", "));
                // The body is guarded on the acquire result: with
                // throwOnTimeout="false" a lock that times out skips the body and
                // execution continues (and __cflock_end must not run, since nothing
                // was acquired). Any other outcome returns true.
                (format!(
                    "if (__cflock_start({})) {{\ntry {{\n{}\n__cflock_end({});\n}} catch(any __lock_e) {{\n__cflock_end({});\nthrow __lock_e;\n}}\n}}\n",
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

            // `<cfinvoke>` may be a container with `<cfinvokeargument name= value=>`
            // children (each a distinct method argument) — e.g. Mura/Masa's event
            // dispatch passes the scope object as an argument literally named `$`.
            // The children are the ONLY place those args live, so a self-closing
            // parse silently dropped every argument. If a matching `</cfinvoke>`
            // exists (and the open tag isn't self-closed), harvest the children.
            let mut child_parts: Vec<String> = Vec::new();
            let mut consumed = tag_end - start;
            if !is_self_closing_tag(chars, tag_end) {
                if let Some(close_pos) = find_closing_tag(chars, tag_end, len, "cfinvoke") {
                    let mut i = tag_end;
                    while i < close_pos {
                        if chars[i] == '<' {
                            let rem: String =
                                chars[i..].iter().take(17).collect::<String>().to_lowercase();
                            if rem.starts_with("<cfinvokeargument") {
                                // Name-end: skip past the tag name to its attributes.
                                let mut ne = i + 1;
                                while ne < len
                                    && (chars[ne].is_alphanumeric() || chars[ne] == '_')
                                {
                                    ne += 1;
                                }
                                let (cattrs, cquoted, _order, cend) =
                                    parse_tag_attributes_ordered(chars, ne, len);
                                if let Some(argname) = cattrs.get("name") {
                                    // The arg key is the (possibly `$`) name; always
                                    // quote it so keys like `$` are legal struct keys.
                                    let key = strip_hashes(argname);
                                    let val = cattrs
                                        .get("value")
                                        .map(|v| format_attr_value(v, cquoted.contains("value")))
                                        .unwrap_or_else(|| "\"\"".to_string());
                                    child_parts.push(format!("\"{}\": {}", key, val));
                                }
                                i = cend;
                                continue;
                            }
                        }
                        i += 1;
                    }
                    consumed = find_tag_end(chars, close_pos, len) - start;
                }
            }

            // Third argument: argumentcollection or struct of remaining attrs +
            // any harvested `<cfinvokeargument>` children.
            let third_arg = if let Some(ac) = arg_collection {
                let ac = strip_hashes(&ac);
                if child_parts.is_empty() {
                    ac
                } else {
                    // Base collection plus per-arg overrides (individual args win).
                    format!("structAppend(duplicate({}), {{ {} }}, true)", ac, child_parts.join(", "))
                }
            } else {
                let reserved = ["component", "method", "returnvariable", "argumentcollection"];
                let mut extra_parts = Vec::new();
                for (k, v) in &attrs {
                    if reserved.contains(&k.as_str()) {
                        continue;
                    }
                    extra_parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k))));
                }
                extra_parts.extend(child_parts);
                format!("{{ {} }}", extra_parts.join(", "))
            };

            // `webservice=` invokes a SOAP endpoint. RustCFML has no web-service
            // client, and because `webservice` was not a reserved attribute it
            // used to ride along as a METHOD ARGUMENT while `component` resolved
            // to `""` — a confusing "component not found" at best, a silent call
            // on the wrong object at worst (docs known-issues §27). Fail with the
            // reason instead. Deliberately a runtime throw, not a compile error:
            // Lucee compiles the tag happily, so an app that merely *contains* an
            // unreached SOAP call must still start.
            if attrs.contains_key("webservice") {
                return (
                    "throw(type=\"Application\", message=\"cfinvoke webservice= is not supported: RustCFML has no SOAP web-service client. Call the endpoint with cfhttp instead.\");\n"
                        .to_string(),
                    consumed,
                );
            }

            let call = format!("__cfinvoke({}, {}, {})", comp_expr, method_expr, third_arg);
            if let Some(rv) = return_var {
                (format!("{} = {};\n", rv, call), consumed)
            } else {
                (format!("{};\n", call), consumed)
            }
        }
        "cfmodule" => {
            // <cfmodule template="path.cfm" attr1="val1"> or <cfmodule name="dot.path" attr1="val1">
            let template = attrs.get("template").cloned();
            let name_attr = attrs.get("name").cloned();
            let uses_template = template.is_some();

            let path_expr = if let Some(t) = template {
                // template may contain #hash# interpolation, e.g.
                // <cfmodule template="#modulePath#">. Evaluate it like any
                // other attribute rather than treating it as a literal string.
                format_attr_value(&t, quoted.contains("template"))
            } else if let Some(n) = name_attr {
                // name= form is dispatched via a "__name:" sentinel prefix, so
                // concatenate the (possibly interpolated) name onto it.
                format!("\"__name:\" & ({})", format_attr_value(&n, quoted.contains("name")))
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
                // XML-style self-closing custom tags still run the end phase.
                let run_end = is_self_closing_tag(chars, tag_end);
                let result = format!("__cfcustomtag({}, {}, {});\n", path_expr, attrs_expr, run_end);
                (result, tag_end - start)
            }
        }
        "cfcache" => {
            let mut parts = Vec::new();
            for (k, v) in &attrs {
                parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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
            opts.push(format!("name: {}", format_attr_value(&name_attr, quoted.contains("name"))));
            // arguments/timeout may carry #expr# interpolation inside an
            // otherwise-literal quoted value; route them through format_attr_value
            // like cfhttp's attributes. (The old path emitted timeout verbatim —
            // leaving literal hashes that fail to parse — and escaped embedded
            // quotes with a backslash, which CFML does not honor; a quote is
            // escaped by doubling it.)
            if let Some(a) = &arguments {
                opts.push(format!("arguments: {}", format_attr_value(a, quoted.contains("arguments"))));
            }
            if let Some(t) = &timeout {
                opts.push(format!("timeout: {}", format_attr_value(t, quoted.contains("timeout"))));
            }
            if variable.is_some() {
                opts.push("variable: true".to_string());
            }

            // Check for body (stdin input)
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfexecute") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let body_trimmed = body.trim();
                if !body_trimmed.is_empty() {
                    opts.push(format!("body: \"{}\"", body_trimmed.replace('"', "\"\"")));
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
            // Route through format_attr_value so a mixed literal+#expr# value
            // (e.g. subject="Order #id# shipped") interpolates correctly, the
            // same as cfthrow/cfargument/cffile. strip_hashes only handled a
            // value that is entirely one #expr#.
            for (k, v) in &attrs {
                opts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
            }

            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfmail") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                // Runtime body assembly when the body contains control-flow
                // tags (issue #55): <cfmailparam>/<cfmailpart> inside <cfif>/
                // <cfloop> must be evaluated at runtime; the body text is
                // captured via savecontent.
                if body_has_control_flow_except(&body, &["cfmailparam", "cfmailpart"]) {
                    let body_script = tags_to_script_inner(&body, imports, true);
                    opts.push("params: __cfmail_params".to_string());
                    opts.push("parts: __cfmail_parts".to_string());
                    opts.push("body: __cfmail_body".to_string());
                    let code = format!(
                        "__cfmail_params = [];\n__cfmail_parts = [];\n__cfsavecontent_start();\n{}__cfmail_body = __cfsavecontent_end();\n__cfmail({{ {} }});\n",
                        body_script, opts.join(", ")
                    );
                    return (code, close_end - start);
                }

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
                    opts.push(format!("body: \"{}\"", body_text.replace('"', "\"\"")));
                }

                (format!("__cfmail({{ {} }});\n", opts.join(", ")), close_end - start)
            } else {
                // Self-closing cfmail
                (format!("__cfmail({{ {} }});\n", opts.join(", ")), tag_end - start)
            }
        }
        "cfmailparam" => {
            // Reached only when a cfmail body is built at runtime (it contains
            // control-flow tags). Append the param struct to the runtime list;
            // outside a runtime cfmail body __cfmail_params is undefined — which
            // is correct since cfmailparam is invalid there anyway.
            let lit = cfmailparam_attrs_to_literal(&attrs, &quoted);
            (format!("arrayAppend(__cfmail_params, {});\n", lit), tag_end - start)
        }
        "cfmailpart" => {
            // Runtime cfmail body. Capture the part's own body via a nested
            // savecontent and append the parts list.
            let attrs_lit = cfmailpart_attrs_only_literal(&attrs);
            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfmailpart") {
                let body_chars = &chars[tag_end..end_tag_pos];
                let body_source: String = body_chars.iter().collect();
                let body_script = tags_to_script_inner(&body_source, imports, true);
                let close_end = find_tag_end(chars, end_tag_pos, len);
                let code = format!(
                    "__cfsavecontent_start();\n{}arrayAppend(__cfmail_parts, structAppend({}, {{ body: __cfsavecontent_end() }}));\n",
                    body_script, attrs_lit
                );
                (code, close_end - start)
            } else {
                // Self-closing cfmailpart (no body)
                (
                    format!("arrayAppend(__cfmail_parts, {});\n", attrs_lit),
                    tag_end - start,
                )
            }
        }
        "cfstoredproc" => {
            let procedure = attrs.get("procedure").cloned().unwrap_or_default();
            let datasource = attrs.get("datasource").cloned();

            if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfstoredproc") {
                let body: String = chars[tag_end..end_tag_pos].iter().collect();
                let close_end = find_tag_end(chars, end_tag_pos, len);

                // Statically pluck the result variable name (cfprocresult is
                // declarative and rarely conditional; this lets the runtime
                // path still assign to the right caller variable).
                let proc_results = parse_cfprocresult_tags(&body);
                let result_var = proc_results.first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| "cfresult".to_string());

                let mut query_opts = Vec::new();
                if let Some(ds) = &datasource {
                    query_opts.push(format!("datasource: \"{}\"", ds));
                }
                let opts_str = if query_opts.is_empty() {
                    String::new()
                } else {
                    format!(", {{ {} }}", query_opts.join(", "))
                };

                // Runtime body assembly when the body wraps cfprocparams in
                // control flow (issue #55).
                if body_has_control_flow_except(&body, &["cfprocparam", "cfprocresult"]) {
                    let body_script = tags_to_script_impl(&body, imports);
                    // Placeholders are constructed from the runtime list length
                    // — repeatString is a builtin in stdlib.
                    let code = format!(
                        "__cfproc_params = [];\n{}__cfproc_placeholders = arrayLen(__cfproc_params) == 0 ? \"\" : reReplace(repeatString(\"?,\", arrayLen(__cfproc_params)), \",$\", \"\", \"one\");\n{} = queryExecute(\"CALL {}(\" & __cfproc_placeholders & \")\", __cfproc_params{});\n",
                        body_script, result_var, procedure, opts_str
                    );
                    return (code, close_end - start);
                }

                let proc_params = parse_cfprocparam_tags(&body);

                // Build param placeholders
                let placeholders: Vec<&str> = proc_params.iter().map(|_| "?").collect();
                let sql = format!("CALL {}({})", procedure, placeholders.join(","));

                // Build params array
                let params_arr: Vec<String> = proc_params.iter().map(|p| {
                    let mut parts = Vec::new();
                    if let Some(ref v) = p.value {
                        // cfprocparam value="..." is a quoted attribute; route
                        // through format_attr_value so mixed literal+#expr# values
                        // interpolate and embedded quotes are doubled.
                        parts.push(format!("value: {}", format_attr_value(v, true)));
                    }
                    if let Some(ref t) = p.cfsqltype {
                        parts.push(format!("cfsqltype: \"{}\"", t));
                    }
                    format!("{{ {} }}", parts.join(", "))
                }).collect();

                (format!("{} = queryExecute(\"{}\", [{}]{});\n",
                    result_var, sql, params_arr.join(", "), opts_str), close_end - start)
            } else {
                // Self-closing (unusual)
                (format!("queryExecute(\"CALL {}()\");\n", procedure), tag_end - start)
            }
        }
        "cfprocparam" => {
            // Runtime cfstoredproc body. Append the param struct to the runtime list.
            let lit = cfprocparam_attrs_to_literal(&attrs, &quoted);
            (format!("arrayAppend(__cfproc_params, {});\n", lit), tag_end - start)
        }
        "cfprocresult" => {
            // Inert in the runtime path — the result variable name is decided
            // statically at the cfstoredproc dispatcher (see body-scan in the
            // "cfstoredproc" arm). Emit nothing so a cfprocresult inside a
            // cfif/cfloop doesn't break compilation.
            (String::new(), tag_end - start)
        }
        "cfthread" => {
            let action = attrs.get("action").cloned().unwrap_or_else(|| "run".to_string()).to_lowercase();
            let thread_name = attrs.get("name").cloned().unwrap_or_else(|| "thread1".to_string());
            let thread_name_expr = format_attr_value(&thread_name, quoted.contains("name"));

            match action.as_str() {
                "run" => {
                    if let Some(end_tag_pos) = find_closing_tag(chars, tag_end, len, "cfthread") {
                        let body: String = chars[tag_end..end_tag_pos].iter().collect();
                        let close_end = find_tag_end(chars, end_tag_pos, len);
                        let body_script = tags_to_script_impl(&body, imports);
                        // Custom attributes (everything but the reserved control
                        // attributes) become the thread's `attributes` scope,
                        // bound from the parent context at spawn. Values are
                        // emitted as double-quoted strings so `#expr#` in an
                        // attribute interpolates in the parent, as for any tag.
                        let attr_entries: Vec<String> = attrs
                            .iter()
                            .filter(|(k, _)| {
                                !matches!(
                                    k.to_lowercase().as_str(),
                                    "action" | "name" | "priority" | "timeout"
                                )
                            })
                            .map(|(k, v)| {
                                format!(
                                    "\"{}\": {}",
                                    k.replace('"', "\"\""),
                                    format_attr_value(v, quoted.contains(k.as_str()))
                                )
                            })
                            .collect();
                        let attrs_arg = if attr_entries.is_empty() {
                            String::new()
                        } else {
                            format!(", {{ {} }}", attr_entries.join(", "))
                        };
                        (format!(
                            "__cfthread_run({}, function() {{\n{}\n}}{});\n",
                            thread_name_expr, body_script, attrs_arg
                        ), close_end - start)
                    } else {
                        (String::new(), tag_end - start)
                    }
                }
                "join" => {
                    let timeout = attrs.get("timeout").cloned().unwrap_or_else(|| "0".to_string());
                    // No `name` attribute => join ALL outstanding threads. (The
                    // shared `thread_name` defaults to "thread1", so the join
                    // arm must consult the raw attribute rather than the default.)
                    let join_name = match attrs.get("name") {
                        Some(n) => format_attr_value(n, quoted.contains("name")),
                        None => "\"\"".to_string(),
                    };
                    (format!("__cfthread_join({}, {});\n", join_name, timeout), tag_end - start)
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
                    format!("throw(\"cfimport without taglib is not implemented ({}). Only CFML taglib imports are supported.\");\n", escape_for_string_literal(&detail)),
                    tag_end - start,
                );
            }
            (String::new(), tag_end - start)
        }
        "cfsleep" => {
            // <cfsleep time="ms"> → sleep(ms);  (the sleep() BIF already exists)
            let raw = attrs.get("time").cloned().unwrap_or_else(|| "0".to_string());
            (
                format!("sleep({});\n", format_attr_value(&raw, quoted.contains("time"))),
                tag_end - start,
            )
        }
        "cfexit" => {
            // <cfexit method="exittag|exittemplate|loop">. When `method` is
            // omitted the default is "exittemplate". Lowered to the VM-intercepted
            // `__cfexit` control-flow signal.
            let method = attrs
                .get("method")
                .map(|m| m.trim().to_lowercase())
                .unwrap_or_else(|| "exittemplate".to_string());
            (format!("__cfexit(\"{}\");\n", method), tag_end - start)
        }
        "cfhtmlhead" | "cfhtmlbody" => {
            // <cfhtmlhead text="..."> / <cfhtmlbody text="..."> (and the body
            // form <cfhtmlhead>...</cfhtmlhead>). Content is buffered by the VM
            // and injected into the response <head>/<body> at output flush time.
            let fn_name = if tag_lower == "cfhtmlhead" {
                "__cfhtmlhead"
            } else {
                "__cfhtmlbody"
            };
            if let Some(body_start) = find_closing_tag(chars, tag_end, len, &tag_lower) {
                let body_text: String = chars[tag_end..body_start].iter().collect();
                let close_end = find_tag_end(chars, body_start, len);
                (
                    format!(
                        "{}(\"{}\");\n",
                        fn_name,
                        escape_for_string_literal(&body_text)
                    ),
                    close_end - start,
                )
            } else {
                let raw = attrs.get("text").cloned().unwrap_or_default();
                (
                    format!(
                        "{}({});\n",
                        fn_name,
                        format_attr_value(&raw, quoted.contains("text"))
                    ),
                    tag_end - start,
                )
            }
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
                    // XML-style self-closing custom tags still run the end phase.
                    let run_end = is_self_closing_tag(chars, tag_end);
                    let result = format!("__cfcustomtag({}, {}, {});\n", path_expr, attrs_expr, run_end);
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
    while i < len {
        // A CFML comment embedded in the tag (e.g. inside a multi-line cfset
        // expression body) may contain `>` in its `--->` terminator — skip the
        // whole comment so it doesn't prematurely end the tag.
        if i + 4 < len && chars[i] == '<' && chars[i + 1] == '!'
            && chars[i + 2] == '-' && chars[i + 3] == '-' && chars[i + 4] == '-'
        {
            let mut j = i + 5;
            while j + 2 < len && !(chars[j] == '-' && chars[j + 1] == '-' && chars[j + 2] == '>') {
                j += 1;
            }
            i = if j + 2 < len { j + 3 } else { len };
            continue;
        }
        let c = chars[i];
        if c == '"' || c == '\'' {
            // Skip the whole CFML string, honouring `""`/`''` escapes and
            // `#...#` interpolations (which may themselves contain nested
            // strings — e.g. `'"#replaceNoCase(v,'"','""','all')#"'`). A naive
            // quote toggle mis-tokenises those and loses the tag's `>`.
            i = skip_cfml_string(chars, i, len);
            continue;
        }
        if c == '>' {
            return i + 1;
        }
        i += 1;
    }
    len
}

/// Advance past a CFML string literal. `start` must point at the opening quote.
/// Returns the index just past the closing quote (or `len` if unterminated).
/// Handles `""`/`''` doubled-quote escapes, `##` escaped hashes, and `#...#`
/// interpolations whose expressions may contain their own nested strings.
fn skip_cfml_string(chars: &[char], start: usize, len: usize) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < len {
        let c = chars[i];
        if c == quote {
            if i + 1 < len && chars[i + 1] == quote {
                i += 2; // escaped doubled quote — stays inside the string
                continue;
            }
            return i + 1;
        }
        if c == '#' {
            if i + 1 < len && chars[i + 1] == '#' {
                i += 2; // escaped `##` — a literal hash
                continue;
            }
            i = skip_interpolation(chars, i, len);
            continue;
        }
        i += 1;
    }
    len
}

/// Advance past a `#...#` interpolation. `start` must point at the opening `#`.
/// Returns the index just past the closing `#`. Nested strings inside the
/// interpolated expression are skipped so their quotes don't leak out.
fn skip_interpolation(chars: &[char], start: usize, len: usize) -> usize {
    let mut i = start + 1;
    while i < len {
        let c = chars[i];
        if c == '#' {
            return i + 1;
        }
        if c == '"' || c == '\'' {
            i = skip_cfml_string(chars, i, len);
            continue;
        }
        i += 1;
    }
    len
}

fn is_self_closing_tag(chars: &[char], tag_end: usize) -> bool {
    let mut i = tag_end.saturating_sub(1);
    while i > 0 {
        i -= 1;
        if chars[i].is_whitespace() {
            continue;
        }
        return chars[i] == '/';
    }
    false
}

fn parse_tag_attributes(
    chars: &[char],
    start: usize,
    len: usize,
) -> (std::collections::HashMap<String, String>, std::collections::HashSet<String>, usize) {
    let (attrs, quoted, _order, end) = parse_tag_attributes_ordered(chars, start, len);
    (attrs, quoted, end)
}

/// Like `parse_tag_attributes` but also returns the attribute keys in the order
/// they appear in the source. Tag attributes are otherwise stored in a HashMap,
/// whose iteration order is non-deterministic across processes — fine for tags
/// that emit key:value struct literals (order-irrelevant), but NOT for
/// `<cfproperty>`, whose attribute order is preserved into component metadata
/// and feeds identity hashes (e.g. Preside's FK constraint names are
/// `Hash(SerializeJson(property))`). Emitting attributes in source order keeps
/// that hash stable run-to-run and matches Lucee.
fn parse_tag_attributes_ordered(
    chars: &[char],
    start: usize,
    len: usize,
) -> (std::collections::HashMap<String, String>, std::collections::HashSet<String>, Vec<String>, usize) {
    let mut attrs = std::collections::HashMap::new();
    let mut quoted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut order: Vec<String> = Vec::new();
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
                let raw_val: String = chars[val_start..i].iter().collect();
                // Store the DECODED value — `""`/`''` are source escaping for
                // the delimiting quote, not part of the value (GH #287).
                let val = decode_attr_escapes(&raw_val, quote);
                if i < len {
                    i += 1; // skip closing quote
                }
                let key = attr_name.to_lowercase();
                quoted.insert(key.clone());
                if !attrs.contains_key(&key) {
                    order.push(key.clone());
                }
                attrs.insert(key, val);
            } else {
                // Unquoted value
                let val_start = i;
                while i < len && !chars[i].is_whitespace() && chars[i] != '>' {
                    i += 1;
                }
                let val: String = chars[val_start..i].iter().collect();
                let key = attr_name.to_lowercase();
                if !attrs.contains_key(&key) {
                    order.push(key.clone());
                }
                attrs.insert(key, val);
            }
        } else if !attr_name.is_empty() {
            let key = attr_name.to_lowercase();
            if !attrs.contains_key(&key) {
                order.push(key.clone());
            }
            attrs.insert(key, String::new());
        }

        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
    }

    // Find the actual end of the tag
    let tag_end = find_tag_end(chars, i, len);
    (attrs, quoted, order, tag_end)
}

/// Decode the CFML source escaping of a *quoted* tag attribute value (GH #287).
///
/// `parse_tag_attributes_ordered` scans past a doubled delimiter (`""` inside a
/// double-quoted attribute, `''` inside a single-quoted one) as an escape but
/// stores the raw source slice, so the stored value is still source-escaped.
/// Every downstream consumer — `format_attr_value`'s `escape_literal_segment`,
/// and the handful of tags that build their own string literal with
/// `.replace('"', "\"\"")` — then escapes it a *second* time, so
/// `<cflog text="p ""q"" r">` logged `p ""q"" r`. Decoding here, where the
/// delimiting quote is still known, makes the stored value the real CFML value
/// and leaves exactly one escaping pass downstream.
///
/// `#...#` interpolations are left untouched: quotes doubled inside an
/// interpolated expression belong to a *nested* string literal
/// (`text="#f("a""b")#"`) and are the expression parser's to decode. `##` stays
/// doubled for the same reason — `format_attr_value` decodes it when it splits
/// literal segments from expressions.
fn decode_attr_escapes(raw: &str, quote: char) -> String {
    if !raw.contains(quote) {
        return raw.to_string();
    }
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        let c = chars[i];
        if c == '#' {
            if i + 1 < len && chars[i + 1] == '#' {
                out.push_str("##"); // escaped literal hash — leave for format_attr_value
                i += 2;
                continue;
            }
            let end = skip_interpolation(&chars, i, len);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if c == quote && i + 1 < len && chars[i + 1] == quote {
            out.push(quote);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
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
    // Pure `#expr#` (whole value is one expression) — preserve native type
    // rather than coercing through string concat. Custom-tag attrs in
    // particular need this so attributeCollection="#someStruct#" arrives as
    // a struct, not its stringified form.
    if let Some(inner) = single_hash_expr(raw) {
        return inner;
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
                parts.push(format!("\"{}\"", escape_literal_segment(&buf)));
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
        parts.push(format!("\"{}\"", escape_literal_segment(&buf)));
    }
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        parts.join(" & ")
    }
}

/// If `raw` is exactly one `#expr#` spanning the whole value (no surrounding
/// literal text and no second `#...#` segment), return the inner expression.
/// Such a value keeps the expression's native type instead of being coerced to a
/// string — e.g. cfparam default="#[]#" must yield an array, not "", and
/// `attributeCollection="#getQueryAttrs(...)#"` must arrive as a struct.
///
/// A nested `#...#` living inside a string literal *within* the expression is
/// fine: `#f(x='#g()#')#` is still ONE expression. We locate the hash that
/// closes the opening `#` with `find_closing_hash` (string/paren aware) and only
/// treat the value as a single expression when that closing hash is the very
/// last character.
fn single_hash_expr(raw: &str) -> Option<String> {
    if !raw.starts_with('#') || raw.len() < 2 {
        return None;
    }
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let close = find_closing_hash(&chars, 1, len)?;
    if close != len - 1 {
        return None;
    }
    let inner: String = chars[1..close].iter().collect();
    if inner.is_empty() {
        return None;
    }
    Some(inner)
}

/// Remove CFML comments (`<!--- ... --->`) from a string, honouring nesting
/// (CFML allows `<!--- outer <!--- inner ---> --->`). An unclosed comment is
/// dropped through end-of-input, matching the document-level stripper.
fn strip_cfml_comments(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let is_open = |k: usize| {
        k + 4 < len && chars[k] == '<' && chars[k + 1] == '!'
            && chars[k + 2] == '-' && chars[k + 3] == '-' && chars[k + 4] == '-'
    };
    let is_close = |k: usize| {
        k + 2 < len && chars[k] == '-' && chars[k + 1] == '-' && chars[k + 2] == '>'
    };
    while i < len {
        if is_open(i) {
            let mut depth = 1usize;
            i += 5;
            while i < len && depth > 0 {
                if is_open(i) {
                    depth += 1;
                    i += 5;
                } else if is_close(i) {
                    depth -= 1;
                    i += 3;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn escape_for_string_literal(s: &str) -> String {
    // CFML double-quoted strings escape `"` by doubling. We emit script-style
    // string literals into generated code that goes through the script parser;
    // the script parser supports both `\"` and `""`. Use `""` to keep it CFML-native.
    s.replace('"', "\"\"")
}

/// Escape a value that is KNOWN to be a pure literal segment (no interpolation)
/// for emission into a generated script double-quoted string. In addition to
/// doubling `"`, a literal `#` must be doubled to `##` — otherwise the script
/// lexer reads it as the start of `#expr#` interpolation. Used for attribute
/// literal segments where any `#...#` interpolation has already been split out
/// into separate expression parts, so every remaining `#` is genuinely literal.
fn escape_literal_segment(s: &str) -> String {
    s.replace('"', "\"\"").replace('#', "##")
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
        return format!("\"{}\"", escape_for_string_literal(s));
    }
    if s.contains('(') || s.contains('+') || s.contains('-') || s.contains('*')
        || s.contains('/') || s.contains('&') || s.contains('.') || s.contains('[')
    {
        return s.to_string();
    }
    // Otherwise, quote it. Embedded quotes are doubled (CFML escape), not
    // backslash-escaped — the script lexer ends a string at a lone `"`.
    format!("\"{}\"", escape_for_string_literal(s))
}

fn strip_hashes(s: &str) -> String {
    let s = s.trim();
    // If the entire string is wrapped in #...#, just strip outer hashes
    if s.starts_with('#') && s.ends_with('#') && s.len() > 2 && s[1..s.len()-1].find('#').is_none() {
        return s[1..s.len() - 1].to_string();
    }
    // Handle embedded #expr# within larger expressions
    // Replace #expr# with just expr (strip the hash delimiters), but preserve
    // hashes that sit inside string literals (CFML uses `#expr#` for string
    // interpolation and the script lexer will handle it later).
    if !s.contains('#') {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0;
    let mut string_quote: Option<char> = None;
    while i < len {
        let c = chars[i];
        if let Some(q) = string_quote {
            // A `#...#` interpolation inside the string is EXPRESSION context:
            // its contents (including nested quotes, e.g. `'#gv('a')#'`) must not
            // be mistaken for the string's closing delimiter. Copy the whole
            // interpolation across verbatim using the same string/paren-aware
            // scanner the rest of the preprocessor uses. Without this, the first
            // inner quote flipped `string_quote` off mid-string and the hashes
            // were then mangled — producing an "unterminated '#'" from Mura/Masa
            // expressions like `'#iif(a eq '',de('#x#'),de('#y# #z#'))#'`.
            if c == '#' {
                result.push(c);
                if let Some(end) = find_closing_hash(&chars, i + 1, len) {
                    for k in (i + 1)..=end {
                        result.push(chars[k]);
                    }
                    i = end + 1;
                    continue;
                }
                i += 1;
                continue;
            }
            // Inside a string literal — preserve everything, watch for the
            // closing quote (with `""`/`''` doubling treated as an escape).
            result.push(c);
            if c == q {
                if i + 1 < len && chars[i + 1] == q {
                    result.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                string_quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            string_quote = Some(c);
            result.push(c);
            i += 1;
            continue;
        }
        if c == '#' {
            // Look for closing # at the same nesting level (skip string contents).
            let mut j = i + 1;
            let mut inner_quote: Option<char> = None;
            while j < len {
                let cj = chars[j];
                if let Some(q) = inner_quote {
                    if cj == q {
                        if j + 1 < len && chars[j + 1] == q {
                            j += 2;
                            continue;
                        }
                        inner_quote = None;
                    }
                    j += 1;
                    continue;
                }
                if cj == '"' || cj == '\'' {
                    inner_quote = Some(cj);
                    j += 1;
                    continue;
                }
                if cj == '#' {
                    break;
                }
                j += 1;
            }
            if j < len && chars[j] == '#' {
                let expr: String = chars[i + 1..j].iter().collect();
                result.push_str(&expr);
                i = j + 1;
                continue;
            }
            result.push(c);
            i += 1;
            continue;
        }
        result.push(c);
        i += 1;
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

/// Scan ahead from current position to find `<cfargument>` tags and build the
/// script-form parameter declarations for the enclosing `<cffunction>`.
///
/// Each declaration mirrors CFScript param syntax — `[required] [type] name
/// attr="val"...` — so that the type, the `required` flag, AND any custom
/// attributes (notably WireBox's `inject="coldbox"`) survive into the parsed
/// `Param` (its `type`, `required`, and `annotations`). Previously only the
/// bare name was emitted, so `getMetadata().functions[].parameters[].inject`
/// came back absent and frameworks that drive constructor injection off that
/// annotation (ColdBox/WireBox `buildArgumentCollection`) silently built every
/// tag-based CFC with ZERO constructor arguments. The `default` attribute is
/// NOT emitted here — the `cfargument` tag converter handles it as a body
/// `if (isNull(arguments.x)) {...}` — and an argument carrying a default is
/// treated as optional regardless of a `required="true"` attribute.
fn scan_cfargument_tags(chars: &[char], start: usize, len: usize) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = start;

    while i < len {
        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        // Check if we hit a <cfargument. The tag name must be followed by a
        // word boundary — ANY whitespace (space, newline, tab) or `>` — not just
        // a literal space: MockBox-style multi-line `<cfargument\n  name="..."/>`
        // declarations put a newline after the name, and a space-only check
        // silently dropped them from the generated parameter list (GitHub #177).
        if i + 11 <= len && chars[i] == '<' {
            let tag: String = chars[i..i + 11].iter().collect();
            let boundary = chars
                .get(i + 11)
                .is_some_and(|c| c.is_whitespace() || *c == '>' || *c == '/');
            if tag.eq_ignore_ascii_case("<cfargument") && boundary {
                // Parse the tag's attributes
                let name_start = i + 1; // skip <
                let mut j = name_start;
                while j < len && chars[j].is_alphanumeric() {
                    j += 1;
                }
                let (tag_attrs, _quoted, attr_order, _) =
                    parse_tag_attributes_ordered(chars, j, len);
                if let Some(name) = tag_attrs.get("name") {
                    let mut decl = String::new();
                    let has_default = tag_attrs.contains_key("default");
                    let is_required = tag_attrs
                        .get("required")
                        .map(|v| {
                            matches!(
                                strip_hashes(v).trim().to_lowercase().as_str(),
                                "true" | "yes" | "1"
                            )
                        })
                        .unwrap_or(false);
                    // A defaulted argument is optional even if marked required.
                    if is_required && !has_default {
                        decl.push_str("required ");
                    }
                    // Always emit an explicit type token. The script param
                    // grammar needs `type name` (two barewords) to disambiguate
                    // an annotation (`name attr="v"`) from a defaulted param
                    // (`type name="v"`); with a single leading bareword the
                    // parser would read the name AS the type. `any` is the
                    // cfargument default type, so this stays Lucee-faithful.
                    let arg_type = tag_attrs
                        .get("type")
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty())
                        .unwrap_or("any");
                    decl.push_str(arg_type);
                    decl.push(' ');
                    decl.push_str(name);
                    // Emit every remaining attribute as a `key="value"` annotation
                    // in source order. `name`/`type`/`required`/`default` are
                    // structural and handled above; everything else (chiefly
                    // `inject`) becomes a Param annotation.
                    for key in &attr_order {
                        let kl = key.to_lowercase();
                        if matches!(kl.as_str(), "name" | "type" | "required" | "default") {
                            continue;
                        }
                        if let Some(val) = tag_attrs.get(key) {
                            // Double embedded quotes for CFML string escaping.
                            let escaped = val.replace('"', "\"\"");
                            decl.push_str(&format!(" {}=\"{}\"", key, escaped));
                        }
                    }
                    names.push(decl);
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

/// Read the `groupCaseSensitive` attribute. ACF documents the default as `Yes`,
/// but the reference engine disagrees: on Lucee 7.0.4 a `group=` break with no
/// `groupCaseSensitive` merges `eng` and `ENG` into one group, and only
/// `groupCaseSensitive="true"` splits them (probed). Defaulting to `true` here
/// split groups Lucee merges.
fn group_case_sensitive(attrs: &std::collections::HashMap<String, String>) -> bool {
    attrs
        .get("groupcasesensitive")
        .map(|v| {
            !matches!(
                strip_hashes(v).to_lowercase().as_str(),
                "false" | "no" | "0"
            )
        })
        .unwrap_or(false)
}

/// Index of the opening `<` of the first `<cfoutput ...>`/`<cfloop ...>`
/// (word-bounded) at or after `from`. Locates the nested detail block inside a
/// grouped `<cfoutput query group=>` / `<cfloop query group=>`.
fn find_nested_tag(chars: &[char], from: usize, len: usize, tag: &str) -> Option<usize> {
    let needle: Vec<char> = tag.chars().collect();
    let mut i = from;
    while i < len {
        if chars[i] == '<' {
            let name_start = i + 1;
            if name_start + needle.len() <= len
                && (0..needle.len()).all(|k| chars[name_start + k].eq_ignore_ascii_case(&needle[k]))
            {
                let after = name_start + needle.len();
                let boundary = after >= len
                    || chars[after].is_whitespace()
                    || chars[after] == '>'
                    || chars[after] == '/';
                if boundary {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Emit the control-break loop for a grouped `<cfoutput query group=>` /
/// `<cfloop query group=>` over `rows_var` (a CFML array of row structs). The
/// per-group body (everything outside the nested detail block) runs once per
/// distinct consecutive value of `group_col`; the nested block iterates that
/// group's rows, recursing for further `group=` levels. `uid`/`depth` keep
/// generated temp names unique.
///
/// `inner_tag` names the detail block: `<cfoutput>` nested in a grouped
/// cfoutput, `<cfloop>` nested in a grouped cfloop. `in_cfoutput` carries the
/// caller's interpolation context — a cfoutput body always interpolates `#…#`,
/// a cfloop body only does so inside an enclosing `<cfoutput>`.
#[allow(clippy::too_many_arguments)]
fn emit_grouped_query_body(
    rows_var: &str,
    query_var: &str,
    body: &str,
    group_col: &str,
    case_sensitive: bool,
    inner_tag: &str,
    in_cfoutput: bool,
    imports: &mut std::collections::HashMap<String, String>,
    uid: usize,
    depth: usize,
) -> String {
    let chars: Vec<char> = body.chars().collect();
    let len = chars.len();

    // Split the body into (pre, detail-block attrs, detail inner body, post).
    // For cfloop the detail block is a BARE `<cfloop>` (or one carrying only
    // `group=`/`groupCaseSensitive=` for a further level) — any other nested
    // `<cfloop from=…/query=…>` is an unrelated loop and must stay in the body.
    let detail_open = {
        let mut at = 0;
        loop {
            match find_nested_tag(&chars, at, len, inner_tag) {
                None => break None,
                Some(open) => {
                    let name_end = open + 1 + inner_tag.len();
                    let (attrs, _q, _end) = parse_tag_attributes(&chars, name_end, len);
                    let group_only = attrs.keys().all(|k| {
                        k == "group" || k == "groupcasesensitive"
                    });
                    if inner_tag != "cfloop" || group_only {
                        break Some(open);
                    }
                    at = open + 1;
                }
            }
        }
    };
    let (pre, nested_attrs, nested_inner, post) = if let Some(open) = detail_open {
        let name_end = open + 1 + inner_tag.len();
        let (attrs, _quoted, tag_end) = parse_tag_attributes(&chars, name_end, len);
        let pre: String = chars[..open].iter().collect();
        if let Some(close) = find_closing_tag(&chars, tag_end, len, inner_tag) {
            let inner: String = chars[tag_end..close].iter().collect();
            let close_end = find_tag_end(&chars, close, len);
            let post: String = chars[close_end..].iter().collect();
            (pre, Some(attrs), inner, post)
        } else {
            let inner: String = chars[tag_end..].iter().collect();
            (pre, Some(attrs), inner, String::new())
        }
    } else {
        (body.to_string(), None, String::new(), String::new())
    };

    let pre_script = tags_to_script_inner(&pre, imports, in_cfoutput);
    let post_script = tags_to_script_inner(&post, imports, in_cfoutput);

    let sfx = format!("{}_{}", uid, depth);
    let i = format!("__cfg_i_{}", sfx);
    let gval = format!("__cfg_gval_{}", sfx);
    let gstart = format!("__cfg_gstart_{}", sfx);
    let sub = format!("__cfg_sub_{}", sfx);
    let cmp = if case_sensitive { "compare" } else { "compareNoCase" };

    // The nested detail block, operating on this group's sub-array `sub`.
    let nested_script = match nested_attrs {
        None => String::new(),
        Some(attrs) => {
            if let Some(g2) = attrs.get("group") {
                let g2 = strip_hashes(g2);
                let cs2 = group_case_sensitive(&attrs);
                emit_grouped_query_body(
                    &sub, query_var, &nested_inner, &g2, cs2, inner_tag, in_cfoutput, imports,
                    uid, depth + 1,
                )
            } else {
                // Innermost block: iterate every row in the current group.
                let nested_body = tags_to_script_inner(&nested_inner, imports, in_cfoutput);
                let ri = format!("__cfg_ri_{}", sfx);
                format!(
                    "var {ri} = 1;\nwhile ({ri} <= arrayLen({sub})) {{\nstructAppend(variables, {sub}[{ri}], true);\n{q} = {sub}[{ri}];\n{body}\n{ri} = {ri} + 1;\n}}\n",
                    ri = ri,
                    sub = sub,
                    q = query_var,
                    body = nested_body,
                )
            }
        }
    };

    format!(
        "var {i} = 1;\nwhile ({i} <= arrayLen({rows})) {{\nvar {gval} = {rows}[{i}][\"{col}\"];\nvar {gstart} = {i};\nwhile ({i} <= arrayLen({rows}) && {cmp}({rows}[{i}][\"{col}\"], {gval}) == 0) {{ {i} = {i} + 1; }}\nvar {sub} = arraySlice({rows}, {gstart}, {i} - {gstart});\nstructAppend(variables, {sub}[1], true);\n{q} = {sub}[1];\n{pre}\n{nested}{post}\n}}\n",
        i = i,
        rows = rows_var,
        gval = gval,
        gstart = gstart,
        col = group_col,
        cmp = cmp,
        sub = sub,
        q = query_var,
        pre = pre_script,
        nested = nested_script,
        post = post_script,
    )
}

/// Lower a `<cfloop>` opening tag to its CFScript equivalent.
///
/// `consumed` is the tag's byte length (returned verbatim to the caller).
/// `uniq` is the tag's **start offset**, used to name the `__cfloop_*` temps.
/// These must be separate: `consumed` is a length, so two nested loops written
/// with the same-length opening tag (`<cfloop times="2">` inside
/// `<cfloop times="3">`) produced *identical* temp names, and the inner loop
/// clobbered the outer loop's counter — the outer body ran once instead of N
/// times. The start offset is unique per tag occurrence.
fn parse_cfloop_tag(
    attrs: &std::collections::HashMap<String, String>,
    consumed: usize,
    uniq: usize,
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
        // Decide loop direction at runtime from the sign of the (possibly
        // dynamic) step, matching Lucee. Hoist step into a temp so it is
        // evaluated once rather than per-iteration.
        let step_var = format!("__cfloop_step_{}", uniq);
        // A C-style `for (var X = …; …)` declaration only accepts a SIMPLE
        // name. When the index is scope-qualified (e.g. `index="local.i"`,
        // common in Wheels), emitting `var local.i = 1` mis-parses — the
        // initializer is dropped, so the counter starts empty, lags by one,
        // and over-runs (in nested loops this corrupts the `local` scope and
        // spins forever, issue #188). Lucee never emits `var` for a scoped
        // index — it just assigns to that scope. Mirror that: drop `var` when
        // the index carries a scope prefix. (for-in forms below tolerate
        // `var local.x` fine, so only this C-style branch needs the guard.)
        let var_kw = if index.contains('.') { "" } else { "var " };
        (
            format!(
                "var {sv} = {step};\nfor ({vk}{i} = {from}; ({sv} < 0 ? {i} >= {to} : {i} <= {to}); {i} = {i} + {sv}) {{\n",
                sv = step_var,
                step = step,
                vk = var_kw,
                i = index,
                from = from,
                to = to
            ),
            consumed,
        )
    } else if let Some(times) = attrs.get("times") {
        // <cfloop times="N"> repeats the body N times (Lucee 5+). Mirrors the
        // script-tag lowering in `parser.rs` (branch "2) times=N"); the two
        // lowerings must agree — this one was missing, so the tag form fell
        // through to the `while (true)` fallback at the bottom of this function
        // and HUNG, appending to the output buffer until the process died
        // (12 GB observed on `<cfloop times="3">X</cfloop>`).
        // Lucee rejects `times` combined with anything else: "Wrong Context,
        // when you use attribute times, no other attributes are allowed"
        // (verified against Lucee 7.0.4). In particular there is no `index`
        // binding — `<cfloop times="3" index="i">` is a compile error, not a
        // counted loop. Mirror that rather than inventing a superset.
        if attrs.len() > 1 {
            let mut others: Vec<&str> =
                attrs.keys().map(|k| k.as_str()).filter(|k| *k != "times").collect();
            others.sort_unstable();
            return cfloop_form_error(&format!(
                "Wrong Context, when you use attribute times, no other attributes are allowed (found: {})",
                others.join(", ")
            ), consumed);
        }
        let times = strip_hashes(times);
        let counter = format!("__cfloop_times_{}", uniq);
        let limit = format!("__cfloop_timesmax_{}", uniq);
        // Hoist the bound into a temp so a dynamic expression is evaluated once
        // rather than on every iteration (mirrors `step` in the from/to branch,
        // and matches Lucee — mutating the source variable inside the body does
        // not change the trip count there either).
        (
            format!(
                "var {lim} = {times};\nfor (var {c} = 1; {c} <= {lim}; {c} = {c} + 1) {{\n",
                lim = limit,
                times = times,
                c = counter
            ),
            consumed,
        )
    } else if let Some(condition) = attrs.get("condition") {
        let condition = strip_hashes(condition);
        (format!("while ({}) {{\n", condition), consumed)
    } else if let Some(array) = attrs.get("array") {
        let array = strip_hashes(array);
        if let (Some(item), Some(index)) = (attrs.get("item"), attrs.get("index")) {
            let item = strip_hashes(item);
            let index = strip_hashes(index);
            let array_var = format!("__cfloop_array_{}", uniq);
            let index_var = format!("__cfloop_index_{}", uniq);
            (
                format!(
                    "var {} = {};\nfor (var {} = 1; {} <= arrayLen({}); {} = {} + 1) {{\n{} = {};\n{} = {}[{}];\n",
                    array_var,
                    array,
                    index_var,
                    index_var,
                    array_var,
                    index_var,
                    index_var,
                    index,
                    index_var,
                    item,
                    array_var,
                    index_var
                ),
                consumed,
            )
        } else if let Some(item) = attrs.get("item").or_else(|| attrs.get("index")) {
            let item = strip_hashes(item);
            (format!("for (var {} in {}) {{\n", item, array), consumed)
        } else {
            (
                "throw(\"cfloop array requires an item or index attribute.\");\n".to_string(),
                consumed,
            )
        }
    } else if let (Some(list), Some(index)) =
        (attrs.get("list"), attrs.get("index").or_else(|| attrs.get("item")))
    {
        // With only ONE of `item`/`index`, that attribute names the current
        // element binding (`item` is the modern spelling, bare `index` the
        // legacy one). With BOTH present, Lucee splits them: `item` = the
        // element, `index` = the 1-based position.
        // The `list` attribute is ALWAYS a literal string on Lucee/Adobe CF —
        // only `#expr#` interpolation makes it dynamic. Quote it verbatim so
        // values containing operators or spaces (e.g. date masks like
        // "yyyy-MM-dd HH:mm:ss") survive intact rather than being mis-parsed as
        // an expression by quote_if_needed's heuristic.
        let list = if list.contains('#') {
            strip_hashes(list)
        } else {
            format!("\"{}\"", escape_for_string_literal(list))
        };
        let delimiters = attrs
            .get("delimiters")
            .cloned()
            .unwrap_or(",".to_string());
        if let (Some(item), Some(idx)) = (attrs.get("item"), attrs.get("index")) {
            let item = strip_hashes(item);
            let idx = strip_hashes(idx);
            let list_var = format!("__cfloop_list_{}", uniq);
            let counter = format!("__cfloop_listidx_{}", uniq);
            (
                format!(
                    "var {} = listToArray({}, \"{}\");\nfor (var {} = 1; {} <= arrayLen({}); {} = {} + 1) {{\n{} = {};\n{} = {}[{}];\n",
                    list_var,
                    list,
                    delimiters,
                    counter,
                    counter,
                    list_var,
                    counter,
                    counter,
                    idx,
                    counter,
                    item,
                    list_var,
                    counter
                ),
                consumed,
            )
        } else {
            let index = strip_hashes(index);
            (
                format!(
                    "for (var {} in listToArray({}, \"{}\")) {{\n",
                    index, list, delimiters
                ),
                consumed,
            )
        }
    } else if let Some(query) = attrs.get("query") {
        let query = strip_hashes(query);
        if let Some(index) = attrs.get("index").or(attrs.get("item")) {
            (
                format!("for (var {} in {}) {{\n", index, query),
                consumed,
            )
        } else {
            // <cfloop query="q"> without index — CFML query row loop. NOTE: the
            // real handling (body capture + query-var restore + currentRow/
            // recordCount/columnList pseudo-columns) lives in the `"cfloop"` arm
            // of `parse_cf_tag`, which intercepts this shape before we get here.
            // This branch is a defensive fallback only (e.g. a direct caller).
            (
                format!("for (var __qrow in {}) {{ {} = __qrow;\n", query, query),
                consumed,
            )
        }
    } else if let Some(collection) = attrs.get("collection") {
        let collection = strip_hashes(collection);
        let item = attrs.get("item").cloned().unwrap_or("item".to_string());
        // `index` is a Lucee alias for `key` in collection loops: it names the
        // loop key while `item` names the value. (`item` alone iterates keys.)
        let key = attrs.get("key").or_else(|| attrs.get("index"));
        if let Some(key) = key {
            (
                format!("for (var {} in structKeyArray({})) {{ var {} = {}[{}];\n", key, collection, item, collection, key),
                consumed,
            )
        } else {
            (format!("for (var {} in {}) {{\n", item, collection), consumed)
        }
    } else if let (Some(file), Some(index)) = (
        attrs.get("file"),
        attrs.get("index").or_else(|| attrs.get("item")),
    ) {
        // <cfloop file="path" index="line"> iterates the file line by line,
        // binding `index` to each line. `__cfloop_file_lines` reads via the VFS
        // and preserves empty lines (accurate line numbering). Without this the
        // file form fell through to `while(true)` and hung (GitHub issue #158).
        let file = strip_hashes(file);
        let index = strip_hashes(index);
        (
            format!("for (var {} in __cfloop_file_lines({})) {{\n", index, file),
            consumed,
        )
    } else {
        // No recognised loop form. This used to emit `while (true) {`, which
        // turned every unimplemented `<cfloop>` attribute into a silent hang —
        // strictly worse than a no-op, because the output buffer grows without
        // bound until the process is killed (issue #158 hit it via `file=`,
        // `times=` hit it again). Fail loudly instead.
        //
        // Two layers, because `tags_to_script` is the tolerant entry point and
        // must still return balanced script: record a structural error so
        // `tags_to_script_checked` reports it at compile time, AND emit a block
        // that throws at runtime so the tolerant path can never hang either.
        // The emitted block is still a `{`-opener — `</cfloop>` unconditionally
        // emits the matching `}`.
        let form = {
            let mut keys: Vec<&str> = attrs.keys().map(|k| k.as_str()).collect();
            keys.sort_unstable();
            keys.join(", ")
        };
        let msg = if form.is_empty() {
            "<cfloop> requires a loop form (from/to, times, condition, array, list, query, collection or file); none was given".to_string()
        } else {
            format!(
                "<cfloop> has no recognised loop form for attribute(s): {}. Expected one of from/to, times, condition, array, list, query, collection or file",
                form
            )
        };
        cfloop_form_error(&msg, consumed)
    }
}

/// Emit a `<cfloop>` that fails loudly instead of looping.
///
/// Two layers, because [`tags_to_script`] is the tolerant entry point and must
/// still return balanced script: record a structural error so
/// [`tags_to_script_checked`] reports it at compile time, AND emit a block that
/// throws at runtime so the tolerant path can never hang either. The emitted
/// block is a `{`-opener — `</cfloop>` unconditionally emits the matching `}`.
fn cfloop_form_error(msg: &str, consumed: usize) -> (String, usize) {
    record_preprocess_error(msg.to_string());
    (
        format!(
            "if (true) {{ throw(type=\"expression\", message=\"{}\");\n",
            msg.replace('\\', "\\\\").replace('"', "\\\"")
        ),
        consumed,
    )
}

/// Parse the body of a <cfswitch> tag, scanning for <cfcase> and <cfdefaultcase>
fn parse_cfswitch_body(body: &str, imports: &mut std::collections::HashMap<String, String>, in_cfoutput: bool) -> String {
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
                    let case_script = tags_to_script_inner(&case_body, imports, in_cfoutput);
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
                    let case_script = tags_to_script_inner(&case_body, imports, in_cfoutput);
                    // Value can be comma-separated for multiple case values.
                    // `<cfcase value="">` is legal and matches the empty string —
                    // a SINGLE `case "":` value (Mura/Masa's setCookieLegacy uses
                    // it). The empty-filter is only for skipping empties WITHIN a
                    // comma list (e.g. `value="a,,b"`); when the whole value is
                    // empty it must still emit one `""` case, not a valueless
                    // `case :` that corrupts the switch and leaks a parse error.
                    let quoted_values: Vec<String> = if value.trim().is_empty() {
                        vec!["\"\"".to_string()]
                    } else {
                        value
                            .split(',')
                            .map(|v| v.trim())
                            .filter(|v| !v.is_empty())
                            .map(|v| {
                                let v = strip_hashes(v);
                                if v.parse::<f64>().is_ok() {
                                    v
                                } else {
                                    format!("\"{}\"", escape_for_string_literal(&v))
                                }
                            })
                            .collect()
                    };
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
    quoted: &std::collections::HashSet<String>,
    consumed: usize,
) -> (String, usize) {
    let action = attrs.get("action").cloned().unwrap_or("read".to_string()).to_lowercase();

    // Route a path/value attribute through format_attr_value so `#...#`
    // segments interpolate and literal text is preserved — uniform with
    // cfparam (Lucee parity). A quoted value that is exactly one `#expr#`
    // keeps the expression's native type; an unquoted value falls back to the
    // expr-or-literal heuristic. Replaces the prior `.`/`(` guesswork that
    // mis-quoted single-variable paths and mis-parsed literal paths.
    let attr_expr = |key: &str| -> String {
        match attrs.get(key) {
            Some(raw) if quoted.contains(key) => match single_hash_expr(raw) {
                Some(expr) => format!("({})", expr),
                None => format_attr_value(raw, true),
            },
            Some(raw) => format_attr_value(raw, false),
            None => "\"\"".to_string(),
        }
    };

    match action.as_str() {
        "read" => {
            let variable = attrs.get("variable").cloned().unwrap_or("cffile".to_string());
            (format!("{} = fileRead({});\n", variable, attr_expr("file")), consumed)
        }
        "readbinary" => {
            let variable = attrs.get("variable").cloned().unwrap_or("cffile".to_string());
            (format!("{} = fileReadBinary({});\n", variable, attr_expr("file")), consumed)
        }
        "write" => {
            (format!("fileWrite({}, {});\n", attr_expr("file"), attr_expr("output")), consumed)
        }
        "append" => {
            (format!("fileAppend({}, {});\n", attr_expr("file"), attr_expr("output")), consumed)
        }
        // `nameConflict` rides as a third argument: overwrite (the default),
        // skip, error, or makeunique. It used to be dropped here, so every
        // conflicting copy/move silently overwrote (docs known-issues §27).
        "copy" => {
            (format!("fileCopy({}, {}, {});\n", attr_expr("source"), attr_expr("destination"), attr_expr("nameconflict")), consumed)
        }
        "move" | "rename" => {
            (format!("fileMove({}, {}, {});\n", attr_expr("source"), attr_expr("destination"), attr_expr("nameconflict")), consumed)
        }
        "delete" => {
            (format!("fileDelete({});\n", attr_expr("file")), consumed)
        }
        "upload" | "uploadall" => {
            let func = if action == "uploadall" { "__cffile_upload" } else { "__cffile_upload" };
            let mut parts = Vec::new();
            for (k, v) in attrs {
                if k == "action" { continue; }
                parts.push(format!("{}: {}", k, format_attr_value(v, quoted.contains(k.as_str()))));
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

/// Build a `CfQueryParam` from a parsed `<cfqueryparam>` attribute map.
fn parse_cfqueryparam_attrs(tag_attrs: &std::collections::HashMap<String, String>) -> CfQueryParam {
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
    } else if value_raw.is_empty() {
        "\"\"".to_string()
    } else if !value_raw.contains('#') {
        // Pure literal string
        format!("\"{}\"", escape_for_string_literal(&value_raw))
    } else {
        let trimmed = value_raw.trim();
        let is_pure_expr = trimmed.starts_with('#')
            && trimmed.ends_with('#')
            && trimmed.len() > 2
            && trimmed[1..trimmed.len() - 1].find('#').is_none();
        if is_pure_expr {
            // Single bare expression — emit unquoted
            trimmed[1..trimmed.len() - 1].to_string()
        } else {
            // Mixed literal text + #expr# interpolation — emit as a quoted
            // CFML string with hashes intact; the lexer handles `#expr#`
            // interpolation inside double-quoted strings. Quotes are doubled
            // (escape_for_string_literal), not backslash-escaped.
            format!("\"{}\"", escape_for_string_literal(&value_raw))
        }
    };

    CfQueryParam {
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
    }
}

/// Format a `CfQueryParam` as a CFScript struct literal for the queryExecute
/// params array.
fn cfqueryparam_to_literal(p: &CfQueryParam) -> String {
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
}

/// Does a cfquery body contain control-flow / script tags (anything other than
/// plain SQL text and `<cfqueryparam>`)? If so the SQL and bound-param set can
/// vary at runtime and must be built by executing the body.
fn body_has_control_flow(body: &str) -> bool {
    body_has_control_flow_except(body, &["cfqueryparam"])
}

/// Emit a `{ type:..., name:..., value:..., file:..., encoded:..., mimeType:... }`
/// struct literal from parsed `<cfhttpparam>` attributes.
fn cfhttpparam_attrs_to_literal(
    tag_attrs: &std::collections::HashMap<String, String>,
    quoted: &std::collections::HashSet<String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(t) = tag_attrs.get("type") {
        parts.push(format!("type: \"{}\"", t.to_lowercase()));
    }
    if let Some(n) = tag_attrs.get("name") {
        parts.push(format!("name: {}", format_attr_value(n, quoted.contains("name"))));
    }
    if let Some(v) = tag_attrs.get("value") {
        parts.push(format!("value: {}", format_attr_value(v, quoted.contains("value"))));
    }
    if let Some(f) = tag_attrs.get("file") {
        parts.push(format!("file: {}", format_attr_value(f, quoted.contains("file"))));
    }
    if let Some(e) = tag_attrs.get("encoded") {
        parts.push(format!("encoded: \"{}\"", e));
    }
    if let Some(m) = tag_attrs.get("mimetype") {
        parts.push(format!("mimeType: \"{}\"", m));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Emit a `{ name:..., value:..., file:..., type:..., disposition:... }` struct
/// literal from parsed `<cfmailparam>` attributes.
fn cfmailparam_attrs_to_literal(
    tag_attrs: &std::collections::HashMap<String, String>,
    quoted: &std::collections::HashSet<String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(n) = tag_attrs.get("name") {
        parts.push(format!("name: {}", format_attr_value(n, quoted.contains("name"))));
    }
    if let Some(v) = tag_attrs.get("value") {
        parts.push(format!("value: {}", format_attr_value(v, quoted.contains("value"))));
    }
    if let Some(f) = tag_attrs.get("file") {
        parts.push(format!("file: {}", format_attr_value(f, quoted.contains("file"))));
    }
    if let Some(t) = tag_attrs.get("type") {
        parts.push(format!("type: \"{}\"", t.to_lowercase()));
    }
    if let Some(d) = tag_attrs.get("disposition") {
        parts.push(format!("disposition: \"{}\"", d));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Emit a `<cfmailpart>` attrs-only literal (everything except body, which is
/// captured separately via savecontent in the runtime path).
fn cfmailpart_attrs_only_literal(
    tag_attrs: &std::collections::HashMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(t) = tag_attrs.get("type") {
        parts.push(format!("type: \"{}\"", t.to_lowercase()));
    }
    if let Some(c) = tag_attrs.get("charset") {
        parts.push(format!("charset: \"{}\"", c));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Emit a `{ value:..., cfsqltype:... }` struct literal from parsed
/// `<cfprocparam>` attributes.
fn cfprocparam_attrs_to_literal(
    tag_attrs: &std::collections::HashMap<String, String>,
    quoted: &std::collections::HashSet<String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(v) = tag_attrs.get("value") {
        parts.push(format!("value: {}", format_attr_value(v, quoted.contains("value"))));
    }
    if let Some(t) = tag_attrs.get("cfsqltype") {
        parts.push(format!("cfsqltype: \"{}\"", t));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Same as `body_has_control_flow` but treats the named cf-tags as inert
/// (i.e. they don't count as control flow). Used by the container-tag
/// runtime-vs-static decision: `cfhttp`/`cfmail`/`cfstoredproc` bodies
/// holding only their own child tags can stay on the cheap static-scan
/// path, while bodies wrapping those children in `<cfif>`/`<cfloop>`
/// must switch to runtime assembly.
fn body_has_control_flow_except(body: &str, allowed: &[&str]) -> bool {
    let lower = body.to_lowercase();
    let mut search = lower.as_str();
    while let Some(pos) = search.find("<cf") {
        let rest = &search[pos + 1..]; // after '<'
        let name: String = rest.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !allowed.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            return true;
        }
        search = &search[pos + 3..];
    }
    false
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

                    params.push(parse_cfqueryparam_attrs(&tag_attrs));

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
    // Preserve line breaks: SQL `--` comments are line-scoped, so flattening
    // newlines to spaces would make the comment swallow the rest of the
    // statement (e.g. `SELECT a\n-- note\nFROM t` -> `SELECT a -- note FROM t`).
    // Normalize CRLF / bare CR to LF so the generated string keeps real breaks.
    let sql = sql.trim().replace("\r\n", "\n").replace('\r', "\n");

    if !sql.contains('#') {
        // No hash expressions — simple string literal. CFML escapes an embedded
        // double-quote by DOUBLING it (""), not with a backslash; SQL bodies use
        // double-quoted identifiers (AS "where"), so a backslash escape would
        // terminate the string early and fail to parse.
        return format!("\"{}\"", sql.replace('"', "\"\""));
    }

    // Split on hash pairs and build concatenation
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut current_text = String::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '#' {
            // Look for the MATCHING closing `#`, skipping `#` that live inside
            // nested string literals (and honouring paren depth). A naive
            // "next `#`" scan split a nested interpolation like
            // `#right("IX_#arguments.table#",30)#` at the inner `#`, extracting
            // the malformed expression `right("IX_` and failing to parse — this
            // is exactly what broke Mura/Masa's `dbCreateIndex` SQL. Reuse the
            // same string-aware scanner the general interpolation path uses.
            if let Some(end) = find_closing_hash(&chars, i + 1, len) {
                // Flush current text
                if !current_text.is_empty() {
                    parts.push(format!("\"{}\"", current_text.replace('"', "\"\"")));
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
        parts.push(format!("\"{}\"", current_text.replace('"', "\"\"")));
    }

    if parts.len() == 1 {
        parts.into_iter().next().unwrap()
    } else {
        parts.join(" & ")
    }
}

/// Parse <cfhttpparam> tags from the body of a <cfhttp> tag.
/// Returns a vector of struct literal strings like: { type: "header", name: "X-Custom", value: "foo" }
/// NOTE (issue #55): this is a compile-time scan — it does not execute control
/// flow, so cfhttpparams inside a <cfloop>/<cfif> are missed. Needs the runtime
/// param-building treatment cfquery received in 28af97d.
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
                    let (tag_attrs, quoted, _) = parse_tag_attributes(&chars, name_end, len);

                    let mut parts = Vec::new();
                    if let Some(t) = tag_attrs.get("type") {
                        parts.push(format!("type: \"{}\"", t.to_lowercase()));
                    }
                    // name/value/file may contain #expr# interpolation inside an
                    // otherwise-literal quoted string (e.g. value="value-#x#").
                    // format_attr_value emits literal segments quoted and only
                    // evaluates the #...# parts, instead of strip_hashes turning the
                    // whole thing into a bare (mis-parsed) expression.
                    if let Some(n) = tag_attrs.get("name") {
                        parts.push(format!("name: {}", format_attr_value(n, quoted.contains("name"))));
                    }
                    if let Some(v) = tag_attrs.get("value") {
                        parts.push(format!("value: {}", format_attr_value(v, quoted.contains("value"))));
                    }
                    if let Some(f) = tag_attrs.get("file") {
                        parts.push(format!("file: {}", format_attr_value(f, quoted.contains("file"))));
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
                    let (tag_attrs, quoted, _) = parse_tag_attributes(&chars, name_end, len);

                    let mut parts = Vec::new();
                    // name/value/file may contain #expr# interpolation inside an
                    // otherwise-literal quoted string (e.g. value="x-#var#").
                    // format_attr_value emits literal segments quoted and only
                    // evaluates the #...# parts, instead of strip_hashes turning the
                    // whole thing into a bare (mis-parsed) expression.
                    if let Some(n) = tag_attrs.get("name") {
                        parts.push(format!("name: {}", format_attr_value(n, quoted.contains("name"))));
                    }
                    if let Some(v) = tag_attrs.get("value") {
                        parts.push(format!("value: {}", format_attr_value(v, quoted.contains("value"))));
                    }
                    if let Some(f) = tag_attrs.get("file") {
                        parts.push(format!("file: {}", format_attr_value(f, quoted.contains("file"))));
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
                            part_parts.push(format!("body: \"{}\"", content_trimmed.replace('"', "\"\"")));
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

    /// Every `<cfhttp>` attribute must reach the options struct. The lowering
    /// used to copy a fixed ten-key whitelist, so `name=`/`file=`/`path=` (and
    /// throwOnError/redirect/port/proxyPort/encodeURL, all implemented in the
    /// runtime) were discarded at compile time — docs known-issues §27.
    /// `<cfdump>` forwarded only var/label/expand/top, so `output=` (console or
    /// a file) and `abort=` were discarded — docs known-issues §27.
    #[test]
    fn test_cfdump_forwards_output_and_abort() {
        let result = tags_to_script(
            r##"<cfdump var="#x#" label="L" output="console" abort="true" top="3">"##,
        );
        for key in ["label=", "output=", "abort=", "top="] {
            assert!(result.contains(key), "cfdump dropped `{}`: {}", key, result);
        }
    }

    /// `<cffile action="copy"/"move">` dropped `nameConflict=`, so every
    /// conflicting copy silently overwrote — docs known-issues §27.
    #[test]
    fn test_cffile_forwards_nameconflict() {
        let copied = tags_to_script(
            r#"<cffile action="copy" source="a.txt" destination="b.txt" nameConflict="makeunique">"#,
        );
        assert!(copied.contains(r#"fileCopy("a.txt", "b.txt", "makeunique")"#), "unexpected: {copied}");
        let moved = tags_to_script(
            r#"<cffile action="move" source="a.txt" destination="b.txt" nameConflict="error">"#,
        );
        assert!(moved.contains(r#"fileMove("a.txt", "b.txt", "error")"#), "unexpected: {moved}");
    }

    /// `<cfinvoke webservice=…>` used to pass the WSDL URL as a method ARGUMENT
    /// with an empty component. There is no SOAP client, so say so — at runtime,
    /// not compile time, so an app containing an unreached SOAP call still starts.
    #[test]
    fn test_cfinvoke_webservice_throws_at_runtime() {
        let result = tags_to_script(
            r#"<cfinvoke webservice="http://x/y?wsdl" method="m" returnvariable="r">"#,
        );
        assert!(result.starts_with("throw("), "unexpected lowering: {result}");
        assert!(result.contains("webservice= is not supported"), "unexpected message: {result}");
        assert!(!result.contains("__cfinvoke("), "still invokes a component: {result}");
    }

    #[test]
    fn test_cfhttp_forwards_every_attribute() {
        let result = tags_to_script(
            r#"<cfhttp url="http://x/y" name="q" file="a.txt" path="/tmp" throwOnError="true" redirect="false" port="8080" encodeURL="false" delimiter="|" firstRowAsHeaders="false" columns="a,b" result="r">"#,
        );
        for key in [
            "name", "file", "path", "throwonerror", "redirect", "port", "encodeurl",
            "delimiter", "firstrowasheaders", "columns",
        ] {
            assert!(
                result.contains(&format!("{}: ", key)),
                "cfhttp dropped `{}` at lowering: {}",
                key,
                result
            );
        }
        // `result` is the assignment target, not an option.
        assert!(result.starts_with("r = cfhttp("), "unexpected target: {result}");
    }

    /// `<cfloop query>`'s row window (`startrow`/`endrow`/`maxrows`) and
    /// `group=` reached the lowering and were dropped, so every row was
    /// iterated and a grouped loop had no control break — docs known-issues §27.
    #[test]
    fn test_cfloop_query_window_and_group_are_lowered() {
        let windowed = tags_to_script(
            r#"<cfloop query="q" startrow="2" endrow="4">X</cfloop>"#,
        );
        assert!(windowed.contains("__cfloopq_sr_"), "startrow dropped: {windowed}");
        assert!(windowed.contains("__cfloopq_er_"), "endrow dropped: {windowed}");
        assert!(
            !windowed.contains("for (var __cfloopq_i_0 = 1;"),
            "window ignored, loop still starts at row 1: {windowed}"
        );

        let grouped = tags_to_script(
            r#"<cfloop query="q" group="dept">A<cfloop>B</cfloop></cfloop>"#,
        );
        // Control break: compareNoCase is the default (Lucee-probed), and the
        // bare inner <cfloop> becomes the per-group detail iteration.
        assert!(grouped.contains("compareNoCase("), "group break missing: {grouped}");
        assert!(grouped.contains("__cfg_sub_"), "detail block missing: {grouped}");
    }

    /// `__cftransaction_start` takes (action, isolation, datasource) in FIXED
    /// positions. Emitting only the attributes present shifted `isolation` into
    /// the datasource slot, so `<cftransaction isolation="serializable">` tried
    /// to open a connection to a datasource named "serializable".
    #[test]
    fn test_cftransaction_emits_all_three_argument_positions() {
        let iso_only = tags_to_script(
            r#"<cftransaction isolation="serializable">X</cftransaction>"#,
        );
        assert!(
            iso_only.contains(r#"__cftransaction_start("begin", "serializable", "")"#),
            "isolation-only lowering has the wrong argument shape: {iso_only}"
        );
        let ds_only = tags_to_script(
            r#"<cftransaction datasource="ds1">X</cftransaction>"#,
        );
        assert!(
            ds_only.contains(r#"__cftransaction_start("begin", "", "ds1")"#),
            "datasource-only lowering has the wrong argument shape: {ds_only}"
        );
    }

    #[test]
    fn test_cfset() {
        let input = "<cfset x = 5>";
        assert!(has_cfml_tags(input));
        let result = tags_to_script(input);
        assert!(result.contains("x = 5"));
    }

    /// `<cfloop times=N>` must lower to a counted `for`, never to the
    /// `while (true)` fallback that used to hang the request.
    #[test]
    fn test_cfloop_times_is_a_counted_loop() {
        let result = tags_to_script(r#"<cfloop times="3">X</cfloop>"#);
        assert!(
            !result.contains("while (true)"),
            "times= fell through to the infinite-loop fallback: {result}"
        );
        assert!(result.contains("<= __cfloop_timesmax_"), "unexpected lowering: {result}");
    }

    /// Lucee 7.0.4: "Wrong Context, when you use attribute times, no other
    /// attributes are allowed". There is no `index` binding for this form.
    #[test]
    fn test_cfloop_times_rejects_other_attributes() {
        let err = tags_to_script_checked(r#"<cfloop times="3" index="i">X</cfloop>"#)
            .expect_err("times= with index= must be a compile error");
        assert!(err.contains("no other attributes are allowed"), "unexpected error: {err}");
    }

    /// An unrecognised loop form must fail loudly. Emitting `while (true)` for
    /// it turned every unimplemented attribute into an unbounded hang.
    #[test]
    fn test_cfloop_unknown_form_is_an_error_not_an_infinite_loop() {
        let err = tags_to_script_checked(r#"<cfloop wibble="3">X</cfloop>"#)
            .expect_err("an unrecognised <cfloop> form must be a compile error");
        assert!(err.contains("no recognised loop form"), "unexpected error: {err}");

        // Even on the tolerant path the emitted script must throw rather than
        // loop, and must stay brace-balanced for the `</cfloop>` that follows.
        let tolerant = tags_to_script(r#"<cfloop wibble="3">X</cfloop>"#);
        assert!(!tolerant.contains("while (true)"), "still hangs: {tolerant}");
        assert!(tolerant.contains("throw("), "does not fail loudly: {tolerant}");
    }

    #[test]
    fn test_cfloop_no_attributes_is_an_error() {
        let err = tags_to_script_checked("<cfloop>X</cfloop>")
            .expect_err("<cfloop> with no attributes must be a compile error");
        assert!(err.contains("requires a loop form"), "unexpected error: {err}");
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
        // cfloop chooses its direction at runtime from the sign of the step
        // (matching Lucee), so the step is hoisted into a temp and the loop
        // condition branches on it rather than emitting a fixed `i <= to`.
        assert!(result.contains("for (var i = 1; ("));
        assert!(result.contains("i >= 10 : i <= 10"));
        assert!(result.contains("i = i + __cfloop_step_"));
    }

    #[test]
    fn test_single_hash_expr_with_nested_interpolation() {
        // A whole-value `#expr#` whose expression contains a nested `#...#`
        // inside a string literal is still ONE expression and must keep its
        // native type — NOT be coerced through `"" & (...)` string concat.
        // Regression: <cfdbinfo attributeCollection="#getQueryAttrs(name='rs',
        // table='#qualifySchema(t)#',type='columns')#"> was arriving as a
        // STRING, so cfdbinfo could not find [name] (Masa admin boot).
        let raw = "#getQueryAttrs(name='rs',table='#qualifySchema(t)#',type='columns')#";
        let out = format_attr_value(raw, true);
        assert!(!out.starts_with("\"\" &"), "must not string-coerce: {out}");
        assert_eq!(
            out,
            "getQueryAttrs(name='rs',table='#qualifySchema(t)#',type='columns')"
        );

        // Plain single-expression case still works (no nested hash).
        assert_eq!(format_attr_value("#getQueryAttrs(x=1)#", true), "getQueryAttrs(x=1)");

        // Two separate interpolations are NOT a single expression — the
        // interpolation (concat) path must still apply.
        assert!(format_attr_value("#a#-#b#", true).contains(" & "));
        // Trailing literal after the expression is also interpolation.
        assert!(format_attr_value("#a#x", true).contains(" & "));
    }

    #[test]
    fn test_cfhttpparam_value_interpolation() {
        // A quoted attr with #expr# must interpolate (literal segment quoted,
        // expression in parens), NOT be stripped into a bare expression that
        // parses as `value - paramValue`.
        let input = r#"<cfhttp url="http://x"><cfhttpparam type="url" name="probe" value="value-#paramValue#"></cfhttp>"#;
        let result = tags_to_script(input);
        assert!(result.contains(r#""value-" & (paramValue)"#), "got: {result}");
        assert!(!result.contains("value: value-paramValue"), "got: {result}");
        // Plain literal (no hashes) stays a quoted literal string.
        let input2 = r#"<cfhttp url="http://x"><cfhttpparam type="url" name="probe" value="static-val"></cfhttp>"#;
        assert!(tags_to_script(input2).contains(r#"value: "static-val""#));
    }

    #[test]
    fn test_cfmailparam_value_interpolation() {
        let input = r#"<cfmail to="a@b.c" from="c@d.e" subject="s"><cfmailparam name="X-Tag" value="tok-#mailVar#"></cfmail>"#;
        let result = tags_to_script(input);
        assert!(result.contains(r#""tok-" & (mailVar)"#), "got: {result}");
        assert!(!result.contains("value: tok-mailVar"), "got: {result}");
    }

    #[test]
    fn test_cfquery_preserves_sql_line_comment_newlines() {
        // A `--` SQL comment is line-scoped; the lowered query string must keep
        // the newline so the comment does not swallow the following clauses.
        let input = "<cfquery name=\"q\" dbtype=\"query\">\nSELECT name\n-- comment\nFROM src\nWHERE id = #targetId#\n</cfquery>";
        let result = tags_to_script(input);
        assert!(
            result.contains("SELECT name\n-- comment\nFROM src\nWHERE id = "),
            "got: {result}"
        );
        assert!(
            !result.contains("SELECT name -- comment FROM src"),
            "got: {result}"
        );
    }

    #[test]
    fn test_unclosed_cfscript_is_a_compile_error() {
        // A <cfscript> with no </cfscript> before EOF is rejected by Lucee/ACF
        // at compile time. The strict entry point must surface that rather than
        // silently emitting the unterminated body as literal output.
        let input = "<cfscript>\nwriteOutput(\"hi\");\nx = 1 + 1;\n";
        let err = tags_to_script_checked(input)
            .expect_err("unclosed <cfscript> must be a compile error");
        assert!(err.contains("Unclosed <cfscript>"), "got: {err}");
        // The tolerant entry point still degrades (drops the open tag, body
        // passes through) — callers that want strictness use the checked form.
        let _ = tags_to_script(input);
    }

    #[test]
    fn test_closed_cfscript_is_ok() {
        let input = "<cfscript>\nx = 1 + 1;\n</cfscript>";
        let out = tags_to_script_checked(input).expect("closed <cfscript> is valid");
        assert!(out.contains("x = 1 + 1"), "got: {out}");
    }
}
