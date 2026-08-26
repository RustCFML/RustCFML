//! Loading dynamic native extensions (`.rcx`).
//!
//! An extension is a zip carrying a manifest and one shared library per target
//! triple, plus optional CFML. At start-up the loader resolves the extension
//! directories, reads each manifest **without loading anything** (so a conflict
//! or a wrong-target build is reported cheaply and legibly), extracts the right
//! library to a content-addressed cache, `dlopen`s it, checks the compatibility
//! token, runs `on_load` once, and hands the result to `set_registrar` so every
//! VM the process builds gets the extension's BIFs and classes.
//!
//! # Two rules that shape all of this
//!
//! **Nothing is ever unloaded.** Foreign function pointers live in VM
//! registries and foreign objects can outlive any request, so `dlclose` while
//! either is alive is undefined behaviour. The `Library` is deliberately
//! leaked, and `ext install` tells you to restart. This is a real gap versus
//! OSGi and is documented as one.
//!
//! **An extension is trusted code.** It is arbitrary native code with full
//! process privilege, exactly like a Lucee `.lex`. The manifest's SHA-256
//! digests protect against a corrupted or truncated download, not against a
//! hostile author. Do not read them as a sandbox.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cfml_common::dynamic::CfmlValue;
use cfml_config::schema::ExtensionsCfg;
use cfml_module_abi as abi;
use cfml_vm::foreign::{self, LoadedModule};

/// A loaded extension, and the library it must keep alive forever.
pub struct Extension {
    pub name: String,
    pub version: String,
    pub source: PathBuf,
    pub module: LoadedModule,
}

/// What a manifest says, before anything is loaded.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub abi_major: u32,
    pub tier: u32,
    pub description: String,
    /// Declared BIF and class names, so conflicts can be reported without
    /// loading a single byte of native code.
    pub provides_bifs: Vec<String>,
    pub provides_classes: Vec<String>,
    /// Exclusive capabilities (e.g. `"v8"`): the host refuses to load a second
    /// provider of the same one, because two extensions each initialising the
    /// same process-global runtime is a crash, not a conflict.
    pub exclusive: Vec<String>,
    /// triple -> (path inside the zip, sha256 hex)
    pub libraries: HashMap<String, (String, String)>,
}

impl Manifest {
    pub fn parse(json: &str, source: &Path) -> Result<Manifest, String> {
        let v: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| format!("{}: module.json is not valid JSON: {}", source.display(), e))?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let name = s("name");
        if name.is_empty() {
            return Err(format!("{}: module.json has no name", source.display()));
        }
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|i| i.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        let mut libraries = HashMap::new();
        if let Some(map) = v.get("libraries").and_then(|x| x.as_object()) {
            for (triple, entry) in map {
                let path = entry.get("path").and_then(|x| x.as_str()).unwrap_or_default();
                let sha = entry.get("sha256").and_then(|x| x.as_str()).unwrap_or_default();
                if !path.is_empty() {
                    libraries.insert(triple.clone(), (path.to_string(), sha.to_string()));
                }
            }
        }
        Ok(Manifest {
            name,
            version: s("version"),
            abi_major: v.get("abi_major").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            tier: v.get("tier").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
            description: s("description"),
            provides_bifs: list("bifs"),
            provides_classes: list("classes"),
            exclusive: list("exclusive"),
            libraries,
        })
    }

    pub fn to_json(&self) -> String {
        let libs: serde_json::Map<String, serde_json::Value> = self
            .libraries
            .iter()
            .map(|(triple, (path, sha))| {
                (
                    triple.clone(),
                    serde_json::json!({ "path": path, "sha256": sha }),
                )
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "name": self.name,
            "version": self.version,
            "abi_major": self.abi_major,
            "tier": self.tier,
            "description": self.description,
            "bifs": self.provides_bifs,
            "classes": self.provides_classes,
            "exclusive": self.exclusive,
            "libraries": libs,
        }))
        .unwrap_or_default()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// The extension-library extension for this platform.
pub fn dylib_suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    }
}

/// Where extensions are looked for, first hit wins per module name (§4.10).
pub fn search_dirs(explicit: Option<&str>, app_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = explicit {
        dirs.push(PathBuf::from(d));
    }
    if let Some(app) = app_dir {
        dirs.push(app.join("extensions"));
    }
    if let Some(home) = home_dir() {
        dirs.push(home.join(".rustcfml").join("extensions"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("extensions"));
        }
    }
    dirs
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn cache_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".rustcfml")
        .join("ext-cache")
}

/// A candidate found on disk, before it is loaded.
pub struct Candidate {
    pub path: PathBuf,
    pub manifest: Option<Manifest>,
    /// Name used for newest-wins de-duplication.
    pub name: String,
    pub version: String,
}

/// Enumerate every extension in `dirs`, reading manifests but loading no code.
pub fn discover(dirs: &[PathBuf]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            if ext.eq_ignore_ascii_case("rcx") {
                match read_manifest(&path) {
                    Ok(m) => out.push(Candidate {
                        name: m.name.clone(),
                        version: m.version.clone(),
                        manifest: Some(m),
                        path,
                    }),
                    Err(e) => eprintln!("Warning: skipping extension: {}", e),
                }
            } else if ext.eq_ignore_ascii_case(dylib_suffix()) {
                // Loose libraries: the development shape. No manifest, so no
                // conflict pre-check — the decl itself is the only source of
                // truth, and it is read after loading.
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("extension")
                    .trim_start_matches("lib")
                    .to_string();
                out.push(Candidate { path, manifest: None, name, version: String::new() });
            }
        }
    }
    out
}

fn read_manifest(rcx: &Path) -> Result<Manifest, String> {
    let file = fs::File::open(rcx).map_err(|e| format!("{}: {}", rcx.display(), e))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("{}: not a readable .rcx archive: {}", rcx.display(), e))?;
    let mut json = String::new();
    zip.by_name("module.json")
        .map_err(|_| format!("{}: no module.json in the archive", rcx.display()))?
        .read_to_string(&mut json)
        .map_err(|e| format!("{}: unreadable module.json: {}", rcx.display(), e))?;
    Manifest::parse(&json, rcx)
}

/// Extract this host's library out of an `.rcx` into the content-addressed
/// cache and return its path.
///
/// `dlopen` needs a real filesystem path on every platform we support — there
/// is no in-memory load — so the extraction is not avoidable. Keying the cache
/// on the digest means an unchanged extension is extracted exactly once, ever.
fn stage_library(rcx: &Path, manifest: &Manifest) -> Result<(PathBuf, PathBuf), String> {
    let Some((inner, want_sha)) = manifest.libraries.get(abi::TARGET) else {
        let mut have: Vec<&str> = manifest.libraries.keys().map(String::as_str).collect();
        have.sort_unstable();
        return Err(format!(
            "{} {}: no library for this platform [{}]. The archive carries: {}",
            manifest.name,
            manifest.version,
            abi::TARGET,
            if have.is_empty() { "none".to_string() } else { have.join(", ") }
        ));
    };

    let file = fs::File::open(rcx).map_err(|e| format!("{}: {}", rcx.display(), e))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("{}: {}", rcx.display(), e))?;
    let mut bytes = Vec::new();
    zip.by_name(inner)
        .map_err(|_| format!("{}: manifest names [{}] but it is not in the archive", rcx.display(), inner))?
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {}", rcx.display(), e))?;

    let got = sha256_hex(&bytes);
    if !want_sha.is_empty() && got != *want_sha {
        return Err(format!(
            "{}: {} failed its digest check (manifest says {}, archive contains {}) — the file is \
             corrupt or was modified",
            rcx.display(),
            inner,
            &want_sha[..want_sha.len().min(12)],
            &got[..12]
        ));
    }

    let dir = cache_dir().join(&got);
    let name = Path::new(inner).file_name().unwrap_or_else(|| std::ffi::OsStr::new("ext"));
    let out = dir.join(name);
    if !out.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
        fs::write(&out, &bytes).map_err(|e| format!("{}: {}", out.display(), e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(0o755));
        }
        clear_quarantine(&out);
    }
    Ok((out, dir))
}

/// Strip macOS's `com.apple.quarantine` xattr.
///
/// Gatekeeper sets it on anything downloaded, and `dlopen` of a quarantined
/// library fails outright — which presents as "extension did not load" with no
/// further explanation. Clearing it is the difference between an extension you
/// can distribute and one only its author can run.
fn clear_quarantine(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// Extract an extension's `cfml/` payload beside its staged library.
///
/// Keyed on the same content hash as the library, so an unchanged extension
/// extracts once, ever. Returns `None` when the archive ships no CFML.
fn stage_cfml(rcx: &Path, into: &Path) -> Option<PathBuf> {
    let dest = into.join("cfml");
    if dest.is_dir() {
        return Some(dest);
    }
    let file = fs::File::open(rcx).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut wrote = false;
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        let Some(rel) = name.strip_prefix("cfml/") else { continue };
        if rel.is_empty() || name.ends_with('/') {
            continue;
        }
        // Reject anything that would escape the destination. A `.rcx` is
        // trusted code, but a path-traversal entry writing outside the cache is
        // still not something to hand it for free.
        if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            eprintln!("Warning: {} contains a suspicious cfml path [{}]; skipped", rcx.display(), name);
            continue;
        }
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            if fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_ok() && fs::write(&out, buf).is_ok() {
            wrote = true;
        }
    }
    if wrote {
        Some(dest)
    } else {
        None
    }
}

/// Load one library and adopt its module declaration.
fn load_library(path: &Path, config: CfmlValue) -> Result<LoadedModule, String> {
    let label = path.display().to_string();
    unsafe {
        let library = libloading::Library::new(path)
            .map_err(|e| format!("{}: could not be loaded: {}", label, e))?;
        let decl_fn: libloading::Symbol<extern "C" fn() -> *const abi::ModuleDecl> = library
            .get(abi::DECL_SYMBOL)
            .map_err(|_| {
                format!(
                    "{}: no `rustcfml_module_decl` symbol — this is not a RustCFML extension",
                    label
                )
            })?;
        let decl = decl_fn();
        let module = foreign::adopt(decl, &label, config)?;
        // Never unloaded (see the module docs): foreign fn pointers outlive any
        // request, so dropping the Library would be undefined behaviour.
        std::mem::forget(library);
        Ok(module)
    }
}

/// Resolve, verify and load every extension, newest version winning per name.
///
/// Returns the loaded extensions and any problems, so the caller can decide how
/// loudly to complain: a broken extension is reported, never silently skipped.
pub fn load_all(dirs: &[PathBuf], cfg: &ExtensionsCfg) -> (Vec<Extension>, Vec<String>) {
    let mut problems = Vec::new();
    let mut chosen: HashMap<String, Candidate> = HashMap::new();

    for candidate in discover(dirs) {
        match chosen.get(&candidate.name) {
            // First hit wins by directory precedence; within a directory the
            // higher version wins.
            Some(existing) if version_ge(&existing.version, &candidate.version) => continue,
            _ => {
                chosen.insert(candidate.name.clone(), candidate);
            }
        }
    }

    // Conflict detection from manifests, before anything is loaded.
    let mut owner_of_bif: HashMap<String, String> = HashMap::new();
    let mut owner_of_exclusive: HashMap<String, String> = HashMap::new();
    let mut refused: Vec<String> = Vec::new();
    let mut candidates: Vec<&Candidate> = chosen.values().collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    for c in &candidates {
        let Some(m) = &c.manifest else { continue };
        for cap in &m.exclusive {
            if let Some(owner) = owner_of_exclusive.get(cap) {
                problems.push(format!(
                    "extension [{}] provides the exclusive capability [{}], which [{}] already \
                     provides — only one may be loaded",
                    m.name, cap, owner
                ));
                refused.push(m.name.clone());
            } else {
                owner_of_exclusive.insert(cap.clone(), m.name.clone());
            }
        }
        for bif in &m.provides_bifs {
            let key = bif.to_lowercase();
            if let Some(owner) = owner_of_bif.get(&key) {
                problems.push(format!(
                    "[{}] is provided by both [{}] and [{}]; the later one wins",
                    bif, owner, m.name
                ));
            }
            owner_of_bif.insert(key, m.name.clone());
        }
    }

    let mut out = Vec::new();
    for c in candidates {
        if refused.contains(&c.name) {
            continue;
        }
        // `enabled`/`disabled` from `.cfconfig.json`. Filtered here, BEFORE the
        // library is opened: a disabled extension must not get to run its
        // `on_load`, and on macOS must not even be dlopen'd.
        if !cfg.allows(&c.name) {
            continue;
        }
        let settings = cfg
            .settings_for(&c.name)
            .cloned()
            .map(cfml_vm::json_value_to_cfml)
            .unwrap_or_else(|| CfmlValue::strukt(Default::default()));
        let (lib_path, cfml_dir) = match &c.manifest {
            Some(m) => match stage_library(&c.path, m) {
                Ok((lib, cache_dir)) => (lib, stage_cfml(&c.path, &cache_dir)),
                Err(e) => {
                    problems.push(e);
                    continue;
                }
            },
            // A loose library in development: honour a `cfml/` directory
            // sitting beside it, so the CFML half is testable before packaging.
            None => {
                let beside = c.path.parent().map(|p| p.join("cfml")).filter(|p| p.is_dir());
                (c.path.clone(), beside)
            }
        };
        match load_library(&lib_path, settings) {
            Ok(mut module) => {
                module.cfml_dir = cfml_dir;
                out.push(Extension {
                    name: module.name.to_string(),
                    version: module.version.clone(),
                    source: c.path.clone(),
                    module,
                });
            }
            Err(e) => problems.push(e),
        }
    }
    (out, problems)
}

/// Compare dotted versions numerically, so `0.10.0` beats `0.9.0`.
fn version_ge(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-'])
            .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>())
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_win_numerically() {
        assert!(version_ge("0.10.0", "0.9.0"));
        assert!(!version_ge("0.9.0", "0.10.0"));
        assert!(version_ge("1.0.0", "1.0.0"));
        assert!(version_ge("2.0", "1.99.99"));
    }

    #[test]
    fn a_manifest_round_trips() {
        let mut libraries = HashMap::new();
        libraries.insert(
            "aarch64-apple-darwin".to_string(),
            ("lib/aarch64-apple-darwin/libx.dylib".to_string(), "ab".repeat(32)),
        );
        let m = Manifest {
            name: "x".into(),
            version: "0.1.0".into(),
            abi_major: 1,
            tier: 1,
            description: "d".into(),
            provides_bifs: vec!["xOne".into()],
            provides_classes: vec!["X".into()],
            exclusive: vec![],
            libraries,
        };
        let back = Manifest::parse(&m.to_json(), Path::new("t")).expect("round trip");
        assert_eq!(back.name, "x");
        assert_eq!(back.provides_bifs, vec!["xOne".to_string()]);
        assert_eq!(back.libraries.len(), 1);
    }

    #[test]
    fn a_manifest_without_a_name_is_refused() {
        assert!(Manifest::parse("{}", Path::new("t")).is_err());
    }
}
