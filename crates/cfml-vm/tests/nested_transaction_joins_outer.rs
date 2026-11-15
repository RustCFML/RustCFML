//! A nested `transaction { }` JOINS the transaction around it (Lucee 7.1.2,
//! probed on PostgreSQL, MySQL and SQL Server): it takes no savepoint, an
//! exception caught inside the outer block rolls nothing back, and an in-block
//! `action="commit"`/`"rollback"` acts on the whole transaction.
//!
//! Until v0.751 every nested block took a savepoint. That made nesting fail
//! outright on a driver without savepoints (DuckDB, supplied by an extension),
//! so the stub driver here has none: if anything still issued one for a
//! nested block, these tests would see the refusal.
//!
//! The hooks are plain function pointers on the VM, so the driver is stubbed
//! rather than run against a real database.

use std::sync::atomic::{AtomicUsize, Ordering};

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_common::vm::{CfmlError, CfmlResult};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

/// What the stub driver was told to do.
///
/// The stub hooks are plain `fn` pointers, so their bookkeeping has to be
/// process-global — which means the tests in this file cannot run at the same
/// time as each other. Each test takes this gate first.
static BEGINS: AtomicUsize = AtomicUsize::new(0);
static COMMITS: AtomicUsize = AtomicUsize::new(0);
static ROLLBACKS: AtomicUsize = AtomicUsize::new(0);
static SAVEPOINTS: AtomicUsize = AtomicUsize::new(0);
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn begin(_ds: &str) -> Result<Box<dyn std::any::Any>, CfmlError> {
    BEGINS.fetch_add(1, Ordering::SeqCst);
    Ok(Box::new(0u8))
}
fn commit(_c: &mut Box<dyn std::any::Any>) -> Result<(), CfmlError> {
    COMMITS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
fn rollback(_c: &mut Box<dyn std::any::Any>) -> Result<(), CfmlError> {
    ROLLBACKS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
/// The driver has no savepoints — exactly what DuckDB reports.
fn savepoint(_c: &mut Box<dyn std::any::Any>, _n: &str) -> Result<(), CfmlError> {
    SAVEPOINTS.fetch_add(1, Ordering::SeqCst);
    Err(CfmlError::database(
        "this driver does not support savepoints".to_string(),
    ))
}
fn exec(
    _c: &mut Box<dyn std::any::Any>,
    _sql: &str,
    _p: &CfmlValue,
    _rt: &str,
) -> CfmlResult {
    Ok(CfmlValue::Null)
}

struct Outcome {
    output: String,
    begins: usize,
    commits: usize,
    rollbacks: usize,
    savepoints: usize,
    depth_after: usize,
}

/// Run `script` (CFScript) against the stub driver.
fn run(script: &str) -> Outcome {
    let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    for c in [&BEGINS, &COMMITS, &ROLLBACKS, &SAVEPOINTS] {
        c.store(0, Ordering::SeqCst);
    }
    let mut files = HashMap::new();
    files.insert(
        "index.cfm".to_string(),
        format!("<cfscript>\n{}\n</cfscript>", script).into_bytes(),
    );
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(Arc::from(page_path.as_str()));
    vm.base_template_path = Some(page_path);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.txn_begin = Some(begin);
    vm.txn_commit = Some(commit);
    vm.txn_rollback = Some(rollback);
    vm.txn_savepoint = Some(savepoint);
    vm.txn_execute = Some(exec);

    vm.execute().unwrap();
    Outcome {
        output: vm.get_output(),
        begins: BEGINS.load(Ordering::SeqCst),
        commits: COMMITS.load(Ordering::SeqCst),
        rollbacks: ROLLBACKS.load(Ordering::SeqCst),
        savepoints: SAVEPOINTS.load(Ordering::SeqCst),
        depth_after: vm.transaction_depth,
    }
}

fn compile_page(vfs: &Arc<dyn Vfs>, path: &str) -> BytecodeProgram {
    let source = vfs.read_to_string(path).unwrap();
    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };
    let ast = Parser::new(processed).parse().unwrap();
    CfmlCompiler::new().compile(ast)
}

const INSERT: &str = r#"queryExecute( "insert into t values (1)", [], { datasource = "stub" } );"#;

#[test]
fn a_nested_block_joins_without_a_savepoint_and_commits_once() {
    let o = run(&format!("transaction {{ {INSERT} transaction {{ {INSERT} transaction {{ {INSERT} }} }} }}"));
    assert_eq!(o.savepoints, 0, "a nested block must not take a savepoint");
    assert_eq!((o.begins, o.commits, o.rollbacks), (1, 1, 0));
    assert_eq!(o.depth_after, 0);
}

#[test]
fn nesting_before_the_outer_block_has_a_connection_also_joins() {
    // No statement before the nested block, so the outer one is still pending.
    let o = run(&format!("transaction {{ transaction {{ {INSERT} }} {INSERT} }}"));
    assert_eq!(o.savepoints, 0);
    assert_eq!((o.begins, o.commits, o.rollbacks), (1, 1, 0));
}

#[test]
fn an_exception_caught_inside_the_outer_block_rolls_nothing_back() {
    let o = run(&format!(
        r#"transaction {{ {INSERT}
            try {{ transaction {{ {INSERT} throw( message = "inner" ); }} }} catch ( any e ) {{ writeOutput( "caught" ); }}
            {INSERT} }}"#
    ));
    assert_eq!(o.output, "caught");
    assert_eq!((o.commits, o.rollbacks), (1, 0));
    assert_eq!(o.depth_after, 0);
}

#[test]
fn an_exception_escaping_the_outer_block_rolls_back_once() {
    let o = run(&format!(
        r#"try {{ transaction {{ {INSERT} transaction {{ {INSERT} throw( message = "inner" ); }} }} }} catch ( any e ) {{ writeOutput( e.message ); }}"#
    ));
    assert_eq!(o.output, "inner");
    assert_eq!((o.commits, o.rollbacks), (0, 1));
    assert_eq!(o.depth_after, 0);
}

#[test]
fn an_in_block_rollback_in_a_nested_block_acts_on_the_whole_transaction_and_keeps_it_open() {
    let o = run(&format!(
        r#"transaction {{ {INSERT} transaction {{ {INSERT} transaction action="rollback"; {INSERT} }}
            writeOutput( isWithinTransaction() ); {INSERT} }}
            writeOutput( "," & isWithinTransaction() );"#
    ));
    assert_eq!(o.output, "true,false");
    // One rollback for the action, one commit for the block's end, and the
    // connection was never given up in between.
    assert_eq!((o.begins, o.commits, o.rollbacks), (1, 1, 1));
    assert_eq!(o.depth_after, 0);
}

#[test]
fn actions_and_the_transaction_bifs_do_nothing_outside_a_transaction() {
    let o = run(
        r#"transaction action="commit"; transaction action="rollback"; transactionCommit(); transactionRollback();
           transactionSetSavepoint( "a" ); writeOutput( isWithinTransaction() );"#,
    );
    assert_eq!(o.output, "false");
    assert_eq!((o.begins, o.commits, o.rollbacks, o.savepoints), (0, 0, 0, 0));
}

#[test]
fn an_explicit_savepoint_on_a_driver_without_them_reports_the_driver_refusal() {
    let o = run(&format!(
        r#"try {{ transaction {{ {INSERT} transaction action="setsavepoint" savepoint="a"; }} }} catch ( any e ) {{ writeOutput( e.message ); }}"#
    ));
    assert!(o.output.contains("does not support savepoints"), "got: {}", o.output);
    assert_eq!((o.commits, o.rollbacks), (0, 1));
}

#[test]
fn rolling_back_to_an_unknown_savepoint_is_an_error() {
    let o = run(&format!(
        r#"try {{ transaction {{ {INSERT} transactionRollback( "nope" ); }} }} catch ( any e ) {{ writeOutput( e.message ); }}"#
    ));
    assert_eq!(o.output, "There are no savepoint with name [nope] set");
    assert_eq!((o.commits, o.rollbacks), (0, 1));
}
