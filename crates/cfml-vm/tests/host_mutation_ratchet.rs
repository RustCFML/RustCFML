//! Ratchet on host filesystem mutation and process launch in `cfml-vm` and
//! `cfml-stdlib`.
//!
//! The sandbox used to be one dispatch arm that most write paths never reached
//! (`cfexecute`, `java.nio.file.Files`, `writeDump(output=)`,
//! `spreadsheetWriteToCsv`, …). The name-level gate in
//! `cfml-vm/src/host_policy.rs` now refuses them, but a NEW writer added
//! somewhere else would be invisible to it. So every `std::fs` mutation and
//! every `Command::new` is counted per file, and a count that grows fails here.
//!
//! If this fails because you added one: make sure the call is refused in
//! sandbox mode — classify the builtin in `host_policy.rs` (the
//! `every_path_changing_builtin_is_classified` test will ask you to), or call
//! `cfml_common::host_policy::check_host_write` / `check_host_exec` right
//! before it, or `host_write_guard` / `host_exec_guard` from VM code — then
//! raise the number below in the same commit. If you removed one, lower it.

use std::path::Path;

/// (path under `crates/`, number of mutation / exec call sites).
const BASELINE: &[(&str, usize)] = &[
    ("cfml-vm/src/disk_cache.rs", 7),
    ("cfml-vm/src/ehcache_shim.rs", 7),
    ("cfml-vm/src/intercepts_output.rs", 1),
    ("cfml-vm/src/intercepts_upload.rs", 3),
    ("cfml-vm/src/io_csv_shim.rs", 3),
    ("cfml-vm/src/java_shims.rs", 16),
    ("cfml-vm/src/lib.rs", 2),
    ("cfml-vm/src/upload_temp.rs", 10),
    ("cfml-vm/src/web.rs", 10),
    ("cfml-vm/src/mcp/client.rs", 1),
    ("cfml-stdlib/src/builtins.rs", 34), // +1: cfzip removes a partial file past its cap (cfzip is sandbox-refused)
    ("cfml-stdlib/src/file_handles.rs", 2),
    ("cfml-stdlib/src/image.rs", 2),
    ("cfml-stdlib/src/s3_builtins.rs", 1),
    ("cfml-stdlib/src/s3_service.rs", 1),
    ("cfml-stdlib/src/spreadsheet.rs", 1),
];

const PATTERNS: &[&str] = &[
    "fs::write(",
    "fs::copy(",
    "fs::rename(",
    "fs::remove_file(",
    "fs::remove_dir(",
    "fs::remove_dir_all(",
    "fs::create_dir(",
    "fs::create_dir_all(",
    "fs::hard_link(",
    "fs::set_permissions(",
    "File::create(",
    "File::create_new(",
    "OpenOptions::new(",
    "Command::new(",
];

fn count_in(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            PATTERNS.iter().any(|p| compact.contains(p))
        })
        .count()
}

fn walk(dir: &Path, crates_root: &Path, out: &mut Vec<(String, usize)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, crates_root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let n = count_in(&std::fs::read_to_string(&path).unwrap());
            if n > 0 {
                let rel = path.strip_prefix(crates_root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, n));
            }
        }
    }
}

#[test]
fn host_mutation_call_sites_do_not_grow_unreviewed() {
    let crates_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut found = Vec::new();
    for c in ["cfml-vm/src", "cfml-stdlib/src"] {
        walk(&crates_root.join(c), crates_root, &mut found);
    }
    let mut problems = Vec::new();
    for (file, n) in &found {
        match BASELINE.iter().find(|(f, _)| f == file) {
            Some((_, allowed)) if n <= allowed => {}
            Some((_, allowed)) => problems.push(format!("{file}: {n} call sites (baseline {allowed})")),
            None => problems.push(format!("{file}: {n} call sites (new file, baseline 0)")),
        }
    }
    assert!(
        problems.is_empty(),
        "host write / exec call sites grew — see this file's header for what to do:\n  {}",
        problems.join("\n  ")
    );
}
