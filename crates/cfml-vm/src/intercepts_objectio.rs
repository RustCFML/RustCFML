//! `objectSave(value, filePath)` / `objectLoad(filePath)` — Lucee's file forms.
//!
//! Lucee's `objectSave` writes the serialized bytes to `filePath` when one is
//! given (and still returns them), and `objectLoad` treats a non-binary argument
//! as a path to read. RustCFML ignored the path and treated a path string as the
//! blob itself. Preside's DiskStore could use these forms to skip ColdBox's
//! base64 layer entirely.
//!
//! The file I/O goes through `fileWrite` / `fileReadBinary` via `call_function`,
//! so template-relative path resolution, the VFS sandbox, s3 mappings and
//! written-file cache invalidation all apply exactly as they do to those BIFs.
//! That is also why both names are declared VM-intercepted: a compile-time
//! bound call would skip all of it.

use super::*;

/// Names this module handles. Both fall through (via
/// [`intercepts_common::unhandled`]) when no file path is involved.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "objectsave" | "objectload")
}

/// The prefix every `objectSave()` blob starts with (any format version).
const BLOB_PREFIX: &[u8] = b"RCFMLOBJ";

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_objectio(
        &mut self,
        name_lower: &str,
        args: Vec<CfmlValue>,
        parent_locals: &ValueMap,
    ) -> CfmlResult {
        match name_lower {
            "objectsave" => {
                let path = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                if path.trim().is_empty() {
                    return Err(intercepts_common::unhandled());
                }
                let value = args.into_iter().next().unwrap_or(CfmlValue::Null);
                let blob = self.call_named_builtin("objectSave", vec![value])?;
                self.call_function(
                    &Self::builtin_fn_value("fileWrite"),
                    vec![CfmlValue::string(path), blob.clone()],
                    parent_locals,
                )?;
                Ok(blob)
            }
            "objectload" => match args.first() {
                // A string that is not itself a blob is a file path (Lucee).
                Some(CfmlValue::String(s)) if !s.as_bytes().starts_with(BLOB_PREFIX) => {
                    let bytes = self.call_function(
                        &Self::builtin_fn_value("fileReadBinary"),
                        vec![CfmlValue::String(s.clone())],
                        parent_locals,
                    )?;
                    self.call_named_builtin("objectLoad", vec![bytes])
                }
                _ => Err(intercepts_common::unhandled()),
            },
            _ => Err(intercepts_common::unhandled()),
        }
    }
}
