//! File resolution order for `.cfconfig.json`.
//!
//! The caller assembles an ordered `Vec<PathBuf>` of candidate directories
//! based on the run mode (web server, CLI, or bundled binary) and we return
//! the first existing `.cfconfig.json` within them.

use std::path::{Path, PathBuf};

/// Which run mode the loader is operating in. Currently used only by callers
/// when assembling the search-path list; the resolver itself just walks the
/// list it is handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// `rustcfml --serve` — webroot, cwd, then exe dir.
    Serve,
    /// `rustcfml file.cfm` — entry-point dir, cwd, then exe dir.
    Cli,
    /// Self-contained `--build` binary at runtime — external override next to
    /// the binary takes precedence over the embedded config baked into the VFS.
    Bundled,
}

pub const FILENAME: &str = ".cfconfig.json";

/// Search `dirs` in order for a `.cfconfig.json`. Returns the first match.
pub fn resolve_config_path(dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Glob-like check used by the HTTP-block list. Matches `.cfconfig.json`,
/// `.CFConfig.json`, `.cfconfig.local.json`, etc.
///
/// Also the dot-names that commonly carry secrets or source history and must
/// never be served from a web root: VCS metadata (`.git`, `.svn`, `.hg`),
/// `.env.*` variants, Apache auth files and macOS `.DS_Store`. Matched per
/// path SEGMENT by the caller, so `/.git/config` is caught by its `.git`
/// directory. `.well-known` is deliberately not on the list (ACME, security.txt).
pub fn is_protected_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.starts_with(".cfconfig")
        || lower.ends_with(".lex")
        || matches!(
            lower.as_str(),
            ".git" | ".gitignore" | ".gitmodules" | ".svn" | ".hg" | ".htaccess" | ".htpasswd"
                | ".ds_store" | ".npmrc" | ".pgpass" | ".netrc"
        )
}

/// Convenience for callers: exe-directory candidate, with errors swallowed.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn first_match_wins() {
        let tmp = tempdir();
        let a = tmp.join("a");
        let b = tmp.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join(FILENAME), "{}").unwrap();
        let found = resolve_config_path(&[a.clone(), b.clone()]);
        assert_eq!(found, Some(b.join(FILENAME)));
    }

    #[test]
    fn returns_none_when_missing() {
        let tmp = tempdir();
        assert!(resolve_config_path(&[tmp.clone()]).is_none());
    }

    #[test]
    fn protected_filename_matches_variants() {
        assert!(is_protected_filename(".cfconfig.json"));
        assert!(is_protected_filename(".CFConfig.json"));
        assert!(is_protected_filename(".cfconfig.local.json"));
        assert!(is_protected_filename(".env"));
        assert!(is_protected_filename("Foo.lex"));
        assert!(!is_protected_filename("config.json"));
        assert!(!is_protected_filename("index.cfm"));
    }

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "rustcfml-config-test-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn rand_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos().to_string())
            .unwrap_or_else(|_| "0".into())
    }
}
