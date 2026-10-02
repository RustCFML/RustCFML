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
container it runs in. ([Memory management](memory.md) covers how the engine frees
memory and what you can measure.)

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
freed the allocator keeps a plateau of about 1.6× the live data (see
[Memory management](memory.md#allocator-retention)). A Preside site idling at ~450 M sits at
750–850 M after a few reloads; `auto` in a 2 G container (1.5 G) is right for
it, a 700 M limit is not.

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

The reference image lives in its own repo, **[RustCFML-Docker](https://github.com/RustCFML/RustCFML-Docker)**, and is published to the GitHub container registry:

```
ghcr.io/rustcfml/rustcfml:<tag>
```

It is multi-arch (`linux/amd64`, `linux/arm64`), around 36 MB, runs as a non-root user, stops cleanly on `docker stop`, and checks for native extensions (`.rcx`) before the server starts. The base is Chainguard Wolfi, so anything the app needs at the OS level is an `apk add` away.

Mount a webroot at `/app` and run it:

```bash
docker run --rm -p 8500:8500 -v "$PWD/webroot:/app" ghcr.io/rustcfml/rustcfml
```

The default mode is `production`, so everything is cached until restart. For local development, where edits should appear without one:

```bash
docker run --rm -p 8500:8500 -v "$PWD/webroot:/app" -e RUSTCFML_MODE=dev ghcr.io/rustcfml/rustcfml
```

To bake an application into an image, use it as a base:

```dockerfile
FROM ghcr.io/rustcfml/rustcfml:v0.653.3
COPY --chown=nonroot:nonroot webroot/ /app/
RUN rustcfml-warm-extensions   # only if the app ships .rcx files
```

`rustcfml-warm-extensions` extracts native extensions at build time, so the first container start does not pay for it and a broken or wrong-platform archive fails the **build** rather than the deploy.

A standalone web-application binary (below) copied into a `scratch`/`distroless` base remains a valid alternative, since RustCFML has no runtime dependencies.

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

Run RustCFML at the edge by compiling to WebAssembly.

> **There is no `rustcfml` flag for this target.** The CLI has no `--wasm` option, and the edge build is not produced by the `rustcfml` binary at all. You deploy from the worker host repo with `wrangler`, which builds the WASM module and carries the bindings.

The Worker integration (Hyperdrive datasources, KV/R2/Durable Objects, session storage) lives in a separate repo:

- **[RustCFML-Cloudflare-worker](https://github.com/RustCFML/RustCFML-Cloudflare-worker)**

The short version, once `wrangler` and `worker-build` are installed and `wrangler.toml` carries your binding ids:

```bash
cargo install worker-build
npm install                          # postgres / mysql2, for the Hyperdrive path
wrangler kv namespace create SESSIONS # paste the id into wrangler.toml
wrangler deploy
```

Full setup, including Hyperdrive provisioning and the JSPI architecture behind `<cfquery>`, is in that repo's README. See **[WebAssembly](wasm.md)** for the WASM target generally.

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

How the engine frees memory (reference counting, the cycle collector, the
allocator), what you can measure, the collector's tunables and diagnostics, and
mimalloc's retention options are all in **[Memory management](memory.md)**.
