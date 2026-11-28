//! An extension calling back into CFML, across real requests.
//!
//! Every request here is a fresh VM sharing one `ServerState`, compiled from
//! CFML source and run through `execute_with_lifecycle` — the way serve mode
//! builds them — with the extension registered into each VM the way the CLI
//! registers a loaded `.rcx`. The extension is written against
//! `rustcfml-module`, exactly as an author would write one. What this file does
//! not cover is the `dlopen` itself (see `foreign_abi.rs`).
//!
//! Three things are under test:
//!
//! - **CFML errors as data (tier 4).** CFML the extension called throws; the
//!   extension can catch it, read it, and handle it or raise it again. An
//!   extension that does not opt in sees exactly the old behaviour.
//! - **Named arguments (tier 4)** — the ABI form of `argumentCollection`.
//! - **`Rooted` values called in a later request** behave as the same value
//!   kept in `application` scope does: captured state shared and mutable, the
//!   dynamic scopes the CALLER's.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_common::dynamic::{CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_module_abi as abi;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{foreign, CfmlVirtualMachine, ServerState};
use rustcfml_module::{module, CfmlException, Ctx, Error, Result, Rooted, Value};

// ---------------------------------------------------------------------------
// The extension
// ---------------------------------------------------------------------------

fn arg(args: &[Value<'_>], i: usize) -> String {
    args.get(i).map(|v| v.to_string()).unwrap_or_default()
}

fn callee<'a>(args: &[Value<'a>]) -> Result<Value<'a>> {
    args.first().copied().ok_or_else(|| Error::new("need a function"))
}

/// One line describing a caught exception, for assertions.
fn describe(e: &CfmlException<'_>) -> String {
    format!(
        "caught|{}|{}|{}|{}|{}|ends={}",
        e.error_type,
        e.message,
        e.detail,
        e.extended_info,
        e.error_code,
        e.ends_request()
    )
}

/// `xCatch( fn, args… )` — the shape the Rig extension needs: a tool that
/// throws becomes data, and the run carries on.
fn x_catch<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.try_call_as_fn(args.get(1..).unwrap_or(&[])) {
        Ok(v) => Ok(v),
        Err(e) => Ok(ctx.string(describe(&e))),
    }
}

static LAST_ERR: Mutex<String> = Mutex::new(String::new());

/// `xIgnore( fn )` — the PRE-tier-4 idiom: catch the plain `Err` and carry on.
/// The error stays pending and is raised anyway, exactly as it always was.
fn x_ignore<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.call_as_fn(&[]) {
        Ok(v) => Ok(v),
        Err(e) => {
            *LAST_ERR.lock().unwrap() = format!("{}|{}|{}", e.kind, e.custom_type, e.message);
            Ok(ctx.string("ignored"))
        }
    }
}

/// `xLastErr()` — what the plain `Err` carried.
fn x_last_err<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Ok(ctx.string(LAST_ERR.lock().unwrap().clone()))
}

/// `xPropagate( fn )` — `?` on the plain call.
fn x_propagate<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    callee(args)?.call_as_fn(&[])
}

/// `xRethrow( fn )` — catch, then raise it again.
fn x_rethrow<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.try_call_as_fn(&[]) {
        Ok(v) => Ok(v),
        Err(e) => Err(e.rethrow()),
    }
}

/// `xQuestion( fn )` — `?` on a `try_` call raises the original too.
fn x_question<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    Ok(callee(args)?.try_call_as_fn(&[])?)
}

/// `xReplace( fn )` — catch, then fail with the extension's own error.
fn x_replace<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.try_call_as_fn(&[]) {
        Ok(v) => Ok(v),
        Err(_) => Err(Error::custom("ext.replaced", "the extension's own error")),
    }
}

/// `xTakeAfter( fn )` — a plain call, then `take_error` to handle it.
fn x_take_after<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.call_as_fn(&[]) {
        Ok(v) => Ok(v),
        Err(_) => match ctx.take_error() {
            Some(e) => Ok(ctx.string(format!("took:{}", e.message))),
            None => Ok(ctx.string("nothing to take")),
        },
    }
}

/// `xInfo( fn )` — the caught exception's full `catch( any e )` struct.
fn x_info<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match callee(args)?.try_call_as_fn(&[]) {
        Ok(_) => Ok(ctx.null()),
        Err(e) => Ok(e.info()),
    }
}

/// `xWriteAfter( fn )` — handle the error, then write page output.
fn x_write_after<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let _ = callee(args)?.try_call_as_fn(&[]);
    ctx.write_output("[after]")?;
    Ok(ctx.null())
}

/// `xTryNew( path )`, `xTryCall( name )`, `xTryInclude( path )`.
fn x_try_new<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let path = arg(args, 0);
    match ctx.catching(|| ctx.new_component(&path, &[])) {
        Ok(_) => Ok(ctx.string("constructed")),
        Err(e) => Ok(ctx.string(describe(&e))),
    }
}

fn x_try_call<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match ctx.try_call(&arg(args, 0), args.get(1..).unwrap_or(&[])) {
        Ok(v) => Ok(v),
        Err(e) => Ok(ctx.string(describe(&e))),
    }
}

fn x_try_include<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let path = arg(args, 0);
    match ctx.catching(|| ctx.include(&path)) {
        Ok(()) => Ok(ctx.string("included")),
        Err(e) => Ok(ctx.string(describe(&e))),
    }
}

/// `xThrow()` — an extension error raised INSIDE CFML another extension call
/// is running, to show a caught exception crossing two extension frames.
fn x_throw<'a>(_ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Err(Error::custom("ext.inner", "raised by an inner extension call"))
}

/// `xNamed( fn, args )`, `xInvokeNamed( obj, method, args )`,
/// `xCatchNamed( fn, args )`.
fn x_named<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let named = args.get(1).copied().ok_or_else(|| Error::new("need args"))?;
    callee(args)?.call_as_fn_named(named)
}

fn x_invoke_named<'a>(_ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let named = args.get(2).copied().ok_or_else(|| Error::new("need args"))?;
    callee(args)?.invoke_named(&arg(args, 1), named)
}

fn x_catch_named<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let named = args.get(1).copied().ok_or_else(|| Error::new("need args"))?;
    let f = callee(args)?;
    match ctx.catching(|| f.call_as_fn_named(named)) {
        Ok(v) => Ok(v),
        Err(e) => Ok(ctx.string(describe(&e))),
    }
}

// ---- rooted values ---------------------------------------------------------

static ROOTED: Mutex<Option<HashMap<String, Rooted>>> = Mutex::new(None);

fn rooted() -> MutexGuard<'static, Option<HashMap<String, Rooted>>> {
    ROOTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// `xRoot( name, value )`
fn x_root<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let v = args.get(1).copied().ok_or_else(|| Error::new("xRoot( name, value )"))?;
    let r = ctx.root(v)?;
    // A replaced root is dropped after the lock is released.
    let previous = rooted().get_or_insert_with(HashMap::new).insert(arg(args, 0).to_lowercase(), r);
    drop(previous);
    Ok(ctx.bool(true))
}

/// Resolve a root in THIS call. The lock is released before any CFML runs, so
/// a closure that calls back into this extension cannot deadlock on it.
fn resolve<'a>(ctx: &'a Ctx, name: &str) -> Result<Value<'a>> {
    let guard = rooted();
    let r = guard
        .as_ref()
        .and_then(|m| m.get(&name.to_lowercase()))
        .ok_or_else(|| Error::new(format!("nothing rooted as [{name}]")))?;
    Ok(r.get(ctx))
}

/// `xRootCall( name, args… )`
fn x_root_call<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    resolve(ctx, &arg(args, 0))?.call_as_fn(args.get(1..).unwrap_or(&[]))
}

/// `xRootInvoke( name, method )`
fn x_root_invoke<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    resolve(ctx, &arg(args, 0))?.invoke(&arg(args, 1), &[])
}

/// `xRootGet( name )`
fn x_root_get<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    resolve(ctx, &arg(args, 0))
}

/// `xUnroot( name )` — drop the extension's root.
fn x_unroot<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let removed = rooted().as_mut().and_then(|m| m.remove(&arg(args, 0).to_lowercase()));
    let existed = removed.is_some();
    drop(removed);
    Ok(ctx.bool(existed))
}

module! {
    name: "cfmlcalls",
    version: "1.0.0",
    tier: rustcfml_module::abi::tier::EXCEPTIONS,
    bifs: {
        "xCatch"        => x_catch,
        "xIgnore"       => x_ignore,
        "xLastErr"      => x_last_err,
        "xPropagate"    => x_propagate,
        "xRethrow"      => x_rethrow,
        "xQuestion"     => x_question,
        "xReplace"      => x_replace,
        "xTakeAfter"    => x_take_after,
        "xInfo"         => x_info,
        "xWriteAfter"   => x_write_after,
        "xTryNew"       => x_try_new,
        "xTryCall"      => x_try_call,
        "xTryInclude"   => x_try_include,
        "xThrow"        => x_throw,
        "xNamed"        => x_named,
        "xInvokeNamed"  => x_invoke_named,
        "xCatchNamed"   => x_catch_named,
        "xRoot"         => x_root,
        "xRootCall"     => x_root_call,
        "xRootInvoke"   => x_root_invoke,
        "xRootGet"      => x_root_get,
        "xUnroot"       => x_unroot,
    },
}

/// Written against the raw ABI, the way an extension built before tier 4
/// behaves: it calls a closure, sees the failure, ignores it and returns a
/// value. It never calls `error_take`, so the CFML error must still be raised.
unsafe extern "C-unwind" fn legacy_ignore(
    raw: *mut abi::Ctx,
    args: *const abi::ValueHandle,
    argc: usize,
) -> abi::ValueHandle {
    let vt = &foreign::HOST_VTABLE;
    assert_eq!(argc, 1);
    let out = (vt.call_value)(raw, *args, std::ptr::null(), 0);
    assert!(out.is_null(), "the closure was meant to throw");
    (vt.new_string)(raw, abi::StrRef::new("ignored"))
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The root table is process-wide and some assertions count it, so the tests
/// in this file take turns.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let leftovers = rooted().take();
    drop(leftovers);
    guard
}

fn load() -> foreign::LoadedModule {
    let mut m = unsafe { foreign::adopt(rustcfml_module_decl(), "cfmlcalls", CfmlValue::Null) }
        .expect("the module should adopt");
    m.bifs.push(foreign::ForeignBuiltin {
        entry: legacy_ignore,
        module: "cfmlcalls",
        name: "xLegacyIgnore",
    });
    // As the CLI does at load: codegen then binds these names at compile time
    // (`CallBuiltin`), the path a real `.rcx` BIF takes. Without it they would
    // go through the generic `Call` op, which isolates the caller's `try`
    // handlers itself and so hides what `CallBuiltin` does not.
    cfml_common::builtins_meta::register_foreign_builtin_names(m.bifs.iter().map(|b| b.name));
    m
}

fn fixtures() -> HashMap<String, Vec<u8>> {
    let mut f = HashMap::new();
    f.insert(
        "Application.cfc".into(),
        br#"component { this.name = "extension-cfml-calls-test"; }"#.to_vec(),
    );
    f.insert(
        "Counter.cfc".into(),
        br#"component {
    this.name = "counter";
    variables.hits = 0;
    function bump() { variables.hits++; return variables.hits; }
    function describe( prefix, suffix = "!" ) { return prefix & this.name & ":" & variables.hits & suffix; }
}"#
        .to_vec(),
    );
    f.insert(
        "Broken.cfc".into(),
        br#"component {
    throw( message = "cannot construct", type = "broken.init" );
}"#
        .to_vec(),
    );
    f.insert(
        "throws.cfm".into(),
        br#"<cfscript>throw( message = "from an included template", type = "inc.fail" );</cfscript>"#
            .to_vec(),
    );
    f
}

/// One application, many requests.
struct App {
    ss: ServerState,
    m: foreign::LoadedModule,
    files: HashMap<String, Vec<u8>>,
    root: &'static str,
}

impl App {
    fn new() -> App {
        App { ss: ServerState::with_production(false), m: load(), files: fixtures(), root: "/app" }
    }

    /// No `Application.cfc`, so no application function table: once a
    /// request's VM is gone, nothing else holds the bytecode it compiled.
    fn without_application() -> App {
        App { ss: ServerState::with_production(false), m: load(), files: HashMap::new(), root: "/noapp" }
    }

    fn run(&self, page: &str, source: &str) -> String {
        self.run_with_url(page, source, &[])
    }

    fn run_with_url(&self, page: &str, source: &str, url: &[(&str, &str)]) -> String {
        let mut files = self.files.clone();
        files.insert(page.to_string(), source.as_bytes().to_vec());
        let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, self.root.to_string()));
        let page_path = format!("{}/{page}", self.root);
        let program = compile(&vfs, &page_path);

        let mut vm = CfmlVirtualMachine::new(program);
        vm.vfs = vfs;
        vm.source_file = Some(Arc::from(page_path.as_str()));
        vm.base_template_path = Some(page_path.clone());
        for (name, value) in get_builtins() {
            vm.globals.insert(name, value);
        }
        for (name, func) in get_builtin_functions() {
            vm.builtins.insert(name, func);
        }
        let url_scope = CfmlStruct::empty();
        for (k, v) in url {
            url_scope.insert(k.to_string(), CfmlValue::string(v.to_string()));
        }
        let cgi = CfmlStruct::empty();
        cgi.insert("script_name".to_string(), CfmlValue::string(format!("/{page}")));
        vm.globals.insert("url".to_string(), CfmlValue::Struct(url_scope));
        vm.globals.insert("cgi".to_string(), CfmlValue::Struct(cgi));
        vm.globals.insert("form".to_string(), CfmlValue::strukt(ValueMap::default()));
        vm.register_foreign_module(&self.m);
        vm.server_state = Some(self.ss.clone());
        match vm.execute_with_lifecycle() {
            Ok(_) => vm.output_buffer.trim().to_string(),
            Err(e) => format!("ERROR: {} || {}", e.message, vm.output_buffer.trim()),
        }
    }
}

fn compile(vfs: &Arc<dyn Vfs>, path: &str) -> BytecodeProgram {
    let source = vfs.read_to_string(path).unwrap();
    let processed =
        if tag_parser::has_cfml_tags(&source) { tag_parser::tags_to_script(&source) } else { source };
    let ast = Parser::new(processed).parse().unwrap();
    CfmlCompiler::new().compile(ast)
}

/// A page that catches whatever escapes `expr` and reports it the way the
/// assertions read it.
fn catching_page(expr: &str) -> String {
    format!(
        r#"<cfscript>
        try {{
            writeOutput( "returned|" & {expr} );
        }} catch ( any e ) {{
            writeOutput( "raised|" & e.type & "|" & e.message & "|" & ( e.detail ?: "" )
                & "|" & ( e.extendedInfo ?: "" ) & "|" & ( e.errorCode ?: "" ) );
        }}
        </cfscript>"#
    )
}

const THROWING_TOOL: &str = r#"function( name ) {
    throw( message = "no data for " & ( name ?: "?" ), type = "my.tool",
        detail = "the lookup came back empty", extendedInfo = "xi", errorCode = "E42" );
}"#;

// ---------------------------------------------------------------------------
// CFML errors as data
// ---------------------------------------------------------------------------

#[test]
fn an_extension_can_catch_a_cfml_error_and_carry_on() {
    // The Rig extension's repro: a tool that throws must become a tool result,
    // and the request must go on.
    let _serial = serial();
    let app = App::new();
    let out = app.run(
        "catch.cfm",
        &catching_page(&format!(r#"xCatch( {THROWING_TOOL}, "Atlantis" ) & "|page continued""#)),
    );
    assert_eq!(
        out,
        "returned|caught|my.tool|no data for Atlantis|the lookup came back empty|xi|E42|ends=false|page continued"
    );
}

#[test]
fn the_old_idiom_still_has_the_error_raised_but_now_knows_what_it_was() {
    // Catching the plain `Err` and returning normally does NOT handle the
    // error — that is unchanged, so an extension built before tier 4 behaves
    // exactly as it did. What is new is that its `Err` names the CFML error
    // instead of saying "[function value] failed".
    let _serial = serial();
    let app = App::new();
    let out = app.run("ignore.cfm", &catching_page(&format!("xIgnore( {THROWING_TOOL} )")));
    assert_eq!(out, "raised|my.tool|no data for ?|the lookup came back empty|xi|E42");
    let last = app.run("last.cfm", r#"<cfscript>writeOutput( xLastErr() );</cfscript>"#);
    assert_eq!(last, format!("{}|my.tool|no data for ?", abi::error_type::CUSTOM));
}

#[test]
fn an_extension_that_never_takes_the_error_behaves_exactly_as_before() {
    // Written against the raw ABI with no knowledge of `error_take`: the host
    // must still raise the staged error when the call returns.
    let _serial = serial();
    let app = App::new();
    let out = app.run("legacy.cfm", &catching_page(&format!("xLegacyIgnore( {THROWING_TOOL} )")));
    assert_eq!(out, "raised|my.tool|no data for ?|the lookup came back empty|xi|E42");
}

#[test]
fn propagating_raises_the_original_error_unchanged() {
    let _serial = serial();
    let app = App::new();
    let direct = app.run("direct.cfm", &catching_page(&format!("( {THROWING_TOOL} )()")));
    let expected = "raised|my.tool|no data for ?|the lookup came back empty|xi|E42";
    assert_eq!(direct, expected, "CFML calling the closure itself");
    for bif in ["xPropagate", "xRethrow", "xQuestion"] {
        let out = app.run("propagate.cfm", &catching_page(&format!("{bif}( {THROWING_TOOL} )")));
        assert_eq!(out, expected, "{bif} must raise exactly what CFML threw");
    }
}

#[test]
fn an_error_raised_through_an_extension_reaches_the_cfml_catch_around_it() {
    // The CFML that called the extension is inside a `try`. The closure the
    // extension calls throws. That error must reach THIS catch — not consume
    // its handler from inside the closure and jump to its address there, which
    // is what happened before the re-entry set the caller's handlers aside:
    // the error vanished and the page carried on as if nothing had happened.
    let _serial = serial();
    let app = App::new();
    let src = format!(
        r#"<cfscript>
        try {{
            xPropagate( {THROWING_TOOL} );
            writeOutput( "NOT RAISED" );
        }} catch ( any e ) {{
            writeOutput( "raised " & e.type );
        }}
        try {{
            xLegacyIgnore( {THROWING_TOOL} );
            writeOutput( "|NOT RAISED" );
        }} catch ( any e ) {{
            writeOutput( "|raised " & e.type );
        }}
        try {{
            writeOutput( "|" & xCatch( {THROWING_TOOL} ) );
        }} catch ( any e ) {{
            writeOutput( "|wrongly raised " & e.type );
        }}
        writeOutput( "|end" );
        </cfscript>"#
    );
    assert_eq!(
        app.run("trycatch.cfm", &src),
        "raised my.tool|raised my.tool|caught|my.tool|no data for ?|the lookup came back empty|xi|E42|ends=false|end"
    );
}

#[test]
fn a_rethrown_error_keeps_its_tag_context() {
    let _serial = serial();
    let app = App::new();
    let src = format!(
        r#"<cfscript>
        try {{ xRethrow( {THROWING_TOOL} ); }}
        catch ( any e ) {{
            writeOutput( arrayLen( e.tagContext ) > 0 );
            writeOutput( "|" & listLast( e.tagContext[ 1 ].template, "/" ) );
        }}
        </cfscript>"#
    );
    assert_eq!(app.run("ctx.cfm", &src), "true|ctx.cfm");
}

#[test]
fn an_extension_may_replace_a_caught_error_with_its_own() {
    let _serial = serial();
    let app = App::new();
    let out = app.run("replace.cfm", &catching_page(&format!("xReplace( {THROWING_TOOL} )")));
    // (`errorCode` is "0" for an error raised without one, as CFML reports it.)
    assert_eq!(out, "raised|ext.replaced|the extension's own error|||0");
}

#[test]
fn take_error_handles_a_failure_after_the_fact() {
    let _serial = serial();
    let app = App::new();
    let out = app.run("take.cfm", &catching_page(&format!("xTakeAfter( {THROWING_TOOL} )")));
    assert_eq!(out, "returned|took:no data for ?");
}

#[test]
fn the_caught_exception_is_the_struct_cfml_catch_would_see() {
    let _serial = serial();
    let app = App::new();
    let src = format!(
        r#"<cfscript>
        e = xInfo( {THROWING_TOOL} );
        writeOutput( e.type & "|" & e.message & "|" & e.detail & "|" & e.errorCode & "|" & e.extendedInfo
            & "|" & isArray( e.tagContext ) & "|" & ( len( e.stackTrace ) > 0 ) & "|" & isStruct( e.rootCause ) );
        </cfscript>"#
    );
    assert_eq!(
        app.run("info.cfm", &src),
        "my.tool|no data for ?|the lookup came back empty|E42|xi|true|true|true"
    );
}

#[test]
fn an_abort_cannot_be_swallowed() {
    // `abort` is not an exception — `catch( any )` cannot stop it in CFML, and
    // an extension must not be able to keep an aborted request running.
    let _serial = serial();
    let app = App::new();
    let src = r#"<cfscript>
        writeOutput( "before|" );
        writeOutput( xCatch( function() { abort; } ) );
        writeOutput( "|after" );
        </cfscript>"#;
    let out = app.run("abort.cfm", src);
    assert!(out.starts_with("before|"), "got: {out}");
    assert!(!out.contains("after"), "the request must stop at the abort: {out}");
}

#[test]
fn output_capture_left_open_by_the_failure_is_closed_at_the_boundary() {
    // The closure opens a savecontent and throws inside it. A CFML catch puts
    // the output stack back; so must an extension that handles the error, or
    // everything written afterwards vanishes into the dead capture buffer.
    let _serial = serial();
    let app = App::new();
    let cfml = app.run(
        "cfml.cfm",
        r#"<cfscript>
        try { savecontent variable="x" { writeOutput( "[inside]" ); throw( "boom" ); } }
        catch ( any e ) {}
        writeOutput( "[after]" );
        </cfscript>"#,
    );
    assert_eq!(cfml, "[after]", "the CFML reference behaviour");
    let ext = app.run(
        "ext.cfm",
        r#"<cfscript>
        xWriteAfter( function() {
            savecontent variable="x" { writeOutput( "[inside]" ); throw( "boom" ); }
        } );
        writeOutput( "[page]" );
        </cfscript>"#,
    );
    assert_eq!(ext, "[after][page]");
}

#[test]
fn errors_from_every_kind_of_call_into_cfml_can_be_caught() {
    let _serial = serial();
    let app = App::new();
    assert_eq!(
        app.run("new.cfm", r#"<cfscript>writeOutput( xTryNew( "Broken" ) );</cfscript>"#),
        "caught|broken.init|cannot construct||||ends=false"
    );
    let missing = app.run("missing.cfm", r#"<cfscript>writeOutput( xTryNew( "NoSuchThing" ) );</cfscript>"#);
    assert!(missing.starts_with("caught|") && missing.contains("NoSuchThing"), "got: {missing}");
    let nofn = app.run("nofn.cfm", r#"<cfscript>writeOutput( xTryCall( "noSuchFunction" ) );</cfscript>"#);
    assert!(nofn.starts_with("caught|") && nofn.contains("noSuchFunction"), "got: {nofn}");
    assert_eq!(
        app.run("inc.cfm", r#"<cfscript>writeOutput( xTryInclude( "throws.cfm" ) );</cfscript>"#),
        "caught|inc.fail|from an included template||||ends=false"
    );
    // A successful call through the same entry points is untouched.
    assert_eq!(app.run("ok.cfm", r#"<cfscript>writeOutput( xTryCall( "ucase", "fine" ) );</cfscript>"#), "FINE");
}

#[test]
fn an_error_from_a_nested_extension_call_is_caught_by_the_outer_one() {
    // CFML → xCatch → closure → xThrow (a second extension frame) → error.
    let _serial = serial();
    let app = App::new();
    assert_eq!(
        app.run("nested.cfm", r#"<cfscript>writeOutput( xCatch( function() { return xThrow(); } ) );</cfscript>"#),
        "caught|ext.inner|raised by an inner extension call|||0|ends=false"
    );
}

// ---------------------------------------------------------------------------
// Named arguments
// ---------------------------------------------------------------------------

#[test]
fn named_arguments_bind_by_parameter_name() {
    let _serial = serial();
    let app = App::new();
    let src = r#"<cfscript>
        f = function( city, days = 1 ) {
            return city & ":" & days & ":" & ( arguments.extra ?: "-" ) & ":" & structCount( arguments );
        };
        writeOutput( xNamed( f, { days = 3, city = "Paris" } ) );
        writeOutput( "|" & xNamed( f, { city = "Rome" } ) );
        writeOutput( "|" & xNamed( f, { city = "Oslo", extra = "x" } ) );
        writeOutput( "|" & xNamed( f, { "1" = "Lima", "2" = 9 } ) );
        writeOutput( "|" & xNamed( f, [ "Kyiv", 4 ] ) );
        </cfscript>"#;
    let out = app.run("named.cfm", src);
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "Paris:3:-:2", "{out}");
    assert_eq!(parts[1], "Rome:1:-:2", "an omitted optional takes its default: {out}");
    assert_eq!(parts[2], "Oslo:1:x:3", "an unmatched key lands in arguments: {out}");
    assert_eq!(parts[3], "Lima:9:-:2", "integer keys are positional: {out}");
    assert_eq!(parts[4], "Kyiv:4:-:2", "an array is positional: {out}");
}

#[test]
fn a_method_can_be_invoked_with_named_arguments() {
    let _serial = serial();
    let app = App::new();
    let src = r#"<cfscript>
        o = new Counter();
        o.bump();
        writeOutput( xInvokeNamed( o, "describe", { suffix = "?", prefix = ">" } ) );
        writeOutput( "|" & xInvokeNamed( o, "describe", { prefix = "=" } ) );
        writeOutput( "|" & ( invoke( o, "describe", { suffix = "?", prefix = ">" } )
            == xInvokeNamed( o, "describe", { suffix = "?", prefix = ">" } ) ) );
        </cfscript>"#;
    assert_eq!(app.run("invoke.cfm", src), ">counter:1?|=counter:1!|true");
}

#[test]
fn a_named_call_that_throws_can_be_caught() {
    let _serial = serial();
    let app = App::new();
    assert_eq!(
        app.run("named.cfm", &format!(r#"<cfscript>writeOutput( xCatchNamed( {THROWING_TOOL}, {{ name = "Mars" }} ) );</cfscript>"#)),
        "caught|my.tool|no data for Mars|the lookup came back empty|xi|E42|ends=false"
    );
    // Named arguments need a user-defined function; a built-in is refused
    // clearly rather than bound by guesswork.
    let builtin = app.run(
        "builtin.cfm",
        r#"<cfscript>writeOutput( xCatchNamed( ucase, { string = "x" } ) );</cfscript>"#,
    );
    assert!(builtin.starts_with("caught|") && builtin.contains("named arguments"), "got: {builtin}");
}

// ---------------------------------------------------------------------------
// Rooted values, called in later requests
// ---------------------------------------------------------------------------

const MAKE_COUNTER: &str = r#"
function makeCounter() {
    var n = 0;
    return function( by = 1 ) { n += by; return n; };
}
"#;

#[test]
fn a_rooted_closure_called_later_shares_its_captures_with_application_scope() {
    let _serial = serial();
    let app = App::new();
    let define = format!(
        r#"<cfscript>{MAKE_COUNTER}
        c = makeCounter();
        application.counter = c;
        xRoot( "counter", c );
        writeOutput( xRootCall( "counter" ) & "," & c() & "," & isDefined( "n" ) );
        </cfscript>"#
    );
    assert_eq!(app.run("define.cfm", &define), "1,2,false");
    // The extension's copy and application's copy are one closure with one
    // captured scope: each sees the other's increments, request after request.
    let later = r#"<cfscript>
        writeOutput( xRootCall( "counter" ) & "," & application.counter() & "," & xRootCall( "counter", 10 )
            & "," & isDefined( "n" ) );
        </cfscript>"#;
    assert_eq!(app.run("later.cfm", later), "3,4,14,false");
    assert_eq!(app.run("later.cfm", later), "15,16,26,false");
}

#[test]
fn a_rooted_closure_reads_the_calling_requests_scopes() {
    let _serial = serial();
    let app = App::new();
    let define = r#"<cfscript>
        request.marker = "defining";
        application.tag = "app-one";
        xRoot( "scopes", function() {
            return request.marker & "|" & url.who & "|" & cgi.script_name & "|" & application.tag;
        } );
        writeOutput( xRootCall( "scopes" ) );
        </cfscript>"#;
    assert_eq!(app.run_with_url("define.cfm", define, &[("who", "definer")]), "defining|definer|/define.cfm|app-one");

    let later = r#"<cfscript>
        request.marker = "calling";
        application.tag = "app-two";
        application.scopesFn = xRootGet( "scopes" );
        writeOutput( xRootCall( "scopes" ) & "||" & application.scopesFn() );
        </cfscript>"#;
    assert_eq!(
        app.run_with_url("call.cfm", later, &[("who", "caller")]),
        "calling|caller|/call.cfm|app-two||calling|caller|/call.cfm|app-two",
        "request, url and cgi are the CALLER's, as they are for application scope"
    );
}

#[test]
fn page_level_captures_behave_as_they_do_in_application_scope() {
    // A closure defined at PAGE level captures the page's `variables`. Lucee
    // keeps that scope alive with the closure; RustCFML clears a page's closure
    // environment when the page ends (docs/known-issues.md). Whichever the
    // engine does, a rooted closure must do the same as application scope.
    let _serial = serial();
    let app = App::new();
    let define = r#"<cfscript>
        pageVar = 0;
        f = function() { pageVar++; return pageVar; };
        application.pageFn = f;
        xRoot( "pageFn", f );
        writeOutput( xRootCall( "pageFn" ) & "," & application.pageFn() );
        </cfscript>"#;
    assert_eq!(app.run("define.cfm", define), "1,2");
    let probe = |expr: &str| app.run("probe.cfm", &catching_page(expr));
    let via_app = probe("application.pageFn()");
    let via_root = probe(r#"xRootCall( "pageFn" )"#);
    assert_eq!(via_root, via_app, "rooted and application-scope copies agree");
}

#[test]
fn a_rooted_component_keeps_its_variables_and_this() {
    let _serial = serial();
    let app = App::new();
    let define = r#"<cfscript>
        o = new Counter();
        o.bump();
        xRoot( "comp", o );
        writeOutput( xRootInvoke( "comp", "bump" ) );
        </cfscript>"#;
    assert_eq!(app.run("define.cfm", define), "2");
    let later = r#"<cfscript>
        writeOutput( xRootInvoke( "comp", "bump" ) & ";" & xRootGet( "comp" ).describe( "" ) & ";"
            & xRootGet( "comp" ).bump() );
        </cfscript>"#;
    assert_eq!(app.run("later.cfm", later), "3;counter:3!;4");
}

#[test]
fn a_rooted_closure_still_runs_after_the_request_that_compiled_it_is_gone() {
    // No application, and each request compiles its page fresh: once the
    // defining request's VM is dropped, nothing but the root refers to the
    // closure's bytecode. Lucee's stored UDF keeps its compiled class; a root
    // must keep the body too.
    let _serial = serial();
    let app = App::without_application();
    let define = format!(
        r#"<cfscript>{MAKE_COUNTER}
        xRoot( "counter", makeCounter() );
        writeOutput( xRootCall( "counter" ) );
        </cfscript>"#
    );
    assert_eq!(app.run("define.cfm", &define), "1");
    let later = r#"<cfscript>writeOutput( xRootCall( "counter" ) );</cfscript>"#;
    assert_eq!(app.run("later.cfm", later), "2");
    assert_eq!(app.run("later.cfm", later), "3");
}

#[test]
fn dropping_a_root_releases_it() {
    let _serial = serial();
    let app = App::new();
    let base = foreign::rooted_count();
    app.run(
        "keep.cfm",
        r#"<cfscript>for ( i = 1; i <= 20; i++ ) { xRoot( "k" & i, function() { return 1; } ); }</cfscript>"#,
    );
    assert_eq!(foreign::rooted_count(), base + 20);
    app.run("drop.cfm", r#"<cfscript>for ( i = 1; i <= 20; i++ ) { xUnroot( "k" & i ); }</cfscript>"#);
    assert_eq!(foreign::rooted_count(), base);
}

#[test]
fn many_requests_root_call_and_release_at_once() {
    let _serial = serial();
    let app = Arc::new(App::new());
    let base = foreign::rooted_count();
    // One shared counter every request increments, under a named lock taken
    // by the CALLER so the final total is exact.
    app.run(
        "define.cfm",
        &format!(
            r#"<cfscript>{MAKE_COUNTER}
            xRoot( "shared", makeCounter() );
            </cfscript>"#
        ),
    );
    const THREADS: usize = 8;
    const REQUESTS: usize = 4;
    const PER_REQUEST: usize = 25;
    let failures = Arc::new(AtomicUsize::new(0));
    let workers: Vec<_> = (0..THREADS)
        .map(|t| {
            let app = Arc::clone(&app);
            let failures = Arc::clone(&failures);
            std::thread::spawn(move || {
                for r in 0..REQUESTS {
                    let src = format!(
                        r#"<cfscript>
                        ok = 0;
                        for ( i = 1; i <= {PER_REQUEST}; i++ ) {{
                            key = "t{t}_r{r}_" & i;
                            xRoot( key, function( x ) {{ return x * 2; }} );
                            if ( xRootCall( key, i ) == i * 2 ) {{ ok++; }}
                            lock name="rooted-stress" type="exclusive" timeout="30" {{
                                xRootCall( "shared" );
                            }}
                            xUnroot( key );
                        }}
                        writeOutput( ok );
                        </cfscript>"#
                    );
                    let out = app.run(&format!("stress_{t}_{r}.cfm"), &src);
                    if out != PER_REQUEST.to_string() {
                        eprintln!("thread {t} request {r}: {out}");
                        failures.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();
    for w in workers {
        w.join().expect("a worker panicked");
    }
    assert_eq!(failures.load(Ordering::SeqCst), 0, "every call answered correctly");
    let total = app.run("total.cfm", r#"<cfscript>writeOutput( xRootCall( "shared" ) );</cfscript>"#);
    assert_eq!(total, (THREADS * REQUESTS * PER_REQUEST + 1).to_string(), "no update lost");
    assert_eq!(foreign::rooted_count(), base + 1);
    app.run("drop.cfm", r#"<cfscript>xUnroot( "shared" );</cfscript>"#);
    assert_eq!(foreign::rooted_count(), base);
}

// ---------------------------------------------------------------------------
// Compatibility
// ---------------------------------------------------------------------------

fn decl_requiring(tier: u32) -> abi::ModuleDecl {
    abi::ModuleDecl {
        size: std::mem::size_of::<abi::ModuleDecl>(),
        abi_major: abi::ABI_MAJOR,
        tier,
        name: abi::StrRef::new("tiered"),
        version: abi::StrRef::new("0.0.1"),
        target: abi::StrRef::new(abi::TARGET),
        on_load: None,
        bifs: std::ptr::null(),
        bif_count: 0,
        classes: std::ptr::null(),
        class_count: 0,
        qoq_fns: std::ptr::null(),
        qoq_fn_count: 0,
        drivers: std::ptr::null(),
        driver_count: 0,
    }
}

#[test]
fn extensions_built_for_any_earlier_tier_still_load() {
    for tier in [abi::tier::VALUES, abi::tier::SCOPES, abi::tier::EXECUTION, abi::tier::EXCEPTIONS] {
        let decl = decl_requiring(tier);
        if let Err(e) = unsafe { foreign::adopt(&decl, "tiered.rcx", CfmlValue::Null) } {
            panic!("a tier-{tier} extension must load: {e}");
        }
    }
    assert_eq!(foreign::HOST_VTABLE.tier, abi::tier::EXCEPTIONS);
    assert_eq!(foreign::HOST_VTABLE.size, std::mem::size_of::<abi::HostVtable>());
}

#[test]
fn an_extension_needing_a_newer_tier_is_refused_by_name() {
    let decl = decl_requiring(abi::tier::EXCEPTIONS + 1);
    match unsafe { foreign::adopt(&decl, "future.rcx", CfmlValue::Null) } {
        Ok(_) => panic!("a host must refuse a tier it does not implement"),
        Err(e) => assert!(
            e.contains("requires extension capability tier 5 but this engine implements tier 4"),
            "{e}"
        ),
    }
}

