//! Regression tests for `rustcfml --licenses`.
//!
//! The flag prints RustCFML's MIT licence plus the embedded third-party
//! attribution notice (~13k lines), so the notices travel with a bare binary
//! copied out of a release, container image or `--build` bundle.
//!
//! Two things are easy to break:
//!
//! 1. **Broken pipe.** At that length, `--licenses | head` and `--licenses |
//!    less` (quitting early) are the normal ways to read it, and both close the
//!    pipe before the writer finishes. Rust ignores SIGPIPE process-wide, so the
//!    original `println!` implementation hit EPIPE and panicked with "failed
//!    printing to stdout: Broken pipe" — after the release binary had shipped.
//!    Restoring SIG_DFL globally is not a fix: serve mode depends on socket
//!    writes returning EPIPE rather than killing the process on client
//!    disconnect.
//!
//! 2. **Content.** The notice must actually carry the attribution — the MIT
//!    grant plus the weak-copyleft dependencies whose terms need more than a
//!    passing mention (see about.toml).

use std::process::{Command, Stdio};

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_rustcfml")
}

#[test]
fn licenses_prints_own_grant_and_third_party_notices() {
    let out = Command::new(exe())
        .arg("--licenses")
        .output()
        .expect("failed to run rustcfml --licenses");

    assert!(out.status.success(), "--licenses exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Our own grant, with a copyright holder.
    assert!(stdout.contains("MIT License"), "missing MIT licence text");
    assert!(
        stdout.contains("Copyright (c) 2026 RustCFML Team"),
        "missing first-party copyright line"
    );

    // Third-party attribution: the header, plus the MPL-2.0 crates that reach a
    // default build via the `cluster` and `html` features. If these vanish, the
    // notice has silently stopped covering weak-copyleft dependencies.
    assert!(
        stdout.contains("Third-Party Notices"),
        "missing third-party notice section"
    );
    assert!(
        stdout.contains("Mozilla Public License 2.0"),
        "missing MPL-2.0 licence text"
    );
    assert!(
        stdout.contains("memberlist-core") && stdout.contains("cssparser"),
        "missing MPL-2.0 crates (memberlist / cssparser)"
    );

    // First-party crates must NOT be listed as third parties. They used to be,
    // which made the generated notice churn on every version bump and broke the
    // CI staleness gate on every release.
    assert!(
        !stdout.contains("cfml-vm 0."),
        "first-party crates leaked into THIRD-PARTY.txt"
    );

    // Sanity: this is a large document, not a stub.
    assert!(
        stdout.lines().count() > 5_000,
        "notice implausibly short: {} lines",
        stdout.lines().count()
    );
}

/// Closing the pipe early must not panic. `head -1` exits after one line,
/// leaving the writer with EPIPE.
#[cfg(unix)]
#[test]
fn licenses_survives_a_closed_pipe() {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("'{}' --licenses | head -1", exe()))
        .stdin(Stdio::null())
        .output()
        .expect("failed to run piped rustcfml --licenses");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "--licenses panicked on a closed pipe: {}",
        stderr
    );
    assert!(
        !stderr.contains("Broken pipe"),
        "--licenses reported a broken pipe: {}",
        stderr
    );
    assert!(
        stderr.is_empty(),
        "--licenses wrote unexpected stderr: {}",
        stderr
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("RustCFML v"),
        "expected the first line to survive, got: {:?}",
        stdout
    );
}
