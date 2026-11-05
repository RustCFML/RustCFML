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
    /// Bodies moved VERBATIM from `call_function`: `args` stays an owned `Vec` exactly as
    /// it was in the original scope, so `?`, `return Ok(..)`, `&args` and `args.into_iter()`
    /// all still work and the diff is a pure move. The caller checks [`handles`] first, so
    /// `args` is only moved when this domain will actually consume it.
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
                        return self.cfdirectory_list_from_opts(opts, parent_locals);
                    } else if action == "copy" {
                        return self.cfdirectory_copy_from_opts(opts, parent_locals);
                    }
                }
            }

            if name_lower == "directorylist" {
                // directoryList(path [, recurse [, listInfo [, filter [, sort [, type [, filterDelimiters]]]]]])
                // If filter is a closure/UDF, handle it here
                if args.len() >= 4 {
                    if let Some(filter) = args.get(3) {
                        if matches!(filter, CfmlValue::Function(_)) {
                            return self.directory_list_with_closure(args, parent_locals);
                        }
                    }
                }
            }

            if name_lower == "directorycopy" {
                // directoryCopy(source, destination [, recurse [, filter [, createPath]]])
                // If filter is a closure/UDF, handle it here
                if args.len() >= 4 {
                    if let Some(filter) = args.get(3) {
                        if matches!(filter, CfmlValue::Function(_)) {
                            return self.directory_copy_with_closure(args, parent_locals);
                        }
                    }
                }
            }

        // Fell out of every branch — exactly what the original `if` chain did.
        // The caller turns this into fall-through; it is never seen by CFML.
        Err(intercepts_common::unhandled())
    }

    /// directoryList with closure/UDF filter
    fn directory_list_with_closure(
        &mut self,
        args: Vec<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let path = args.get(0).map(|v| v.as_string()).unwrap_or_default();
        let recurse = args.get(1).map(|v| v.is_true()).unwrap_or(false);
        let list_info = args.get(2).map(|v| v.as_string().to_lowercase()).unwrap_or_else(|| "path".to_string());
        let filter = match args.get(3) {
            Some(f) => f.clone(),
            None => return Err(CfmlError::runtime("directoryList: filter required".to_string())),
        };
        let sort = args.get(4).map(|v| v.as_string()).unwrap_or_default();
        let type_filter = args.get(5).map(|v| v.as_string().to_lowercase()).unwrap_or_else(|| "all".to_string());

        // Validate type
        let type_filter_trimmed = type_filter.trim();
        if !type_filter_trimmed.is_empty()
            && !matches!(type_filter_trimmed, "all" | "dir" | "directory" | "file")
        {
            return Err(CfmlError::runtime(format!(
                "Invalid type [{}], valid types are [all, directory, file]",
                type_filter
            )));
        }

        // Resolve and validate path
        let resolved_path = match self.resolve_directory_path_with_mappings(&path) {
            Some(p) => p,
            None => {
                return Err(CfmlError::runtime(format!("Directory [{}] doesn't exist", path)));
            }
        };

        // Collect entries
        let mut entries = Vec::new();
        let mut visited = std::collections::HashSet::new();
        if let Ok(canon) = self.vfs.canonicalize(&resolved_path) {
            visited.insert(canon);
        }
        self.sandbox_collect_entries(&resolved_path, recurse, &mut entries, &mut visited)?;

        // Apply type filter first, then closure filter
        entries.retain(|(_, _, is_dir)| {
            match type_filter_trimmed {
                "dir" | "directory" => *is_dir,
                "file" => !*is_dir,
                _ => true,
            }
        });

        // Filter with closure
        let mut filtered = Vec::new();
        for (name, full_path, is_dir) in entries {
            let entry_type = if is_dir { "directory" } else { "file" };
            let extension = if is_dir {
                CfmlValue::string("".to_string())
            } else {
                // Extract extension (no dot, preserve case)
                if let Some(dot_idx) = name.rfind('.') {
                    CfmlValue::string(name[(dot_idx + 1)..].to_string())
                } else {
                    CfmlValue::Null
                }
            };

            let cb_args = vec![
                CfmlValue::string(full_path.clone()),
                CfmlValue::string(entry_type.to_string()),
                extension,
            ];
            self.closure_parent_writeback = None;
            self.closure_parent_deletes = None;
            let keep = self.call_function(&filter, cb_args, parent_locals)?;
            
            // Coerce return value to boolean
            let should_include = keep.to_condition().map_err(|e| {
                CfmlError::runtime(format!("Can't cast {} to a boolean", e))
            })?;
            if should_include {
                filtered.push((name, full_path, is_dir));
            }
        }

        // Build result based on list_info
        let result = match list_info.as_str() {
            "name" => {
                let names: Vec<CfmlValue> = filtered
                    .iter()
                    .map(|(n, _, _)| CfmlValue::string(n.clone()))
                    .collect();
                CfmlValue::array(names)
            }
            "path" => {
                let paths: Vec<CfmlValue> = filtered
                    .iter()
                    .map(|(_, p, _)| CfmlValue::string(p.clone()))
                    .collect();
                CfmlValue::array(paths)
            }
            "query" => Self::build_directory_query(&filtered, &resolved_path),
            _ => {
                let paths: Vec<CfmlValue> = filtered
                    .iter()
                    .map(|(_, p, _)| CfmlValue::string(p.clone()))
                    .collect();
                CfmlValue::array(paths)
            }
        };

        // Apply sort for arrays (same as query sort - use a temp query)
        if matches!(result, CfmlValue::Array(_)) && !sort.is_empty() {
            if let CfmlValue::Array(arr) = &result {
                // Create a single-column query to sort
                let query = cfml_common::dynamic::CfmlQuery::new(vec!["name".to_string()]);
                for item in arr.iter() {
                    let mut row = ValueMap::default();
                    row.insert("name".to_string(), item.clone());
                    query.add_row(row);
                }
                cfml_common::dirlist::sort_listing_query(&query, &sort);
                // Extract back to array
                if let Some(col_arc) = query.column_values_ci("name") {
                    return Ok(CfmlValue::array((*col_arc).clone()));
                }
            }
        }
        
        // Apply sort for queries
        if let CfmlValue::Query(ref q) = result {
            cfml_common::dirlist::sort_listing_query(q, &sort);
        }

        Ok(result)
    }

    /// directoryCopy with closure/UDF filter
    fn directory_copy_with_closure(
        &mut self,
        args: Vec<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let src = args.get(0).map(|v| v.as_string()).unwrap_or_default();
        let dst = args.get(1).map(|v| v.as_string()).unwrap_or_default();
        let recurse = args.get(2).map(|v| v.is_true()).unwrap_or(false);
        let filter = match args.get(3) {
            Some(f) => f.clone(),
            None => return Err(CfmlError::runtime("directoryCopy: filter required".to_string())),
        };
        let create_path = args.get(4).map(|v| v.is_true()).unwrap_or(true);

        // Validate source
        let src_path = std::path::Path::new(&src);
        if !src_path.exists() {
            return Err(CfmlError::runtime(format!("Directory [{}] doesn't exist", src)));
        }
        if !src_path.is_dir() {
            return Err(CfmlError::runtime(format!("File [{}] exists, but isn't a directory", src)));
        }

        // Resolve destination
        let dst_path = if !dst.contains('/') && !dst.contains('\\') && !dst.contains(std::path::MAIN_SEPARATOR) {
            if let Some(parent) = src_path.parent() {
                parent.join(&dst)
            } else {
                std::path::PathBuf::from(&dst)
            }
        } else {
            std::path::PathBuf::from(&dst)
        };

        // Check createPath
        if !create_path {
            if let Some(parent) = dst_path.parent() {
                if !parent.exists() {
                    return Err(CfmlError::runtime(format!(
                        "parent directory for [{}] doesn't exist",
                        dst_path.display()
                    )));
                }
            }
        }

        // Create destination root
        std::fs::create_dir_all(&dst_path).map_err(|e| {
            CfmlError::runtime(format!("directoryCopy: failed to create destination: {}", e))
        })?;

        // Copy with filter
        let copy_result = self.copy_recursive_with_closure(
            src_path,
            &dst_path,
            src_path,
            &dst_path,
            recurse,
            &filter,
            parent_locals,
        );

        // Wrap errors as application errors (Lucee parity)
        let copy_result = copy_result.map_err(|e| {
            CfmlError::application(format!("directoryCopy filter error: {}", e))
        })?;

        // Remove empty directories if we filtered (Lucee parity)
        if recurse {
            self.remove_empty_folders_closure(&dst_path, &filter, parent_locals)?;
        }

        Ok(copy_result)
    }

    /// Recursive copy with closure filter
    fn copy_recursive_with_closure(
        &mut self,
        current_src: &std::path::Path,
        _current_dst: &std::path::Path,
        root_src: &std::path::Path,
        root_dst: &std::path::Path,
        recurse: bool,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let entries = std::fs::read_dir(current_src).map_err(|e| {
            CfmlError::runtime(format!("Failed to read directory: {}", e))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| CfmlError::runtime(format!("Directory entry error: {}", e)))?;
            let src_entry = entry.path();
            let name = src_entry
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            
            let is_dir = src_entry.is_dir();
            let rel_path = src_entry.strip_prefix(root_src).unwrap_or(&src_entry);
            let dst_entry = root_dst.join(rel_path);

            // Call filter
            let entry_type = if is_dir { "directory" } else { "file" };
            let extension = if is_dir {
                CfmlValue::string("".to_string())
            } else {
                if let Some(dot_idx) = name.rfind('.') {
                    CfmlValue::string(name[(dot_idx + 1)..].to_string())
                } else {
                    CfmlValue::Null
                }
            };

            let cb_args = vec![
                CfmlValue::string(dst_entry.to_string_lossy().to_string()),
                CfmlValue::string(entry_type.to_string()),
                extension,
            ];
            self.closure_parent_writeback = None;
            self.closure_parent_deletes = None;
            let keep = self.call_function(filter, cb_args, parent_locals)?;

            let should_copy = keep.to_condition().map_err(|e| {
                CfmlError::runtime(format!("Can't cast {} to a boolean", e))
            })?;

            if is_dir {
                // Always recurse into directories, even if rejected (Lucee parity)
                if recurse {
                    std::fs::create_dir_all(&dst_entry).ok();
                    self.copy_recursive_with_closure(
                        &src_entry,
                        &dst_entry,
                        root_src,
                        root_dst,
                        recurse,
                        filter,
                        parent_locals,
                    )?;
                }
            } else if should_copy {
                // Copy file
                if let Some(parent) = dst_entry.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::copy(&src_entry, &dst_entry).map_err(|e| {
                    CfmlError::runtime(format!("Failed to copy file: {}", e))
                })?;
            }
        }

        Ok(CfmlValue::Null)
    }

    /// Remove empty directories, checking each with closure filter
    fn remove_empty_folders_closure(
        &mut self,
        path: &std::path::Path,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let entries: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| CfmlError::runtime(format!("directoryCopy: {}", e)))?
            .collect::<Result<_, _>>()
            .map_err(|e| CfmlError::runtime(format!("directoryCopy: {}", e)))?;

        for entry in entries {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                // Recurse first (bottom-up)
                self.remove_empty_folders_closure(&entry_path, filter, parent_locals)?;
                
                // Check if directory is empty
                let is_empty = std::fs::read_dir(&entry_path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false);
                
                if is_empty {
                    // Check filter with closure
                    let cb_args = vec![
                        CfmlValue::string(entry_path.to_string_lossy().to_string()),
                        CfmlValue::string("directory"),
                        CfmlValue::string(""),
                    ];
                    self.closure_parent_writeback = None;
                    self.closure_parent_deletes = None;
                    let keep = self.call_function(filter, cb_args, parent_locals)?;
                    
                    let should_keep = keep.to_condition().unwrap_or(true);
                    if !should_keep {
                        let _ = std::fs::remove_dir(&entry_path);
                    }
                }
            }
        }

        Ok(CfmlValue::Null)
    }

    /// cfdirectory action=copy with closure filter support
    fn cfdirectory_copy_from_opts(
        &mut self,
        opts: &CfmlStruct,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let source = opts
            .get_ci("source")
            .or_else(|| opts.get_ci("directory"))
            .map(|v| v.as_string())
            .unwrap_or_default();
        let destination = opts
            .get_ci("destination")
            .or_else(|| opts.get_ci("newdirectory"))
            .map(|v| v.as_string())
            .unwrap_or_default();
        let recurse = opts
            .get_ci("recurse")
            .or_else(|| opts.get_ci("recursive"))
            .map(|v| v.is_true())
            .unwrap_or(false);
        let create_path = opts
            .get_ci("createpath")
            .map(|v| v.is_true())
            .unwrap_or(true);
        let filter = opts.get_ci("filter");

        // Build args for directory_copy_with_closure
        let mut args = vec![
            CfmlValue::string(source),
            CfmlValue::string(destination),
            CfmlValue::Bool(recurse),
        ];
        
        if let Some(f) = filter {
            args.push(f.clone());
        } else {
            args.push(CfmlValue::string(""));
        }
        
        args.push(CfmlValue::Bool(create_path));

        // If filter is a closure/UDF, use the closure path; otherwise fall through to stdlib
        if let Some(CfmlValue::Function(_)) = args.get(3) {
            self.directory_copy_with_closure(args, parent_locals)
        } else {
            // String filter or no filter: let the stdlib cfdirectory handle it
            Err(intercepts_common::unhandled())
        }
    }
}
