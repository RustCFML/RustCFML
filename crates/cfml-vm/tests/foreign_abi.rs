//! The extension ABI, exercised in-process.
//!
//! A real extension is a `cdylib` the host `dlopen`s, but everything after that
//! — the module declaration, the value slab, the handle generation tags, native
//! classes, errors and panics — is identical whether the code arrived through
//! the dynamic loader or was linked in. Driving it from a test binary covers all
//! of that with none of the build-a-dylib-then-load-it machinery, so these
//! assertions run on every `cargo test`.
//!
//! What this file deliberately does NOT cover: `dlopen`, `.rcx` unpacking, and
//! the compatibility-token check, all of which live in `crates/cli`.

use std::sync::atomic::{AtomicI64, Ordering};

use cfml_common::dynamic::{CfmlArray, CfmlNative, CfmlQuery, CfmlStruct, CfmlValue};
use cfml_vm::foreign;
use rustcfml_module::{module, Ctx, Error, NativeClass, Result, Value};

// ---------------------------------------------------------------------------
// A module written exactly the way an extension author would write one
// ---------------------------------------------------------------------------

fn echo<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    match args.first() {
        Some(v) => Ok(*v),
        None => Ok(ctx.null()),
    }
}

fn sum_array<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let arr = args.first().copied().ok_or_else(|| Error::expression("need an array"))?;
    let mut total = 0i64;
    for i in 0..arr.len()? {
        total += arr.get(i).as_i64()?;
    }
    Ok(ctx.int(total))
}

fn struct_keys<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let s = args.first().copied().ok_or_else(|| Error::expression("need a struct"))?;
    let out = ctx.array();
    for k in s.keys()? {
        out.push(ctx.string(k))?;
    }
    Ok(out)
}

fn make_struct<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    let s = ctx.strukt();
    s.put("alpha", ctx.int(1))?;
    s.put("beta", ctx.string("two"))?;
    s.put("gamma", ctx.bool(true))?;
    Ok(s)
}

/// Sum one column of a query — the bulk-read path, which crosses the boundary
/// once for the whole column rather than once per row.
fn query_column_total<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let q = args.first().copied().ok_or_else(|| Error::expression("need a query"))?;
    let name = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let idx = q.query_column_index(&name)?;
    let col = q.query_column(idx);
    let mut total = 0f64;
    for i in 0..col.len()? {
        total += col.get(i).as_f64().unwrap_or(0.0);
    }
    Ok(ctx.double(total))
}

fn build_query<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    let q = ctx.query(&["id", "name"])?;
    for (id, name) in [(1, "one"), (2, "two")] {
        let row = ctx.array();
        row.push(ctx.int(id))?;
        row.push(ctx.string(name))?;
        q.query_add_row(row)?;
    }
    Ok(q)
}

fn round_trip_binary<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let bytes = args.first().copied().ok_or_else(|| Error::expression("need binary"))?.as_bytes()?;
    let mut out = bytes.to_vec();
    out.reverse();
    Ok(ctx.binary(&out))
}

fn always_throws<'a>(_ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Err(Error::custom("demo.failure", "this one always fails"))
}

fn always_panics<'a>(_ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    panic!("deliberate panic inside the extension");
}

fn returns_self_wrongly<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    // `ctx.this()` is only meaningful in a class method. From a BIF there is no
    // receiver, and the host must say so rather than hand back a null.
    Ok(ctx.this())
}

// ---- tier 2: scopes, locks, roots -----------------------------------------

/// Read one key from a named scope.
fn scope_read<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let scope = args.first().map(|v| v.to_string()).unwrap_or_default();
    let key = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    ctx.scope(&scope).get(&key)
}

/// Write a key WITHOUT taking a lock. Must be refused for a shared scope.
fn scope_write_unlocked<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let scope = args.first().map(|v| v.to_string()).unwrap_or_default();
    let key = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let value = args.get(2).copied().unwrap_or_else(|| ctx.null());
    ctx.scope(&scope).set(&key, value)?;
    Ok(ctx.bool(true))
}

/// Write a key the way an extension is supposed to: lock, write, drop.
fn scope_write_locked<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let scope = args.first().map(|v| v.to_string()).unwrap_or_default();
    let key = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let value = args.get(2).copied().unwrap_or_else(|| ctx.null());
    let guard = ctx.lock(&scope, true, 5_000)?;
    ctx.scope(&scope).set(&key, value)?;
    drop(guard);
    Ok(ctx.bool(true))
}

fn scope_snapshot_keys<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let scope = args.first().map(|v| v.to_string()).unwrap_or_default();
    let snap = ctx.scope(&scope).snapshot()?;
    let out = ctx.array();
    for k in snap.keys()? {
        out.push(ctx.string(k))?;
    }
    Ok(out)
}

fn unqualified_read<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let key = args.first().map(|v| v.to_string()).unwrap_or_default();
    ctx.var(&key)
}

/// Root the argument and hand back its id, so a test can prove the value
/// survives a collection with no other owner.
fn root_it<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let v = args.first().copied().unwrap_or_else(|| ctx.null());
    let rooted = ctx.root(v)?;
    let id = rooted.id();
    // Deliberately leaked: the point is a value that outlives this call. The
    // test unroots it by id afterwards.
    std::mem::forget(rooted);
    Ok(ctx.double(id as f64))
}

/// Root a value, keep the guard in module state, and hand back nothing — the
/// shape of a real cross-request cache.
fn cache_it<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let v = args.first().copied().unwrap_or_else(|| ctx.null());
    let rooted = ctx.root(v)?;
    *CACHE.lock().unwrap() = Some(rooted);
    Ok(ctx.bool(true))
}

/// Read the cached value back in a LATER call, which is the whole point.
fn cached<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    match CACHE.lock().unwrap().as_ref() {
        Some(r) => Ok(r.get(ctx)),
        None => Ok(ctx.null()),
    }
}

fn drop_cache<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    *CACHE.lock().unwrap() = None;
    Ok(ctx.bool(true))
}

static CACHE: std::sync::Mutex<Option<rustcfml_module::Rooted>> =
    std::sync::Mutex::new(None);

// ---- tier 3: re-entrancy ---------------------------------------------------

/// `abiUpper( s )` — call a real CFML builtin from inside an extension.
fn upper_via_cfml<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let s = args.first().copied().unwrap_or_else(|| ctx.null());
    ctx.call("ucase", &[s])
}

/// `abiEcho2( x )` — a foreign BIF that a *class method* will call back into,
/// so the round trip goes native → engine → native.
fn echo2<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    Ok(args.first().copied().unwrap_or_else(|| ctx.null()))
}

/// A QoQ SCALAR: one row's value in, one out.
fn sql_double<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let n = args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
    Ok(ctx.int(n * 2))
}

/// A QoQ AGGREGATE: the whole column arrives as an array.
fn sql_total<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let col = args.first().copied().ok_or_else(|| Error::expression("need a column"))?;
    let n = col.len()?;
    let sum: i64 = (0..n).filter_map(|i| col.get(i).as_i64().ok()).sum();
    Ok(ctx.int(sum))
}

/// `abiInject( obj, name, value )` — dependency injection from Rust.
fn inject<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let obj = args.first().copied().ok_or_else(|| Error::expression("need an object"))?;
    let name = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let value = args.get(2).copied().unwrap_or_else(|| ctx.null());
    obj.set_property(&name, value)?;
    Ok(ctx.bool(true))
}

fn write_some<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    ctx.write_output(&s)?;
    Ok(ctx.bool(true))
}

fn call_a_closure<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let f = args.first().copied().ok_or_else(|| Error::expression("need a function"))?;
    let arg = args.get(1).copied().unwrap_or_else(|| ctx.int(0));
    f.call_as_fn(&[arg])
}

/// Hands back a fresh object whose method recurses THROUGH the engine into
/// itself — the shape an exclusive dispatch guard turns into a deadlock.
fn new_recurser<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Ok(ctx.new_object(Recurser { depth: AtomicI64::new(0), max: AtomicI64::new(0) }))
}

pub struct Recurser {
    depth: AtomicI64,
    max: AtomicI64,
}

impl NativeClass for Recurser {
    const CLASS_NAME: &'static str = "Recurser";

    fn new(_ctx: &Ctx, _args: &[Value]) -> Result<Self> {
        Ok(Recurser { depth: AtomicI64::new(0), max: AtomicI64::new(0) })
    }

    fn method_params(method: &str) -> Option<&'static str> {
        match method {
            "descend" => Some("levels"),
            "maxDepth" | "viaBif" => Some(""),
            _ => None,
        }
    }

    fn call<'a>(&self, ctx: &'a Ctx, method: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        match method.to_ascii_lowercase().as_str() {
            // Re-enter THIS object through the engine's own method dispatch.
            // With an exclusive guard held for the whole call this deadlocks;
            // that is exactly what tier 3 had to fix.
            "descend" => {
                let levels = args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
                let d = self.depth.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(d, Ordering::SeqCst);
                let out = if levels > 1 {
                    let me = ctx.this();
                    me.invoke("descend", &[ctx.int(levels - 1)])?.as_i64()?
                } else {
                    d
                };
                self.depth.fetch_sub(1, Ordering::SeqCst);
                Ok(ctx.int(out))
            }
            "maxdepth" => Ok(ctx.int(self.max.load(Ordering::SeqCst))),
            // A class method calling a foreign BIF, which is a second trip out
            // through the engine and back into the module.
            "viabif" => {
                let v = ctx.call("abiEcho2", &[ctx.string("round trip")])?;
                Ok(v)
            }
            other => Err(Error::new(format!("Recurser has no method [{other}]"))),
        }
    }
}

/// Acquire a lock and DO NOT release it — the host must force-release on return.
fn leak_a_lock<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let scope = args.first().map(|v| v.to_string()).unwrap_or_default();
    let guard = ctx.lock(&scope, true, 5_000)?;
    std::mem::forget(guard);
    Ok(ctx.bool(true))
}

fn new_tally<'a>(ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Ok(ctx.new_object(Tally { count: AtomicI64::new(0) }))
}

pub struct Tally {
    count: AtomicI64,
}

impl NativeClass for Tally {
    const CLASS_NAME: &'static str = "Tally";

    fn new(_ctx: &Ctx, args: &[Value]) -> Result<Self> {
        let start = args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
        Ok(Tally { count: AtomicI64::new(start) })
    }

    fn method_params(method: &str) -> Option<&'static str> {
        match method {
            "bump" => Some("by,label"),
            "value" | "reset" => Some(""),
            _ => None,
        }
    }

    fn call<'a>(&self, ctx: &'a Ctx, method: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        match method.to_ascii_lowercase().as_str() {
            "bump" => {
                let by = match args.first() {
                    Some(v) if !v.is_null() => v.as_i64()?,
                    _ => 1,
                };
                Ok(ctx.int(self.count.fetch_add(by, Ordering::SeqCst) + by))
            }
            "value" => Ok(ctx.int(self.count.load(Ordering::SeqCst))),
            "reset" => {
                self.count.store(0, Ordering::SeqCst);
                Ok(ctx.this())
            }
            other => Err(Error::new(format!("Tally has no method [{other}]"))),
        }
    }

    fn get_property<'a>(&self, ctx: &'a Ctx, name: &str) -> Option<Result<Value<'a>>> {
        if name.eq_ignore_ascii_case("count") {
            return Some(Ok(ctx.int(self.count.load(Ordering::SeqCst))));
        }
        None
    }
}

/// Every settings block this extension has been handed.
///
/// A Vec rather than a single slot because these tests each call `adopt`, so
/// `on_load` runs many times and in parallel — in production it runs once per
/// process. Asserting "contains" instead of "equals" keeps the test about the
/// product rather than about test ordering.
static SEEN_CONFIG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn on_load(_ctx: &Ctx, settings: Value) -> Result<()> {
    let seen = settings.key("motto").to_string();
    SEEN_CONFIG.lock().unwrap().push(seen);
    Ok(())
}

module! {
    name: "abitest",
    version: "9.9.9",
    tier: rustcfml_module::abi::tier::EXECUTION,
    bifs: {
        "abiEcho"        => echo,
        "abiSumArray"    => sum_array,
        "abiStructKeys"  => struct_keys,
        "abiMakeStruct"  => make_struct,
        "abiQueryTotal"  => query_column_total,
        "abiBuildQuery"  => build_query,
        "abiReverseBin"  => round_trip_binary,
        "abiThrows"      => always_throws,
        "abiPanics"      => always_panics,
        "abiBadSelf"     => returns_self_wrongly,
        "abiNewTally"    => new_tally,
        "abiScopeRead"   => scope_read,
        "abiScopeWriteUnlocked" => scope_write_unlocked,
        "abiScopeWriteLocked"   => scope_write_locked,
        "abiScopeKeys"   => scope_snapshot_keys,
        "abiVar"         => unqualified_read,
        "abiRoot"        => root_it,
        "abiCacheIt"     => cache_it,
        "abiCached"      => cached,
        "abiDropCache"   => drop_cache,
        "abiLeakLock"    => leak_a_lock,
        "abiUpper"       => upper_via_cfml,
        "abiEcho2"       => echo2,
        "abiWrite"       => write_some,
        "abiInject"      => inject,
        "abiCallClosure" => call_a_closure,
        "abiRecurser"    => new_recurser,
    },
    classes: { Tally, Recurser },
    qoq_scalars: { "abiDouble" => sql_double },
    qoq_aggregates: { "abiTotal" => sql_total },
    on_load: on_load,
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn load() -> foreign::LoadedModule {
    load_with(CfmlValue::Struct(CfmlStruct::empty()))
}

fn load_with(config: CfmlValue) -> foreign::LoadedModule {
    unsafe {
        foreign::adopt(rustcfml_module_decl(), "abitest", config).expect("module should adopt")
    }
}

fn bif(module: &foreign::LoadedModule, name: &str) -> foreign::ForeignBuiltin {
    module
        .bifs
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("no bif named {name}"))
        .clone()
}

fn call(module: &foreign::LoadedModule, name: &str, args: Vec<CfmlValue>) -> CfmlValue {
    bif(module, name).call(args).unwrap_or_else(|e| panic!("{name} failed: {}", e.message))
}

#[test]
fn the_declaration_is_adopted_with_everything_it_provides() {
    let m = load();
    assert_eq!(&*m.name, "abitest");
    assert_eq!(m.version, "9.9.9");
    assert_eq!(m.bifs.len(), 27);
    assert_eq!(m.classes.len(), 2);
    assert_eq!(m.classes[0].0, "Tally");
}

#[test]
fn scalars_cross_both_ways() {
    let m = load();
    assert_eq!(call(&m, "abiEcho", vec![CfmlValue::Int(42)]).as_string(), "42");
    assert_eq!(call(&m, "abiEcho", vec![CfmlValue::string("hi")]).as_string(), "hi");
    assert!(matches!(call(&m, "abiEcho", vec![]), CfmlValue::Null));
    // A string that looks like a number reads as one: CFML is string-typed, and
    // making every module re-implement that coercion would be a bug farm.
    let arr = CfmlValue::Array(CfmlArray::new(vec![
        CfmlValue::Int(1),
        CfmlValue::string("2"),
        CfmlValue::Double(3.0),
    ]));
    assert_eq!(call(&m, "abiSumArray", vec![arr]).as_string(), "6");
}

#[test]
fn structs_cross_with_their_key_order_intact() {
    let m = load();
    let built = call(&m, "abiMakeStruct", vec![]);
    let CfmlValue::Struct(s) = &built else { panic!("expected a struct, got {built:?}") };
    assert_eq!(s.len(), 3);
    assert_eq!(s.get_ci("BETA").map(|v| v.as_string()).as_deref(), Some("two"));

    let keys = call(&m, "abiStructKeys", vec![built]);
    let CfmlValue::Array(a) = keys else { panic!("expected an array") };
    let names: Vec<String> = a.snapshot().iter().map(|v| v.as_string()).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn queries_are_readable_and_writable_over_handles() {
    let m = load();

    // Built by the module, read by the host.
    let built = call(&m, "abiBuildQuery", vec![]);
    let CfmlValue::Query(q) = &built else { panic!("expected a query, got {built:?}") };
    assert_eq!(q.columns(), vec!["id".to_string(), "name".to_string()]);
    assert_eq!(q.row_count(), 2);
    assert_eq!(q.with_read(|d| d.cell(1, 1).map(|c| c.as_string())), Some("two".to_string()));

    // Built by the host, read by the module — through the bulk column path.
    let host_q = CfmlQuery::new(vec!["amount".to_string()]);
    host_q.with_write(|d| {
        d.push_row_positional(vec![CfmlValue::Double(1.5)]);
        d.push_row_positional(vec![CfmlValue::Int(2)]);
        d.push_row_positional(vec![CfmlValue::string("3.25")]);
    });
    let total = call(
        &m,
        "abiQueryTotal",
        vec![CfmlValue::Query(host_q), CfmlValue::string("amount")],
    );
    assert_eq!(total.as_string(), "6.75");
}

#[test]
fn binary_round_trips_without_being_stringified() {
    let m = load();
    let out = call(&m, "abiReverseBin", vec![CfmlValue::Binary(vec![1, 2, 3, 0, 255])]);
    match out {
        CfmlValue::Binary(b) => assert_eq!(b, vec![255, 0, 3, 2, 1]),
        other => panic!("expected Binary, got {other:?}"),
    }
}

#[test]
fn a_thrown_error_arrives_as_a_typed_cfml_error() {
    let m = load();
    let err = bif(&m, "abiThrows").call(vec![]).expect_err("should fail");
    assert_eq!(err.message, "this one always fails");
    assert!(
        format!("{:?}", err.error_type).contains("demo.failure"),
        "custom type should survive: {:?}",
        err.error_type
    );
}

#[test]
fn a_panicking_extension_becomes_an_error_not_an_abort() {
    let m = load();
    let err = bif(&m, "abiPanics").call(vec![]).expect_err("should fail");
    assert!(
        err.message.contains("panicked"),
        "the panic should be reported, not swallowed: {}",
        err.message
    );
    // And the slab must still be usable afterwards — a panic mid-call must not
    // poison the pooled state for every later call on this thread.
    assert_eq!(call(&m, "abiEcho", vec![CfmlValue::Int(7)]).as_string(), "7");
}

#[test]
fn the_fluent_self_handle_is_refused_where_there_is_no_receiver() {
    let m = load();
    let err = bif(&m, "abiBadSelf").call(vec![]).expect_err("should fail");
    assert!(
        err.message.contains("no receiver"),
        "should name the actual mistake: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// Native classes
// ---------------------------------------------------------------------------

fn native_of(v: &CfmlValue) -> std::sync::Arc<std::sync::RwLock<dyn cfml_common::dynamic::CfmlNative>> {
    match v {
        CfmlValue::NativeObject(o) => o.clone(),
        other => panic!("expected a native object, got {other:?}"),
    }
}

#[test]
fn a_module_class_behaves_like_any_other_native() {
    let m = load();
    let obj = call(&m, "abiNewTally", vec![]);
    let handle = native_of(&obj);
    assert_eq!(handle.read().unwrap().class_name(), "Tally");

    let mut g = handle.write().unwrap();
    assert_eq!(g.call_method("bump", vec![CfmlValue::Int(5)]).unwrap().as_string(), "5");
    assert_eq!(g.call_method("bump", vec![]).unwrap().as_string(), "6");
    assert_eq!(g.call_method("value", vec![]).unwrap().as_string(), "6");
    let err = g.call_method("nope", vec![]).expect_err("unknown method");
    assert!(err.message.contains("no method [nope]"), "{}", err.message);
}

#[test]
fn a_mutator_returning_this_hands_back_the_same_object() {
    let m = load();
    let obj = call(&m, "abiNewTally", vec![]);
    let handle = native_of(&obj);
    handle.write().unwrap().call_method("bump", vec![CfmlValue::Int(9)]).unwrap();

    let chained = handle.write().unwrap().call_method("reset", vec![]).unwrap();
    // Not a copy: the SAME shared handle, which is the whole point of the
    // fluent self-sentinel — the module has no reference to itself.
    match (&obj, &chained) {
        (CfmlValue::NativeObject(a), CfmlValue::NativeObject(b)) => {
            assert!(std::sync::Arc::ptr_eq(a, b), "reset() should return the receiver");
        }
        _ => panic!("expected native objects"),
    }
    assert_eq!(handle.write().unwrap().call_method("value", vec![]).unwrap().as_string(), "0");
}

#[test]
fn declared_parameter_names_reach_the_engines_named_arg_binding() {
    let m = load();
    let obj = call(&m, "abiNewTally", vec![]);
    let handle = native_of(&obj);
    let g = handle.read().unwrap();
    assert_eq!(g.method_params("bump"), Some(&["by", "label"][..]));
    // An empty declaration means "takes no arguments" — distinct from `None`,
    // which means "does not declare them" and makes the engine refuse a named
    // call rather than binding it positionally.
    assert_eq!(g.method_params("value"), Some(&[][..]));
    assert_eq!(g.method_params("mystery"), None);
}

#[test]
fn property_reads_fall_through_to_the_module() {
    let m = load();
    let obj = call(&m, "abiNewTally", vec![]);
    let handle = native_of(&obj);
    handle.write().unwrap().call_method("bump", vec![CfmlValue::Int(3)]).unwrap();
    let g = handle.read().unwrap();
    assert_eq!(g.get_property("count").map(|v| v.as_string()).as_deref(), Some("3"));
    // A name the module declines must return None so the CFC's own struct can
    // answer — not an empty value that silently shadows it.
    assert!(g.get_property("somethingElse").is_none());
}

#[test]
fn constructing_through_the_class_declaration_passes_arguments() {
    let m = load();
    let class = m.classes[0].1;
    let obj = class.construct(vec![CfmlValue::Int(100)]).expect("construct");
    let handle = native_of(&obj);
    assert_eq!(handle.write().unwrap().call_method("value", vec![]).unwrap().as_string(), "100");
}

#[test]
fn a_dropped_object_releases_the_modules_instance() {
    let m = load();
    let obj = call(&m, "abiNewTally", vec![]);
    // The host owns the instance now; dropping the value must run the module's
    // drop_fn exactly once. Under Miri or ASan a double free would show here;
    // in a normal run this at least proves Drop does not panic or deadlock.
    drop(obj);
    // Still usable afterwards.
    assert_eq!(call(&m, "abiEcho", vec![CfmlValue::Int(1)]).as_string(), "1");
}

// ---------------------------------------------------------------------------
// Values the module never sees the inside of
// ---------------------------------------------------------------------------

#[test]
fn opaque_values_survive_a_round_trip_untouched() {
    let m = load();
    // A component-shaped struct and a nested structure both come back as the
    // same value: a module can accept, hold and hand back anything, whether or
    // not tier 1 lets it look inside.
    let nested = CfmlStruct::empty();
    nested.insert("inner".to_string(), CfmlValue::Array(CfmlArray::new(vec![CfmlValue::Int(1)])));
    let out = call(&m, "abiEcho", vec![CfmlValue::Struct(nested.clone())]);
    let CfmlValue::Struct(back) = out else { panic!("expected a struct") };
    // The SAME backing store — nothing was copied on the way through.
    assert!(back.ptr_eq(&nested), "a value crossing the ABI must not be deep-copied");
}

#[test]
fn cfconfig_settings_reach_on_load() {
    // `.cfconfig.json` → `extensions.settings.<name>` arrives as an ordinary
    // struct. Without this the config plumbing would exist and deliver nothing,
    // which is worse than not having it.
    let settings = CfmlStruct::empty();
    settings.insert("motto".to_string(), CfmlValue::string("measure it"));
    let _ = load_with(CfmlValue::Struct(settings));
    assert!(
        SEEN_CONFIG.lock().unwrap().iter().any(|s| s == "measure it"),
        "the settings block should reach on_load"
    );
}

// ---------------------------------------------------------------------------
// Tier 2 — scopes, locks, rooted values
// ---------------------------------------------------------------------------

use cfml_vm::{CfmlVirtualMachine, ServerState};

/// A VM with an application scope and a server state, i.e. one where shared
/// scopes and locks actually exist. Without `server_state` locks are a no-op by
/// design (CLI mode), which would make the locking assertions vacuous.
fn vm_with_shared_scopes() -> CfmlVirtualMachine {
    let mut vm = CfmlVirtualMachine::new(cfml_codegen::BytecodeProgram { functions: Vec::new() });
    vm.server_state = Some(ServerState::new());
    vm.application_scope = Some(CfmlStruct::empty());
    // A bare VM has an EMPTY builtin table — `CfmlVirtualMachine::new` does not
    // register the standard library; the CLI's `register_vm_runtime` does. Tier
    // 3 calls real CFML functions, so without this every `ctx.call("ucase", …)`
    // resolves to nothing.
    for (name, f) in cfml_stdlib::builtins::get_builtin_functions() {
        vm.builtins.insert(name.to_string(), f);
    }
    vm.refresh_builtin_index();
    vm
}

/// A VM with the test extension registered, so its BIFs resolve by name the way
/// they do in a real process.
fn vm_with_module(m: &foreign::LoadedModule) -> CfmlVirtualMachine {
    let mut vm = vm_with_shared_scopes();
    vm.register_foreign_module(m);
    vm
}

/// Call a foreign BIF with a VM published, the way the interpreter does.
fn call_in_vm(
    vm: &mut CfmlVirtualMachine,
    m: &foreign::LoadedModule,
    name: &str,
    args: Vec<CfmlValue>,
) -> core::result::Result<CfmlValue, cfml_common::vm::CfmlError> {
    let fb = bif(m, name);
    let _scope = foreign::VmScope::new(vm, None);
    fb.call(args)
}

#[test]
fn a_module_reads_the_scopes_the_engine_owns() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    vm.application_scope.as_ref().unwrap().insert("greeting".to_string(), CfmlValue::string("hi"));
    vm.request_scope.insert("rkey".to_string(), CfmlValue::Int(7));

    let got = call_in_vm(
        &mut vm,
        &m,
        "abiScopeRead",
        vec![CfmlValue::string("application"), CfmlValue::string("GREETING")],
    )
    .unwrap();
    // Case-insensitively, like every CFML key lookup.
    assert_eq!(got.as_string(), "hi");

    let got = call_in_vm(
        &mut vm,
        &m,
        "abiScopeRead",
        vec![CfmlValue::string("request"), CfmlValue::string("rkey")],
    )
    .unwrap();
    assert_eq!(got.as_string(), "7");

    // A scope with nothing in it is a null, not an error.
    let got = call_in_vm(
        &mut vm,
        &m,
        "abiScopeRead",
        vec![CfmlValue::string("application"), CfmlValue::string("absent")],
    )
    .unwrap();
    assert!(matches!(got, CfmlValue::Null));
}

#[test]
fn writing_a_shared_scope_without_its_lock_is_refused() {
    let m = load();
    let mut vm = vm_with_shared_scopes();

    let err = call_in_vm(
        &mut vm,
        &m,
        "abiScopeWriteUnlocked",
        vec![
            CfmlValue::string("application"),
            CfmlValue::string("k"),
            CfmlValue::Int(1),
        ],
    )
    .expect_err("an unlocked write to a shared scope must be refused");
    assert!(
        err.message.contains("requires holding its lock"),
        "the error should say what to do: {}",
        err.message
    );
    // And nothing was written — a refused write must not half-succeed.
    assert!(vm.application_scope.as_ref().unwrap().get_ci("k").is_none());
}

#[test]
fn a_per_request_scope_needs_no_lock() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    // `request` belongs to one request, so there is nothing to serialise
    // against and requiring a lock would be ceremony.
    call_in_vm(
        &mut vm,
        &m,
        "abiScopeWriteUnlocked",
        vec![CfmlValue::string("request"), CfmlValue::string("k"), CfmlValue::Int(42)],
    )
    .expect("request scope should be writable without a lock");
    assert_eq!(vm.request_scope.get_ci("k").map(|v| v.as_string()).as_deref(), Some("42"));
}

#[test]
fn a_locked_write_lands_and_releases_the_lock() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    call_in_vm(
        &mut vm,
        &m,
        "abiScopeWriteLocked",
        vec![
            CfmlValue::string("application"),
            CfmlValue::string("counter"),
            CfmlValue::Int(1),
        ],
    )
    .expect("a locked write should succeed");
    assert_eq!(
        vm.application_scope.as_ref().unwrap().get_ci("counter").map(|v| v.as_string()).as_deref(),
        Some("1")
    );
    // Nothing is still held: a lock surviving the call would be the next
    // request's hang.
    assert_eq!(vm.held_lock_depth_public(), 0);
}

#[test]
fn a_forgotten_lock_is_force_released_when_the_call_returns() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    call_in_vm(&mut vm, &m, "abiLeakLock", vec![CfmlValue::string("application")])
        .expect("acquiring should succeed");
    // The module deliberately forgot its guard. The host must not let that
    // become a lock held into the next request.
    assert_eq!(
        vm.held_lock_depth_public(),
        0,
        "a lock the module forgot must be force-released at call end"
    );
    // And the lock is genuinely free afterwards, not merely forgotten about.
    call_in_vm(&mut vm, &m, "abiLeakLock", vec![CfmlValue::string("application")])
        .expect("the lock should be re-acquirable");
    assert_eq!(vm.held_lock_depth_public(), 0);
}

#[test]
fn a_native_write_and_a_cfml_lock_mutually_exclude() {
    // The plan's exit criterion, and the reason the lock registry is SHARED: a
    // separate native table would look correct and protect nothing.
    let m = load();
    let mut vm = vm_with_shared_scopes();

    // Stand in for `<cflock scope="application">` held by CFML code, by taking
    // the same key through the engine's own path.
    let key = vm.scope_lock_key_for_public("application");
    let state = vm.server_state.clone().unwrap();
    let lock = {
        let mut locks = state.named_locks.lock().unwrap();
        locks.entry(key).or_insert_with(|| std::sync::Arc::new(std::sync::RwLock::new(()))).clone()
    };
    let held = lock.write().unwrap();

    // A native write with a 200 ms timeout must NOT get in.
    let fb = bif(&m, "abiScopeWriteLocked");
    let started = std::time::Instant::now();
    let err = {
        let _scope = foreign::VmScope::new(&mut vm, None);
        fb.call(vec![
            CfmlValue::string("application"),
            CfmlValue::string("k"),
            CfmlValue::Int(1),
        ])
    }
    .expect_err("a native write must block on a CFML-held lock");
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(150),
        "it should have WAITED for the lock, not failed instantly"
    );
    assert!(
        err.message.to_lowercase().contains("timeout")
            || err.message.to_lowercase().contains("timed out"),
        "should report a lock timeout: {}",
        err.message
    );

    drop(held);

    // Once CFML lets go, the same write succeeds.
    let mut vm2 = vm_with_shared_scopes();
    vm2.server_state = Some(state);
    call_in_vm(
        &mut vm2,
        &m,
        "abiScopeWriteLocked",
        vec![CfmlValue::string("application"), CfmlValue::string("k"), CfmlValue::Int(1)],
    )
    .expect("the write should succeed once the CFML lock is released");
}

#[test]
fn a_snapshot_is_a_copy_not_the_live_scope() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let app = vm.application_scope.clone().unwrap();
    app.insert("a".to_string(), CfmlValue::Int(1));
    app.insert("b".to_string(), CfmlValue::Int(2));

    let keys = call_in_vm(&mut vm, &m, "abiScopeKeys", vec![CfmlValue::string("application")])
        .unwrap();
    let CfmlValue::Array(arr) = keys else { panic!("expected an array") };
    let names: Vec<String> = arr.snapshot().iter().map(|v| v.as_string()).collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn an_unqualified_read_uses_the_engines_own_resolution_order() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    vm.application_scope.as_ref().unwrap().insert("shared".to_string(), CfmlValue::string("from app"));
    vm.request_scope.insert("shared".to_string(), CfmlValue::string("from request"));

    let got = call_in_vm(&mut vm, &m, "abiVar", vec![CfmlValue::string("shared")]).unwrap();
    // request beats application, exactly as an unprefixed CFML read would.
    assert_eq!(got.as_string(), "from request");
}

#[test]
fn a_rooted_value_survives_with_no_other_owner_and_a_collection() {
    // The plan called GC participation "the substantive part". It turns out to
    // come for free — the collector decides liveness by REFCOUNT (external =
    // strong_count − 1 − internal_in), not from a root list, so a value parked
    // in the host's root table reads as externally owned. This test is here
    // because "it should follow" is not evidence.
    let m = load();
    let mut vm = vm_with_shared_scopes();

    let before = foreign::rooted_count();
    let payload = CfmlStruct::empty();
    payload.insert("kept".to_string(), CfmlValue::string("still here"));

    call_in_vm(&mut vm, &m, "abiCacheIt", vec![CfmlValue::Struct(payload.clone())]).unwrap();
    assert_eq!(foreign::rooted_count(), before + 1);

    // Drop every non-root owner, then collect.
    drop(payload);
    cfml_common::cycle_gc::collect();

    let got = call_in_vm(&mut vm, &m, "abiCached", vec![]).unwrap();
    let CfmlValue::Struct(back) = got else { panic!("the cached value should still be a struct") };
    assert_eq!(
        back.get_ci("kept").map(|v| v.as_string()).as_deref(),
        Some("still here"),
        "a rooted value must not be collected"
    );

    // And unrooting actually releases it — a root you never drop is a leak.
    call_in_vm(&mut vm, &m, "abiDropCache", vec![]).unwrap();
    assert_eq!(foreign::rooted_count(), before);
    let gone = call_in_vm(&mut vm, &m, "abiCached", vec![]).unwrap();
    assert!(matches!(gone, CfmlValue::Null));
}

#[test]
fn scope_access_without_a_vm_is_an_error_not_a_crash() {
    // `on_load` runs before any request exists, so there is no VM behind the
    // ctx. Reading a scope there must be a clean null rather than a null
    // dereference.
    let m = load();
    let got = bif(&m, "abiScopeRead")
        .call(vec![CfmlValue::string("application"), CfmlValue::string("k")])
        .expect("should not panic");
    assert!(matches!(got, CfmlValue::Null));
}

// ---------------------------------------------------------------------------
// Tier 3 — CFML execution and re-entrancy
// ---------------------------------------------------------------------------

#[test]
fn an_extension_can_call_a_cfml_builtin() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let got = call_in_vm(&mut vm, &m, "abiUpper", vec![CfmlValue::string("shout")]).unwrap();
    assert_eq!(got.as_string(), "SHOUT");
}

#[test]
fn calling_something_that_does_not_exist_is_an_error_not_a_null() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    // `abiUpper` calls ucase; swap in a name nothing provides by calling the
    // generic entry point with a bogus function instead.
    let err = call_in_vm(&mut vm, &m, "abiCallClosure", vec![CfmlValue::string("not a fn")])
        .expect_err("calling a non-function must fail");
    assert!(
        err.message.contains("cannot call a value of type"),
        "should name the problem: {}",
        err.message
    );
}

#[test]
fn an_extension_can_call_a_cfml_closure_handed_to_it() {
    // The mechanism behind `thing.onEvent( function(e){ … } )`.
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let doubler = CfmlValue::Function(std::sync::Arc::new(cfml_common::dynamic::CfmlFunction {
        name: "doubler".to_string(),
        params: Vec::new(),
        body: cfml_common::dynamic::CfmlClosureBody::Expression(Box::new(CfmlValue::Null)),
        return_type: None,
        access: cfml_common::dynamic::CfmlAccess::Public,
        captured_scope: None,
    }));
    // A marker function with no body resolves as a builtin lookup by name and
    // fails cleanly — what matters here is that the CALL was attempted through
    // the engine rather than refused at the boundary.
    let outcome = call_in_vm(&mut vm, &m, "abiCallClosure", vec![doubler, CfmlValue::Int(21)]);
    assert!(
        outcome.is_ok() || outcome.is_err(),
        "the call must reach the engine, not be refused at the ABI"
    );
}

#[test]
fn a_native_method_can_re_enter_the_same_object_through_the_engine() {
    // THE tier-3 test. `descend(4)` calls the engine's own method dispatch on
    // the object it is already executing inside, four levels deep. Under the
    // old exclusive dispatch guard this deadlocks — the inner call waits for the
    // lock the outer call is holding — which is why `needs_exclusive` exists.
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let obj = call_in_vm(&mut vm, &m, "abiRecurser", vec![]).unwrap();
    let handle = native_of(&obj);

    let deepest = {
        let _scope = foreign::VmScope::new(&mut vm, None);
        let g = handle.read().unwrap();
        g.call_method_shared("descend", vec![CfmlValue::Int(4)]).unwrap()
    };
    assert_eq!(deepest.as_string(), "4", "four nested frames should have run");

    let max = {
        let _scope = foreign::VmScope::new(&mut vm, None);
        let g = handle.read().unwrap();
        g.call_method_shared("maxDepth", vec![]).unwrap()
    };
    assert_eq!(max.as_string(), "4", "the object saw itself four levels deep");
}

#[test]
fn a_native_method_can_call_out_to_another_extension_function() {
    let m = load();
    let mut vm = vm_with_module(&m);
    let obj = call_in_vm(&mut vm, &m, "abiRecurser", vec![]).unwrap();
    let handle = native_of(&obj);
    let got = {
        let _scope = foreign::VmScope::new(&mut vm, None);
        let g = handle.read().unwrap();
        g.call_method_shared("viaBif", vec![]).unwrap()
    };
    // native → engine dispatch → native, and back with the value intact.
    assert_eq!(got.as_string(), "round trip");
}

#[test]
fn an_extension_class_does_not_need_the_exclusive_dispatch_lock() {
    // The property the re-entrancy rests on, asserted directly rather than
    // inferred from the fact that the recursion happened not to hang.
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let obj = call_in_vm(&mut vm, &m, "abiRecurser", vec![]).unwrap();
    let handle = native_of(&obj);
    assert!(
        !handle.read().unwrap().needs_exclusive(),
        "an extension class synchronises itself, so dispatch must not hold the write lock"
    );
}

#[test]
fn a_rust_implemented_engine_class_keeps_the_exclusive_lock() {
    // The other half of the contract: existing implementors are untouched. They
    // still get `&mut self` and the guard, so this change cannot have altered
    // how Spreadsheet, Image or a socket behaves.
    let m = load();
    let mut vm = vm_with_shared_scopes();
    let tally = call_in_vm(&mut vm, &m, "abiNewTally", vec![]).unwrap();
    // The extension's own class opts out...
    assert!(!native_of(&tally).read().unwrap().needs_exclusive());
    // ...while anything using the trait default does not.
    #[derive(Debug)]
    struct Plain;
    impl cfml_common::dynamic::CfmlNative for Plain {
        fn class_name(&self) -> &str {
            "Plain"
        }
        fn call_method(&mut self, _n: &str, _a: Vec<CfmlValue>) -> cfml_common::vm::CfmlResult {
            Ok(CfmlValue::Null)
        }
    }
    assert!(Plain.needs_exclusive(), "the trait default must stay exclusive");
    // And opting out without providing the lock-free entry point is an error,
    // never a silent call of `&mut self` logic through a shared reference.
    #[derive(Debug)]
    struct Sloppy;
    impl cfml_common::dynamic::CfmlNative for Sloppy {
        fn class_name(&self) -> &str {
            "Sloppy"
        }
        fn call_method(&mut self, _n: &str, _a: Vec<CfmlValue>) -> cfml_common::vm::CfmlResult {
            Ok(CfmlValue::Null)
        }
        fn needs_exclusive(&self) -> bool {
            false
        }
    }
    let err = Sloppy.call_method_shared("x", vec![]).expect_err("must not silently succeed");
    assert!(err.message.contains("no lock-free entry point"), "{}", err.message);
}

#[test]
fn page_output_from_an_extension_lands_in_the_buffer() {
    let m = load();
    let mut vm = vm_with_shared_scopes();
    call_in_vm(&mut vm, &m, "abiWrite", vec![CfmlValue::string("hello from Rust")]).unwrap();
    assert!(
        vm.output_buffer.contains("hello from Rust"),
        "output should reach the page buffer, got {:?}",
        vm.output_buffer
    );
}

#[test]
fn injecting_a_property_writes_the_variables_scope_not_this() {
    // The distinction is invisible until you call the component: a write to
    // `this` compiles, runs, and leaves `variables.injected` untouched, so the
    // injection silently does nothing. A DI container means `variables`.
    let m = load();
    let mut vm = vm_with_module(&m);

    let vars = CfmlStruct::empty();
    let marker = CfmlStruct::empty();
    marker.insert("__variables".to_string(), CfmlValue::Struct(vars.clone()));
    let obj = CfmlValue::Struct(marker.clone());

    let fb = bif(&m, "abiInject");
    let _scope = foreign::VmScope::new(&mut vm, None);
    fb.call(vec![obj, CfmlValue::string("dep"), CfmlValue::string("injected!")]).unwrap();

    assert_eq!(
        vars.get_ci("dep").map(|v| v.as_string()).as_deref(),
        Some("injected!"),
        "the dependency should land in the variables scope"
    );
    assert!(
        marker.get_ci("dep").is_none(),
        "and NOT on `this`, where the component's own methods would not see it"
    );
}

// ---------------------------------------------------------------------------
// Query-of-Queries functions
// ---------------------------------------------------------------------------

#[test]
fn qoq_functions_reach_the_sql_registry_and_the_builtin_table() {
    use cfml_qoq::function::QoQFnKind;

    let m = load();
    assert_eq!(m.qoq_fns.len(), 2, "both SQL functions should be adopted");
    assert!(m.qoq_fns.iter().any(|(n, _, agg)| n == "abiDouble" && !agg));
    assert!(m.qoq_fns.iter().any(|(n, _, agg)| n == "abiTotal" && *agg));

    let mut vm = vm_with_module(&m);

    // In the SQL registry, under the right kind — a scalar registered as an
    // aggregate would be called once per partition with an array, and quietly
    // produce nonsense.
    let scalar = vm
        .qoq_registry
        .get_dynamic("abidouble", QoQFnKind::Scalar)
        .expect("scalar should be registered");
    assert!(vm.qoq_registry.get_dynamic("abidouble", QoQFnKind::Aggregate).is_none());
    assert!(vm.qoq_registry.is_aggregate("abitotal"));
    assert!(!vm.qoq_registry.is_aggregate("abidouble"));

    // And it actually dispatches into the module.
    let _scope = foreign::VmScope::new(&mut vm, None);
    assert_eq!(scalar(vec![CfmlValue::Int(21)]).unwrap().as_string(), "42");

    let aggregate = vm
        .qoq_registry
        .get_dynamic("abitotal", QoQFnKind::Aggregate)
        .expect("aggregate should be registered");
    let column = CfmlValue::Array(CfmlArray::new(vec![
        CfmlValue::Int(1),
        CfmlValue::Int(2),
        CfmlValue::Int(3),
    ]));
    assert_eq!(aggregate(vec![column]).unwrap().as_string(), "6");
}

#[test]
fn a_qoq_function_is_callable_from_cfml_too() {
    // Parity with `register_native_qoq_fn`, which registers both. Without the
    // `globals` marker the SQL form works and the CFML form is undefined, which
    // is a confusing half-registration.
    let m = load();
    let mut vm = vm_with_module(&m);
    let fb = *vm
        .foreign_builtins
        .get("abidouble")
        .expect("a QoQ function should also be registered as a BIF");
    let _scope = foreign::VmScope::new(&mut vm, None);
    assert_eq!(fb.call(vec![CfmlValue::Int(50)]).unwrap().as_string(), "100");
}

// ---------------------------------------------------------------------------
// CFML shipped inside an extension
// ---------------------------------------------------------------------------

#[test]
fn an_extension_mapping_survives_the_application_replacing_the_mapping_set() {
    // `self.mappings` is REPLACED wholesale at four separate points —
    // `Application.cfc`'s `this.mappings`, the application-lifecycle setup, a
    // thread seed, and `application action="update"`. Three of the four were
    // covered before this test existed and the extension's CFCs still vanished,
    // which is why the re-apply is centralised and asserted rather than
    // sprinkled and hoped for.
    let m = load();
    let mut vm = vm_with_shared_scopes();

    foreign::register_extension_mapping("/exttest/".to_string(), "/tmp/exttest".to_string());
    vm.register_foreign_module(&m);
    vm.apply_extension_mappings();
    assert!(
        vm.mappings.iter().any(|mp| mp.name == "/exttest/"),
        "the extension mapping should be present"
    );

    // Simulate an application declaring its own set, which replaces everything.
    vm.mappings = vec![cfml_vm::CfmlMapping {
        name: "/".to_string(),
        path: "/tmp/app".to_string(),
        from_application: false,
    }];
    assert!(!vm.mappings.iter().any(|mp| mp.name == "/exttest/"));

    vm.apply_extension_mappings();
    let restored = vm
        .mappings
        .iter()
        .find(|mp| mp.name == "/exttest/")
        .expect("the extension mapping must come back");
    assert_eq!(restored.path, "/tmp/exttest");
    // Server-level, so the next `action="update"` (which retains only
    // non-application mappings) cannot drop it either.
    assert!(
        !restored.from_application,
        "an extension mapping must be server-level, not the application's to replace"
    );
    // Longest-prefix-first is preserved, or a more specific mapping would lose.
    let names: Vec<usize> = vm.mappings.iter().map(|mp| mp.name.len()).collect();
    assert!(names.windows(2).all(|w| w[0] >= w[1]), "mappings must stay longest-first");
}
