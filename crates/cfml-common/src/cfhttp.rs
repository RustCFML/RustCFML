//! `<cfhttp>` response handling that needs neither the HTTP client nor the
//! stdlib: parsing a response body into a query (`name=`) and writing it to
//! disk (`file=`/`path=`).
//!
//! These live in `cfml-common` because the VM drives them from its `cfhttp`
//! intercept — the VM depends on `cfml-stdlib` only optionally (the `s3`
//! feature), so calling into the stdlib from there breaks the wasm builds,
//! which take `cfml-vm` with `default-features = false`.

use crate::dynamic::{CfmlQuery, CfmlValue, ValueMap};
use crate::vm::{CfmlError, CfmlErrorType, CfmlResult};

/// Split one line of delimited text, honouring a text qualifier. A qualifier
/// only opens a field at the field start; inside a qualified field a doubled
/// qualifier is a literal one. An empty `qualifier` disables qualification.
fn cfhttp_split_line(line: &str, delimiter: char, qualifier: Option<char>) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut at_field_start = true;
    while let Some(c) = chars.next() {
        if at_field_start && Some(c) == qualifier {
            // Qualified field: consume to the closing qualifier, doubling escapes.
            at_field_start = false;
            while let Some(qc) = chars.next() {
                if Some(qc) == qualifier {
                    if chars.peek().copied() == qualifier {
                        chars.next();
                        cur.push(qc);
                    } else {
                        break;
                    }
                } else {
                    cur.push(qc);
                }
            }
            continue;
        }
        at_field_start = false;
        if c == delimiter {
            fields.push(std::mem::take(&mut cur));
            at_field_start = true;
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

/// Read a single-character attribute (delimiter / textQualifier), falling back
/// to `default` when absent or empty. `textQualifier="none"` — and an explicitly
/// empty string — disable qualification, hence the `Option`.
fn cfhttp_char_attr(opts: &ValueMap, key: &str, default: Option<char>) -> Option<char> {
    match opts.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
        Some((_, v)) => {
            let s = v.as_string();
            if s.is_empty() || s.eq_ignore_ascii_case("none") {
                None
            } else {
                s.chars().next()
            }
        }
        None => default,
    }
}

/// Build the `<cfhttp name=>` query from a response body. Errors carry Lucee's
/// own wording so `catch( application e )` sees the same thing on both engines.
pub fn cfhttp_body_to_query(body: &str, opts: &ValueMap) -> CfmlResult {
    let delimiter = cfhttp_char_attr(opts, "delimiter", Some(',')).unwrap_or(',');
    let qualifier = cfhttp_char_attr(opts, "textqualifier", Some('"'));
    let first_row_as_headers = opts
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("firstrowasheaders"))
        .map(|(_, v)| match v {
            CfmlValue::Bool(b) => *b,
            CfmlValue::String(s) => !s.eq_ignore_ascii_case("false") && !s.eq_ignore_ascii_case("no"),
            CfmlValue::Int(i) => *i != 0,
            CfmlValue::Double(d) => *d != 0.0,
            _ => true,
        })
        .unwrap_or(true);
    let explicit_columns: Option<Vec<String>> = opts
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("columns"))
        .map(|(_, v)| v.as_string())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.split(',').map(|c| c.trim().to_string()).collect());

    let mut lines = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| cfhttp_split_line(l, delimiter, qualifier));

    let first = lines.next();
    let columns: Vec<String> = match (&explicit_columns, &first) {
        (Some(cols), _) => cols.clone(),
        (None, Some(f)) if first_row_as_headers => f.clone(),
        (None, Some(f)) => (1..=f.len()).map(|i| format!("COLUMN_{}", i)).collect(),
        (None, None) => Vec::new(),
    };

    let query = CfmlQuery::new(columns.clone());
    // The first physical line is data unless it was consumed as the header row.
    let leading: Vec<Vec<String>> = match first {
        Some(f) if !first_row_as_headers => vec![f],
        _ => Vec::new(),
    };
    for fields in leading.into_iter().chain(lines) {
        if fields.len() != columns.len() {
            return Err(CfmlError::new(
                format!(
                    "Invalid CSV line size, expected {} columns but found {} instead",
                    columns.len(),
                    fields.len()
                ),
                CfmlErrorType::Application,
            ));
        }
        query.add_row_positional(fields.into_iter().map(CfmlValue::string).collect());
    }
    Ok(CfmlValue::Query(query))
}

/// Write a cfhttp response body to `dir`/`file`, as `<cfhttp file= path=>` does.
/// `dir` must already be absolute (the VM resolves a relative `path=` against
/// the calling template first) and must exist — Lucee refuses to create it and
/// reports the missing parent as an IOException, which is reproduced here.
/// An existing file is overwritten.
pub fn cfhttp_write_body_to_file(
    dir: &str,
    file_name: &str,
    body: &CfmlValue,
) -> Result<(), CfmlError> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.is_dir() {
        // Report the tidied path Lucee reports — no `/./` segments, no trailing
        // separator — so the two engines' messages read the same.
        let shown = dir.replace("/./", "/");
        let shown = shown.trim_end_matches('/');
        return Err(CfmlError::io_exception(format!(
            "parent directory [{}] does not exist",
            if shown.is_empty() { dir } else { shown }
        )));
    }
    let target = dir_path.join(file_name);
    let bytes: Vec<u8> = match body {
        CfmlValue::Binary(b) => b.clone(),
        other => other.as_string().into_bytes(),
    };
    std::fs::write(&target, bytes).map_err(|e| {
        CfmlError::io_exception(format!("cannot write [{}]: {}", target.display(), e))
    })
}

/// The file name `<cfhttp path="…">` uses when no `file=` was given: the last
/// path segment of the request URL (Lucee behaviour), ignoring any query string.
pub fn cfhttp_file_name_from_url(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = no_query.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    if last.is_empty() || last.contains("://") {
        "index.htm".to_string()
    } else {
        last.to_string()
    }
}

