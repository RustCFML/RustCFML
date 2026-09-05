# RustCFML - Claude Code Guide

## Project Overview

CFML (ColdFusion Markup Language) interpreter written in Rust. Compiles CFML source through a pipeline: **Tag Preprocessor -> Lexer -> Parser -> AST -> Bytecode Compiler -> Stack-based VM**. Inspired by RustPython.

## Build & Test

```bash
cargo build                          # Debug build
cargo build --release                # Release build
cargo run -- tests/runner.cfm        # Run all tests (~3965 assertions, 478 suites)
cargo run --release -- file.cfm      # Run a CFML file
cargo run --release -- --serve       # Start web server on port 8500
cargo test --workspace               # ALL Rust tests. Run serial if a parallel
                                     # run is ever flaky: `-- --test-threads=1`.

# Wasm-target members — NOT built by the commands above (see warning below):
cargo build -p cfml-worker -p rustcfml-wasm --target wasm32-unknown-unknown

# Interactive-demo build — exercises the wasm-pack + wasm-bindgen path that the
# "Deploy Interactive Demo" GitHub Action uses (the plain cargo build above does
# NOT). Run before pushing to main. (`cargo install wasm-pack` if missing.)
wasm-pack build crates/wasm --target web
```

> 🚨 **Verification gate — a red OR skipped test in ANY suite is a release
> blocker, never a shrug.** A green `cargo build` + `tests/runner.cfm` is NOT
> sufficient. Before tagging you MUST have all of these green: `cargo test
> --workspace` (all Rust tests), `cargo run -- tests/runner.cfm`
> (CFML, CLI **and** serve-mode cold+warm — see "Validate in serve mode"), served
> **both** `--serve` and `--serve --production` (production-only code paths exist:
> the bytecode/freshness caches and the component-path cache all behave
> differently there. Serving the runner in dev only is exactly how the GH #284
> regression test stayed red in production for 23 releases, v0.521.0→v0.544.0), the
> wasm build above, AND — when pushing to `main` — `wasm-pack build crates/wasm
> --target web` (the demo-deploy path; a plain `cargo build --target
> wasm32-unknown-unknown` does NOT run wasm-pack/wasm-bindgen, so it cannot catch
> a broken demo deploy — this is exactly how the v0.240.0 wasm-opt SIGSEGV slipped
> through to the "Deploy Interactive Demo" Action). If a test fails or is `#[ignore]`d, do NOT dismiss it as
> "flaky" or "unrelated" — `git bisect` to the commit that broke it and fix the
> root cause (or open a tracked issue). The CFML suite cannot see everything the
> Rust suites can, so `cargo test --workspace` is not optional.
>
> (The JIT was removed in v0.653.0 — measured at 13 of 1,345 functions admitted
> on Preside, and turning it OFF was 1-7% FASTER on a warm render. Its numeric
> checks live on as `crates/cfml-vm/tests/numeric_semantics.rs`.)

> ⚠️ **A plain `cargo build` does NOT compile the wasm32-only workspace members
> (`cfml-worker`, `rustcfml-wasm`).** They only target `wasm32-unknown-unknown`,
> so host builds silently skip them. Any change touching `CfmlValue`/`CfmlArray`/
> `CfmlStruct` (or any shared type) MUST also be verified with the
> `--target wasm32-unknown-unknown` build above before tagging — otherwise the
> Cloudflare worker build breaks for downstream consumers even though `cargo
> build`, `cargo test`, and `tests/runner.cfm` are all green. (This bit us in
> v0.33.0/v0.34.0: `CfmlArray::iter()` yielding owned values broke
> `cfml-worker`'s HyperdriveDriver; fixed in v0.34.1.) Requires the target:
> `rustup target add wasm32-unknown-unknown`.

> ℹ️ **Network test — `tests/stdlib/test_cfhttp.cfm`.** This suite makes live
> `<cfhttp>` calls to the **RustCFML Cloudflare Worker** echo service at
> `https://rustcfml-worker.rustcfml.workers.dev/echo/` (request mirror,
> response-header reflector, status endpoint — sources live in the sibling
> `RustCFMLWorker` repo under `cfml/echo/`). This replaced the old, flaky
> `httpbin.org` dependency: the worker is edge-hosted and effectively always-on.
> The test has a **reachability guard** — if the first probe doesn't return 200
> (offline dev box, edge outage, OR the worker simply hasn't been redeployed
> with the `/echo/` endpoints yet) it skips the ~46 network assertions with a
> single informational pass rather than spraying false reds. So a green run here
> means *either* "all assertions passed" *or* "endpoint unreachable, skipped" —
> if you changed cfhttp, confirm the endpoints are deployed and the guard isn't
> silently masking a regression. Don't `git bisect` a red here; it's
> environmental. (Engine-side cfhttp coverage that needs **no** network lives in
> `tests/tags/test_cfhttp_attribute_collection.cfm` and
> `tests/tags/test_tags_cfhttp_interpolation.cfm`, which hit the local
> `tests/tags/http_statements_target.cfm` echo endpoint.)

Tests are CFML-based, not Rust-based. The test runner (`tests/runner.cfm`) includes all test files and uses the harness (`tests/harness.cfm`) which provides `assert()`, `assertTrue()`, `assertFalse()`, `assertNull()`, `assertThrows()`, `suiteBegin()`, `suiteEnd()`.

### Cross-engine testing (Lucee)

The same test suite runs against Lucee to verify compatibility with the reference engine. Start Lucee via CommandBox (served out of the project root), then hit `tests/runner.cfm` over HTTP:

```bash
box server start cfengine=lucee@be   # starts Lucee on a CommandBox-assigned port (e.g. 127.0.0.1:8585)
curl -s http://127.0.0.1:8585/tests/runner.cfm -o /tmp/lucee_out.txt
grep -E "^(SUMMARY|FAIL \||ERROR)" /tmp/lucee_out.txt
box server status                    # show port if you don't see it in startup output
box server stop                      # shut it down
```

**Writing tests that pass on both engines:**
- Do NOT use `var` at page scope — Lucee rejects it ("Unsupported Context for Local Scope"). Declare without `var` at page level, or wrap the test body in a function.
- Always close `<cfscript>` blocks with `</cfscript>` — Lucee's parser is strict about this; RustCFML tolerates EOF.
- The test runner includes `harness.cfm` once at the top. Individual test files must NOT re-include it, because the harness body resets `request._test_total*` counters and masks the grand summary.
- HTTP-dependent tests (`tests/tags/test_tags_cfscript_statements.cfm`) discover the port from `cgi.server_port` (set by the server at request-time) and skip the HTTP subtests when the runner is invoked from the CLI with no server available. Don't hardcode a port.
- Lucee and RustCFML both run through the same `tests/runner.cfm`, so a green run on both is the compatibility bar.
- **A file Lucee cannot run is marked in the runner, not left to fail:**
  `<cf_runtest file="stdlib/test_xmp.cfm" rustcfmlOnly="true">`. It runs on
  RustCFML and reports `SKIP` elsewhere, with a `SKIPPED:` block in the summary.
  Nothing is skipped on RustCFML, so the release gate is unaffected. Use it only
  when the file's *purpose* is RustCFML-only; a file with one superset assertion
  should guard that line with `isRustCFML()` and keep its cross-engine value —
  unless Lucee rejects it at PARSE time, where an in-file guard is too late.
- **Do not leave the application altered for the tests that follow.**
  `application action="update" mappings={...}` REPLACES the application's mapping
  set (Lucee parity). A test that registers a mapping must merge — read
  `getApplicationMetadata().mappings`, add, write back. Getting this wrong is
  invisible on RustCFML and silently cost 67 aborted files on Lucee for months.
- Don't call a BIF via `argumentCollection` with mixed-case argument names —
  Lucee uppercases struct keys and its BIF lookup is case-sensitive, so it fails
  there naming the very argument you passed.

## Architecture

```
crates/
  cfml-common/      # CfmlValue enum, CfmlError, Position (small, ~470 lines)
  cfml-compiler/    # Lexer, Parser, AST, Tag Preprocessor (~5,700 lines)
  cfml-codegen/     # AST -> BytecodeOp compiler (~1,850 lines)
  cfml-vm/          # Stack-based bytecode VM (~6,900 lines) - THE BIG FILE
  cfml-stdlib/      # 400+ built-in functions (~9,800 lines) - THE OTHER BIG FILE
  cli/              # CLI entry point + Axum web server (~1,500 lines)
  wasm/             # WebAssembly target (small wrapper)
```

### Compilation Pipeline

1. **Tag Preprocessor** (`tag_parser.rs`): Converts `<cfset x=1>` style tags to CFScript (`x=1;`). All CFML tags become function calls or script constructs. Body tags use `find_closing_tag` to extract content.

2. **Lexer** (`lexer.rs`): Tokenizes CFScript source. Tokens defined in `token.rs`.

3. **Parser** (`parser.rs`): Recursive descent parser producing AST nodes defined in `ast.rs`.

4. **Codegen** (`compiler.rs`): Walks AST and emits `BytecodeOp` instructions defined in `bytecode.rs`.

5. **VM** (`lib.rs`): Stack-based execution engine. The main loop is `execute_function_with_args()` which processes bytecode ops.

## Key Patterns

### Adding a New Built-in Function

**Simple (pure function, no VM state needed):**
1. Register in `builtins.rs` → `get_builtin_functions()` with section comment
2. Implement as `fn fn_name(args: Vec<CfmlValue>) -> CfmlResult`

**VM-intercepted (needs access to VM state like output_buffer, globals, closures):**

> 🚨 **DO NOT append another `if name_lower == "..."` branch to `call_function`.** That
> instruction is what this recipe used to say, and following it faithfully grew
> `call_function` to **7,497 lines — 20% of `cfml-vm/src/lib.rs`** — with ~190 name
> comparisons interleaved with `sandbox_intercept` / `s3_intercept` /
> `resolve_file_bif_paths`. Nothing ever removed a branch, so "is this name intercepted?"
> became unanswerable without reading the whole function, which in turn blocked
> compile-time builtin binding: a wrong answer there does not run slow, it **bypasses the
> sandbox**. (This doc still said the list was at "~line 1718" when it was at ~12,990.)

1. Register in `builtins.rs` as usual. A stub returning an error is only needed if the
   function cannot work at all outside the VM; otherwise give it a sensible CLI-mode body
   (`writeOutput` prints to stdout, for instance).
2. **Declare it** in `cfml-common/src/builtins_meta.rs` → `VM_INTERCEPTED`. This is the
   single enumerable source of truth; codegen reads it to decide which builtins may be
   compile-time bound. Over-declaring is safe (the name just keeps the slower generic
   dispatch); **under-declaring is a correctness/security bug**.
3. Put the handler in the matching `crates/cfml-vm/src/intercepts_*.rs` module — or add a
   new one — and add the name to that module's `handles()`. `call_function` consults these
   as a short list of guards.
4. If your handler does **not** return for every call it claims (e.g. `cfdirectory` handles
   `action="list"` and defers the rest), end the dispatch with
   `intercepts_common::unhandled()`; the caller turns that back into fall-through. Do NOT
   try to predict fall-through inside `handles()` — that duplicates each branch's inner
   conditions somewhere they will drift.

You cannot silently get this wrong: `crates/cfml-vm/tests/intercept_declaration_guard.rs`
scans the VM source and **fails the build** if any dispatched-on name is undeclared, and
`cfml-stdlib` asserts `BUILTIN_NAMES` matches the registration table exactly.

Examples of VM-intercepted: `writeOutput`, `writeDump`, `sleep`, `include`, all higher-order functions (arrayMap, structFilter, etc.), savecontent, cfthread.

### Adding a New CFML Tag

1. Add the tag name match arm in `tag_parser.rs` → `tags_to_script_impl()`
2. Convert to CFScript equivalent (function call, block, or statement)
3. Body tags: use `find_closing_tag(chars, tag_end, len, "tagname")` to find `</cftagname>`
4. For tags needing VM support: register stub functions + add VM intercepts (see cfthread, cfsavecontent patterns)

### Adding a New Operator

1. Add token variant in `token.rs`
2. Add lexer recognition in `lexer.rs`
3. Add parser handling in `parser.rs` (check precedence level)
4. Add AST node if needed in `ast.rs`
5. Add codegen in `compiler.rs` → emit bytecode ops
6. Add VM handling in `lib.rs` if new bytecode op

### Adding Tests

1. Create `tests/<category>/test_<feature>.cfm`
2. Use the harness: `suiteBegin("Name")`, `assert("label", actual, expected)`, `suiteEnd()`
3. Add `try { include "category/test_file.cfm"; } catch ...` line in `tests/runner.cfm`

### Native (Rust) modules

Users can extend a self-contained binary with first-class Rust BIFs and classes. The plumbing:

- `CfmlValue::NativeObject(Arc<RwLock<dyn CfmlNative>>)` in `cfml-common/src/dynamic.rs`. The `CfmlNative` trait (`Send + Sync + Debug`) exposes `class_name()` + `call_method(name, args)`, with optional `get_property`/`set_property` (default-impls return None) for CFC `this.X` fall-through.
- `vm.register_native_fn(name, f)` and `vm.register_native_class(name, ctor)` (`cfml-vm/src/lib.rs`).
- **Query-of-Queries** lives in `crates/cfml-qoq/` (pure-Rust SQL `SELECT` engine; no JDBC). `queryExecute(sql, params, {dbtype:"query"})` is VM-intercepted in `lib.rs` (`execute_qoq`): it parses, resolves the referenced query variables from scope, `mem::take`s `qoq_registry` (so the CFML-UDF callback can borrow `&mut self` via `call_function`), runs `cfml_qoq::execute`, then restores. Engine design: one `eval` with a `RowCtx` (Row vs Group) is the dual-path evaluator; `intersection.rs` folds joins into 1-based row-index vectors (0 = NULL sentinel). Extend SQL functions in `cfml-qoq/src/functions.rs` (scalars) / `execution.rs` (aggregates). `vm.register_native_qoq_fn(name, f, QoQFnKind::Scalar|Aggregate)` exposes a native fn as both a BIF and a QoQ function; `queryRegisterFunction(name, udf[, "aggregate"])` registers a CFML UDF for use in SQL. RustCFML is a BoxLang-faithful superset of Lucee QoQ — see `docs/known-issues.md` §9.
- `createObject("rust", "Name", ...)` consults `vm.native_classes` before falling through. Method dispatch short-circuits in `call_member_function()` for `NativeObject`.
- CFC inheritance from a Rust class: `component extends="rust:Name" { ... }`. `resolve_inheritance_chain` (`cfml-vm/src/lib.rs`) detects the `rust:` prefix and stashes `__rust_extends`; `attach_native_parent` default-constructs the parent into `__super`. `super(args)` inside `init` is compiled to `BytecodeOp::CallRustSuperCtor` which re-runs the registered ctor. `super.X` and unqualified method fall-through both reach the native parent. `this.X` reads/writes route through `CfmlNative::get_property`/`set_property` when CFC has no such key.
- The `rustcfml-cli` crate is lib+bin. Library exposes `set_registrar` / `run_with_registrar` so externally-generated `main.rs` can inject modules.
- `rustcfml --build` runs the "cocktail" path when a project contains `native/<crate>/Cargo.toml`: generates a synthetic Cargo workspace under `.rustcfml-cocktail/`, path-deps on `rustcfml-cli` + each user module, shells out to `cargo build --release`, then appends the VFS archive to the produced binary. Plain CFML apps with no `native/` directory stay on the toolchain-free bundling path.
- The smoke-test in `crates/cli/src/main.rs` is gated on `RUSTCFML_NATIVE_SMOKE_TEST=1` and exercises `tests/native/*.cfm` end-to-end without needing the full cocktail build.
- Working example: `examples/native_module_demo/`. Module author contract documented in its README.

## Important Conventions

> 🚫 **NEVER kill processes by name. `pkill -f rustcfml`, `pkill -f "rustcfml
> --serve"`, `killall rustcfml` and friends are BANNED.** The developer runs
> long-lived `--serve` instances of their own — a Preside site on 8500, other
> apps on other ports — and a name pattern matches every one of them, not just
> the servers you started. This has killed a running Preside server mid-session.
> Kill ONLY what you started, and only by something that cannot match anything
> else: the **PID** you captured at launch, or a pattern anchored to **your own
> port** (`pkill -f "rustcfml --serve --port 8611"`). If you did not start it,
> do not stop it. Before restarting anything of the developer's, ASK — booting a
> Preside site WRITES to its database, so guessing the docroot is not free.

> 🚫 **NEVER solve a problem by making a function a no-op (or a stub / fake-empty
> return) without first flagging it and getting the user's approval.** If a boot
> blocker or failing call can be "fixed" by having a function return
> `""`/`[]`/`{}`/`null`/do-nothing, STOP and ask before doing it. No-ops hide
> missing behaviour, silently corrupt correctness, and can mask performance
> problems (a real implementation that was never written). Prefer a faithful
> implementation; if one is genuinely out of scope, surface that trade-off to the
> user and let them decide — do not quietly ship the no-op. Any no-op that *is*
> approved MUST be recorded in `docs/known-issues.md` (see below).

- **Case-insensitive**: All CFML identifiers, function names, scope keys are case-insensitive. Use `.to_lowercase()` or `eq_ignore_ascii_case()` for comparisons.
- **IndexMap, not HashMap**: Use `IndexMap<String, CfmlValue>` for ordered key-value structures (structs, scopes). `HashMap` only for internal lookups (builtins, user_functions).
- **CfmlValue**: The core enum — `Null`, `Boolean(bool)`, `Int(i64)`, `Double(f64)`, `String(String)`, `Array(Vec<CfmlValue>)`, `Struct(IndexMap<String, CfmlValue>)`, `Function(CfmlFunction)`, `Query(...)`, `Binary(Vec<u8>)`, `Component(...)`.
- **CfmlResult**: `Result<CfmlValue, CfmlError>`. Functions return `Ok(CfmlValue::Null)` for void operations.
- **1-based arrays**: CFML arrays are 1-based. Convert to 0-based for Rust Vec access.
- **Output buffering**: `self.output_buffer` collects all output. `saved_output_buffers` is a stack for nested capture (cfsavecontent, cfsilent, cfthread).
- **Closure scope capture**: Closures carry `captured_scope: Option<Arc<RwLock<IndexMap>>>`. Sibling closures share the same Arc. Write-back propagates mutations to parent scope.

## Scope Resolution Order

Variable lookup checks scopes in CFML-standard order: `local` -> `arguments` -> `thread` (inside cfthread) -> `variables` -> `cgi` -> `url` -> `form` -> `cookie` -> `request` -> `application` -> `server` -> `session`.

Explicit scope prefix (`variables.x`, `request.x`) bypasses the search chain.

## Common Gotchas

- `lib.rs` and `builtins.rs` are very large files (~7k and ~10k lines). Use grep to find specific handlers rather than reading the whole file.
- The VM intercept pattern means some "builtin" functions are actually handled in `lib.rs`, not `builtins.rs`. Always check both.
- Tag preprocessing happens before parsing — the parser only sees CFScript. Debug tag issues by checking the tag_parser output first.
- Query column access via dot notation (`q.name`) returns an Array of column values. Bracket notation (`q["name"]`) is different.
- The `request` scope persists across includes within a single execution — tests use it for state (harness counters).

## Git Commit Style

Short, descriptive first line summarizing the change. Examples from history:
```
Optimize VM function call dispatch: 5.9x faster recursive calls
Add getProfileString, setProfileString, getProfileSections; simplify README
Add bytecode cache for serve mode, fix all build warnings
```

**NEVER add Co-Authored-By lines to commits.**

## Reference

- Compatibility target: [cfdocs.org](https://cfdocs.org) (functions and tags)
- [docs/status.md](docs/status.md) — detailed implementation status
- [docs/testing.md](docs/testing.md) — testing guide
- Repo: github.com/RustCFML/RustCFML
