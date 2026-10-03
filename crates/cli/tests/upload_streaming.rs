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
//!
//! And their temp files must not outlive the request (GH #386): deleted when it
//! ends, unless it handed work to a `cfthread`, in which case they are kept for
//! the age reaper — which also sweeps files a dead process left behind.

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

/// Held from picking a port until the server is accepting on it. free_port()
/// binds :0 and releases it, so two tests starting at once can be handed the
/// SAME port: one server binds, the other exits with "Address already in use",
/// both readiness probes still succeed (they reach the survivor), and the loser
/// then gets "Connection refused" once the other test kills its server. Seen as
/// an intermittent CI failure of streamed_upload_arrives_intact_and_exposes_no_raw_body.
static START: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn start_server() -> Server {
    start_server_in(fixtures_dir(), None)
}

fn start_server_in(root: PathBuf, cfconfig: Option<PathBuf>) -> Server {
    let _serialised = START.lock().unwrap_or_else(|e| e.into_inner());
    let port = free_port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustcfml"));
    cmd.arg("--serve").arg(root).arg("--port").arg(port.to_string());
    if let Some(cfg) = cfconfig {
        cmd.arg("--cfconfig").arg(cfg);
    }
    let child = cmd.spawn().expect("spawn rustcfml --serve");
    let mut server = Server { child, port };
    for _ in 0..600 {
        // A socket whose local and peer addresses match connected to ITSELF (Linux
        // TCP simultaneous open on an ephemeral-range port), not to a server.
        if TcpStream::connect(("127.0.0.1", port))
            .is_ok_and(|s| s.local_addr().ok() != s.peer_addr().ok())
        {
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
    post_upload_to(port, "/up.cfm", filename, payload)
}

fn post_upload_to(port: u16, page: &str, filename: &str, payload: &[u8]) -> String {
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
        "POST {page} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\
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

    // The template hashed the temp file while it existed.
    assert_eq!(
        field(&resp, "sha256"),
        sha256_hex(&payload),
        "the streamed bytes must reach disk unchanged"
    );

    // ...and the request's end removed it (GH #386).
    let temp_path = field(&resp, "tempFilePath").to_string();
    assert!(
        !std::path::Path::new(&temp_path).exists(),
        "the upload temp file must be deleted when the request ends: {temp_path}"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn wait_until_gone(path: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !std::path::Path::new(path).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !std::path::Path::new(path).exists()
}

#[test]
fn a_request_that_started_a_cfthread_keeps_its_upload_temp_file() {
    let server = start_server();
    let resp = post_upload_to(server.port, "/thread.cfm", "doc.txt", b"for the thread");
    assert!(resp.contains("200 OK"), "upload should succeed; got:\n{resp}");
    let temp_path = field(&resp, "tempFilePath").to_string();
    // The thread may still be reading it; Lucee keeps it for good, we keep it
    // until the age reaper (default maxAgeSecs 3600) or process exit.
    assert!(
        std::path::Path::new(&temp_path).exists(),
        "a cfthread may still be reading the upload: it must survive request end"
    );
    // Graceful shutdown would remove it; a kill does not, so tidy up here.
    drop(server);
    let _ = std::fs::remove_file(&temp_path);
}

#[test]
fn the_reaper_sweeps_aged_files_and_files_from_dead_processes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upload_reaper");

    // A file left behind by a process that no longer exists: swept on the
    // first tick, whatever its age.
    let mut gone = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn short-lived process");
    let dead_pid = gone.id();
    gone.wait().unwrap();
    let orphan = std::env::temp_dir().join(format!("cfupload_{dead_pid}_0.upload"));
    std::fs::write(&orphan, b"left by a crashed run").unwrap();
    // A name that is not ours must never be touched.
    let bystander = std::env::temp_dir().join(format!("cfupload_{dead_pid}_bystander.txt"));
    std::fs::write(&bystander, b"not ours").unwrap();

    let server = start_server_in(root.clone(), Some(root.join(".cfconfig.json")));

    #[cfg(unix)]
    assert!(
        wait_until_gone(&orphan.to_string_lossy(), Duration::from_secs(10)),
        "a dead process's upload temp file must be reaped"
    );
    #[cfg(not(unix))]
    let _ = std::fs::remove_file(&orphan);

    // Kept past request end (it started a cfthread), then aged out by the
    // reaper (maxAgeSecs 1, reapIntervalSecs 1).
    let resp = post_upload_to(server.port, "/thread.cfm", "doc.txt", b"for the thread");
    assert!(resp.contains("200 OK"), "upload should succeed; got:\n{resp}");
    let temp_path = field(&resp, "tempFilePath").to_string();
    assert!(
        wait_until_gone(&temp_path, Duration::from_secs(15)),
        "the age reaper must remove a kept upload temp file: {temp_path}"
    );

    assert!(bystander.exists(), "the reaper must only touch cfupload_<pid>_<n>.upload");
    let _ = std::fs::remove_file(&bystander);
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

    assert_eq!(
        field(&resp_a, "sha256"),
        sha256_hex(&vec![b'A'; 512 * 1024]),
        "upload A was overwritten by B"
    );
    assert_eq!(
        field(&resp_b, "sha256"),
        sha256_hex(&vec![b'B'; 512 * 1024]),
        "upload B was overwritten by A"
    );
}
