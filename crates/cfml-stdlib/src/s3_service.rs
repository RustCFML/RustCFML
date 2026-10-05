//! The S3 operations behind the `org.pixl8.s3storageprovider.Service` shim
//! (`cfml-vm/src/s3storageprovider_shim.rs`) — one function per method of the
//! Java class that `preside-ext-s3-storage-provider` constructs.
//!
//! Semantics are the jar's (v2.1.1, AWS SDK for Java 2.20.96), pinned by running
//! the real jar on Lucee 7.1 against MinIO: see `s3shim.md`. The points that are
//! easy to get wrong:
//! - errors carry the SDK's exception class as their type and its message
//!   shape (`<S3 message> (Service: S3, Status Code: 404, Request ID: …)`);
//!   the extension matches on the S3 message text in `validate()`
//! - `list_objects` walks EVERY page, with no delimiter
//! - `get_to_file` refuses to overwrite an existing file
//! - `bucket_location` returns the raw location constraint: empty for
//!   `us-east-1`, `EU` for a legacy eu-west-1 bucket

use crate::s3::{block_on, global_clients, normalize_endpoint, S3Config};
use aws_sdk_s3::config::http::HttpResponse;
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::operation::RequestId;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{MetadataDirective, ObjectCannedAcl, StorageClass};
use aws_sdk_s3::Client;
use cfml_common::dynamic::{CfmlQuery, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType};
use std::io::Write;

const MODEL: &str = "software.amazon.awssdk.services.s3.model.";
const CLIENT_EXCEPTION: &str = "software.amazon.awssdk.core.exception.SdkClientException";

/// One `Service` instance's settings. The secret key lives here and only here —
/// never on the CFML-visible shim struct, which `writeDump` would show.
#[derive(Clone)]
pub struct ServiceConfig {
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    /// An explicit endpoint (path-style). `None` — what the shim passes — means
    /// the environment's, or the region's AWS endpoint.
    pub endpoint: Option<String>,
}

impl ServiceConfig {
    /// The client for these credentials. Static credentials only (the Java
    /// service uses `StaticCredentialsProvider`), never the app-level S3
    /// settings or the ambient chain. The endpoint honours `AWS_ENDPOINT_URL_S3`
    /// / `AWS_ENDPOINT_URL` so tests can point it at MinIO.
    fn client(&self) -> Client {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let endpoint_url = self
            .endpoint
            .clone()
            .or_else(|| env("AWS_ENDPOINT_URL_S3"))
            .or_else(|| env("AWS_ENDPOINT_URL"))
            .map(|e| normalize_endpoint(&e));
        let use_path_style = self.endpoint.is_some()
            || env("RUSTCFML_S3_PATH_STYLE")
                .or_else(|| env("LUCEE_S3_PATH_STYLE"))
                .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false);
        global_clients().get_or_create(&S3Config {
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
            region: self.region.clone(),
            endpoint_url,
            key_prefix: None,
            use_path_style,
        })
    }
}

fn error(class: &str, message: String) -> CfmlError {
    CfmlError::new(message, CfmlErrorType::Custom(class.to_string()))
}

/// An SDK failure as AWS SDK for Java reports it. `missing` names the model
/// exception a bare 404 maps to (HEAD responses carry no error code).
fn sdk_error<E>(e: SdkError<E, HttpResponse>, missing: Option<&str>) -> CfmlError
where
    E: ProvideErrorMetadata + std::error::Error + 'static,
{
    match &e {
        SdkError::ServiceError(se) => {
            let status = se.raw().status().as_u16();
            let code = e.code().unwrap_or("");
            let class = match code {
                "NoSuchKey" => "NoSuchKeyException",
                "NoSuchBucket" => "NoSuchBucketException",
                // A HEAD response has no body, so no error code: the Rust SDK
                // reports `NotFound`, the Java one maps it to the model's
                // not-found exception for the operation.
                "" | "NotFound" if status == 404 => missing.unwrap_or("S3Exception"),
                _ => "S3Exception",
            };
            let message = format!(
                "{} (Service: S3, Status Code: {}, Request ID: {})",
                e.message().unwrap_or("null"),
                status,
                e.request_id().unwrap_or("null"),
            );
            error(&format!("{}{}", MODEL, class), message)
        }
        _ => {
            let mut detail = e.to_string();
            let mut src = std::error::Error::source(&e);
            while let Some(s) = src {
                detail.push_str(": ");
                detail.push_str(&s.to_string());
                src = s.source();
            }
            error(CLIENT_EXCEPTION, format!("Unable to execute HTTP request: {}", detail))
        }
    }
}

fn local_datetime(dt: &aws_sdk_s3::primitives::DateTime) -> CfmlValue {
    let millis = dt.to_millis().unwrap_or(0);
    CfmlValue::DateTime(cfml_common::datetime::CfmlDate::from_epoch_millis(millis))
}

/// `checkS3Access()` — `listBuckets()` succeeded. Never throws.
pub fn check_s3_access(cfg: &ServiceConfig) -> bool {
    let client = cfg.client();
    block_on(async { client.list_buckets().send().await.is_ok() })
}

/// `checkBucketAccess()` — `headBucket()` succeeded. Never throws.
pub fn check_bucket_access(cfg: &ServiceConfig) -> bool {
    let client = cfg.client();
    block_on(async { client.head_bucket().bucket(&cfg.bucket).send().await.is_ok() })
}

/// `checkBucketRegion()` — the raw location constraint equals the region.
/// A `us-east-1` bucket's constraint is empty and a legacy eu-west-1 one is
/// `EU`, so both are `false` even when the region is right: the jar does the
/// same plain comparison.
pub fn check_bucket_region(cfg: &ServiceConfig) -> Result<bool, CfmlError> {
    let client = cfg.client();
    block_on(async {
        let resp = client
            .get_bucket_location()
            .bucket(&cfg.bucket)
            .send()
            .await
            .map_err(|e| sdk_error(e, Some("NoSuchBucketException")))?;
        let constraint = resp.location_constraint().map(|c| c.as_str()).unwrap_or("");
        Ok(constraint == cfg.region)
    })
}

/// Split a key the way `java.io.File` does: trailing and repeated `/` are
/// dropped, then the name is the last segment and the path is `/` + the rest.
fn name_and_path(key: &str) -> (String, String) {
    let lead = key.starts_with('/');
    let segments: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
    let name = segments.last().copied().unwrap_or("").to_string();
    let parent = if segments.len() > 1 { segments[..segments.len() - 1].join("/") } else { String::new() };
    // `new File("/x").getParent()` is `/`, so a leading `/` doubles up; the
    // bare root `/` has no parent at all.
    let path = match (lead && !segments.is_empty(), parent.is_empty()) {
        (true, true) => "//".to_string(),
        (true, false) => format!("//{}", parent),
        (false, true) => "/".to_string(),
        (false, false) => format!("/{}", parent),
    };
    (name, path)
}

/// `listObjects(prefix)` — every object under `prefix`, all pages, as a query
/// of `name`, `path`, `size` (double) and `lastmodified` (date).
pub fn list_objects(cfg: &ServiceConfig, prefix: &str) -> Result<CfmlValue, CfmlError> {
    let client = cfg.client();
    let q = CfmlQuery::new(vec![
        "name".to_string(),
        "path".to_string(),
        "size".to_string(),
        "lastmodified".to_string(),
    ]);
    block_on(async {
        let mut pages = client
            .list_objects_v2()
            .bucket(&cfg.bucket)
            .prefix(prefix)
            .into_paginator()
            .send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(|e| sdk_error(e, Some("NoSuchBucketException")))?;
            for obj in page.contents() {
                let (name, path) = name_and_path(obj.key().unwrap_or(""));
                q.add_row_positional(vec![
                    CfmlValue::string(name),
                    CfmlValue::string(path),
                    CfmlValue::Double(obj.size().unwrap_or(0) as f64),
                    obj.last_modified().map(local_datetime).unwrap_or(CfmlValue::Null),
                ]);
            }
        }
        Ok::<(), CfmlError>(())
    })?;
    Ok(CfmlValue::Query(q))
}

/// `getObject(key)` — the object's bytes.
pub fn get_bytes(cfg: &ServiceConfig, key: &str) -> Result<Vec<u8>, CfmlError> {
    let client = cfg.client();
    block_on(async {
        let resp = client
            .get_object()
            .bucket(&cfg.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| sdk_error(e, Some("NoSuchKeyException")))?;
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| error(CLIENT_EXCEPTION, format!("Unable to read response body: {}", e)))?;
        Ok(bytes.into_bytes().to_vec())
    })
}

/// `getObject(key, filePath)` — stream the object to a NEW file. Like the SDK's
/// `getObject(req, Path)`, an existing file is an error, not overwritten; the
/// request is made first, so a missing key reports as missing.
pub fn get_to_file(cfg: &ServiceConfig, key: &str, path: &str) -> Result<(), CfmlError> {
    let client = cfg.client();
    block_on(async {
        let mut resp = client
            .get_object()
            .bucket(&cfg.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| sdk_error(e, Some("NoSuchKeyException")))?;
        let unmarshall = |detail: String| {
            error(
                CLIENT_EXCEPTION,
                format!("Unable to unmarshall response (Failed to read response into file: {}). {}", path, detail),
            )
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| unmarshall(e.to_string()))?;
        while let Some(chunk) = resp.body.next().await {
            let chunk = chunk.map_err(|e| unmarshall(e.to_string()))?;
            file.write_all(&chunk).map_err(|e| unmarshall(e.to_string()))?;
        }
        Ok(())
    })
}

/// `getObjectInfo(key)` — `{ size, lastmodified }`. A missing key throws.
pub fn get_object_info(cfg: &ServiceConfig, key: &str) -> Result<CfmlValue, CfmlError> {
    let client = cfg.client();
    block_on(async {
        let resp = client
            .head_object()
            .bucket(&cfg.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| sdk_error(e, Some("NoSuchKeyException")))?;
        let mut m = ValueMap::default();
        m.insert("size".to_string(), CfmlValue::Int(resp.content_length().unwrap_or(0)));
        m.insert(
            "lastmodified".to_string(),
            resp.last_modified().map(local_datetime).unwrap_or(CfmlValue::Null),
        );
        Ok(CfmlValue::strukt(m))
    })
}

/// `deleteObject(key)`. Deleting a missing key succeeds (S3 semantics).
pub fn delete_object(cfg: &ServiceConfig, key: &str) -> Result<(), CfmlError> {
    let client = cfg.client();
    block_on(async {
        client
            .delete_object()
            .bucket(&cfg.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| sdk_error(e, None))?;
        Ok(())
    })
}

/// What `putObject` uploads.
pub enum PutBody {
    Bytes(Vec<u8>),
    File(String),
}

/// The access settings every put and move applies: `private` when private or
/// trashed, otherwise `public-read`; reduced redundancy when trashed.
pub fn access(is_private: bool, is_trashed: bool) -> (ObjectCannedAcl, StorageClass) {
    let acl = if is_private || is_trashed { ObjectCannedAcl::Private } else { ObjectCannedAcl::PublicRead };
    let class = if is_trashed { StorageClass::ReducedRedundancy } else { StorageClass::Standard };
    (acl, class)
}

/// `putObject(key, bytes | localFilePath, mimetype, disposition, isPrivate, isTrashed)`.
pub fn put_object(
    cfg: &ServiceConfig,
    key: &str,
    body: PutBody,
    mimetype: &str,
    disposition: &str,
    is_private: bool,
    is_trashed: bool,
) -> Result<(), CfmlError> {
    let client = cfg.client();
    let (acl, class) = access(is_private, is_trashed);
    block_on(async {
        let body = match body {
            PutBody::Bytes(b) => ByteStream::from(b),
            PutBody::File(p) => ByteStream::from_path(&p).await.map_err(|_| {
                error("java.io.UncheckedIOException", format!("java.nio.file.NoSuchFileException: {}", p))
            })?,
        };
        let mut req = client
            .put_object()
            .bucket(&cfg.bucket)
            .key(key)
            .body(body)
            .acl(acl)
            .storage_class(class);
        if !mimetype.is_empty() {
            req = req.content_type(mimetype);
        }
        if !disposition.is_empty() {
            req = req.content_disposition(disposition);
        }
        req.send().await.map_err(|e| sdk_error(e, None))?;
        Ok(())
    })
}

/// `x-amz-copy-source` is `bucket/key` with the key URL-encoded (its `/` kept).
/// The raw key broke on spaces, `+`, `%` and non-ASCII.
pub fn copy_source(bucket: &str, key: &str) -> String {
    let mut out = format!("{}/", bucket);
    for b in key.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// `moveObject(source, target, mimetype, disposition, isPrivate, isTrashed)` —
/// a copy that REPLACES the headers, ACL and storage class, then a delete of
/// the source. A missing source fails at the copy, so nothing is deleted.
pub fn move_object(
    cfg: &ServiceConfig,
    source: &str,
    target: &str,
    mimetype: &str,
    disposition: &str,
    is_private: bool,
    is_trashed: bool,
) -> Result<(), CfmlError> {
    let client = cfg.client();
    let (acl, class) = access(is_private, is_trashed);
    block_on(async {
        let mut req = client
            .copy_object()
            .copy_source(copy_source(&cfg.bucket, source))
            .bucket(&cfg.bucket)
            .key(target)
            .metadata_directive(MetadataDirective::Replace)
            .acl(acl)
            .storage_class(class);
        if !mimetype.is_empty() {
            req = req.content_type(mimetype);
        }
        if !disposition.is_empty() {
            req = req.content_disposition(disposition);
        }
        req.send().await.map_err(|e| sdk_error(e, Some("NoSuchKeyException")))?;
        Ok::<(), CfmlError>(())
    })?;
    delete_object(cfg, source)
}

/// `getPresignedUrl(key, minutes)` — a presigned GET. SigV4 caps a presigned
/// URL at seven days; beyond that the SDK throws.
pub fn presigned_url(cfg: &ServiceConfig, key: &str, minutes: i64) -> Result<String, CfmlError> {
    use aws_sdk_s3::presigning::PresigningConfig;
    const WEEK_MINUTES: i64 = 7 * 24 * 60;
    if !(0..=WEEK_MINUTES).contains(&minutes) {
        return Err(error(
            CLIENT_EXCEPTION,
            "Requests that are pre-signed by SigV4 algorithm are valid for at most 7 days.".to_string(),
        ));
    }
    let presign = PresigningConfig::expires_in(std::time::Duration::from_secs(minutes as u64 * 60))
        .map_err(|e| error(CLIENT_EXCEPTION, e.to_string()))?;
    let client = cfg.client();
    block_on(async {
        let req = client
            .get_object()
            .bucket(&cfg.bucket)
            .key(key)
            .presigned(presign)
            .await
            .map_err(|e| sdk_error(e, None))?;
        Ok(req.uri().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths_follow_java_io_file() {
        // Pinned against the real jar on Lucee 7.1 (see s3shim.md).
        assert_eq!(name_and_path("site/public/a/b.jpg"), ("b.jpg".into(), "/site/public/a".into()));
        assert_eq!(name_and_path("b.jpg"), ("b.jpg".into(), "/".into()));
        assert_eq!(name_and_path("p/dir/"), ("dir".into(), "/p".into()));
        assert_eq!(name_and_path("p/sp ace+plus%pct/ünï.txt"), ("ünï.txt".into(), "/p/sp ace+plus%pct".into()));
        // java.io.File collapses repeated separators; a leading `/` is kept.
        assert_eq!(name_and_path("a//b.txt"), ("b.txt".into(), "/a".into()));
        assert_eq!(name_and_path("/lead.txt"), ("lead.txt".into(), "//".into()));
        assert_eq!(name_and_path("/a/b.txt"), ("b.txt".into(), "//a".into()));
    }

    #[test]
    fn copy_source_encodes_the_key_but_keeps_slashes() {
        assert_eq!(copy_source("bkt", "a/b c+d%e/ü.txt"), "bkt/a/b%20c%2Bd%25e/%C3%BC.txt");
    }

    /// A one-shot-per-connection HTTP server that records each request's
    /// method, path and headers, so the test can assert on what went over the
    /// wire. MinIO can't show this: it does not enforce or report per-object
    /// canned ACLs.
    fn capture_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<(String, String, Vec<(String, String)>)>>>) {
        use std::io::{BufRead, BufReader, Read};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut headers = Vec::new();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = h.trim_end().split_once(':') {
                        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
                        if k == "content-length" {
                            len = v.parse().unwrap_or(0);
                        }
                        headers.push((k, v));
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let is_copy = headers.iter().any(|(k, _)| k == "x-amz-copy-source");
                // The Rust SDK appends `?x-id=<Operation>`; keep the path only.
                let path = path.split('?').next().unwrap_or("").to_string();
                log.lock().unwrap().push((method.clone(), path, headers));
                let reply = if method == "DELETE" {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_string()
                } else if is_copy {
                    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyObjectResult><ETag>\"e\"</ETag></CopyObjectResult>";
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml)
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                };
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (addr, seen)
    }

    fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
        h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn put_and_move_send_the_access_headers() {
        let (endpoint, seen) = capture_server();
        let cfg = ServiceConfig {
            region: "eu-west-2".into(),
            bucket: "bkt".into(),
            access_key: "ak".into(),
            secret_key: "sk".into(),
            endpoint: Some(endpoint),
        };
        put_object(&cfg, "pub/a.jpg", PutBody::Bytes(b"x".to_vec()), "image/jpeg", "inline", false, false).unwrap();
        put_object(&cfg, "priv/b.pdf", PutBody::Bytes(b"y".to_vec()), "application/pdf", "attachment", true, false).unwrap();
        move_object(&cfg, "pub/a b+ü.jpg", ".trash/a.jpg", "image/jpeg", "attachment", false, true).unwrap();

        let seen = seen.lock().unwrap();
        let (m, path, h) = &seen[0];
        assert_eq!((m.as_str(), path.as_str()), ("PUT", "/bkt/pub/a.jpg"));
        assert_eq!(header(h, "x-amz-acl"), Some("public-read"));
        assert_eq!(header(h, "x-amz-storage-class"), Some("STANDARD"));
        assert_eq!(header(h, "content-type"), Some("image/jpeg"));
        assert_eq!(header(h, "content-disposition"), Some("inline"));

        let h = &seen[1].2;
        assert_eq!(header(h, "x-amz-acl"), Some("private"));
        assert_eq!(header(h, "x-amz-storage-class"), Some("STANDARD"));

        // The move: a copy that REPLACES metadata, then the delete.
        let (m, path, h) = &seen[2];
        assert_eq!((m.as_str(), path.as_str()), ("PUT", "/bkt/.trash/a.jpg"));
        assert_eq!(header(h, "x-amz-copy-source"), Some("bkt/pub/a%20b%2B%C3%BC.jpg"));
        assert_eq!(header(h, "x-amz-metadata-directive"), Some("REPLACE"));
        assert_eq!(header(h, "x-amz-acl"), Some("private"));
        assert_eq!(header(h, "x-amz-storage-class"), Some("REDUCED_REDUNDANCY"));
        assert_eq!(header(h, "content-disposition"), Some("attachment"));
        let (m, path, _) = &seen[3];
        assert_eq!(m, "DELETE");
        assert_eq!(path, "/bkt/pub/a%20b%2B%C3%BC.jpg");
    }

    #[test]
    fn private_or_trashed_objects_are_private() {
        assert_eq!(access(false, false), (ObjectCannedAcl::PublicRead, StorageClass::Standard));
        assert_eq!(access(true, false), (ObjectCannedAcl::Private, StorageClass::Standard));
        assert_eq!(access(false, true), (ObjectCannedAcl::Private, StorageClass::ReducedRedundancy));
        assert_eq!(access(true, true), (ObjectCannedAcl::Private, StorageClass::ReducedRedundancy));
    }
}
