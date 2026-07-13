//! HTTP → CFML scope helpers shared between the CLI serve mode and
//! cloudflare-worker hosts.
//!
//! Everything here is pure data shuffling on top of the `Vfs` trait — no
//! sockets, no Tokio, no platform syscalls beyond `std::fs::write` for
//! multipart temp files (and that one is `cfg(not(target_arch = "wasm32"))`).

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::Vfs;
use std::path::{Path, PathBuf};

/// Result of resolving a URL path to a file.
#[derive(Clone, Debug)]
pub struct ResolvedFile {
    pub file_path: PathBuf,
    /// The script portion of the URL (e.g. "/index.cfm")
    pub script_name: String,
    /// Extra path info after the script (e.g. "/hello/world")
    pub path_info: String,
}

/// Resolve a URL path to a file under `doc_root` using `vfs` for existence
/// checks.
///
/// `welcome_files` are the names tried when the URL resolves to a directory.
/// `cfml_extensions` are the file extensions treated as CFML for path-info
/// matching (e.g. `/foo.cfm/bar/baz`).
pub fn resolve_file(
    doc_root: &Path,
    url_path: &str,
    vfs: &dyn Vfs,
    welcome_files: &[String],
    cfml_extensions: &[String],
) -> Option<ResolvedFile> {
    let relative = url_path.trim_start_matches('/');

    let try_welcome_in = |dir: &Path, rel: &str| -> Option<ResolvedFile> {
        for welcome in welcome_files {
            let candidate = dir.join(welcome);
            if vfs.is_file(&candidate.to_string_lossy()) {
                let script = if rel.is_empty() {
                    format!("/{}", welcome)
                } else {
                    format!("/{}/{}", rel, welcome)
                };
                return Some(ResolvedFile {
                    file_path: candidate,
                    script_name: script,
                    path_info: String::new(),
                });
            }
        }
        None
    };

    if !relative.is_empty() {
        let candidate = doc_root.join(relative);
        if vfs.is_file(&candidate.to_string_lossy()) {
            return Some(ResolvedFile {
                file_path: candidate,
                script_name: format!("/{}", relative),
                path_info: String::new(),
            });
        }
        if let Some(rf) = try_welcome_in(&doc_root.join(relative), relative) {
            return Some(rf);
        }
        // Path-info pattern: walk up segments looking for a CFML file.
        let mut parts: Vec<&str> = relative.split('/').collect();
        while parts.len() > 1 {
            parts.pop();
            let partial = parts.join("/");
            let candidate = doc_root.join(&partial);
            if vfs.is_file(&candidate.to_string_lossy())
                && candidate
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| {
                        cfml_extensions
                            .iter()
                            .any(|ext| ext.eq_ignore_ascii_case(e))
                    })
            {
                let script_name = format!("/{}", partial);
                let path_info = url_path[script_name.len()..].to_string();
                return Some(ResolvedFile {
                    file_path: candidate,
                    script_name,
                    path_info,
                });
            }
        }
    } else if let Some(rf) = try_welcome_in(doc_root, "") {
        return Some(rf);
    }

    None
}

/// Build CGI, URL, Form, and Cookie scopes from extracted HTTP request data.
///
/// Returns `(globals, http_request_data)` — globals contains the four scopes
/// keyed by their lowercase names; `http_request_data` is the struct exposed
/// to CFML as `getHttpRequestData()`.
pub fn build_web_scopes(
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
    script_name: &str,
    path_info: &str,
    query_string: &str,
    port: u16,
    remote_addr: &str,
) -> (ValueMap, CfmlValue) {
    let mut globals = ValueMap::default();

    let mut cgi = ValueMap::default();
    cgi.insert("request_method".to_string(), CfmlValue::string(method.to_string()));
    // `cgi.path_info` is the extra path *after* the resolved script — empty when
    // the URL maps straight to a file with nothing trailing (`/index.cfm?x=1`),
    // matching Lucee/ACF. Do NOT coerce empty to "/": frameworks distinguish the
    // two (Taffy's `parseRequest` only falls back to its `?endpoint=` param when
    // `len(cgi.path_info) == 0`; Wheels' `$cgiScope` branches on `!Len(path_info)`).
    // Verified against Lucee 7.0.4: `/x.cfm?q=1` → "", `/x.cfm/a/b` → "/a/b".
    cgi.insert("path_info".to_string(), CfmlValue::string(path_info.to_string()));
    cgi.insert("script_name".to_string(), CfmlValue::string(script_name.to_string()));
    cgi.insert("query_string".to_string(), CfmlValue::string(query_string.to_string()));
    cgi.insert("server_port".to_string(), CfmlValue::string(port.to_string()));
    cgi.insert("remote_addr".to_string(), CfmlValue::string(remote_addr.to_string()));
    cgi.insert("remote_host".to_string(), CfmlValue::string(remote_addr.to_string()));

    let mut content_type = String::new();
    let mut server_name = "127.0.0.1".to_string();
    let mut host_header = String::new();
    // The CLI server is HTTP-only and sits behind a TLS-terminating proxy, so
    // the only honest "is this request secure" signal is `X-Forwarded-Proto`.
    let mut is_https = false;
    for (name, value) in headers {
        let lower = name.to_lowercase();
        if lower == "content-type" {
            content_type = value.clone();
            cgi.insert("content_type".to_string(), CfmlValue::string(value.clone()));
        }
        if lower == "host" {
            server_name = value.split(':').next().unwrap_or(value).to_string();
            host_header = value.clone();
        }
        if lower == "x-forwarded-proto" && value.eq_ignore_ascii_case("https") {
            is_https = true;
        }
        let cgi_key = format!("http_{}", lower.replace('-', "_"));
        cgi.insert(cgi_key, CfmlValue::string(value.clone()));
    }
    cgi.insert("server_name".to_string(), CfmlValue::string(server_name.clone()));
    // Mirror the secure-transport view into the standard CGI variable
    // (`on`/`off`), matching CFML convention. Previously absent entirely.
    cgi.insert(
        "https".to_string(),
        CfmlValue::string(if is_https { "on" } else { "off" }.to_string()),
    );

    // Standard CGI variables that frameworks (Preside, ColdBox, FW/1) rely on.
    // `server_protocol`/`server_port_secure` were absent entirely; `request_url`
    // (the full URL of the current request) is what Preside's Bootstrap._getUrl
    // falls back to, so its absence aborted app bootstrap with an "undefined"
    // error on every request.
    let scheme = if is_https { "https" } else { "http" };
    cgi.insert(
        "server_protocol".to_string(),
        CfmlValue::string("HTTP/1.1".to_string()),
    );
    cgi.insert(
        "server_port_secure".to_string(),
        CfmlValue::string(if is_https { "1" } else { "0" }.to_string()),
    );
    let authority = if !host_header.is_empty() {
        host_header.clone()
    } else if port == 80 || port == 443 {
        server_name.clone()
    } else {
        format!("{}:{}", server_name, port)
    };
    let request_url = format!("{}://{}{}", scheme, authority, script_name);
    cgi.insert("request_url".to_string(), CfmlValue::string(request_url));

    // Tag cgi as a magic scope: reading an unset key returns "" (Lucee/ACF
    // parity), not null. Wheels' $cgiScope copies optional keys like
    // `http_x_requested_with` out of cgi unconditionally; without "" defaults
    // the copy stored null, which the null-delete guard then dropped.
    cgi.insert(
        cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER.to_string(),
        CfmlValue::Bool(true),
    );

    globals.insert("cgi".to_string(), CfmlValue::strukt(cgi));

    let url_scope = parse_query_string(query_string);
    globals.insert("url".to_string(), CfmlValue::strukt(url_scope));

    let raw_body = String::from_utf8_lossy(body).to_string();

    // Populate the form scope from a form-encoded request body for ANY method
    // that carries one — PUT, PATCH and DELETE submit forms just like POST
    // (Lucee/ACF key off the content-type, not the method). GET and other
    // bodyless requests fall through to an empty form scope.
    let form_scope = if content_type.starts_with("application/x-www-form-urlencoded")
        && !raw_body.is_empty()
    {
        parse_query_string(&raw_body)
    } else if content_type.starts_with("multipart/form-data") {
        parse_multipart_sync(&content_type, body)
    } else {
        ValueMap::default()
    };
    globals.insert("form".to_string(), CfmlValue::strukt(form_scope));

    let cookie_scope = {
        let mut cookies = ValueMap::default();
        for (name, value) in headers {
            if name.to_lowercase() == "cookie" {
                for cookie in value.split(';') {
                    let cookie = cookie.trim();
                    if let Some(eq) = cookie.find('=') {
                        let cname = cookie[..eq].trim().to_string();
                        let cvalue = cookie[eq + 1..].trim().to_string();
                        cookies.insert(cname, CfmlValue::string(cvalue));
                    }
                }
            }
        }
        cookies
    };
    globals.insert("cookie".to_string(), CfmlValue::strukt(cookie_scope));

    let mut headers_struct = ValueMap::default();
    for (name, value) in headers {
        headers_struct.insert(name.clone(), CfmlValue::string(value.clone()));
    }

    let mut http_request_data = ValueMap::default();
    http_request_data.insert("headers".to_string(), CfmlValue::strukt(headers_struct));
    http_request_data.insert("content".to_string(), CfmlValue::string(raw_body));
    http_request_data.insert("method".to_string(), CfmlValue::string(method.to_string()));
    http_request_data.insert("protocol".to_string(), CfmlValue::string("HTTP/1.1".to_string()));

    (globals, CfmlValue::strukt(http_request_data))
}

/// Parse a query string like `name=World&id=1` into an ordered map.
pub fn parse_query_string(qs: &str) -> ValueMap {
    let mut map = ValueMap::default();
    if qs.is_empty() {
        return map;
    }
    for pair in qs.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let Some(key) = parts.next() {
            let value = parts.next().unwrap_or("");
            let key = url_decode(key);
            let value = url_decode(value);
            if !key.is_empty() {
                map.insert(key.to_lowercase(), CfmlValue::string(value));
            }
        }
    }
    map
}

/// HTTP "singleton" response headers: ones that must carry exactly one value
/// and therefore have to be *replaced*, not appended. `cfheader` can set any of
/// these; per HTTP semantics the last value wins. See issue #148.
pub const SINGLETON_RESPONSE_HEADERS: [&str; 3] =
    ["content-type", "content-length", "location"];

/// Resolve serve-mode response headers, enforcing singleton-header semantics.
///
/// CFML exposes the response content type through two channels — `cfcontent`
/// (→ `response_content_type`) and `cfheader(name="Content-Type")` (→ a row in
/// `response_headers`). Naively the host would emit the engine default *and*
/// the cfheader value, producing two `Content-Type` headers (issue #148).
///
/// This returns the single effective `Content-Type` (an explicit `cfheader`
/// wins over a `cfcontent` type, which wins over the engine default) plus the
/// remaining headers to emit. `Content-Type` is stripped from that list (the
/// caller applies it as the singleton); every other singleton header is
/// de-duplicated so only its last occurrence survives, while non-singleton
/// headers (e.g. `Set-Cookie`) keep their full, ordered multiplicity.
pub fn resolve_response_headers(
    response_content_type: Option<&str>,
    response_headers: &[(String, String)],
) -> (String, Vec<(String, String)>) {
    let explicit_ct = response_headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .last();
    let content_type = explicit_ct
        .or_else(|| response_content_type.map(str::to_string))
        .unwrap_or_else(|| "text/html; charset=utf-8".to_string());

    let mut out: Vec<(String, String)> = Vec::with_capacity(response_headers.len());
    for (name, value) in response_headers {
        let lname = name.to_lowercase();
        if lname == "content-type" {
            // Applied separately as the singleton above.
            continue;
        }
        if SINGLETON_RESPONSE_HEADERS.contains(&lname.as_str()) {
            // Drop any earlier value so the last one wins (singleton replace).
            out.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        }
        out.push((name.clone(), value.clone()));
    }
    (content_type, out)
}

/// Minimal URL decoder: `+` → space, `%XX` → byte.
pub fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            result.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                result.push(byte);
                i += 3;
            } else {
                result.push(bytes[i]);
                i += 1;
            }
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&result).to_string()
}

/// Parse a `multipart/form-data` body synchronously.
///
/// File uploads are written to the host's temp directory on native targets so
/// `cffile action="upload"` can move them later. On `wasm32-unknown-unknown`
/// there is no temp dir; we still expose the metadata + inline content but
/// `tempFilePath` is empty.
pub fn parse_multipart_sync(content_type: &str, body: &[u8]) -> ValueMap {
    let mut form = ValueMap::default();

    let boundary = content_type
        .split(';')
        .find_map(|part| {
            let trimmed = part.trim();
            if trimmed.to_lowercase().starts_with("boundary=") {
                Some(trimmed[9..].trim_matches('"').to_string())
            } else {
                None
            }
        });

    let boundary = match boundary {
        Some(b) => b,
        None => return form,
    };

    let delimiter = format!("--{}", boundary).into_bytes();

    // IMPORTANT: split on the RAW bytes, not on a lossy UTF-8 view of the body.
    // File uploads are binary (a PNG starts with 0x89, which is not valid
    // UTF-8), so `String::from_utf8_lossy` would replace every non-UTF-8 byte
    // with U+FFFD and corrupt the uploaded file before it reaches disk —
    // yielding "unrecognised image format" and the like downstream.
    for raw_part in split_on_bytes(body, &delimiter) {
        // Strip the single CRLF the delimiter left at the front and the single
        // trailing CRLF that precedes the next delimiter. Only ONE each — the
        // body may legitimately end in 0x0D 0x0A bytes.
        let part = strip_one_trailing_crlf(strip_one_leading_crlf(raw_part));
        // The closing delimiter leaves a "--" (optionally followed by CRLF) part.
        if part.is_empty() || part == b"--" || part.starts_with(b"--") {
            continue;
        }

        let (header_end, body_start) = if let Some(pos) = find_subslice(part, b"\r\n\r\n") {
            (pos, pos + 4)
        } else if let Some(pos) = find_subslice(part, b"\n\n") {
            (pos, pos + 2)
        } else {
            continue;
        };

        // Headers are ASCII; a lossy string view here is safe and convenient.
        let header_section = String::from_utf8_lossy(&part[..header_end]);
        let part_body = &part[body_start..];

        let mut field_name = String::new();
        let mut file_name = None;
        let mut part_content_type = None;

        for line in header_section.lines() {
            let lower = line.to_lowercase();
            if lower.starts_with("content-disposition:") {
                if let Some(pos) = line.find("name=\"") {
                    let rest = &line[pos + 6..];
                    if let Some(end) = rest.find('"') {
                        field_name = rest[..end].to_string();
                    }
                }
                if let Some(pos) = line.find("filename=\"") {
                    let rest = &line[pos + 10..];
                    if let Some(end) = rest.find('"') {
                        file_name = Some(rest[..end].to_string());
                    }
                }
            } else if lower.starts_with("content-type:") {
                part_content_type = Some(line[13..].trim().to_string());
            }
        }

        if field_name.is_empty() {
            continue;
        }

        if let Some(fname) = file_name {
            let (server_dir, temp_path) = write_multipart_file(&fname, part_body);

            let mut file_info = ValueMap::default();
            file_info.insert("serverFile".to_string(), CfmlValue::string(fname.clone()));
            file_info.insert("clientFile".to_string(), CfmlValue::string(fname.clone()));
            file_info.insert("serverDirectory".to_string(), CfmlValue::string(server_dir));
            file_info.insert("serverFileName".to_string(), CfmlValue::string(fname.clone()));
            file_info.insert("tempFilePath".to_string(), CfmlValue::string(temp_path));
            file_info.insert(
                "contentType".to_string(),
                CfmlValue::string(
                    part_content_type
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                ),
            );
            file_info.insert("fileSize".to_string(), CfmlValue::Int(part_body.len() as i64));
            file_info.insert("fileWasSaved".to_string(), CfmlValue::Bool(true));

            form.insert(field_name.to_lowercase(), CfmlValue::strukt(file_info));
        } else {
            form.insert(
                field_name.to_lowercase(),
                CfmlValue::string(String::from_utf8_lossy(part_body).into_owned()),
            );
        }
    }

    form
}

/// Find the first index of `needle` within `haystack` (byte-level `.find`).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Split `body` on every occurrence of `delimiter`, returning the byte slices
/// between delimiters (mirrors `str::split` semantics for `&[u8]`).
fn split_on_bytes<'a>(body: &'a [u8], delimiter: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    while start <= body.len() {
        match find_subslice(&body[start..], delimiter) {
            Some(rel) => {
                let abs = start + rel;
                parts.push(&body[start..abs]);
                start = abs + delimiter.len();
            }
            None => {
                parts.push(&body[start..]);
                break;
            }
        }
    }
    parts
}

fn strip_one_leading_crlf(part: &[u8]) -> &[u8] {
    if let Some(rest) = part.strip_prefix(b"\r\n") {
        rest
    } else if let Some(rest) = part.strip_prefix(b"\n") {
        rest
    } else {
        part
    }
}

fn strip_one_trailing_crlf(part: &[u8]) -> &[u8] {
    if let Some(rest) = part.strip_suffix(b"\r\n") {
        rest
    } else if let Some(rest) = part.strip_suffix(b"\n") {
        rest
    } else {
        part
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_multipart_file(fname: &str, bytes: &[u8]) -> (String, String) {
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("cfupload_{}", fname));
    let _ = std::fs::write(&temp_path, bytes);
    (
        temp_dir.to_string_lossy().to_string(),
        temp_path.to_string_lossy().to_string(),
    )
}

#[cfg(target_arch = "wasm32")]
fn write_multipart_file(_fname: &str, _bytes: &[u8]) -> (String, String) {
    // No filesystem on Workers — surface metadata only.
    (String::new(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn default_content_type_when_none_set() {
        let (ct, hdrs) = resolve_response_headers(None, &[]);
        assert_eq!(ct, "text/html; charset=utf-8");
        assert!(hdrs.is_empty());
    }

    #[test]
    fn cfcontent_type_used_when_no_cfheader() {
        let (ct, hdrs) =
            resolve_response_headers(Some("application/pdf"), &h(&[("X-Foo", "bar")]));
        assert_eq!(ct, "application/pdf");
        assert_eq!(hdrs, h(&[("X-Foo", "bar")]));
    }

    #[test]
    fn cfheader_content_type_replaces_default_and_is_not_duplicated() {
        // Issue #148: cfheader(name="Content-Type") must REPLACE, not append.
        let (ct, hdrs) = resolve_response_headers(
            None,
            &h(&[("Content-Type", "application/json; charset=utf-8")]),
        );
        assert_eq!(ct, "application/json; charset=utf-8");
        // Content-Type is stripped from the emit list (applied as the singleton).
        assert!(hdrs.iter().all(|(n, _)| !n.eq_ignore_ascii_case("content-type")));
    }

    #[test]
    fn cfheader_content_type_wins_over_cfcontent() {
        let (ct, _) = resolve_response_headers(
            Some("text/html; charset=utf-8"),
            &h(&[("content-type", "application/json")]),
        );
        assert_eq!(ct, "application/json");
    }

    #[test]
    fn last_content_type_wins() {
        let (ct, _) = resolve_response_headers(
            None,
            &h(&[("Content-Type", "text/plain"), ("Content-Type", "application/json")]),
        );
        assert_eq!(ct, "application/json");
    }

    #[test]
    fn other_singletons_deduped_last_wins() {
        let (_, hdrs) = resolve_response_headers(
            None,
            &h(&[("Location", "/a"), ("X-Foo", "1"), ("location", "/b")]),
        );
        let locs: Vec<_> = hdrs
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("location"))
            .collect();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].1, "/b");
    }

    #[test]
    fn multipart_preserves_binary_upload_bytes() {
        // Regression: a PNG (or any binary) upload must survive multipart
        // parsing byte-for-byte. The old lossy-UTF-8 path corrupted the very
        // first byte (0x89) into U+FFFD, breaking image format detection.
        let boundary = "----RustCFMLBoundary";
        let ct = format!("multipart/form-data; boundary={}", boundary);
        // Bytes that are NOT valid UTF-8: PNG signature + a stray high byte.
        let file_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xC0, 0x00, 0x0D,
        ];

        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"upload\"; filename=\"x.png\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        let form = parse_multipart_sync(&ct, &body);
        let file = match form.get("upload") {
            Some(CfmlValue::Struct(s)) => s,
            other => panic!("expected file struct, got {:?}", other),
        };
        match file.get("fileSize") {
            Some(CfmlValue::Int(n)) => assert_eq!(n, file_bytes.len() as i64),
            other => panic!("expected fileSize Int, got {:?}", other),
        }
        let temp_path = match file.get("tempFilePath") {
            Some(CfmlValue::String(p)) => p.to_string(),
            other => panic!("expected tempFilePath, got {:?}", other),
        };
        let written = std::fs::read(&temp_path).expect("temp file should exist");
        let _ = std::fs::remove_file(&temp_path);
        assert_eq!(written, file_bytes, "uploaded bytes must be preserved exactly");
    }

    #[test]
    fn multipart_parses_text_field() {
        let boundary = "----B";
        let ct = format!("multipart/form-data; boundary={}", boundary);
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"greeting\"\r\n\r\n");
        body.extend_from_slice("héllo wörld".as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        let form = parse_multipart_sync(&ct, &body);
        match form.get("greeting") {
            Some(CfmlValue::String(s)) => assert_eq!(s.as_str(), "héllo wörld"),
            other => panic!("expected greeting String, got {:?}", other),
        }
    }

    #[test]
    fn non_singleton_headers_keep_multiplicity() {
        let (_, hdrs) = resolve_response_headers(
            None,
            &h(&[("Set-Cookie", "a=1"), ("Set-Cookie", "b=2")]),
        );
        let cookies: Vec<_> = hdrs
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie"))
            .collect();
        assert_eq!(cookies.len(), 2);
    }
}
