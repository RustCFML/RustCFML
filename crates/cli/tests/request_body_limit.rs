//! Regression test: a request body over `server.maxRequestBodySize` MUST be
//! rejected with 413, and a body under it MUST arrive intact.
//!
//! This previously read the body with `.unwrap_or_default()`, so an over-limit
//! body was silently replaced with an EMPTY one: the request then ran with no
//! form scope at all — not merely a missing file — and the failure surfaced far
//! downstream as a baffling "variable is undefined". Preside's asset upload hit
//! exactly that, reporting `Variable 'serverFile' is undefined` for what was
//! really "your upload exceeded the limit".
//!
//! The under-limit half of the test matters just as much: the default limit used
//! to be 10 MiB, which is precisely the chunk size Preside's chunked uploader
//! slices at, so every full chunk overshot by a few hundred bytes of multipart
//! envelope and lost `uuid`/`chunkNumber` along with the data.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/request_body_limit")
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_server() -> Server {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--serve")
        .arg(fixtures_dir())
        .arg("--port")
        .arg(port.to_string())
        .spawn()
        .expect("spawn rustcfml --serve");
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, port }
}

/// POST a multipart body of `payload_len` bytes of file content, shaped the way
/// Preside's chunked uploader posts a chunk (uuid + chunkNumber + chunkData).
/// Returns the full raw response.
fn post_chunk(port: u16, payload_len: usize) -> String {
    let boundary = "----------------------------rustcfmltestboundary";
    let mut body: Vec<u8> = Vec::new();
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    };
    field("uuid", "abc-123");
    field("chunkNumber", "1");
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"chunkData\"; \
             filename=\"chunk.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend(std::iter::repeat(b'A').take(payload_len));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();
    let head = format!(
        "POST /up.cfm HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: multipart/form-data; boundary={boundary}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).expect("read response");
    String::from_utf8_lossy(&out).to_string()
}

#[test]
fn over_limit_body_returns_413_and_under_limit_body_arrives_intact() {
    let server = start_server();

    // The fixture pins maxRequestBodySize to 1 MiB so the test stays fast; the
    // shipped default is far larger.
    const LIMIT: usize = 1024 * 1024;

    // Comfortably under the limit: every field must be present.
    let ok = post_chunk(server.port, 512 * 1024);
    assert!(
        ok.contains("200 OK"),
        "under-limit upload should succeed; got:\n{ok}"
    );
    assert!(
        ok.contains("formKeys=[uuid,chunknumber,chunkdata]"),
        "under-limit upload should deliver ALL form fields; got:\n{ok}"
    );

    // Over the limit: a loud 413, never a silent empty form.
    let too_big = post_chunk(server.port, LIMIT + 4096);
    assert!(
        too_big.contains("413"),
        "over-limit upload must be rejected with 413; got:\n{too_big}"
    );
    assert!(
        too_big.to_lowercase().contains("too large"),
        "413 body should explain the limit; got:\n{too_big}"
    );
    // The old bug: request ran anyway with an empty form scope.
    assert!(
        !too_big.contains("formKeys=[]"),
        "over-limit upload must NOT execute the template with an empty form \
         scope (the silent-truncation regression); got:\n{too_big}"
    );
}
