//! `rustcfml ext …` — scaffold, build, package, install and inspect native
//! extensions.
//!
//! The commands exist so that publishing an extension does not require knowing
//! anything about the ABI: `ext new` writes a crate that already compiles,
//! `ext build` produces the `.rcx`, and `ext install` puts it somewhere the
//! engine will find it (and clears the macOS quarantine that would otherwise
//! make it fail to load with no explanation).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use cfml_module_abi as abi;

use crate::extensions::{self, Manifest};

pub fn main(args: &[String]) -> i32 {
    let Some(cmd) = args.first().map(String::as_str) else {
        usage();
        return 1;
    };
    let rest = &args[1..];
    match cmd {
        "new" => cmd_new(rest),
        "build" => cmd_build(rest),
        "install" => cmd_install(rest),
        "list" => cmd_list(rest),
        "remove" => cmd_remove(rest),
        "-h" | "--help" | "help" => {
            usage();
            0
        }
        other => {
            eprintln!("Unknown command `ext {}`.\n", other);
            usage();
            1
        }
    }
}

fn usage() {
    println!(
        "Usage: rustcfml ext <command>\n\
         \n\
         \x20 new <name> [dir]        scaffold an extension crate\n\
         \x20 build [dir]             build the cdylib and package a .rcx\n\
         \x20 install <file> [--user|--dir D]\n\
         \x20                         verify and install a .rcx\n\
         \x20 list                    installed extensions and their load status\n\
         \x20 remove <name>           delete an installed .rcx\n\
         \n\
         This engine speaks extension ABI {} on target {}.",
        abi::ABI_MAJOR,
        abi::TARGET
    );
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{}=", name)) {
            return Some(v.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// new
// ---------------------------------------------------------------------------

fn cmd_new(args: &[String]) -> i32 {
    let Some(name) = args.first().filter(|s| !s.starts_with('-')) else {
        eprintln!("Usage: rustcfml ext new <name> [dir]");
        return 1;
    };
    let dir = args
        .get(1)
        .filter(|s| !s.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name));
    if dir.exists() {
        eprintln!("Error: {} already exists", dir.display());
        return 1;
    }
    let crate_name = name.replace(['-', ' '], "_");
    if let Err(e) = scaffold(&dir, name, &crate_name) {
        eprintln!("Error: {}", e);
        return 1;
    }
    println!(
        "Created {}\n\nNext:\n  cd {}\n  rustcfml ext build .\n  rustcfml ext install {}-0.1.0.rcx --user",
        dir.display(),
        dir.display(),
        name
    );
    0
}

/// How the scaffold should depend on the wrapper crate.
///
/// Inside an engine checkout there is nothing published to depend on yet, so a
/// path dependency is the only thing that builds; anywhere else the released
/// crate is what an extension author wants.
fn module_dependency() -> String {
    match crate::rustcfml_source_root() {
        Some(root) => format!(
            "rustcfml-module = {{ path = \"{}\" }}",
            root.join("crates").join("rustcfml-module").display()
        ),
        None => "rustcfml-module = \"0.1\"".to_string(),
    }
}

fn scaffold(dir: &Path, name: &str, crate_name: &str) -> Result<(), String> {
    let src = dir.join("src");
    fs::create_dir_all(&src).map_err(|e| e.to_string())?;

    let cargo = format!(
        r#"[package]
name    = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
# `cdylib` is what `.rcx` ships. `rlib` keeps the same source usable by the
# static `rustcfml --build` path, so one crate serves both delivery modes.
crate-type = ["cdylib", "rlib"]

[dependencies]
{module_dep}

[profile.release]
opt-level = 3
"#,
        module_dep = module_dependency()
    );
    fs::write(dir.join("Cargo.toml"), cargo).map_err(|e| e.to_string())?;

    let lib = format!(
        r#"//! A RustCFML native extension.

use rustcfml_module::{{module, Ctx, NativeClass, Result, Value}};

/// `{crate_name}Greet( [name] )`
fn greet<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {{
    let who = match args.first() {{
        Some(v) if !v.is_null() => v.to_string(),
        _ => "World".to_string(),
    }};
    Ok(ctx.string(format!("Hello, {{who}}, from Rust")))
}}

/// A class with state. Note `&self`: interior mutability is the contract, and
/// it is what lets the host dispatch without an exclusive lock.
pub struct Tally {{
    count: std::sync::atomic::AtomicI64,
}}

impl NativeClass for Tally {{
    const CLASS_NAME: &'static str = "Tally";

    fn new(_ctx: &Ctx, _args: &[Value]) -> Result<Self> {{
        Ok(Tally {{ count: std::sync::atomic::AtomicI64::new(0) }})
    }}

    fn method_params(method: &str) -> Option<&'static str> {{
        match method {{
            "bump" => Some("by"),
            "value" | "reset" => Some(""),
            _ => None,
        }}
    }}

    fn call<'a>(&self, ctx: &'a Ctx, method: &str, args: &[Value<'a>]) -> Result<Value<'a>> {{
        use std::sync::atomic::Ordering;
        match method.to_ascii_lowercase().as_str() {{
            "bump" => {{
                let by = args.first().map(|v| v.as_i64().unwrap_or(1)).unwrap_or(1);
                Ok(ctx.int(self.count.fetch_add(by, Ordering::SeqCst) + by))
            }}
            "value" => Ok(ctx.int(self.count.load(Ordering::SeqCst))),
            // A mutator returning `ctx.this()` chains: the host substitutes the
            // receiver, because the module has no handle to itself.
            "reset" => {{
                self.count.store(0, Ordering::SeqCst);
                Ok(ctx.this())
            }}
            other => Err(Error(other)),
        }}
    }}
}}

#[allow(non_snake_case)]
fn Error(method: &str) -> rustcfml_module::Error {{
    rustcfml_module::Error::new(format!("Tally has no method [{{method}}]"))
}}

/// `{crate_name}Tally()` — a fresh Tally, for callers who prefer a function to
/// `createObject( "rust", "Tally" )`.
fn tally<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {{
    Ok(ctx.new_object(Tally {{ count: std::sync::atomic::AtomicI64::new(0) }}))
}}

module! {{
    name: "{name}",
    version: "0.1.0",
    bifs: {{
        "{crate_name}Greet" => greet,
        "{crate_name}Tally" => tally,
    }},
    classes: {{ Tally }},
}}
"#
    );
    fs::write(src.join("lib.rs"), lib).map_err(|e| e.to_string())?;

    let readme = format!(
        "# {name}\n\nA RustCFML native extension.\n\n```sh\nrustcfml ext build .\nrustcfml ext install {name}-0.1.0.rcx --user\n```\n\nThen, from CFML:\n\n```cfml\nwriteOutput( {crate_name}Greet( \"there\" ) );\nt = {crate_name}Tally();\nwriteOutput( t.bump( by = 5 ).value() );\n```\n"
    );
    fs::write(dir.join("README.md"), readme).map_err(|e| e.to_string())?;
    fs::write(dir.join(".gitignore"), "/target\n*.rcx\n").map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

fn cmd_build(args: &[String]) -> i32 {
    let dir = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = fs::canonicalize(&dir).unwrap_or(dir);
    let manifest_path = dir.join("Cargo.toml");
    if !manifest_path.exists() {
        eprintln!("Error: no Cargo.toml in {}", dir.display());
        return 1;
    }

    let (name, version) = match read_crate_identity(&manifest_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {}", e);
            return 1;
        }
    };

    // The extension must not inherit the engine's PGO profile flags: they point
    // at our profdata, which says nothing about this crate.
    //
    // The artifact path comes from cargo's own JSON output rather than being
    // guessed from the crate directory — a crate inside a workspace puts its
    // output in the WORKSPACE target dir, which no amount of `dir/target`
    // guessing finds.
    println!("Building {} for {} …", dir.display(), abi::TARGET);
    let built = match cargo_build_cdylib(&dir, &name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {}", e);
            return 1;
        }
    };

    match package(&dir, &name, &version, &built) {
        Ok(out) => {
            let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!("Packaged {} ({:.1} KB)", out.display(), size as f64 / 1024.0);
            0
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            1
        }
    }
}

fn read_crate_identity(manifest: &Path) -> Result<(String, String), String> {
    let text = fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let mut name = String::new();
    let mut version = String::new();
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(v) = t.strip_prefix("name") {
            name = unquote(v);
        } else if let Some(v) = t.strip_prefix("version") {
            version = unquote(v);
        }
    }
    if name.is_empty() {
        return Err("Cargo.toml has no [package] name".to_string());
    }
    if version.is_empty() {
        version = "0.0.0".to_string();
    }
    Ok((name, version))
}

fn unquote(v: &str) -> String {
    v.trim_start_matches(['=', ' ']).trim().trim_matches('"').to_string()
}

/// Build the crate and return the `cdylib` cargo actually produced.
fn cargo_build_cdylib(dir: &Path, package: &str) -> Result<PathBuf, String> {
    use std::process::{Command, Stdio};

    let output = Command::new("cargo")
        .args(["build", "--release", "--message-format=json-render-diagnostics"])
        .current_dir(dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not run cargo: {}", e))?;
    if !output.status.success() {
        return Err(format!("cargo build failed ({})", output.status));
    }

    let suffix = extensions::dylib_suffix();
    let mut found: Option<PathBuf> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-artifact") {
            continue;
        }
        let is_ours = msg
            .get("target")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            .map(|n| n == package || n == package.replace('-', "_"))
            .unwrap_or(false);
        if !is_ours {
            continue;
        }
        if let Some(files) = msg.get("filenames").and_then(|f| f.as_array()) {
            for f in files {
                let Some(path) = f.as_str() else { continue };
                if path.ends_with(suffix) {
                    found = Some(PathBuf::from(path));
                }
            }
        }
    }
    found.ok_or_else(|| {
        format!(
            "no cdylib produced for [{}]. Does the crate declare\n  [lib]\n  crate-type = [\"cdylib\"]?",
            package
        )
    })
}

fn package(dir: &Path, name: &str, version: &str, lib: &Path) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};

    let bytes = fs::read(lib).map_err(|e| format!("{}: {}", lib.display(), e))?;
    let sha: String = {
        let mut h = Sha256::new();
        h.update(&bytes);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    };
    let file_name = lib.file_name().and_then(|s| s.to_str()).unwrap_or("extension");
    let inner = format!("lib/{}/{}", abi::TARGET, file_name);

    // Ask the built library what it actually provides, rather than trusting a
    // hand-written manifest: the declaration in the code is the truth, and a
    // manifest that disagrees is how conflict reporting quietly goes wrong.
    // That includes the NAME — a crate called `rustcfml-typst` may well declare
    // itself as `typst`, and the loader keys on the declared name.
    let decl = introspect(lib)?;
    let (bifs, classes, abi_major, tier) = (decl.bifs, decl.classes, decl.abi_major, decl.tier);
    let name = if decl.name.is_empty() { name } else { &decl.name };
    let version = if decl.version.is_empty() { version } else { &decl.version };

    let mut libraries = std::collections::HashMap::new();
    libraries.insert(abi::TARGET.to_string(), (inner.clone(), sha));

    // Merge into an existing .rcx if one is present, so a second platform's
    // build produces the "fat extension" rather than replacing the first.
    let out = dir.join(format!("{}-{}.rcx", name, version));
    let mut existing_libs: Vec<(String, String, Vec<u8>)> = Vec::new();
    if out.exists() {
        if let Ok(prior) = fs::File::open(&out).map_err(|e| e.to_string()).and_then(|f| {
            zip::ZipArchive::new(f).map_err(|e| e.to_string())
        }) {
            let mut prior = prior;
            for i in 0..prior.len() {
                let mut entry = match prior.by_index(i) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let entry_name = entry.name().to_string();
                if entry_name.starts_with("lib/") && entry_name != inner {
                    let mut buf = Vec::new();
                    use std::io::Read;
                    if entry.read_to_end(&mut buf).is_ok() {
                        let triple = entry_name
                            .split('/')
                            .nth(1)
                            .unwrap_or_default()
                            .to_string();
                        existing_libs.push((triple, entry_name, buf));
                    }
                }
            }
        }
        // Carry the other platforms' digests across.
        if let Ok(m) = read_manifest_from(&out) {
            for (triple, (path, sha)) in m.libraries {
                if triple != abi::TARGET {
                    libraries.insert(triple, (path, sha));
                }
            }
        }
    }

    let manifest = Manifest {
        name: name.to_string(),
        version: version.to_string(),
        abi_major,
        tier,
        description: String::new(),
        provides_bifs: bifs,
        provides_classes: classes,
        exclusive: Vec::new(),
        libraries,
    };

    let file = fs::File::create(&out).map_err(|e| format!("{}: {}", out.display(), e))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("module.json", opts).map_err(|e| e.to_string())?;
    zip.write_all(manifest.to_json().as_bytes()).map_err(|e| e.to_string())?;

    zip.start_file(&inner, opts).map_err(|e| e.to_string())?;
    zip.write_all(&bytes).map_err(|e| e.to_string())?;

    for (_triple, path, data) in existing_libs {
        zip.start_file(&path, opts).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }

    // Ship the CFML side, if there is one. The format carries it, but nothing
    // registers it yet — so say so here rather than let someone ship a CFC
    // facade that silently never appears.
    let cfml_dir = dir.join("cfml");
    if cfml_dir.is_dir() {
        eprintln!(
            "Warning: {} contains CFML, which is packaged but NOT yet registered as a mapping by \n\
             \x20        the engine. Its CFCs will not be reachable until that lands.",
            cfml_dir.display()
        );
        add_dir(&mut zip, &cfml_dir, &cfml_dir, "cfml", opts)?;
    }
    for extra in ["README.md", "LICENSE"] {
        let p = dir.join(extra);
        if p.is_file() {
            if let Ok(data) = fs::read(&p) {
                zip.start_file(extra, opts).map_err(|e| e.to_string())?;
                zip.write_all(&data).map_err(|e| e.to_string())?;
            }
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

fn add_dir<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &Path,
    dir: &Path,
    prefix: &str,
    opts: zip::write::FileOptions<()>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            add_dir(zip, base, &path, prefix, opts)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            let name = format!("{}/{}", prefix, rel.to_string_lossy().replace('\\', "/"));
            let data = fs::read(&path).map_err(|e| e.to_string())?;
            zip.start_file(&name, opts).map_err(|e| e.to_string())?;
            zip.write_all(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn read_manifest_from(rcx: &Path) -> Result<Manifest, String> {
    use std::io::Read;
    let f = fs::File::open(rcx).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut json = String::new();
    zip.by_name("module.json")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut json)
        .map_err(|e| e.to_string())?;
    Manifest::parse(&json, rcx)
}

/// Load the freshly built library just far enough to read its declaration.
///
/// This is why the manifest can be trusted for conflict reporting: it is
/// generated from what the code actually declares, never hand-maintained.
struct Decl {
    name: String,
    version: String,
    bifs: Vec<String>,
    classes: Vec<String>,
    abi_major: u32,
    tier: u32,
}

fn introspect(lib: &Path) -> Result<Decl, String> {
    unsafe {
        let library = libloading::Library::new(lib)
            .map_err(|e| format!("{}: could not be loaded for inspection: {}", lib.display(), e))?;
        let decl_fn: libloading::Symbol<extern "C" fn() -> *const abi::ModuleDecl> =
            library.get(abi::DECL_SYMBOL).map_err(|_| {
                format!(
                    "{}: no `rustcfml_module_decl` symbol. Did you call the module! macro?",
                    lib.display()
                )
            })?;
        let decl = decl_fn();
        if decl.is_null() {
            return Err("rustcfml_module_decl() returned null".to_string());
        }
        let d = &*decl;
        let mut bifs = Vec::with_capacity(d.bif_count);
        for i in 0..d.bif_count {
            bifs.push((*d.bifs.add(i)).name.as_str().to_string());
        }
        let mut classes = Vec::with_capacity(d.class_count);
        for i in 0..d.class_count {
            classes.push((*d.classes.add(i)).name.as_str().to_string());
        }
        let out = Decl {
            name: d.name.as_str().to_string(),
            version: d.version.as_str().to_string(),
            bifs,
            classes,
            abi_major: d.abi_major,
            tier: d.tier,
        };
        std::mem::forget(library);
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// install / list / remove
// ---------------------------------------------------------------------------

fn install_dir(args: &[String]) -> PathBuf {
    if let Some(d) = flag_value(args, "--dir") {
        return PathBuf::from(d);
    }
    if args.iter().any(|a| a == "--user") {
        if let Some(home) = extensions::home_dir() {
            return home.join(".rustcfml").join("extensions");
        }
    }
    PathBuf::from("extensions")
}

fn cmd_install(args: &[String]) -> i32 {
    let Some(file) = args.first().filter(|s| !s.starts_with('-')) else {
        eprintln!("Usage: rustcfml ext install <file.rcx> [--user | --dir DIR]");
        return 1;
    };
    let src = PathBuf::from(file);
    if !src.is_file() {
        eprintln!("Error: {} not found", src.display());
        return 1;
    }
    let manifest = match read_manifest_from(&src) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error: {}", e);
            return 1;
        }
    };
    if manifest.abi_major != abi::ABI_MAJOR {
        eprintln!(
            "Error: {} {} is built for extension ABI {}, but this engine speaks {}.",
            manifest.name, manifest.version, manifest.abi_major, abi::ABI_MAJOR
        );
        return 1;
    }
    if !manifest.libraries.contains_key(abi::TARGET) {
        let mut have: Vec<&str> = manifest.libraries.keys().map(String::as_str).collect();
        have.sort_unstable();
        eprintln!(
            "Error: {} {} carries no library for this platform [{}].\n       It has: {}",
            manifest.name,
            manifest.version,
            abi::TARGET,
            if have.is_empty() { "none".to_string() } else { have.join(", ") }
        );
        return 1;
    }

    let dir = install_dir(args);
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Error: {}: {}", dir.display(), e);
        return 1;
    }
    let dest = dir.join(format!("{}-{}.rcx", manifest.name, manifest.version));
    if let Err(e) = fs::copy(&src, &dest) {
        eprintln!("Error: {}: {}", dest.display(), e);
        return 1;
    }

    // Stage now rather than at first boot, so a digest mismatch or a Gatekeeper
    // problem is reported here, where someone is watching.
    match load_check(&dest, &manifest) {
        Ok(()) => println!(
            "Installed {} {} to {}\nRestart the engine to activate it.",
            manifest.name,
            manifest.version,
            dest.display()
        ),
        Err(e) => {
            eprintln!("Error: installed, but it does not load: {}", e);
            return 1;
        }
    }
    0
}

fn load_check(rcx: &Path, manifest: &Manifest) -> Result<(), String> {
    // Deliberately unfiltered: `ext install` is checking that this file loads
    // at all, not whether the server config happens to enable it.
    let (loaded, problems) = extensions::load_all(
        &[rcx.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))],
        &Default::default(),
    );
    if let Some(p) = problems.first() {
        return Err(p.clone());
    }
    if !loaded.iter().any(|e| e.name == manifest.name) {
        return Err("it did not appear after loading".to_string());
    }
    Ok(())
}

fn cmd_list(_args: &[String]) -> i32 {
    let dirs = extensions::search_dirs(None, Some(Path::new(".")));
    let found = extensions::discover(&dirs);
    if found.is_empty() {
        println!(
            "No extensions found. Searched:\n{}",
            dirs.iter().map(|d| format!("  {}", d.display())).collect::<Vec<_>>().join("\n")
        );
        return 0;
    }
    println!("{:<24} {:<10} {:<6} {}", "NAME", "VERSION", "TIER", "SOURCE");
    for c in &found {
        let (version, tier) = match &c.manifest {
            Some(m) => (m.version.clone(), m.tier.to_string()),
            None => ("-".to_string(), "-".to_string()),
        };
        println!("{:<24} {:<10} {:<6} {}", c.name, version, tier, c.path.display());
    }
    let (loaded, problems) = extensions::load_all(&dirs, &Default::default());
    println!("\n{} loaded, {} problem(s)", loaded.len(), problems.len());
    for p in problems {
        println!("  ! {}", p);
    }
    for e in loaded {
        println!(
            "  ok {} {} — {} bif(s), {} class(es)",
            e.name,
            e.version,
            e.module.bifs.len(),
            e.module.classes.len()
        );
    }
    0
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(name) = args.first().filter(|s| !s.starts_with('-')) else {
        eprintln!("Usage: rustcfml ext remove <name>");
        return 1;
    };
    let dirs = extensions::search_dirs(None, Some(Path::new(".")));
    let mut removed = 0;
    for c in extensions::discover(&dirs) {
        if c.name.eq_ignore_ascii_case(name) {
            match fs::remove_file(&c.path) {
                Ok(()) => {
                    println!("Removed {}", c.path.display());
                    removed += 1;
                }
                Err(e) => eprintln!("Error: {}: {}", c.path.display(), e),
            }
        }
    }
    if removed == 0 {
        eprintln!("Error: no installed extension named [{}]", name);
        return 1;
    }
    println!("Restart the engine to deactivate it.");
    0
}
