//! Builtin-level security policy: `--sandbox` / `security.sandbox` and
//! `security.disallowedFunctions`.
//!
//! Both used to be enforced in exactly one place — the top (disallowed) and the
//! generic `_ =>` arm (sandbox) of `call_function_slow`. Every route that does
//! not pass through there walked past them:
//!
//! * `BytecodeOp::CallBuiltin`, which codegen emits for every builtin outside
//!   `VM_INTERCEPTED` (`ucase(x)`, `spreadsheetWriteToCsv(...)`);
//! * member calls (`"a".ucase()`), which resolve the BIF themselves;
//! * Java shims backed by a builtin (`call_bif`, `call_named_builtin`);
//! * every VM intercept that runs before the generic arm (`cfexecute`,
//!   `writeDump(output=path)`, `cfhttp file=`, the upload BIFs, …).
//!
//! So the checks now live here and are called from each of those entry points.
//! The name lists are explicit rather than derived from the cache-invalidation
//! predicates (`builtin_may_create_path` and friends), because those answer a
//! different question — "may this change what exists?" — and say yes to every
//! `image*` BIF, most of which only touch memory. The guard test at the bottom
//! of this file keeps the two in step: every name those predicates flag must be
//! classified here, as refused or as deliberately allowed.

use super::*;

/// Canonical spelling a policy entry and a call are compared in: lowercase,
/// with the `__` prefix the tag lowerings add stripped, and the script
/// statement form `execute` folded onto `cfexecute`. So the documented
/// `"disallowedFunctions": ["cfexecute"]` matches `<cfexecute>` (lowered to
/// `__cfexecute`), `cfexecute(...)` and `execute ...;` alike.
pub(crate) fn policy_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stripped = lower.trim_start_matches("__");
    match stripped {
        "execute" => "cfexecute".to_string(),
        other => other.to_string(),
    }
}

/// Builtins that write, modify or remove host paths whatever their arguments.
const SANDBOX_WRITE_BUILTINS: &[&str] = &[
    "filewrite",
    "fileappend",
    "filedelete",
    "filemove",
    "filecopy",
    "filewriteline",
    "filewritebinary",
    "filesetaccessmode",
    "filesetattribute",
    "filesetlastmodified",
    "directorycreate",
    "directorydelete",
    "directoryrename",
    "directorycopy",
    "setprofilestring",
    "gettempfile",
    "fileupload",
    "fileuploadall",
    "cffile_upload",
    "cfzip",
    "spreadsheetwrite",
    "spreadsheetwritetocsv",
    "imagewrite",
    "imagewritebase64",
    "cfftp",
];

/// Builtins that launch a host process.
const SANDBOX_EXEC_BUILTINS: &[&str] = &["cfexecute"];

/// Names the cache-invalidation predicates flag that the sandbox deliberately
/// lets through, each for a stated reason. See the guard test below.
#[cfg(test)]
const SANDBOX_ALLOWED_PATH_BUILTINS: &[&str] = &[
    // Reads: served from the virtual filesystem by `sandbox_intercept`.
    "fileread",
    "filereadbinary",
    "filereadline",
    "fileiseof",
    "fileexists",
    "directoryexists",
    "directorylist",
    "getfileinfo",
    "getprofilestring",
    "getprofilesections",
    "filegetmimetype",
    "filewassaved",
    "fileopen",
    "fileclose",
    "fileseek",
    "fileskipbytes",
    "fileinfo",
    // Inspected per call by `sandbox_refusal` (action / output attributes).
    "cffile",
    "file",
    "cfdirectory",
    "directory",
    "cfdump",
    "writedump",
    "cfhttp",
    "cfspreadsheet",
    "cfdocument",
    "cfpdf",
    "cfmail",
    "cfimage",
    "cfzipparam",
    // Engine-owned log directory, not a caller-chosen path.
    "cflog",
    "writelog",
    // Remote object storage, not the host filesystem.
    "storeaddacl",
    "storesetmetadata",
];

fn struct_attr(args: &[CfmlValue], key: &str) -> Option<String> {
    match args.first() {
        Some(CfmlValue::Struct(s)) => s.data_get(key).map(|v| v.as_string()),
        _ => None,
    }
}

fn has_any_attr(args: &[CfmlValue], keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| struct_attr(args, k).is_some_and(|v| !v.trim().is_empty()))
}

/// Why the sandbox refuses this builtin call, or `None` if it may run.
/// `name` must already be in [`policy_name`] form.
pub(crate) fn sandbox_refusal(name: &str, args: &[CfmlValue]) -> Option<CfmlError> {
    use cfml_common::host_policy::{exec_refused, write_refused};
    if SANDBOX_EXEC_BUILTINS.contains(&name) {
        return Some(exec_refused(&format!("{name}()")));
    }
    if SANDBOX_WRITE_BUILTINS.contains(&name) {
        return Some(write_refused(&format!("{name}()")));
    }
    // Tag-shaped builtins: the attribute struct decides.
    let action = || struct_attr(args, "action").unwrap_or_default().to_ascii_lowercase();
    let refused = match name {
        // Reads fall through to the VFS-backed `fileread`; everything else
        // (write, append, copy, move, rename, delete, upload) is a write.
        "cffile" | "file" => !matches!(action().as_str(), "read" | "readbinary" | "" | "info"),
        "cfdirectory" | "directory" => !matches!(action().as_str(), "list" | ""),
        "cfspreadsheet" => !matches!(action().as_str(), "read" | ""),
        "cfimage" => has_any_attr(args, &["destination"]),
        "cfdocument" | "cfpdf" => has_any_attr(args, &["filename", "destination", "directory"]),
        "cfmail" | "cfmailparam" => {
            struct_attr(args, "remove").is_some_and(|v| {
                v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            }) || has_any_attr(args, &["spoolDir"])
        }
        "cfdump" | "writedump" => struct_attr(args, "output").is_some_and(|o| {
            let o = o.trim();
            !o.is_empty()
                && !o.eq_ignore_ascii_case("browser")
                && !o.eq_ignore_ascii_case("console")
                && !o.eq_ignore_ascii_case("buffer")
        }),
        "cfhttp" => has_any_attr(args, &["file", "path"]),
        _ => false,
    };
    refused.then(|| write_refused(&format!("{name}()")))
}

impl CfmlVirtualMachine {
    /// Is any builtin-level policy in force? Two field loads; every entry
    /// point tests this before doing anything else so the common case (no
    /// sandbox, no disallowed list) pays nothing measurable.
    #[inline]
    pub(crate) fn builtin_policy_active(&self) -> bool {
        self.sandbox || !self.disallowed_functions.is_empty()
    }

    /// `security.disallowedFunctions` refusal for a callee name, if any.
    pub(crate) fn disallowed_refusal(&self, name: &str) -> Option<CfmlError> {
        if self.disallowed_functions.is_empty() {
            return None;
        }
        let canonical = policy_name(name);
        self.disallowed_functions.contains(&canonical).then(|| {
            CfmlError::new(
                format!("Function '{}' is disallowed by security policy", name),
                CfmlErrorType::Custom("security".to_string()),
            )
        })
    }

    /// The whole builtin gate: disallowed list, then sandbox. For the entry
    /// points that call a builtin directly (member calls, shims); the generic
    /// path in `call_function_slow` applies the same two checks inline.
    pub(crate) fn builtin_policy_refusal(
        &self,
        name: &str,
        args: &[CfmlValue],
    ) -> Option<CfmlError> {
        if let Some(e) = self.disallowed_refusal(name) {
            return Some(e);
        }
        if self.sandbox {
            return sandbox_refusal(&policy_name(name), args);
        }
        None
    }

    /// `Err` in sandbox mode. For VM handlers that write a host path only
    /// under some arguments (`writeDump(output=path)`, `cfhttp file=`), where
    /// the name-level gate cannot see the decision.
    pub(crate) fn host_write_guard(&self, what: &str) -> Result<(), CfmlError> {
        if self.sandbox {
            return Err(cfml_common::host_policy::write_refused(what));
        }
        cfml_common::host_policy::check_host_write(what)
    }

    /// `Err` in sandbox mode. For VM handlers that launch a host process.
    pub(crate) fn host_exec_guard(&self, what: &str) -> Result<(), CfmlError> {
        if self.sandbox {
            return Err(cfml_common::host_policy::exec_refused(what));
        }
        cfml_common::host_policy::check_host_exec(what)
    }

    /// Java shims that reach the host. In sandbox mode a class/method pair is
    /// refused unless it is pure path arithmetic or an existence probe (which
    /// is answered from the virtual filesystem instead).
    pub(crate) fn java_shim_sandbox_refusal(
        &self,
        java_class: &str,
        method: &str,
        object: &CfmlValue,
    ) -> Option<CfmlResult> {
        if !self.sandbox {
            return None;
        }
        let what = || format!("{java_class}.{method}()");
        let receiver_path = || match object {
            CfmlValue::Struct(s) => s
                .get("__path")
                .or_else(|| s.get("__spec"))
                .map(|v| v.as_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        match java_class {
            "java.io.file" => match method {
                // Path arithmetic only.
                "init" | "getname" | "getparent" | "getpath" | "getabsolutepath"
                | "tostring" | "getparentfile" | "getabsolutefile" | "topath" | "touri"
                | "isabsolute" | "equals" | "hashcode" | "compareto" | "getclass" => None,
                "exists" => Some(Ok(CfmlValue::Bool(self.vfs.exists(&receiver_path())))),
                "isfile" => Some(Ok(CfmlValue::Bool(self.vfs.is_file(&receiver_path())))),
                "isdirectory" => Some(Ok(CfmlValue::Bool(self.vfs.is_dir(&receiver_path())))),
                _ => Some(Err(cfml_common::host_policy::write_refused(&what()))),
            },
            "java.nio.file.files"
            | "java.io.fileoutputstream"
            | "java.io.fileinputstream"
            | "java.io.randomaccessfile" => {
                Some(Err(cfml_common::host_policy::write_refused(&what())))
            }
            c if io_csv_shim::is_writer_class(c) && c != io_csv_shim::CSV_WRITER_CLASS => {
                Some(Err(cfml_common::host_policy::write_refused(&what())))
            }
            "java.lang.processbuilder" | "java.lang.process" => {
                Some(Err(cfml_common::host_policy::exec_refused(&what())))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_name_folds_tag_and_statement_spellings() {
        assert_eq!(policy_name("__cfexecute"), "cfexecute");
        assert_eq!(policy_name("CFExecute"), "cfexecute");
        assert_eq!(policy_name("execute"), "cfexecute");
        assert_eq!(policy_name("uCase"), "ucase");
    }

    /// Every builtin the existence-cache predicates say may create or remove a
    /// path must be classified by the sandbox: refused, or deliberately
    /// allowed with a reason. A new file-touching builtin that lands in
    /// neither list fails here instead of silently writing in a sandbox.
    #[test]
    fn every_path_changing_builtin_is_classified() {
        let names = cfml_common::builtins_meta::BUILTIN_NAMES
            .iter()
            .chain(cfml_common::builtins_meta::VM_INTERCEPTED.iter());
        let mut unclassified = Vec::new();
        for raw in names {
            let name = policy_name(raw);
            let flagged = CfmlVirtualMachine::builtin_may_create_path(raw)
                || CfmlVirtualMachine::builtin_may_remove_path(raw)
                || CfmlVirtualMachine::creator_has_known_target(raw);
            if !flagged {
                continue;
            }
            // image* in-memory transforms are flagged by the broad predicate
            // but never touch a path; the ones that do are listed above.
            let in_memory_image = name.starts_with("image")
                && !SANDBOX_WRITE_BUILTINS.contains(&name.as_str());
            if SANDBOX_WRITE_BUILTINS.contains(&name.as_str())
                || SANDBOX_EXEC_BUILTINS.contains(&name.as_str())
                || SANDBOX_ALLOWED_PATH_BUILTINS.contains(&name.as_str())
                || in_memory_image
            {
                continue;
            }
            unclassified.push(name);
        }
        unclassified.sort();
        unclassified.dedup();
        assert!(
            unclassified.is_empty(),
            "classify these in host_policy.rs (refuse or allow with a reason): {unclassified:?}"
        );
    }

    #[test]
    fn tag_forms_refuse_by_attribute() {
        let attrs = |pairs: &[(&str, &str)]| {
            let mut m = ValueMap::default();
            for (k, v) in pairs {
                m.insert(k.to_string(), CfmlValue::string(v.to_string()));
            }
            vec![CfmlValue::strukt(m)]
        };
        assert!(sandbox_refusal("cffile", &attrs(&[("action", "write")])).is_some());
        assert!(sandbox_refusal("cffile", &attrs(&[("action", "read")])).is_none());
        assert!(sandbox_refusal("cfhttp", &attrs(&[("url", "x"), ("file", "/tmp/x")])).is_some());
        assert!(sandbox_refusal("cfhttp", &attrs(&[("url", "x")])).is_none());
        assert!(sandbox_refusal("cfdump", &attrs(&[("output", "/tmp/f")])).is_some());
        assert!(sandbox_refusal("cfdump", &attrs(&[("output", "browser")])).is_none());
        assert!(sandbox_refusal("cfexecute", &[]).is_some());
        assert!(sandbox_refusal("spreadsheetwritetocsv", &[]).is_some());
        assert!(sandbox_refusal("ucase", &[]).is_none());
    }
}
