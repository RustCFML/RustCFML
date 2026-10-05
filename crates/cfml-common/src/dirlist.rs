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
        // One sort key per (key column, row), built once. The comparator used
        // to call `as_string()` on BOTH cells per comparison: sorting the 6,164
        // rows of a recursive `directoryList(..., "DateLastModified")` over
        // Preside's assets tree ran ~78k comparisons = ~156k String
        // allocations, ~40 ms against Lucee's ~12 ms for the same listing
        // (Sticker's addAssets does this once per bundle on every reload).
        let key_cols: Vec<Vec<SortKey>> = keys
            .iter()
            .map(|&(ci, _)| d.data[ci].iter().map(SortKey::of).collect())
            .collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            for (k, &(_, asc)) in key_cols.iter().zip(keys.iter()) {
                let ord = k[a].cmp(&k[b]);
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

/// A listing cell reduced to something `Ord` once per row (see
/// [`sort_listing_query`]). Two integers compare numerically, everything else
/// as its string form — the same rule the per-comparison version applied.
enum SortKey {
    Int(i64),
    Str(String),
}

impl SortKey {
    fn of(v: &CfmlValue) -> Self {
        match v {
            CfmlValue::Int(x) => SortKey::Int(*x),
            other => SortKey::Str(other.as_string()),
        }
    }
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortKey::Int(x), SortKey::Int(y)) => x.cmp(y),
            (SortKey::Int(x), SortKey::Str(y)) => x.to_string().as_str().cmp(y.as_str()),
            (SortKey::Str(x), SortKey::Int(y)) => x.as_str().cmp(y.to_string().as_str()),
            (SortKey::Str(x), SortKey::Str(y)) => x.cmp(y),
        }
    }
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

/// `dateLastModified`: the entry's modification time as a date (Lucee gives a
/// date object, as `getFileInfo().lastModified` does). An empty string when
/// the filesystem cannot say.
pub fn date_last_modified(meta: Option<&std::fs::Metadata>) -> crate::dynamic::CfmlValue {
    match meta.and_then(|m| m.modified().ok()) {
        Some(t) => crate::dynamic::CfmlValue::DateTime(crate::datetime::from_system_time(t)),
        None => crate::dynamic::CfmlValue::string(String::new()),
    }
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
