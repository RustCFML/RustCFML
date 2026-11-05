//! VM-intercepted query and directory builtins — cfdirectory, queryAppend, querySetRow.
//!
//! Extracted from `call_function`, which had grown to ~7,500 lines because the documented
//! way to add a VM-intercepted builtin was to append another `if name_lower == "…"` branch
//! and nothing ever moved one out (see `PERFORMANCE_ROADMAP.md` Part -1).
//!
//! Returns `Option<CfmlResult>`: `Some` when this domain handled the call, `None` to fall
//! through. `handles()` lives next to the implementations so the two cannot drift, and the
//! source-scanning guard in `tests/intercept_declaration_guard.rs` still sees these names,
//! so moving code here cannot silently drop an interception.

use super::*;

/// Names this module handles.
///
/// Name-only on purpose: several branches here fall through (e.g. `cfdirectory` returns
/// for `action = "list"` and defers every other action to the generic builtin path).
/// Reproducing those inner conditions here would duplicate them in a second place that
/// would drift; instead `dispatch_querydir` reports fall-through via
/// [`intercepts_common::unhandled`].
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "cfdirectory" | "__cfdirectory" | "directorylist" | "directorycopy"
    )
}

impl CfmlVirtualMachine {
    /// A closure or named UDF passed as a directory `filter` (Lucee: `instanceof UDF`,
    /// UDFFilter.java:78). RustCFML has two callable variants; match both.
    fn is_callable_filter(v: &CfmlValue) -> bool {
        matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_))
    }

    /// Validate a directory filter UDF/closure before any I/O, with Lucee's messages
    /// (UDFFilterSupport.java:30-48): return type boolean|any, at most 3 params, and a
    /// single param must be string|any.
    fn validate_directory_filter(filter: &CfmlValue) -> Result<(), CfmlError> {
        if let CfmlValue::Function(func) = filter {
            if let Some(rt) = func.return_type.as_deref() {
                let rt_lc = rt.trim().to_lowercase();
                if !rt_lc.is_empty() && rt_lc != "any" && rt_lc != "boolean" {
                    return Err(CfmlError::expression(format!(
                        "invalid return type [{}] for UDF Filter, valid return types are [boolean,any]",
                        rt
                    )));
                }
            }
            let param_count = func.params.len();
            if param_count > 3 {
                return Err(CfmlError::expression(format!(
                    "UDF filter has too many arguments [{}], should have at maximum 3 argument",
                    param_count
                )));
            }
            if param_count == 1 {
                let t = func.params[0]
                    .param_type
                    .as_deref()
                    .map(|s| s.trim().to_lowercase())
                    .unwrap_or_else(|| "any".to_string());
                if !t.is_empty() && t != "any" && t != "string" {
                    return Err(CfmlError::expression(format!(
                        "invalid type [{}] for first argument of UDF Filter, valid return types are [string,any]",
                        func.params[0].param_type.as_deref().unwrap_or("")
                    )));
                }
            }
        }
        Ok(())
    }

    /// Lucee `Caster.toBooleanValue` on the filter's return (UDFFilter.java:41,57;
    /// Caster.java:268-299): null/void goes through `toBooleanValue("")` and throws.
    fn directory_filter_result(keep: &CfmlValue) -> Result<bool, CfmlError> {
        if matches!(keep, CfmlValue::Null) {
            return Err(CfmlError::expression("Can't cast String [] to a boolean".to_string()));
        }
        keep.to_condition()
    }

    /// Call the filter for one entry with Lucee's Resource-form arguments
    /// (UDFFilter.java:50-57): absolute path, "file"|"directory", extension
    /// (`""` for a directory, null for a file without a dot).
    fn call_directory_filter(
        &mut self,
        filter: &CfmlValue,
        abs_path: &str,
        name: &str,
        is_dir: bool,
        parent_locals: &ValueMap,
    ) -> Result<bool, CfmlError> {
        let ext = if is_dir {
            CfmlValue::string(String::new())
        } else {
            match name.rfind('.') {
                Some(i) => CfmlValue::string(name[i + 1..].to_string()),
                None => CfmlValue::Null,
            }
        };
        let cb_args = vec![
            CfmlValue::string(abs_path.to_string()),
            CfmlValue::string(if is_dir { "directory" } else { "file" }.to_string()),
            ext,
        ];
        self.closure_parent_writeback = None;
        self.closure_parent_deletes = None;
        let keep = self.call_function(filter, cb_args, parent_locals)?;
        Self::directory_filter_result(&keep)
    }

    fn validate_directory_type(type_filter: &str) -> Result<&str, CfmlError> {
        let t = type_filter.trim();
        if t.is_empty() || matches!(t, "all" | "dir" | "directory" | "file") {
            Ok(t)
        } else {
            Err(CfmlError::application(format!(
                "Invalid type [{}], valid types are [all, directory, file]",
                t
            )))
        }
    }

    pub(crate) fn dispatch_querydir(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        if matches!(name_lower, "cfdirectory" | "__cfdirectory") {
            if let Some(CfmlValue::Struct(opts)) = args.first() {
                let action = opts
                    .get_ci("action")
                    .map(|v| v.as_string().to_lowercase())
                    .unwrap_or_else(|| "list".to_string());
                if action == "list" {
                    if opts.get_ci("filter").map(|f| Self::is_callable_filter(&f)).unwrap_or(false) {
                        return self.cfdirectory_list_with_closure(opts, parent_locals);
                    }
                    // String/no filter: the VM list path (Lucee wildcard, type,
                    // filterDelimiters, sort). Must NOT fall through to the stdlib
                    // `fn_cfdirectory` list, which has the old matcher and sort.
                    return self.cfdirectory_list_from_opts(opts, parent_locals);
                } else if action == "copy" {
                    // Sandbox: fall through so `sandbox_intercept` blocks the write.
                    if self.sandbox {
                        return Err(intercepts_common::unhandled());
                    }
                    return self.cfdirectory_copy_from_opts(opts, parent_locals);
                }
            }
        }

        if name_lower == "directorylist" {
            if args.get(3).map(Self::is_callable_filter).unwrap_or(false) {
                return self.directory_list_with_closure(args, parent_locals);
            }
        }

        if name_lower == "directorycopy" && !self.sandbox {
            if args.get(3).map(Self::is_callable_filter).unwrap_or(false) {
                return self.directory_copy_with_closure(args, parent_locals);
            }
        }

        Err(intercepts_common::unhandled())
    }

    /// Walk + type gate + closure filter. Returns `None` for a missing / non-directory
    /// path, which Lucee answers with an EMPTY result for the local filesystem
    /// (Directory.java:474-481), not an error.
    fn collect_with_closure(
        &mut self,
        path: &str,
        recurse: bool,
        type_filter: &str,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> Result<Option<(String, Vec<(String, String, bool)>)>, CfmlError> {
        // Validation happens when Lucee builds the filter, before any I/O.
        Self::validate_directory_filter(filter)?;
        let type_filter = Self::validate_directory_type(type_filter)?.to_string();
        let resolved = match self.resolve_directory_path_with_mappings(path) {
            Some(p) => p,
            None => return Ok(None),
        };
        let mut entries = Vec::new();
        let mut visited = std::collections::HashSet::new();
        if let Ok(canon) = self.vfs.canonicalize(&resolved) {
            visited.insert(canon);
        }
        self.sandbox_collect_entries(&resolved, recurse, &mut entries, &mut visited)?;
        let mut out = Vec::with_capacity(entries.len());
        for (name, full, is_dir) in entries {
            // AndResourceFilter{type, filter} short-circuits (Directory.java:435-439):
            // the UDF is never called for an entry of the wrong type.
            let type_ok = match type_filter.as_str() {
                "dir" | "directory" => is_dir,
                "file" => !is_dir,
                _ => true,
            };
            if !type_ok {
                continue;
            }
            if self.call_directory_filter(filter, &full, &name, is_dir, parent_locals)? {
                out.push((name, full, is_dir));
            }
        }
        Ok(Some((resolved, out)))
    }

    /// directoryList with a closure/UDF filter.
    fn directory_list_with_closure(&mut self, args: Vec<CfmlValue>, parent_locals: &ValueMap) -> CfmlResult {
        let path = args.get(0).map(|v| v.as_string()).unwrap_or_default();
        let recurse = args.get(1).map(|v| v.is_true()).unwrap_or(false);
        // DirectoryList.java:60-75: "name" | "query", anything else is "path".
        let list_info = args.get(2).map(|v| v.as_string().trim().to_lowercase()).unwrap_or_default();
        let filter = args.get(3).cloned().unwrap_or(CfmlValue::Null);
        let sort = args.get(4).map(|v| v.as_string()).unwrap_or_default();
        let type_filter = args.get(5).map(|v| v.as_string().to_lowercase()).unwrap_or_default();

        let (resolved, entries) =
            match self.collect_with_closure(&path, recurse, &type_filter, &filter, parent_locals)? {
                Some(x) => x,
                None => (path.clone(), Vec::new()),
            };

        if list_info == "query" {
            let listing = Self::build_directory_query(&entries, &resolved);
            if let CfmlValue::Query(ref q) = listing {
                cfml_common::dirlist::sort_listing_query(q, &sort);
            }
            return Ok(listing);
        }
        let want_name = list_info == "name";
        if sort.trim().is_empty() {
            return Ok(CfmlValue::array(
                entries
                    .into_iter()
                    .map(|(n, p, _)| CfmlValue::string(if want_name { n } else { p }))
                    .collect(),
            ));
        }
        // LDEV-3188 (Directory.java:451-464, 532-539): a sorted array is the full
        // query listing, sorted on its real columns, then projected to name/path.
        Ok(Self::sorted_listing_array(&entries, &resolved, &sort, want_name))
    }

    /// Sort `entries` as the full directory query (so `size`, `dateLastModified`,
    /// `type`, `directory` sort on real values) and project to names or paths.
    pub(crate) fn sorted_listing_array(
        entries: &[(String, String, bool)],
        listed_dir: &str,
        sort: &str,
        want_name: bool,
    ) -> CfmlValue {
        // Carry the original index through the sort in an extra column.
        let listing = Self::build_directory_query(entries, listed_dir);
        let CfmlValue::Query(q) = listing else { return CfmlValue::array(Vec::new()) };
        let idx: Vec<CfmlValue> = (0..entries.len()).map(|i| CfmlValue::Int(i as i64)).collect();
        q.with_write(|d| d.add_column("__idx".to_string(), idx));
        cfml_common::dirlist::sort_listing_query(&q, sort);
        let order: Vec<usize> = q
            .column_values_ci("__idx")
            .map(|c| c.iter().filter_map(|v| match v { CfmlValue::Int(i) => Some(*i as usize), _ => None }).collect())
            .unwrap_or_else(|| (0..entries.len()).collect());
        CfmlValue::array(
            order
                .into_iter()
                .filter_map(|i| entries.get(i))
                .map(|(n, p, _)| CfmlValue::string(if want_name { n.clone() } else { p.clone() }))
                .collect(),
        )
    }

    /// cfdirectory action=list with a closure/UDF filter: always a QUERY
    /// (tag listInfo: "name" → single-column relative-name query, else full query;
    /// Directory.java:283-285, 498-503).
    fn cfdirectory_list_with_closure(&mut self, opts: &CfmlStruct, parent_locals: &ValueMap) -> CfmlResult {
        let dir = opts.get_ci("directory").map(|v| v.as_string()).unwrap_or_default();
        let filter = opts.get_ci("filter").unwrap_or(CfmlValue::Null);
        let recurse = opts.get_ci("recurse").map(|v| v.is_true()).unwrap_or(false);
        let list_info = opts
            .get_ci("listinfo")
            .map(|v| v.as_string().trim().to_lowercase())
            .unwrap_or_else(|| "all".to_string());
        let sort = opts.get_ci("sort").map(|v| v.as_string()).unwrap_or_default();
        let type_filter = opts.get_ci("type").map(|v| v.as_string().to_lowercase()).unwrap_or_default();

        let (resolved, entries) =
            match self.collect_with_closure(&dir, recurse, &type_filter, &filter, parent_locals)? {
                Some(x) => x,
                None => (dir.clone(), Vec::new()),
            };
        let listing = if list_info == "name" {
            Self::build_directory_name_query(&entries, &resolved)
        } else {
            Self::build_directory_query(&entries, &resolved)
        };
        if let CfmlValue::Query(ref q) = listing {
            cfml_common::dirlist::sort_listing_query(q, &sort);
        }
        Ok(listing)
    }

    /// directoryCopy with a closure/UDF filter (Directory.actionCopy, Directory.java:904-958).
    fn directory_copy_with_closure(&mut self, args: Vec<CfmlValue>, parent_locals: &ValueMap) -> CfmlResult {
        // Invalidate file-exists/is-dir cache after the copy completes (guard dropped on exit).
        let _retire_negatives = RetireNegativesOnDrop;
        
        let src = args.get(0).map(|v| v.as_string()).unwrap_or_default();
        let dst = args.get(1).map(|v| v.as_string()).unwrap_or_default();
        let recurse = args.get(2).map(|v| v.is_true()).unwrap_or(false);
        let filter = args.get(3).cloned().unwrap_or(CfmlValue::Null);
        let create_path = args.get(4).map(|v| v.is_true()).unwrap_or(true);

        Self::validate_directory_filter(&filter)?;
        
        // Resolve source path through mappings (same resolution as directoryList with closure).
        let resolved_src = match self.resolve_directory_path_with_mappings(&src) {
            Some(p) => p,
            None => return Err(CfmlError::application(format!("Directory [{}] doesn't exist", src))),
        };
        let src_path = std::path::Path::new(&resolved_src);
        if !src_path.is_dir() {
            return Err(CfmlError::application(format!("File [{}] exists, but isn't a directory", src)));
        }
        if dst.is_empty() {
            return Err(CfmlError::application("Attribute [destination] is not defined".to_string()));
        }
        // Directory.java:960-966: no separator at all → sibling of the source.
        let dst_path = if !dst.contains('/') && !dst.contains('\\') {
            src_path.parent().map(|p| p.join(&dst)).unwrap_or_else(|| std::path::PathBuf::from(&dst))
        } else {
            std::path::PathBuf::from(&dst)
        };
        if !create_path {
            if let Some(parent) = dst_path.parent() {
                if !parent.exists() {
                    return Err(CfmlError::application(format!(
                        "parent directory for [{}] doesn't exist",
                        dst_path.display()
                    )));
                }
            }
        }

        // Everything inside Lucee's try{} becomes ApplicationException(t.getMessage())
        // (Directory.java:950-953): type `application`, message unchanged.
        let run = (|| -> Result<(), CfmlError> {
            std::fs::create_dir_all(&dst_path)
                .map_err(|e| CfmlError::application(e.to_string()))?;
            self.copy_recursive_with_closure(src_path, &dst_path, recurse, &filter, parent_locals)?;
            if recurse {
                // removeEmptyFolders(newdirectory, Not(filter)) — root included
                // (Directory.java:947; ResourceUtil.java:1648-1666).
                self.remove_empty_folders_closure(&dst_path, &filter, parent_locals)?;
            }
            Ok(())
        })();
        run.map_err(|e| CfmlError::application(e.message.clone()))?;
        Ok(CfmlValue::Null)
    }

    /// ResourceUtil.copyRecursive with the effective filter of Directory.java:924-938:
    /// non-recursive → And(filter, Not(Dir)); recursive → Or(filter, Dir). The UDF is
    /// evaluated FIRST in both, so it is called for directories too, and it receives
    /// the SOURCE path (`src.listResources(filter)`, ResourceUtil.java:1070).
    fn copy_recursive_with_closure(
        &mut self,
        src: &std::path::Path,
        dst: &std::path::Path,
        recurse: bool,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> Result<(), CfmlError> {
        let rd = std::fs::read_dir(src).map_err(|e| CfmlError::application(e.to_string()))?;
        for entry in rd {
            let entry = entry.map_err(|e| CfmlError::application(e.to_string()))?;
            let src_entry = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = src_entry.is_dir();
            let accepted = self.call_directory_filter(
                filter,
                &src_entry.to_string_lossy(),
                &name,
                is_dir,
                parent_locals,
            )?;
            let dst_entry = dst.join(&name);
            if is_dir {
                if recurse {
                    std::fs::create_dir_all(&dst_entry).map_err(|e| CfmlError::application(e.to_string()))?;
                    self.copy_recursive_with_closure(&src_entry, &dst_entry, recurse, filter, parent_locals)?;
                }
            } else if accepted && src_entry.is_file() {
                std::fs::copy(&src_entry, &dst_entry).map_err(|e| CfmlError::application(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// ResourceUtil.removeEmptyFolders(dir, Not(filter)), bottom-up, root included.
    /// Emptiness ignores `.DS_Store` (IgnoreSystemFiles), which `remove(true)` then
    /// deletes along with the directory. The filter sees DESTINATION paths here.
    fn remove_empty_folders_closure(
        &mut self,
        dir: &std::path::Path,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> Result<(), CfmlError> {
        if !dir.is_dir() {
            return Ok(());
        }
        let list = |d: &std::path::Path| -> Vec<std::path::PathBuf> {
            std::fs::read_dir(d)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.file_name() != ".DS_Store")
                        .map(|e| e.path())
                        .collect()
                })
                .unwrap_or_default()
        };
        for child in list(dir) {
            if child.is_dir() {
                self.remove_empty_folders_closure(&child, filter, parent_locals)?;
            }
        }
        if list(dir).is_empty() {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let keep = self.call_directory_filter(filter, &dir.to_string_lossy(), &name, true, parent_locals)?;
            if !keep {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        Ok(())
    }

    /// cfdirectory action=copy (same Directory.actionCopy as the BIF).
    fn cfdirectory_copy_from_opts(&mut self, opts: &CfmlStruct, parent_locals: &ValueMap) -> CfmlResult {
        let filter = opts.get_ci("filter");
        if !filter.as_ref().map(Self::is_callable_filter).unwrap_or(false) {
            // String / no filter: stdlib `fn_cfdirectory` "copy" → fn_directory_copy.
            return Err(intercepts_common::unhandled());
        }
        let source = opts.get_ci("directory").map(|v| v.as_string()).unwrap_or_default();
        let destination = opts
            .get_ci("destination")
            .or_else(|| opts.get_ci("newdirectory"))
            .map(|v| v.as_string())
            .unwrap_or_default();
        let recurse = opts.get_ci("recurse").map(|v| v.is_true()).unwrap_or(false);
        let create_path = opts.get_ci("createpath").map(|v| v.is_true()).unwrap_or(true);
        let args = vec![
            CfmlValue::string(source),
            CfmlValue::string(destination),
            CfmlValue::Bool(recurse),
            filter.unwrap_or(CfmlValue::Null),
            CfmlValue::Bool(create_path),
        ];
        self.directory_copy_with_closure(args, parent_locals)
    }
}
