//! A worked `.rcx` extension: every shape of the tier-1 value surface, plus a
//! stateful class.
//!
//! ```sh
//! rustcfml ext build examples/native_extension_demo
//! rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
//! rustcfml examples/native_extension_demo/demo.cfm
//! ```
//!
//! Nothing here links the engine. The only RustCFML dependency is
//! `rustcfml-module`, which is a safe wrapper over a C ABI — which is why this
//! crate does not need an engine checkout, a matching rustc, or the engine's
//! allocator.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use rustcfml_module::{abi, module, Ctx, Error, NativeClass, Result, Value};

/// `demoGreet( [name] )` — the smallest useful thing.
fn greet<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let who = match args.first() {
        Some(v) if !v.is_null() => v.to_string(),
        _ => "World".to_string(),
    };
    let hello = GREETING.get().map(String::as_str).unwrap_or("Hello");
    Ok(ctx.string(format!("{hello}, {who}, from Rust")))
}

/// `demoStats( array )` — reading a container one element at a time.
///
/// Each `get` is one crossing. That is the deal the handle model makes: you pay
/// for what you touch, and the 10,000-element array you *didn't* touch cost
/// nothing to receive.
fn stats<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let arr = args
        .first()
        .copied()
        .filter(|v| v.kind() == abi::ty::ARRAY)
        .ok_or_else(|| Error::expression("demoStats() takes an array of numbers"))?;
    let n = arr.len()?;
    if n == 0 {
        return Err(Error::expression("demoStats(): the array is empty"));
    }
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for i in 0..n {
        let v = arr.get(i).as_f64().map_err(|_| {
            Error::expression(format!("demoStats(): element {} is not a number", i + 1))
        })?;
        sum += v;
        min = min.min(v);
        max = max.max(v);
    }
    let out = ctx.strukt();
    out.put("count", ctx.int(n as i64))?;
    out.put("sum", ctx.double(sum))?;
    out.put("mean", ctx.double(sum / n as f64))?;
    out.put("min", ctx.double(min))?;
    out.put("max", ctx.double(max))?;
    Ok(out)
}

/// `demoSummariseQuery( query, columnName )` — the bulk-read path.
///
/// `query_column` materialises a whole column in ONE crossing. Looping
/// `query_cell` instead would cost one crossing per row, which on a real report
/// is the difference between three calls and thirty thousand.
fn summarise_query<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let q = args
        .first()
        .copied()
        .filter(|v| v.kind() == abi::ty::QUERY)
        .ok_or_else(|| Error::expression("demoSummariseQuery() takes a query"))?;
    let column = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let idx = q
        .query_column_index(&column)
        .map_err(|_| Error::expression(format!("no column named [{column}]")))?;

    let col = q.query_column(idx);
    let n = col.len()?;
    let mut total = 0.0;
    for i in 0..n {
        total += col.get(i).as_f64().unwrap_or(0.0);
    }
    let out = ctx.strukt();
    out.put("column", ctx.string(&column))?;
    out.put("rows", ctx.int(n as i64))?;
    out.put("total", ctx.double(total))?;
    out.put("columns", {
        let names = ctx.array();
        for name in q.query_columns()? {
            names.push(ctx.string(name))?;
        }
        names
    })?;
    Ok(out)
}

/// `demoBuildQuery( rows )` — a query built entirely on the module side.
fn build_query<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let rows = args.first().map(|v| v.as_i64().unwrap_or(3)).unwrap_or(3).clamp(0, 10_000);
    let q = ctx.query(&["n", "square"])?;
    for n in 1..=rows {
        let row = ctx.array();
        row.push(ctx.int(n))?;
        row.push(ctx.int(n * n))?;
        q.query_add_row(row)?;
    }
    Ok(q)
}

/// `demoChecksum( binary )` — binary in, integer out, nothing stringified.
fn checksum<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let v = args
        .first()
        .copied()
        .ok_or_else(|| Error::expression("demoChecksum() takes a binary value"))?;
    let bytes = v.as_bytes().map_err(|_| Error::expression("demoChecksum() takes a binary value"))?;
    // FNV-1a: short, dependency-free, and enough to prove the bytes arrived.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(ctx.string(format!("{hash:016x}")))
}

/// `demoFail()` — a typed error, so `<cfcatch type="demo.deliberate">` works.
fn fail<'a>(_ctx: &'a Ctx, _args: &[Value<'a>]) -> Result<Value<'a>> {
    Err(Error::custom("demo.deliberate", "this function always fails, on purpose"))
}

/// `demoTally( [start] )` — a class instance from a function, so callers can
/// write `demoTally()` rather than `createObject( "rust", "Tally" )`.
fn new_tally<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let start = args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
    Ok(ctx.new_object(Tally::at(start)))
}

/// A counter with a label — enough state to show both the `&self` contract and
/// the fluent self-handle.
pub struct Tally {
    count: AtomicI64,
    label: Mutex<String>,
}

impl Tally {
    fn at(start: i64) -> Tally {
        Tally { count: AtomicI64::new(start), label: Mutex::new(String::new()) }
    }
}

impl NativeClass for Tally {
    const CLASS_NAME: &'static str = "Tally";

    fn new(_ctx: &Ctx, args: &[Value]) -> Result<Self> {
        Ok(Tally::at(args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0)))
    }

    /// Declared so `t.bump( by = 5 )` binds by name. Without this the engine
    /// REFUSES named arguments for the method — deliberately, because binding
    /// them by call-site order instead is a silent wrong answer.
    fn method_params(method: &str) -> Option<&'static str> {
        Some(match method {
            "bump" => "by",
            "label" => "text",
            "value" | "reset" | "describe" => "",
            _ => return None,
        })
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
            // Mutators return `ctx.this()`, which the engine resolves to the
            // receiver — the module has no handle to itself, and the engine
            // does. That is what makes `.reset().bump()` chain onto the SAME
            // object rather than a copy of it.
            "reset" => {
                self.count.store(0, Ordering::SeqCst);
                Ok(ctx.this())
            }
            "label" => {
                let text = args.first().map(|v| v.to_string()).unwrap_or_default();
                *self.label.lock().unwrap_or_else(|e| e.into_inner()) = text;
                Ok(ctx.this())
            }
            "describe" => {
                let label = self.label.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let n = self.count.load(Ordering::SeqCst);
                Ok(ctx.string(if label.is_empty() {
                    format!("{n}")
                } else {
                    format!("{label}: {n}")
                }))
            }
            other => Err(Error::new(format!("Tally has no method [{other}]"))),
        }
    }

    /// `this.count` on a CFC that does `extends="rust:Tally"`.
    fn get_property<'a>(&self, ctx: &'a Ctx, name: &str) -> Option<Result<Value<'a>>> {
        if name.eq_ignore_ascii_case("count") {
            return Some(Ok(ctx.int(self.count.load(Ordering::SeqCst))));
        }
        None
    }
}

/// Once per process, never per request — the place for thread pools and caches.
///
/// `settings` is this extension's `.cfconfig.json` block:
///
/// ```json
/// { "extensions": { "settings": { "demo": { "greeting": "Hi" } } } }
/// ```
fn on_load(_ctx: &Ctx, settings: Value) -> Result<()> {
    if let Ok(greeting) = settings.key("greeting").as_str() {
        GREETING.set(greeting.to_string()).ok();
    }
    Ok(())
}

/// Configured greeting, if `.cfconfig.json` supplied one.
static GREETING: std::sync::OnceLock<String> = std::sync::OnceLock::new();

module! {
    name: "demo",
    version: "0.1.0",
    bifs: {
        "demoGreet"          => greet,
        "demoStats"          => stats,
        "demoSummariseQuery" => summarise_query,
        "demoBuildQuery"     => build_query,
        "demoChecksum"       => checksum,
        "demoFail"           => fail,
        "demoTally"          => new_tally,
    },
    classes: { Tally },
    on_load: on_load,
}
