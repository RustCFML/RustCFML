# Sessions

How RustCFML tracks sessions: the `session` scope, the lifecycle, the `CFID`
cookie (and how to harden it), the pluggable storage backends, and expiry. This
is the conceptual guide; the exhaustive backend configuration reference lives in
**[Configuration → caches and sessionStorage](configuration.md#caches-and-sessionstorage)**,
and the precise Lucee/BoxLang divergences in
**[Known Issues §12](known-issues.md)**.

## Enabling sessions

Sessions are per-application and off until you opt in from `Application.cfc`:

```cfc
component {
    this.name              = "myapp";
    this.sessionManagement = true;
    this.sessionTimeout    = createTimeSpan(0, 0, 30, 0); // 30 minutes
}
```

With management on, the `session` scope is a live struct shared across a
visitor's requests and across `cfthread` bodies within a request (guard
concurrent writes with `cflock`). Sessions are namespaced per application: the
storage key is the composite `(applicationName, id)`, so two apps served from
the same process never share a session record even if a browser presents the
same `CFID`.

## The lazy-creation default *(divergence from Lucee)*

**No session record, no `CFID` cookie, and no `onSessionStart` fire until code
writes to the `session` scope.** A request that only *reads* session — or never
touches it — mints nothing and hands out no tracking cookie. This is stricter
than Lucee 7 (which mints the cookie on a read/check) and is a deliberate,
privacy-friendly default: crawlers and `curl` hits don't accrete empty sessions.

Opt back into eager creation (a record + `onSessionStart` up front) with:

```cfc
this.lazySessionCreation = false; // alias: this.lazySessions
```

## The session cookie

When a record is minted, the runtime emits the session id as a `Set-Cookie`
(named `CFID`). Both runtimes — the `--serve` HTTP layer and the Cloudflare
Worker — render the cookie through one shared builder, so the attributes are
identical across deployments.

### Defaults

| Attribute | Default |
|---|---|
| `Path` | `/` |
| `HttpOnly` | on |
| `SameSite` | `Lax` |
| `Secure` | **auto** — see below |
| `Domain` | omitted (host-only cookie) |

### The `Secure` default — "secure if the connection is secure"

When you don't set `secure` explicitly, `Secure` is emitted **iff the request
arrived over a secure transport**:

- **Cloudflare Worker** — HTTPS is guaranteed end-to-end, so `Secure` is always
  on.
- **`--serve` (CLI)** — the server is HTTP-only by design and is meant to sit
  behind a TLS-terminating reverse proxy (nginx, Caddy, …). The secure signal is
  therefore the `X-Forwarded-Proto: https` header the proxy sets. A bare
  `http://` dev box (a LAN IP, a custom local hostname) gets **no** `Secure`, so
  the session survives; a deployment behind TLS gets `Secure` automatically with
  no configuration. The same header also populates `cgi.https` (`on`/`off`).

> **Proxy requirement.** For `Secure` (and `cgi.https`) to be correct behind a
> reverse proxy, the proxy must forward the scheme. With nginx:
> ```nginx
> location / {
>     proxy_pass http://127.0.0.1:8500;
>     proxy_set_header Host              $host;
>     proxy_set_header X-Forwarded-Proto $scheme;
> }
> ```

This auto-`Secure` behaviour is a **deliberate divergence** from Lucee, whose
spec default is `secure:false` everywhere — but it is confined to the case where
the app says nothing. An explicit setting always wins (next section).

### Overriding attributes — `this.sessioncookie`

Set any of these from `Application.cfc`; they are honoured identically on both
runtimes:

```cfc
this.sessioncookie = {
    secure   = true,        // force on/off regardless of transport
    httponly = true,
    samesite = "Strict",    // Lax (default) | Strict | None | "" (omit attribute)
    domain   = ".example.com",
    path     = "/"
};
```

- An explicit `secure = true` is emitted even over plain HTTP; an explicit
  `secure = false` suppresses it even over HTTPS. The auto-detection only applies
  when `secure` is unset.
- `samesite = "None"` forces `Secure` on as well, since browsers reject
  `SameSite=None` cookies without it.

## Storage backends

The in-process store is the default. Two distributed backends ship in the stock
binary and are selected purely by `.cfconfig.json` — no rebuild:

- **Memcached** — sessions in an external Memcached cluster (Lucee-compatible
  config shape).
- **Cluster** — gossip-based peer-to-peer replication across native RustCFML
  nodes ([memberlist](https://github.com/al8n/memberlist) membership +
  [Automerge](https://automerge.org) CRDTs); good for LAN/WAN deployments up to a
  few dozen nodes, no external store.
- **Datasource (SQL)** — `sessionStorage` may also name a SQL datasource; the
  blob is stored in an auto-created table.

All four share the same `sessionStorage` / `caches` keys, so the config carries
across Lucee and BoxLang. See
**[Configuration → caches and sessionStorage](configuration.md#caches-and-sessionstorage)**
for the full reference, a multi-node walkthrough, and a troubleshooting table.

### Live objects in session — allowed in memory, data-only on serializing stores

The **default in-memory store keeps live object references**, so a component,
closure, or native object stored in `session` round-trips as the same live
object — matching Lucee/ACF in-memory sessions. This is what ColdBox/WireBox
session-scoped beans rely on.

```cfml
session.myBean = new Bean();   // in-memory: fine — isObject(session.myBean) is true
```

A **serializing store** (datasource, memcached, KV/Workers, cluster) can only
hold plain data — a live CFC/closure/native object cannot survive the
serialize→store→deserialize round trip. On those stores the **data-only rule**
applies: writing a non-data value throws and names the offending key path.

```
session.cart.items[3].product is a component; the session scope only persists
data values (no components, closures, functions, or native objects)
```

> **Divergence from Lucee (deliberate).** On a serializing store Lucee *attempts*
> to serialize the object (Java serialization), which may succeed for a
> serializable CFC or silently corrupt/duplicate it. RustCFML has no CFC
> serialization, so it **rejects loudly** rather than dropping the value to a
> silent `null` on the next request. The direct-write case
> (`session.x = new C()`) throws at the assignment and **is catchable**; a value
> smuggled in via reference mutation (`local.x = {}; session.box = local.x;
> local.x.p = new C()`) is caught by the airtight persist-time deep walk at the
> request boundary and is *not* catchable. Dates, binary, and queries have
> round-trip forms, so everything that can serialize is allowed on every store.

## Expiry

Expiry does not ride on request handling. Two mechanisms:

- **Read-path exactness (hard guarantee).** Every store treats a record past
  `last_accessed + timeout` as absent the instant it expires, so application code
  never sees a session that should have died.
- **Background reaper (serve mode only).** A timer task drains expired session
  *data* off the request path, so an idle server still evicts and a normal
  request pays ~zero expiry cost. Tunable under the `session` key in
  `.cfconfig.json` (`reapIntervalSecs`, `reapAdaptive`, `reapBatchMax`).

`onSessionEnd` is **cleanup-only with no delivery guarantee**: the reaper has no
request context, so the hook is queued and fires on the next request for that
application (and never at all for memcached/KV native-TTL expiry). Full detail in
**[Known Issues §12d](known-issues.md)**.

## See also

- **[Web Server](web-server.md)** — serve mode and the `Application.cfc` lifecycle
- **[Configuration](configuration.md#caches-and-sessionstorage)** — backend config reference
- **[Known Issues §12](known-issues.md)** — exact divergences and edge cases
