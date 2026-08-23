//! Lucee-compatible `S3*` and `store*` builtin functions.
//!
//! Pure builtins — they take only `Vec<CfmlValue>` and have no VM access. The
//! credential resolution chain is:
//!   1. inline function args (accessKeyId, secretKey, host)
//!   2. env vars (AWS_*, LUCEE_S3_*)
//!   3. (Application.cfc `this.s3` integration is currently routed via env vars)
//!
//! All ops go through `crate::s3::global_clients()` and `block_on()`.

#![cfg(feature = "s3")]

use crate::s3::{
    arg_opt_string, arg_string, buckets_to_array, client_and_config, client_and_config_for_url,
    global_clients, guess_content_type, objects_to_query_struct, s3_clear_bucket,
    s3_copy_object, s3_create_bucket, s3_delete_object, s3_generate_presigned_url,
    s3_get_metadata, s3_get_object, s3_head_bucket, s3_head_object, s3_list_buckets,
    s3_list_objects, s3_put_object, S3Config, S3Url,
};
use cfml_common::{
    dynamic::CfmlValue,
    vm::{CfmlError, CfmlErrorType, CfmlResult},
};

fn err(msg: impl Into<String>) -> CfmlError {
    CfmlError::new(msg.into(), CfmlErrorType::Custom("S3".to_string()))
}

/// Extract optional credential triple from variadic args at given positions.
/// Lucee accepts `accessKeyId`, `secretKey`, `host` typically as the last three
/// positional args of every S3 function.
fn cred_args(args: &[CfmlValue], start: usize) -> (Option<String>, Option<String>, Option<String>) {
    (
        arg_opt_string(args, start),
        arg_opt_string(args, start + 1),
        arg_opt_string(args, start + 2),
    )
}

/// Resolve a client AND its config (so callers can apply `key_prefix`).
fn client_for_args(
    inline_key: Option<&str>,
    inline_secret: Option<&str>,
    inline_host: Option<&str>,
) -> Result<(aws_sdk_s3::Client, S3Config), CfmlError> {
    client_and_config(inline_key, inline_secret, inline_host, None)
}

/// Body of a `value` arg: Strings are encoded as bytes using the optional
/// charset (default UTF-8); Binary is passed through; numbers are stringified.
fn value_to_bytes(v: &CfmlValue, _charset: Option<&str>) -> Result<Vec<u8>, CfmlError> {
    match v {
        CfmlValue::Binary(b) => Ok(b.clone()),
        CfmlValue::String(s) => Ok(s.as_bytes().to_vec()),
        CfmlValue::Int(i) => Ok(i.to_string().into_bytes()),
        CfmlValue::Double(d) => Ok(d.to_string().into_bytes()),
        CfmlValue::Bool(b) => Ok(b.to_string().into_bytes()),
        _ => Err(err("S3 value must be a string, binary, or scalar")),
    }
}

// ---------- Functions ----------

/// S3Read(bucket, object [, charset, accessKeyId, secretKey, host])
pub fn fn_s3_read(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let _charset = arg_opt_string(&args, 2);
    let (k, s, h) = cred_args(&args, 3);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let key = cfg.full_key(&object);
    let bytes = s3_get_object(&client, &bucket, &key)?;
    let text = String::from_utf8(bytes)
        .map_err(|e| err(format!("S3Read: non-UTF-8 body for {}/{}: {}", bucket, key, e)))?;
    Ok(CfmlValue::string(text))
}

/// S3ReadBinary(bucket, object [, accessKeyId, secretKey, host])
pub fn fn_s3_read_binary(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let (k, s, h) = cred_args(&args, 2);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let bytes = s3_get_object(&client, &bucket, &cfg.full_key(&object))?;
    Ok(CfmlValue::Binary(bytes))
}

/// S3Write(bucket, object, value [, charset, mimeType, acl, location, accessKeyId, secretKey, host])
pub fn fn_s3_write(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let value = args
        .get(2)
        .ok_or_else(|| err("S3Write: missing value argument"))?;
    let charset = arg_opt_string(&args, 3);
    let mime = arg_opt_string(&args, 4);
    let _acl = arg_opt_string(&args, 5);
    let _location = arg_opt_string(&args, 6);
    let (k, s, h) = cred_args(&args, 7);

    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let body = value_to_bytes(value, charset.as_deref())?;
    let ct = mime.or_else(|| guess_content_type(&object).map(|s| s.to_string()));
    s3_put_object(&client, &bucket, &cfg.full_key(&object), body, ct.as_deref())?;
    Ok(CfmlValue::Null)
}

/// S3Upload(bucket, object, source [, acl, location, accessKeyId, secretKey, host])
pub fn fn_s3_upload(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let source = arg_string(&args, 2)?;
    let _acl = arg_opt_string(&args, 3);
    let _location = arg_opt_string(&args, 4);
    let (k, s, h) = cred_args(&args, 5);

    let body = std::fs::read(&source)
        .map_err(|e| err(format!("S3Upload: failed to read source '{}': {}", source, e)))?;
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let ct = guess_content_type(&object).map(|s| s.to_string());
    s3_put_object(&client, &bucket, &cfg.full_key(&object), body, ct.as_deref())?;
    Ok(CfmlValue::Null)
}

/// S3Download(bucket, object [, target, charset, accessKeyId, secretKey, host])
///
/// If `target` is supplied, the bytes are written to that local path and the
/// function returns true. Otherwise the body is returned as a String.
pub fn fn_s3_download(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let target = arg_opt_string(&args, 2);
    let _charset = arg_opt_string(&args, 3);
    let (k, s, h) = cred_args(&args, 4);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let key = cfg.full_key(&object);
    let bytes = s3_get_object(&client, &bucket, &key)?;
    if let Some(path) = target {
        std::fs::write(&path, &bytes).map_err(|e| {
            err(format!(
                "S3Download: failed to write target '{}': {}",
                path, e
            ))
        })?;
        return Ok(CfmlValue::Bool(true));
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| err(format!("S3Download: non-UTF-8 body for {}/{}: {}", bucket, key, e)))?;
    Ok(CfmlValue::string(text))
}

/// S3ListBuckets([accessKeyId, secretKey, host])
pub fn fn_s3_list_buckets(args: Vec<CfmlValue>) -> CfmlResult {
    let (k, s, h) = cred_args(&args, 0);
    let (client, _cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let buckets = s3_list_buckets(&client)?;
    Ok(buckets_to_array(buckets))
}

/// S3ListBucket(bucket [, prefix, maxKeys, accessKeyId, secretKey, host])
pub fn fn_s3_list_bucket(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let prefix = arg_opt_string(&args, 1).unwrap_or_default();
    let max_keys = match args.get(2) {
        Some(CfmlValue::Int(i)) => Some(*i as i32),
        Some(CfmlValue::Double(d)) => Some(*d as i32),
        Some(CfmlValue::String(s)) => s.parse::<i32>().ok(),
        _ => None,
    };
    let (k, s, h) = cred_args(&args, 3);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let mut objects = s3_list_objects(&client, &bucket, &cfg.full_prefix(&prefix), None, max_keys)?;
    // Strip the configured prefix from returned keys so the script only sees
    // its own scoped keyspace.
    if let Some(strip) = cfg.key_prefix.as_deref() {
        if !strip.is_empty() {
            for o in objects.iter_mut() {
                if let Some(rest) = o.key.strip_prefix(strip) {
                    o.key = rest.to_string();
                }
            }
        }
    }
    Ok(objects_to_query_struct(objects))
}

/// S3CreateBucket(bucket [, acl, location, accessKeyId, secretKey, host])
pub fn fn_s3_create_bucket(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let acl = arg_opt_string(&args, 1);
    let location = arg_opt_string(&args, 2).unwrap_or_else(|| "us-east-1".to_string());
    let (k, s, h) = cred_args(&args, 3);
    let (client, _cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    s3_create_bucket(&client, &bucket, &location, acl.as_deref())?;
    Ok(CfmlValue::Null)
}

/// S3Delete(bucket [, object, force, accessKeyId, secretKey, host])
///
/// Without `object` → delete the bucket. With `object` → delete a single key.
/// `force=true` on bucket delete will empty it first.
pub fn fn_s3_delete(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_opt_string(&args, 1);
    let force = match args.get(2) {
        Some(CfmlValue::Bool(b)) => *b,
        Some(CfmlValue::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "true" | "yes" | "1"),
        _ => false,
    };
    let (k, s, h) = cred_args(&args, 3);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    match object {
        Some(obj) => s3_delete_object(&client, &bucket, &cfg.full_key(&obj))?,
        None => crate::s3::s3_delete_bucket(&client, &bucket, force)?,
    }
    Ok(CfmlValue::Null)
}

/// S3ClearBucket(bucket [, accessKeyId, secretKey, host])
pub fn fn_s3_clear_bucket(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let (k, s, h) = cred_args(&args, 1);
    let (client, _cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    s3_clear_bucket(&client, &bucket)?;
    Ok(CfmlValue::Null)
}

/// S3Exists(bucket [, object, accessKeyId, secretKey, host])
pub fn fn_s3_exists(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_opt_string(&args, 1);
    let (k, s, h) = cred_args(&args, 2);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let exists = match object {
        Some(obj) => s3_head_object(&client, &bucket, &cfg.full_key(&obj))?,
        None => s3_head_bucket(&client, &bucket)?,
    };
    Ok(CfmlValue::Bool(exists))
}

/// S3Copy(srcBucket, srcObject, trgBucket [, trgObject, acl, accessKeyId, secretKey, host])
pub fn fn_s3_copy(args: Vec<CfmlValue>) -> CfmlResult {
    let src_bucket = arg_string(&args, 0)?;
    let src_object = arg_string(&args, 1)?;
    let trg_bucket = arg_string(&args, 2)?;
    let trg_object = arg_opt_string(&args, 3).unwrap_or_else(|| src_object.clone());
    let _acl = arg_opt_string(&args, 4);
    let (k, s, h) = cred_args(&args, 5);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    s3_copy_object(
        &client,
        &src_bucket,
        &cfg.full_key(&src_object),
        &trg_bucket,
        &cfg.full_key(&trg_object),
    )?;
    Ok(CfmlValue::Null)
}

/// S3Move(srcBucket, srcObject, trgBucket [, trgObject, acl, accessKeyId, secretKey, host])
pub fn fn_s3_move(args: Vec<CfmlValue>) -> CfmlResult {
    let src_bucket = arg_string(&args, 0)?;
    let src_object = arg_string(&args, 1)?;
    let trg_bucket = arg_string(&args, 2)?;
    let trg_object = arg_opt_string(&args, 3).unwrap_or_else(|| src_object.clone());
    let _acl = arg_opt_string(&args, 4);
    let (k, s, h) = cred_args(&args, 5);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let full_src = cfg.full_key(&src_object);
    let full_trg = cfg.full_key(&trg_object);
    s3_copy_object(&client, &src_bucket, &full_src, &trg_bucket, &full_trg)?;
    s3_delete_object(&client, &src_bucket, &full_src)?;
    Ok(CfmlValue::Null)
}

/// S3GetMetaData(bucket, object [, accessKeyId, secretKey, host])
/// Also accepts a single s3:// URL as the first arg (then object is omitted).
pub fn fn_s3_get_metadata(args: Vec<CfmlValue>) -> CfmlResult {
    // Path A: first arg is an s3:// URL
    if let Some(CfmlValue::String(first)) = args.first() {
        if first.to_lowercase().starts_with("s3://") {
            let url = S3Url::parse(first)?;
            let (k, s, h) = cred_args(&args, 1);
            let key = if !url.key.is_empty() {
                url.key.clone()
            } else if let Some(CfmlValue::String(o)) = args.get(1) {
                if o.to_lowercase().starts_with("s3://") {
                    return Err(err("S3GetMetaData: second arg looks like another URL"));
                }
                (**o).clone()
            } else {
                String::new()
            };
            let cfg = crate::s3::S3Config::resolve(
                k.as_deref().or(url.access_key.as_deref()),
                s.as_deref().or(url.secret_key.as_deref()),
                h.as_deref().or(url.host.as_deref()),
                None,
            )?;
            let client = global_clients().get_or_create(&cfg);
            let meta = s3_get_metadata(&client, &url.bucket, &key)?;
            return Ok(CfmlValue::strukt(meta));
        }
    }
    // Path B: bucket, object, [creds]
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let (k, s, h) = cred_args(&args, 2);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let meta = s3_get_metadata(&client, &bucket, &cfg.full_key(&object))?;
    Ok(CfmlValue::strukt(meta))
}

/// S3GeneratePresignedURL(bucket, object [, expires=60, method="GET", accessKeyId, secretKey, host])
///
/// `expires` is in **minutes** (Lucee convention).
/// Lucee's default presign window is 15 minutes.
const PRESIGN_DEFAULT_SECS: u64 = 900;

/// How long a presigned URL should live, from the `expireDate` argument.
///
/// Lucee types this argument `datetime` and signs the window between now and
/// that instant. This engine's own positional API documented it as a plain
/// number of MINUTES, so both spellings are accepted: a value that parses as a
/// datetime is treated as an absolute expiry, anything numeric as minutes.
/// The two cannot be confused — a CFML datetime never looks like a bare number
/// here — and dropping the numeric form would silently shorten every existing
/// positional caller's URL.
fn presign_expiry_secs(v: Option<&CfmlValue>) -> u64 {
    let Some(v) = v else {
        return PRESIGN_DEFAULT_SECS;
    };
    match v {
        CfmlValue::Null => PRESIGN_DEFAULT_SECS,
        CfmlValue::Int(i) if *i > 0 => (*i as u64) * 60,
        CfmlValue::Double(d) if *d > 0.0 => (*d as u64) * 60,
        other => {
            let raw = other.as_string();
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return PRESIGN_DEFAULT_SECS;
            }
            if let Ok(mins) = trimmed.parse::<u64>() {
                return mins.saturating_mul(60);
            }
            match crate::builtins::parse_datetime_to_epoch_secs(trimmed) {
                // Lucee measures the window from the call, so an expiry
                // already in the past yields no window rather than a wrap.
                Some(target) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    (target - now).max(0) as u64
                }
                None => PRESIGN_DEFAULT_SECS,
            }
        }
    }
}

pub fn fn_s3_generate_presigned_url(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_string(&args, 1)?;
    let expires_secs = presign_expiry_secs(args.get(2));
    let method = arg_opt_string(&args, 3).unwrap_or_else(|| "GET".to_string());
    let (k, s, h) = cred_args(&args, 4);
    let (client, cfg) = client_for_args(k.as_deref(), s.as_deref(), h.as_deref())?;
    let url = s3_generate_presigned_url(
        &client,
        &bucket,
        &cfg.full_key(&object),
        expires_secs,
        &method,
    )?;
    Ok(CfmlValue::string(url))
}

/// S3GenerateURI(bucket [, object, style="virtualhost", secure=true, host])
///
/// Builds a non-signed canonical URL. `style` may be `virtualhost` (default)
/// or `path`. `host` overrides the AWS endpoint (e.g. R2).
pub fn fn_s3_generate_uri(args: Vec<CfmlValue>) -> CfmlResult {
    let bucket = arg_string(&args, 0)?;
    let object = arg_opt_string(&args, 1).unwrap_or_default();
    let style = arg_opt_string(&args, 2)
        .unwrap_or_else(|| "virtualhost".to_string())
        .to_lowercase();
    let secure = match args.get(3) {
        Some(CfmlValue::Bool(b)) => *b,
        Some(CfmlValue::String(s)) => !matches!(s.to_lowercase().as_str(), "false" | "no" | "0"),
        _ => true,
    };
    let host = arg_opt_string(&args, 4);
    let scheme = if secure { "https" } else { "http" };

    let url = match host {
        Some(h) => {
            let h = h
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/');
            if style == "path" {
                format!("{}://{}/{}/{}", scheme, h, bucket, object)
            } else {
                format!("{}://{}.{}/{}", scheme, bucket, h, object)
            }
        }
        None => {
            if style == "path" {
                format!("{}://s3.amazonaws.com/{}/{}", scheme, bucket, object)
            } else {
                format!("{}://{}.s3.amazonaws.com/{}", scheme, bucket, object)
            }
        }
    };
    // Strip dangling slash if no object was supplied.
    let url = if object.is_empty() {
        url.trim_end_matches('/').to_string()
    } else {
        url
    };
    Ok(CfmlValue::string(url))
}

/// StoreGetMetadata(url) — parses an `s3://` URL and returns the head metadata.
pub fn fn_store_get_metadata(args: Vec<CfmlValue>) -> CfmlResult {
    let raw = arg_string(&args, 0)?;
    let url = S3Url::parse(&raw)?;
    let (client, cfg) = client_and_config_for_url(&url, None)?;
    let meta = s3_get_metadata(&client, &url.bucket, &cfg.full_key(&url.key))?;
    Ok(CfmlValue::strukt(meta))
}
