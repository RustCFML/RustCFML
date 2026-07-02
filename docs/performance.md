# Performance

[← Back to README](../README.md)

RustCFML compiles to a native binary with no runtime VM overhead — it starts instantly and serves requests with a fraction of the memory of JVM-based CFML engines.

## Benchmark

Serving a "Hello World" `.cfm` page in `--production` mode against a warmed Lucee 7, on the same machine (Apple M-series), same page, using Apache Bench with 8-second runs. Requests/sec, higher is better:

| Concurrency | RustCFML | Lucee 7.0 | RustCFML (keep-alive) | Lucee (keep-alive) |
|---|---|---|---|---|
| `-c 1`   | **1,908** | 1,205 | **3,118**  | 1,625 |
| `-c 10`  | **5,466** | 2,648 | **21,716** | 8,125 |
| `-c 50`  | **6,983** | 3,503 | **25,833** | 8,085 |
| `-c 100` | **7,528** | 3,107 | **25,855** | 7,419 |

| | RustCFML | Lucee 7.0 |
|---|---|---|
| **Memory (RSS, under load)** | **~60 MB** | ~560 MB |
| **Startup** | **instant** | ~15s |

### Notes on methodology

- **Both engines were warmed** before measuring (a JVM needs JIT warmup to reach steady state; an early, cold run undercounts Lucee heavily).
- **Keep-alive helps both engines** — roughly 3× for Lucee and 4× for RustCFML — by amortising TCP connection setup. The non-keep-alive columns are closer to a "cold client" worst case.
- Numbers are from a developer laptop and are indicative, not a lab benchmark. They will vary by hardware, OS, and page complexity.
- BoxLang is not included in this run; it is broadly comparable to Lucee on this workload.

To reproduce: build `--release`, serve a one-line `<cfoutput>Hello World</cfoutput>` page with `--production`, warm with `ab -n 20000 -c 50`, then measure with `ab -t 8 -c <N>` (add `-k` for keep-alive).

## Query-of-Queries

`queryExecute(sql, params, {dbtype:"query"})` runs in-memory SQL `SELECT` against query variables already in scope, on a pure-Rust engine (`crates/cfml-qoq` — no JDBC, no HSQLDB). Filter, projection, dedup and sort are parallelised across cores with rayon on non-wasm targets.

Running [bdw429s/cfml-qoq-perf-tests](https://github.com/bdw429s/cfml-qoq-perf-tests) — 10 representative SELECTs against a 1M-row `employees` query — in serve mode with the source query cached in `application` scope, 5-run median on a 14-core Apple M-series, lower is better:

| Query | RustCFML v0.112 | BoxLang 1.14 | Lucee 7.0.4 |
|---|---:|---:|---:|
| 1. basic SELECT + WHERE + ORDER BY | 126 | **63** | 985 |
| 2. UNION                           | **70**  | 73   | 565 |
| 3. non-grouping aggregate          | **18**  | 143  | 278 |
| 4. grouped aggregate               | **38**  | 91   | 110 |
| 5. string concat                   | **38**  | 73   | 141 |
| 6. `LIKE '%Harry%'`                | **15**  | 25   | 73  |
| 7. 2-table comma join              | 171     | **72** | 1,162 |
| 8. 3-table comma join              | 217     | **214** | 1,209 |
| 9. ANSI 3-table join               | 166     | **110** | 1,209 |
| 10. 5× UNION DISTINCT              | **258** | 383  | 2,351 |
| **Total**                          | **1,116** | 1,368 | 7,884 |

RustCFML wins six of ten queries and the total against BoxLang (1.23× faster overall), and is roughly 7× faster than Lucee. The single largest gap to BoxLang is on simple single-table scans (Q1, Q7) where BoxLang's compiled column representation makes per-cell clones cheaper; the gaps narrow under aggregation and `UNION DISTINCT` where the rayon-parallel paths dominate.

To reproduce: clone [bdw429s/cfml-qoq-perf-tests](https://github.com/bdw429s/cfml-qoq-perf-tests), substitute the `cfloop` test driver for a `for` loop (Lucee/BoxLang only — RustCFML supports both), then run `test_rcf.cfm` from each engine's serve mode with the source query cached in `application` scope (the script does this) so the 1M-row build is paid once.

## Production mode caching

By default the server re-validates files on each request (statting `Application.cfc` resolution and every cached bytecode entry) so edits are picked up live. Passing `--production` (or `RUSTCFML_PRODUCTION=1`) enables four persistent in-memory caches:

- **Application.cfc path resolution** — the directory walk is done once, then memoized (including negative results).
- **URL → file resolution** — routing `is_file` stats are memoized.
- **Bytecode cache trust** — the per-hit `mtime` check on every compiled file is skipped.
- **Component path resolution** — resolving a dotted/relative component name (`createObject`, `new`, `extends=`, WireBox lookups) normally `stat()`s many candidate paths (relative-to-caller, each mapping, the webroot). That probing is memoized per `(component, caller dir)`, so repeat resolutions skip the filesystem. This is the dominant filesystem cost on framework-heavy apps: profiling a Preside boot, it cut `RealFs::exists` CPU by roughly two-thirds. Overridable with `RUSTCFML_NO_COMPONENT_CACHE=1`.

Once warm, requests pay zero filesystem IO. The typical speedup on an app with `Application.cfc` + cfincludes is 3–4× requests/sec. Files changed on disk are not picked up until restart. Self-contained binaries running in sandbox mode enable production caching automatically, since the embedded VFS is immutable. See **[Deployment](deployment.md)**.
