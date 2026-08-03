// Thin binary entry point for the `rustcfml` CLI. All runtime logic lives in
// the sibling library crate `rustcfml-cli` so that `--build`-produced
// binaries (and tests) can call `run_with_registrar(...)` directly.
//
// The only logic in this file is the smoke-test hook: when the env var
// `RUSTCFML_NATIVE_SMOKE_TEST=1` is set, we install a registrar that wires
// up a couple of Rust-backed BIFs and a Counter class so the test suite can
// exercise the native-module pathway end-to-end. The hook is irrelevant to
// production users.

use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::{CfmlError, CfmlResult};

// DHAT heap profiler (feature `dhat-heap`). Swaps in DHAT's global allocator so
// every allocation is tracked; the Profiler created in `main` is held for the
// whole process and writes `dhat-heap.json` (in cwd) when it drops on graceful
// shutdown. Builds without the feature pay zero cost.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

// Sampling heap profiler (feature `memprofile`). Unlike DHAT above, this
// allocator is inert until `--memprofile` arms it at runtime, so a build with
// the feature on is still a usable server binary. Only one global allocator can
// exist, hence the `not(dhat-heap)` guard.
#[cfg(all(feature = "memprofile", unix, not(feature = "dhat-heap")))]
#[global_allocator]
static ALLOC: rustcfml_cli::memprofile::SamplingAlloc = rustcfml_cli::memprofile::SamplingAlloc;

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    if std::env::var("RUSTCFML_NATIVE_SMOKE_TEST").as_deref() == Ok("1") {
        rustcfml_cli::run_with_registrar(|vm| {
            vm.register_native_fn("nativeAdd", native_add);
            vm.register_native_fn("nativeGreet", native_greet);
            vm.register_native_class("Counter", counter_new);
        });
    } else {
        rustcfml_cli::run();
    }
}

// ---------------------------------------------------------------------------
// Smoke-test native module
// ---------------------------------------------------------------------------

fn native_add(args: Vec<CfmlValue>) -> CfmlResult {
    let a = args.get(0).map(coerce_num).unwrap_or(0.0);
    let b = args.get(1).map(coerce_num).unwrap_or(0.0);
    let sum = a + b;
    if sum.fract() == 0.0 && sum.abs() < (i64::MAX as f64) {
        Ok(CfmlValue::Int(sum as i64))
    } else {
        Ok(CfmlValue::Double(sum))
    }
}

fn native_greet(args: Vec<CfmlValue>) -> CfmlResult {
    let name = args
        .get(0)
        .map(|v| v.as_string())
        .unwrap_or_else(|| "World".to_string());
    Ok(CfmlValue::string(format!("Hello, {}!", name)))
}

fn coerce_num(v: &CfmlValue) -> f64 {
    match v {
        CfmlValue::Int(i) => *i as f64,
        CfmlValue::Double(d) => *d,
        CfmlValue::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        CfmlValue::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[derive(Debug)]
struct Counter {
    value: i64,
}

impl cfml_common::dynamic::CfmlNative for Counter {
    fn class_name(&self) -> &str {
        "Counter"
    }
    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_lowercase().as_str() {
            "increment" => {
                self.value += 1;
                Ok(CfmlValue::Int(self.value))
            }
            "add" => {
                let n = args.get(0).map(coerce_num).unwrap_or(0.0) as i64;
                self.value += n;
                Ok(CfmlValue::Int(self.value))
            }
            "get" | "value" => Ok(CfmlValue::Int(self.value)),
            "reset" => {
                self.value = 0;
                Ok(CfmlValue::Null)
            }
            other => Err(CfmlError::runtime(format!(
                "Counter has no method '{}'",
                other
            ))),
        }
    }
    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        match name.to_lowercase().as_str() {
            "value" => Some(CfmlValue::Int(self.value)),
            _ => None,
        }
    }
    fn set_property(&mut self, name: &str, value: CfmlValue) -> Option<Result<(), CfmlError>> {
        match name.to_lowercase().as_str() {
            "value" => {
                self.value = coerce_num(&value) as i64;
                Some(Ok(()))
            }
            _ => None,
        }
    }
}

fn counter_new(args: Vec<CfmlValue>) -> CfmlResult {
    let start = args.get(0).map(coerce_num).unwrap_or(0.0) as i64;
    let obj = Counter { value: start };
    Ok(CfmlValue::NativeObject(std::sync::Arc::new(
        std::sync::RwLock::new(obj),
    )))
}
