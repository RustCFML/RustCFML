# `.cfconfig.json` — RustCFML Configuration

RustCFML reads an optional `.cfconfig.json` file at startup. The format follows
the [Ortus CFConfig](https://cfconfig.ortusbooks.com/) filename convention and
the BoxLang-style flat, declarative layout, so the same file can be shared with
CommandBox/Lucee and CommandBox/BoxLang projects — RustCFML silently ignores
any keys it doesn't recognise.

All fields are optional. When the file is missing, compiled-in defaults apply.

## Two tiers: server baseline + per-application overlay

cfconfig is **application-level**. There are two tiers:

1. **Server baseline** — one config loaded at process startup. Set it explicitly
   with `--cfconfig <path>` (or the `CFCONFIG` environment variable), otherwise
   it is discovered (see the table below). It supplies defaults for every
   application and owns the `server.*` section (host, welcome files, body size).
2. **Per-application overlay** — a `.cfconfig.json` sitting **beside an
   `Application.cfc`** is auto-discovered per request and overlaid on top of the
   baseline for that request. Only the keys present in the app file override the
   baseline; everything else is inherited. Because one server can host many
   applications (each is the nearest `Application.cfc` walking up from the
   requested page), each application can carry its own config.

   The app overlay's **`server.*` section is ignored** — server settings (and the
   listening **port** in particular) are a server/environment concern, never an
   application-level setting. An app file cannot change the port; pages read the
   live port from `cgi.server_port`.

### Baseline file resolution

When `--cfconfig` / `CFCONFIG` is not set, the baseline is found by first match:

| Mode | Search order |
|---|---|
| `--serve` | webroot → cwd → directory of the `rustcfml` binary |
| CLI (`rustcfml file.cfm`) | entry file's directory → cwd → binary directory |
| `--build` self-contained binary | external file next to the binary → copy embedded into the VFS at build time → defaults |

CLI flags (`--port`, `--serve <path>`, `--sandbox`, `--cfconfig`) always win over
file contents. The baseline is read once at process startup; restart the server
to pick up baseline changes. Per-application overlays are re-read per request
(subject to the production-mode resolution cache).

## Environment variable substitution

Every string value supports `${VAR:default}` placeholders, expanded once after
parse. The syntax matches Lucee's `.CFConfig.json` importer, so one file can
serve both engines:

```jsonc
"host":     "${DB_HOST:localhost}"     // env var with fallback
"password": "${DB_PASSWORD}"           // empty string if unset
```

A name is resolved in this order:

1. environment variable with that exact name
2. environment variable with `.` → `_` and upper-cased, so `${my.setting}` also
   finds `MY_SETTING` (Lucee does the same)
3. the fallback after the first `:`, or an empty string if there is none

Lucee's extra step — a Java system property between (1) and (2) — has no
equivalent here and is skipped. Only the first `}` closes a placeholder, an
unterminated `${` is left verbatim, and expansion is single-pass: a value that
itself contains `${...}` is not re-scanned.

**Nested placeholders are deliberately not supported.** A resolved value that
contains `${...}` is left exactly as it is, at any offset and to any depth.
Recursive expansion of environment-supplied text is an abuse surface — it lets
one env var inject a reference that pulls in another, so "set `DB_PASSWORD`"
quietly becomes "set `DB_PASSWORD` and thereby read anything else in the
process's environment" — and it buys nothing a flatter config doesn't. Lucee is
inconsistent rather than recursive here (it re-expands a nested `${` at offset
≥ 1 and skips one at offset 0, an artifact of its scan loop); we match it in the
skip case and refuse in the other. See §39 of `docs/known-issues.md` and
GH [#306](https://github.com/RustCFML/RustCFML/issues/306).

A placeholder that resolves to nothing and has no fallback becomes an empty
string; it is not left verbatim.

### Legacy `${env.VAR}` form

Before v0.548.0 RustCFML required an `env.` namespace prefix
(`${env.DB_HOST:localhost}`). Those configs still work — an unresolved name
beginning with `env.` falls back to looking up the remainder — but the prefix is
deprecated and should be dropped, because **Lucee reads it wrongly rather than
loudly**: Lucee treats `env.DB_HOST` as the whole variable name, never finds it,
and silently uses the default. `${env.DB_HOST:localhost}` therefore connects to
`localhost` on Lucee in every environment. Write `${DB_HOST:localhost}` instead.

## HTTP protection

In web server mode, requests for `.cfconfig*`, `.env`, `*.lex`, and anything
matching `security.blockedPaths` return **HTTP 404** (not 403, to avoid
confirming the file's existence).

## Example

A realistic production file:

```json
{
  "server": {
    "host": "0.0.0.0"
  },
  "runtime": {
    "locale": "en-GB",
    "timezone": "Europe/London",
    "trustedCache": true
  },
  "datasources": {
    "myapp": {
      "driver":   "mysql",
      "host":     "${DB_HOST:localhost}",
      "port":     "${DB_PORT:3306}",
      "database": "${DB_NAME:myapp}",
      "username": "${DB_USER:root}",
      "password": "${DB_PASS}",
      "default":  true
    }
  },
  "mailServers": [
    {
      "smtp":     "${SMTP_HOST}",
      "port":     587,
      "username": "${SMTP_USER}",
      "password": "${SMTP_PASS}",
      "tls":      true
    }
  ],
  "mappings": {
    "/mylib": "/app/lib"
  },
  "debugging": {
    "enabled": false,
    "errorTemplate": "/errors/500.cfm"
  },
  "security": {
    "disallowedFunctions": ["cfexecute"]
  }
}
```

## Sections and keys

### `server`

Server-level only — taken from the **baseline**, never from a per-application
overlay. There is intentionally **no `port` key**: the listening port is set with
`--port` (default `8500`); pages read the live port from `cgi.server_port`.

| Key | Type | Default | Notes |
|---|---|---|---|
| `host` | string | `0.0.0.0` | Bind address. `0.0.0.0` = all interfaces; set `127.0.0.1` to accept connections from this machine only |
| `webroot` | string | `""` | Document root. Overridden by `--serve <path>` |
| `welcomeFiles` | string[] | `["index.cfm", "index.htm", "index.html"]` | Tried in order for directory requests |
| `cfmlExtensions` | string[] | `["cfm", "cfc"]` | Extensions dispatched through the interpreter |
| `maxRequestBodySize` | int (bytes) | `10485760` | `0` = unlimited |
| `maxConcurrentRequests` | int | `0` | `0` = unlimited (reserved; not enforced yet) |
| `requestTimeout` | int (sec) | `0` | `0` = no timeout (reserved; not enforced yet) |

### `runtime`

| Key | Type | Default | Notes |
|---|---|---|---|
| `nullSupport` | bool | `false` | Unset variables return null vs `""` |
| `dotNotationUpperCase` | bool | `true` | Force upper-case struct keys (classic CF) |
| `locale` | string | `""` | IETF BCP 47 (e.g. `en-GB`). Empty = system |
| `timezone` | string | `""` | IANA tz name. Empty = system |
| `whitespaceCompressionEnabled` | bool | `false` | Global `cfsetting enableCFOutputOnly=true` |
| `trustedCache` | bool | `false` | Skip recompile when template mtime unchanged |
| `reportAsLucee` | bool | `false` | Report `server.coldfusion.productname` as `"Lucee"` instead of `"RustCFML"`. RustCFML targets the Lucee dialect and always advertises `server.lucee`, but some frameworks (e.g. ColdBox's mapping-helper selection) branch specifically on `productname == "Lucee"`. `server.lucee.versionName` stays `"RustCFML"` regardless |
| `existenceCacheScope` | `"application"` \| `"request"` | `"application"` | How long a resolved file-existence answer (`fileExists`, `directoryExists`, and the engine's own template/helper probing) may be reused. Only consulted in `--production`; dev serve mode and the CLI are always request-scoped. `"application"` accepts that an answer can survive a change made by a *different* process — RustCFML's own writes always invalidate — in exchange for not re-`stat`ing paths it has already resolved. See [known-issues §45](known-issues.md) |
| `applicationTimeout` | `"d,h,m,s"` | `"1,0,0,0"` | Application scope timeout |
| `sessionTimeout` | `"d,h,m,s"` | `"0,0,30,0"` | Session scope timeout |
| `clientTimeout` | `"d,h,m,s"` | `"7,0,0,0"` | Client scope timeout |

### `session`

Background session-expiry reaper (serve mode only). The reaper drains expired
session data off the request path on a timer, so a normal request pays ~zero
expiry cost and an idle server still evicts expired sessions. `onSessionEnd`
fires opportunistically on the next request for the owning application
(cleanup-only delivery — see `docs/known-issues.md` §12d).

| Key | Type | Default | Notes |
|---|---|---|---|
| `reapIntervalSecs` | int | `60` | Reaper tick in seconds. `0` disables the reaper entirely (read-path exactness + native store TTL still apply) |
| `reapAdaptive` | bool | `false` | Sleep until the next session's expiry (capped at `reapIntervalSecs`). Only memory/cluster stores benefit; others use the fixed tick |
| `reapBatchMax` | int | `1000` | Max pending `onSessionEnd` deliveries buffered per application between requests; beyond it the oldest is dropped (logged) |

> Note: `sessionTimeout` (under `runtime`, or `this.sessionTimeout`) is clamped
> to a 60-second floor — sub-minute session timeouts are raised to 60s.

### `datasources`

Map of name → driver config. The name becomes the value used in
`cfquery datasource="name"` / `queryExecute(..., {datasource: "name"})`.

```jsonc
"datasources": {
  "myDSN": {
    "driver":   "mysql",          // mysql | mariadb | postgresql | postgres | mssql | sqlserver | sqlite
                                  // Lucee aliases also accepted: "type" / "dbdriver" (e.g. "type": "MySQL"),
                                  // or a JDBC "class" (e.g. "com.mysql.cj.jdbc.Driver").
    "host":     "localhost",
    "port":     "3306",
    "database": "mydb",
    "username": "u",
    "password": "p",
    "connectionString": "",       // optional — overrides the synthesised URL
    "default": false              // when true, used when cfquery omits datasource
  }
}
```

`Application.cfc this.datasources` overrides global entries at application scope.
The same `type` / `dbdriver` / `class` aliases work there too, so a standard
Lucee declaration — `this.datasources["x"] = { type:"MySQL", host:…, … }` —
resolves to the right driver.

A datasource name that resolves to none of: a registered datasource, a dynamic
driver, or an explicit connection string raises an error rather than silently
falling back to an in-memory SQLite database.

### `mappings`

```jsonc
"mappings": {
  "/mylib": "/var/www/shared/lib"
}
```

Layered underneath `Application.cfc this.mappings` — the app file wins on
conflict.

### `customTagPaths`

```jsonc
"customTagPaths": ["/var/www/tags"]
```

Searched after `Application.cfc this.customTagPaths`.

### `mailServers`

First entry becomes cfmail's default when its tag attributes omit `server`.

`tls` selects STARTTLS (connect in the clear on the submission port, then
upgrade) and `ssl` selects implicit TLS/SMTPS (the whole connection is wrapped,
conventionally port 465). `<cfmail useTLS=/useSSL=>` overrides them per message.
If encryption is requested and cannot be established the send fails — it never
falls back to an unencrypted connection.

```jsonc
"mailServers": [
  {
    "smtp":     "smtp.example.com",
    "port":     587,
    "username": "u",
    "password": "p",
    "tls":      true,
    "ssl":      false,
    "timeout":  30
  }
]
```

### `logging`

| Key | Type | Default | Notes |
|---|---|---|---|
| `level` | string | `"warn"` | Engine's own (Rust) log output: `error`/`warn`/`info`/`debug`/`trace`/`off` |
| `logsDirectory` | string | `""` | Where `<cflog file="x">` writes `x.log`. Empty ⇒ `<webroot>/logs` under `--serve`, `./logs` under the CLI |
| `cfmlLevel` | string | `""` | Default threshold for CFML logs. Empty ⇒ `trace` (log everything), matching Lucee's handling of an unconfigured `file=` logger |
| `loggers.<name>.level` | string | — | Per-logger override, for engine targets (e.g. `datasource`) **and** CFML log names. `off`/`none` mutes |
| `maxFileSize` | number | `10485760` | Rotate once a log file would exceed this many bytes. `0` = never |
| `maxFiles` | number | `10` | Rotated generations to keep (`x.1.log` … `x.N.log`) |
| `flushEachLine` | bool | `true` | log4j2's `immediateFlush` — what makes `tail -f` work. `false` batches lines until request end |
| `echoToStderr` | bool | `false` | Also echo CFML log lines to the console (Lucee doesn't). The `RUSTCFML_LOG_STDERR` env var forces it on |
| `format` | string | `"text"` | Reserved — JSON sink not yet implemented |

`RUST_LOG` and `--verbose` still take precedence for the engine's own `level`.

`<cflog>` / `writeLog()` write Lucee 7's exact line layout, so existing log tooling
works unchanged:

```
"Severity","ThreadID","Date","Time","Context","Application","Message"
"ERROR","tokio-rt-worker","07/26/2026","22:56:48","http://127.0.0.1:8500","MyApp","boom"
```

The resolved directory is readable from CFML as
`server.cfconfig.logging.logsDirectory`.

### `debugging`

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | When false, hides error detail from clients (server log keeps it) |
| `errorTemplate` | string | `""` | CFML template rendered for unhandled errors; receives `request._error` |
| `errorStatusCode` | bool | `true` | When false, error responses return 200 |
| `showExecutionTime` | bool | `false` | Reserved |

### `security`

| Key | Type | Default | Notes |
|---|---|---|---|
| `sandbox` | bool | `false` | Same as `--sandbox`: blocks host filesystem writes |
| `disallowedFunctions` | string[] | `[]` | Case-insensitive BIF/user-function names that are refused |
| `disallowedImports` | string[] | `[]` | Regex patterns blocking `createObject("component"\|"rust", ...)` |
| `blockedPaths` | string[] | `["*.cfm.bak","*.cfm~","Application.cfc","*.config.cfm"]` | URL globs returning 404 |
| `csrfEnabled` | bool | `true` | When false, `csrfGenerateToken` / `csrfVerifyToken` error out |
| `secureJSON` | bool | `false` | Prepend `secureJSONPrefix` to `serializeJSON` output |
| `secureJSONPrefix` | string | `"//"` | Hijack-prevention prefix |

### `urlRewriting`

| Key | Type | Default | Notes |
|---|---|---|---|
| `configFile` | string | `"urlrewrite.xml"` | Path to the rewrite rules (relative to webroot or absolute) |
| `enabled` | bool | `true` | Skip rewriting entirely when false |

### `caches` and `sessionStorage`

`sessionStorage` names a cache defined in `caches` that should back the session store.
`caches` is a map of named cache definitions, each with a `provider` and `properties` block.

**Supported providers:**

| `provider` | Description |
|-----------|-------------|
| `"memory"` | In-process store (default — no config needed) |
| `"memcached"` | External Memcached cluster |
| `"cluster"` | Gossip-based multi-node replication via memberlist + Automerge CRDT |

All three providers are built into the stock `rustcfml` binary — there is nothing to enable at build time. Each provider is dormant until a cache definition with the matching `provider` value is referenced as session storage in `.cfconfig.json` (or `this.sessionStorage` in `Application.cfc`).

**Example — Memcached (RustCFML native format):**
```json
{
    "sessionStorage": "mc",
    "caches": {
        "mc": {
            "provider": "memcached",
            "storage": true,
            "properties": {
                "servers": ["localhost:11211"],
                "keyPrefix": "myapp:sess:"
            }
        }
    }
}
```

**Lucee compatibility format** — if you export a `.cfconfig.json` from Lucee with the Memcached extension installed, it uses `class` instead of `provider` and a `custom` map with a space-separated `servers` string. RustCFML accepts this format directly:

```json
{
    "sessionStorage": "sessions",
    "caches": {
        "sessions": {
            "class": "org.lucee.extension.io.cache.memcache.MemCacheRaw",
            "storage": true,
            "custom": {
                "servers": "host1:11211 host2:11211",
                "storage_format": "Binary"
            }
        }
    }
}
```

Both Lucee Memcached class names are recognised:
- `org.lucee.extension.io.cache.memcache.MemCacheRaw` (Lucee 5 / early 6)
- `org.lucee.extension.cache.mc.MemcachedCache` (Lucee 6 current)

**Lucee notes:**
- The `storage: true` flag is required by Lucee for session-eligible caches. RustCFML emits a warning if it is absent but does not refuse.
- Lucee serialises sessions as binary Java objects; RustCFML serialises as JSON. Sessions written by one engine cannot be read by the other — they do not share session data in the same Memcached instance.
- Lucee has `sessionCluster: true/false` (`this.sessionCluster` in Application.cfc) to control whether reads are always pulled from the external store. RustCFML always reads from the store on each request.

**Example — Cluster (single-node config):**
```json
{
    "sessionStorage": "cluster",
    "caches": {
        "cluster": {
            "provider": "cluster",
            "storage": true,
            "properties": {
                "listenAddr": "0.0.0.0:7946",
                "advertiseAddr": "192.168.1.10:7946",
                "seeds": ["node2.internal:7946", "node3.internal:7946"],
                "nodeName": "node1"
            }
        }
    }
}
```

> **`storage: true` is required.** The cache must explicitly opt in to being used as session storage. Lucee enforces this; RustCFML warns if it is missing but uses the cache anyway.

Cluster properties:

| Property | Default | Description |
|----------|---------|-------------|
| `listenAddr` | `0.0.0.0:7946` | TCP `host:port` this node binds for memberlist gossip. Use `0.0.0.0` (IPv4) or `[::]` (dual-stack) to bind every interface; restrict to a specific IP for tighter networking. **On Fly.io use `[::]:7946`** — 6PN is IPv6-only. |
| `advertiseAddr` | (empty) | Public address other nodes should reach this one on. Required when `listenAddr` binds `0.0.0.0`/`[::]`; leave empty when `listenAddr` already specifies a routable address. Also used as the default `nodeName`. |
| `seeds` | `[]` | Legacy static seed list. Used when `discovery` is absent; equivalent to `discovery.method = "static"`. Prefer `discovery` for new configs. |
| `nodeName` | derived | Stable identifier used as the node's id. Defaults to `advertiseAddr`, or `listenAddr-<uuid>` when neither is set. Set this explicitly in production so a node keeps the same identity across restarts. |
| `discovery` | `{}` | Peer discovery strategy. See [Discovery methods](#discovery-methods) below. |

### Discovery methods

`discovery.method` selects how this node finds peers. The choice determines whether the cluster can scale dynamically.

| Method | What it does | Use for |
|--------|---|---|
| `static`   | Connects to the addresses in `seeds`. No refresh. | Tests, fixed 2–3 node deployments. |
| `dns`      | Resolves a DNS name to A/AAAA records every `intervalSecs` and joins any new addresses. | **Fly.io**, Kubernetes headless services, ECS / Nomad service discovery, anywhere the platform exposes peers via DNS. |
| `multicast`| UDP multicast announce / listen on `group:port`. | LAN / bare-metal / VMware development; Kubernetes clusters using a CNI that carries multicast (Calico VXLAN, Weave, Flannel VXLAN). **Does not work** on AWS VPC CNI, Fly.io, GCP, Azure. |

Common `discovery` fields:

| Field | Default | Used by | Description |
|-------|---------|---------|-------------|
| `method` | (legacy: `static` if `seeds` set) | all | `"static"` / `"dns"` / `"multicast"`. |
| `name` | (empty) | `dns` | DNS name to resolve. |
| `port` | derived from `listenAddr` | `dns`, `multicast` | Port to attach to discovered addresses. |
| `intervalSecs` | `10` for dns, `5` for multicast | `dns`, `multicast` | Refresh / announce interval in seconds. |
| `group` | `239.255.42.42` | `multicast` | IPv4 multicast group (admin-scoped `239/8` recommended). |
| `seeds` | (empty) | `static` | Per-strategy seed list; overrides the top-level `seeds` when set. |

### Fly.io recipe (DNS discovery on 6PN)

Fly's private network is IPv6-only WireGuard and **does not support multicast**. Fly's internal DNS exposes every running Machine via `<app>.internal`, so DNS polling is the right strategy:

```json
{
    "sessionStorage": "cluster",
    "caches": { "cluster": { "provider": "cluster", "storage": true,
        "properties": {
            "listenAddr": "[::]:7946",
            "nodeName":   "${FLY_MACHINE_ID}",
            "discovery": {
                "method":       "dns",
                "name":         "${FLY_APP_NAME}.internal",
                "port":         7946,
                "intervalSecs": 5
            }
        } } }
}
```

Variants:
- `top6.nearest.of.${FLY_APP_NAME}.internal` — bound the cluster to the 6 nearest Machines by latency.
- `<region>.${FLY_APP_NAME}.internal` — region-scoped cluster (`lhr.…`, `iad.…`).
- `${FLY_PROCESS_GROUP}.process.${FLY_APP_NAME}.internal` — process-group-scoped.

### Kubernetes recipe (DNS via headless service)

Create a headless `Service` (`clusterIP: None`) for the session cluster pods with `publishNotReadyAddresses: true`. Then point each pod at its DNS name:

```json
{
    "sessionStorage": "cluster",
    "caches": { "cluster": { "provider": "cluster", "storage": true,
        "properties": {
            "listenAddr": "0.0.0.0:7946",
            "nodeName":   "${HOSTNAME}",
            "discovery": {
                "method":       "dns",
                "name":         "rustcfml-cluster.default.svc.cluster.local",
                "port":         7946,
                "intervalSecs": 10
            }
        } } }
}
```

For EKS clusters running Calico/Weave/Flannel that carry multicast, `discovery.method = "multicast"` also works.

### Local development (multicast)

Two `rustcfml --serve` processes on the same LAN/laptop auto-find each other:

```json
{
    "sessionStorage": "cluster",
    "caches": { "cluster": { "provider": "cluster", "storage": true,
        "properties": {
            "listenAddr": "192.168.1.42:7946",
            "discovery": { "method": "multicast" }
        } } }
}
```

Multicast announcements include this node's `listenAddr` so peers can dial back — don't leave `listenAddr` as a wildcard with multicast (a warning is logged if you do).

### Three-node walkthrough

Three machines, all on the same internal network, all running `rustcfml --serve --port 8500`:

```jsonc
// On node1 (192.168.1.10) — .cfconfig.json
{
    "sessionStorage": "cluster",
    "caches": { "cluster": { "provider": "cluster", "storage": true,
        "properties": {
            "listenAddr":    "0.0.0.0:7946",
            "advertiseAddr": "192.168.1.10:7946",
            "seeds":         [],
            "nodeName":      "node1"
        } } }
}
```
```jsonc
// On node2 (192.168.1.11) — .cfconfig.json
{ "sessionStorage": "cluster",
  "caches": { "cluster": { "provider": "cluster", "storage": true,
    "properties": {
        "listenAddr":    "0.0.0.0:7946",
        "advertiseAddr": "192.168.1.11:7946",
        "seeds":         ["192.168.1.10:7946"],
        "nodeName":      "node2"
    } } } }
```
```jsonc
// On node3 (192.168.1.12) — .cfconfig.json
{ "sessionStorage": "cluster",
  "caches": { "cluster": { "provider": "cluster", "storage": true,
    "properties": {
        "listenAddr":    "0.0.0.0:7946",
        "advertiseAddr": "192.168.1.12:7946",
        "seeds":         ["192.168.1.10:7946", "192.168.1.11:7946"],
        "nodeName":      "node3"
    } } } }
```

Start order: any node can start first. Nodes whose seeds are unreachable at boot log a `partial join` warning, but the cluster heals automatically as the missing peers come up — periodic anti-entropy will pull the latest state in the next push/pull cycle.

Each node logs a single line on success, e.g. `[session/cluster] node 'node2' listening on 0.0.0.0:7946` plus `[session/cluster] joined 1 seed(s) successfully`.

### Firewalls and ports

The cluster uses **one TCP port per node** (the `listenAddr` port — 7946 by default, matching HashiCorp Serf's convention). Open it bidirectionally between every pair of cluster members. No additional UDP ports are needed in this build (the `tcp` feature is the only transport enabled).

Run multiple nodes on one host (e.g. for local testing) by giving each a distinct `listenAddr` port:
```bash
# node A on :7946, node B on :7947
```

### How it works

Each session is held in its own per-process [Automerge](https://automerge.org) document. On `set` / `remove`, the local document records a change and the incremental change bytes are reliably sent to every currently-online cluster member as a [memberlist](https://github.com/al8n/memberlist) user-message. On receive, the change is applied via Automerge's CRDT merge — concurrent writes converge deterministically across the cluster without coordination.

Membership and failure detection come from memberlist (the Rust port of HashiCorp's gossip protocol). On node join, memberlist's TCP push/pull state exchange invokes the cluster store's `local_state` hook on each side, round-tripping the union of all session documents — so a newly-joined node catches up to the cluster's full state immediately, and the same mechanism runs periodically thereafter as anti-entropy against any messages dropped on the live path.

### Sizing

Tested for native rustcfml server deployments up to a few dozen nodes on LAN or WAN. WASM and Cloudflare Workers **cannot** participate — memberlist requires a persistent TCP socket model unavailable in those runtimes.

### Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `[session/cluster] partial join — reached 0 seed(s); error: Connection refused` on every node | None of the seeds were running yet, **or** they aren't actually listening on `listenAddr`, **or** a firewall is blocking the port. | Start at least one seed first, double-check the `host:port` strings, open the port between the nodes. |
| Session set on node A is never visible on node B | Almost always: `nodeName` collision (two nodes share the same name, so memberlist sees them as the same node and ignores one). Less commonly: `advertiseAddr` is set to a value the peer can't actually reach. | Give every node a unique `nodeName`. Verify each `advertiseAddr` resolves and is reachable from every other node. |
| Sessions sometimes appear after a delay rather than immediately | Live `send_reliable` was dropped (network glitch). Anti-entropy will catch it on the next push/pull cycle (a few seconds). | Expected behaviour — the cluster is eventually consistent. If delays exceed ~10 s, investigate network or memberlist tuning. |
| A node's CFML test suite fails when the cluster is configured | Unlikely — the test runner uses CLI mode and never touches `build_session_store`. | If you actually see this, file a bug with the failing suite name. |

**Application.cfc override** — per-app session storage follows Lucee conventions:
```cfml
component {
    this.name            = "MyApp";
    this.sessionManagement = true;
    this.sessionStorage  = "mc";  // references a named cache

    this.cache["mc"] = {
        provider: "memcached",
        properties: { servers: ["localhost:11211"] }
    };
}
```

`this.cache` definitions merge with and override same-named entries from `.cfconfig.json`.
`this.sessionStorage` overrides the server-wide `sessionStorage` for this application.

## Inspecting the resolved config from CFML

The merged config is exposed as a read-only struct on the `server` scope:

```cfml
<cfscript>
writeOutput(server.cfconfig.server.port);
writeOutput(server.cfconfig.runtime.locale);
for (name in server.cfconfig.datasources) {
    writeOutput(name & " -> " & server.cfconfig.datasources[name].driver);
}
</cfscript>
```

Useful for debugging deploys and for templates that want to branch on
environment.

## Precedence summary

```
CLI flag  >  .cfconfig.json  >  compiled-in default
```

At application scope, `Application.cfc this.*` overrides the runtime,
datasource, mapping, and custom-tag-path layers from `.cfconfig.json`.
