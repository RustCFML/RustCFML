//! CFML identifiers are case-insensitive, but the builtin registry is keyed
//! lowerCamel (`len`, `trim`, `structKeyExists`). A call spelled any other way
//! used to fall into an unmemoized chain of `eq_ignore_ascii_case` linear scans
//! over all ~730 builtins — +236..635 ns per call, 2-4x the cost of the call
//! itself, levied on every builtin call in every UpperCamel-writing codebase
//! (Preside, ColdBox, and much CFML in the wild).
//!
//! Resolution now goes through `builtin_names_lc`, a lowercased index carrying
//! each name's canonical registry spelling and fn pointer. These tests lock in
//! that the answer is identical for every casing, on BOTH the armed-index path
//! (what the CLI/serve/worker/wasm embedders take, since they all call
//! `refresh_builtin_index()`) and the deliberately-retained O(n) fallback an
//! embedder gets when it inserts into the public `builtins` field without
//! arming the index. Native (Rust-registered) functions and user functions must
//! be equally casing-blind, and must keep their precedence over builtins.

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::CfmlResult;
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

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

/// Run `src` as `index.cfm` and return its trimmed output. `arm_index` mirrors
/// the embedder that calls `refresh_builtin_index()` after bulk registration;
/// `false` leaves the index unarmed so the O(n) fallback is exercised instead.
/// `register` runs after the stdlib registration, for native-module tests.
fn run(src: &str, arm_index: bool, register: impl FnOnce(&mut CfmlVirtualMachine)) -> String {
    let mut files = HashMap::new();
    files.insert("index.cfm".to_string(), src.as_bytes().to_vec());

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let page_path = format!("{}/index.cfm", VROOT);
    let program = compile_page(&vfs, &page_path);

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    if arm_index {
        vm.refresh_builtin_index();
    }
    register(&mut vm);

    vm.execute().unwrap();
    vm.get_output().trim().to_string()
}

/// Every casing of a lowerCamel-registered builtin resolves to the same fn.
const CASINGS_PAGE: &str = r##"<cfoutput>#len("abcd")#|#Len("abcd")#|#LEN("abcd")#|#lEn("abcd")#
|#trim("  x  ")#|#Trim("  x  ")#|#TRIM("  x  ")#|#tRiM("  x  ")#
|#listLast("a,b,c")#|#ListLast("a,b,c")#|#LISTLAST("a,b,c")#|#lIsTlAsT("a,b,c")#
|#reReplace("a1b2","[0-9]","-","all")#|#ReReplace("a1b2","[0-9]","-","all")#|#REREPLACE("a1b2","[0-9]","-","all")#</cfoutput>"##;

const CASINGS_EXPECTED: &str =
    "4|4|4|4\n|x|x|x|x\n|c|c|c|c\n|a-b-|a-b-|a-b-";

#[test]
fn armed_index_resolves_every_casing() {
    assert_eq!(CASINGS_EXPECTED, run(CASINGS_PAGE, true, |_| {}));
}

#[test]
fn unarmed_index_falls_back_and_resolves_every_casing() {
    // Same answers via the retained O(n) scan — an embedder that forgets
    // `refresh_builtin_index()` must be slow, never wrong.
    assert_eq!(CASINGS_EXPECTED, run(CASINGS_PAGE, false, |_| {}));
}

/// A struct-taking BIF, plus the same names reached as first-class VALUES —
/// the `LoadGlobal` half of the resolution chain, which hands back the shared
/// memoized `Arc` for the canonical name instead of allocating a fresh
/// `CfmlFunction` per miscased read.
///
/// (Only the reference is asserted, not a call through it: invoking a builtin
/// reference held in a variable is a separate, pre-existing gap — `f = arrayLen;
/// f([1,2,3])` fails at the REGISTRY casing too, so it is nothing to do with
/// casing. `isCustomFunction` forces the same resolution without depending on
/// that gap.)
const VALUE_PAGE: &str = r##"<cfscript>
    s = { a = 1 };
    out = [];
    out.append( StructKeyExists( s, "a" ) );
    out.append( STRUCTKEYEXISTS( s, "a" ) );
    out.append( sTrUcTkEyExIsTs( s, "zz" ) );
    f1 = ArrayLen;
    f2 = arraylen;
    f3 = ARRAYLEN;
    out.append( isCustomFunction( f1 ) );
    out.append( isCustomFunction( f2 ) );
    out.append( isCustomFunction( f3 ) );
    writeOutput( arrayToList( out, "|" ) );
</cfscript>"##;

#[test]
fn builtin_as_first_class_value_resolves_every_casing() {
    assert_eq!(
        "true|true|false|true|true|true",
        run(VALUE_PAGE, true, |_| {})
    );
    assert_eq!(
        "true|true|false|true|true|true",
        run(VALUE_PAGE, false, |_| {})
    );
}

fn native_echo(args: Vec<CfmlValue>) -> CfmlResult {
    let s = args
        .first()
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok(CfmlValue::String(format!("native:{}", s).into()))
}

/// `register_native_fn` refreshes the index itself, so a Rust-registered BIF is
/// casing-blind exactly like a stdlib one — including when the registration
/// spelling is the UpperCamel one and the call site is lowercase.
#[test]
fn native_fn_resolves_every_casing() {
    let page = r##"<cfoutput>#myNativeEcho("a")#|#MyNativeEcho("b")#|#MYNATIVEECHO("c")#|#mynativeecho("d")#|#mYnAtIvEeChO("e")#</cfoutput>"##;
    assert_eq!(
        "native:a|native:b|native:c|native:d|native:e",
        run(page, true, |vm| {
            vm.register_native_fn("myNativeEcho", native_echo)
        })
    );

    let upper = r##"<cfoutput>#UpperRegistered("a")#|#upperregistered("b")#|#UPPERREGISTERED("c")#</cfoutput>"##;
    assert_eq!(
        "native:a|native:b|native:c",
        run(upper, true, |vm| {
            vm.register_native_fn("UpperRegistered", native_echo)
        })
    );
}

/// User functions are casing-blind too, and keep their precedence: a scope
/// entry holding a function wins over the user-function table, which wins over
/// the builtin table. Speeding the lookup must not change WHICH one is found.
#[test]
fn user_function_casing_and_precedence() {
    let page = r##"<cfscript>
        function ciHelper( s ) { return "udf:" & arguments.s; }
        out = [];
        out.append( ciHelper( "a" ) );
        out.append( cihelper( "b" ) );
        out.append( CIHELPER( "c" ) );
        out.append( CiHeLpEr( "d" ) );
        // A scope entry holding a function shadows the same-named UDF, whatever
        // casing either side is spelled with.
        CIHELPER = function( s ) { return "scope:" & arguments.s; };
        out.append( ciHelper( "e" ) );
        // A CFC method named after a BIF must NOT steal the bare call: object
        // dispatch keeps the method, the bare name keeps the builtin.
        writeOutput( arrayToList( out, "|" ) );
    </cfscript>"##;
    assert_eq!(
        "udf:a|udf:b|udf:c|udf:d|scope:e",
        run(page, true, |_| {})
    );
}
