//! Worker → cfml-vm::web bridge.
//!
//! Pulls headers, body, query string, etc. from `worker::Request` and feeds
//! them into the shared `cfml_vm::web::build_web_scopes` helper. Returns the
//! globals + http_request_data pair the VM expects.

#![cfg(target_arch = "wasm32")]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use worker::{Method, Request};

/// Extracts request data and builds the four standard CFML scopes.
///
/// Reads the body via `req.bytes().await`; the caller must hand over a fresh
/// (un-consumed) `worker::Request`.
pub async fn build_from_request(
    req: &mut Request,
    script_name: &str,
    path_info: &str,
) -> worker::Result<(ValueMap, CfmlValue)> {
    let method = req.method().to_string().to_uppercase();
    let url = req.url()?;
    let query_string = url.query().unwrap_or("").to_string();

    let headers_vec: Vec<(String, String)> = req
        .headers()
        .entries()
        .collect();

    let body_bytes: Vec<u8> = if matches!(req.method(), Method::Post | Method::Put | Method::Patch) {
        req.bytes().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let host = url.host_str().unwrap_or("").to_string();
    let port: u16 = url.port().unwrap_or(if url.scheme() == "https" { 443 } else { 80 });

    let remote_addr = req
        .headers()
        .get("cf-connecting-ip")
        .ok()
        .flatten()
        .unwrap_or_else(|| "0.0.0.0".to_string());

    // build_web_scopes uses the Host header (if present) for server_name —
    // ensure it's there since worker::Request strips it from .headers() in
    // some runtime versions.
    let mut headers = headers_vec;
    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("host")) && !host.is_empty() {
        headers.push(("host".to_string(), host));
    }

    Ok(cfml_vm::web::build_web_scopes(
        &method,
        &headers,
        // The worker has no filesystem to stream file parts to, so a multipart
        // body stays buffered here (see `RequestBody`).
        cfml_vm::web::RequestBody::Buffered(&body_bytes),
        script_name,
        path_info,
        &query_string,
        port,
        &remote_addr,
    ))
}
