//! End-to-end cover for streamed `multipart/form-data` uploads.
//!
//! A file upload is parsed off the wire and written straight to a temp file, so
//! the body is never assembled in memory (GH #384). Three properties have to
//! hold once it is, and none of them were true of the buffered parser it
//! replaced:
//!
//! * the bytes still arrive intact, whatever the size;
//! * the client-supplied filename cannot steer where the temp file lands
//!   (`filename="../../x"` used to be interpolated straight into the path);
//! * two requests uploading the same filename get their own temp files, rather
//!   than sharing one `cfupload_<name>` and clobbering each other.

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upload_streaming")
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
    let mut server = Server { child, port };
    for _ in 0..600 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            panic!("rustcfml --serve exited before accepting connections: {status}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("rustcfml --serve not accepting connections on port {port} after 30s");
}

/// POST one file part (plus a plain field) and return the response body.
fn post_upload(port: u16, filename: &str, payload: &[u8]) -> String {
    let boundary = "----------------------------rustcfmluploadboundary";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nhi\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; \
             filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(payload);
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

/// Pull `key=value;` out of the fixture's echo.
fn field<'a>(resp: &'a str, key: &str) -> &'a str {
    let needle = format!("{key}=");
    let start = resp
        .find(&needle)
        .unwrap_or_else(|| panic!("no {key} in response:\n{resp}"))
        + needle.len();
    let rest = &resp[start..];
    let end = rest.find(';').expect("field terminator");
    &rest[..end]
}

#[test]
fn streamed_upload_arrives_intact_and_exposes_no_raw_body() {
    let server = start_server();

    // Large enough that a buffered parser's copies would dominate, small enough
    // to keep the test quick.
    let payload: Vec<u8> = (0..4 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let resp = post_upload(server.port, "photo.png", &payload);

    assert!(resp.contains("200 OK"), "upload should succeed; got:\n{resp}");
    assert_eq!(field(&resp, "clientFile"), "photo.png");
    assert_eq!(
        field(&resp, "fileSize"),
        payload.len().to_string(),
        "the streamed part must report its true size"
    );
    // Plain fields still land in the form scope alongside the file.
    assert_eq!(field(&resp, "note"), "hi");
    // A multipart body is never materialised, so there is nothing to expose as
    // getHttpRequestData().content (Lucee returns an empty stream here too).
    assert_eq!(
        field(&resp, "rawContentLen"),
        "0",
        "a multipart request must not materialise its raw envelope"
    );

    let temp_path = field(&resp, "tempFilePath").to_string();
    let written = std::fs::read(&temp_path).expect("temp file should exist");
    let _ = std::fs::remove_file(&temp_path);
    assert_eq!(
        written, payload,
        "the streamed bytes must reach disk unchanged"
    );
}

#[test]
fn client_filename_cannot_steer_the_temp_path() {
    let server = start_server();

    let resp = post_upload(server.port, "../../../etc/evil.txt", b"payload");
    assert!(resp.contains("200 OK"), "upload should succeed; got:\n{resp}");

    // The name survives as metadata, reduced to a bare basename — `cffile
    // action="upload"` joins clientFile onto its destination, so a path here
    // escapes that directory too.
    assert_eq!(field(&resp, "clientFile"), "evil.txt");

    let temp_path = field(&resp, "tempFilePath").to_string();
    let _ = std::fs::remove_file(&temp_path);
    let parent = std::path::Path::new(&temp_path)
        .parent()
        .expect("temp path has a parent");
    assert_eq!(
        parent,
        std::env::temp_dir(),
        "the upload must land in the temp dir, not where the client walked to: {temp_path}"
    );
    assert!(
        !temp_path.contains("evil") && !temp_path.contains(".."),
        "the temp path must carry nothing from the client filename: {temp_path}"
    );
}

#[test]
fn concurrent_uploads_of_one_filename_do_not_clobber_each_other() {
    let server = start_server();
    let port = server.port;

    // Same filename, different contents, in flight together: the old
    // `cfupload_<filename>` scheme gave both the same path.
    let a = std::thread::spawn(move || post_upload(port, "avatar.png", &vec![b'A'; 512 * 1024]));
    let b = std::thread::spawn(move || post_upload(port, "avatar.png", &vec![b'B'; 512 * 1024]));
    let resp_a = a.join().expect("upload A");
    let resp_b = b.join().expect("upload B");

    let path_a = field(&resp_a, "tempFilePath").to_string();
    let path_b = field(&resp_b, "tempFilePath").to_string();
    assert_ne!(
        path_a, path_b,
        "two uploads named avatar.png must not share one temp file"
    );

    let bytes_a = std::fs::read(&path_a).expect("temp file A");
    let bytes_b = std::fs::read(&path_b).expect("temp file B");
    let _ = std::fs::remove_file(&path_a);
    let _ = std::fs::remove_file(&path_b);
    assert!(
        bytes_a.iter().all(|&c| c == b'A'),
        "upload A was overwritten by B"
    );
    assert!(
        bytes_b.iter().all(|&c| c == b'B'),
        "upload B was overwritten by A"
    );
}
