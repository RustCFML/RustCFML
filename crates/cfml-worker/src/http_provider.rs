//! `<cfhttp>` on Cloudflare Workers, over the platform's `fetch`.
//!
//! The engine's native HTTP client is ureq, which pulls `ring` for TLS and has
//! no `wasm32-unknown-unknown` target, so `cfml-stdlib` is built here without
//! its `http` feature and never registers a `cfhttp` builtin. Without one the
//! VM's cfhttp intercept fails with "HTTP support is not enabled in this
//! build", which is what made a Worker unable to call an API.
//!
//! This registers a replacement that speaks the same contract as the native
//! one — same attributes in, same result struct out — and routes the request
//! through the JSPI `fetch` bridge in `jspi.rs`. The result keys are matched
//! deliberately: CFML in the wild reads `cfhttp.fileContent`,
//! `cfhttp.statusCode` and `cfhttp.responseHeader.status_code`, and anything
//! that differs between the native binary and the Worker is a portability bug.
//!
//! Not supported here, because a Worker has no filesystem: `<cfhttpparam
//! type="file">` and multipart file uploads. Everything else — headers,
//! cookies, URL params, form fields, an explicit body, XML, timeouts,
//! `throwOnError`, `getAsBinary`, `redirect` — behaves as it does natively.

//! Only the transport is wasm-specific. The attribute parsing, body building,
//! base64 and result shaping below are ordinary Rust and are unit-tested on the
//! host — `cargo test -p cfml-worker` covers them without a Worker.

// On the host, only the tests call these helpers — the wasm entry point is what
// uses them in anger — so the host build would otherwise warn dead_code on
// every one of them.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

#[cfg(target_arch = "wasm32")]
use crate::jspi::http_fetch_sync;

/// Reason phrases for the codes a Worker's `fetch` most often leaves blank.
/// `Response.statusText` is frequently empty on the Workers runtime, while the
/// native client always has one from the wire, and `statusCode` is documented
/// as "<code> <text>" — an empty half would change what CFML sees.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Split a `Content-Type` into its mime and charset halves, defaulting the
/// charset to UTF-8. Mirrors the stdlib's private `parse_content_type`.
fn parse_content_type(ct: &str) -> (String, String) {
    let mut parts = ct.splitn(2, ';');
    let mime = parts.next().unwrap_or("").trim().to_string();
    let charset = parts
        .next()
        .and_then(|p| p.split('=').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "UTF-8".to_string());
    (mime, charset)
}

/// Expand a top-level `attributeCollection` into the options struct, with an
/// explicitly-supplied attribute winning. Same semantics as the stdlib's
/// private `merge_cfhttp_attribute_collection`.
fn merge_attribute_collection(arg: CfmlValue) -> CfmlValue {
    let CfmlValue::Struct(opts) = &arg else {
        return arg;
    };
    let Some(CfmlValue::Struct(ac)) = opts.get_ci("attributeCollection") else {
        return arg;
    };
    let mut merged: ValueMap = ac.snapshot();
    for (k, v) in opts.iter() {
        if k.eq_ignore_ascii_case("attributeCollection") {
            continue;
        }
        merged.insert(k.as_str().to_string(), v.clone());
    }
    CfmlValue::strukt(merged)
}

/// Decode standard base64 (with or without padding). Hand-rolled to avoid
/// adding a dependency for the `getAsBinary` path alone.
fn base64_decode(input: &str) -> Result<Vec<u8>, &'static str> {
    fn sextet(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out: Vec<u8> = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = sextet(c).ok_or("invalid base64 character")? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Encode to standard padded base64. Needed for the request side too: a
/// multipart upload carrying binary parts cannot travel as a JSON string.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn attr<'a>(opts: &'a ValueMap, key: &str) -> Option<&'a CfmlValue> {
    opts.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn attr_string(opts: &ValueMap, key: &str) -> Option<String> {
    attr(opts, key).map(|v| v.as_string())
}

fn attr_bool(opts: &ValueMap, key: &str, default: bool) -> bool {
    match attr(opts, key) {
        Some(CfmlValue::Bool(b)) => *b,
        Some(CfmlValue::String(s)) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
        Some(CfmlValue::Int(i)) => *i != 0,
        Some(CfmlValue::Double(d)) => *d != 0.0,
        _ => default,
    }
}

/// One `<cfhttpparam type="file">` part. `content` rather than a path: there is
/// no filesystem here to read from.
#[derive(Debug, Clone)]
struct FilePart {
    name: String,
    filename: String,
    mime: String,
    content: Vec<u8>,
}

/// A boundary unlikely to collide with body bytes. Same approach as the native
/// client: a fixed prefix plus two varying values, no `rand` dependency.
fn multipart_boundary(url: &str, parts: usize) -> String {
    format!(
        "----RustCFMLWorkerBoundary{:016x}{:04x}",
        url.len() as u64 * 0x9E3779B9,
        parts & 0xffff
    )
}

/// Assemble a `multipart/form-data` body. CRLF line endings throughout, which
/// the spec requires and some parsers enforce.
fn build_multipart(
    boundary: &str,
    fields: &[(String, String)],
    files: &[FilePart],
) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    for (name, value) in fields {
        buf.extend_from_slice(b"--");
        buf.extend_from_slice(boundary.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
        );
        buf.extend_from_slice(value.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    for f in files {
        buf.extend_from_slice(b"--");
        buf.extend_from_slice(boundary.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                f.name, f.filename
            )
            .as_bytes(),
        );
        buf.extend_from_slice(format!("Content-Type: {}\r\n\r\n", f.mime).as_bytes());
        buf.extend_from_slice(&f.content);
        buf.extend_from_slice(b"\r\n");
    }
    buf.extend_from_slice(b"--");
    buf.extend_from_slice(boundary.as_bytes());
    buf.extend_from_slice(b"--\r\n");
    buf
}

/// The parsed request, ready to hand to the bridge.
#[derive(Debug)]
struct Request {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    /// Bytes, not a String: a multipart upload can carry binary parts. The
    /// bridge decides at serialisation time whether it travels as JSON text or
    /// as base64.
    body: Option<Vec<u8>>,
    timeout_ms: u64,
    follow_redirects: bool,
    get_as_binary: bool,
    throw_on_error: bool,
}

fn parse_request(arg: &CfmlValue) -> Result<Request, CfmlError> {
    // `cfhttp("https://…")` — a bare URL, everything else defaulted.
    let opts = match arg {
        CfmlValue::String(url) => {
            return Ok(Request {
                url: (**url).clone(),
                method: "GET".to_string(),
                headers: Vec::new(),
                body: None,
                timeout_ms: 30_000,
                follow_redirects: true,
                get_as_binary: false,
                throw_on_error: false,
            })
        }
        CfmlValue::Struct(opts) => opts,
        _ => {
            return Err(CfmlError::runtime(
                "cfhttp: expected a URL string or an attributes struct".to_string(),
            ))
        }
    };
    let opts: ValueMap = opts.snapshot();

    let mut url = attr_string(&opts, "url").unwrap_or_default();
    if url.trim().is_empty() {
        return Err(CfmlError::new(
            "cfhttp: attribute [url] is required".to_string(),
            CfmlErrorType::Application,
        ));
    }

    let method = attr_string(&opts, "method")
        .map(|m| m.to_uppercase())
        .unwrap_or_else(|| "GET".to_string());

    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(CfmlValue::Struct(h)) = attr(&opts, "headers") {
        for (k, v) in h.iter() {
            headers.push((k.as_str().to_string(), v.as_string()));
        }
    }

    // `<cfhttpparam>` children arrive as a `params` array. `file` is absent by
    // design: there is no filesystem to read from on a Worker.
    let mut form_fields: Vec<(String, String)> = Vec::new();
    let mut file_parts: Vec<FilePart> = Vec::new();
    let mut param_body: Option<String> = None;
    let mut cookies: Vec<String> = Vec::new();
    if let Some(CfmlValue::Array(params)) = attr(&opts, "params") {
        for param in params.iter() {
            let CfmlValue::Struct(p) = param else { continue };
            let p: ValueMap = p.snapshot();
            let ptype = attr_string(&p, "type").unwrap_or_default().to_lowercase();
            let pname = attr_string(&p, "name").unwrap_or_default();
            let pvalue = attr_string(&p, "value").unwrap_or_default();
            match ptype.as_str() {
                "header" => headers.push((pname, pvalue)),
                "cookie" => cookies.push(format!("{}={}", pname, pvalue)),
                "url" => {
                    let sep = if url.contains('?') { "&" } else { "?" };
                    url = format!(
                        "{}{}{}={}",
                        url,
                        sep,
                        urlencode_query(&pname),
                        urlencode_query(&pvalue)
                    );
                }
                "formfield" => form_fields.push((pname, pvalue)),
                "body" | "xml" => {
                    param_body = Some(pvalue);
                    if ptype == "xml"
                        && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                    {
                        headers.push(("Content-Type".to_string(), "text/xml".to_string()));
                    }
                }
                "file" => {
                    // On a binary, `file=` is a path to read. A Worker has no
                    // filesystem, so the content comes from `value=` instead
                    // and `file=` supplies only the filename to present. A
                    // path with no value cannot be honoured, and saying so is
                    // better than uploading nothing.
                    let content = match attr(&p, "value") {
                        Some(CfmlValue::Binary(b)) => b.clone(),
                        Some(CfmlValue::Null) | None => {
                            return Err(CfmlError::new(
                                "cfhttp: <cfhttpparam type=\"file\"> needs a value= holding \
                                 the file content — a Worker has no filesystem, so file= \
                                 names the upload rather than pointing at a path"
                                    .to_string(),
                                CfmlErrorType::Application,
                            ))
                        }
                        Some(other) => other.as_string().into_bytes(),
                    };
                    let filename = attr_string(&p, "file")
                        .filter(|f| !f.trim().is_empty())
                        .map(|f| {
                            // Accept a path and present just its leaf, so CFML
                            // written for a binary still names the part sanely.
                            f.rsplit(['/', '\\'])
                                .next()
                                .unwrap_or(&f)
                                .to_string()
                        })
                        .unwrap_or_else(|| pname.clone());
                    file_parts.push(FilePart {
                        name: pname,
                        filename,
                        mime: attr_string(&p, "mimetype")
                            .filter(|m| !m.trim().is_empty())
                            .unwrap_or_else(|| "application/octet-stream".to_string()),
                        content,
                    });
                }
                _ => {}
            }
        }
    }
    if !cookies.is_empty() {
        headers.push(("Cookie".to_string(), cookies.join("; ")));
    }

    // An explicit body= wins over params, matching the native ordering.
    let explicit_body = attr(&opts, "body")
        .filter(|v| !matches!(v, CfmlValue::Null))
        .map(|v| v.as_string());

    // A file part forces multipart, as it must — you cannot send an upload as
    // urlencoded — and `multipart="true"` opts in without one.
    let use_multipart = attr_bool(&opts, "multipart", false) || !file_parts.is_empty();

    let body: Option<Vec<u8>> = if let Some(b) = explicit_body {
        Some(b.into_bytes())
    } else if let Some(b) = param_body {
        Some(b.into_bytes())
    } else if use_multipart {
        let boundary = multipart_boundary(&url, form_fields.len() + file_parts.len());
        // Replace rather than append: a caller-supplied multipart content type
        // would carry the wrong boundary and the far end would parse nothing.
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Type"));
        headers.push((
            "Content-Type".to_string(),
            format!("multipart/form-data; boundary={}", boundary),
        ));
        Some(build_multipart(&boundary, &form_fields, &file_parts))
    } else if !form_fields.is_empty() {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Content-Type")) {
            headers.push((
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            ));
        }
        Some(
            form_fields
                .iter()
                .map(|(k, v)| format!("{}={}", urlencode_form(k), urlencode_form(v)))
                .collect::<Vec<_>>()
                .join("&")
                .into_bytes(),
        )
    } else {
        None
    };

    // `timeout` is seconds in CFML; the bridge wants milliseconds.
    let timeout_ms = attr(&opts, "timeout")
        .map(|v| v.as_string())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u64)
        .filter(|ms| *ms > 0)
        .unwrap_or(30_000);

    Ok(Request {
        url,
        method,
        headers,
        body,
        timeout_ms,
        // `redirect="false"` and `followRedirects="no"` both appear in the wild.
        follow_redirects: attr_bool(&opts, "redirect", true)
            && attr_bool(&opts, "followRedirects", true),
        get_as_binary: attr_bool(&opts, "getAsBinary", false),
        throw_on_error: attr_bool(&opts, "throwOnError", false),
    })
}

/// Percent-encode a component. `space_as_plus` picks the convention: a form
/// body uses `+`, a query string uses `%20`.
///
/// The distinction matters for parity, not just neatness. Verified against the
/// native binary calling the deployed worker: a `type="url"` param with a space
/// arrives as `hello=from%20the%20edge`, so emitting `+` here would make the
/// same CFML produce a different request URL depending on where it ran.
fn urlencode_with(s: &str, space_as_plus: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' if space_as_plus => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Query-string component: a space is `%20`.
fn urlencode_query(s: &str) -> String {
    urlencode_with(s, false)
}

/// Form-body component: a space is `+`.
fn urlencode_form(s: &str) -> String {
    urlencode_with(s, true)
}

/// A result struct for a request that never reached the wire. Mirrors the
/// native transport-error branch: status 0, the reason in `errorDetail`.
fn transport_failure(detail: &str, throw_on_error: bool) -> CfmlResult {
    if throw_on_error {
        return Err(CfmlError::new(
            format!("cfhttp connection failed: {}", detail),
            CfmlErrorType::Application,
        ));
    }
    let mut r: ValueMap = ValueMap::default();
    r.insert("statusCode".to_string(), CfmlValue::string("0".to_string()));
    r.insert("status_code".to_string(), CfmlValue::Int(0));
    r.insert("statusText".to_string(), CfmlValue::string(String::new()));
    r.insert("status_text".to_string(), CfmlValue::string(String::new()));
    r.insert("fileContent".to_string(), CfmlValue::string(String::new()));
    r.insert("mimeType".to_string(), CfmlValue::string(String::new()));
    r.insert("charset".to_string(), CfmlValue::string("UTF-8".to_string()));
    r.insert(
        "responseHeader".to_string(),
        CfmlValue::strukt(ValueMap::default()),
    );
    r.insert("errorDetail".to_string(), CfmlValue::string(detail.to_string()));
    r.insert("HTTP_Version".to_string(), CfmlValue::string(String::new()));
    Ok(CfmlValue::strukt(r))
}

/// The `cfhttp` builtin the VM's intercept looks up by name. Signature is
/// fixed by the VM's `BuiltinFunction`: a plain fn, no captured state, which is
/// why the JSPI bridge is reached through a free function.
#[cfg(target_arch = "wasm32")]
pub fn cfhttp_worker(args: Vec<CfmlValue>) -> CfmlResult {
    let arg = merge_attribute_collection(args.into_iter().next().unwrap_or(CfmlValue::Null));
    let req = parse_request(&arg)?;

    let mut header_json = serde_json::Map::new();
    for (k, v) in &req.headers {
        header_json.insert(k.clone(), serde_json::Value::String(v.clone()));
    }

    // The bridge is JSON, so a body that is not valid UTF-8 — a multipart
    // upload with binary parts — has to travel base64-encoded instead.
    let (body_text, body_b64) = match &req.body {
        None => (None, None),
        Some(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => (Some(text.to_string()), None),
            Err(_) => (None, Some(base64_encode(bytes))),
        },
    };

    let payload = serde_json::json!({
        "url": req.url,
        "method": req.method,
        "headers": serde_json::Value::Object(header_json),
        "body": body_text,
        "bodyBase64": body_b64,
        "timeoutMs": req.timeout_ms,
        "followRedirects": req.follow_redirects,
        "getAsBinary": req.get_as_binary,
    });

    let raw = http_fetch_sync(&payload.to_string())?;
    let resp: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        CfmlError::runtime(format!("cfhttp: unreadable response from the fetch bridge: {}", e))
    })?;

    if !resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let detail = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown transport error");
        return transport_failure(detail, req.throw_on_error);
    }

    let status = resp.get("status").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let mut status_text = resp
        .get("statusText")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if status_text.is_empty() {
        status_text = reason_phrase(status).to_string();
    }

    let mut resp_headers: ValueMap = ValueMap::default();
    let mut content_type = String::new();
    if let Some(map) = resp.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in map {
            let value = v.as_str().unwrap_or("").to_string();
            if k.eq_ignore_ascii_case("content-type") {
                content_type = value.clone();
            }
            resp_headers.insert(k.clone(), CfmlValue::string(value));
        }
    }
    // ACF/Lucee put the status inside responseHeader as well; plenty of CFML
    // checks `responseHeader.status_code` rather than the top-level key.
    resp_headers.insert("status_code".to_string(), CfmlValue::Int(status as i64));
    resp_headers.insert(
        "explanation".to_string(),
        CfmlValue::string(status_text.clone()),
    );

    if req.throw_on_error && status >= 400 {
        // Lucee raises an `application`-typed error whose message is just
        // "<code> <text>", so `catch( application e )` behaves the same here.
        return Err(CfmlError::new(
            format!("{} {}", status, status_text),
            CfmlErrorType::Application,
        ));
    }

    let file_content = if req.get_as_binary {
        let b64 = resp.get("bodyBase64").and_then(|v| v.as_str()).unwrap_or("");
        match base64_decode(b64) {
            Ok(bytes) => CfmlValue::Binary(bytes),
            Err(e) => {
                return Err(CfmlError::runtime(format!(
                    "cfhttp: getAsBinary response could not be decoded: {}",
                    e
                )))
            }
        }
    } else {
        CfmlValue::string(
            resp.get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        )
    };

    let (mime, charset) = parse_content_type(&content_type);

    let mut r: ValueMap = ValueMap::default();
    r.insert(
        "statusCode".to_string(),
        CfmlValue::string(format!("{} {}", status, status_text)),
    );
    r.insert("status_code".to_string(), CfmlValue::Int(status as i64));
    r.insert(
        "statusText".to_string(),
        CfmlValue::string(status_text.clone()),
    );
    r.insert("status_text".to_string(), CfmlValue::string(status_text));
    r.insert("fileContent".to_string(), file_content);
    r.insert("mimeType".to_string(), CfmlValue::string(mime));
    r.insert("charset".to_string(), CfmlValue::string(charset));
    r.insert("responseHeader".to_string(), CfmlValue::strukt(resp_headers));
    r.insert("errorDetail".to_string(), CfmlValue::string(String::new()));
    // Workers' fetch does not expose the negotiated protocol version. The
    // native client reports it from the wire; here it is HTTP/1.1 by
    // convention, which is what the vast majority of CFML compares against.
    r.insert(
        "HTTP_Version".to_string(),
        CfmlValue::string("HTTP/1.1".to_string()),
    );
    Ok(CfmlValue::strukt(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_known_vectors() {
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        // Unpadded input is accepted too: some producers omit the padding.
        assert_eq!(base64_decode("Zm9vYmE").unwrap(), b"fooba");
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn content_type_splits_into_mime_and_charset() {
        assert_eq!(
            parse_content_type("application/json; charset=utf-8"),
            ("application/json".to_string(), "utf-8".to_string())
        );
        assert_eq!(
            parse_content_type("text/html"),
            ("text/html".to_string(), "UTF-8".to_string())
        );
    }

    #[test]
    fn urlencode_escapes_what_it_must() {
        // Form bodies use `+` for a space; query strings use `%20`, which is
        // what the native client produces (verified against the live worker).
        assert_eq!(urlencode_form("a b&c=d"), "a+b%26c%3Dd");
        assert_eq!(urlencode_query("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode_query("safe-_.~"), "safe-_.~");
        assert_eq!(urlencode_form("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn blank_status_text_falls_back_to_a_reason_phrase() {
        assert_eq!(reason_phrase(404), "Not Found");
        assert_eq!(reason_phrase(200), "OK");
        assert_eq!(reason_phrase(599), "");
    }

    fn opts(pairs: &[(&str, CfmlValue)]) -> CfmlValue {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        CfmlValue::strukt(m)
    }

    fn param(pairs: &[(&str, &str)]) -> CfmlValue {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert((*k).to_string(), CfmlValue::string(*v));
        }
        CfmlValue::strukt(m)
    }

    fn body_of(req: &Request) -> Option<String> {
        req.body
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).to_string())
    }

    fn header_of(req: &Request, name: &str) -> Option<String> {
        req.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    #[test]
    fn a_bare_url_string_defaults_everything() {
        let req = parse_request(&CfmlValue::string("https://api.example.com/x")).unwrap();
        assert_eq!(req.url, "https://api.example.com/x");
        assert_eq!(req.method, "GET");
        assert_eq!(req.timeout_ms, 30_000);
        assert!(req.follow_redirects);
        assert!(!req.throw_on_error);
        assert!(req.body.is_none());
    }

    #[test]
    fn a_missing_url_is_an_application_error() {
        let err = parse_request(&opts(&[("method", CfmlValue::string("GET"))])).unwrap_err();
        assert!(err.message.contains("url"), "got: {}", err.message);
    }

    #[test]
    fn timeout_seconds_become_milliseconds() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("timeout", CfmlValue::string("5")),
        ]))
        .unwrap();
        assert_eq!(req.timeout_ms, 5_000);
    }

    #[test]
    fn header_and_cookie_params_become_headers() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            (
                "params",
                CfmlValue::array(vec![
                    param(&[("type", "header"), ("name", "Authorization"), ("value", "Bearer t")]),
                    param(&[("type", "cookie"), ("name", "a"), ("value", "1")]),
                    param(&[("type", "cookie"), ("name", "b"), ("value", "2")]),
                ]),
            ),
        ]))
        .unwrap();
        assert_eq!(header_of(&req, "Authorization").as_deref(), Some("Bearer t"));
        // Multiple cookies collapse into one header, as they must on the wire.
        assert_eq!(header_of(&req, "Cookie").as_deref(), Some("a=1; b=2"));
    }

    #[test]
    fn url_params_are_appended_and_encoded() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/search?a=1")),
            (
                "params",
                CfmlValue::array(vec![param(&[
                    ("type", "url"),
                    ("name", "q"),
                    ("value", "two words&more"),
                ])]),
            ),
        ]))
        .unwrap();
        assert_eq!(req.url, "https://x.test/search?a=1&q=two%20words%26more");
    }

    #[test]
    fn form_fields_build_an_urlencoded_body_with_its_content_type() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("method", CfmlValue::string("post")),
            (
                "params",
                CfmlValue::array(vec![
                    param(&[("type", "formfield"), ("name", "one"), ("value", "1")]),
                    param(&[("type", "formfield"), ("name", "two"), ("value", "a b")]),
                ]),
            ),
        ]))
        .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(body_of(&req).as_deref(), Some("one=1&two=a+b"));
        assert_eq!(
            header_of(&req, "Content-Type").as_deref(),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn an_explicit_body_wins_over_params() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("body", CfmlValue::string("{\"json\":true}")),
            (
                "params",
                CfmlValue::array(vec![param(&[
                    ("type", "formfield"),
                    ("name", "ignored"),
                    ("value", "yes"),
                ])]),
            ),
        ]))
        .unwrap();
        assert_eq!(body_of(&req).as_deref(), Some("{\"json\":true}"));
    }

    #[test]
    fn an_xml_param_sets_the_content_type() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            (
                "params",
                CfmlValue::array(vec![param(&[("type", "xml"), ("value", "<a/>")])]),
            ),
        ]))
        .unwrap();
        assert_eq!(body_of(&req).as_deref(), Some("<a/>"));
        assert_eq!(header_of(&req, "Content-Type").as_deref(), Some("text/xml"));
    }

    #[test]
    fn a_file_param_with_no_content_says_why() {
        let err = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            (
                "params",
                CfmlValue::array(vec![param(&[("type", "file"), ("name", "f"), ("file", "/x")])]),
            ),
        ]))
        .unwrap_err();
        assert!(err.message.contains("no filesystem"), "got: {}", err.message);
    }

    #[test]
    fn a_file_param_builds_a_multipart_body() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("method", CfmlValue::string("POST")),
            (
                "params",
                CfmlValue::array(vec![
                    param(&[("type", "formfield"), ("name", "note"), ("value", "hello")]),
                    param(&[
                        ("type", "file"),
                        ("name", "upload"),
                        ("file", "/tmp/report.csv"),
                        ("value", "a,b\n1,2"),
                        ("mimetype", "text/csv"),
                    ]),
                ]),
            ),
        ]))
        .unwrap();

        let ct = header_of(&req, "Content-Type").expect("a content type");
        assert!(ct.starts_with("multipart/form-data; boundary="), "got: {}", ct);
        let boundary = ct.split("boundary=").nth(1).unwrap().to_string();

        let body = body_of(&req).expect("a body");
        // The form field and the file both appear, the filename is the leaf of
        // the path, and the body is terminated with the closing boundary.
        assert!(body.contains(&format!("--{}\r\n", boundary)));
        assert!(body.contains("name=\"note\""));
        assert!(body.contains("hello"));
        assert!(body.contains("name=\"upload\"; filename=\"report.csv\""));
        assert!(body.contains("Content-Type: text/csv"));
        assert!(body.contains("a,b\n1,2"));
        assert!(body.ends_with(&format!("--{}--\r\n", boundary)));
    }

    #[test]
    fn multipart_true_works_without_any_file() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("multipart", CfmlValue::Bool(true)),
            (
                "params",
                CfmlValue::array(vec![param(&[
                    ("type", "formfield"),
                    ("name", "only"),
                    ("value", "field"),
                ])]),
            ),
        ]))
        .unwrap();
        let ct = header_of(&req, "Content-Type").unwrap();
        assert!(ct.starts_with("multipart/form-data; boundary="));
        assert!(body_of(&req).unwrap().contains("name=\"only\""));
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // Round-trips with the decoder, including bytes that are not UTF-8.
        let raw: Vec<u8> = vec![0x00, 0xff, 0x10, 0x80, 0x7f];
        assert_eq!(base64_decode(&base64_encode(&raw)).unwrap(), raw);
    }

    #[test]
    fn attribute_collection_is_merged_with_explicit_attributes_winning() {
        let mut ac = ValueMap::default();
        ac.insert("url".to_string(), CfmlValue::string("https://from-collection/"));
        ac.insert("method".to_string(), CfmlValue::string("PUT"));
        let arg = opts(&[
            ("attributeCollection", CfmlValue::strukt(ac)),
            ("method", CfmlValue::string("DELETE")),
        ]);
        let req = parse_request(&merge_attribute_collection(arg)).unwrap();
        assert_eq!(req.url, "https://from-collection/");
        assert_eq!(req.method, "DELETE");
    }

    #[test]
    fn redirect_false_disables_following() {
        let req = parse_request(&opts(&[
            ("url", CfmlValue::string("https://x.test/")),
            ("redirect", CfmlValue::Bool(false)),
        ]))
        .unwrap();
        assert!(!req.follow_redirects);
    }

    #[test]
    fn a_transport_failure_reports_status_zero_and_the_detail() {
        let v = transport_failure("dns failure", false).unwrap();
        let CfmlValue::Struct(s) = v else { panic!("expected a struct") };
        assert_eq!(s.get_ci("statusCode").unwrap().as_string(), "0");
        assert_eq!(s.get_ci("errorDetail").unwrap().as_string(), "dns failure");
    }

    #[test]
    fn a_transport_failure_throws_when_asked_to() {
        let err = transport_failure("dns failure", true).unwrap_err();
        assert!(
            err.message.contains("cfhttp connection failed"),
            "got: {}",
            err.message
        );
    }
}
