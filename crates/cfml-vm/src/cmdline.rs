//! Shell-style tokenization of a `<cfexecute arguments="...">` string.
//!
//! A faithful port of Lucee's `lucee.commons.cli.Command.toList` — quoting a
//! path is *the* standard CFML idiom for shelling out
//! (`arguments='-a "#filePath#"'`), so a naive whitespace split hands the child
//! process literal quote characters and splits any path containing a space.
//!
//! The rules, all of them Lucee's:
//!
//! * Both `'` and `"` quote, and a quoted span may contain whitespace.
//! * A quote character only opens a span if the **same** character occurs again
//!   later in the (trimmed) string. An unmatched quote is a literal character —
//!   that is what keeps `it's` working as one argument.
//! * The other quote character inside an open span is literal, so
//!   `"it's here"` is one argument.
//! * Quotes do not delimit arguments, they only suppress whitespace: `a"b c"d`
//!   is the single argument `ab cd`.
//! * Each argument is trimmed as it is flushed, and empty arguments are
//!   dropped — so `""` contributes nothing at all.
//! * There is no backslash escape. Lucee's source carries a commented-out
//!   implementation of one; keep it unimplemented so a Windows path such as
//!   `C:\dir\file` survives unchanged.

/// Split `arguments` the way Lucee does. See the module comment for the rules.
pub fn tokenize_arguments(arguments: &str) -> Vec<String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut list: Vec<String> = Vec::new();
    let mut buf = String::new();
    // The quote character currently holding a span open, or `None` outside one.
    let mut inside: Option<char> = None;

    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' | '"' => match inside {
                // Only open a span when the closing partner actually exists.
                None => {
                    if chars[i + 1..].contains(&c) {
                        inside = Some(c);
                    } else {
                        buf.push(c);
                    }
                }
                Some(q) if q == c => inside = None,
                // The other quote character: literal inside this span.
                Some(_) => buf.push(c),
            },
            ' ' | '\u{8}' | '\t' | '\n' | '\r' | '\u{c}' => {
                if inside.is_none() {
                    flush(&mut buf, &mut list);
                } else {
                    buf.push(c);
                }
            }
            _ => buf.push(c),
        }
    }
    flush(&mut buf, &mut list);

    list
}

/// Trim the pending argument into `list`, dropping it if nothing is left.
fn flush(buf: &mut String, list: &mut Vec<String>) {
    let tmp = buf.trim();
    if !tmp.is_empty() {
        list.push(tmp.to_string());
    }
    buf.clear();
}

#[cfg(test)]
mod tests {
    use super::tokenize_arguments as tk;

    #[test]
    fn unquoted_splits_on_whitespace() {
        assert_eq!(tk("-a /tmp/photo.jpg"), vec!["-a", "/tmp/photo.jpg"]);
        assert_eq!(tk("  -a \t -b \n -c  "), vec!["-a", "-b", "-c"]);
        assert_eq!(tk(""), Vec::<String>::new());
        assert_eq!(tk("   "), Vec::<String>::new());
    }

    #[test]
    fn quoted_span_is_one_argument_with_quotes_stripped() {
        assert_eq!(tk(r#"-a "/tmp/photo.jpg""#), vec!["-a", "/tmp/photo.jpg"]);
        assert_eq!(
            tk(r#"-f width "/tmp/dir with spaces/photo.jpg""#),
            vec!["-f", "width", "/tmp/dir with spaces/photo.jpg"]
        );
        assert_eq!(tk("'single quoted arg'"), vec!["single quoted arg"]);
    }

    #[test]
    fn internal_whitespace_inside_quotes_is_preserved_exactly() {
        // The discriminating case: a naive split would rejoin these as one space.
        assert_eq!(tk(r#""two  spaces""#), vec!["two  spaces"]);
        assert_eq!(tk("\"a\tb\""), vec!["a\tb"]);
    }

    #[test]
    fn an_unmatched_quote_is_a_literal_character() {
        assert_eq!(tk("it's fine"), vec!["it's", "fine"]);
        assert_eq!(tk(r#"say "hi"#), vec!["say", "\"hi"]);
    }

    #[test]
    fn the_other_quote_character_inside_a_span_is_literal() {
        assert_eq!(tk(r#""it's here""#), vec!["it's here"]);
        assert_eq!(tk(r#"'say "hi" now'"#), vec![r#"say "hi" now"#]);
    }

    #[test]
    fn quotes_suppress_whitespace_they_do_not_delimit() {
        assert_eq!(tk(r#"a"b c"d"#), vec!["ab cd"]);
        assert_eq!(tk(r#"--path="/a b/c""#), vec!["--path=/a b/c"]);
    }

    #[test]
    fn empty_and_whitespace_only_quoted_arguments_are_dropped() {
        assert_eq!(tk(r#"a "" b"#), vec!["a", "b"]);
        assert_eq!(tk(r#"a "   " b"#), vec!["a", "b"]);
    }

    #[test]
    fn backslashes_are_never_escapes() {
        assert_eq!(tk(r"C:\dir\file.txt"), vec![r"C:\dir\file.txt"]);
        assert_eq!(tk(r#""C:\Program Files\app.exe""#), vec![r"C:\Program Files\app.exe"]);
    }
}
