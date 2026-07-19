//! Differential fuzzing harness for the JIT.
//!
//! Generates random CFML programs from a constrained grammar and asserts the
//! interpreter and the JIT produce bit-equal output for every one. A
//! divergence is a Phase-2 (or earlier) correctness bug — typed in CFML, but
//! checked against the exact same VM/codegen path real programs use.
//!
//! Two grammars:
//! * `random_call_graph` — N functions, each calls one other at random.
//!   Targets Phase-2's mutual-recursion / forward-call / speculation paths.
//! * `random_arithmetic_kernel` — counted loop summing a random
//!   arithmetic expression over Int + Double + a builtin shim. Targets
//!   Tier-1 / Tier-1.5 / Option-A surface that Phase-2 must not regress.
//!
//! Determinism: we own the RNG (a small LCG) and seed it from
//! `RUSTCFML_FUZZ_SEED` (default `0xC0FFEE`) so failures are reproducible.
//! Count is `RUSTCFML_FUZZ_PROGRAMS` (default 200 in debug, 2000 in release —
//! kept modest so the harness stays under ~5 s on the CI test budget).
#![cfg(feature = "jit")]

use cfml_codegen::{compiler::CfmlCompiler, BytecodeProgram};
use cfml_compiler::parser::Parser;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;

/// Small LCG (Numerical Recipes constants) — keeps the harness dep-free and
/// reproducible from a single u64 seed.
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        // Avoid degenerate state=0 (it would stay 0 forever for some LCG
        // constants; not these, but be defensive).
        Self(seed.wrapping_add(0x9E3779B97F4A7C15))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
    fn range(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
    /// Inclusive on both ends.
    fn between(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(hi >= lo);
        let span = (hi - lo + 1) as u64;
        lo + (self.next() % span) as i64
    }
}

fn compile(src: &str) -> BytecodeProgram {
    let ast = Parser::new(src.to_string()).parse().expect("parse");
    CfmlCompiler::new().compile(ast)
}

fn run(src: &str, jit_on: bool) -> String {
    let mut vm = CfmlVirtualMachine::new(compile(src));
    // API, not env vars: parallel test threads share the process environment.
    if jit_on {
        vm.jit_set_threshold(1);
    } else {
        vm.jit_disable();
    }
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.execute().expect("execute");
    vm.get_output().trim().to_string()
}

/// Build a CFML program with `nfuncs` functions, each calling one other at
/// random (possibly itself). Each body computes `c + target(n-1)` for n>0,
/// returning 0 at the base. The driver hammers `f0(depth)` in a hot loop so
/// the JIT engages on as many of the call sites as possible.
fn random_call_graph(rng: &mut Lcg) -> String {
    let nfuncs = rng.between(2, 5) as usize; // 2..=5 functions
    let depth = rng.between(2, 6); // shallow → fits the 2 MB debug stack
    let loops = rng.between(40, 80);
    let mut src = String::new();
    for i in 0..nfuncs {
        let target = rng.range(nfuncs); // may be self
        let constant = rng.between(0, 5);
        // Use no-collision names f0..f(N-1).
        src.push_str(&format!(
            "function f{}(n) {{\n    if (n <= 0) {{ return 0; }}\n    return {} + f{}(n - 1);\n}}\n",
            i, constant, target
        ));
    }
    src.push_str(&format!(
        "total = 0;\nfor (k = 1; k <= {}; k++) {{ total = total + f0({}); }}\nwriteOutput(total);\n",
        loops, depth,
    ));
    src
}

/// Build a CFML program with a counted hot loop accumulating a random
/// arithmetic expression. Mixes Int + Double + one of {abs, floor, ceiling,
/// sqr, sin, cos} so the analyser's kind lattice gets exercised under noise.
fn random_arithmetic_kernel(rng: &mut Lcg) -> String {
    let iters = rng.between(50, 200);
    let use_double = rng.next() & 1 == 1;
    let builtins = ["abs", "floor", "ceiling", "sqr", "sin", "cos"];
    let b = builtins[rng.range(builtins.len())];
    let init = if use_double { "0.0" } else { "0" };
    let constant: i64 = rng.between(1, 7);
    let expr = if use_double {
        format!("({}(i / 3.0) + i * {})", b, constant)
    } else {
        // For Int-only kernels, prefer the Int-returning shims so the
        // accumulator's slot kind stays Int.
        let bi = if matches!(b, "abs" | "floor" | "ceiling") { b } else { "abs" };
        format!("({}(i) + i * {})", bi, constant)
    };
    format!(
        "function kernel(n) {{\n    var t = {init};\n    for (var i = 1; i <= n; i++) {{ t = t + {expr}; }}\n    return t;\n}}\n\
         total = {init};\nfor (k = 1; k <= 40; k++) {{ total = total + kernel({iters}); }}\nwriteOutput(total);\n"
    )
}

fn fuzz_count() -> usize {
    if let Ok(v) = std::env::var("RUSTCFML_FUZZ_PROGRAMS") {
        if let Ok(n) = v.parse() {
            return n;
        }
    }
    // Keep the default conservative: a debug-mode run with full Cranelift
    // compilation per program is ~25 ms; 200 programs ≈ 5 s. Bump via
    // `RUSTCFML_FUZZ_PROGRAMS=2000` for a closer-to-spec sweep.
    if cfg!(debug_assertions) { 200 } else { 1000 }
}

fn fuzz_seed() -> u64 {
    std::env::var("RUSTCFML_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x00C0_FFEE_u64)
}

/// Run `f` on a thread with a 64 MB stack, matching the production CLI/serve
/// worker threads (`crates/cli/src/lib.rs`). The VM recurses on the Rust stack
/// for nested CFML calls, and the deep mutual-recursion these random call
/// graphs generate overflows the default ~2 MB `cargo test` thread stack — a
/// test-harness artifact the real binary never hits. A panic inside `f` (a
/// divergence assertion) is re-raised on the caller so the test still fails.
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

/// Run `gen` `n_programs` times, asserting JIT-on output == interpreter
/// output. On divergence, panic with the source + both outputs + seed so
/// the failure reproduces.
fn fuzz_with(gen: fn(&mut Lcg) -> String, label: &'static str) {
    on_big_stack(move || {
        let seed = fuzz_seed();
        let mut rng = Lcg::new(seed);
        let n = fuzz_count();
        for i in 0..n {
            let src = gen(&mut rng);
            let interp = run(&src, false);
            let jit = run(&src, true);
            assert_eq!(
                interp, jit,
                "[{label} #{i}, seed=0x{seed:016X}] JIT diverges from interpreter\n\
                 --- source ---\n{src}\n--- interpreter ---\n{interp}\n--- jit ---\n{jit}\n",
            );
        }
    })
}

#[test]
fn fuzz_random_call_graphs_match_interpreter() {
    fuzz_with(random_call_graph, "call_graph");
}

#[test]
fn fuzz_random_arithmetic_kernels_match_interpreter() {
    fuzz_with(random_arithmetic_kernel, "arith_kernel");
}
