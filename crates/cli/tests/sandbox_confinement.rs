//! A self-contained binary run with `--sandbox` must not read, write or execute
//! on the host.
//!
//! The VFS used to be built with its sandbox switch off and nothing ever turned
//! it on, so `fileRead("/etc/hosts")` in a `--sandbox` binary returned the file;
//! and the write/exec refusal lived in one generic dispatch arm that compile-
//! time-bound builtins, member calls, Java shims and every VM intercept walked
//! past. This test builds a real bundle (toolchain-free path, no `native/`), runs
//! it with `--sandbox`, and checks every route the audit reproduced — and that
//! no file appeared on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_rustcfml")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml-sandbox-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A host file that certainly exists and is readable.
fn host_file(dir: &Path) -> PathBuf {
    let p = dir.join("host-secret.txt");
    std::fs::write(&p, "TOP-SECRET-HOST-CONTENT").unwrap();
    p
}

#[test]
fn sandboxed_binary_cannot_touch_the_host() {
    let work = scratch("work");
    let app = work.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let outside = work.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let secret = host_file(&outside);
    let target = |n: u32| outside.join(format!("written{n}")).to_string_lossy().to_string();

    let probe = format!(
        r#"<cfscript>
function probe(label, fn) {{
  try {{ r = fn(); writeOutput(label & "=ALLOWED" & chr(10)); }}
  catch (any e) {{ writeOutput(label & "=refused" & chr(10)); }}
}}
writeOutput("exists=" & fileExists("{secret}") & chr(10));
writeOutput("dirExists=" & directoryExists("{outside}") & chr(10));
probe("fileRead", function(){{ return fileRead("{secret}"); }});
probe("fileReadBinary", function(){{ return fileReadBinary("{secret}"); }});
probe("fileWrite", function(){{ fileWrite("{t1}", "x"); return 1; }});
probe("cfexecute", function(){{ cfexecute(name="/bin/echo", arguments="x", variable="v", timeout=5); return 1; }});
probe("javaFiles", function(){{ return createObject("java","java.nio.file.Files").writeString("{t2}", "x"); }});
probe("writeDump", function(){{ writeDump(var=[1], output="{t3}"); return 1; }});
probe("csv", function(){{ spreadsheetWriteToCsv(spreadsheetNew(), "{t4}"); return 1; }});
probe("processBuilder", function(){{ return createObject("java","java.lang.ProcessBuilder").init(["/bin/echo","x"]).start(); }});
probe("mkdirs", function(){{ return createObject("java","java.io.File").init("{t5}").mkdirs(); }});
probe("fileOutputStream", function(){{ return createObject("java","java.io.FileOutputStream").init("{t6}"); }});
probe("cffile", function(){{ cffile(action="write", file="{t7}", output="x"); return 1; }});
probe("embedded", function(){{ return fileRead(expandPath("data.txt")); }});
</cfscript>"#,
        secret = secret.display(),
        outside = outside.display(),
        t1 = target(1),
        t2 = target(2),
        t3 = target(3),
        t4 = target(4),
        t5 = target(5),
        t6 = target(6),
        t7 = target(7),
    );
    std::fs::write(app.join("main.cfm"), probe).unwrap();
    std::fs::write(app.join("data.txt"), "embedded data").unwrap();

    let bin = work.join("sbapp");
    let built = Command::new(exe())
        .args(["--build"])
        .arg(&app)
        .arg("-o")
        .arg(&bin)
        .args(["--mode", "cli"])
        .output()
        .expect("run --build");
    assert!(
        built.status.success(),
        "--build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let run = Command::new(&bin)
        .arg("--sandbox")
        .current_dir(&work)
        .output()
        .expect("run sandboxed binary");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let err = String::from_utf8_lossy(&run.stderr).to_string();

    assert!(!out.contains("TOP-SECRET"), "host file content leaked:\n{out}");
    assert!(out.contains("exists=false"), "fileExists saw the host:\n{out}\n{err}");
    assert!(out.contains("dirExists=false"), "directoryExists saw the host:\n{out}");
    for label in [
        "fileRead",
        "fileReadBinary",
        "fileWrite",
        "cfexecute",
        "javaFiles",
        "writeDump",
        "csv",
        "processBuilder",
        "mkdirs",
        "fileOutputStream",
        "cffile",
    ] {
        assert!(
            out.contains(&format!("{label}=refused")),
            "{label} was not refused in sandbox mode:\n{out}\n{err}"
        );
    }
    // The bundled application can still read its own files.
    assert!(out.contains("embedded=ALLOWED"), "embedded read broke:\n{out}\n{err}");

    let leaked: Vec<_> = std::fs::read_dir(&outside)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("written"))
        .collect();
    assert!(leaked.is_empty(), "sandboxed binary wrote to the host: {leaked:?}");

    let _ = std::fs::remove_dir_all(&work);
}
