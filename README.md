## RustCFML

![RustCFML Mascot](crab.svg)

A CFML (ColdFusion&reg; Markup Language) interpreter written in Rust — a single, fast, run-anywhere binary with a tiny memory footprint.

![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)
![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)

> ColdFusion is a registered trademark of Adobe Inc. RustCFML is not affiliated with or endorsed by Adobe.

**[Try RustCFML in your browser](https://rustcfml.github.io/RustCFML/demo/)** — interactive demo running on WebAssembly.

## Project Aims

RustCFML aims to be a **compatible, fast, run-anywhere** CFML engine with a minimal memory footprint and maximum performance. It is deliberately opinionated:

- **A lean, stable core.** We don't add things to the core that are prone to constant churn in the wider ecosystem. Reliability comes first — think of RustCFML as an LTS-style engine. It is already blazingly fast.
- **Libraries over built-ins.** We won't add core functions that are better served by libraries. Instead, where possible, we make the engine compatible enough to *run* those libraries.
- **No administrator, ever.** RustCFML does not have — and never will have — a ColdFusion Administrator. Configuration is file-based via [`.cfconfig.json`](docs/configuration.md), with environment-variable substitution for secrets.
- **Inspired by real apps and modern deployment.** We follow modern deployment practices and won't bake in CI/CD features that belong in your pipeline, not your language runtime.
- **Honest about Java.** There is no JVM under the hood. We provide limited [Java-class shim support](docs/java-shims.md) (faking common Java classes), but you should expect — and will find — differences.

## Project Background

RustCFML began as a proof of AI model capabilities by Alex Skinner, CEO of [Pixl8 Group](https://www.pixl8.co.uk/). It has been written almost entirely by AI — predominantly Claude Opus — with research and test synthesis assisted by local models.

## Getting Started

The fastest way to start is with a prebuilt binary — no toolchain required.

1. **Download a binary** for your platform from the **[latest release](https://github.com/RustCFML/RustCFML/releases/latest)** (Linux x86_64/aarch64, macOS aarch64), then put it on your `PATH`:

   ```bash
   chmod +x rustcfml-macos-aarch64
   sudo mv rustcfml-macos-aarch64 /usr/local/bin/rustcfml
   ```

2. **Run a web application** — the most common way to get going. Point RustCFML at a directory of `.cfm` files:

   ```bash
   rustcfml --serve ./mywebroot --port 8500
   ```

   It serves `.cfm` pages and static assets, runs the `Application.cfc` lifecycle, and supports sessions, URL rewriting, and file uploads. See **[Web Server](docs/web-server.md)**.

3. **Go to production** — add `--production` to enable warm in-memory caching (skips per-request filesystem checks for a 3–4× throughput gain):

   ```bash
   rustcfml --serve ./mywebroot --production
   ```

   See **[Deployment](docs/deployment.md)** for production mode, the sandbox/virtual filesystem, standalone binaries, and Cloudflare Workers.

You can also run a single template (`rustcfml myapp.cfm`), drop into a REPL (`rustcfml -r`), or run inline code (`rustcfml -c '...'`). See **[Getting Started](docs/getting-started.md)** for those and shebang scripts.

## Performance

RustCFML compiles to a native binary with no runtime VM overhead, so it starts instantly and serves requests with a fraction of the memory of JVM-based CFML engines.

Serving a "Hello World" `.cfm` page in `--production` mode against a warmed Lucee 7 — same machine (Apple M-series), same page, Apache Bench, 8s runs. Requests/sec, higher is better:

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

RustCFML serves roughly 2–3.5× the throughput at about a tenth of the memory, with no JVM warmup. Both engines benefit from HTTP keep-alive; RustCFML scales further with it, sustaining ~26,000 req/s. See **[Performance](docs/performance.md)** for full methodology and production-mode caching.

### Query-of-Queries

Running [bdw429s/cfml-qoq-perf-tests](https://github.com/bdw429s/cfml-qoq-perf-tests) — 10 representative SELECTs against a 1M-row in-memory `employees` query — in serve mode, 5-run median, same machine, lower is better:

| Engine | Total (ms) | RustCFML speedup |
|---|---:|---:|
| **RustCFML** v0.112 | **1,116** | — |
| BoxLang 1.14 | 1,368 | **1.23× faster** |
| Lucee 7.0.4 | 7,884 | **7.1× faster** |

RustCFML wins six of ten queries outright (and the total), including the 5×UNION DISTINCT and the grouped-aggregate cases. Pure-Rust SQL engine in `crates/cfml-qoq` — no JDBC, no HSQLDB, parallelised across cores with rayon (non-wasm).

## Documentation

### Build & run

| Topic | Description |
|---|---|
| **[Getting Started](docs/getting-started.md)** | Prebuilt binaries, running files, REPL, shebang scripts, building from source |
| **[Web Server](docs/web-server.md)** | Serve mode, Application.cfc lifecycle, URL rewriting, distributed sessions |
| **[Sessions](docs/sessions.md)** | The `session` scope, lazy default, the `CFID` cookie & `this.sessioncookie`, storage backends, expiry |
| **[Configuration](docs/configuration.md)** | `.cfconfig.json` — datasources, mappings, mail, security, caches, env vars |

### Data

| Topic | Description |
|---|---|
| **[Database](docs/database.md)** | `queryExecute`, datasources, `cfqueryparam`, engine specifics |
| **[Object Storage](docs/s3.md)** | S3 / R2 / MinIO — `S3*` functions and transparent `s3://` paths |

### Debug & operate

| Topic | Description |
|---|---|
| **[Debugging & observability](docs/debugging.md)** | Classic debug-output footer (queries, template times, exceptions, scopes; `getDebugData()`/`isDebugMode()`/`debugAdd()`), threshold-gated **sampling profiler** (`profileNow()`/`getRequestProfile()`), **OpenTelemetry** OTLP traces + Prometheus RED metrics, and the native **`--profile`** CPU flamegraph/pprof profiler |
| **[Observability ops](docs/observability-ops.md)** | Tail sampling at an OTel Collector (keep slow/errored traces off the app host) + a one-command Collector → Tempo → Grafana stack |
| **[Performance](docs/performance.md)** | Benchmarks and production-mode caching |
| **[Deployment](docs/deployment.md)** | Web app, Docker, CLI tools, Cloudflare Workers; production mode & sandbox |

### Concurrency & realtime

| Topic | Description |
|---|---|
| **[Threading](docs/threads.md)** | `cfthread` on real OS threads — shared vs copied scopes, join/terminate, caveats |
| **[WebSockets](docs/websockets.md)** | Realtime channel components, rooms, presence, auth, resumability, multi-node fan-out, ack-by-return — over raw WebSocket **and** socket.io |

### Extend & embed

| Topic | Description |
|---|---|
| **[Native Modules](docs/native-modules.md)** | Extend a binary with first-class Rust built-ins and classes |
| **[Java Shims](docs/java-shims.md)** | Emulated Java classes for `createObject("java", …)` — what's supported and known gaps |
| **[Embedding](docs/embedding.md)** | Use the RustCFML engine from your own Rust code |
| **[WebAssembly](docs/wasm.md)** | Compile to WASM; Cloudflare Workers notes |

### Reference

| Topic | Description |
|---|---|
| **[Architecture](docs/architecture.md)** | Compilation pipeline and crate layout |
| **[Testing](docs/testing.md)** | Running the test suites and writing tests |
| **[Status](docs/status.md)** | Implementation status and remaining work |
| **[Known Issues](docs/known-issues.md)** | Documented gaps, silent no-ops, and Lucee/BoxLang divergences |

## Deployment

RustCFML is designed to deploy as a single artifact in several shapes — see **[Deployment](docs/deployment.md)** for full detail:

- **Web application** — run `--serve` behind your reverse proxy, with `--production` for warm in-memory caching:

  ```bash
  rustcfml --serve ./mywebroot --production
  ```

  On the same host as the proxy, bind a **Unix domain socket** with `--socket` instead of a TCP port — it skips loopback TCP and sustains ~40% more throughput than TCP loopback at high concurrency. See **[Deployment → Behind a reverse proxy (nginx + Unix socket)](docs/deployment.md#behind-a-reverse-proxy-nginx--unix-socket)**.

- **Optimised Docker container** — *coming soon*: a minimal image for containerised deployment.

- **CLI tool** — compile a CFML app into a standalone command-line binary. See **[Deployment → CLI tools](docs/deployment.md#cli-tools)**.

- **Cloudflare Workers** — run RustCFML at the edge via WebAssembly. See **[RustCFML-Cloudflare-worker](https://github.com/RustCFML/RustCFML-Cloudflare-worker)**.

**Production mode** (warm caching) and **sandbox / virtual filesystem** (host isolation, embedded files) apply to both web and CLI deployments — they're documented once in **[Deployment](docs/deployment.md)**.

## Features

- **Complete CFML language** — CFScript and tag syntax (a preprocessor converts 50+ tags to CFScript), components with inheritance and interfaces, closures, member functions, and higher-order functions across arrays, structs, queries, and lists.
- **400+ built-in functions** — strings, arrays, structs, dates, math, lists, queries, JSON, XML, regex, encoding, hashing, and modern password hashing (bcrypt/scrypt/argon2).
- **Batteries-included web server** — `Application.cfc` lifecycle, [sessions](docs/sessions.md) (in-process, Memcached, or clustered), cookies, authentication, URL rewriting, and file uploads.
- **Data & integration** — `queryExecute` over SQLite, MySQL, PostgreSQL, and MSSQL with pooling and `cftransaction`; in-memory **Query-of-Queries** (`dbtype="query"`) on a pure-Rust SQL engine — see the perf table below; `cfhttp`; `cfmail`; and S3-compatible object storage (AWS S3, Cloudflare R2, MinIO).
- **Real concurrency** — `cfthread` runs bodies on real OS threads with shared `application`/`request`/`session` scopes and `cflock`. See **[Threading](docs/threads.md)**.
- **Native WebSockets** — realtime channels on the same port as HTTP: one CFC per channel with convention lifecycle methods, rooms, presence, auth, `lastEventId` resumability, multi-node fan-out, ack-by-return, and emit-from-anywhere (`wsPublish`/`io()`) — reachable over both **raw WebSocket** and the **socket.io** transport, plus an imperative **socket.io-lucee-compatible** API (`new SocketIoServer()`). See **[WebSockets](docs/websockets.md)**.
- **Debugging & observability** — a Lucee-style debug-output footer, a FusionReactor-class threshold-gated **sampling profiler**, **OpenTelemetry** distributed traces (OTLP) + Prometheus RED metrics, and a native CPU **flamegraph profiler** (`--profile`) — all off by default and zero-cost when off. See **[Debugging & observability](docs/debugging.md)** and **[Observability ops](docs/observability-ops.md)**.
- **Run anywhere** — native binaries, self-contained single-file apps, and a WebAssembly target that runs on Cloudflare Workers.
- **Extensible** — drop in first-class built-ins and classes written in Rust ([native modules](docs/native-modules.md)).

See **[Compatibility & Status](docs/status.md)** for implementation status.

### Partially Supported

- **Image functions** — core surface shipped: `imageNew`/`imageRead`/`imageResize`/`imageScaleToFit`/`imageCrop`/`imageRotate`/`imageFlip`/`imageInfo`/`imageGetWidth`/`imageGetHeight`/`imageWrite`/`imageWriteBase64`/`imageGetBlob` plus `<cfimage>` (`read`/`write`/`resize`/`info`/`convert`/`writeToBrowser`), backed by the pure-Rust `image` crate (native **and** wasm). Drawing primitives, filters, free-angle rotation, and EXIF/IPTC metadata are not yet implemented. See **[Known Issues §18](docs/known-issues.md)**.
- **Spreadsheets** — native, JVM-free `.xlsx` read/create/edit/write: the full `Spreadsheet*` function family, a fluent `Spreadsheet()` builder, and the `<cfspreadsheet>` tag. Cell values, formulas, struct-based styling, merged cells, freeze/split panes, charts, images, comments, hyperlinks, autofilter, data validation, conditional formatting, delete/shift rows/columns, headers/footers & print setup, password-protected write, query/array/CSV interchange, and full read→modify→write round-tripping (styling/charts/images preserved). Reads `.xlsx` (full round-trip) plus legacy `.xls`/`.xlsb`/`.ods` (data-only, via `calamine`). Backed by the pure-Rust `umya-spreadsheet` crate (native/server builds; not in the wasm target). See **[Spreadsheets](docs/spreadsheets.md)**.

### Not Supported

- ORM, SOAP/WSDL, Flash/Flex, PDF, LDAP, Registry
- `cfschedule`, `cfwddx`

For documented gaps, silent no-ops, and Lucee/BoxLang divergences, see **[Known Issues](docs/known-issues.md)**.

## Threading

`<cfthread>` bodies run on **real OS threads** — concurrently, on separate cores — not sequentially inline. `action="join"` blocks until a thread (or, with no `name`, all threads) completes; results land in `cfthread.NAME` (`status`, `output`, `error`, `elapsedtime`, plus the body's `thread` scope). `application`, `server`, `session`, and `request` scopes are **shared live** across threads (guard concurrent writes with `cflock`); `variables` is **copied at spawn**, and data is passed in via the thread's `attributes`. An error in a thread sets its status to `TERMINATED` without aborting the parent.

Two deliberate differences from Lucee, both arising from doing threading safely in Rust:

- **`terminate` is cooperative, not forceful.** Rust can't safely kill a running thread mid-instruction (it could leave locks held or memory half-written — the reason Java deprecated `Thread.stop()`). Instead, `terminate` sets a flag the body checks at loop back-edges and then aborts. A thread spinning in a loop stops promptly; one parked in a single long call (`sleep`, a slow query) won't notice until it returns to a loop. It's a difference in responsiveness, not correctness.
- **A `cftransaction` can't span the parent↔child boundary.** Its live DB connection can't be used from another thread, so a spawned thread starts with no transaction — its queries aren't part of one the parent has open, and a parent rollback won't undo them. Keep transactional work within a single thread.

Full detail, scope tables, and examples: **[Threading](docs/threads.md)**.

## Architecture

```plaintext
CFML Source (.cfm / .cfc)
    → Tag Preprocessor → CFScript → Lexer → Parser → AST → Compiler → Bytecode → VM → Output
```

RustCFML is a Cargo workspace of focused crates (`cfml-common`, `cfml-compiler`, `cfml-codegen`, `cfml-vm`, `cfml-stdlib`, `cli`, `wasm`). See **[Architecture](docs/architecture.md)** for the full breakdown.

## Building from Source

If you'd rather build it yourself (or you're contributing), you need Rust stable (>= 1.75.0) — install via [rustup.rs](https://rustup.rs/):

```bash
git clone https://github.com/RustCFML/RustCFML.git
cd RustCFML
cargo build --release        # binary at target/release/rustcfml
cargo install --path crates/cli   # optional: install on your PATH
```

See **[Getting Started → Building from source](docs/getting-started.md#building-from-source)** for feature flags, the WebAssembly target, and the self-contained-binary build path.

## Contributing

Contributions are welcome.

- **New here?** If you haven't contributed before, please **[open an Issue](https://github.com/RustCFML/RustCFML/issues)** with detail (a minimal reproducible CFML snippet, expected vs actual behaviour) before opening a PR.
- **Pull requests are the preferred way to contribute a fix.** A great place to start is a **CFML-based test** that demonstrates the behaviour (see **[Testing](docs/testing.md)**).
- **Lucee is the reference for compatibility.** Your test **must pass on Lucee** — if it doesn't, we won't accept it. RustCFML targets [cfdocs.org](https://cfdocs.org) with Lucee as the primary implementation target. (By rare exception, where Lucee allows something genuinely unreasonable, we may choose not to match it.)

See **[Testing](docs/testing.md)** for how to run the suite against both RustCFML and Lucee.

### Contributors

[![Contributors](https://contrib.rocks/image?repo=RustCFML/RustCFML)](https://github.com/RustCFML/RustCFML/graphs/contributors)

_Avatars are generated automatically from the [GitHub contributor graph](https://github.com/RustCFML/RustCFML/graphs/contributors) by [contrib.rocks](https://contrib.rocks)._

## Project Inspiration

- [Lucee](https://github.com/lucee/Lucee) — open-source CFML engine (Java)
- [BoxLang](https://github.com/ortus-boxlang/BoxLang) — modern CFML+ runtime (Java)
- [RustPython](https://github.com/RustPython/RustPython) — Python interpreter in Rust (architectural reference)

## License

MIT
