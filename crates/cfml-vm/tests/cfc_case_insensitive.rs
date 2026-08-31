//! GH #387: Lucee resolves CFC/CFM filenames case-insensitively even on
//! case-sensitive filesystems. `createObject("component","sqlRunner")` must
//! find `SqlRunner.cfc`, and a prior exact-case `fileExists` miss (which
//! caches a negative for the wrong-case path) must not hide that hit.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlMapping, CfmlVirtualMachine};

fn compile_page(source: &str) -> BytecodeProgram {
    let processed = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script(source)
    } else {
        source.to_string()
    };
    let ast = Parser::new(processed).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

fn run_page(page_path: &str, source: &str, mappings: Vec<CfmlMapping>) -> String {
    let mut vm = CfmlVirtualMachine::new(compile_page(source));
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

fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("write fixture");
}

fn cfc(ping: &str) -> String {
    format!(
        r#"component {{
    public string function ping() {{ return "{ping}"; }}
}}"#
    )
}

fn tmp(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml_cfc_case_{}_{}_{:?}",
        label,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn createobject_finds_cfc_when_filename_case_differs() {
    let dir = tmp("relative");
    write(&dir.join("SqlRunner.cfc"), &cfc("sql"));
    write(
        &dir.join("TaskmanagerLogAppender.cfc"),
        &cfc("append"),
    );

    let page_src = r#"<cfscript>
sql = createObject("component", "sqlRunner");
appender = createObject("component", "TaskManagerLogAppender");
writeOutput(sql.ping() & "," & appender.ping());
</cfscript>"#;
    write(&dir.join("index.cfm"), page_src);
    let page = dir.join("index.cfm").to_string_lossy().to_string();

    assert_eq!("sql,append", run_page(&page, page_src, vec![]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mapping_dotted_name_finds_cfc_when_filename_case_differs() {
    let dir = tmp("mapping");
    let pkg = dir.join("system").join("services").join("database");
    std::fs::create_dir_all(&pkg).expect("mkdir pkg");
    write(&pkg.join("SqlRunner.cfc"), &cfc("mapped"));

    let page_src = r#"<cfscript>
obj = createObject("component", "preside.system.services.database.sqlRunner");
writeOutput(obj.ping());
</cfscript>"#;
    write(&dir.join("index.cfm"), page_src);
    let page = dir.join("index.cfm").to_string_lossy().to_string();
    let mappings = vec![CfmlMapping {
        name: "/preside".to_string(),
        path: dir.to_string_lossy().to_string(),
        from_application: true,
    }];

    assert_eq!("mapped", run_page(&page, page_src, mappings));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn negative_exists_cache_does_not_block_case_folded_cfc_hit() {
    let dir = tmp("negcache");
    write(&dir.join("SqlRunner.cfc"), &cfc("after-miss"));

    // fileExists on the wrong-case path caches a negative for that exact
    // string; createObject must still case-fold to the real file.
    let page_src = r#"<cfscript>
miss = fileExists(expandPath("./sqlRunner.cfc"));
obj = createObject("component", "sqlRunner");
writeOutput((miss ? "hit" : "miss") & "," & obj.ping());
</cfscript>"#;
    write(&dir.join("index.cfm"), page_src);
    let page = dir.join("index.cfm").to_string_lossy().to_string();

    assert_eq!("miss,after-miss", run_page(&page, page_src, vec![]));
    let _ = std::fs::remove_dir_all(&dir);
}
