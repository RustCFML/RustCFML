//! Transparent `s3://` VFS intercept for file/directory CFML builtins.
//!
//! Compiled only when the `s3` feature is enabled on cfml-vm (which also
//! enables it on cfml-stdlib via Cargo feature passthrough).
//!
//! When a CFML script calls `fileRead("s3://bucket/key")` etc., the VM
//! dispatcher consults [`CfmlVirtualMachine::s3_intercept`] before the regular
//! builtin path. The intercept returns `Some(result)` when it handled the
//! call, or `None` to fall through to the existing on-disk implementation.

#![cfg(feature = "s3")]

use crate::CfmlVirtualMachine;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};
use cfml_stdlib::s3::{
    client_and_config_for_url, guess_content_type, objects_to_query_struct, s3_copy_object,
    s3_delete_object, s3_get_object, s3_head_object, s3_list_objects, s3_put_object,
    S3AppConfig, S3Url,
};

fn err(msg: impl Into<String>) -> CfmlError {
    CfmlError::new(msg.into(), CfmlErrorType::Custom("S3".to_string()))
}

fn is_s3_string(v: &CfmlValue) -> Option<String> {
    match v {
        CfmlValue::String(s) if s.to_lowercase().starts_with("s3://") => Some((**s).clone()),
        _ => None,
    }
}

fn nth_string(args: &[CfmlValue], idx: usize) -> Option<&str> {
    match args.get(idx) {
        Some(CfmlValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

impl CfmlVirtualMachine {
    /// Extract `this.s3` from the current Application.cfc template, if any.
    pub(crate) fn s3_app_config(&self) -> Option<S3AppConfig> {
        let tpl = self.app_cfc_template.as_ref()?;
        let s = match tpl {
            CfmlValue::Struct(s) => s,
            _ => return None,
        };
        let s3v = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("s3"))
            .map(|(_, v)| v)?;
        S3AppConfig::from_value(&s3v)
    }

    /// If `path` matches an `s3://`-targeted CFML mapping, rewrite it to the
    /// fully-qualified s3:// URL by replacing the mapping name with the
    /// mapping's target. Returns `None` when no mapping matches.
    ///
    /// Example: `this.mappings["/logs"] = "s3://key:sec@bucket/logs/"` makes
    /// `fileRead("/logs/today.txt")` resolve to
    /// `s3://key:sec@bucket/logs/today.txt`.
    fn resolve_s3_mapping(&self, path: &str) -> Option<String> {
        // Mapping names are normalized to /name/ at parse time. Match prefix.
        let path_with_slash = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };
        for m in &self.mappings {
            if !m.path.to_lowercase().starts_with("s3://") {
                continue;
            }
            if path_with_slash.starts_with(&m.name) {
                let target = m.path.trim_end_matches('/');
                let remainder = &path_with_slash[m.name.len()..];
                return Some(format!("{}/{}", target, remainder));
            }
            // Also match the exact mapping name minus trailing slash (e.g.
            // fileRead("/logs") with mapping name "/logs/").
            let name_no_trail = m.name.trim_end_matches('/');
            if path_with_slash == name_no_trail {
                return Some(m.path.trim_end_matches('/').to_string());
            }
        }
        None
    }

    /// Intercept CFML file/directory ops where the path argument is an
    /// `s3://...` URL (or maps to one via `this.mappings`). Returns `None`
    /// when the call should fall through to the regular on-disk dispatcher.
    pub(crate) fn s3_intercept(&self, name: &str, args: &[CfmlValue]) -> Option<CfmlResult> {
        // The `s3*()` builtins take only a Vec<CfmlValue>, so there is nowhere
        // in their signature for an application context to arrive. Publish
        // `this.s3` for them here — this runs on the way to every builtin, so
        // the settings always match the application actually running (GH
        // #334). They are all declared VM-intercepted, so none of them is
        // compile-time bound past this point. Cheap: the name test rejects
        // everything else immediately.
        //
        // This deliberately lives here rather than in `call_function`: the
        // intercept-declaration guard scans that function for dispatched-on
        // names, and a `starts_with` prefix there reads to it as a dispatch on
        // the literal name "s3", which is not a builtin.
        if name.starts_with("s3") || name == "storegetmetadata" {
            cfml_stdlib::s3::set_app_s3_settings(self.s3_app_config().map(|c| c.settings));
        }

        match name {
            "fileread" | "filereadbinary" | "fileexists" | "filedelete"
            | "filewrite" | "fileappend" | "filemove" | "filecopy"
            | "directorylist" | "directoryexists" | "directorycreate"
            | "directorydelete" | "directoryrename" | "directorycopy" => {}
            _ => return None,
        }

        // Resolve arg 0 (and optionally arg 1) against s3 mappings. We have to
        // decide whether to intercept *before* we know the resolved URL, so
        // try the cheap "already s3://" path first, then fall through to a
        // mapping rewrite.
        let first_raw = nth_string(args, 0)?.to_string();
        let (path, came_from_mapping) =
            if is_s3_string(&CfmlValue::string(first_raw.clone())).is_some() {
                (first_raw, false)
            } else if let Some(rewritten) = self.resolve_s3_mapping(&first_raw) {
                (rewritten, true)
            } else {
                return None;
            };

        let app = self.s3_app_config();
        let url = match S3Url::parse(&path) {
            Ok(u) => u,
            Err(e) => return Some(Err(e)),
        };
        let (client, cfg) = match client_and_config_for_url(&url, app.as_ref()) {
            Ok(p) => p,
            Err(e) => return Some(Err(e)),
        };

        // Helper closure to apply the key prefix *unless* the URL came from a
        // mapping expansion (mappings already encode the desired prefix) or
        // had inline creds (the latter is already enforced inside
        // `S3Config::resolve`, but `came_from_mapping` is our extra signal).
        let full_key = |key: &str| -> String {
            if came_from_mapping {
                key.to_string()
            } else {
                cfg.full_key(key)
            }
        };
        // `full_prefix` (for list operations) is currently unused inside the
        // intercept itself because directorylist derives its prefix from
        // `src_dir`. Kept as a comment so future operations that take a
        // separate prefix arg can call it: `cfg.full_prefix(user_prefix)`.

        // Resolve the destination arg (for copy/move/rename) through the same
        // mapping-or-direct logic, returning the parsed URL + a "from mapping"
        // flag so we know whether to apply the prefix.
        let resolve_dst = |raw: &str| -> Result<(S3Url, bool), CfmlError> {
            let (resolved, from_mapping) = if is_s3_string(&CfmlValue::string(raw.to_string()))
                .is_some()
            {
                (raw.to_string(), false)
            } else if let Some(rewritten) = self.resolve_s3_mapping(raw) {
                (rewritten, true)
            } else {
                return Err(err(format!(
                    "{}: destination must be an s3:// URL or s3 mapping",
                    name
                )));
            };
            let u = S3Url::parse(&resolved)?;
            Ok((u, from_mapping))
        };
        let dst_full_key = |u: &S3Url, from_mapping: bool| -> String {
            if from_mapping {
                u.key.clone()
            } else {
                cfg.full_key(&u.key)
            }
        };

        // The current url.key, with prefix applied (if applicable).
        let src_key = full_key(&url.key);

        // Build the "directory" form (trailing-slash key) from src_key.
        let src_dir = if src_key.is_empty() || src_key.ends_with('/') {
            src_key.clone()
        } else {
            format!("{}/", src_key)
        };

        let res: CfmlResult = match name {
            "fileread" => s3_get_object(&client, &url.bucket, &src_key)
                .and_then(|bytes| {
                    String::from_utf8(bytes)
                        .map_err(|e| err(format!("fileRead({}): non-UTF-8 body: {}", path, e)))
                })
                .map(CfmlValue::string),

            "filereadbinary" => s3_get_object(&client, &url.bucket, &src_key)
                .map(CfmlValue::Binary),

            "fileexists" => s3_head_object(&client, &url.bucket, &src_key).map(CfmlValue::Bool),

            "filedelete" => s3_delete_object(&client, &url.bucket, &src_key)
                .map(|_| CfmlValue::Null),

            "filewrite" => {
                let body = match args.get(1) {
                    Some(CfmlValue::Binary(b)) => b.clone(),
                    Some(CfmlValue::String(s)) => s.as_bytes().to_vec(),
                    Some(v) => v.as_string().into_bytes(),
                    None => return Some(Err(err("fileWrite: missing content argument"))),
                };
                let ct = guess_content_type(&src_key).map(|s| s.to_string());
                s3_put_object(&client, &url.bucket, &src_key, body, ct.as_deref())
                    .map(|_| CfmlValue::Null)
            }

            "fileappend" => {
                let existing = match s3_get_object(&client, &url.bucket, &src_key) {
                    Ok(b) => b,
                    Err(_) => Vec::new(),
                };
                let addition = match args.get(1) {
                    Some(CfmlValue::Binary(b)) => b.clone(),
                    Some(CfmlValue::String(s)) => s.as_bytes().to_vec(),
                    Some(v) => v.as_string().into_bytes(),
                    None => return Some(Err(err("fileAppend: missing content argument"))),
                };
                let mut combined = existing;
                combined.extend_from_slice(&addition);
                let ct = guess_content_type(&src_key).map(|s| s.to_string());
                s3_put_object(&client, &url.bucket, &src_key, combined, ct.as_deref())
                    .map(|_| CfmlValue::Null)
            }

            "filecopy" | "filemove" => {
                let dst_raw = match nth_string(args, 1) {
                    Some(d) => d,
                    None => {
                        return Some(Err(err(format!(
                            "{}: missing destination argument",
                            name
                        ))))
                    }
                };
                let (dst_url, dst_from_mapping) = match resolve_dst(dst_raw) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e)),
                };
                let dst_key = dst_full_key(&dst_url, dst_from_mapping);
                let copied =
                    s3_copy_object(&client, &url.bucket, &src_key, &dst_url.bucket, &dst_key);
                if name == "filemove" {
                    copied
                        .and_then(|_| s3_delete_object(&client, &url.bucket, &src_key))
                        .map(|_| CfmlValue::Null)
                } else {
                    copied.map(|_| CfmlValue::Null)
                }
            }

            "directorylist" => {
                let recurse = args.get(1).map(|v| v.is_true()).unwrap_or(false);
                let list_info = args
                    .get(2)
                    .map(|v| v.as_string().to_lowercase())
                    .unwrap_or_else(|| "path".to_string());
                let delim: Option<&str> = if recurse { None } else { Some("/") };
                let objects = match s3_list_objects(&client, &url.bucket, &src_dir, delim, None) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                if list_info == "query" {
                    Ok(objects_to_query_struct(objects))
                } else {
                    // Default: array of full s3:// URIs (with prefix stripped
                    // again so the script sees its own keyspace).
                    let strip = cfg.key_prefix.clone().unwrap_or_default();
                    let mut arr = Vec::with_capacity(objects.len());
                    for o in objects {
                        let visible = if !came_from_mapping
                            && !strip.is_empty()
                            && o.key.starts_with(&strip)
                        {
                            &o.key[strip.len()..]
                        } else {
                            o.key.as_str()
                        };
                        arr.push(CfmlValue::string(format!(
                            "s3://{}/{}",
                            url.bucket, visible
                        )));
                    }
                    Ok(CfmlValue::array(arr))
                }
            }

            "directoryexists" => {
                let objects = match s3_list_objects(&client, &url.bucket, &src_dir, Some("/"), Some(1)) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Ok(CfmlValue::Bool(!objects.is_empty()))
            }

            "directorycreate" => {
                s3_put_object(&client, &url.bucket, &src_dir, Vec::new(), None)
                    .map(|_| CfmlValue::Null)
            }

            "directorydelete" => {
                let objects = match s3_list_objects(&client, &url.bucket, &src_dir, None, None) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                for o in objects {
                    if let Err(e) = s3_delete_object(&client, &url.bucket, &o.key) {
                        return Some(Err(e));
                    }
                }
                Ok(CfmlValue::Null)
            }

            "directoryrename" | "directorycopy" => {
                let dst_raw = match nth_string(args, 1) {
                    Some(d) => d,
                    None => {
                        return Some(Err(err(format!(
                            "{}: missing destination argument",
                            name
                        ))))
                    }
                };
                let (dst_url, dst_from_mapping) = match resolve_dst(dst_raw) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e)),
                };
                let dst_key = dst_full_key(&dst_url, dst_from_mapping);
                let dst_dir = if dst_key.ends_with('/') || dst_key.is_empty() {
                    dst_key.clone()
                } else {
                    format!("{}/", dst_key)
                };
                let objects = match s3_list_objects(&client, &url.bucket, &src_dir, None, None) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                for o in &objects {
                    let suffix = o.key.strip_prefix(&src_dir).unwrap_or(&o.key);
                    let new_key = format!("{}{}", dst_dir, suffix);
                    if let Err(e) = s3_copy_object(&client, &url.bucket, &o.key, &dst_url.bucket, &new_key) {
                        return Some(Err(e));
                    }
                }
                if name == "directoryrename" {
                    for o in &objects {
                        if let Err(e) = s3_delete_object(&client, &url.bucket, &o.key) {
                            return Some(Err(e));
                        }
                    }
                }
                Ok(CfmlValue::Null)
            }

            _ => return None,
        };

        Some(res)
    }
}

// IndexMap is referenced via `Arc::new(IndexMap::...)` patterns elsewhere; keep
// the import alive so future expansions don't have to re-import.
#[allow(dead_code)]
fn _unused_indexmap_marker() -> ValueMap {
    ValueMap::default()
}
