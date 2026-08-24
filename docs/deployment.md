# Deployment

[← Back to README](../README.md)

RustCFML deploys as a single artifact in several shapes. Two cross-cutting capabilities — **production mode** (warm caching) and the **sandbox / virtual filesystem** — apply to both web and CLI deployments and are documented at the end of this page.

## Web application

Run the built-in server behind your reverse proxy. Enable production caching for deployment:

```bash
rustcfml --serve ./mywebroot --production
```

See **[Web Server](web-server.md)** for serve-mode details, `Application.cfc` lifecycle, sessions, and URL rewriting.

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

## Memory footprint (allocator tuning)

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
