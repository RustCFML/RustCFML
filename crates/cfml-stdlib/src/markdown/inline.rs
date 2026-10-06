//! Strings in an **inline slot**: the argument of `.heading()`,
//! `.paragraph()`, a list item, a table cell. They are markdown, but only
//! inline markdown: `**bold**`, `` `code` ``, `[link](url)`, `~~gone~~`.
//! Syntax that would start a block at the beginning of a line is literal, so
//! `.heading( "1. Introduction" )` is a heading, not a numbered list, and
//! `.paragraph( "# hashtag" )` is a paragraph.
//!
//! Done by escaping line-start block markers and parsing the result as one
//! paragraph. Every character we escape is ASCII punctuation, and a
//! backslash before ASCII punctuation is always just that character, so the
//! escaping never changes what the text says.

use super::options::{md_err, MdOptions};
use super::parse::parse_unnumbered;
use super::tree::{normalise_inlines, Kind, Node};
use cfml_common::vm::CfmlError;

pub fn parse_inline(src: &str, opts: &MdOptions) -> Result<Vec<Node>, CfmlError> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        // `" "` between two runs is a separator, not nothing.
        return Ok(if src.contains([' ', '\t']) && !src.contains('\n') { vec![Node::text(" ")] } else { Vec::new() });
    }
    let mut prepared = String::with_capacity(trimmed.len() + 8);
    for (i, line) in trimmed.split('\n').enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            return Err(md_err(
                "A blank line can't go in a single block (a heading, paragraph, list item or table cell): \
                 it would start a second paragraph. Use .markdown() or a block content argument for several blocks."
                    .to_string(),
            ));
        }
        if i > 0 {
            prepared.push('\n');
        }
        prepared.push_str(&escape_line_start(line.trim_start()));
    }
    let mut o = opts.clone();
    o.front_matter = false;
    o.dedent = false;
    let doc = parse_unnumbered(&prepared, &o)?;
    let mut blocks = doc.children;
    if blocks.len() > 1 {
        return Err(md_err(format!(
            "[{}] is more than one block of markdown; use .markdown() for that.",
            excerpt(src)
        )));
    }
    let Some(only) = blocks.pop() else { return Ok(Vec::new()) };
    let mut out = match only.kind {
        Kind::Paragraph => only.children,
        // An HTML block at the start of an inline string is inline HTML that
        // happened to begin the line.
        Kind::Html { value } => vec![Node::new(Kind::Html { value })],
        _ => vec![Node::text(src.trim())],
    };
    // A paragraph trims its edges, but in a runs array the space in
    // `[ "Balance for ", { text = name } ]` is content.
    if src.starts_with([' ', '\t']) {
        out.insert(0, Node::text(" "));
    }
    if src.ends_with([' ', '\t']) {
        out.push(Node::text(" "));
    }
    normalise_inlines(&mut out);
    Ok(out)
}

fn excerpt(s: &str) -> String {
    let t: String = s.trim().chars().take(40).collect();
    if s.trim().chars().count() > 40 {
        format!("{}…", t)
    } else {
        t
    }
}

/// Neutralise whatever would make this line start a block other than a
/// paragraph continuation. `line` has no leading whitespace.
fn escape_line_start(line: &str) -> String {
    let bytes = line.as_bytes();
    let Some(&first) = bytes.first() else { return String::new() };
    let rest_after = |n: usize| -> &str { &line[n..] };
    let followed_by_space = |n: usize| bytes.get(n).map(|b| *b == b' ' || *b == b'\t').unwrap_or(true);
    let only = |c: u8| {
        let non_ws: Vec<u8> = bytes.iter().copied().filter(|b| *b != b' ' && *b != b'\t').collect();
        non_ws.len() >= 3 && non_ws.iter().all(|b| *b == c)
    };
    match first {
        // Never inline syntax at a line start: escaping is free.
        b'#' | b'>' | b'-' | b'+' | b'=' | b'|' | b':' => format!("\\{}", line),
        b'*' if followed_by_space(1) || only(b'*') => format!("\\{}", line),
        b'_' if only(b'_') => format!("\\{}", line),
        b'`' | b'~' => {
            let run = bytes.iter().take_while(|b| **b == first).count();
            let is_fence = run >= 3 && (first == b'~' || !rest_after(run).contains('`'));
            if is_fence {
                format!("\\{}", line)
            } else {
                line.to_string()
            }
        }
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            match bytes.get(digits) {
                Some(b'.') | Some(b')') if digits <= 9 && followed_by_space(digits + 1) => {
                    format!("{}\\{}", &line[..digits], &line[digits..])
                }
                _ => line.to_string(),
            }
        }
        _ => line.to_string(),
    }
}

/// `markdownEscape( text )`: make `text` read literally wherever it is put,
/// in an inline slot or a block slot.
pub fn escape_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let chars: Vec<char> = text.chars().collect();
    let mut at_line_start = true;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if at_line_start {
            if c == ' ' || c == '\t' {
                // Leading indentation never shows in rendered markdown, and
                // four or more spaces would make a code block: drop it.
                i += 1;
                continue;
            }
            // `1.` / `1)` list markers: escape the delimiter.
            if c.is_ascii_digit() {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && (chars[j] == '.' || chars[j] == ')') {
                    out.extend(&chars[i..j]);
                    out.push('\\');
                    out.push(chars[j]);
                    i = j + 1;
                    at_line_start = false;
                    continue;
                }
            }
            if matches!(c, '-' | '+' | '=' | ':') {
                out.push('\\');
                out.push(c);
                i += 1;
                at_line_start = false;
                continue;
            }
        }
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '&' | '#' | '!' | '@' | '$' => {
                out.push('\\');
                out.push(c);
            }
            // `http://` / `mailto:` autolinks need the colon.
            ':' => {
                out.push('\\');
                out.push(c);
            }
            // `www.` autolinks need the dot after `www`.
            '.' if i >= 3 && chars[i - 3..i].iter().collect::<String>().eq_ignore_ascii_case("www") => {
                out.push('\\');
                out.push(c);
            }
            '\n' => {
                out.push('\n');
                at_line_start = true;
                i += 1;
                continue;
            }
            _ => out.push(c),
        }
        at_line_start = false;
        i += 1;
    }
    out
}
