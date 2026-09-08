# Web Server

[← Back to README](../README.md)

Serve `.cfm` files over HTTP with full CFML web scopes (CGI, URL, Form, Cookie, Session, Application, Request):

```bash
rustcfml --serve                           # Current dir on port 8500
rustcfml --serve ./mywebroot --port 3000   # Custom root and port
rustcfml --serve ./mywebroot --socket      # Bind a Unix socket (default /run/rustcfml.sock)
```

The server is built on [Axum](https://github.com/tokio-rs/axum) with concurrent request handling. It serves `.cfm` files and static assets from the document root. Directory requests serve `index.cfm` if present. Path-info routing is supported (`/index.cfm/users/123` resolves to `index.cfm` with path info `/users/123`). Bytecode caching skips recompilation for unchanged files across requests.

For deploying the server (production caching, Docker, standalone binaries), see **[Deployment](deployment.md)**.

## Listening: TCP port or Unix socket

By default the server listens on a TCP port (`--port`, default `8500`, bound to `0.0.0.0`). For production deployments behind a reverse proxy on the same host, it can instead listen on a **Unix domain socket** — this avoids the loopback TCP stack entirely (no ephemeral-port churn, no `TIME_WAIT` pressure) and is measurably faster under load.

```bash
rustcfml --serve ./mywebroot --socket                    # default path: /run/rustcfml.sock
rustcfml --serve ./mywebroot --socket /var/run/app.sock  # explicit path
```

Behaviour:

- **`--socket` overrides `--port`.** If both are given, the socket wins and the port is ignored.
- A bare `--socket` (no path) uses `/run/rustcfml.sock`.
- On startup, any **stale socket file** at the path is removed before binding, so a crashed previous run won't block a restart.
- On a clean shutdown (Ctrl+C / `SIGINT`), the socket file is **removed**.
- Unix sockets are a Unix-only feature (Linux/macOS). On other platforms `--socket` exits with an error — use `--port`.
- `cgi.remote_addr` is reported as `127.0.0.1` for socket connections (peers over a Unix socket have no IP); set `X-Forwarded-For` in your proxy and read it if you need the real client address.

Quick local check:

```bash
curl --unix-socket /run/rustcfml.sock http://localhost/index.cfm
```

See **[Deployment → Behind a reverse proxy (nginx + Unix socket)](deployment.md#behind-a-reverse-proxy-nginx--unix-socket)** for a complete nginx configuration and a load comparison.

## Configuration (`.cfconfig.json`)

Drop a `.cfconfig.json` at the webroot to configure datasources, mappings, mail, security policies, error handling, and more. The format follows the Ortus CFConfig filename convention with a BoxLang-style flat schema, so the same file works across CommandBox/Lucee, BoxLang, and RustCFML — engine-specific keys are silently ignored. Secrets can use environment-variable substitution.

cfconfig is **application-level**: the webroot file is the server *baseline* (or set one explicitly with `--cfconfig <path>` / the `CFCONFIG` env var), and any `.cfconfig.json` sitting beside an `Application.cfc` is overlaid on top of it per request — so each application under the server can tune its own runtime, datasources, and security. The listening **port is not a cfconfig setting** — it is set with `--port` (pages read `cgi.server_port`), or replaced entirely by `--socket` to listen on a Unix domain socket (see [below](#listening-tcp-port-or-unix-socket)).

See **[Configuration](configuration.md)** for the full reference.

## Application.cfc lifecycle

If an `Application.cfc` file exists in the document root (or any parent directory), it is automatically loaded and its lifecycle methods are called:

- `onApplicationStart()` — runs once when the application is first accessed
- `onRequestStart(targetPage)` — runs before each request
- `onRequest(targetPage)` — handles the request (replaces default page execution)
- `onRequestEnd(targetPage)` — runs after each request
- `onError(exception, eventName)` — handles uncaught errors

Application state (`application` scope) persists across requests in serve mode. Component mappings defined via `this.mappings` in `Application.cfc` are supported for virtual path resolution.

## Sessions

For the full picture — the `session` scope, the lazy-creation default, the
`CFID` cookie and `this.sessioncookie` hardening, storage backends, and expiry —
see **[Sessions](sessions.md)**. The distributed backends are summarised below.

### Distributed sessions

RustCFML supports two pluggable session backends beyond the in-process default, both built into the stock binary and selected via `.cfconfig.json`:

- **Memcached** — sessions stored in an external Memcached cluster. Lucee-compatible config shape.
- **Cluster** — gossip-based peer-to-peer replication across native RustCFML nodes using [memberlist](https://github.com/al8n/memberlist) for membership and [Automerge](https://automerge.org) CRDTs for conflict-free merging. Suitable for LAN or WAN deployments up to a few dozen nodes; no external store required.

Both backends share the same `sessionStorage` / `caches` keys in `.cfconfig.json`, so the configuration shape carries across Lucee and BoxLang. Switching backends is a config-only change — no rebuild needed. See the **[`caches` and `sessionStorage` section of Configuration](configuration.md#caches-and-sessionstorage)** for the full reference, a three-node walkthrough, and a troubleshooting table.

## Streaming responses with `<cfflush>`

By default a request's output is buffered and sent as one response when the page
finishes, which is what lets `<cfheader>`, `<cfcontent>` and `<cflocation>` be
decided at any point during the page.

`<cfflush>` opts out of that for a single request. The first flush **commits**
the response — the status line and headers are sent with that chunk — and
everything the page produces afterwards is streamed as it is produced, using
chunked transfer-encoding with no `Content-Length`. Use it for slow reports,
progress output, or server-sent-event style endpoints:

```cfml
<cfoutput>Starting…</cfoutput>
<cfflush>

<cfloop query="bigResult">
    <cfoutput>#bigResult.name#<br></cfoutput>
    <cfflush>
</cfloop>
```

`<cfflush interval="8192">` sets it and forgets it: output is flushed
automatically whenever the buffer passes that many bytes, for the rest of the
request.

Because committing the response is irreversible, once you have flushed:

| after `<cfflush>` | behaviour |
|---|---|
| another `<cfflush>`, `<cfabort>` | fine |
| `<cfcookie>` | accepted but ineffective — the header has already gone |
| `<cfheader>`, `<cfcontent>`, `<cflocation>`, `<cfhtmlhead>`, `<cfhtmlbody>` | raise an error |

So set your status, headers and cookies *before* the first flush. A page that
never calls `<cfflush>` is completely unaffected and keeps its `Content-Length`.

`isFlushed()` tells you which side of that line you are on, so shared code can
branch instead of guessing:

```cfml
<cfif not isFlushed()>
    <cfheader statuscode="404" statustext="Not Found">
</cfif>
```

Behaviour matches Lucee, including a flush inside `<cfsavecontent>`, which
pushes the page output written before the capture and leaves the capture itself
intact. The exact semantics and the two paths where a flush degrades back to
buffering are in **[known issues §87](known-issues.md)**.

## URL rewriting

Place a `urlrewrite.xml` file in your document root for Tuckey-compatible URL rewriting. This enables clean URLs and REST-style routing:

```xml
<?xml version="1.0" encoding="utf-8"?>
<urlrewrite>
    <rule>
        <from>^/([a-zA-Z][a-zA-Z0-9_/-]*)$</from>
        <to>/index.cfm/$1</to>
    </rule>
    <rule>
        <from>^/old-page$</from>
        <to type="permanent-redirect">/new-page</to>
    </rule>
</urlrewrite>
```

Supported features:

- **Regex and wildcard patterns** with backreference substitution (`$1`, `$2`)
- **Forward**, **redirect** (302), and **permanent-redirect** (301) actions
- **Conditions** on HTTP method, port, and headers
- **Rule chaining** with `last="true"` to stop processing
