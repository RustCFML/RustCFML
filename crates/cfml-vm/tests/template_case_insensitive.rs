//! GH #387: on a case-sensitive filesystem, component and template lookup must
//! not depend on the caller spelling the on-disk filename exactly. Preside
//! ships `SqlRunner.cfc` and asks for `...database.sqlRunner`, and it
//! disagrees with directory spelling too (`sitetree` vs `siteTree/`).
//!
//! **These tests run the fold on every host.** The developer's Mac (APFS) and
//! Windows are both case-INSENSITIVE, so a test that leans on the real
//! filesystem proves nothing there: the exact-path probe succeeds and the
//! folding code never executes. So the VM is handed a `CaseSensitiveFs` — a
//! `RealFs` that refuses any path whose spelling differs from the on-disk
//! entry — which reproduces Linux/ext4 semantics anywhere. PR #388's tests
//! passed on macOS with the fold stubbed out entirely; these do not.
//!
//! The negative half matters as much: `file_bifs_stay_case_sensitive` pins the
//! Lucee behaviour verified against Lucee 7.1.0.204 on a case-sensitive APFS
//! volume — `fileExists`/`fileRead` do NOT fold, so `fileDelete("./Foo.txt")`
//! can never delete an on-disk `foo.txt`.

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{FileCursorOpts, RealFs, Vfs, VfsDirEntry, VfsFileChunks};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlMapping, CfmlVirtualMachine};

/// A `RealFs` that behaves like ext4 regardless of the host filesystem: a path
/// only exists if every segment matches an on-disk entry byte for byte.
#[derive(Debug)]
struct CaseSensitiveFs(RealFs);

impl CaseSensitiveFs {
    /// Is every segment of `path` spelled exactly as it is on disk?
    ///
    /// Only the segments below the temp root are checked — the root itself is
    /// whatever the OS handed us and is not under test.
    fn exact(&self, path: &str) -> bool {
        let p = std::path::Path::new(path);
        let mut acc = std::path::PathBuf::new();
        for comp in p.components() {
            match comp {
                std::path::Component::Normal(seg) => {
                    let listed = std::fs::read_dir(&acc)
                        .map(|rd| {
                            rd.flatten().any(|e| e.file_name() == seg)
                        })
                        .unwrap_or(false);
                    if !listed && acc.join(seg).exists() {
                        // Present under a different spelling only.
                        return false;
                    }
                    acc.push(seg);
                }
                other => acc.push(other.as_os_str()),
            }
        }
        true
    }

    fn miss<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no such file or directory"))
    }
}

impl Vfs for CaseSensitiveFs {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.read_to_string(path)
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.exact(path) && self.0.exists(path)
    }
    fn is_file(&self, path: &str) -> bool {
        self.exact(path) && self.0.is_file(path)
    }
    fn is_dir(&self, path: &str) -> bool {
        self.exact(path) && self.0.is_dir(path)
    }
    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.read_dir(path)
    }
    fn modified(&self, path: &str) -> io::Result<SystemTime> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.modified(path)
    }
    fn canonicalize(&self, path: &str) -> io::Result<String> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.canonicalize(path)
    }
    fn open_chunks(
        &self,
        path: &str,
        opts: FileCursorOpts,
    ) -> io::Result<Box<dyn VfsFileChunks>> {
        if !self.exact(path) {
            return Self::miss();
        }
        self.0.open_chunks(path, opts)
    }
}

fn compile_page(source: &str) -> BytecodeProgram {
    let processed = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script(source)
    } else {
        source.to_string()
    };
    let ast = Parser::new(processed).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

/// Run `source` as the page at `page_path`, with the case-sensitive VFS in
/// place, and return its trimmed output.
fn run(page_path: &str, source: &str, mappings: Vec<CfmlMapping>) -> String {
    run_vm(page_path, source, mappings, false)
}

/// As `run`, but with sandbox mode on so that EVERY file BIF is routed through
/// the VFS. Outside sandbox mode only `fileExists`/`directoryExists` consult it
/// and the rest reach `std::fs` directly, which on a case-insensitive host
/// answers before the engine has any say.
fn run_sandboxed(page_path: &str, source: &str) -> String {
    run_vm(page_path, source, vec![], true)
}

fn run_vm(page_path: &str, source: &str, mappings: Vec<CfmlMapping>, sandbox: bool) -> String {
    let mut vm = CfmlVirtualMachine::new(compile_page(source));
    vm.vfs = Arc::new(CaseSensitiveFs(RealFs));
    vm.sandbox = sandbox;
    vm.source_file = Some(page_path.to_string());
    vm.base_template_path = Some(page_path.to_string());
    vm.mappings = mappings;
    vm.refresh_mappings_fingerprint();
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.globals
        .entry("url".to_string())
        .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.execute().expect("execute");
    vm.get_output().trim().to_string()
}

/// A fresh, uniquely-named directory. Not `TempDir`-backed: the tests assert on
/// on-disk spelling, so the fixture layout has to be exactly what we wrote.
struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rustcfml_tmpl_case_{}_{}_{:?}",
            label,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir fixture");
        Self(dir)
    }
    fn dir(&self, rel: &str) -> std::path::PathBuf {
        let d = self.0.join(rel);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }
    fn write(&self, rel: &str, body: &str) -> String {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(&p, body).expect("write");
        p.to_string_lossy().into_owned()
    }
    fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfc(ping: &str) -> String {
    format!("component {{ public string function ping() {{ return \"{ping}\"; }} }}")
}

/// The fixture itself must be case-sensitive under this VFS, or every test
/// below passes vacuously — which is exactly how PR #388's suite stayed green
/// on macOS with the fold removed.
#[test]
fn harness_is_actually_case_sensitive() {
    let fx = Fixture::new("harness");
    fx.write("Widget.cfc", &cfc("w"));
    let fs = CaseSensitiveFs(RealFs);
    assert!(fs.exists(&fx.path("Widget.cfc")), "exact spelling must exist");
    assert!(
        !fs.exists(&fx.path("widget.cfc")),
        "wrong-case spelling must NOT exist, or these tests prove nothing"
    );
    assert!(fs.read_to_string(&fx.path("widget.cfc")).is_err());
}

#[test]
fn createobject_folds_the_filename() {
    let fx = Fixture::new("filename");
    fx.write("SqlRunner.cfc", &cfc("sql"));
    fx.write("TaskmanagerLogAppender.cfc", &cfc("append"));
    let src = r#"<cfscript>
writeOutput(createObject("component","sqlRunner").ping() & ","
          & createObject("component","TaskManagerLogAppender").ping());
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    assert_eq!("sql,append", run(&page, src, vec![]));
}

#[test]
fn createobject_folds_a_mapped_dotted_path() {
    let fx = Fixture::new("mapped");
    fx.write("system/services/database/SqlRunner.cfc", &cfc("mapped"));
    let src = r#"<cfscript>
writeOutput(createObject("component","preside.system.services.database.sqlRunner").ping());
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    let mappings = vec![CfmlMapping {
        name: "/preside".to_string(),
        path: fx.0.to_string_lossy().into_owned(),
        from_application: true,
    }];
    assert_eq!("mapped", run(&page, src, mappings));
}

/// Not just the filename: a mid-path DIRECTORY may disagree too.
#[test]
fn createobject_folds_intermediate_directories() {
    let fx = Fixture::new("dircase");
    fx.write("system/siteTree/SiteService.cfc", &cfc("site"));
    let src = r#"<cfscript>
writeOutput(createObject("component","preside.system.sitetree.siteservice").ping());
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    let mappings = vec![CfmlMapping {
        name: "/preside".to_string(),
        path: fx.0.to_string_lossy().into_owned(),
        from_application: true,
    }];
    assert_eq!("site", run(&page, src, mappings));
}

#[test]
fn cfinclude_folds_relative_and_mapped_templates() {
    let fx = Fixture::new("include");
    fx.write("views/Partial.cfm", "partial");
    fx.write("lib/Helpers.cfm", "helper");
    let src = r#"<cfinclude template="Views/partial.cfm"><cfoutput>|</cfoutput><cfinclude template="/mapped/helpers.cfm">"#;
    let page = fx.write("index.cfm", src);
    let mappings = vec![CfmlMapping {
        name: "/mapped".to_string(),
        path: fx.path("lib"),
        from_application: true,
    }];
    assert_eq!("partial|helper", run(&page, src, mappings));
}

/// An EXACT match must always win, even when it sits behind a mapping that a
/// case-folded match could have satisfied first. Folding inside the first pass
/// (rather than as a second pass over the whole order) silently loads the wrong
/// CFC here.
#[test]
fn exact_match_beats_a_case_folded_one_in_an_earlier_mapping() {
    let fx = Fixture::new("priority");
    fx.write("first/Widget.cfc", &cfc("WRONG-folded-first"));
    fx.write("second/widget.cfc", &cfc("right-exact-second"));
    let src = r#"<cfscript>
writeOutput(createObject("component","app.widget").ping());
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    let mappings = vec![
        CfmlMapping {
            name: "/app".to_string(),
            path: fx.path("first"),
            from_application: true,
        },
        CfmlMapping {
            name: "/app".to_string(),
            path: fx.path("second"),
            from_application: true,
        },
    ];
    assert_eq!("right-exact-second", run(&page, src, mappings));
}

/// A path that does not exist under ANY casing still fails — folding must not
/// turn a genuine miss into a hit on some unrelated neighbour.
#[test]
fn a_genuine_miss_still_misses() {
    let fx = Fixture::new("miss");
    fx.write("Widget.cfc", &cfc("w"));
    let src = r#"<cfscript>
try { createObject("component","gadget"); writeOutput("FOUND"); }
catch (any e) { writeOutput("notfound"); }
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    assert_eq!("notfound", run(&page, src, vec![]));
}

/// The scope line. Lucee 7.1.0.204 on a case-sensitive filesystem answers
/// `false` to `fileExists` on the wrong casing and throws from `fileRead`; the
/// engine matches that, and the fold added for GH #387 must not leak into it.
/// This is what stops `fileDelete("./Foo.txt")` from deleting an on-disk
/// `foo.txt`, and stops `fileExists` from disagreeing with `fileWrite` about
/// whether a file is there.
///
/// `fileExists`/`directoryExists` always route through the VFS (so the
/// case-sensitive harness governs them here); the rest do so only under
/// sandbox mode, which is why the read half is a separate test below.
#[test]
fn file_existence_bifs_stay_case_sensitive() {
    let fx = Fixture::new("filebifs");
    fx.dir("data");
    fx.write("data/report.txt", "lower");
    let src = r#"<cfscript>
writeOutput("file=" & fileExists(expandPath("./data/Report.txt")));
writeOutput(",dir=" & directoryExists(expandPath("./Data")));
writeOutput(",exact=" & fileExists(expandPath("./data/report.txt")));
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    assert_eq!("file=false,dir=false,exact=true", run(&page, src, vec![]));
}

/// The reading and listing BIFs, routed through the VFS by sandbox mode: a
/// wrong-case path must fail rather than be folded onto its neighbour.
#[test]
fn file_read_bifs_stay_case_sensitive() {
    let fx = Fixture::new("fileread");
    fx.dir("data");
    fx.write("data/report.txt", "lower");
    let src = r#"<cfscript>
try { writeOutput("read=" & fileRead(expandPath("./data/Report.txt"))); }
catch (any e) { writeOutput("read=threw"); }
try { writeOutput(",info=" & getFileInfo(expandPath("./data/Report.txt")).size); }
catch (any e) { writeOutput(",info=threw"); }
writeOutput(",exact=" & fileRead(expandPath("./data/report.txt")));
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    assert_eq!("read=threw,info=threw,exact=lower", run_sandboxed(&page, src));
}

/// The fold index is cached per DIRECTORY, and a file created after that
/// listing was taken must still be found — the generation stamp, not a stale
/// snapshot.
#[test]
fn a_component_created_after_a_folded_miss_is_found() {
    let fx = Fixture::new("fresh");
    fx.write("Present.cfc", &cfc("present"));
    let src = r#"<cfscript>
try { createObject("component","later"); writeOutput("early=FOUND"); }
catch (any e) { writeOutput("early=notfound"); }
fileWrite(expandPath("./Later.cfc"), 'component { function ping() { return "late"; } }');
writeOutput("," & createObject("component","later").ping());
</cfscript>"#;
    let page = fx.write("index.cfm", src);
    assert_eq!("early=notfound,late", run(&page, src, vec![]));
}
