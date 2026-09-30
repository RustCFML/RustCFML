//! `org.pixl8.s3storageprovider.Service` — the one Java class
//! `preside-ext-s3-storage-provider` constructs, implemented natively so the
//! extension runs unmodified. Without it a configured extension fails at
//! application start ("Java class … is not supported") and every request
//! retries the failing boot.
//!
//! The provider CFC calls:
//!
//! | method | returns |
//! |---|---|
//! | `init(region, bucket, accessKey, secretKey)` | the service (no network I/O; a blank region throws) |
//! | `checkS3Access()`, `checkBucketAccess()` | boolean, never throws |
//! | `checkBucketRegion()` | boolean (location constraint == region), throws on S3 errors |
//! | `listObjects(prefix)` | query `name, path, size, lastmodified`, every page |
//! | `getObject(key)` / `getObject(key, filePath)` | binary / writes a NEW file |
//! | `getObjectInfo(key)` | `{ size, lastmodified }`, a missing key throws |
//! | `deleteObject(key)` | void |
//! | `putObject(key, bytes \| localPath, mimetype, disposition, isPrivate, isTrashed)` | void |
//! | `moveObject(src, dst, mimetype, disposition, isPrivate, isTrashed)` | void |
//! | `getPresignedUrl(key, minutes)` | URL string |
//!
//! Behaviour is the jar's (v2.1.1), pinned by running it on Lucee 7.1 against
//! MinIO; the S3 calls live in `cfml_stdlib::s3_service`. See `s3shim.md`.
//!
//! The secret key never goes on the CFML-visible struct (`writeDump` shows a
//! shim's `__` fields): instances hold only an id into a Rust-side registry.

use super::*;
#[cfg(feature = "s3")]
use crate::shim_util::{field, shim};

pub(crate) const SERVICE_CLASS: &str = "org.pixl8.s3storageprovider.service";
const DISPLAY_CLASS: &str = "org.pixl8.s3storageprovider.Service";

pub(crate) fn constructs(class_lower: &str) -> bool {
    class_lower == SERVICE_CLASS
}

pub(crate) fn handles(class_lower: &str) -> bool {
    class_lower == SERVICE_CLASS
}

fn no_such_method(method: &str) -> CfmlError {
    CfmlError::new(
        format!("No matching method for {}.{}() found.", DISPLAY_CLASS, method),
        CfmlErrorType::Custom("java.lang.NoSuchMethodException".to_string()),
    )
}

#[cfg(feature = "s3")]
mod imp {
    use super::*;
    use cfml_stdlib::s3_service::{self as svc, PutBody, ServiceConfig};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    fn instances() -> &'static Mutex<HashMap<u64, Arc<ServiceConfig>>> {
        static I: OnceLock<Mutex<HashMap<u64, Arc<ServiceConfig>>>> = OnceLock::new();
        I.get_or_init(|| Mutex::new(HashMap::new()))
    }

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    fn config_of(object: &CfmlValue, method: &str) -> Result<Arc<ServiceConfig>, CfmlError> {
        let id = match field(object, "__s3sp_id") {
            Some(CfmlValue::Int(i)) => i as u64,
            // Called on the class itself, before init(): Lucee looks for a
            // STATIC method of that name and finds none.
            _ => return Err(no_such_method(method)),
        };
        instances()
            .lock()
            .ok()
            .and_then(|m| m.get(&id).cloned())
            .ok_or_else(|| no_such_method(method))
    }

    pub(super) fn dispatch(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
        let s = |i: usize| args.get(i).map(|v| v.as_string()).unwrap_or_default();
        let b = |i: usize| args.get(i).map(|v| v.is_true()).unwrap_or(false);
        let arity = |n: usize| -> Result<(), CfmlError> {
            if args.len() == n { Ok(()) } else { Err(no_such_method(method)) }
        };
        if method == "init" {
            arity(4)?;
            let region = s(0);
            // Region.of("") throws in the SDK, so the jar's constructor does too.
            if region.trim().is_empty() {
                return Err(CfmlError::new(
                    "region must not be blank or empty.".to_string(),
                    CfmlErrorType::Custom("java.lang.IllegalArgumentException".to_string()),
                ));
            }
            let cfg = ServiceConfig { region, bucket: s(1), access_key: s(2), secret_key: s(3), endpoint: None };
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut m) = instances().lock() {
                m.insert(id, Arc::new(cfg));
            }
            // A fresh instance each time, as `new Service(...)` is: the class
            // reference createObject returned stays reusable.
            let mut inst = shim(DISPLAY_CLASS);
            inst.insert("__s3sp_id".to_string(), CfmlValue::Int(id as i64));
            return Ok(CfmlValue::strukt(inst));
        }
        let cfg = config_of(object, method)?;
        match method {
            "checks3access" => { arity(0)?; Ok(CfmlValue::Bool(svc::check_s3_access(&cfg))) }
            "checkbucketaccess" => { arity(0)?; Ok(CfmlValue::Bool(svc::check_bucket_access(&cfg))) }
            "checkbucketregion" => { arity(0)?; Ok(CfmlValue::Bool(svc::check_bucket_region(&cfg)?)) }
            "listobjects" => { arity(1)?; svc::list_objects(&cfg, &s(0)) }
            // Overloaded by arity: one argument returns the bytes, two write a file.
            "getobject" if args.len() == 1 => Ok(CfmlValue::Binary(svc::get_bytes(&cfg, &s(0))?.into())),
            "getobject" if args.len() == 2 => { svc::get_to_file(&cfg, &s(0), &s(1))?; Ok(CfmlValue::Null) }
            "getobjectinfo" => { arity(1)?; svc::get_object_info(&cfg, &s(0)) }
            "deleteobject" => { arity(1)?; svc::delete_object(&cfg, &s(0))?; Ok(CfmlValue::Null) }
            // Overloaded by the type of argument 2: binary is the byte[] form,
            // anything else is the local-file-path form.
            "putobject" => {
                arity(6)?;
                let body = match &args[1] {
                    CfmlValue::Binary(bytes) => PutBody::Bytes(bytes.to_vec()),
                    other => PutBody::File(other.as_string()),
                };
                svc::put_object(&cfg, &s(0), body, &s(2), &s(3), b(4), b(5))?;
                Ok(CfmlValue::Null)
            }
            "moveobject" => {
                arity(6)?;
                svc::move_object(&cfg, &s(0), &s(1), &s(2), &s(3), b(4), b(5))?;
                Ok(CfmlValue::Null)
            }
            "getpresignedurl" => {
                arity(2)?;
                let minutes = match &args[1] {
                    CfmlValue::Int(n) => *n,
                    CfmlValue::Double(d) => *d as i64,
                    other => other.as_string().trim().parse::<f64>().map(|d| d as i64).map_err(|_| no_such_method(method))?,
                };
                Ok(CfmlValue::string(svc::presigned_url(&cfg, &s(0), minutes)?))
            }
            _ => Err(no_such_method(method)),
        }
    }
}

impl CfmlVirtualMachine {
    pub(crate) fn construct_s3storageprovider_shim(&self) -> CfmlResult {
        #[cfg(not(feature = "s3"))]
        return Err(CfmlError::runtime(format!(
            "createObject: Java class [{}] needs S3 support, and this build of RustCFML was compiled without it (the `s3` feature)",
            DISPLAY_CLASS
        )));
        #[cfg(feature = "s3")]
        Ok(CfmlValue::strukt(shim(DISPLAY_CLASS)))
    }

    pub(crate) fn dispatch_s3storageprovider_shim(
        &mut self,
        method: &str,
        args: Vec<CfmlValue>,
        object: &CfmlValue,
    ) -> CfmlResult {
        #[cfg(feature = "s3")]
        {
            imp::dispatch(method, args, object).map_err(|e| self.wrap_error(e))
        }
        #[cfg(not(feature = "s3"))]
        {
            let _ = (args, object);
            Err(self.wrap_error(no_such_method(method)))
        }
    }
}
