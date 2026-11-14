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

/// The `type` argument of `directoryList()` / `<cfdirectory>`: which kinds of
/// entry a listing keeps. It never stops recursion into a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    All,
    Dir,
    File,
}

impl EntryType {
    /// Lucee's `Directory.toType`: `all`, `dir`, `directory` or `file`, trimmed
    /// and case-insensitive; empty means `all`.
    pub fn parse(raw: &str) -> Result<Self, crate::vm::CfmlError> {
        let t = raw.trim().to_lowercase();
        match t.as_str() {
            "" | "all" => Ok(EntryType::All),
            "dir" | "directory" => Ok(EntryType::Dir),
            "file" => Ok(EntryType::File),
            _ => Err(crate::vm::CfmlError::application(format!(
                "Invalid type [{t}], valid types are [all, directory, file]"
            ))),
        }
    }

    pub fn admits(self, is_dir: bool) -> bool {
        match self {
            EntryType::All => true,
            EntryType::Dir => is_dir,
            EntryType::File => !is_dir,
        }
    }
}

/// A `directoryCopy()` filter: called with an entry's path, name and whether it
/// is a directory; `Ok(false)` rejects it.
pub type CopyFilter<'a> =
    dyn FnMut(&std::path::Path, &str, bool) -> Result<bool, crate::vm::CfmlError> + 'a;

/// Lucee's `Directory.actionCopy`, shared by `directoryCopy()` and
/// `<cfdirectory action="copy">` with a wildcard or a closure filter.
///
/// - Without `recurse` only the top-level FILES are copied, filter or not.
/// - With `recurse` every directory is created and descended into whatever the
///   filter says; files are copied when the filter accepts them. When there is
///   a filter, each directory then left empty (`.DS_Store` aside) that the
///   filter rejects is removed, bottom-up, the destination root included. The
///   filter sees SOURCE paths while copying and DESTINATION paths while
///   clearing.
/// - The filter is consulted for directories too (Lucee evaluates it first in
///   both of its compound filters).
/// - A destination without any path separator is a sibling of the source.
/// - Every failure inside the copy, a filter's own exception included, is an
///   `application` error carrying the original message.
pub fn copy_directory(
    src: &std::path::Path,
    destination: &str,
    recurse: bool,
    create_path: bool,
    mut filter: Option<&mut CopyFilter<'_>>,
) -> Result<(), crate::vm::CfmlError> {
    use crate::vm::CfmlError;
    use std::path::{Path, PathBuf};

    fn io(e: std::io::Error) -> CfmlError {
        CfmlError::application(e.to_string())
    }

    fn copy(
        src: &Path,
        dst: &Path,
        recurse: bool,
        filter: &mut Option<&mut CopyFilter<'_>>,
    ) -> Result<(), CfmlError> {
        std::fs::create_dir_all(dst).map_err(io)?;
        for entry in std::fs::read_dir(src).map_err(io)? {
            let entry = entry.map_err(io)?;
            let from = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = from.is_dir();
            let accepted = match filter {
                Some(f) => f(&from, &name, is_dir)?,
                None => true,
            };
            let to = dst.join(&name);
            if is_dir {
                if recurse {
                    copy(&from, &to, recurse, filter)?;
                }
            } else if accepted && from.is_file() {
                std::fs::copy(&from, &to).map_err(io)?;
            }
        }
        Ok(())
    }

    /// `ResourceUtil.removeEmptyFolders(dir, Not(filter))`.
    fn clear_rejected_empty(dir: &Path, filter: &mut CopyFilter<'_>) -> Result<(), CfmlError> {
        let children = |d: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(d)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter(|e| e.file_name() != ".DS_Store")
                        .map(|e| e.path())
                        .collect()
                })
                .unwrap_or_default()
        };
        for child in children(dir) {
            if child.is_dir() {
                clear_rejected_empty(&child, filter)?;
            }
        }
        if children(dir).is_empty() {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !filter(dir, &name, true)? {
                std::fs::remove_dir_all(dir).map_err(io)?;
            }
        }
        Ok(())
    }

    if !src.exists() {
        return Err(CfmlError::application(format!("Directory [{}] doesn't exist", src.display())));
    }
    if !src.is_dir() {
        return Err(CfmlError::application(format!(
            "File [{}] exists, but isn't a directory",
            src.display()
        )));
    }
    if destination.is_empty() {
        return Err(CfmlError::application("Attribute [destination] is not defined".to_string()));
    }
    let dst = if destination.contains(['/', '\\']) {
        PathBuf::from(destination)
    } else {
        src.parent().map_or_else(|| PathBuf::from(destination), |p| p.join(destination))
    };
    if !create_path && dst.parent().is_some_and(|p| !p.exists()) {
        return Err(CfmlError::application(format!(
            "parent directory for [{}] doesn't exist",
            dst.display()
        )));
    }

    let mut run = || -> Result<(), CfmlError> {
        copy(src, &dst, recurse, &mut filter)?;
        if recurse {
            if let Some(f) = filter.as_mut() {
                clear_rejected_empty(&dst, *f)?;
            }
        }
        Ok(())
    };
    run().map_err(|e| CfmlError::application(e.message.clone()))
}
