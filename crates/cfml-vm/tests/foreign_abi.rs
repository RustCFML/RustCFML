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

use cfml_common::dynamic::{CfmlArray, CfmlQuery, CfmlStruct, CfmlValue};
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

module! {
    name: "abitest",
    version: "9.9.9",
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
    },
    classes: { Tally },
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn load() -> foreign::LoadedModule {
    unsafe { foreign::adopt(rustcfml_module_decl(), "abitest").expect("module should adopt") }
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
    assert_eq!(m.bifs.len(), 11);
    assert_eq!(m.classes.len(), 1);
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
