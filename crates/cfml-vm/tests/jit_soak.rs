//! JIT soak harness. The fuzz tests exercise **breadth** (many random
//! programs); soak tests exercise **depth** — running the same Phase-2
//! workloads many times in a single process to surface cache-growth,
//! Cranelift-context corruption, bail/eviction loops, and counter
//! wraparound that single-shot tests don't see.
//!
//! Each soak test:
//! * Compiles the program once.
//! * Builds a fresh VM and executes it.
//! * Repeats N times (default 30 — kept modest for CI; bump via
//!   `RUSTCFML_SOAK_ITERATIONS=N`).
//! * Asserts output is identical every iteration *and* the JIT
//!   `compiled_count` stabilises (does not grow unboundedly).
#![cfg(feature = "jit")]

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_compiler::parser::Parser;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;

fn compile(src: &str) -> BytecodeProgram {
    let ast = Parser::new(src.to_string()).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

/// Run `f` on a thread with a 64 MB stack, matching the production CLI/serve
/// worker threads. The VM recurses on the Rust stack for nested CFML calls, so
/// even the modest mutual-recursion these soak programs exercise can overflow
/// the default ~2 MB `cargo test` thread stack — a harness artifact never hit by
/// the real binary (whose worker threads are 64 MB). Mirrors the wrapper the
/// other JIT test suites (`jit_numeric`/`jit_fuzz`/`jit_recursion`) already use.
/// A panic inside `f` (a failed assert) is re-raised so the test still fails.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack test thread")
        .join()
    {
        Ok(v) => v,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn fresh_vm(program: BytecodeProgram) -> CfmlVirtualMachine {
    let mut vm = CfmlVirtualMachine::new(program);
    // API, not env vars: parallel test threads share the process environment.
    vm.jit_set_threshold(1);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm
}

fn soak_iterations() -> usize {
    std::env::var("RUSTCFML_SOAK_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

/// Run `src` repeatedly under JIT threshold=1. Each iteration gets a fresh
/// VM (and therefore a fresh `JitEngine` cache), so this primarily probes
/// `Backend` reuse — `JITModule` keeps growing as new functions are declared
/// across iterations. Asserts every iteration produces identical output and
/// the per-iteration compile count is stable (it is bounded by the program's
/// distinct JIT-eligible functions, not by the iteration index).
fn soak(label: &str, src: &str) {
    let label = label.to_string();
    let src = src.to_string();
    on_big_stack(move || soak_inner(&label, &src));
}

fn soak_inner(label: &str, src: &str) {
    let program = compile(src);
    let iters = soak_iterations();
    let mut first_out: Option<String> = None;
    let mut counts: Vec<usize> = Vec::with_capacity(iters);
    for i in 0..iters {
        let mut vm = fresh_vm(program.clone());
        vm.execute().expect("execute");
        let out = vm.get_output().trim().to_string();
        let c = vm.jit_compiled_count();
        counts.push(c);
        if let Some(ref f) = first_out {
            assert_eq!(
                f, &out,
                "[{label}] iteration {i} diverged from iteration 0:\n  first={f:?}\n  now  ={out:?}",
            );
        } else {
            first_out = Some(out);
        }
    }
    // Each iteration spins up its own JitEngine, so compiled_count is
    // bounded per iteration by the eligible function set. Asserting the
    // *max* never exceeds a sane upper bound catches the case where a
    // bug makes us speculatively recompile the same function inside one
    // iteration (e.g. an eviction loop).
    let max = *counts.iter().max().unwrap_or(&0);
    let min = *counts.iter().min().unwrap_or(&0);
    assert!(
        max <= 16,
        "[{label}] suspiciously large compile count per iteration: max={max} (counts={counts:?})",
    );
    // And all iterations should converge on the same count (the engine is
    // deterministic; runs are not allowed to drift).
    assert_eq!(
        max, min,
        "[{label}] per-iteration compile count not stable: min={min} max={max} (counts={counts:?})",
    );
}

#[test]
fn soak_mutual_recursion() {
    soak(
        "mutual_recursion",
        r#"
        function isEven(n) {
            if (n == 0) { return 1; }
            return isOdd(n - 1);
        }
        function isOdd(n) {
            if (n == 0) { return 0; }
            return isEven(n - 1);
        }
        total = 0;
        for (k = 1; k <= 40; k++) { total = total + isEven(6); }
        writeOutput(total);
        "#,
    );
}

#[test]
fn soak_three_way_cycle() {
    soak(
        "three_way_cycle",
        r#"
        function fa(n) { if (n <= 0) { return 0; } return 1 + fb(n - 1); }
        function fb(n) { if (n <= 0) { return 0; } return 1 + fc(n - 1); }
        function fc(n) { if (n <= 0) { return 0; } return 1 + fa(n - 1); }
        total = 0;
        for (k = 1; k <= 40; k++) { total = total + fa(6); }
        writeOutput(total);
        "#,
    );
}

#[test]
fn soak_speculation_recovery() {
    soak(
        "speculation_recovery",
        r#"
        function callee(x) { return x + 0.5; }
        function caller(n) {
            var t = 0.0;
            for (var i = 1; i <= n; i++) { t = t + callee(i); }
            return t;
        }
        total = 0.0;
        for (k = 1; k <= 40; k++) { total = total + caller(10); }
        writeOutput(total);
        "#,
    );
}

#[test]
fn soak_pure_arithmetic() {
    soak(
        "pure_arithmetic",
        r#"
        function kernel(n) {
            var t = 0;
            for (var i = 1; i <= n; i++) { t = t + abs(i - 50) + floor(i / 3.0); }
            return t;
        }
        total = 0;
        for (k = 1; k <= 40; k++) { total = total + kernel(80); }
        writeOutput(total);
        "#,
    );
}
