//! `fileUpload()` / `fileUploadAll()` and `<cffile action="upload"|"uploadAll">`
//! (both the tag form, lowered to `__cffile_upload({...})`, and the script
//! form `cffile(action="upload", ...)`).
//!
//! These need the request's `form` scope (for the temp file each part was
//! streamed to) and the active Application.cfc (for
//! `this.blockedExtForFileUpload`), so they live in the VM.
//!
//! The `accept` attribute used to be read and discarded, and the client
//! filename was joined straight onto the destination, so a `.cfm` posted with
//! `Content-Type: image/png` against `accept="image/png"` was saved into the
//! destination and could then be requested. The checks here follow Lucee's
//! `FileTag.checkContentType`, in its order: extension allow/block lists first
//! (with Lucee's default block list when neither is given), then `accept`.
//!
//! `accept` / `strict` semantics, chosen on the safe side of Lucee:
//!
//! * `accept` is a comma list of MIME types (`image/png`, `image/*`, `*`) and/or
//!   extensions (`.png`, `*.png`). An extension entry is matched against the
//!   client filename's extension in BOTH strict modes (Lucee refuses extension
//!   entries outright under `strict=true`; accepting them is a superset).
//! * A MIME entry is matched against the client-declared `Content-Type`. With
//!   `strict=true` (the default, as on Lucee/ACF) it must ALSO match the type
//!   implied by the filename's extension — Lucee sniffs the bytes with Tika for
//!   this; we stand in for that with the extension table, which is what decides
//!   whether the server would later execute the file. A declared type the
//!   extension contradicts is therefore rejected, as is an unknown extension
//!   (its implied type is `application/octet-stream`).
//!
//! The saved path is additionally proven to sit directly inside the resolved
//! destination directory. Multipart parsing already reduces the client name to
//! a bare basename, but a `form.<field>` struct can also be built by CFML, so
//! the check is made again here.

use super::*;

/// Names this module handles. `cffile` falls through (via
/// [`intercepts_common::unhandled`]) for every action but upload/uploadAll.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "fileupload" | "fileuploadall" | "__cffile_upload" | "cffile"
    )
}

/// Lucee's `SystemUtil.DEFAULT_UPLOAD_EXT_BLOCKLIST`: what an upload may NOT
/// have as its extension when the tag/function names no allow or block list
/// and Application.cfc sets no `this.blockedExtForFileUpload`.
const DEFAULT_BLOCKED_EXTENSIONS: &str = "asp,aspx,cfc,cfm,cfml,do,htm,html,jsp,jspx,php";

/// Lucee's `FileTag.DETAIL`, appended to every extension rejection so the
/// message says where the policy can be changed.
const EXT_DETAIL: &str = "You can set a [allowedExtensions] and a [blockedExtensions] list as an \
    argument/attribute with the tag [cffile] and the functions [fileUpload] and [fileUploadAll]. \
    In addition you can configure this via the Application.cfc \
    [this.blockedExtForFileUpload] property to allow this type of file to be uploaded.";

/// The type a filename's extension implies. Mirrors `fileGetMimeType()`'s
/// table for the types browsers upload; anything else is `None` and is treated
/// as `application/octet-stream` by the strict check.
fn mime_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "text" | "log" => "text/plain",
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "cfm" | "cfml" | "cfc" => "text/x-coldfusion",
        _ => return None,
    })
}

/// Lowercase extension of a client filename, without the dot; empty when the
/// name has none. Lucee (`FileUtil.toExtensions`) lowercases the same way.
fn extension_of(client_file: &str) -> String {
    std::path::Path::new(client_file)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `type/subtype` with any parameters (`; charset=...`) dropped, lowercased.
fn bare_mime(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

/// Does `mime` (a bare `type/subtype`) satisfy `pattern`, where either half of
/// the pattern may be `*`?
fn mime_matches(mime: &str, pattern: &str) -> bool {
    let (pt, ps) = match pattern.split_once('/') {
        Some(p) => p,
        None => return false,
    };
    let (mt, ms) = match mime.split_once('/') {
        Some(m) => m,
        None => return false,
    };
    (pt == "*" || pt == mt) && (ps == "*" || ps == ms)
}

/// An extension list as the tag/function accepts it: a comma list or an array,
/// entries with or without a leading `.`/`*.`, case-insensitive. `None` when
/// the attribute was not given (or was empty), which is what selects the
/// Application.cfc / default block list.
fn extension_list(value: Option<&CfmlValue>) -> Option<Vec<String>> {
    let raw: Vec<String> = match value {
        None | Some(CfmlValue::Null) => return None,
        Some(CfmlValue::Array(arr)) => arr.iter().map(|v| v.as_string()).collect(),
        Some(v) => v.as_string().split(',').map(|s| s.to_string()).collect(),
    };
    let list: Vec<String> = raw
        .iter()
        .map(|e| {
            e.trim()
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .to_lowercase()
        })
        .filter(|e| !e.is_empty())
        .collect();
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

/// What one upload call permits. Built once per call, applied to every file
/// (`fileUploadAll` checks each part against the same policy, as Lucee does).
struct UploadPolicy {
    accept: String,
    strict: bool,
    allowed: Option<Vec<String>>,
    blocked: Option<Vec<String>>,
    /// `this.blockedExtForFileUpload` from the active Application.cfc, already
    /// normalised; consulted only when neither explicit list is given.
    app_blocked: Option<Vec<String>>,
}

impl UploadPolicy {
    /// Lucee's `checkContentType`: the extension lists, then `accept`. Returns
    /// the `application` exception Lucee throws, so `catch (application e)`
    /// around an upload behaves the same on both engines.
    fn check(&self, client_file: &str, declared_type: &str) -> Result<(), CfmlError> {
        let ext = extension_of(client_file);

        if !ext.is_empty() {
            let mut extension_accepted = false;
            if let Some(allowed) = &self.allowed {
                if !allowed.iter().any(|a| *a == ext) {
                    return Err(CfmlError::application(format!(
                        "Upload of files with extension [{}] is not permitted. The tag cffile/function \
                         fileUpload[All] only allows the following extensions in this context [{}]. {}",
                        ext,
                        allowed.join(","),
                        EXT_DETAIL
                    )));
                }
                extension_accepted = true;
            }
            if !extension_accepted {
                if let Some(blocked) = &self.blocked {
                    if blocked.iter().any(|b| *b == ext) {
                        return Err(CfmlError::application(format!(
                            "Upload of files with extension [{}] is not permitted. The tag cffile/function \
                             fileUpload[All] does not allow the following extensions in this context [{}]. {}",
                            ext,
                            blocked.join(","),
                            EXT_DETAIL
                        )));
                    }
                } else {
                    let default_list;
                    let blocked = match &self.app_blocked {
                        Some(l) => l,
                        None => {
                            default_list = extension_list(Some(&CfmlValue::string(
                                DEFAULT_BLOCKED_EXTENSIONS.to_string(),
                            )))
                            .unwrap_or_default();
                            &default_list
                        }
                    };
                    if blocked.iter().any(|b| *b == ext) {
                        return Err(CfmlError::application(format!(
                            "Upload of files with extension [{}] is not permitted. {}",
                            ext, EXT_DETAIL
                        )));
                    }
                }
            }
        }

        if self.accept.trim().is_empty() {
            return Ok(());
        }
        let declared = bare_mime(declared_type);
        // What the extension says the file is; the stand-in for Lucee's byte
        // sniff. Unknown extensions (and no extension) are opaque bytes.
        let implied = if ext.is_empty() {
            None
        } else {
            mime_for_extension(&ext)
        }
        .unwrap_or("application/octet-stream");

        for entry in self.accept.split(',') {
            let entry = entry.trim().to_lowercase();
            if entry.is_empty() {
                continue;
            }
            if entry == "*" || entry == "*/*" {
                return Ok(());
            }
            if entry.starts_with("*.") || entry.starts_with('.') {
                let want = entry.trim_start_matches("*.").trim_start_matches('.');
                if !ext.is_empty() && ext == want {
                    return Ok(());
                }
                continue;
            }
            if entry.contains('/')
                && mime_matches(&declared, &entry)
                && (!self.strict || mime_matches(implied, &entry))
            {
                return Ok(());
            }
        }
        Err(CfmlError::application(format!(
            "The MIME type or the Extension of the uploaded file [{}] (declared as [{}]) was not \
             accepted by the server. Only the following type(s) are allowed, [{}]. Verify that you \
             are uploading a file of the appropriate type.",
            client_file, declared, self.accept
        )))
    }
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_upload(&mut self, name_lower: &str, args: Vec<CfmlValue>) -> CfmlResult {
        // Attribute-struct forms: the tag lowering and the script-call form
        // both deliver one struct. `cffile` with any other action is not ours.
        let opts: Option<ValueMap> = match (name_lower, args.first()) {
            ("__cffile_upload", Some(CfmlValue::Struct(s))) => Some(s.snapshot()),
            ("cffile", Some(CfmlValue::Struct(s))) => {
                let action = s
                    .get_ci("action")
                    .map(|v| v.as_string().to_lowercase())
                    .unwrap_or_default();
                if action == "upload" || action == "uploadall" {
                    Some(s.snapshot())
                } else {
                    return Err(intercepts_common::unhandled());
                }
            }
            ("cffile", _) => return Err(intercepts_common::unhandled()),
            _ => None,
        };

        let positional_all = name_lower == "fileuploadall";
        let (all, destination, form_field, accept, name_conflict, strict, allowed, blocked) =
            match &opts {
                Some(o) => {
                    let get = |k: &str| o.get(k).cloned();
                    let s = |k: &str| get(k).map(|v| v.as_string()).unwrap_or_default();
                    let action = s("action").to_lowercase();
                    (
                        action == "uploadall",
                        s("destination"),
                        s("filefield"),
                        s("accept"),
                        s("nameconflict"),
                        get("strict").map(|v| v.is_true()).unwrap_or(true),
                        extension_list(get("allowedextensions").as_ref()),
                        extension_list(get("blockedextensions").as_ref()),
                    )
                }
                None => {
                    let a = |i: usize| args.get(i).map(|v| v.as_string()).unwrap_or_default();
                    if positional_all {
                        // fileUploadAll(destination, accept, nameConflict, strict,
                        //               allowedExtensions, blockedExtensions)
                        (
                            true,
                            a(0),
                            String::new(),
                            a(1),
                            a(2),
                            args.get(3).map(|v| v.is_true()).unwrap_or(true),
                            extension_list(args.get(4)),
                            extension_list(args.get(5)),
                        )
                    } else {
                        // fileUpload(destination, fileField, accept, nameConflict,
                        //            strict, allowedExtensions, blockedExtensions)
                        (
                            false,
                            a(0),
                            a(1),
                            a(2),
                            a(3),
                            args.get(4).map(|v| v.is_true()).unwrap_or(true),
                            extension_list(args.get(5)),
                            extension_list(args.get(6)),
                        )
                    }
                }
            };
        let name_conflict = if name_conflict.trim().is_empty() {
            "error".to_string()
        } else {
            name_conflict.trim().to_lowercase()
        };
        if !matches!(
            name_conflict.as_str(),
            "error" | "skip" | "overwrite" | "makeunique" | "forceunique"
        ) {
            return Err(CfmlError::application(format!(
                "Invalid value [{}] for attribute nameConflict; valid values are \
                 error, skip, overwrite, makeunique and forceunique",
                name_conflict
            )));
        }
        if destination.trim().is_empty() {
            return Err(CfmlError::application(
                "The destination for the upload is required".to_string(),
            ));
        }

        // Application.cfc `this.blockedExtForFileUpload` (Lucee), read from the
        // loaded template's settings.
        let app_blocked = match &self.app_cfc_template {
            Some(CfmlValue::Struct(s)) => extension_list(s.get_ci("blockedextforfileupload").as_ref()),
            _ => None,
        };
        let policy = UploadPolicy {
            accept,
            strict,
            allowed,
            blocked,
            app_blocked,
        };

        let form = match self.globals.get("form").cloned() {
            Some(CfmlValue::Struct(f)) => f,
            _ => match CfmlValue::strukt(ValueMap::default()) {
                CfmlValue::Struct(f) => f,
                _ => unreachable!("strukt() builds a Struct"),
            },
        };

        if all {
            let mut results = Vec::new();
            // Prefer the per-request upload registry so every part under a
            // repeated field name is returned (GH #547). Fall back to scanning
            // form for CFML-constructed structs when the registry is empty.
            if !self.upload_registry.is_empty() {
                for parts in self.upload_registry.values() {
                    for file_info in parts {
                        let temp_path = file_info
                            .get("tempFilePath")
                            .or_else(|| file_info.get("tempfilepath"))
                            .map(|v| v.as_string())
                            .unwrap_or_default();
                        if temp_path.is_empty() {
                            continue;
                        }
                        let saved = Self::save_uploaded_file(
                            file_info,
                            &destination,
                            &name_conflict,
                            &policy,
                        )?;
                        results.push(CfmlValue::strukt(saved));
                    }
                }
            } else {
                for (_, val) in form.iter() {
                    if let CfmlValue::Struct(file_info) = val {
                        let temp_path = file_info
                            .data_get("tempfilepath")
                            .map(|v| v.as_string())
                            .unwrap_or_default();
                        if temp_path.is_empty() {
                            continue;
                        }
                        let saved = Self::save_uploaded_file(
                            &file_info.snapshot(),
                            &destination,
                            &name_conflict,
                            &policy,
                        )?;
                        results.push(CfmlValue::strukt(saved));
                    }
                }
            }
            return Ok(CfmlValue::array(results));
        }

        // `fileField="form.myFile"` is accepted as well as the bare field name.
        let field_lower = form_field
            .trim()
            .to_lowercase()
            .trim_start_matches("form.")
            .to_string();

        // Registry first: when several parts share the field name, take the
        // first (Lucee parity; GH #547). form.<field> is only the temp path.
        if let Some(parts) = self.upload_registry.get(&field_lower) {
            if let Some(file_info) = parts.first() {
                let has_temp = file_info
                    .get("tempFilePath")
                    .or_else(|| file_info.get("tempfilepath"))
                    .map(|v| !v.as_string().is_empty())
                    .unwrap_or(false);
                if has_temp {
                    let saved = Self::save_uploaded_file(
                        file_info,
                        &destination,
                        &name_conflict,
                        &policy,
                    )?;
                    return Ok(CfmlValue::strukt(saved));
                }
            }
        }

        // Backward compat: CFML code that manually builds a struct in
        // form.<field> (see tests/tags/test_cffile_upload_accept.cfm).
        if let Some(CfmlValue::Struct(file_info)) = form.data_get(&field_lower) {
            let has_temp = file_info
                .data_get("tempfilepath")
                .map(|v| !v.as_string().is_empty())
                .unwrap_or(false);
            if has_temp {
                let saved = Self::save_uploaded_file(
                    &file_info.snapshot(),
                    &destination,
                    &name_conflict,
                    &policy,
                )?;
                return Ok(CfmlValue::strukt(saved));
            }
        }
        Err(CfmlError::application(format!(
            "fileUpload: form field [{}] was not found or did not carry an uploaded file",
            form_field
        )))
    }

    /// Check one uploaded part against `policy` and move it into place.
    ///
    /// `destination` is a directory (existing, or created) unless it does not
    /// exist and names a file with an extension, in which case the upload is
    /// saved under that name (`fileWasRenamed`), as Lucee does. The client
    /// filename must be a bare basename — a path separator, `.` or `..` is
    /// refused before anything is joined — and the resulting path is then
    /// checked to sit directly inside the resolved destination directory.
    fn save_uploaded_file(
        file_info: &ValueMap,
        destination: &str,
        name_conflict: &str,
        policy: &UploadPolicy,
    ) -> Result<ValueMap, CfmlError> {
        let get = |k: &str| file_info.get(k).map(|v| v.as_string()).unwrap_or_default();
        let temp_path = get("tempfilepath");
        let client_file = get("clientfile");
        let declared_type = get("contenttype");

        if client_file != crate::web::sanitize_upload_filename(&client_file)
            || client_file.contains('/')
            || client_file.contains('\\')
        {
            return Err(CfmlError::application(format!(
                "Upload of file [{}] was refused: the client filename must be a bare file name",
                client_file
            )));
        }

        policy.check(&client_file, &declared_type)?;

        let dest = std::path::Path::new(destination);
        let trailing_sep = destination.ends_with('/') || destination.ends_with('\\');
        let (dest_dir, mut target_name, mut renamed) = if dest.is_dir() || trailing_sep {
            (dest.to_path_buf(), client_file.clone(), false)
        } else if !dest.exists()
            && dest.extension().is_some()
            && !dest
                .file_name()
                .map(|n| n.to_string_lossy().eq_ignore_ascii_case(&client_file))
                .unwrap_or(false)
        {
            let parent = dest.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let name = dest
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (parent, name, true)
        } else {
            (dest.to_path_buf(), client_file.clone(), false)
        };
        if dest_dir.as_os_str().is_empty() {
            return Err(CfmlError::application(format!(
                "The destination [{}] for the upload is not valid",
                destination
            )));
        }
        std::fs::create_dir_all(&dest_dir).map_err(|e| {
            CfmlError::application(format!(
                "Can't create destination directory [{}] for the upload: {}",
                dest_dir.display(),
                e
            ))
        })?;
        let dest_canonical = dest_dir.canonicalize().map_err(|e| {
            CfmlError::application(format!(
                "Can't resolve destination directory [{}] for the upload: {}",
                dest_dir.display(),
                e
            ))
        })?;

        let attempted = dest_dir.join(&target_name);
        let existed = attempted.exists();
        let old_size = if existed {
            std::fs::metadata(&attempted).map(|m| m.len() as i64).unwrap_or(0)
        } else {
            0
        };
        let mut overwritten = false;
        let mut final_path = attempted.clone();
        if existed {
            match name_conflict {
                "error" => {
                    return Err(CfmlError::application(format!(
                        "Destination file [{}] already exists",
                        attempted.display()
                    )));
                }
                "skip" => {
                    let mut result = Self::upload_result(
                        file_info,
                        &client_file,
                        &dest_dir,
                        &target_name,
                        &declared_type,
                    );
                    result.insert("fileExisted".to_string(), CfmlValue::Bool(true));
                    result.insert("fileWasSaved".to_string(), CfmlValue::Bool(false));
                    result.insert("oldFileSize".to_string(), CfmlValue::Int(old_size));
                    result.insert("attemptedServerFile".to_string(), CfmlValue::string(target_name));
                    return Ok(result);
                }
                "overwrite" => {
                    overwritten = true;
                }
                _ => {
                    // makeunique / forceunique: a fresh name before the
                    // extension until one is free.
                    let stem = attempted
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let ext = attempted
                        .extension()
                        .map(|s| format!(".{}", s.to_string_lossy()))
                        .unwrap_or_default();
                    let sep = if name_conflict == "forceunique" { "_" } else { "" };
                    let mut n: u64 = cfml_common::clock::now_unix_millis() as u64;
                    loop {
                        let candidate = dest_dir.join(format!("{}{}{:x}{}", stem, sep, n, ext));
                        if !candidate.exists() {
                            final_path = candidate;
                            break;
                        }
                        n = n.wrapping_add(1);
                    }
                    target_name = final_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    renamed = true;
                }
            }
        }

        // Defence in depth: whatever the name was, the file lands directly in
        // the destination directory, not beside or above it.
        let parent_canonical = final_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .unwrap_or_default();
        if parent_canonical != dest_canonical
            || final_path.file_name().map(|n| n.to_string_lossy() != target_name).unwrap_or(true)
        {
            return Err(CfmlError::application(format!(
                "Upload of file [{}] was refused: the resolved path [{}] is outside the \
                 destination directory [{}]",
                client_file,
                final_path.display(),
                dest_dir.display()
            )));
        }

        match std::fs::copy(&temp_path, &final_path) {
            Ok(_) => {
                let _ = std::fs::remove_file(&temp_path);
            }
            Err(e) => {
                return Err(CfmlError::application(format!(
                    "Can't save uploaded file [{}] to [{}]: {}",
                    client_file,
                    final_path.display(),
                    e
                )));
            }
        }
        let size = std::fs::metadata(&final_path).map(|m| m.len() as i64).unwrap_or(0);

        let mut result =
            Self::upload_result(file_info, &client_file, &dest_dir, &target_name, &declared_type);
        result.insert("fileExisted".to_string(), CfmlValue::Bool(existed));
        result.insert("fileWasOverwritten".to_string(), CfmlValue::Bool(overwritten));
        result.insert("fileWasRenamed".to_string(), CfmlValue::Bool(renamed));
        result.insert("fileWasSaved".to_string(), CfmlValue::Bool(true));
        result.insert("fileSize".to_string(), CfmlValue::Int(size));
        result.insert("oldFileSize".to_string(), CfmlValue::Int(old_size));
        result.insert(
            "attemptedServerFile".to_string(),
            CfmlValue::string(
                attempted
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
        );
        Ok(result)
    }

    /// The `cffile` result struct for one upload, keyed as Lucee keys it. The
    /// caller sets the outcome flags (`fileWasSaved`, `fileExisted`, ...).
    fn upload_result(
        file_info: &ValueMap,
        client_file: &str,
        dest_dir: &std::path::Path,
        server_file: &str,
        declared_type: &str,
    ) -> ValueMap {
        let split = |name: &str| -> (String, String) {
            let p = std::path::Path::new(name);
            (
                p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                p.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            )
        };
        let (client_stem, client_ext) = split(client_file);
        let (server_stem, server_ext) = split(server_file);
        let mime = bare_mime(declared_type);
        let (ctype, csub) = mime.split_once('/').unwrap_or((mime.as_str(), ""));

        let mut result = file_info.clone();
        result.insert("clientDirectory".to_string(), CfmlValue::string(String::new()));
        result.insert("clientFile".to_string(), CfmlValue::string(client_file.to_string()));
        result.insert("clientFileName".to_string(), CfmlValue::string(client_stem));
        result.insert("clientFileExt".to_string(), CfmlValue::string(client_ext));
        result.insert(
            "serverDirectory".to_string(),
            CfmlValue::string(dest_dir.to_string_lossy().to_string()),
        );
        result.insert("serverFile".to_string(), CfmlValue::string(server_file.to_string()));
        result.insert("serverFileName".to_string(), CfmlValue::string(server_stem));
        result.insert("serverFileExt".to_string(), CfmlValue::string(server_ext));
        result.insert("contentType".to_string(), CfmlValue::string(ctype.to_string()));
        result.insert("contentSubType".to_string(), CfmlValue::string(csub.to_string()));
        result.insert("fileWasAppended".to_string(), CfmlValue::Bool(false));
        result.insert("fileWasOverwritten".to_string(), CfmlValue::Bool(false));
        result.insert("fileWasRenamed".to_string(), CfmlValue::Bool(false));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(accept: &str, strict: bool) -> UploadPolicy {
        UploadPolicy {
            accept: accept.to_string(),
            strict,
            allowed: None,
            blocked: None,
            app_blocked: None,
        }
    }

    #[test]
    fn accept_by_mime_requires_extension_agreement_when_strict() {
        let p = policy("image/png", true);
        assert!(p.check("a.png", "image/png").is_ok());
        // The proven bypass: a .cfm declared as image/png.
        assert!(p.check("a.cfm", "image/png").is_err());
        // Declared type the extension contradicts.
        assert!(p.check("a.txt", "image/png").is_err());
        // Unknown extension cannot be proven to be a PNG.
        assert!(p.check("a.xyz", "image/png").is_err());
        // Non-strict trusts the declaration (a .cfm is still caught by the
        // default block list, which runs first).
        assert!(policy("image/png", false).check("a.txt", "image/png").is_ok());
        assert!(policy("image/png", false).check("a.cfm", "image/png").is_err());
    }

    #[test]
    fn accept_wildcards_and_parameters() {
        let p = policy("image/*", true);
        assert!(p.check("a.jpg", "image/jpeg; charset=binary").is_ok());
        assert!(p.check("a.pdf", "image/jpeg").is_err());
        assert!(policy("*", true).check("a.bin", "application/octet-stream").is_ok());
        assert!(policy("*/*", true).check("a.bin", "").is_ok());
    }

    #[test]
    fn accept_by_extension_in_both_strict_modes() {
        for strict in [true, false] {
            let p = policy(".jpg, *.PNG", strict);
            assert!(p.check("a.JPG", "application/octet-stream").is_ok());
            assert!(p.check("a.png", "").is_ok());
            assert!(p.check("a.gif", "image/gif").is_err());
        }
    }

    #[test]
    fn extension_lists_follow_lucee_precedence() {
        // Default block list: server-side source is refused with no accept at all.
        assert!(policy("", true).check("x.cfm", "text/plain").is_err());
        assert!(policy("", true).check("x.PHP", "text/plain").is_err());
        assert!(policy("", true).check("x.txt", "text/plain").is_ok());
        // An explicit allow list overrides the block list entirely.
        let mut p = policy("", true);
        p.allowed = Some(vec!["cfm".into()]);
        assert!(p.check("x.cfm", "text/plain").is_ok());
        assert!(p.check("x.txt", "text/plain").is_err());
        // An explicit block list replaces the default one.
        let mut p = policy("", true);
        p.blocked = Some(vec!["exe".into()]);
        assert!(p.check("x.exe", "text/plain").is_err());
        assert!(p.check("x.cfm", "text/plain").is_ok());
        // Application.cfc list replaces the default one too.
        let mut p = policy("", true);
        p.app_blocked = Some(vec!["exe".into()]);
        assert!(p.check("x.cfm", "text/plain").is_ok());
        assert!(p.check("x.exe", "text/plain").is_err());
    }

    #[test]
    fn extension_list_accepts_list_or_array_with_dots() {
        let s = extension_list(Some(&CfmlValue::string(".jpg, *.PNG,gif,,".to_string())));
        assert_eq!(s, Some(vec!["jpg".into(), "png".into(), "gif".into()]));
        let a = extension_list(Some(&CfmlValue::array(vec![CfmlValue::string(".Pdf".to_string())])));
        assert_eq!(a, Some(vec!["pdf".into()]));
        assert_eq!(extension_list(Some(&CfmlValue::string(" ".to_string()))), None);
        assert_eq!(extension_list(None), None);
    }
}
