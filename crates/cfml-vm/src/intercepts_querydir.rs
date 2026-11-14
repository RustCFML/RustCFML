//! VM-intercepted directory builtins: `cfdirectory`, and `directoryList()` /
//! `directoryCopy()` when their filter is a closure or UDF.
//!
//! Extracted from `call_function`, which had grown to ~7,500 lines because the documented
//! way to add a VM-intercepted builtin was to append another `if name_lower == "…"` branch
//! and nothing ever moved one out (see `PERFORMANCE_ROADMAP.md` Part -1).
//!
//! `handles()` lives next to the implementations so the two cannot drift, and the
//! source-scanning guard in `tests/intercept_declaration_guard.rs` still sees these names,
//! so moving code here cannot silently drop an interception.
//!
//! Reached from `call_function_slow` AFTER its policy gate, so
//! `security.disallowedFunctions` and the sandbox's name-level refusals (which refuse
//! `directoryCopy` and every `cfdirectory` action but `list`) have already run. The
//! copy paths still take `host_write_guard` themselves.

use super::*;

/// Names this module handles.
///
/// Name-only on purpose: every branch here falls through for some arguments (a
/// string-filtered `directoryList`/`directoryCopy` stays with the stdlib builtin, as does
/// every `cfdirectory` action but `list` and `copy`). Reproducing those conditions here
/// would duplicate them in a second place that would drift; instead `dispatch_querydir`
/// reports fall-through via [`intercepts_common::unhandled`].
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "cfdirectory" | "__cfdirectory" | "directorylist" | "directorycopy")
}

/// A closure or UDF given as a directory `filter` (Lucee: `instanceof UDF`).
fn is_callable(v: &CfmlValue) -> bool {
    matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_))
}

/// One closure/UDF directory filter, called the way Lucee's `UDFFilter.accept(Resource)`
/// calls it: `(absolutePath, "file"|"directory", extension)`, where the extension is `""`
/// for a directory and null for a file without one. Writes the callback makes to the
/// caller's locals are carried between calls and published by [`Self::finish`], as the
/// higher-order array functions do.
struct DirectoryFilter<'a> {
    filter: &'a CfmlValue,
    parent_locals: &'a ValueMap,
    written: Option<ValueMap>,
}

impl<'a> DirectoryFilter<'a> {
    /// Lucee's `UDFFilterSupport` checks, made before any I/O: return type `boolean` or
    /// `any`, at most three parameters, and a lone parameter typed `string` or `any`.
    fn new(filter: &'a CfmlValue, parent_locals: &'a ValueMap) -> Result<Self, CfmlError> {
        if let CfmlValue::Function(func) = filter {
            fn declared(t: Option<&str>) -> Option<&str> {
                t.map(str::trim).filter(|t| !t.is_empty())
            }
            if let Some(rt) = declared(func.return_type.as_deref()) {
                if !rt.eq_ignore_ascii_case("any") && !rt.eq_ignore_ascii_case("boolean") {
                    return Err(CfmlError::expression(format!(
                        "invalid return type [{rt}] for UDF Filter, valid return types are [boolean,any]"
                    )));
                }
            }
            if func.params.len() > 3 {
                return Err(CfmlError::expression(format!(
                    "UDF filter has too many arguments [{}], should have at maximum 3 argument",
                    func.params.len()
                )));
            }
            if let [only] = func.params.as_slice() {
                if let Some(t) = declared(only.param_type.as_deref()) {
                    if !t.eq_ignore_ascii_case("any") && !t.eq_ignore_ascii_case("string") {
                        return Err(CfmlError::expression(format!(
                            "invalid type [{t}] for first argument of UDF Filter, valid return types are [string,any]"
                        )));
                    }
                }
            }
        }
        Ok(Self { filter, parent_locals, written: None })
    }
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_querydir(
        &mut self,
        name_lower: &str,
        mut args: Vec<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        // (if-chains, not a `match` on the name: the declaration guard harvests every
        // string literal inside such a match as an intercepted name.)
        if matches!(name_lower, "cfdirectory" | "__cfdirectory") {
            return match args.first() {
                Some(CfmlValue::Struct(opts)) => self.dispatch_cfdirectory(opts, parent_locals),
                _ => Err(intercepts_common::unhandled()),
            };
        }
        // A string filter stays with the stdlib builtin.
        if !(matches!(name_lower, "directorylist" | "directorycopy") && args.get(3).is_some_and(is_callable)) {
            return Err(intercepts_common::unhandled());
        }
        // The same path rebasing the string-filter builtin gets.
        self.resolve_file_bif_paths(name_lower, &mut args);
        let str_arg = |i: usize| args.get(i).map(|v| v.as_string()).unwrap_or_default();
        let bool_arg = |i: usize, default: bool| args.get(i).map_or(default, |v| v.is_true());
        if name_lower == "directorycopy" {
            // directoryCopy(source, destination, recurse, filter, createPath)
            return self.directory_copy(
                &str_arg(0),
                &str_arg(1),
                bool_arg(2, false),
                &args[3],
                "|",
                bool_arg(4, true),
                parent_locals,
            );
        }
        // directoryList(path, recurse, listInfo, filter, sort, type)
        let entry_type = cfml_common::dirlist::EntryType::parse(&str_arg(5))?;
        let mut filter = DirectoryFilter::new(&args[3], parent_locals)?;
        let path = str_arg(0);
        let entries = self.udf_filtered_entries(&path, bool_arg(1, false), entry_type, &mut filter);
        self.finish_directory_filter(filter);
        let list_info = if args.len() > 2 { str_arg(2).to_lowercase() } else { "path".to_string() };
        Ok(Self::directory_list_result(entries?, &path, &list_info, &str_arg(4)))
    }

    /// `<cfdirectory>`: `list` and `copy` are handled here, every other action by the
    /// stdlib builtin.
    fn dispatch_cfdirectory(&mut self, opts: &CfmlStruct, parent_locals: &ValueMap) -> CfmlResult {
        let action = opts
            .get_ci("action")
            .map(|v| v.as_string().to_lowercase())
            .unwrap_or_else(|| "list".to_string());
        match action.as_str() {
            "list" => match opts.get_ci("filter").filter(is_callable) {
                Some(filter) => self.cfdirectory_list_with_udf(opts, &filter, parent_locals),
                None => self.cfdirectory_list_from_opts(opts),
            },
            "copy" => self.cfdirectory_copy(opts, parent_locals),
            _ => Err(intercepts_common::unhandled()),
        }
    }

    /// Call a closure/UDF filter for one entry. Lucee casts the result with
    /// `Caster.toBooleanValue`, which refuses a missing return value.
    fn call_directory_filter(
        &mut self,
        f: &mut DirectoryFilter<'_>,
        path: &str,
        is_dir: bool,
    ) -> Result<bool, CfmlError> {
        let extension = if is_dir {
            CfmlValue::string(String::new())
        } else {
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            match name.rfind('.') {
                Some(i) => CfmlValue::string(name[i + 1..].to_string()),
                None => CfmlValue::Null,
            }
        };
        let args = vec![
            CfmlValue::string(path.to_string()),
            CfmlValue::string(if is_dir { "directory" } else { "file" }.to_string()),
            extension,
        ];
        self.closure_parent_writeback = None;
        self.closure_parent_deletes = None;
        let scope = f.written.as_ref().unwrap_or(f.parent_locals);
        let keep = self.call_function(f.filter, args, scope)?;
        if let Some(wb) = self.closure_parent_writeback.take() {
            let written = f.written.get_or_insert_with(|| f.parent_locals.clone());
            for (k, v) in &wb {
                written.insert(k.clone(), v.clone());
            }
            Self::write_back_to_captured_scope(f.filter, &wb);
            self.closure_parent_writeback = Some(wb);
        }
        match keep {
            CfmlValue::Null => Err(CfmlError::expression("Can't cast String [] to a boolean".to_string())),
            v => v.to_condition(),
        }
    }

    /// Publish the caller-local writes a filter made (see [`DirectoryFilter`]).
    fn finish_directory_filter(&mut self, f: DirectoryFilter<'_>) {
        if let Some(written) = f.written {
            self.set_ho_final_writeback(&written, f.parent_locals);
        }
    }

    /// The listing under `path` that a closure/UDF filter keeps. The `type` gate runs
    /// first, so the filter is never called for an entry of the wrong kind; a rejected
    /// directory is still recursed into.
    fn udf_filtered_entries(
        &mut self,
        path: &str,
        recurse: bool,
        entry_type: cfml_common::dirlist::EntryType,
        filter: &mut DirectoryFilter<'_>,
    ) -> Result<Vec<(String, String, bool)>, CfmlError> {
        let mut kept = Vec::new();
        for (name, full, is_dir) in self.collect_directory_entries(path, recurse)? {
            if entry_type.admits(is_dir) && self.call_directory_filter(filter, &full, is_dir)? {
                kept.push((name, full, is_dir));
            }
        }
        Ok(kept)
    }

    /// `<cfdirectory action="list">` with a closure/UDF filter.
    fn cfdirectory_list_with_udf(
        &mut self,
        opts: &CfmlStruct,
        filter: &CfmlValue,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        let attr = |key: &str| opts.get_ci(key).map(|v| v.as_string()).unwrap_or_default();
        let entry_type = cfml_common::dirlist::EntryType::parse(&attr("type"))?;
        let mut filter = DirectoryFilter::new(filter, parent_locals)?;
        let dir = attr("directory");
        let path = self.resolve_directory_path_with_mappings(&dir).unwrap_or(dir);
        let recurse = opts.get_ci("recurse").is_some_and(|v| v.is_true());
        let entries = self.udf_filtered_entries(&path, recurse, entry_type, &mut filter);
        self.finish_directory_filter(filter);
        Ok(Self::cfdirectory_list_result(entries?, &path, &attr("listinfo").to_lowercase(), &attr("sort")))
    }

    /// `<cfdirectory action="copy">`, with a wildcard or a closure/UDF filter.
    fn cfdirectory_copy(&mut self, opts: &CfmlStruct, parent_locals: &ValueMap) -> CfmlResult {
        let attr = |key: &str| opts.get_ci(key).map(|v| v.as_string()).unwrap_or_default();
        let dir = attr("directory");
        let source = self.resolve_directory_path_with_mappings(&dir).unwrap_or(dir);
        let destination = opts
            .get_ci("destination")
            .or_else(|| opts.get_ci("newdirectory"))
            .map(|v| v.as_string())
            .unwrap_or_default();
        let recurse = opts.get_ci("recurse").is_some_and(|v| v.is_true());
        let create_path = opts.get_ci("createpath").map_or(true, |v| v.is_true());
        let filter = opts.get_ci("filter").unwrap_or(CfmlValue::Null);
        let delimiters = attr("filterdelimiters");
        self.directory_copy(&source, &destination, recurse, &filter, &delimiters, create_path, parent_locals)
    }

    /// `directoryCopy()` / `<cfdirectory action="copy">`: see
    /// `cfml_common::dirlist::copy_directory`. `filter` is a closure/UDF, or a wildcard
    /// string (none when blank).
    #[allow(clippy::too_many_arguments)]
    fn directory_copy(
        &mut self,
        source: &str,
        destination: &str,
        recurse: bool,
        filter: &CfmlValue,
        delimiters: &str,
        create_path: bool,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        use cfml_common::dirlist::copy_directory;
        self.host_write_guard("directoryCopy()")?;
        let src = std::path::Path::new(source);
        if is_callable(filter) {
            let mut udf = DirectoryFilter::new(filter, parent_locals)?;
            let copied = copy_directory(
                src,
                destination,
                recurse,
                create_path,
                Some(&mut |path: &std::path::Path, _: &str, is_dir: bool| {
                    self.call_directory_filter(&mut udf, &path.to_string_lossy(), is_dir)
                }),
            );
            self.finish_directory_filter(udf);
            copied?;
        } else {
            let pattern = filter.as_string();
            if pattern.is_empty() {
                copy_directory(src, destination, recurse, create_path, None)?;
            } else {
                let wildcard = cfml_common::wildcard::WildcardPattern::new(&pattern, delimiters);
                let mut accept = |_: &std::path::Path, name: &str, _: bool| Ok(wildcard.matches(name));
                copy_directory(src, destination, recurse, create_path, Some(&mut accept))?;
            }
        }
        Ok(CfmlValue::Null)
    }
}
