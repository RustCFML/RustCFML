//! S3 (and S3-compatible) object storage support.
//!
//! Compiled only when the `s3` feature is enabled. Provides:
//! - `s3://` URL parsing (matching Lucee's S3 extension format)
//! - Credential resolution (inline → Application.cfc this.s3 → env vars → IAM)
//! - Per-endpoint `aws_sdk_s3::Client` caching
//! - Sync wrappers around async S3 operations (GetObject, PutObject, etc.)

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::Client;
use cfml_common::{
    dynamic::{CfmlValue, ValueMap},
    vm::{CfmlError, CfmlErrorType, CfmlResult},
};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------- Tokio runtime bridge ----------

/// Lazy global multi-thread runtime used when the caller isn't already inside
/// a tokio context (e.g. plain CLI script mode). When the caller is inside the
/// `#[tokio::main]` runtime (axum/serve, fetch handler) we use `Handle::current`.
static FALLBACK_RT: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("failed to build fallback tokio runtime for s3")
});

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // We're already inside a runtime; run on the current handle without
            // re-entering it (futures::executor would deadlock). Spawn a blocking
            // task and wait.
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        Err(_) => FALLBACK_RT.block_on(fut),
    }
}

fn err(msg: impl Into<String>) -> CfmlError {
    CfmlError::new(msg.into(), CfmlErrorType::Custom("S3".to_string()))
}

/// AWS SDK errors `Display` very tersely ("service error") and hide the real
/// detail in the `.source()` chain. This walks the chain and concatenates
/// every level so the resulting `CfmlError` is actually diagnosable.
fn fmt_sdk_err<E: std::error::Error>(e: &E) -> String {
    let mut out = e.to_string();
    let mut current: Option<&dyn std::error::Error> = e.source();
    while let Some(src) = current {
        out.push_str(": ");
        out.push_str(&src.to_string());
        current = src.source();
    }
    out
}

// ---------- S3 URL parsing ----------

#[derive(Debug, Clone)]
pub struct S3Url {
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub host: Option<String>,
    pub bucket: String,
    pub key: String,
}

impl S3Url {
    /// `s3://[key:secret@[host/]]bucket[/key]`
    pub fn parse(url: &str) -> Result<Self, CfmlError> {
        let rest = url
            .strip_prefix("s3://")
            .or_else(|| url.strip_prefix("S3://"))
            .ok_or_else(|| err(format!("Not an s3:// URL: {}", url)))?;

        let (creds, after_creds) = match rest.find('@') {
            Some(i) => {
                let cred_part = &rest[..i];
                let after = &rest[i + 1..];
                let (k, s) = match cred_part.find(':') {
                    Some(j) => (cred_part[..j].to_string(), cred_part[j + 1..].to_string()),
                    None => (cred_part.to_string(), String::new()),
                };
                (Some((k, s)), after)
            }
            None => (None, rest),
        };

        let mut parts = after_creds.splitn(2, '/');
        let first = parts.next().unwrap_or("").to_string();
        let remainder = parts.next().unwrap_or("").to_string();

        let (host, bucket, key) = if first.contains('.') && creds.is_some() {
            // host segment present
            let mut sub = remainder.splitn(2, '/');
            let bucket = sub.next().unwrap_or("").to_string();
            let key = sub.next().unwrap_or("").to_string();
            (Some(first), bucket, key)
        } else {
            (None, first, remainder)
        };

        if bucket.is_empty() {
            return Err(err(format!("S3 URL has no bucket: {}", url)));
        }

        let (access_key, secret_key) = match creds {
            Some((k, s)) => (
                Some(urldecode(&k)),
                if s.is_empty() { None } else { Some(urldecode(&s)) },
            ),
            None => (None, None),
        };

        Ok(S3Url {
            access_key,
            secret_key,
            host,
            bucket,
            key,
        })
    }
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------- Config resolution ----------

#[derive(Debug, Clone)]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub endpoint_url: Option<String>,
    /// Optional bucket-scoped subpath, e.g. `myapp/` — when set, every object
    /// key passed by the script gets this transparently prepended (so calls
    /// effectively address `bucket/<prefix>/<key>`). Source precedence:
    /// `this.s3.keyPrefix` → env vars (`RUSTCFML_S3_KEY_PREFIX`,
    /// `LUCEE_S3_KEYPREFIX`, `LUCEE_S3_KEY_PREFIX`).
    ///
    /// The prefix is **not** applied when the caller supplied explicit inline
    /// credentials (full `s3://key:sec@host/...` URL, or
    /// `accessKeyId`/`secretKey`/`host` function args): inline creds = explicit
    /// addressing, so the script controls the full path itself.
    pub key_prefix: Option<String>,
    /// Force path-style addressing (`https://host/bucket/key`) instead of the
    /// virtual-host default. Needed by MinIO and any provider that does not
    /// serve bucket subdomains. Source precedence: `this.s3.usePathStyle` →
    /// `RUSTCFML_S3_PATH_STYLE` / `LUCEE_S3_PATH_STYLE`.
    pub use_path_style: bool,
}

impl S3Config {
    pub fn resolve(
        inline_key: Option<&str>,
        inline_secret: Option<&str>,
        inline_host: Option<&str>,
        app_s3: Option<&ValueMap>,
    ) -> Result<Self, CfmlError> {
        let app_get = |k: &str| -> Option<String> {
            app_s3.and_then(|m| {
                m.iter().find(|(ak, _)| ak.eq_ignore_ascii_case(k)).and_then(
                    |(_, v)| match v {
                        CfmlValue::String(s) if !s.is_empty() => Some((**s).clone()),
                        _ => None,
                    },
                )
            })
        };
        let env_get = |keys: &[&str]| -> Option<String> {
            for k in keys {
                if let Ok(v) = std::env::var(k) {
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
            None
        };

        let access_key = inline_key
            .map(|s| s.to_string())
            .or_else(|| app_get("accessKeyId"))
            .or_else(|| app_get("accessKey"))
            .or_else(|| {
                env_get(&[
                    "AWS_ACCESS_KEY_ID",
                    "LUCEE_S3_ACCESSKEYID",
                    "LUCEE_S3_ACCESSKEY",
                ])
            })
            .unwrap_or_default();

        let secret_key = inline_secret
            .map(|s| s.to_string())
            .or_else(|| app_get("awsSecretKey"))
            .or_else(|| app_get("secretKey"))
            .or_else(|| {
                env_get(&[
                    "AWS_SECRET_ACCESS_KEY",
                    "LUCEE_S3_SECRETACCESSKEY",
                    "LUCEE_S3_SECRETKEY",
                ])
            })
            .unwrap_or_default();

        let region = app_get("defaultLocation")
            .or_else(|| app_get("region"))
            .or_else(|| {
                env_get(&[
                    "AWS_DEFAULT_REGION",
                    "AWS_REGION",
                    "LUCEE_S3_LOCATION",
                    "LUCEE_S3_REGION",
                ])
            })
            .unwrap_or_else(|| "us-east-1".to_string());

        let endpoint_url = inline_host
            .map(|s| normalize_endpoint(s))
            .or_else(|| app_get("host").map(|s| normalize_endpoint(&s)))
            .or_else(|| env_get(&["LUCEE_S3_HOST", "LUCEE_S3_SERVER", "AWS_ENDPOINT_URL"]).map(|s| normalize_endpoint(&s)));

        // Inline creds = explicit caller intent → suppress the app/env prefix
        // so the caller's bucket+key is used verbatim.
        let inline_present = inline_key.is_some() || inline_secret.is_some() || inline_host.is_some();
        let key_prefix = if inline_present {
            None
        } else {
            app_get("keyPrefix")
                .or_else(|| app_get("key_prefix"))
                .or_else(|| app_get("prefix"))
                .or_else(|| {
                    env_get(&[
                        "RUSTCFML_S3_KEY_PREFIX",
                        "LUCEE_S3_KEYPREFIX",
                        "LUCEE_S3_KEY_PREFIX",
                    ])
                })
                .map(normalize_prefix)
        };

        let truthy = |v: &str| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "on");
        let use_path_style = app_s3
            .and_then(|m| {
                m.iter()
                    .find(|(ak, _)| {
                        ak.eq_ignore_ascii_case("usePathStyle")
                            || ak.eq_ignore_ascii_case("pathStyle")
                    })
                    .map(|(_, v)| match v {
                        CfmlValue::Bool(b) => *b,
                        other => truthy(&other.as_string()),
                    })
            })
            .or_else(|| {
                env_get(&["RUSTCFML_S3_PATH_STYLE", "LUCEE_S3_PATH_STYLE"]).map(|v| truthy(&v))
            })
            .unwrap_or(false);

        Ok(S3Config {
            access_key,
            secret_key,
            region,
            endpoint_url,
            key_prefix,
            use_path_style,
        })
    }

    /// Prepend the configured key prefix to a script-supplied object key.
    /// Returns the key unchanged when no prefix is configured.
    pub fn full_key(&self, key: &str) -> String {
        // Exactly ONE leading slash is dropped, as Lucee's S3 resource does:
        // an object stored through Lucee as "/a/b.txt" lives at "a/b.txt", so
        // signing or fetching the slash verbatim addresses "bucket//a/b.txt",
        // a key that exists nowhere. Two slashes therefore leave one behind,
        // which is a real (if unusual) key — hence strip_prefix, not trim.
        let key = key.strip_prefix('/').unwrap_or(key);
        match &self.key_prefix {
            Some(p) => format!("{}{}", p, key),
            None => key.to_string(),
        }
    }

    /// Same as `full_key` but for list-operation prefix arguments. Empty
    /// user-supplied prefix → just the configured prefix (if any).
    pub fn full_prefix(&self, user_prefix: &str) -> String {
        match &self.key_prefix {
            Some(p) => format!("{}{}", p, user_prefix.trim_start_matches('/')),
            None => user_prefix.to_string(),
        }
    }
}

impl S3Config {
    fn cache_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.access_key,
            self.region,
            self.endpoint_url.as_deref().unwrap_or(""),
            // include secret hash so different secrets get separate clients,
            // and the addressing style so the two never share a cached client
            format!("{}:{}", self.secret_key.len(), self.use_path_style)
        )
    }
}

/// Ensure the configured prefix always ends with a single `/` and never has a
/// leading `/`. `myapp` → `myapp/`, `/foo/bar` → `foo/bar/`, `""` → `""`.
fn normalize_prefix(p: String) -> String {
    let trimmed = p.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}/", trimmed)
    }
}

fn normalize_endpoint(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("https://{}", host)
    }
}

// ---------- Client cache ----------

#[derive(Default)]
pub struct S3Clients {
    inner: Mutex<HashMap<String, Client>>,
}

impl S3Clients {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create(&self, config: &S3Config) -> Client {
        let key = config.cache_key();
        {
            let guard = self.inner.lock().unwrap();
            if let Some(c) = guard.get(&key) {
                return c.clone();
            }
        }
        let client = build_client(config);
        let mut guard = self.inner.lock().unwrap();
        guard.entry(key).or_insert(client).clone()
    }
}

fn build_client(cfg: &S3Config) -> Client {
    let creds = Credentials::new(
        cfg.access_key.clone(),
        cfg.secret_key.clone(),
        None,
        None,
        "rustcfml",
    );
    let region = Region::new(cfg.region.clone());

    let loader = aws_config::defaults(BehaviorVersion::latest())
        .credentials_provider(creds)
        .region(region);

    let conf = block_on(loader.load());

    let mut builder = aws_sdk_s3::config::Builder::from(&conf);
    if let Some(ep) = &cfg.endpoint_url {
        builder = builder.endpoint_url(ep);
    }
    // Virtual-host addressing (`https://bucket.host/key`) is what Lucee emits,
    // including against a custom host, so it is the default here too. A custom
    // endpoint used to force path style unconditionally, which made every URL
    // this engine produced differ in shape from the same call on Lucee.
    // MinIO (and any provider not serving bucket subdomains) still needs path
    // style, so it stays available through `this.s3.usePathStyle` or
    // RUSTCFML_S3_PATH_STYLE / LUCEE_S3_PATH_STYLE.
    if cfg.use_path_style {
        builder = builder.force_path_style(true);
    }
    Client::from_conf(builder.build())
}

/// The current application's `this.s3` settings.
///
/// The `s3*()` BIFs are plain `fn(Vec<CfmlValue>)` builtins with no access to
/// VM state, so there is nowhere in their signature for an application context
/// to arrive — which is why `this.s3` was previously reachable only by
/// duplicating every value into process environment variables. That workaround
/// is per-process where `this.s3` is per-application, so a container could only
/// ever hold one credential set.
///
/// The VM republishes this immediately before dispatching any `s3*` builtin,
/// so it always reflects the application that is actually running.
thread_local! {
    static APP_S3_SETTINGS: std::cell::RefCell<Option<ValueMap>> =
        const { std::cell::RefCell::new(None) };
}

/// Publish (or clear) the current application's `this.s3` settings.
pub fn set_app_s3_settings(settings: Option<ValueMap>) {
    APP_S3_SETTINGS.with(|c| *c.borrow_mut() = settings);
}

/// The current application's `this.s3` settings, if any.
pub fn app_s3_settings() -> Option<ValueMap> {
    APP_S3_SETTINGS.with(|c| c.borrow().clone())
}

pub type SharedS3Clients = Arc<S3Clients>;

/// Process-global client cache. S3 clients are cheap to keep alive and
/// expensive to rebuild (credential provider chain + HTTPS connector), so a
/// single shared cache across the whole process is the right granularity.
static GLOBAL_CLIENTS: Lazy<S3Clients> = Lazy::new(S3Clients::new);

pub fn global_clients() -> &'static S3Clients {
    &GLOBAL_CLIENTS
}

// ---------- Core operations (sync wrappers) ----------

pub fn s3_get_object(client: &Client, bucket: &str, key: &str) -> Result<Vec<u8>, CfmlError> {
    block_on(async {
        let resp = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| err(format!("GetObject {}/{} failed: {}", bucket, key, fmt_sdk_err(&e))))?;
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| err(format!("GetObject {}/{} body read failed: {}", bucket, key, fmt_sdk_err(&e))))?
            .into_bytes()
            .to_vec();
        Ok(bytes)
    })
}

pub fn s3_put_object(
    client: &Client,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
    content_type: Option<&str>,
) -> Result<(), CfmlError> {
    block_on(async {
        let mut req = client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(body.into());
        if let Some(ct) = content_type {
            req = req.content_type(ct);
        }
        req.send()
            .await
            .map_err(|e| err(format!("PutObject {}/{} failed: {}", bucket, key, fmt_sdk_err(&e))))?;
        Ok(())
    })
}

pub fn s3_head_object(client: &Client, bucket: &str, key: &str) -> Result<bool, CfmlError> {
    block_on(async {
        match client.head_object().bucket(bucket).key(key).send().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let s = format!("{:?}", e);
                if s.contains("NotFound") || s.contains("404") {
                    Ok(false)
                } else {
                    Err(err(format!("HeadObject {}/{} failed: {}", bucket, key, fmt_sdk_err(&e))))
                }
            }
        }
    })
}

pub fn s3_head_bucket(client: &Client, bucket: &str) -> Result<bool, CfmlError> {
    block_on(async {
        match client.head_bucket().bucket(bucket).send().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let s = format!("{:?}", e);
                if s.contains("NotFound") || s.contains("404") {
                    Ok(false)
                } else {
                    Err(err(format!("HeadBucket {} failed: {}", bucket, fmt_sdk_err(&e))))
                }
            }
        }
    })
}

pub fn s3_delete_object(client: &Client, bucket: &str, key: &str) -> Result<(), CfmlError> {
    block_on(async {
        client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| err(format!("DeleteObject {}/{} failed: {}", bucket, key, fmt_sdk_err(&e))))?;
        Ok(())
    })
}

pub fn s3_copy_object(
    client: &Client,
    src_bucket: &str,
    src_key: &str,
    dst_bucket: &str,
    dst_key: &str,
) -> Result<(), CfmlError> {
    block_on(async {
        let source = format!("{}/{}", src_bucket, src_key);
        client
            .copy_object()
            .copy_source(&source)
            .bucket(dst_bucket)
            .key(dst_key)
            .send()
            .await
            .map_err(|e| err(format!("CopyObject {} -> {}/{} failed: {}", source, dst_bucket, dst_key, fmt_sdk_err(&e))))?;
        Ok(())
    })
}

#[derive(Debug, Clone)]
pub struct S3Object {
    pub key: String,
    pub size: i64,
    pub last_modified: String,
    pub etag: String,
    pub storage_class: String,
    pub is_directory: bool,
}

pub fn s3_list_objects(
    client: &Client,
    bucket: &str,
    prefix: &str,
    delimiter: Option<&str>,
    max_keys: Option<i32>,
) -> Result<Vec<S3Object>, CfmlError> {
    block_on(async {
        let mut req = client.list_objects_v2().bucket(bucket);
        if !prefix.is_empty() {
            req = req.prefix(prefix);
        }
        if let Some(d) = delimiter {
            req = req.delimiter(d);
        }
        if let Some(m) = max_keys {
            req = req.max_keys(m);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| err(format!("ListObjectsV2 {} failed: {}", bucket, fmt_sdk_err(&e))))?;

        let mut out = Vec::new();
        for cp in resp.common_prefixes() {
            if let Some(p) = cp.prefix() {
                out.push(S3Object {
                    key: p.to_string(),
                    size: 0,
                    last_modified: String::new(),
                    etag: String::new(),
                    storage_class: String::new(),
                    is_directory: true,
                });
            }
        }
        for obj in resp.contents() {
            out.push(S3Object {
                key: obj.key().unwrap_or("").to_string(),
                size: obj.size().unwrap_or(0),
                last_modified: obj
                    .last_modified()
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                etag: obj.e_tag().unwrap_or("").trim_matches('"').to_string(),
                storage_class: obj
                    .storage_class()
                    .map(|sc| sc.as_str().to_string())
                    .unwrap_or_default(),
                is_directory: false,
            });
        }
        Ok(out)
    })
}

#[derive(Debug, Clone)]
pub struct S3BucketInfo {
    pub name: String,
    pub creation_date: String,
}

pub fn s3_list_buckets(client: &Client) -> Result<Vec<S3BucketInfo>, CfmlError> {
    block_on(async {
        let resp = client
            .list_buckets()
            .send()
            .await
            .map_err(|e| err(format!("ListBuckets failed: {}", fmt_sdk_err(&e))))?;
        let mut out = Vec::new();
        for b in resp.buckets() {
            out.push(S3BucketInfo {
                name: b.name().unwrap_or("").to_string(),
                creation_date: b
                    .creation_date()
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
            });
        }
        Ok(out)
    })
}

pub fn s3_create_bucket(
    client: &Client,
    bucket: &str,
    region: &str,
    _acl: Option<&str>,
) -> Result<(), CfmlError> {
    block_on(async {
        let mut req = client.create_bucket().bucket(bucket);
        if region != "us-east-1" {
            use aws_sdk_s3::types::{BucketLocationConstraint, CreateBucketConfiguration};
            let cfg = CreateBucketConfiguration::builder()
                .location_constraint(BucketLocationConstraint::from(region))
                .build();
            req = req.create_bucket_configuration(cfg);
        }
        req.send()
            .await
            .map_err(|e| err(format!("CreateBucket {} failed: {}", bucket, fmt_sdk_err(&e))))?;
        Ok(())
    })
}

pub fn s3_delete_bucket(client: &Client, bucket: &str, force: bool) -> Result<(), CfmlError> {
    if force {
        // List + delete all objects first.
        let objects = s3_list_objects(client, bucket, "", None, None)?;
        for obj in &objects {
            s3_delete_object(client, bucket, &obj.key)?;
        }
    }
    block_on(async {
        client
            .delete_bucket()
            .bucket(bucket)
            .send()
            .await
            .map_err(|e| err(format!("DeleteBucket {} failed: {}", bucket, fmt_sdk_err(&e))))?;
        Ok(())
    })
}

pub fn s3_clear_bucket(client: &Client, bucket: &str) -> Result<(), CfmlError> {
    let objects = s3_list_objects(client, bucket, "", None, None)?;
    for obj in &objects {
        s3_delete_object(client, bucket, &obj.key)?;
    }
    Ok(())
}

pub fn s3_get_metadata(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<ValueMap, CfmlError> {
    block_on(async {
        let resp = client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| err(format!("HeadObject {}/{} failed: {}", bucket, key, fmt_sdk_err(&e))))?;

        let mut out = ValueMap::default();
        if let Some(ct) = resp.content_type() {
            out.insert("content_type".to_string(), CfmlValue::string(ct.to_string()));
        }
        out.insert(
            "content_length".to_string(),
            CfmlValue::Int(resp.content_length().unwrap_or(0)),
        );
        if let Some(et) = resp.e_tag() {
            out.insert(
                "etag".to_string(),
                CfmlValue::string(et.trim_matches('"').to_string()),
            );
        }
        if let Some(lm) = resp.last_modified() {
            out.insert(
                "last_modified".to_string(),
                CfmlValue::string(lm.to_string()),
            );
        }
        if let Some(sc) = resp.storage_class() {
            out.insert(
                "storage_class".to_string(),
                CfmlValue::string(sc.as_str().to_string()),
            );
        }
        if let Some(meta) = resp.metadata() {
            for (k, v) in meta.iter() {
                out.insert(format!("x-amz-meta-{}", k), CfmlValue::string(v.clone()));
            }
        }
        Ok(out)
    })
}

pub fn s3_generate_presigned_url(
    client: &Client,
    bucket: &str,
    key: &str,
    expires_secs: u64,
    method: &str,
) -> Result<String, CfmlError> {
    use aws_sdk_s3::presigning::PresigningConfig;
    let cfg = PresigningConfig::expires_in(std::time::Duration::from_secs(expires_secs))
        .map_err(|e| err(format!("invalid expires_in: {}", e)))?;
    let method_upper = method.to_uppercase();
    block_on(async {
        let presigned = match method_upper.as_str() {
            "PUT" => client
                .put_object()
                .bucket(bucket)
                .key(key)
                .presigned(cfg)
                .await
                .map_err(|e| err(format!("presign PUT failed: {}", fmt_sdk_err(&e))))?,
            "DELETE" => client
                .delete_object()
                .bucket(bucket)
                .key(key)
                .presigned(cfg)
                .await
                .map_err(|e| err(format!("presign DELETE failed: {}", fmt_sdk_err(&e))))?,
            _ => client
                .get_object()
                .bucket(bucket)
                .key(key)
                .presigned(cfg)
                .await
                .map_err(|e| err(format!("presign GET failed: {}", fmt_sdk_err(&e))))?,
        };
        Ok(presigned.uri().to_string())
    })
}

// ---------- CfmlValue helpers ----------

pub fn objects_to_query_struct(objects: Vec<S3Object>) -> CfmlValue {
    // Return as an array of structs (lighter weight than a real Query value).
    let mut arr = Vec::with_capacity(objects.len());
    for o in objects {
        let mut s = ValueMap::default();
        s.insert("key".to_string(), CfmlValue::string(o.key));
        s.insert("size".to_string(), CfmlValue::Int(o.size));
        s.insert(
            "lastModified".to_string(),
            CfmlValue::string(o.last_modified),
        );
        s.insert("eTag".to_string(), CfmlValue::string(o.etag));
        s.insert(
            "storageClass".to_string(),
            CfmlValue::string(o.storage_class),
        );
        s.insert("isDirectory".to_string(), CfmlValue::Bool(o.is_directory));
        arr.push(CfmlValue::strukt(s));
    }
    CfmlValue::array(arr)
}

pub fn buckets_to_array(buckets: Vec<S3BucketInfo>) -> CfmlValue {
    let mut arr = Vec::with_capacity(buckets.len());
    for b in buckets {
        let mut s = ValueMap::default();
        s.insert("bucketName".to_string(), CfmlValue::string(b.name));
        s.insert(
            "creationDate".to_string(),
            CfmlValue::string(b.creation_date),
        );
        arr.push(CfmlValue::strukt(s));
    }
    CfmlValue::array(arr)
}

// ---------- App config snapshot ----------

/// Snapshot of `this.s3` from Application.cfc, stored on the VM.
#[derive(Debug, Clone, Default)]
pub struct S3AppConfig {
    pub settings: ValueMap,
}

impl S3AppConfig {
    pub fn from_value(v: &CfmlValue) -> Option<Self> {
        match v {
            CfmlValue::Struct(s) => {
                let mut settings = ValueMap::default();
                for (k, v) in s.iter() {
                    settings.insert(k.clone(), v.clone());
                }
                Some(Self { settings })
            }
            _ => None,
        }
    }

    pub fn from_struct(s: &ValueMap) -> Self {
        Self { settings: s.clone() }
    }

    pub fn as_map(&self) -> &ValueMap {
        &self.settings
    }
}

/// Helper that turns an arg list (key, secret, host) of optional strings into
/// resolved config using the VM's app_s3_config when present.
pub fn resolve_config(
    inline_key: Option<&str>,
    inline_secret: Option<&str>,
    inline_host: Option<&str>,
    app: Option<&S3AppConfig>,
) -> Result<S3Config, CfmlError> {
    S3Config::resolve(
        inline_key,
        inline_secret,
        inline_host,
        app.map(|a| a.as_map()),
    )
}

/// Resolve a client + the config used to build it, so callers can apply the
/// configured `key_prefix`.
pub fn client_and_config(
    inline_key: Option<&str>,
    inline_secret: Option<&str>,
    inline_host: Option<&str>,
    app: Option<&S3AppConfig>,
) -> Result<(Client, S3Config), CfmlError> {
    // Callers that have an explicit app config (the s3:// VFS intercept) pass
    // it; the plain `s3*()` builtins cannot, so fall back to whatever the VM
    // published for the running application.
    let published = if app.is_none() {
        app_s3_settings().map(|settings| S3AppConfig { settings })
    } else {
        None
    };
    let effective = app.or(published.as_ref());
    let cfg = resolve_config(inline_key, inline_secret, inline_host, effective)?;
    let client = global_clients().get_or_create(&cfg);
    Ok((client, cfg))
}

/// Build a client from an S3Url + VM app config, applying inline URL creds.
pub fn client_for_url(
    clients: &S3Clients,
    url: &S3Url,
    app: Option<&S3AppConfig>,
) -> Result<Client, CfmlError> {
    let cfg = resolve_config(
        url.access_key.as_deref(),
        url.secret_key.as_deref(),
        url.host.as_deref(),
        app,
    )?;
    Ok(clients.get_or_create(&cfg))
}

/// Same as `client_for_url` but also returns the resolved config so the
/// caller can apply `key_prefix`. The prefix is suppressed when the URL
/// itself carried inline credentials or a custom host.
pub fn client_and_config_for_url(
    url: &S3Url,
    app: Option<&S3AppConfig>,
) -> Result<(Client, S3Config), CfmlError> {
    client_and_config(
        url.access_key.as_deref(),
        url.secret_key.as_deref(),
        url.host.as_deref(),
        app,
    )
}

pub fn arg_opt_string(args: &[CfmlValue], idx: usize) -> Option<String> {
    args.get(idx).and_then(|v| match v {
        CfmlValue::String(s) if !s.is_empty() => Some((**s).clone()),
        CfmlValue::Null => None,
        CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) => None,
        _ => None,
    })
}

pub fn arg_string(args: &[CfmlValue], idx: usize) -> Result<String, CfmlError> {
    args.get(idx)
        .and_then(|v| match v {
            CfmlValue::String(s) => Some((**s).clone()),
            _ => None,
        })
        .ok_or_else(|| err(format!("expected string argument at position {}", idx + 1)))
}

/// Result type returned by VM dispatcher.
pub fn ok_void() -> CfmlResult {
    Ok(CfmlValue::Null)
}

pub fn ok_string(s: String) -> CfmlResult {
    Ok(CfmlValue::string(s))
}

pub fn ok_bool(b: bool) -> CfmlResult {
    Ok(CfmlValue::Bool(b))
}

pub fn ok_binary(bytes: Vec<u8>) -> CfmlResult {
    Ok(CfmlValue::Binary(bytes))
}

/// Map a key extension to a content-type guess. Returns None when unknown.
pub fn guess_content_type(key: &str) -> Option<&'static str> {
    let ext = key.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        _ => return None,
    })
}
