# ColdBox Test Suite — Serve-Mode Boot Campaign

**Status:** BLOCKED at framework boot. No ColdBox test assertion has executed yet.
**Target:** `../coldbox-platform` (git `64a3c81a7`, TestBox `be`, ColdBox 8.x line).
**Engine:** RustCFML v0.459.0 release binary.

---

## Progress log

- **P0 `java.util.Optional` — DONE (not committed).** Added `make_java_optional` +
  `handle_java_optional` (construction) in `java_shims.rs` and
  `Vm::handle_java_optional_method` (instance methods, incl. map/filter/ifPresent
  which invoke the wrapped `createDynamicProxy` closure via a new
  `invoke_functional_proxy` helper) in `lib.rs`. Also fixed
  `sam_method_for_interface` — `Predicate`/`Comparator`/`BinaryOperator`/the
  `To*Function` SAMs were unmapped and defaulted to `run`, so cbproxies'
  `Predicate.filter` never invoked its closure. Test: `tests/java_shims/test_optional.cfm`
  (22 assertions, green). Full suite 6086/6086; `cargo test --workspace` green;
  wasm target build green. **Verified against the running ColdBox server: boot now
  advances past `coreOptional`** to the next blocker (below).

## Current blocker (P1) — DBAppender autoCreate hits an unconfigured MySQL datasource

After Optional lands, `runner-logbox.cfm` 500s at the **same** `registerAppender`
site with a new, *environmental* error (not an engine bug):

```
Runtime Error: queryExecute: invalid MySQL connection string: URL ParseError { invalid port number }
  1: registerAppender  system/logging/LogBox.cfc:321
  2: configure         system/logging/LogBox.cfc:175   (LogBox config from ColdBox.cfc)
  ...
```

**Root cause:** `test-harness/config/Coldbox.cfc:83` registers a `DBAppender`
(`dsn:"coolblog", table:"logs", autoCreate:true`). `autoCreate` runs a
`queryExecute` at registration to create the `logs` table. The `coolblog`
datasource (`../coldbox-platform/.cfconfig.json`) has DSN
`jdbc:mysql://{host}:{port}/{database}` with **unsubstituted CommandBox
interpolation tokens** — on a real box these resolve from env at server-start; under
`rustcfml --serve` they're literal, so the MySQL driver rejects `{port}`. Lucee
would need the same real `coolblog` DB, so this is **not a divergence**.

**To proceed (env decision for the user):** either (a) point `coolblog` at a real
MySQL (Docker, per `reference_docker_db_testing_macos`) with a concrete DSN, or
(b) temporarily drop the `db` appender from the test-harness LogBox config to
exercise the non-DB boot path first. No engine change required here.

---

## How to run (serve mode)

```bash
# 1. One-time: install the gitignored ForgeBox dep (cbproxies is NOT checked in)
cd ../coldbox-platform && box install cbproxies      # populates system/async/cbproxies/

# 2. Build + serve rooted at the ColdBox checkout
cargo build --release            # in RustCFML
cd ../coldbox-platform && /path/to/RustCFML/target/release/rustcfml --serve --port 8512

# 3. Hit the runners over HTTP
curl -s "http://127.0.0.1:8512/tests/runner.cfm?reporter=simple"      # default specs
#   also: runner-core / runner-async / runner-wirebox / runner-cachebox / runner-logbox / runner-integration
```

All runners boot the framework via `tests/Application.cfc` → `VirtualApp.startup()` →
`LoaderService.loadApplication()` → `LogBox.configure()`. Everything past that is gated on the boot chain below.

---

## Current blocker (P0) — `java.util.Optional` not shimmed

Every runner returns **HTTP 500** with the identical stack:

```
Runtime Error: Variable 'coreOptional' is undefined
  1: registerAppender  system/logging/LogBox.cfc:321
  2: configure         system/logging/LogBox.cfc:175
  3: loadApplication   system/web/services/LoaderService.cfc:52
  4: startup           system/testing/VirtualApp.cfc:69
  5: onRequestStart    tests/Application.cfc:50
```

**Root cause (confirmed against the running server):**
`system/async/cbproxies/models/Optional.cfc:16` runs
`variables.coreOptional = createObject("java","java.util.Optional")` in its pseudo-constructor.
RustCFML has no shim for `java.util.Optional`, so `createObject` throws; `coreOptional`
is never assigned; the next call (`.empty()`) fails with "undefined". LogBox's async log
listener drags in the cbproxies stack during appender registration.

```
createObject("java","java.util.Optional")
→ ERROR: Java class [java.util.Optional] is not supported.
```

**Fix:** add a `java.util.Optional` shim. Route it through the constructor dispatch in
`crates/cfml-vm/src/lib.rs` (~line 12760–12852, the big `match class_lower {…}`) to a new
`handle_java_optional` in `crates/cfml-vm/src/java_shims.rs`. Optional is a small value
container — back it with a `CfmlValue::Struct` holding `__value` + `__present`, and implement
the methods the CFC calls: `empty()`, `of(v)`, `ofNullable(v)`, `isPresent()`, `isEmpty()`,
`get()`, `orElse(v)`, `orElseGet(cb)`, `ifPresent(cb)`, `map(cb)`, `filter(cb)`, `$or(...)`.
(See `Optional.cfc` for the exact surface — it delegates each to the java `Optional` instance.)

---

## Likely next blockers (once Optional lands, re-run and confirm)

ColdBox's `system/async` layer (cbproxies + Future/scheduler) references these java classes.
Missing shims confirmed against `java_shims.rs` — expect them to surface as boot proceeds,
roughly in dependency order:

| Class | Used by | Shimmed? | Notes |
|---|---|---|---|
| `java.util.Optional` | LogBox appender / ScheduledTask `lastResult` | ❌ **P0** | **the current wall** |
| `java.util.concurrent.CompletableFuture` | `Future.cfc` async chains | ❌ P1 | core of AsyncManager; may need real value-carrying shim, not a no-op |
| `java.util.concurrent.ForkJoinPool` | `AsyncManager` default pool | ❌ P1 | existing executor shims (ThreadPoolExecutor etc.) are a model to copy |
| `java.util.concurrent.FutureTask` | task wrapping | ❌ P2 | |
| `java.util.stream.IntStream` | scheduler range helpers | ❌ P2 | |
| `java.net.URI` / `java.net.Socket` | env/util helpers | ❌ P3 | probably narrow; may be guardable |
| `java.io.{ByteArray,Object}{Input,Output}Stream`, `java.io.PrintWriter` | serialization / logging | ❌ P3 | ObjectStream = java serialization (no JVM) — likely shim-to-error + code path avoidance |
| `coldfusion.sql.DataSrcImpl`, `coldfusion.filter.FusionContext` | ACF-only internals | ❌ | ACF-guarded; RustCFML reports as Lucee → these branches should not execute. Verify. |
| `org.hibernate.Version` | ORM version probe | ❌ | ORM-guarded; verify skipped |

**Already shimmed (should be fine):** StringBuilder, System, ConcurrentHashMap, UUID, Thread,
ZoneId/ZoneOffset/Duration/Period/LocalDateTime/DayOfWeek + `java.time.temporal.*`, InetAddress,
Collections, Runtime, TreeMap, LinkedHashMap, TimeUnit, LinkedBlockingQueue, Executors,
SoftReference/ReferenceQueue.

The cbproxies **functional interfaces** (`Function`, `Supplier`, `Consumer`, `BiFunction`,
`Callable`, etc.) go through `createDynamicProxy`; RustCFML already maps proxy method names
(`java_shims.rs:239` — callable→call, supplier→get, function→apply, consumer→accept). Confirm
these actually invoke the wrapped CFML closure once Future/CompletableFuture are real.

---

## Plan of attack

1. **P0 — Optional shim.** Add `handle_java_optional` + dispatch arm. Rebuild, re-serve, re-run
   `runner-logbox.cfm` (smallest boot). Expect the boot to advance past LogBox.
2. **Iterate the boot chain.** Re-run after each shim; the 500 stack tells you the next missing
   class. Work P1 → P3 in the table above. Prefer `runner-logbox` / `runner-cachebox` /
   `runner-wirebox` first — they exercise less of the async layer than the MVC/integration specs.
3. **CompletableFuture is the real risk.** A no-op shim won't do — ColdBox's AsyncManager returns
   futures whose `.get()` must yield the computed value. Decide: synchronous eager-eval shim
   (run the supplier immediately, wrap the result) vs. genuine deferred. Eager/synchronous is the
   pragmatic first cut and matches how the existing executor shims behave.
4. **First green target:** `runner-logbox.cfm` and `runner-wirebox.cfm` fully executing their
   specs (LogBox/WireBox are the least async-dependent).
5. **Then the MVC/integration specs** via `runner.cfm` / `runner-integration.cfm`. These pull in
   the full VirtualApp request lifecycle — expect a second wave of gaps (interceptors, cache,
   handler dispatch) once boot completes.
6. **Record every silent no-op** in `docs/known-issues.md` (project convention) and add a
   RustCFML CFML test for each shim under `tests/` before tagging.

## Prerequisites / gotchas

- `cbproxies` is a **gitignored ForgeBox dependency** — a fresh clone will 500 with
  "Could not find component …cbproxies.models.Optional" until `box install cbproxies` is run.
  Not an engine bug.
- Restart the served binary after each rebuild (bytecode cache is per-process).
- Validate cold **and** warm in serve mode, not just first request (per project convention —
  app-scope/lifecycle regressions only show on the 2nd hit).
- This overlaps the existing **ColdBox hmvc-presso boot campaign** (java.util.concurrent +
  createDynamicProxy + java.time shims). Reuse those handlers; don't duplicate.

## Files

- Constructor dispatch: `crates/cfml-vm/src/lib.rs` ~12760–12852 (`match class_lower`).
- Shim handlers: `crates/cfml-vm/src/java_shims.rs` (4.3k lines; executor/time/proxy shims live here).
- ColdBox proxies: `../coldbox-platform/system/async/cbproxies/models/*.cfc`.
- ColdBox async: `../coldbox-platform/system/async/{AsyncManager,tasks/Future,tasks/ScheduledTask}.cfc`.
