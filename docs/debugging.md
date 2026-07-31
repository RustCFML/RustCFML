# Debugging

RustCFML ships the Adobe/Lucee-familiar **debug output footer** — the panel
appended to a page showing where a request spent its time: the queries it ran
(with bound parameters), every template it executed, exceptions, log/trace
entries, and the request scopes. It is modelled on Lucee 6/7's data model, so it
feels native and existing Lucee debug habits carry over.

The footer is **off by default** and gated so it never leaks to ordinary
visitors. It is built on an internal observability hook bus — the same
foundation later profiling/tracing layers build on — that costs nothing when no
debugger is attached.

> Built with the `observability` Cargo feature, which is on by default for the
> native binary and off for the WebAssembly/Worker build.

## Quick start

Enable it for local development by adding a `debugging` block to your
[`.cfconfig.json`](configuration.md):

```jsonc
{
  "debugging": {
    "enabled": true
  }
}
```

With just `enabled: true`, the footer renders for requests from `127.0.0.1` /
`::1` (the default `showFromIPs` whitelist). Hit any `.cfm` page and scroll past
the content — the panel is appended below it.

## Activation — the four gates

The footer renders only when **all four** of these pass (evaluated
cheapest/most-secure first; a request that fails any gate collects nothing and
allocates nothing):

1. **Enabled** — `debugging.enabled` is `true`.
2. **Viewer allowed** — the client IP is in `debugging.showFromIPs` **OR** the
   URL trigger matches (see below). This is the security gate: debug output
   leaks SQL, scope contents and file paths, so it is localhost-only by default
   and is enforced **identically in production**.
3. **Not suppressed by the page** — `<cfsetting showDebugOutput="false">` turns
   the footer off for that page (it can only turn it *off*, never bypass gates
   1–2). Non-HTML responses (JSON, binary, redirects) auto-suppress.
4. **Renderable** — there is an HTML/text response body to append to. Auto-render
   happens on web requests only; CLI runs still collect the data (so the BIFs
   below work) but don't get a panel appended to stdout.

### Running debug on a live site

Because the IP whitelist is honoured in production, the first-class way to debug
a live site is to leave `enabled: true` and restrict `showFromIPs` to your
office/VPN/ops addresses — every other visitor gets a normal page and never sees
the panel or any timing. Optionally add a secret URL trigger as a second path in.

### The URL trigger (a RustCFML enhancement)

Lucee core matches by IP only; RustCFML adds a **fully configurable** URL trigger
— both the variable *name* and its required *value* — which enables
security-by-obscurity:

```jsonc
"urlTrigger": {
  "enabled": true,
  "param": "myhiddenvar",   // the URL/form variable NAME (default "debug")
  "value": "s3cr3t-9f2a"    // required value (default "true"); set an unguessable secret
}
```

Then `?myhiddenvar=s3cr3t-9f2a` unlocks the footer for that request. An empty
`value` means presence-only (any value) — and is **refused in production mode**,
so a bare `?debug` can never expose a live site.

Behind a reverse proxy, set `trustForwardedFor` so the gate resolves the real
client IP rather than the proxy's (see the config reference below).

## What the footer shows

| Section | Contents |
|---|---|
| **Queries** | Each `queryExecute` / `<cfquery>`: name, execution time, recordcount, datasource, issuing `template:line`, the SQL, and the **bound parameters** (value + cfsqltype). |
| **Execution Time** | The request total, split into **Application** and **Query** time. |
| **Templates** | Every template executed — the requested page, each `<cfinclude>`, every custom tag and `<cfmodule>` body, `Application.cfc` lifecycle methods, and CFC method calls — aggregated per file with total / app / query / count / avg. Same scope as Lucee's Execution Time section ("templates, includes, modules, custom tags, and component method calls"); a body tag's start and end phases count as two executions, as they do on Lucee. |
| **Exceptions** | Exceptions raised during the request (including ones caught by `try`/`catch`), with type, message and tag context. |
| **Trace / Log** | `writeLog` / `<cflog>` and `trace` / `<cftrace>` entries. |
| **Generic data** | App- and framework-injected panels (see `debugAdd` below). |
| **Scopes** | The configured request scopes (`cgi`, `url`, `form`, … — never `variables`/`local`). |

## Templates

Five built-in templates, selected with `debugging.template`:

- `modern` *(default)* — the rich HTML panel.
- `classic` / `simple` — plainer HTML tables.
- `comment` — an HTML `<!-- … -->` block (visible only in view-source), handy
  when a visible panel would disturb the layout.
- `none` — collect the data (so the BIFs work) but render no panel.

## BIFs

Available whenever gates 1–2 pass:

- **`getDebugData()`** → a struct with the sections above (`queries`, `pages`,
  `exceptions`, `traces`, `genericData`, `scopes`, `total`, …). Times are in
  microseconds. Use it to build a custom/AJAX debug view or feed your own tooling.
- **`isDebugMode()`** → boolean; `true` when the footer is active this request.
- **`debugAdd(category, name, value)`** or **`debugAdd(category, struct)`** →
  append rows to the **Generic data** section. The supported channel for app code
  and frameworks to inject their own debug panel.

```cfscript
if ( isDebugMode() ) {
    debugAdd( "MyApp", { controller: "users.index", cacheHit: false } );
}
```

`<cfsetting showDebugOutput="false">` suppresses the footer for the current page.

## Configuration reference

The `debugging` block in `.cfconfig.json` (Lucee-compatible; unknown keys are
ignored):

```jsonc
{
  "debugging": {
    "enabled": false,                          // master switch
    "showFromIPs": ["127.0.0.1", "::1"],       // the security gate — exact IPs allowed to see the footer
    "trustForwardedFor": false,                // reverse-proxy client-IP resolution:
                                               //   false  = use the socket peer (default)
                                               //   true   = trust X-Forwarded-For / X-Real-IP (foot-gun; only
                                               //            safe if your edge overwrites the header on ingress)
    "urlTrigger": {                            // RustCFML enhancement (Lucee matches by IP only)
      "enabled": true,
      "param": "debug",                        // the URL/form variable NAME — rename to hide it
      "value": "true"                          // required value; "" = presence-only (refused in production)
    },
    "template": "modern",                      // modern | classic | simple | comment | none
    "highlightMs": 250,                        // queries slower than this (ms) are highlighted red
    "maxRecords": 10,                          // rolling cap per section
    "fields": {                                // section toggles
      "database": true,
      "exception": true,
      "tracing": true,
      "timer": true,
      "dump": true,
      "scopes": ["cgi", "url", "form"]         // which scopes to dump (never variables/local)
    }
  }
}
```

## Sampling profiler (FusionReactor-style)

The second observability layer is a **threshold-gated cooperative sampling
profiler**. When a request runs longer than a threshold (default 3s), a watchdog
thread asks that request's own VM to snapshot its CFML call stack on an interval
(default 200ms). The snapshots fold into an inverted call tree with self/total
sample counts, so you can see which functions a slow request actually spent its
time in — without instrumenting every call.

It is **off by default** and, like the footer, costs nothing when off. When
armed but a request stays *under* the threshold, the only cost is one relaxed
atomic load per executed source line (almost always false). Only a request that
crosses the threshold pays for stack snapshots, and that cost is constant
(one snapshot per interval) regardless of how much code the request runs.

Enable it under the `observability` block:

```jsonc
{
  "observability": {
    "enabled": true,
    "profiler": {
      "enabled": true,
      "thresholdMs": 3000,     // only requests slower than this start sampling
      "intervalMs": 200,       // sampling cadence once armed
      "maxSamples": 500,       // hard per-request cap
      "watchdogTickMs": 50     // how often the watchdog scans in-flight requests
    }
  }
}
```

**CFML surface:**

- `profileNow()` — force-start profiling the current request immediately
  (FusionReactor's "Profile now"). Takes one sample synchronously and returns
  `true` when the profiler is enabled, `false` when it is off server-wide.
- `getRequestProfile()` — the folded call tree for the current request as a
  struct: `{ sampleCount, root }`, where each node has `function`, `template`,
  `line`, `self`, `total`, `selfPercent`, `totalPercent`, and `children`.

**Admin endpoint:** in serve mode, `GET /__rustcfml/profiler` returns the most
recent profiled (slow) requests as JSON — route, sample count, and the call
tree. It 404s when the profiler is off.

### Limitation — JIT-compiled numeric leaves

RustCFML's JIT compiles small hot numeric functions straight to native code,
bypassing the interpreter loop (and therefore the per-line sampling hook and the
call-frame push). Time spent inside a JIT-compiled numeric leaf is therefore
attributed to its **caller's** self-time rather than showing as its own frame.
This is acceptable in practice — such functions are tiny and fast by definition
(that is why they were JIT'd) — but a profile will not break them out
separately. Interpreted functions (the overwhelming majority of a real request,
and everything in serve mode where the per-request JIT rarely warms up) are
attributed correctly.

## OpenTelemetry traces + metrics

The third observability layer exports **distributed traces** and **RED metrics**
as standard OpenTelemetry, so a slow or errored request in production can be
inspected in Grafana Tempo / Jaeger / Honeycomb / Datadog without runtime
degradation. It is only present in a build compiled with the **`obs-otel`** Cargo
feature (host-only — never in the wasm/worker build) and is off until configured.

```bash
cargo build --release --features obs-otel
```

- **Traces** reproduce the request → CFC-method → query transaction tree as OTel
  spans and export over **OTLP (HTTP/protobuf)** on a background batch thread, so
  export never sits on the request path. Head sampling
  (`ParentBased(TraceIdRatioBased)`) keeps overhead low; an inbound W3C
  `traceparent` is always continued. A **span allow-list + depth cap** bound how
  many spans a request emits — the request root, DB queries and template renders
  are always spanned; user functions are spanned only at/under `spanDepthCap` and
  matching `spanAllowList`. Uncaught exceptions record an `exception` span event
  and set the span status to Error; a `try/catch`-recovered exception does not.
- **RED metrics** (request rate, errors, duration + DB query count/duration) are
  exposed on a native **Prometheus scrape endpoint** (`/__rustcfml/metrics` by
  default) that Prometheus can scrape directly — no collector required.

```jsonc
{
  "observability": {
    "enabled": true,
    "otel": {
      "enabled": true,
      "endpoint": "http://localhost:4318",   // OTLP/HTTP collector; /v1/traces is appended
      "serviceName": "rustcfml",
      "sampleRatio": 0.05,                    // head sampling (0.0–1.0)
      "spanDepthCap": 3,                      // user fns at/under this depth may be spanned
      "spanAllowList": ["*"],                 // name globs eligible for a span
      "metrics": { "enabled": true, "prometheusPath": "/__rustcfml/metrics" }
    }
  }
}
```

Semantic conventions emitted: HTTP server (`http.request.method`, `http.route`,
`url.path`, `http.response.status_code`, `client.address`, …) on the root span;
DB client (`db.system.name`, `db.query.text`, `db.operation.name`,
`db.namespace`, `db.response.returned_rows`) on query spans.

> **Metrics-export note.** RustCFML exposes metrics via the standalone
> `prometheus` crate rather than `opentelemetry-prometheus` (whose release lags
> the core OTel line and would fork the dependency tree). **Traces** push over
> OTLP; **metric** OTLP *push* is a documented follow-up — a collector scraping
> the Prometheus endpoint (or its `prometheusexporter`) covers that case one hop
> downstream. Transaction spans on functions the JIT compiles to native code
> share the profiler's [attribution limitation](known-issues.md) (JIT'd numeric
> leaves don't get their own span).

## Native CPU/wall-clock profiler (`--profile`)

The sampling profiler above works at the *CFML* level (which function is running).
The **native** profiler works one layer down — it samples the **Rust** call stack
(bytecode dispatch, BIF internals, allocator pressure), the hot spots the CFML
sampler can't see. It wraps [pprof-rs](https://docs.rs/pprof): a `SIGPROF` timer
samples at ~100 Hz with a malloc-free signal handler.

Build with the `obs-pprof` feature (Unix-only — it uses `SIGPROF`) and run a
one-shot script with `--profile`:

```bash
cargo build --release --features obs-pprof
./target/release/rustcfml --profile mybench.cfm
```

On exit it writes two files in the working directory:

- **`rustcfml-profile.svg`** — an interactive flamegraph (open in a browser).
- **`rustcfml-profile.pb`** — a pprof protobuf, loadable in `go tool pprof`,
  [speedscope](https://www.speedscope.app/), or Grafana Pyroscope.

`--profile` also works with **`--serve`**:

```bash
./target/release/rustcfml --serve ./www --profile
# drive load, then Ctrl+C — the flamegraph is written on graceful shutdown
```

In serve mode the sampler is **process-wide** (a `SIGPROF` timer over all worker
threads), so the flamegraph is an **aggregate** of CPU across every request
served during the window — the standard "profile the server under load" view, not
a single request. Idle runtime/park frames appear too (filter them out when
reading, or lean on the blocklist). Because sampling has a small always-on cost,
this is an ad-hoc "profile for a few minutes, then Ctrl+C" tool; **continuous**
serve-mode profiling (the Grafana Pyroscope SDK, route-tagged, at lower rates) is
a documented follow-up.

## Ops / production

Tail sampling (keep only slow/errored traces, off the app host) and a ready-to-run
Collector + Tempo + Grafana stack are covered in
[observability-ops.md](observability-ops.md).

## Notes & limitations

- The footer is a web-page artifact and auto-renders on web requests only; in
  CLI runs the data is still collected and reachable via `getDebugData()`.
- Per-template **Load** (compile/startup) time is not yet broken out separately —
  it folds into Application time.
- The remaining observability roadmap layer — a DAP step debugger — is designed
  and builds on the same hook bus, but is not yet shipped.
- OTLP **metric** push (traces already push over OTLP), continuous native
  profiling (Pyroscope), and `<cftransaction>` spans are documented follow-ups;
  the `TxnEvent` hook is wired into the bus but not yet emitted.
