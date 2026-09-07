# Deployment

[← Back to README](../README.md)

RustCFML deploys as a single artifact in several shapes. Two cross-cutting capabilities — **production mode** (warm caching) and the **sandbox / virtual filesystem** — apply to both web and CLI deployments and are documented at the end of this page.

## Web application

Run the built-in server behind your reverse proxy. Enable production caching for deployment:

```bash
rustcfml --serve ./mywebroot --production
```

See **[Web Server](web-server.md)** for serve-mode details, `Application.cfc` lifecycle, sessions, and URL rewriting.

## Memory limit (`--max-memory`)

Give the process a ceiling the way you would give a JVM `-Xmx`, sized to the
container it runs in:

```bash
rustcfml --serve ./mywebroot --production --max-memory 1.5G   # or 1536M
rustcfml --serve ./mywebroot --production --max-memory auto   # 75% of the cgroup limit
RUSTCFML_MAX_MEMORY=1.5G rustcfml --serve ./mywebroot --production
```

It measures real physical footprint: cgroup `memory.current` inside a container,
the process's resident footprint otherwise, and enforces the limit in two tiers.

**At 85% — back-pressure.** The server stops admitting new requests: they get
**503 + `Retry-After: 2`**, which a load balancer or orchestrator treats as
back-pressure rather than a failure. Meanwhile it sheds (a cycle-collector sweep
and a return of retained allocator pages) and lets in-flight requests finish.
Admission reopens once the footprint is back under.

**At 95% — abort the runaway.** Back-pressure cannot help against the case the
limit is really for: one request allocating without bound. Nothing new arrives to
be refused, and the request already inside runs until the OOM killer takes the
whole process down. So a watchdog aborts **the in-flight request that has
allocated the most**, with an error its own `try/catch` cannot swallow. Every
other request keeps running, and the server keeps serving.

The aborted client gets a plain **500** — deliberately not a 503, because a
runaway request should not be retried elsewhere — and the reason goes to the
server log, naming the request and the evidence:

```
[max-memory] footprint 684M is over the hard limit 665M of 700M — aborting
request #2 [/app/report.cfm], the largest allocator (499915 tracked containers).
Other requests continue.
```

**It will not abort a request that did not build the heap.** A request is only
eligible once it has itself allocated more than 100,000 tracked containers. When
memory is held by the application scope, the caches, or the allocator's retained
pages, no request is responsible, killing one would free nothing, and the log
says so instead:

```
[max-memory] footprint 765M is over the hard limit 475M of 500M, but none of the
1 in-flight request(s) has allocated enough to be responsible — the memory is
held by the application, the caches or the allocator. Not aborting anything;
shedding instead.
```

The abort is polled at every user-function call and at the blocking boundaries
(`sleep`, `cfhttp`, `queryExecute`), so a runaway is normally stopped within a
watchdog tick. A request that allocates in a tight loop calling no function of
its own is the exception — the same gap `requestTimeout` has, and for the same
reason.

**Sizing.** Size for roughly **twice the steady-state footprint**. Two things
need the headroom: for ~15 s after a `?fwreinit=true`-style reload two
application generations are legitimately resident, and after the old one is
freed the allocator keeps a plateau of about 1.6× the live data (see *Memory
tuning* at the end of this page). A Preside site idling at ~450 M sits at
750–850 M after a few reloads; `auto` in a 2 G container (1.5 G) is right for
it, a 700 M limit is not. There is no per-request abort yet: a single runaway
request can still take the process to the limit, at which point everything else
is refused until it finishes.

## Stopping the server (SIGINT / SIGTERM)

`--serve` shuts down gracefully on **SIGINT** (Ctrl+C) and **SIGTERM**: it stops
accepting new connections, lets in-flight requests finish and send their
responses, then exits. `docker stop`, `kubectl delete pod` and `systemctl stop`
all send SIGTERM, so a rolling deploy loses no requests and the process exits as
soon as the last one drains rather than waiting out the grace period.

Set the grace period longer than your slowest request so the platform never has
to SIGKILL a draining server (`docker stop -t 30`, or `terminationGracePeriod
Seconds: 30` in Kubernetes).

> Handled since v0.653.14. Before that only SIGINT was handled, which went wrong
> in two ways: outside a container the default action killed the process on the
> spot, cutting off in-flight requests; and as PID 1 in a container the kernel
> installs no default disposition, so SIGTERM was ignored entirely and every stop
> took the full grace period followed by SIGKILL. Images built for an older
> engine work around it with `STOPSIGNAL SIGINT`, which is harmless to keep.

## Behind a reverse proxy (nginx + Unix socket)

In production you typically run RustCFML behind a reverse proxy (nginx, Caddy, HAProxy) that terminates TLS, serves static assets, and load-balances. When the proxy and RustCFML run on the **same host** — the common single-box and containerised setup — **a Unix domain socket is the recommended way to connect them**, in preference to a loopback TCP port.

**Why a socket rather than `127.0.0.1:8500`?** When nginx proxies to RustCFML, every upstream request that isn't served from a warm keep-alive connection opens a fresh transport connection. Over loopback TCP that means the full TCP path — three-way handshake, an ephemeral source port allocated per connection, and a `TIME_WAIT` entry left behind on close. Under sustained load those add up: ephemeral ports are a finite range that can exhaust, and `TIME_WAIT` build-up adds latency and can stall new connections. A Unix domain socket sidesteps all of it — it's kernel-local IPC with no handshake, no port allocation, and no `TIME_WAIT`, so it stays flat as concurrency rises where the loopback-TCP path degrades. It also can't be reached from off-box, so the app server isn't accidentally exposed, and access is governed by ordinary filesystem permissions on the socket file. In our testing the socket path consistently matched direct-serve throughput while loopback TCP trailed it under load (see the comparison below).

Reach for loopback TCP instead only when the proxy and RustCFML are on **different hosts** (where a socket isn't an option), or when a tool in front genuinely can't address a Unix socket.

Start RustCFML on a socket:

```bash
rustcfml --serve /srv/mywebroot --production --socket /run/rustcfml.sock
```

(`--socket` overrides `--port`; a bare `--socket` defaults to `/run/rustcfml.sock`. Stale socket files are cleared on start and removed on clean shutdown — see **[Web Server → Listening](web-server.md#listening-tcp-port-or-unix-socket)**.)

Point nginx at the socket. The `keepalive` pool and `proxy_http_version 1.1` keep upstream connections warm, which matters for throughput:

```nginx
upstream rustcfml {
    server unix:/run/rustcfml.sock;
    keepalive 128;
}

server {
    listen 80;
    server_name example.com;

    # Serve static assets directly from nginx; proxy everything else.
    root /srv/mywebroot;

    location / {
        try_files $uri @cfml;
    }

    location @cfml {
        proxy_pass http://rustcfml;
        proxy_http_version 1.1;
        proxy_set_header Connection "";              # reuse upstream keep-alive connections
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Because Unix-socket peers have no IP, RustCFML reports `cgi.remote_addr` as `127.0.0.1`; read `X-Forwarded-For` (set above) if your application needs the real client address.

**Permissions:** the socket inherits the user/umask of the RustCFML process. nginx must be able to read/write it — run both as the same user, or `chmod`/`chown` the socket (e.g. group-accessible) so the nginx worker can connect.

### Reverse-proxy throughput

A reverse proxy costs a little raw throughput (RustCFML can serve faster when hit directly), but the proxy↔app **transport** matters: a Unix socket consistently beats TCP loopback, and the gap widens as concurrency climbs.

"Hello World" `.cfm` page, `--production` mode, Apple M-series, ApacheBench with keep-alive (`-k`), 8s runs, requests/sec — higher is better:

| Concurrency | Direct TCP (no proxy) | nginx → Unix socket | nginx → TCP loopback |
|---|---|---|---|
| `-c 1`   | 2,673 | 2,504 | 2,272 |
| `-c 10`  | 16,900 | 14,700 | 13,500 |
| `-c 50`  | 19,000 | 16,300 | 14,700 |
| `-c 100` | 19,100 | 16,600 | 11,800 |
| `-c 200` | 17,000 | 16,600 | 10,850 |

As concurrency climbs, the **Unix-socket path tracks direct-serve almost exactly while TCP loopback falls away**: at `-c 100` nginx→socket sustains ~16,600 req/s vs ~11,800 over TCP loopback (~40% more), and at `-c 200` the gap widens to ~16,600 vs ~10,850 (~53% more) — the socket path costs almost nothing over hitting RustCFML directly (~16,600 vs ~17,000), whereas TCP loopback saturates flat at ~10,850 under ephemeral-port/`TIME_WAIT` pressure. Without client keep-alive the same ordering holds (nginx→socket ~5,500 vs nginx→TCP ~4,100 at `-c 50`). Numbers are from a single dev machine and are illustrative — measure on your own hardware — but the ranking is consistent across runs.

See **[Performance](performance.md)** for the direct-serve methodology these build on.

## Docker

*Coming soon* — an optimised, minimal container image for running RustCFML web applications. Until then, a standalone web-application binary (below) copied into a `scratch`/`distroless` base works well, since RustCFML has no runtime dependencies.

## CLI tools

Build a command-line tool from a CFML app. Arguments are available via the `cli` scope, which works like CFML's `arguments` scope — named keys for flags, 1-based numeric keys for positional args.

```bash
rustcfml --build ./myapp -o greet --mode cli --entry main.cfm
```

**myapp/main.cfm:**

```cfml
<cfscript>
name = cli.name ?: "World";
writeOutput("Hello, #name#!" & chr(10));

// Positional args: cli[1], cli[2], ...
for (i = 1; i <= structCount(cli); i++) {
    if (isNumeric(i) && structKeyExists(cli, i))
        writeOutput("  arg #i#: #cli[i]#" & chr(10));
}
</cfscript>
```

```bash
./greet                     # Hello, World!
./greet --name Alex         # Hello, Alex!
./greet foo bar             # positional: cli[1]="foo", cli[2]="bar"
```

## Self-contained web binaries

Package a web application as a single binary with an embedded HTTP server — no runtime dependencies, no source files to deploy.

```bash
rustcfml --build ./webapp -o myserver --mode serve
```

```bash
./myserver                          # Foreground on port 8500
./myserver --port 3000              # Custom port
./myserver start --port 3000        # Daemonize (background)
./myserver status                   # Check if running
./myserver stop                     # Graceful shutdown
```

### Binary sizes

| Build | Size |
|---|---|
| Release binary (no app) | ~13 MB |
| + small web app | ~13 MB |
| + large app (100+ files) | ~13–15 MB |

No JRE, no runtime, no dependencies. Compare: Lucee/BoxLang require a 200+ MB JRE.

### Licensing a binary you distribute

A `--build` binary statically links the same ~560 third-party Rust crates as
RustCFML itself, and their MIT/BSD/ISC/Apache-2.0 terms require the copyright
notices to travel with anything you ship. RustCFML embeds those notices for you
— your binary answers:

```bash
./myapp --licenses
```

That prints RustCFML's MIT licence plus the full third-party attribution. It
covers the engine and its dependencies only; the licence for your own CFML code
and any [native modules](native-modules.md) you add is yours to declare.

### Native (Rust) modules

Self-contained binaries can include user-authored Rust code that surfaces as first-class CFML built-ins and classes. See **[Native Modules](native-modules.md)**.

## Cloudflare Workers

Run RustCFML at the edge by compiling to WebAssembly. The Worker integration (Hyperdrive datasources, KV/R2/Durable Objects, session storage) lives in a separate repo:

- **[RustCFML-Cloudflare-worker](https://github.com/RustCFML/RustCFML-Cloudflare-worker)**

See **[WebAssembly](wasm.md)** for the WASM target generally.

## Production mode (web and CLI)

By default the server re-validates files on each request: it walks up from the page directory to find `Application.cfc`, stats the resolved file, and stats every cached bytecode entry to detect source changes. This keeps the dev loop hot — edit a file and refresh.

Passing `--production` (or setting `RUSTCFML_PRODUCTION=1`) enables three in-memory caches that persist for the server's lifetime:

- **Application.cfc path resolution** — the first request walks the directory tree; subsequent requests hit a hashmap. Negative results (no `Application.cfc` anywhere in the chain) are cached too.
- **URL → file resolution** — `is_file` stats from request routing are memoized.
- **Bytecode cache trust** — the per-hit `mtime` check on every compiled file is skipped.

Net effect: requests pay zero filesystem IO once the cache is warm — typically a 3–4× throughput gain on an app with `Application.cfc` + cfincludes. Files added or modified on disk are not picked up until the server is restarted. See **[Performance](performance.md)** for measured numbers.

## Sandbox / virtual filesystem (web and CLI)

Self-contained binaries can run in **sandbox mode**, which completely isolates the application from the host filesystem. Sandbox mode also enables production caching automatically, since the embedded VFS is immutable at runtime.

```bash
./myserver --sandbox                # No host filesystem access
./myserver --sandbox --port 3000    # Sandbox + custom port
```

In sandbox mode:

- **Embedded files are readable** — `fileRead()`, `fileExists()`, `directoryList()`, `expandPath()`, and `include` all work against the embedded virtual filesystem. Your application can read its own bundled config files, templates, and assets normally.
- **Host filesystem is invisible** — `fileExists("/etc/passwd")` returns `false`; `fileRead()` on any host path returns "file not found". The application cannot discover or read files outside the embedded archive.
- **All writes are blocked** — `fileWrite()`, `fileAppend()`, `fileDelete()`, `directoryCreate()`, and other write operations throw *"filesystem writes are disabled in sandbox mode"*.

Even if application code is compromised (e.g. via a code-injection vulnerability), the attacker cannot read sensitive host files, write persistent backdoors/web shells, or modify host files. The embedded virtual filesystem is **read-only and non-persistent** — any state the application needs to persist should use external services (databases, APIs).

## Memory tuning (advanced)

Most deployments need nothing from this section: set `--max-memory` and stop.
It exists for the two questions that come up when a footprint graph looks
wrong — *is this a leak?* and *can I make it smaller?* — and for the knobs that
answer them.

### How memory behaves

- **Acyclic data is freed the moment its last owner drops it.** Values are
  reference-counted, so a request's arrays, structs and strings go back to the
  allocator as frames return; there is no pause and nothing to tune.
- **Only cycles need the collector.** A CFC instance is inherently cyclic
  (`this → variables → this`), so the cycle collector runs a trial-deletion
  sweep over the containers a request allocated: incrementally during a long
  request, always at request end, and across requests for anything (an
  application-scope graph, say) that became garbage after the request that made
  it. A framework reload drops a whole generation of singletons at once; the
  collector notices the displacement and sweeps within ~15–30 s.
- **Footprint sits above live data.** After a reload the freed generation leaves
  the allocator's segments partially used, so the resident footprint plateaus
  at roughly 1.6× the live heap and stays there. That plateau is not a leak:
  the live heap, measured with the profiler below, moves by a few MB per reload.
  A leak is a footprint that keeps climbing reload after reload with no plateau.

### Collector tunables

Defaults were chosen by measurement on real applications and should not
normally be changed. They are environment variables, read once at startup.

| variable | default | what it controls |
|---|---|---|
| `RUSTCFML_GC_INCREMENTAL` | `100000` | Young-generation budget: how many new containers a request may allocate before a mid-request (minor) sweep. Lower = less transient garbage held, more sweeps. `0` disables mid-request sweeping (request-end only). Measured on a 2,737-spec test suite: 25k / 50k / 100k gave 5.8 / 6.1 / 4.1 s of sweeps with the same peak memory. |
| `RUSTCFML_GC_PERSISTENT` | `50000` | Base budget for the cross-request sweep over survivors carried between requests; the sweep runs when that set has doubled. `0` disables carrying survivors (not recommended: anything that becomes garbage after its request then leaks). |
| `RUSTCFML_GC_DISPLACE_SWEEP_MIN` | `1000` | Minimum size of a displaced graph (a key overwritten or deleted that held that many containers) for the request end to trigger a sweep immediately rather than wait for the budget. `0` disables the trigger. |
| `RUSTCFML_RELOG_BUDGET` | `20000` | Most containers a single overwrite or delete may re-enter into the collector's log. Bounds the cost of the mutation hook. |
| `RUSTCFML_GC_LOG_CAP` | `4000000` | Hard cap on the per-request log (~16 bytes an entry, so ~64 MB). At the cap the log is compacted; only a request holding more distinct containers than that pauses logging for its remainder. |
| `RUSTCFML_MAX_MEMORY` | unset | Same as `--max-memory` (see above). |

### Diagnostics

Each of these prints to stderr at request end and is off unless set. They cost a
pass over the data they report on, so use them on a test instance, not in
production.

| variable | prints |
|---|---|
| `RUSTCFML_GC_DEBUG=1` | Every sweep: entries examined, nodes reclaimed, live count, timing; request-end log size by container type; displacement and cross-request sweep decisions. The first place to look when a footprint climbs. |
| `RUSTCFML_CACHE_CENSUS=1` | Sizes of the process-wide caches (bytecode, component paths, canonicalised paths, interned names), live component blueprints per class, and each application scope's approximate size with a breakdown by shape (component instances, metadata structs, functions, strings). Answers *what is holding the heap*. |
| `RUSTCFML_GC_ROOTS=N` | For the cross-request sweep, the N largest survivors with the reference that pins each (which scope or key holds it). Answers *why is this generation still alive*. |
| `--memprofile` | A sampling heap profiler (release binary built with `--features memprofile`): `kill -USR2 <pid>` dumps live and total allocations by call stack as `.folded` files for a flame graph. Answers *which code allocated what is live*. |

A useful habit: read `RUSTCFML_GC_DEBUG` first. If tracked nodes return to the
same count after each reload while footprint climbs, it is allocator retention
(below), not the engine; if the count climbs, `RUSTCFML_GC_ROOTS` names the holder.

### Allocator retention

Since v0.590.0 the release binary uses **mimalloc** as its global allocator, which
is worth roughly 15% on a warm request. mimalloc retains OS arenas rather than
returning them promptly, so a server's resident size settles at a plateau above
its live data — a rounding error on a large application, but visible on a small
one, where it can roughly double the idle footprint.

Two stock mimalloc options recover most of that. They are read by the allocator
itself, so they work on the binary as shipped, with no rebuild:

```bash
MIMALLOC_ARENA_EAGER_COMMIT=0   # don't commit arena memory up front
MIMALLOC_PURGE_DELAY=0          # purge freed memory immediately, not after 10ms
```

**They are not the default, and the trade-off is real.** `PURGE_DELAY=0` hands
every freed block back to the OS immediately, which is exactly what an
allocation-heavy request does constantly — large query results, report
generation, big JSON. Measured here on macOS arm64, `--production`, `ab -c4`,
three interleaved rounds:

| workload | | req/s | server CPU | RSS |
|---|---|---|---|---|
| 4k-row query → structs → JSON → 3k-line report | defaults | **178** | 6.58 s | 190 MiB |
| | both options | **122** | 9.59 s | 191 MiB |
| trivial page | defaults | **6541** | 0.10 s | 69 MiB |
| | both options | **5846** | 0.17 s | 69 MiB |

That is −31% throughput and +46% CPU on the allocation-heavy workload, with no
RSS saving at all on that shape — and −11% even on a trivial page.

The saving is equally workload-dependent in the other direction: the same two
options took a large, long-lived workload (the engine's own test suite in serve
mode) from 501 MiB to 453 MiB, and GH
[#354](https://github.com/RustCFML/RustCFML/issues/354) reports 156 → 101 MiB on
a routing-heavy application on Linux with throughput unchanged.

So: reach for them when idle RSS is the constraint and the application is
routing- or IO-bound, and measure your own allocation-heavy endpoints before
committing. Both are ordinary environment variables, so they can be set per
deployment without touching the binary.
