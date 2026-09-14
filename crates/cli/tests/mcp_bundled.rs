//! MCP inside a `--build` self-contained binary.
//!
//! A bundled app is exactly what a client launches from its config
//! (`"command": "/path/to/myapp", "args": ["mcp", "docs"]`), and it serves its
//! CFCs out of the embedded archive rather than the filesystem — a different
//! VFS, a different argument parser, and its own stdout banner. None of that
//! is exercised by the ordinary stdio/HTTP suites, and the subcommand was in
//! fact ignored entirely (the binary started a web server instead) until this
//! test existed.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_app")
}

/// A built binary plus the directory holding it, removed on drop.
struct Bundle {
    dir: PathBuf,
    binary: PathBuf,
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Bundle {
    /// `rustcfml --build` the fixture app into a self-contained binary.
    fn build() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rustcfml-mcp-bundle-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let app = dir.join("app");
        copy_dir(&fixtures_dir(), &app);

        let binary = dir.join("bundled");
        let out = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
            .arg("--build")
            .arg(&app)
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("run --build");
        assert!(
            out.status.success(),
            "--build failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(binary.exists(), "no binary was produced");
        Bundle { dir, binary }
    }

    /// Drive the bundled binary's stdio MCP transport.
    fn mcp(&self, messages: &[Value]) -> Vec<Value> {
        let mut child = Command::new(&self.binary)
            .arg("mcp")
            .arg("demo")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the bundled binary");
        {
            let stdin = child.stdin.as_mut().expect("stdin");
            for msg in messages {
                writeln!(stdin, "{msg}").expect("write");
            }
        }
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut replies = Vec::new();
        for _ in 0..messages.len() {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            replies.push(serde_json::from_str(&line).unwrap_or_else(|e| {
                panic!("bundled binary wrote non-JSON to stdout: {e}\nline: {line}")
            }));
        }
        let _ = child.kill();
        let _ = child.wait();
        replies
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file_type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {} }
    })
}

#[test]
fn a_bundled_binary_serves_its_embedded_mcp_server_over_stdio() {
    let bundle = Bundle::build();
    let replies = bundle.mcp(&[
        initialize(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "echo", "arguments": { "text": "x", "times": 3 } } }),
    ]);
    assert_eq!(replies.len(), 3, "got {replies:?}");

    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "demo");
    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"echo"), "got {names:?}");
    assert_eq!(replies[2]["result"]["content"][0]["text"], "xxx");
}

#[test]
fn a_bundled_binary_reads_resources_out_of_its_embedded_archive() {
    // The CFC and everything it serves live in the archive, not on disk, so
    // this is the check that the embedded VFS is actually reached.
    let bundle = Bundle::build();
    let replies = bundle.mcp(&[
        initialize(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read",
                "params": { "uri": "doc://pages/intro" } }),
    ]);
    assert_eq!(
        replies[1]["result"]["contents"][0]["text"],
        "page=intro uri=doc://pages/intro",
        "got {:?}",
        replies[1]
    );
}

#[test]
fn the_bundled_banner_never_reaches_the_protocol_stream() {
    // A self-contained binary prints its own banner and loads extensions
    // before anything else runs. stdout has to be claimed ahead of all of it,
    // or the client's very first read is garbage.
    let bundle = Bundle::build();
    let replies = bundle.mcp(&[initialize()]);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["jsonrpc"], "2.0");
    assert_eq!(replies[0]["id"], 1);
}
