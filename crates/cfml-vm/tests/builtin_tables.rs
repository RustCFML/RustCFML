//! `BuiltinTables` is built once per process and copied into every VM
//! (`install_builtin_tables`). A VM must still behave as if it had registered
//! the builtins itself: builtins resolve, and nothing one VM does to its copy
//! reaches another.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_compiler::parser::Parser;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{BuiltinTables, CfmlVirtualMachine};

fn run(tables: &BuiltinTables, src: &str) -> String {
    let program = CfmlCompiler::new().compile(Parser::new(src.to_string()).parse().unwrap());
    let mut vm = CfmlVirtualMachine::new(program);
    vm.install_builtin_tables(tables);
    vm.execute().unwrap();
    vm.get_output()
}

#[test]
fn builtins_resolve_from_installed_tables() {
    let tables = BuiltinTables::new(get_builtins(), get_builtin_functions());
    assert_eq!(run(&tables, r#"writeOutput(len("abc") & uCase("x"));"#), "3X");
    // Mixed-case and dynamic calls go through the lowercased index.
    assert_eq!(run(&tables, r#"writeOutput(LEN("abcd")); writeOutput(evaluate("lcase('Q')"));"#), "4q");
}

#[test]
fn vms_sharing_tables_are_independent() {
    let tables = BuiltinTables::new(get_builtins(), get_builtin_functions());
    // One VM shadows a builtin name with a variable; the next VM from the same
    // tables must not see it.
    run(&tables, r#"len = 5; uCase = "shadowed"; writeOutput(len);"#);
    assert_eq!(run(&tables, r#"writeOutput(len("ab") & uCase("y"));"#), "2Y");
}

#[test]
fn installing_into_a_vm_with_globals_keeps_them() {
    let tables = BuiltinTables::new(get_builtins(), get_builtin_functions());
    let program = CfmlCompiler::new().compile(Parser::new(r#"writeOutput(len("xyz"));"#.to_string()).parse().unwrap());
    let mut vm = CfmlVirtualMachine::new(program);
    vm.globals.insert("preexisting".to_string(), cfml_common::dynamic::CfmlValue::Int(1));
    vm.install_builtin_tables(&tables);
    assert!(vm.globals.get("preexisting").is_some());
    vm.execute().unwrap();
    assert_eq!(vm.get_output(), "3");
}
