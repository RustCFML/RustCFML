# Known Issues & Unsupported Behaviour

This document inventories behaviours that RustCFML **does not fully implement**, with
an emphasis on **silent no-ops** — settings or attributes that are accepted without
error but have no effect. Those are the dangerous ones: code relying on them appears
to work but silently doesn't.

Each item is tagged:

- 🔇 **silent** — accepted, no error, no effect (the priority list to make overt)
- 🛑 **errors** — fails loudly with a clear message (safe, just unsupported)
- 🌍 **environment** — unsupported only on a specific target (e.g. wasm)
- 🏗 **by design** — intentional scoping decision (documented for clarity)

Compatibility target is Lucee/BoxLang. Items below are gaps against that target unless
marked *by design*.

> Maintenance: when you implement around a gap or skip an attribute/setting, add it
> here in the same change. See `docs/configuration.md` and `docs/status.md` for the
> positive "what's supported" view.

---

## 1. Application.cfc `this.*` settings — silently ignored 🔇

Read today: `this.name`, `this.mappings`, `this.sessionManagement`, `this.sessionTimeout`,
`this.customTagPaths`, `this.localMode`, `this.sessionStorage`, `this.cache`,
`this.lazySessionCreation`, `this.datasources`, `this.datasource`,
`this.sessioncookie` (secure/httponly/samesite/domain/path — see §12e).

Accepted but **ignored** (no error, no effect):

| Setting | Notes |
|---|---|
| `this.timezone` | Per-app timezone ignored. Only the server/cfconfig `runtime.timezone` is honoured. |
| `this.locale` | Per-app locale ignored. Only cfconfig `runtime.locale` is honoured. |
| `this.applicationTimeout` | Per-app value ignored. cfconfig `runtime.applicationTimeout` IS applied. |
| `this.scriptProtect` | No script-protection filtering of scopes. |
| `this.secureJSON` / `this.secureJSONPrefix` | Per-app value ignored. cfconfig `security.secureJSON*` IS applied (process-global — see §4). |
| `this.nullSupport` / `this.enableNullSupport` | Per-app value ignored. cfconfig `runtime.nullSupport` IS applied. |
| `this.clientManagement`, `this.setClientCookies`, `this.setDomainCookies`, `this.clientStorage` | The **client scope is not implemented** at all. |
| `this.invokeImplicitAccessor` | Ignored. |
| `this.serialization`, `this.javaSettings`, `this.compileExtForCFCDirectory`, `this.blockedExtForFileUpload`, `this.triggerDataMember`, `this.sameFormFieldsAsArray`, `this.searchImplicitScopes`, `this.proxyServer`, `this.smtpServerSettings` | No references in the engine — accepted into the component, never consulted. |

Note: any unrecognised `this.X` is captured into an internal `config` map that is then
never read — so nothing throws, but nothing happens either.

## 2. Application.cfc lifecycle methods — mostly invoked; one gap remains 🔇

| Method | Status |
|---|---|
| `onApplicationStart`, `onApplicationEnd`, `onRequestStart`, `onRequest`, `onRequestEnd`, `onSessionStart`, `onSessionEnd` | ✅ invoked |
| `onError` | ✅ invoked. An uncaught exception in the target page / `onRequest` / `onRequestStart` is handed to `onError(exception, eventName)` (`eventName` is `""` for a target-page error, otherwise the running event method). If `onError` returns normally it owns the response (the engine's default error page is suppressed); if absent the error surfaces as the default error page. When `onError` handles an error, `onRequestEnd` is skipped. *(fixed v0.173.0, issue #145)* |
| `onMissingTemplate` | ✅ invoked (serve mode). A request for a `.cfm`/`.cfc` template that doesn't exist on disk calls `onMissingTemplate(targetPage)` (`targetPage` is the web-root-relative requested path) after `onApplicationStart`/`onSessionStart`. Returning `true` (or nothing) handles the request and suppresses the default 404; returning `false` — or having no handler — falls through to the default 404. `onRequestStart`/`onRequest`/`onRequestEnd` are skipped (Adobe semantics). A throw inside the handler routes to `onError`. Non-CFML 404s (`.html`, images, directory requests) bypass the engine and never reach the handler. The cfconfig front-controller `fallback` remains available as an alternative. *(fixed v0.183.0)* |
| `onAbort` | ✅ invoked on `<cfabort>` / `abort` — fired in place of `onRequestEnd`. `<cfabort showError="msg">` is a *catchable* error and is routed to `onError` instead (Adobe/Lucee parity), not `onAbort`. *(fixed v0.173.0)* |
| `onCFCRequest` | 🔇 Not invoked (no CFC-over-HTTP / remote method dispatch). |

## 3. `.cfconfig.json` keys — accepted but not enforced 🔇

These deserialize without error but have no runtime effect:

| Key | Notes |
|---|---|
| `server.maxConcurrentRequests` | No concurrency limiting. |
| `server.requestTimeout` | No per-request timeout enforcement. |
| `server.http2` | Not wired to the HTTP server. |
| `runtime.trustedCache` | Reserved; bytecode-cache trust is driven by `--production`, not this key. |
| `debugging.showExecutionTime` | No timing output. |
| `datasources[].connectionLimit` / `connectionTimeout` / `idleTimeout` / `timezone` | Pool tuning / per-DS timezone not applied. |
| `mailServers[].timeout` | Carried but not applied during send. |
| `caches[].properties.maxObjects` / `defaultTimeout` / `evictionPolicy` | In-memory cache capacity / TTL / eviction not enforced. |
| `logging.logsDirectory` | 🛑→🔇 Warns at startup ("not yet supported"); logs still go to stderr. |
| `logging.format` | Only `"text"`; other values warn and fall back. |
| `logging.loggers[].appender` | Logger name used; appender ignored. |

## 4. Per-application isolation gaps 🏗/🔇

`.cfconfig.json` is application-level (a file beside `Application.cfc` overlays the
server baseline — see `docs/configuration.md`). But some runtime registries are still
**process-global**, so per-app overrides of these do **not** isolate across apps:

| Area | Status |
|---|---|
| Datasources (`this.datasources` / cfconfig) | ✅ **Per-application** (resolved per request). |
| Security flags — `csrfEnabled`, `secureJSON`, `secureJSONPrefix` | 🔇 **Process-global** (`OnceLock`, set once at startup). Per-app override only changes the readable `server.cfconfig` struct, not enforcement. |
| Default mail server (`mailServers[0]`) | 🔇 **Process-global**. The `cfmail server=` attribute still works per-call. |

Making security flags and the default mail server per-application is a planned
follow-up (mirrors the datasource work).

## 5. Server-level keys are not application-level 🏗

The entire cfconfig `server.*` section (host, welcomeFiles, maxRequestBodySize, …) is a
**server/environment** concern and is intentionally **not** overlaid from a per-app
`.cfconfig.json`. There is deliberately **no `port` key** — the listening port is set
via `--port`; pages read `cgi.server_port`. (This is by design, not a gap.)

## 6. Functions / tags that error loudly when unsupported 🛑

These do **not** silently no-op — they throw a clear message (listed for completeness):

| Feature | Behaviour |
|---|---|
| `structSetMetadata()` | Throws — Adobe-CF-only function (ACF 2016.0.2+) for per-key JSON-serialization metadata; not present in Lucee, our compatibility target. |
| `xmlTransform()` | Throws — no XSLT engine. |
| `xmlValidate()` | Throws — no schema-validation engine. |
| `<cfimport>` without `taglib` | Throws — Java/JSP class imports unsupported (custom-tag taglibs work). |
| `<cffile action="...">` outside the supported actions | Throws "not implemented". |
| `<cfthread action="...">` outside run/join/terminate | Throws "not supported". |
| `createObject("java", "…")` for a class outside the shimmed set | Throws "Java class […] is not supported" (RustCFML has no JVM; only a curated set of `java.*` standard-library classes are shimmed). Was previously a **silent null**, which surfaced downstream as a confusing "Variable X is undefined". |
| Dynamically-loaded Java classes (`cbjavaloader` / `java.net.URLClassLoader`) | The classloader *plumbing* (`URLClassLoader`, `coldfusion.runtime.java.JavaProxy`, `Class.forName`, `java.lang.reflect.Array`, `array.iterator()`) is shimmed so ColdBox's `cbjavaloader` module boots, but **invoking a class it loads throws** — there is no JVM to load JAR bytecode. Runtime features that genuinely need a loaded class (e.g. GoogleAuthenticator 2FA) fail loudly when used, not at boot. |

> **`evaluate()` is supported** (read-only). It compiles and runs each string
> argument as a CFML expression against the caller's scope and returns the value
> of the last one. The one caveat: assignment side effects do **not** propagate
> back to the caller's frame — `evaluate("x = 5")` will not set `x`. Read-only
> expression evaluation (the common use) works.

> **Nested `<cftransaction>` is supported** via savepoints. An inner transaction
> block opens a SAVEPOINT on the outer transaction; a nested commit releases it
> and a nested rollback rolls back to it (Lucee/ACF/BoxLang semantics).

## 7. Partially-ignored parameters 🔇

| Function | Ignored argument(s) | Reason |
|---|---|---|
| `csrfGenerateToken(key, forceNew)` | `key`, `forceNew` | No server-side per-key session token storage. |
| `csrfVerifyToken(token, key)` | `key` | Same. |
| `fileSetAccessMode` / file mode setters | mode | No-op on non-Unix platforms. |
| `fileUpload()` / `fileUploadAll()` | — | Stub: returns `fileWasSaved=false` (needs form-scope wiring). |
| `fileClose(handle)` | — | Stub: returns null, closes nothing (no real file-handle management). |
| `setTimeZone(tz)` | `tz` | No-op: the argument is ignored (only cfconfig `runtime.timezone` is honoured — see §1). |
| `<cfstoredproc>` / `cfprocparam` | `direction`, `dbVarName`, `maxLength`, `scale` | Only `value`/`cfsqltype` survive lowering, so OUT/INOUT stored-proc params don't round-trip. |
| `<cftransaction isolation="…">` | `isolation` | Parsed only to disambiguate the `datasource` arg; the isolation level is never applied to the connection. |
| `s3Write` / `s3Upload` / `s3Copy` / `s3Move` | `acl`, `location` | Accepted but not sent to the backend. |
| `s3Read` / `s3Download` | `charset` | Accepted but ignored. |
| `writeDump(output=…)` | a filename path | `output="console"` (→ server stdout) and `output="browser"`/default (→ page) are honoured; a filename value (Lucee writes the dump to that file) is not — it falls back to page output. |

## 8. Environment-specific 🌍

| Feature | Restriction |
|---|---|
| `<cfdirectory>` | Not supported on `wasm32` (no filesystem). |
| `<cfzip>` | Not supported on `wasm32`. |
| `<cflock>` | No-op in CLI mode (no server state); enforced in serve mode. |
| `<cfcache>` | No-op today (could emit Cache-Control in serve mode). |
| `runAsync` / `_schedule` delay+period | On `wasm32` (and other no-real-threads builds) the `delayMs`/`periodMs` args are ignored — the closure runs inline immediately rather than being scheduled. |
| `java.util.Collections.unmodifiable*` / `synchronized*` shims | Identity no-ops — they return the same collection with no true immutability / synchronization. |

## 9. Query-of-Queries — RustCFML/BoxLang superset 🏗

QoQ (`queryExecute(..., {dbtype:"query"})`) follows BoxLang and accepts SQL that **Lucee's
QoQ rejects**. Same query, *more* accepted — not a wrong-result divergence — but such SQL is
**not portable back to Lucee**:

| Feature | RustCFML | Lucee QoQ |
|---|---|---|
| `LIMIT n [OFFSET m]` | ✅ | ❌ (uses `SELECT TOP n`) |
| `CASE … WHEN … END` (searched + simple) | ✅ | ❌ |
| Scalar subquery in the SELECT list | ✅ | ❌ |
| Derived table `FROM (SELECT …) AS t` | ✅ | ❌ |
| Custom SQL functions (`queryRegisterFunction`) | ✅ | ❌ |

`SELECT TOP n`, `IN (SELECT …)`, all JOIN types, `UNION`, params, `LENGTH()` etc. work on both.
Cross-engine tests live in `tests/qoq/test_qoq_{select,aggregates,joins,subqueries_union}.cfm`
(green on both); superset-only coverage is probe-gated in `test_qoq_rustcfml_ext.cfm` /
`test_qoq_custom_functions.cfm` (skipped where unsupported).

**Correlated subqueries** (a subquery referencing the outer row) are **not** supported — subqueries
are executed once (uncorrelated); this matches typical QoQ usage. Errors loudly if a referenced
table/column is missing.


## 10. cfquery / queryExecute result metadata + cfdbinfo 🏗

Shipped for issue #90 (Wheels ORM DB layer): `result=` delivery on cfquery (tag, script
block, attributeCollection) and queryExecute, Lucee-faithful `name=` semantics (an INSERT
leaves `name` untouched), and `<cfdbinfo>`/`cfdbinfo(...)`/`dbinfo(...)` across all four
bundled drivers (SQLite, MySQL, PostgreSQL, SQL Server). Known divergences:

| Behaviour | RustCFML | Lucee |
|---|---|---|
| `queryExecute("INSERT …")` return value | the result-metadata **struct** `{recordCount, cached, sql, executionTime[, generatedKey]}` | the JDBC generated-keys **resultset** (a query; driver-dependent shape) |
| result struct extras | only `executionTime` (ms) | also carries `executionTimeNano`, `sqlparameters`, and a per-generated-key-column entry (e.g. `ID` on H2) |
| `executionTime` in result structs | measured (wall-clock ms of the driver round-trip; `0` on the wasm target, which has no monotonic clock) | measured |
| `generatedKey` on non-SQLite/MySQL INSERTs | absent on PostgreSQL/MSSQL (use `RETURNING` / `OUTPUT`) | driver-dependent |
| dbinfo `DATA_TYPE`/`SQL_DATA_TYPE` columns | always `0` (no JDBC type codes) | JDBC `java.sql.Types` ints |
| dbinfo statement syntax `dbinfo type="x" name="y";` | not parsed (use `cfdbinfo(...)` or the tag) | supported |
| dbinfo `UPDATE_RULE`/`DELETE_RULE` (foreignkeys) | rule **names** (`CASCADE`, `NO ACTION`, …) | JDBC smallint codes |

BoxLang notes (we follow Lucee, which Wheels tries first): Lucee renames `COLUMN_DEF` →
`COLUMN_DEFAULT_VALUE` (BoxLang keeps `COLUMN_DEF`); Lucee `dbnames` uses `database_name`
(BoxLang `DBNAME`); Lucee `IS_PRIMARYKEY`/`IS_FOREIGNKEY` are `YES`/`NO` strings (BoxLang
booleans). Both engines throw on a missing table only after an empty result — so does
RustCFML, with Lucee's message text. Live-server dbinfo tests are env-gated:
`RUSTCFML_TEST_MYSQL_DS` / `RUSTCFML_TEST_PG_DS` / `RUSTCFML_TEST_MSSQL_DS` in
`tests/tags/test_cfdbinfo.cfm`.

## 11. `getPageContext()` servlet bridge 🏗

`getPageContext().getRequest()` / `.getResponse()` return method-faithful servlet shims
in **every** context (serve and CLI), matching Lucee — which synthesizes them even under a
CommandBox task. Request accessors (`getRequestURL`, `getRequestURI`, `getQueryString`,
`getMethod`, `getScheme`, `getServerName`, `getServerPort`, `getServletPath`,
`getContextPath`, `getRemoteAddr`, `getProtocol`, `isSecure`, `getPathInfo`, `getHeader`,
`getContentType`, `getCharacterEncoding`) are synthesized from the request's CGI scope in
serve mode, and from Lucee's task-context defaults in bare CLI. Response mutators
(`setStatus`, `setHeader`, `addHeader`, `setContentType`, `sendRedirect`) drive the **real**
`response_status`/`response_headers` in serve mode; in CLI they update the same fields
harmlessly (as Lucee's response dummy does). We model Lucee (real servlet objects); the page
context also forwards the request/response accessors BoxLang exposes directly, so the surface
is a superset of both.

| Behaviour | RustCFML | Lucee |
|---|---|---|
| `getRemoteAddr()` in bare CLI | `127.0.0.1` | host LAN IP |
| `getPathInfo()` for a plain script request | `null` | `null` |
| Unknown servlet method (e.g. `getLocale`) | returns `null` (non-null receiver keeps chains alive) | full servlet API |
| `getMetaData(getRequest()).getName()` | a struct (no real Java class) | `...HTTPServletRequestWrap` |

## 12. Session storage — datasource store, lazy default, data-only rule 🏗/🌟

Three deliberate changes from issue #88, two of them conscious divergences from Lucee.

### 12a. Datasource (SQL) session store — *new, additive*

`sessionStorage` may now resolve to a SQL datasource, a fourth backend alongside
`memory`, `memcached`, and `cluster`. Two config forms:

```jsonc
// (a) cache entry with provider="datasource"
{ "sessionStorage": "sess_db",
  "caches": { "sess_db": { "provider": "datasource", "storage": true,
    "properties": { "datasource": "appdb", "table": "cf_session_data" } } },
  "datasources": { "appdb": { "driver": "sqlite", "database": "/var/app/sessions.db" } } }

// (b) Lucee-compat: sessionStorage names a defined datasource directly
{ "sessionStorage": "appdb",
  "datasources": { "appdb": { "driver": "postgresql", "host": "...", "database": "..." } } }
```

The table (`cf_session_data` by default, configurable) is auto-created with
`CREATE TABLE IF NOT EXISTS` on first use. The session blob is the same
`serde_json` shape the memcached store writes, so the `data` column is portable
between the two stores.

| Behaviour | RustCFML | Notes |
|---|---|---|
| Concurrency | last-write-wins (whole-blob) | same model as the memcached store; optimistic versioning is a possible v2 |
| Upsert | portable `UPDATE`-then-`INSERT` | avoids dialect-specific `ON CONFLICT`/`ON DUPLICATE KEY`/`MERGE` |
| Expiry sweep | portable `SELECT` + per-row `DELETE` claim (no `RETURNING`), now driven by the background reaper (§12d) not the request path | the delete is the cross-node claim, so multi-node does not double-fire — `onSessionEnd` is **best-effort, no delivery guarantee** (cleanup-only; see §12d) |
| Expiry touch | throttled (skips the write until ~25% of the timeout elapses with no data change) | kills per-request write amplification; semantically invisible |
| App partition | single logical `app_name` per store | multi-app isolation via distinct datasources/tables in v1 |
| DDL denied by grants | clear error telling you to pre-create the documented schema | the store then just uses it |
| Verified driver | SQLite (bundled) end-to-end; MySQL/PostgreSQL/MSSQL portable-by-construction | MSSQL may need a manual schema (`TEXT` is deprecated there) |
| `client` scope storage | not implemented (explicit non-goal for v1) | the schema extends with a scope discriminator if ever wanted |

### 12b. Lazy session creation is the engine-wide default 🌟 *(divergence)*

No session record, no `CFID` cookie, and no `onSessionStart` fire until code
**writes** to the `session` scope. A request that only reads session (or never
touches it) mints nothing — so crawlers and `curl` hits no longer persist empty
sessions or receive a tracking cookie.

This is **stricter than Lucee 7**, which still mints the cookie when a session
is created by a mere read/check. Deferring the cookie until a write is a
conscious, privacy-friendly divergence. `onSessionStart` timing also shifts for
existing apps: first write, not first hit. Opt back into the historical eager
behaviour with `this.lazySessionCreation = false` (alias `this.lazySessions`).

### 12c. Session scope: live objects in memory, data-only on serializing stores 🛑 *(partial divergence)*

The **default in-memory store keeps live object references**, so a component,
closure, or native object stored in `session` round-trips as the same live
object — matching Lucee/ACF in-memory sessions (this is what ColdBox/WireBox
session-scoped beans need). No divergence there.

A **serializing store** (datasource, memcached, KV/Workers, cluster) persists
**data values only** — no components, closures/functions, or native objects,
since they cannot survive the serialize→store→deserialize round trip. A violation
throws and names the offending key path:

```
session.cart.items[3].product is a component; the session scope only persists
data values (no components, closures, functions, or native objects)
```

**Divergence from Lucee (deliberate).** On a serializing store Lucee *attempts*
Java serialization, which may succeed for a serializable CFC or silently
corrupt/duplicate it; RustCFML has no CFC serialization, so it rejects loudly
rather than dropping the value to a **silent `null`** on the next request (the
worse status quo this fix also removed for the memory store). Two layers enforce
it on serializing stores: a check at the `session.x = ...` write site (fails fast
at the call, **catchable**), and a persist-time deep walk (the airtight gate,
which also catches values smuggled in via reference mutation, e.g.
`local.x = {}; session.box = local.x; local.x.p = new C()`; this fires at the
request boundary and is **not** catchable). Dates are strings and binary/query
have JSON round-trip forms, so the allowed set covers everything that serializes.
Behaviour verified against Lucee (in-memory allows a CFC; #236, v0.397.0).

### 12d. Session expiry — background reaper + read-path exactness — *new*

Expiry no longer rides on request handling. Two independent mechanisms:

**Read-path exactness (hard guarantee).** Every store's `get()` treats a record
past `last_accessed + timeout` as absent the instant it expires, independent of
any sweep — so application code never sees a session that should have died. The
memory store removes the dead record opportunistically on read; the datasource
store filters `expires_at > now` in its `SELECT`; memcached/KV rely on native
TTL; the cluster store checks expiry in `get()`.

**Background reaper (serve mode only).** A `tokio` task drains expired session
*data* out of the store on a timer — off the request path, so a normal request
pays ~zero expiry cost, and an **idle server still evicts** expired data (the old
request-driven sweep could leave a dead session lingering with unbounded lateness
until the next hit). Config under `session`:

```jsonc
{ "session": {
    "reapIntervalSecs": 60,   // tick; 0 disables the reaper entirely
    "reapAdaptive": false,    // sleep until the next expiry (capped at the tick)
    "reapBatchMax": 1000      // max pending onSessionEnd per app between requests
} }
```

🛑 **`onSessionEnd` is cleanup-only (delivery bounded by traffic).** The hook is
per-application CFML that needs the owning app's `Application.cfc`, `application`
scope, and mappings — all of which exist only inside a live request. The reaper
has no request context, so it **cannot fire `onSessionEnd` itself**. Instead it
queues the expired session's scope per application, and the hook fires on the
**next request for that application**. Consequences, documented rather than
hidden:

- An application that is **never requested again** drains its data on schedule
  but its `onSessionEnd` hooks never run. The per-app queue is bounded by
  `reapBatchMax`; beyond it the oldest pending hook is dropped (logged).
- **memcached / KV stores never deliver `onSessionEnd`** at all — expiry there is
  native TTL with no drain hook, so there is nothing to queue.
- **Server shutdown drops pending `onSessionEnd`** (matches Lucee's hard-stop
  semantics). A graceful-drain-on-shutdown is *not* offered: under cleanup-only
  delivery it could only evict data (no request context exists on shutdown to run
  the hook), so it would add no hook-delivery value.
- `reapAdaptive` only helps stores that can cheaply report their next expiry
  (memory, cluster); the datasource store falls back to the fixed tick rather
  than issue a `SELECT MIN(expires_at)` every wake-up.

`onSessionEnd` was already **best-effort with no delivery guarantee** before this
change (the datasource store's delete-as-claim row in §12a says as much); the
reaper keeps that contract and additionally fixes the idle-server data-eviction
gap. CLI (single-shot) mode spawns no reaper — expiry is irrelevant for a
one-request process.

### 12e. Session cookie attributes — `this.sessioncookie` + auto-`Secure` 🌟 *(divergence)*

The session `Set-Cookie` is rendered by a single shared builder
(`cfml-common::session_cookie`) used by **both** the `--serve` HTTP layer and the
Cloudflare Worker handler — previously each hand-rolled the header inline and they
had drifted (Worker emitted `SameSite=Lax`, CLI emitted neither `SameSite` nor
`Secure`). Per-application overrides via `this.sessioncookie` are now honoured on
both runtimes:

```cfc
this.sessioncookie = {
    secure   = true,        // see Secure default below
    httponly = true,        // default true
    samesite = "Strict",    // Lax (default) | Strict | None | "" (omit)
    domain   = ".example.com",
    path     = "/"          // default /
};
```

**`Secure` default — "secure if the connection is secure" (divergence from Lucee).**
When the app does **not** set `secure`, `Secure` is emitted iff the request arrived
over a secure transport:

- **Worker** — always HTTPS end-to-end → `Secure` on by default (also makes
  `__Secure-`/`__Host-` prefixes possible later).
- **CLI** — HTTP-only by design, behind a TLS-terminating proxy, so the signal is
  `X-Forwarded-Proto: https`. A bare `http://` dev box (LAN IP, custom hostname)
  gets no `Secure` and the session survives; a deployment behind nginx/Caddy gets
  `Secure` automatically. The same header now also populates `cgi.https`
  (`on`/`off`), which was previously absent.

Lucee's spec default is `secure:false` everywhere, so the Worker-on default is a
**deliberate divergence** — but confined to the *unspecified* case: an explicit
`this.sessioncookie.secure = false` is honoured verbatim on both runtimes.
`SameSite=None` forces `Secure` on (browsers reject it otherwise).

## 13. `<cfoutput query>` / grouped output — implemented, with edges 🏗

`<cfoutput query="q">` now drives row iteration (previously the `query` attribute
and friends were **silently discarded** — the body ran once against page scope).
Supported: per-row looping, `startrow`/`maxrows`, bare column refs (`#name#`,
resolved by merging each row into the `variables` scope), `#q.col#` row scalars,
and `#q.currentRow#`/`#q.recordCount#`/`#q.columnList#`. The query variable is
restored to the full query after the loop. `group` (control-break) output with a
nested detail `<cfoutput>` is supported, including multi-level grouping;
`groupCaseSensitive` defaults to `Yes` (case-sensitive), matching the CFML spec.

Known edges:

| Behaviour | Notes |
|---|---|
| Nested detail block placement | The detail `<cfoutput>` must sit **directly** in the group body. Wrapping it in `<cfif>`/`<cfloop>` is not supported (the pre/detail/post split would straddle the control-flow block). |
| Multiple sibling detail blocks | Only the **first** nested `<cfoutput>` at a given group level is treated as the detail loop; later siblings render once. |
| `group` + `startrow`/`maxrows` | `startrow`/`maxrows` apply to the **non-grouped** form only; the grouped form ignores them. |
| Bare column scope | Columns are merged into `variables`, so a page variable sharing a column's name is shadowed for the duration of the loop. |

## 14. `cfparam` / `param` `type` validation — enforced, with edges 🏗

The `type` attribute (and `min`/`max`/`pattern`) was **silently dropped** —
`<cfparam name="x" type="numeric">` never validated. It now throws on a type
mismatch (tag form, `param name=… type=…`, and the shorthand `param numeric x`).
Edges:

| Behaviour | Notes |
|---|---|
| Unknown type names | Types outside the known set (e.g. `variableName`, `xml`, `component`) are **accepted without validation** rather than wrongly rejected. |
| Dynamic / nested names | `param name="#expr#" type=…` and `param name="a.b['#k#']" type=…` set the default but do **not** validate (rare). |
| Non-literal `type` | A `type` given as an expression (not a string literal) is not validated. |
| "required" semantics | A typed param with no default whose value is absent is defaulted to `""` then type-checked, so the error names the type rather than "parameter required". |

## 15. `serializeJSON` struct key order — insertion order, not Lucee's HashMap order 🌟 *(divergence)*

RustCFML structs are insertion-ordered (`IndexMap`), so `serializeJSON()` emits keys
in the order they were added. Lucee/ACF back component metadata (and many internal
structs) with a Java `HashMap`, whose iteration order is hash-bucket order — neither
insertion nor alphabetical, but deterministic for a given JVM.

This is normally invisible and arguably an improvement (stable, predictable output).
It only bites code that **hashes a serialized struct as an identity key** and expects
byte-for-byte parity with Lucee. The known case is **Preside's foreign-key constraint
names**: `RelationshipGuidance.cfc` computes `fk_#Hash( SerializeJson( property ) )#`
over the normalised property struct. RustCFML produces *deterministic, self-consistent*
FK names (so its own dbSync/diffing works), but they will **not equal the values Lucee
generated**. Preside's `PresideObjectServiceTest` `test011`/`test012` assert the exact
Lucee-generated `fk_<md5>` strings and therefore fail on RustCFML even though the
relationships, columns and referential rules are correct.

Reproducing Lucee's exact hash would require emulating `java.util.HashMap` iteration
order (bucket index + resize thresholds) plus its attribute-name case preservation and
`required="true"`→`"yes"` metadata coercion — brittle and not worth it. Treated as a
**won't-fix divergence**. (FK *rule* reporting via `dbinfo type="foreignkeys"` — the
JDBC numeric `UPDATE_RULE`/`DELETE_RULE` codes — *is* matched; see §10.)

## 16. Sampling profiler — JIT-compiled numeric leaves not attributed 🏗

The threshold-gated sampling profiler (`observability.profiler`, `profileNow()` /
`getRequestProfile()` — see [debugging.md](debugging.md)) samples the CFML call
stack at the interpreter's per-line hook. Small hot **numeric functions the JIT
compiles to native code bypass the interpreter loop**, so they neither push a
call frame nor fire the sampling hook. Time spent inside such a function is
therefore folded into its **caller's** self-time instead of appearing as its own
node in the call tree.

This is a deliberate trade-off, not a silent drop: JIT'd numeric leaves are tiny
and fast by definition, and in serve mode the per-request JIT rarely warms up at
all (see the JIT-in-serve-mode notes), so interpreted frames — the overwhelming
majority of a real request — are attributed correctly. Breaking JIT'd leaves out
separately would require the JIT to push frames it currently elides for speed.

## 17. `objectSave()` / `objectLoad()` — internal binary format, not JVM-compatible 🌟 *(divergence)*

ACF/Lucee implement `objectSave()` / `objectLoad()` via **Java object serialization**
(a JVM-native binary blob). RustCFML has no JVM, so it uses its own **self-describing
internal format**: a magic header (`RCFMLOBJ\x01`) followed by the value serialized
as JSON via `CfmlValue`'s serde impl (Binary/Query are tagged with `_cftype` markers
so they reconstruct exactly). Consequences:

- **Not wire-compatible with the JVM engines.** A blob produced by ACF/Lucee cannot
  be `objectLoad()`ed here, and vice-versa. This is fine for the common use case —
  the pair is only ever round-tripped on the same engine (e.g. ColdBox's cache
  `DiskStore` marshaller saves then loads). `objectLoad()` on a foreign/JVM blob
  throws a clear error rather than corrupting silently.
- **Components / closures / functions serialize to `null`.** They cannot be
  reconstituted without their defining program. Scalars, structs, arrays, and
  queries round-trip with full fidelity. (Lucee can serialize a live CFC instance's
  state; RustCFML does not.)
- Struct key **insertion order** is preserved (see also §15), and whole-number
  doubles collapse to `Int` on load — the same normalisation the JSON path applies
  everywhere else.

## 18. Image functions — Tiers 1–3 implemented; rasterisation is not pixel-identical to Java2D 🏗

Image support is backed by pure-Rust crates (`image`, `imageproc`, `ab_glyph`,
`kamadak-exif`), so it builds natively **and** for the wasm32 targets, behind the
`image_support` feature (on by default). An image is a first-class mutable object —
`imageNew`/`imageRead` return it, and both the function form (`imageResize(img, w, h)`)
and the member form (`img.resize(w, h)`) mutate it in place, matching Lucee.

**Tier 1 — read / write / geometry:** `imageNew`, `imageRead`, `imageReadBase64`,
`imageWrite`, `imageWriteBase64`, `imageGetBlob`, `imageResize`, `imageScaleToFit`,
`imageGetWidth`, `imageGetHeight`, `imageInfo`, `imageCrop`, `imageRotate` (now **any**
angle), `imageFlip`, `isImage`, `isImageFile`, `getReadableImageFormats`/
`getWriteableImageFormats`, and `<cfimage>` `read`/`write`/`resize`/`info`/`convert`/
`writeToBrowser`. Read formats: PNG, JPEG, GIF, BMP, TIFF, WebP, ICO (detected by
**content**, not filename). `imageInfo()` reproduces Lucee's `colormodel` struct.

**Tier 2 — drawing:** `imageSetDrawingColor`/`BackgroundColor`/`DrawingStroke`/
`Antialiasing`/`DrawingTransparency`, `imageXORDrawingMode`; `imageDrawLine`/`Lines`/
`Point`/`Rect`/`RoundRect`/`BeveledRect`/`Oval`/`Arc`/`CubicCurve`/`QuadraticCurve`/
`Text`, `imageClearRect`; `imageDrawImage`/`imagePaste`/`imageOverlay`/`imageCopy`/
`imageAddBorder`; `<cfimage action="border"/"captcha">`.

**Tier 3 — filters / transforms / metadata:** `imageBlur`, `imageSharpen`,
`imageNegative`, `imageGrayscale`, `imageMakeColorTransparent`, `imageMakeTranslucent`;
`imageTranslate`, `imageShear`, `imageRotateDrawingAxis` (+ the `*DrawingAxis` aliases);
`imageGetEXIFMetadata`/`Tag`, `imageGetIPTCMetadata`/`Tag`.

**Documented behaviour differences vs Lucee's Java2D renderer 🌟:**
- **Not pixel-identical.** Antialiasing, curve/arc rasterisation and text hinting use
  `imageproc`/`ab_glyph`, not `java.awt`. Output is visually equivalent but not
  byte-for-byte; assert on dimensions/regions, not exact pixels.
- **Stroke width** on outline primitives is approximated by stamping the shape over a
  small disc of offsets (round joins), rather than Java2D's `BasicStroke` geometry.
- **`imageDrawText`** always renders with the **bundled DejaVu Sans** font; the `font`
  and `style` keys of an `attributeCollection` are ignored (only `size` is honoured).
  There is no system-font enumeration.
- **`imageXORDrawingMode`** is accepted and its flag stored, but drawing proceeds in
  normal paint mode (true Java2D XOR paint is not emulated).
- **`imageGetIPTCMetadata`** parses the JPEG APP13 / IPTC-NAA (8BIM 0x0404) segment for
  the common editorial datasets (title, keywords, caption, by-line, city, credit, …);
  uncommon datasets and non-JPEG IPTC containers are skipped. Dataset **key names** are
  RustCFML's own (`object_name`, `by_line`, …) and may differ from Lucee/ACF.
- **`imageGetBufferedImage`** has no engine equivalent (it returns a
  `java.awt.BufferedImage`) and throws a clear error 🛑.
- **HEIC/AVIF/JXL** decode is intentionally unsupported (their codecs need C libraries
  that don't build for wasm); Lucee treats those as optional codecs too.

## 19. Mixed-in view helper cannot resolve the host Renderer's implicit accessors 🛑 *(GH [#259](https://github.com/RustCFML/RustCFML/issues/259))*

A ColdBox/Preside **view helper** that is *mixed into* the Renderer and then calls one
of the Renderer's **implicit accessors** (e.g. `getController()`) gets `null` back,
because the mixed-in helper runs with the Renderer's `variables` scope but **without
`this`** — so the implicit accessor can't bind to the component instance and returns
the property's empty default instead of `variables.controller`.

Symptom (Preside admin dashboard, `admin.sitetree.index`):

```
cannot call method [renderViewlet] on a null value
```

from `system/helpers/presideProxies.cfm` (`getController().renderViewlet(...)`).
Debug at the call site: `getController() isNull=true`, `variables.controller` exists,
`this` absent.

- Reproduces only for a **mixed-in helper invoked without `this`** calling a host
  implicit accessor. Implicit accessors called from a normal sibling method, or from a
  UDF injected as a member and invoked as a method (both have `this`), resolve
  correctly.
- Likely area: how the engine includes a ColdBox view and mixes in the helper library
  — the mixed-in functions do not retain the Renderer's `this`.
- Impact: blocks rendering ColdBox admin UIs whose view helpers call the Renderer's
  implicit accessors. (Preside boot + admin login/session work as of v0.430–v0.433.)

## 20. Assigning a *function* to a `this` key through a fluent chain is dropped 🔇 *(narrow, related to GH [#260](https://github.com/RustCFML/RustCFML/issues/260))*

`this[ key ] = someClosure; return this;` **inside a method invoked as the 2nd-or-later
step of a fluent chain** (`obj.a().b()`) does not persist the function member — the key
survives but still holds the *original* value, so a call to it dispatches the original
method rather than the injected closure. The equivalent **separate statement** works, and
**scalar/struct** members installed the same way through a chain *do* persist (fixed in
v0.437.0 — GH #260).

Root cause: assigning a *function value* to a `this` key routes through a different path
than scalar assignment and is not captured in the per-call `this` write-back snapshot that
the chain's merge rebuilds from, so the merge keeps the pre-existing method.

This does **not** affect TestBox MockBox, whose `$()` installs overrides via `$include`
(which registers the method properly) — the reported #260 chain (`obj.$("m1",r).$("m2")…`)
works. It only bites hand-rolled fluent builders that inject *closures* onto `this` and are
chained. Use separate statements as a workaround.

*This list is not exhaustive — it captures gaps identified to date. A periodic audit
sweep (e.g. parallel search for "not supported" / accepted-but-unused config keys /
ignored tag attributes) should refresh it.*
