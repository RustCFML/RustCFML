//! Demo native module for RustCFML --build.
//!
//! Exposes two BIFs (`rustGreet`, `rustAdd`) and one class (`Tally`).
//! `register(vm)` is the contract `--build` looks for — it is called once
//! per process, immediately after the standard library has been registered.

use rustcfml_cli::{CfmlError, CfmlNative, CfmlResult, Value, Vm};
use std::sync::{Arc, RwLock};

/// Entry point. The cocktail-generated main.rs calls this once at startup
/// inside `rustcfml_cli::run_with_registrar`.
pub fn register(vm: &mut Vm) {
    vm.register_native_fn("rustGreet", greet);
    vm.register_native_fn("rustAdd", add);
    vm.register_native_class("Tally", tally_new);
}

// ---- BIFs ----

fn greet(args: Vec<Value>) -> CfmlResult {
    let name = args
        .get(0)
        .map(|v| v.as_string())
        .unwrap_or_else(|| "World".to_string());
    // `Value::string(..)` — the String variant holds an Arc<String>, so build it
    // through the constructor rather than the variant.
    Ok(Value::string(format!("Hello, {} (from Rust)", name)))
}

fn add(args: Vec<Value>) -> CfmlResult {
    let a = to_i64(args.get(0));
    let b = to_i64(args.get(1));
    Ok(Value::Int(a + b))
}

fn to_i64(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Int(i)) => *i,
        Some(Value::Double(d)) => *d as i64,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

// ---- Class ----

#[derive(Debug)]
struct Tally {
    count: i64,
}

impl CfmlNative for Tally {
    fn class_name(&self) -> &str {
        "Tally"
    }
    fn call_method(&mut self, name: &str, _args: Vec<Value>) -> CfmlResult {
        match name.to_lowercase().as_str() {
            "bump" => {
                self.count += 1;
                Ok(Value::Int(self.count))
            }
            "value" => Ok(Value::Int(self.count)),
            other => Err(CfmlError::runtime(format!(
                "Tally has no method '{}'",
                other
            ))),
        }
    }
}

fn tally_new(_args: Vec<Value>) -> CfmlResult {
    Ok(Value::NativeObject(Arc::new(RwLock::new(Tally { count: 0 }))))
}
