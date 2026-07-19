//! Phase-2 (`jit_resolve_fn` indirection / not-yet-compiled callees)
//! end-to-end verification. These tests exercise scenarios v0.84.0 could not
//! JIT — mutual recursion (A↔B), call cycles (A→B→C→A), forward calls where
//! the caller compiles before the callee, and the speculation-mismatch
//! recovery path (caller compiles speculating Int return, callee later
//! compiles returning Float → caller evicted + re-specialized).
//!
//! The interpreter is the trusted oracle: with `RUSTCFML_JIT=0` we run the
//! exact same program and demand bit-equal output.
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
/// worker threads (`crates/cli/src/lib.rs`). The VM recurses on the Rust stack
/// for nested CFML calls, so the deep (and mutual) recursion these tests
/// exercise overflows the default ~2 MB `cargo test` thread stack — a
/// test-harness artifact the real binary never hits. A panic inside `f` is
/// re-raised on the caller so the test still fails with its original message.
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

/// Run `src` with the JIT forced on (threshold=1 so any hot function
/// compiles on its second call). Returns `(stdout, fn_compiled_count)`.
fn run_jit(src: &str) -> (String, usize) {
    let src = src.to_string();
    on_big_stack(move || {
        let mut vm = CfmlVirtualMachine::new(compile(&src));
        // API, not env vars: parallel test threads share the process environment.
        vm.jit_set_threshold(1);
        for (name, value) in get_builtins() {
            vm.globals.insert(name, value);
        }
        for (name, func) in get_builtin_functions() {
            vm.builtins.insert(name, func);
        }
        vm.execute().expect("execute");
        (vm.get_output().trim().to_string(), vm.jit_compiled_count())
    })
}

/// Same program, JIT off — the trusted oracle.
fn run_interp(src: &str) -> String {
    let src = src.to_string();
    on_big_stack(move || {
        let mut vm = CfmlVirtualMachine::new(compile(&src));
        vm.jit_disable();
        for (name, value) in get_builtins() {
            vm.globals.insert(name, value);
        }
        for (name, func) in get_builtin_functions() {
            vm.builtins.insert(name, func);
        }
        vm.execute().expect("execute");
        vm.get_output().trim().to_string()
    })
}

/// A↔B mutual recursion (classic isEven / isOdd). Neither function can be
/// JIT'd by v0.84.0: at compile time of A its referenced callee B is not
/// yet Compiled, so the analyser rejects A (and vice versa). Phase-2
/// admits the binding speculatively and the runtime indirection resolves
/// at call time.
#[test]
fn mutual_recursion_a_calls_b_calls_a_jits() {
    let src = r#"
        function isEven(n) {
            if (n == 0) { return 1; }
            return isOdd(n - 1);
        }
        function isOdd(n) {
            if (n == 0) { return 0; }
            return isEven(n - 1);
        }
        total = 0;
        for (k = 1; k <= 60; k++) {
            total = total + isEven(6);
        }
        writeOutput(total);
    "#;
    let oracle = run_interp(src);
    let (out, compiled) = run_jit(src);
    // isEven(6) = 1 (even). Depth kept ≤7 to fit debug-build native
    // stack frames in the 2 MB test-thread stack (same reason as the
    // existing fib(7) test in jit_numeric.rs).
    assert_eq!(out, oracle, "mutual recursion: JIT output must match interpreter");
    assert_eq!(out, "60");
    // Both isEven and isOdd should JIT. We don't require both — even one
    // demonstrates Phase-2 working — but the test should at least show
    // *some* compilation happened on a deeply recursive workload.
    assert!(compiled >= 1, "expected at least one mutually-recursive fn to JIT");
}

/// 3-cycle: A → B → C → A. The same speculative-binding mechanism that
/// admits 2-cycles must admit longer cycles unchanged.
#[test]
fn three_way_call_cycle_jits() {
    let src = r#"
        function fa(n) {
            if (n <= 0) { return 0; }
            return 1 + fb(n - 1);
        }
        function fb(n) {
            if (n <= 0) { return 0; }
            return 1 + fc(n - 1);
        }
        function fc(n) {
            if (n <= 0) { return 0; }
            return 1 + fa(n - 1);
        }
        total = 0;
        for (k = 1; k <= 80; k++) {
            total = total + fa(6);
        }
        writeOutput(total);
    "#;
    let oracle = run_interp(src);
    let (out, compiled) = run_jit(src);
    // fa(6) = 6 → 80 × 6 = 480. Depth kept low (≤7) so unoptimised
    // Cranelift debug-build frames fit a 2 MB test-thread stack.
    assert_eq!(out, oracle, "3-cycle: JIT output must match interpreter");
    assert_eq!(out, "480");
    assert!(compiled >= 1);
}

/// Forward call: the caller's hot-trip happens before the callee has been
/// observed enough times to JIT. v0.84.0's leaf-first warm-up would have
/// rejected the caller's analysis at that point. Phase-2 admits the
/// caller with a speculative binding; once the callee later compiles, the
/// caller's runtime dispatch transparently finds it.
#[test]
fn forward_call_caller_compiles_before_callee() {
    let src = r#"
        function caller(n) {
            // Caller does much more work per call than callee, so its
            // counted-loop drives caller hot before callee.
            var t = 0;
            for (var i = 1; i <= n; i++) { t = t + i + callee(i); }
            return t;
        }
        function callee(x) { return x * 2; }

        // Hammer caller. callee accumulates calls in step; the order in
        // which the two cross the JIT threshold depends on counter
        // increments + lookup costs that change across runs, so what we
        // verify is the *output equivalence*, not which compiled first.
        total = 0;
        for (k = 1; k <= 80; k++) { total = total + caller(20); }
        writeOutput(total);
    "#;
    let oracle = run_interp(src);
    let (out, compiled) = run_jit(src);
    assert_eq!(out, oracle, "forward call: JIT output must match interpreter");
    assert!(compiled >= 1, "expected at least one of caller/callee to JIT");
}

/// Speculation-mismatch recovery: A speculatively binds B's return as Int
/// (the Phase-2 default for not-yet-compiled foreign callees), but B
/// actually returns Float. The compiled A invokes B via the dispatcher,
/// which observes the kind mismatch and sets `*bail = 2`. The outer
/// `try_call` evicts A from the cache; A's next hot trip recompiles
/// against B's now-known `ret_float = true` and runs natively from then on.
///
/// We can't assert the eviction directly from outside, but we *can*
/// assert: the final output is bit-equal to the interpreter (the
/// recovery path must be semantically correct), and the function-compiled
/// counter grows — at least one of {A_int_spec, A_after_recompile, B} ends
/// up in the cache.
#[test]
fn float_returning_callee_triggers_speculation_recovery() {
    let src = r#"
        function callee(x) {
            // Float result: 0.5 forces ret_float = true.
            return x + 0.5;
        }
        function caller(n) {
            var t = 0.0;
            for (var i = 1; i <= n; i++) { t = t + callee(i); }
            return t;
        }
        total = 0.0;
        for (k = 1; k <= 80; k++) { total = total + caller(20); }
        writeOutput(total);
    "#;
    let oracle = run_interp(src);
    let (out, compiled) = run_jit(src);
    assert_eq!(out, oracle, "float-returning callee: JIT output must match interpreter");
    // We expect *something* to JIT after recovery — at minimum, callee
    // alone (a leaf float fn that should JIT from the second call).
    assert!(compiled >= 1, "expected at least one fn to JIT after recovery");
}

/// Sanity: when a callee is *known* `Unjittable` (e.g. it touches strings
/// or any non-JIT-eligible op), the caller still rejects the optimisation
/// for that site rather than speculating. The caller can still JIT if it
/// has *other* eligible call sites or no UDF calls at all.
#[test]
fn unjittable_callee_does_not_break_caller() {
    let src = r#"
        function callee(x) {
            // Non-JIT-eligible: string concat on a slot. Forces Unjittable.
            var s = "x";
            return len(s & x);
        }
        function caller(n) {
            var t = 0;
            for (var i = 1; i <= n; i++) { t = t + i; }
            // call callee once at the very end; not in the hot path of caller's loop
            return t + callee(n);
        }
        total = 0;
        for (k = 1; k <= 80; k++) { total = total + caller(20); }
        writeOutput(total);
    "#;
    let oracle = run_interp(src);
    let (out, _compiled) = run_jit(src);
    assert_eq!(out, oracle, "unjittable callee: JIT output must match interpreter");
}
