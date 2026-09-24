//! Pieces of a directory listing that `directoryList()` (cfml-stdlib) and
//! `<cfdirectory action="list">` (cfml-vm) must agree on: the `sort` spec and
//! the `dateLastModified` / `mode` columns. Behaviour is Lucee 7.1's, probed.

use crate::dynamic::{CfmlQuery, CfmlValue};
use std::cmp::Ordering;

/// Sort a listing query in place by a `sort` spec: `"col [asc|desc][, col …]"`,
/// column and direction case-insensitive.
///
/// Lucee's rules, which are not the obvious ones:
/// - Text compares CASE-SENSITIVELY (`Alpha,B.txt,a.txt,beta`); `size` compares
///   as a number.
/// - A spec that names a column the query lacks, or a direction other than
///   `asc`/`desc`, sorts NOTHING — the listing keeps its filesystem order and
///   nothing is thrown.
/// - Equal keys keep their filesystem order (the sort is stable).
pub fn sort_listing_query(q: &CfmlQuery, spec: &str) {
    let Some(keys) = parse_sort_spec(q, spec) else {
        return;
    };
    let n = q.row_count();
    if n < 2 {
        return;
    }
    q.with_write(|d| {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            for &(ci, asc) in &keys {
                let ord = compare_cells(&d.data[ci][a], &d.data[ci][b]);
                let ord = if asc { ord } else { ord.reverse() };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });
        for col in d.data.iter_mut() {
            let sorted: Vec<CfmlValue> = order.iter().map(|&i| col[i].clone()).collect();
            *col = std::sync::Arc::new(sorted);
        }
    });
}

/// `(column index, ascending)` per key, or `None` when the spec is empty or
/// invalid (see [`sort_listing_query`]).
fn parse_sort_spec(q: &CfmlQuery, spec: &str) -> Option<Vec<(usize, bool)>> {
    let columns = q.columns();
    let mut keys = Vec::new();
    for part in spec.split(',') {
        let mut words = part.split_whitespace();
        let Some(col) = words.next() else {
            continue;
        };
        let asc = match words.next() {
            None => true,
            Some(d) if d.eq_ignore_ascii_case("asc") => true,
            Some(d) if d.eq_ignore_ascii_case("desc") => false,
            Some(_) => return None,
        };
        if words.next().is_some() {
            return None;
        }
        let ci = columns.iter().position(|c| c.eq_ignore_ascii_case(col))?;
        keys.push((ci, asc));
    }
    if keys.is_empty() {
        None
    } else {
        Some(keys)
    }
}

fn compare_cells(a: &CfmlValue, b: &CfmlValue) -> Ordering {
    match (a, b) {
        (CfmlValue::Int(x), CfmlValue::Int(y)) => x.cmp(y),
        _ => a.as_string().cmp(&b.as_string()),
    }
}

/// `dateLastModified`: the entry's modification time as a local-time CFML date
/// string (what `getFileInfo().lastModified` reports, and what `isDate()`
/// accepts). Empty when the filesystem cannot say.
pub fn date_last_modified(meta: Option<&std::fs::Metadata>) -> String {
    use chrono::TimeZone;
    meta.and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| chrono::Local.timestamp_opt(d.as_secs() as i64, 0).single())
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// `mode`: the Unix permission bits in octal (`644`, `755`), as Lucee reports
/// them. Empty where there are none (Windows) or no metadata.
pub fn mode(meta: Option<&std::fs::Metadata>) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(m) = meta {
            return format!("{:o}", m.permissions().mode() & 0o777);
        }
    }
    let _ = meta;
    String::new()
}
