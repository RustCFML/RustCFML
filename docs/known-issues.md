# Known Issues & Unsupported Behaviour

What RustCFML **does not fully do**, as of **v0.574.0**.

Sections are grouped by *what it means for you*, not by when they were found. Section
numbers (`§1`, `§29`, …) are permanent IDs — they are cited from commits and issues, so
they are never renumbered or reused, which is why the numbering inside each group is not
sequential.

| Tag | Meaning |
|---|---|
| 🔇 **silent** | accepted, no error, no effect — the dangerous class, and the priority list |
| 🛑 **loud** | not implemented, but throws a clear message |
| 🌟 **divergence** | works, but deliberately differs from Lucee |
| 🏗 **by design / edges** | implemented; the note records a scoping decision or a known corner |
| 🌍 **environment** | restricted on a specific target (wasm, CLI) |
| ✅ **resolved** | fixed; kept only as an upgrade note |

Compatibility target is **Lucee 7** (BoxLang where Lucee is silent). Anything not marked
*by design* is a gap against that target.

> Maintenance: when you implement around a gap, or skip an attribute or setting, add it
> here **in the same change**, in the group that matches its status — and move it to
> Part F when it is fixed rather than editing the open entry in place. For the positive
> "what *is* supported" view see `docs/configuration.md` and `docs/status.md`.

---

## At a glance

**Part A — Silent no-ops (open) 🔇**

| § | Item | Status |
|---|---|---|
| [1](#1) | Application.cfc `this.*` settings | 🔇 open |
| [2](#2) | Application.cfc lifecycle — `onCFCRequest` | 🔇 open |
| [3](#3) | `.cfconfig.json` keys not enforced | 🔇 open |
| [4](#4) | Per-application isolation (security flags, mail) | 🔇 open |
| [7](#7) | Partially-ignored function/tag parameters | 🔇 open |
| [27](#27) | Tag attributes dropped at lowering (`cfqueryparam`, `cfstoredproc`) | 🔇 open |
| [30](#30) | Java shims — remaining gaps | 🔇/🛑 open |

**Part B — Unsupported, fails loudly (open) 🛑**

| § | Item | Status |
|---|---|---|
| [6](#6) | Functions / tags that throw when unsupported | 🛑 open |

**Part C — Deliberate divergences from Lucee 🌟**

| § | Item | Status |
|---|---|---|
| [15](#15) | Struct iteration order (insertion, not HashMap) | 🌟 won't-fix |
| [17](#17) | `objectSave()`/`objectLoad()` binary format | 🌟 by design |
| [20](#20) | `binary.equals()` compares by value | 🌟 by design |
| [21](#21) | `server.coldfusion.supportedLocales` | 🌟 by design |
| [23](#23) | Custom-tag `caller` read of a shadowed key | 🌟 deferred |
| [39](#39) | `.cfconfig.json` placeholders expand single-pass (GH #306) | 🌟 won't-fix |

**Part D — Implemented, with documented edges 🏗**

| § | Item | Status |
|---|---|---|
| [5](#5) | Server-level cfconfig keys aren't app-level | 🏗 by design |
| [9](#9) | Query-of-Queries superset | 🏗 by design |
| [10](#10) | Query result metadata + `cfdbinfo` | 🏗 edges |
| [11](#11) | `getPageContext()` servlet bridge | 🏗 edges |
| [12](#12) | Session storage, lazy sessions, expiry, cookies | 🏗/🌟 edges |
| [13](#13) | `<cfoutput query>` / grouped output | 🏗 edges |
| [14](#14) | `cfparam` `type=` validation | 🏗 edges |
| [16](#16) | Sampling profiler vs JIT'd numeric leaves | 🏗 by design |
| [18](#18) | Image functions — not pixel-identical to Java2D | 🏗 edges |
| [22](#22) | Within-request template freshness (GH #284) | 🏗 by design |
| [26](#26) | Locale table is hand-maintained (GH #304) | 🏗 edges |
| [38](#38) | Database exception `sqlState` is driver-dependent (GH #295) | 🏗 edges |

**Part E — Environment-specific 🌍**

| § | Item | Status |
|---|---|---|
| [8](#8) | wasm / CLI restrictions | 🌍 |

**Part F — Resolved (upgrade notes only) ✅**

| § | Item | Status |
|---|---|---|
| [19](#19) | Mixed-in view helper vs host implicit accessors (GH #259) | ✅ v0.440.0 |
| [27b](#27b) | Tag-attribute whitelists removed (`cfhttp`, `cfloop`, `cfdump`, …) | ✅ v0.543–0.555 |
| [24](#24) | `writeLog`/`<cflog>` file logging (GH #286) | ✅ v0.528.0 |
| [25](#25) | Member-function dispatch throws on unknown members (GH #307) | ✅ v0.549.0 |
| [28](#28) | Unclosed body tags refused, not erased | ✅ v0.556.0 |
| [29](#29) | Declared parameter / return types enforced | ✅ v0.557.0 |
| [31](#31) | `<cflock>` `scope=` / `throwOnTimeout=` | ✅ v0.553.0 |
| [32](#32) | Page-scope **function** variable visible inside functions | ✅ v0.558.0 |
| [33](#33) | Java `Object` methods on simple values | ✅ v0.558.0 |
| [34](#34) | `createUUID()` random from the first call, v4-shaped | ✅ v0.558.0 |
| [35](#35) | §29 type enforcement rejected two legitimate values (Preside boot) | ✅ v0.560.0 |
| [36](#36) | Built-in scope names were shadowable (ACF behaviour) — GH [#312](https://github.com/RustCFML/RustCFML/issues/312) | ✅ v0.561.0 |
| [37](#37) | A non-SELECT statement returned metadata instead of an empty query | ✅ v0.562.0 |

### Is it getting better?

Yes, and the trend is visible in the table above: Part F has grown by nine sections while
Parts A, B and C shrank — §27's whitelists, §28's erased tag bodies, §29's unenforced
types, §31's collapsed locks, §32's unreachable helpers, §33's missing `Object` methods
and §34's half-zeroed UUIDs were all silent-or-destructive and are now gone. Part B is
down to a single section, and §33's regression against a known-good baseline is closed:
TestBox's own suite is back to 410 pass / 0 fail / 0 error.

Two honest caveats, so the direction isn't oversold:

- **New sections are usually old bugs being *found*, not new breakage.** §32, §33 and
  §34 were all discovered while enforcing declared types (§29); none was caused by it,
  and each was verified as pre-existing against an unmodified earlier binary before being
  fixed. Doing the work is what surfaces them, so expect the list to keep growing at the
  same time as it shrinks.
- **Stale entries flatter the list too.** §19 sat in the open column for ~117 releases
  after being fixed in v0.440.0, because nobody moved it when the issue was closed. If an
  entry looks like it contradicts a workload you know runs, re-probe it before believing
  it — that is what Part F's "move it, don't edit it" rule exists to prevent.

---

# Part A — Silent no-ops (open) 🔇

Accepted without error, no effect. **These are the dangerous ones** — code that relies
on them looks like it works. This is the priority list.

<a id="1"></a>

## 1. Application.cfc `this.*` settings — silently ignored 🔇

Read today: `this.name`, `this.mappings`, `this.sessionManagement`, `this.sessionTimeout`,
`this.customTagPaths`, `this.localMode`, `this.sessionStorage`, `this.cache`,
`this.lazySessionCreation`, `this.datasources`, `this.datasource`,
`this.sessioncookie` (secure/httponly/samesite/domain/path — see §12e),
`this.timezone`, `this.locale`.

`this.timezone` and `this.locale` were fixed in **v0.559.0** — they had been parsed
into the application's settings struct and read by nothing, so
`getApplicationSettings()` reported the declared value while every date and every
`ls*` number stayed on the SERVER's zone and locale. Both now seed the same request
state the cfconfig `runtime.*` keys use; Application.cfc overrides the server
baseline, and `setTimeZone()`/`setLocale()` still override Application.cfc later in
the request. An unusable id is ignored rather than fatal, which is Lucee's verified
behaviour. Pinned in `tests/lifecycle/test_application_timezone_locale.cfm`
(12 assertions, green on both engines).

Accepted but **ignored** (no error, no effect):

| Setting | Notes |
|---|---|


| `this.applicationTimeout` | Per-app value ignored — **and so is the cfconfig `runtime.applicationTimeout`**. The key parses and is seeded into thread contexts, but nothing ever reads it: applications do not time out. (This row previously claimed the cfconfig key "IS applied". It never was.) |
| `this.scriptProtect` | No script-protection filtering of scopes. |
| `this.secureJSON` / `this.secureJSONPrefix` | Per-app value ignored. cfconfig `security.secureJSON*` IS applied (process-global — see §4). |
| `this.nullSupport` / `this.enableNullSupport` | Per-app value ignored — **and so is the cfconfig `runtime.nullSupport`**. The key parses and is seeded into thread contexts but has no consumer; with `"nullSupport": true` an unset variable still throws `expression` rather than returning null. (This row previously claimed the cfconfig key "IS applied". It never was.) |
| `this.clientManagement`, `this.setClientCookies`, `this.setDomainCookies`, `this.clientStorage` | The **client scope is not implemented** at all. |
| `this.invokeImplicitAccessor` | Ignored. |
| `this.serialization`, `this.javaSettings`, `this.compileExtForCFCDirectory`, `this.blockedExtForFileUpload`, `this.triggerDataMember`, `this.sameFormFieldsAsArray`, `this.searchImplicitScopes`, `this.proxyServer`, `this.smtpServerSettings` | No references in the engine — accepted into the component, never consulted. |

Note: any unrecognised `this.X` is captured into an internal `config` map that is then
never read — so nothing throws, but nothing happens either.

<a id="2"></a>

## 2. Application.cfc lifecycle methods — mostly invoked; one gap remains 🔇

| Method | Status |
|---|---|
| `onApplicationStart`, `onApplicationEnd`, `onRequestStart`, `onRequest`, `onRequestEnd`, `onSessionStart`, `onSessionEnd` | ✅ invoked |
| `onError` | ✅ invoked. An uncaught exception in the target page / `onRequest` / `onRequestStart` is handed to `onError(exception, eventName)` (`eventName` is `""` for a target-page error, otherwise the running event method). If `onError` returns normally it owns the response (the engine's default error page is suppressed); if absent the error surfaces as the default error page. When `onError` handles an error, `onRequestEnd` is skipped. *(fixed v0.173.0, issue #145)* |
| `onMissingTemplate` | ✅ invoked (serve mode). A request for a `.cfm`/`.cfc` template that doesn't exist on disk calls `onMissingTemplate(targetPage)` (`targetPage` is the web-root-relative requested path) after `onApplicationStart`/`onSessionStart`. Returning `true` (or nothing) handles the request and suppresses the default 404; returning `false` — or having no handler — falls through to the default 404. `onRequestStart`/`onRequest`/`onRequestEnd` are skipped (Adobe semantics). A throw inside the handler routes to `onError`. Non-CFML 404s (`.html`, images, directory requests) bypass the engine and never reach the handler. The cfconfig front-controller `fallback` remains available as an alternative. *(fixed v0.183.0)* |
| `onAbort` | ✅ invoked on `<cfabort>` / `abort` — fired in place of `onRequestEnd`. `<cfabort showError="msg">` is a *catchable* error and is routed to `onError` instead (Adobe/Lucee parity), not `onAbort`. *(fixed v0.173.0)* |
| `onCFCRequest` | 🔇 Not invoked (no CFC-over-HTTP / remote method dispatch). |

<a id="3"></a>

## 3. `.cfconfig.json` keys — accepted but not enforced 🔇

These deserialize without error but have no runtime effect:

| Key | Notes |
|---|---|
| `server.maxConcurrentRequests` | No concurrency limiting. |
| `server.http2` | Not wired to the HTTP server. |
| `runtime.trustedCache` | Reserved; bytecode-cache trust is driven by `--production`, not this key. |
| `debugging.showExecutionTime` | No timing output. |
| `datasources[].connectionLimit` / `idleTimeout` / `timezone` | Pool tuning / per-DS timezone not applied. (`connectionTimeout` **is** applied — it reaches the pool builder — so it is no longer listed here.) |
| `mailServers[].timeout` | Carried but not applied during send. |
| `caches[].properties.maxObjects` / `defaultTimeout` / `evictionPolicy` | Region **defaults** not applied: a cache has no capacity bound and no eviction policy, and an entry stored with no explicit TTL never expires. A per-entry TTL — `cachePut( id, value, timespan )` — **is** honoured and does expire the entry, so this is narrower than it reads: it bites code that relies on the region's `defaultTimeout`/`maxObjects` instead of passing a TTL per put. |
| `logging.format` | Only `"text"`; other values warn and fall back. |
| `logging.loggers[].appender` | Logger name used; appender ignored. |

**`server.requestTimeout` is enforced as of v0.559.0.** It, `<cfsetting
requestTimeout=N>` and `getPageContext().setRequestTimeout()` all stored a value that
nothing ever compared elapsed time against, so no request was ever aborted — a page
that raised its own timeout expecting protection had none. An overrunning request now
aborts with Lucee's own wording (`Request [<path>] has run into a timeout (timeout: N
seconds) and has been stopped. The thread started Nms ago.`) and, like Lucee's
`RequestTimeoutException`, is **not catchable** by `try { … } catch( any e )` — a
framework catch-all must not be able to swallow the timeout and let the request it was
meant to stop keep running. The limit is re-read on every check, so `<cfsetting>` can
raise or lower it part-way through a request. Pinned in
`crates/cli/tests/request_timeout.rs`. Three things to know: 🏗

- **It fires at blocking points, not from the bytecode loop** — so a tight CPU-bound
  CFML loop still runs to completion. That is deliberate Lucee parity, not an
  oversight: on Lucee 7.0.4 a `while` loop spinning for 8s under a 1-second timeout
  finishes normally, because its watchdog can only interrupt a blocked thread. A
  `sleep()` is interrupted mid-call on both engines; `cfhttp` and `queryExecute` abort
  at the boundary rather than mid-flight (interrupting those needs the remaining budget
  pushed into the client's own timeout).
- **The default is 0 = no timeout**, where Lucee defaults to 50 seconds. 🌟 Enforcement
  only ever applies to a deployment that asked for it, so nothing that runs today
  starts aborting on upgrade — but a deployment expecting Lucee's implicit 50s net has
  to set the key.
- **`server.*` is server-level** (§5), so this cannot be set from an app-level
  `.cfconfig.json` — only from the server config / `--cfconfig`. A `<cfthread>` child
  starts with no deadline of its own and is never killed by its parent's.

<a id="4"></a>

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

<a id="7"></a>

## 7. Partially-ignored parameters 🔇

| Function | Ignored argument(s) | Reason |
|---|---|---|
| `fileSetAccessMode` / file mode setters | mode | No-op on non-Unix platforms. |
| `fileUpload()` / `fileUploadAll()` | `accept` | **No longer a stub** — VM-intercepted, it reads the form scope's `tempFilePath`/`clientFile`, creates the destination directory, honours `nameConflict=makeunique`, and reports the real `serverFile`/`fileWasSaved`. The remaining gap is `accept`: the MIME/extension allow-list is parsed and discarded, so an upload is never rejected on content type. |
| `fileClose(handle)` | — | Stub: returns null, closes nothing (no real file-handle management). |
| `<cfstoredproc>` / `cfprocparam` | `direction`, `dbVarName`, `maxLength`, `scale` | Only `value`/`cfsqltype` survive lowering, so OUT/INOUT stored-proc params don't round-trip. |
| `<cftransaction isolation="…">` | `isolation` | Parsed only to disambiguate the `datasource` arg; the isolation level is never applied to the connection. |
| `queryExecute(…, {timeout=N})` / `<cfquery timeout>` | `timeout` (partial) | Enforced for the **MySQL/MariaDB** driver only (a `KILL QUERY` watchdog aborts an overrunning statement server-side, the JDBC `setQueryTimeout` equivalent). The Postgres, MSSQL and SQLite drivers currently accept the option but do not enforce it. |
| `s3Write` / `s3Upload` / `s3Copy` / `s3Move` | `acl`, `location` | Accepted but not sent to the backend. (`s3CreateBucket` *does* apply both — it is only the object-level calls that drop them.) |
| `s3Read` / `s3Download` | `charset` | Accepted but ignored. |

<a id="27"></a>

## 27. Tag attributes dropped at lowering — per-tag whitelists 🔇

Several tags lower to a builtin by copying a **fixed list** of attributes. Anything
outside that list is discarded at compile time: no error, no effect, and — because the
attribute never reaches the runtime — no "unknown option" either. `<cfquery>`'s
whitelist was removed in v0.543.0 (GH #294) in favour of forwarding every attribute;
the tags below still have theirs.

| Tag | Survives lowering | Silently dropped |
|---|---|---|
| `<cfqueryparam>` | `value`, `cfsqltype`, `list`, `null` | `maxLength`, `scale` — precision/truncation not applied. |
| `<cfstoredproc>` | `procedure`, `datasource` | `returnCode`, `result`, `blockFactor`, `cachedWithin`; a second and subsequent `<cfprocresult>`, and `resultSet=` — only the first result set is bound. |

These two are the **only** rows left; the rest of the original inventory has shipped —
see §27b. Both remaining rows are blocked on the same thing: a database the reference
Lucee can also reach, so the expected precision/OUT-param behaviour can be probed rather
than guessed.

<a id="30"></a>

## 30. Java shims — remaining gaps 🛑/🔇

The shim dispatch contract was reworked in v0.551.0 so that "this shim does not
implement that method" travels out-of-band instead of as `Ok(null)`. A shim's `null` is
now believed, and the operations that used to be silent no-ops (StringBuilder mutators,
`ConcurrentHashMap.replace`, `Collections.sort` on numbers, `TimeZone` offsets, `Date`
comparisons, `File.renameTo`, `Files` I/O, `Optional.orElse*`, `GregorianCalendar`
mutators, `Queue.contains`/`drainTo`, `InetAddress` resolution) do the work. What
remains:

| Shim | Status |
|---|---|
| `ConcurrentHashMap.compute` / `computeIfAbsent` / `computeIfPresent` / `merge` | 🛑 Throws. They take a remapping function, and these handlers are free functions with no VM handle, so a CFML closure cannot be invoked. Previously returned null and never wrote the entry — silently losing the computed value. Needs the VM-intercept treatment the higher-order builtins get. |
| `Queue.take()` | 🛑 Throws. It blocks until an element is available; the shim backs both `ConcurrentLinkedQueue` (no `take()` in Java) and the blocking queues (where it must block) and cannot tell them apart. Use `poll()`. |
| `ChronoUnit.X.between(a, b)` | 🛑 Throws. `ChronoUnit` constants are plain strings, so `.between()` dispatches on a String. Making it work means representing the tokens as shims, which would break code comparing them as strings. |
| `ProcessBuilder` / `Runtime.exec` | 🔇 `directory()`, `environment()`, `redirectOutput()`, `redirectErrorStream()`, `inheritIO()` are ignored; `Process.getInputStream()`/`getErrorStream()` return null so child stdout is unreadable and leaks to the engine console; `Runtime.exec` never launches. Implementing these is a new capability (process spawning with redirected stdio), not a bug fix — deliberately not done. |
| `new SimpleDateFormat(pattern)` | 🔇 The pattern argument is discarded; `.format()` emits the Java MEDIUM style (`Jan 1, 1970`) regardless. |
| `HttpServletRequest.setAttribute` / `getAttribute` / `getSession` | 🔇 Attributes are silently discarded — there is no real servlet state behind the bridge (see §11). |
| Unknown method on a **known** shim class | 🔇 Still returns null rather than throwing. The shim correctly reports "not mine" and falls through to generic dispatch — which must stay, so property access like `system.out` keeps working — but a `__java_shim` struct whose member resolves nowhere does not reach the undefined-member error a plain struct gets. Making that loud is the remaining half of the D2 work. |

---

# Part B — Unsupported, fails loudly (open) 🛑

Genuinely not implemented, but it throws a clear message. Safe to ship against — you
find out at the call site, not in production data.

<a id="6"></a>

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

---

# Part C — Deliberate divergences from Lucee 🌟

Implemented and working, but the behaviour differs from the reference engine on purpose —
usually because RustCFML has no JVM. Each one records why, and what breaks if you depend
on Lucee's exact answer.

<a id="15"></a>

## 15. Struct iteration order — insertion order, not Lucee's HashMap order 🌟 *(divergence)*

RustCFML structs are insertion-ordered (`IndexMap`), so `serializeJSON()`, `for( k in
struct )`, `structKeyList()`, `structKeyArray()` and friends all visit keys in the order
they were added — i.e. RustCFML's default `{}` behaves like Lucee's `structNew("ordered")`.
Lucee/ACF's **default** `{}` (and component metadata + many internal structs) is instead
backed by a Java `HashMap`, whose iteration order is hash-bucket order — neither insertion
nor alphabetical, but deterministic for a given key set (Java `String.hashCode()` is
spec-defined). RustCFML's `structNew("ordered")` and Lucee's `structNew("ordered")` agree;
it is only the *default* struct where Lucee is unordered and RustCFML is ordered.

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

<a id="17"></a>

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

<a id="20"></a>

## 20. `binary.equals(other)` compares by value, not Java reference identity 🌟 *(divergence)*

On Lucee/ACF a binary value is a Java `byte[]`, so `.equals()` is `java.lang.Object`
**reference identity**: two independently-created binaries with identical bytes compare
`false`, and only the *same* array object compares `true`. Consumers rely on this
transitively — a CFC that stores a binary and returns it later hands back the *same*
reference, so `stored.equals(returned)` is `true`.

RustCFML has no JVM and clones `CfmlValue`s freely, so a binary cannot preserve a stable
"reference" through a set/get round-trip. `binary.equals(other)` therefore compares **by
value** (byte-for-byte). This produces the same answer as Lucee for the case consumers
actually depend on (`stored.equals(returned)` → `true`), and only diverges for two
separately-constructed-but-equal binaries: Lucee returns `false`, RustCFML returns `true`
(the intuitive answer). TestBox's `Assertion.equalize()` falls through to `.equals()` for
binary values, so this is what lets binary `expect().toBe()` assertions pass (e.g. Taffy's
`BaseSerializerSpec` / `ResponseHandlingSpec`). The bare `eq` operator on two binaries
still differs too — Lucee throws "can't compare complex object types"; RustCFML currently
returns `false` — but no exercised consumer depends on that edge.

<a id="21"></a>

## 21. `server.coldfusion.supportedLocales` — ACF list, not Lucee's JVM locale set 🌟 *(divergence)*

`server.coldfusion.supportedLocales` is a comma-delimited locale list. It originated as an
Adobe ColdFusion field; Lucee emulates it by returning the JVM's full
`Locale.getAvailableLocales()` set — ~900 entries, and the exact contents vary by the JVM
version Lucee runs on (locale display names plus `en_US_#Latn`-style tags).

RustCFML has no JVM, so replicating that list exactly is infeasible and would be unstable
across builds. Instead it exposes the **ACF-documented supported-locale set** (~47 entries:
`English (US)`, `French (Standard)`, `Japanese`, …) — which is what this field historically
meant and what locale-dropdown consumers were designed around (e.g. Mura/Masa admin
`csettings/editsite.cfm`, whose `isSupportedLocale()` flags anything outside its own set as
deprecated regardless). Apps that enumerate this list get a sensible, stable locale menu;
apps that assume a *specific* JVM locale tag string will see fewer entries than on Lucee.

<a id="23"></a>

## 23. Custom tag `caller` — read of a key shadowed by the calling function's local 🌟 *(divergence)*

The custom-tag `caller` scope is a **live handle** onto the calling frame's variables
scope (Lucee `CallerImpl` / BoxLang `Component.caller` design; replaced the old
snapshot+diff in the caller-scope rework that also fixed lost `structDelete(caller, …)`
and lost new-key writes into CFC callers). **Write** routing is Lucee-faithful, verified
against Lucee 7: a `caller.x` write where `x` exists in the calling method's
`local`/`arguments` scope lands on that scope only (shadow reconciliation), everything
else lands live on `variables`. The one divergence is the **read** of such a shadowed
key: Lucee's caller view reads the method local first; RustCFML's live handle reads the
variables scope. Pinned (tolerantly, green on both engines) in
`tests/tags/test_customtag_caller_semantics.cfm`. Full read-fidelity needs an
intercepting scope value type — deferred.

Related pre-existing (unchanged) page-frame edges, pinned in the same test: a
caller-write of a UDF-local-shadowed key from a page-level UDF also updates variables,
and a caller-write of an `arguments`-shadowed key misses the arguments scope.

<a id="39"></a>

## 39. `.cfconfig.json` placeholder expansion is single-pass 🌟 *(divergence, GH [#306](https://github.com/RustCFML/RustCFML/issues/306))*

`${VAR:default}` substitution in `.cfconfig.json` runs **exactly once** over each string
value. If a resolved value itself contains `${...}`, that text is left verbatim — it is
never re-scanned, at any offset, to any depth. This is deliberate and will not change.

Lucee is inconsistent here rather than recursive. Its importer
(`lucee.runtime.config.CFConfigImport#replacePlaceHolder`) splices the substituted text
in at `startIndex` but resumes scanning at `startIndex + 1`, so with `A="y${B}"` and
`B="zzz"`:

| Input | Lucee | RustCFML |
|---|---|---|
| `${A}` where `A="y${B}"` | `yzzz` — the nested `${` sits at offset 1, so it is re-scanned | `y${B}` |
| `${A}` where `A="${B}"` | `${B}` — the nested `${` lands on `startIndex` and is skipped | `${B}` |

The two engines already agree on the second row. Only the first diverges, and Lucee's
result there is a boundary artifact of the resume index, not a documented rule: the same
value expands differently depending on whether the nested `${` happens to be the first
character.

**Why we don't match it, in either direction:**

- Reproducing the off-by-one bug-for-bug gives a rule nobody can state, let alone rely on.
- Implementing *full* recursive expansion would be a different divergence, not a fix — it
  changes the meaning of a value that today survives verbatim, and it disagrees with
  Lucee on the second row, which currently matches.
- Recursive expansion of environment-supplied text is also the abuse surface. Config
  values come from the deployment environment; letting one env var inject a placeholder
  that expands another turns "set `DB_PASSWORD`" into "set `DB_PASSWORD` and thereby read
  any other variable the process can see", with cycles and expansion blow-ups to bound on
  top. Single-pass makes the value you set the value you get.

Nothing needs it: a config value containing a literal `${` is unusual, and one containing
a nested reference *expected to resolve* more so. If a real dual-engine config turns up
that depends on the nested form, reopen #306 — but the fix would be to flatten the config,
not to add a second pass. Pinned by `env_value_with_dollar_brace_is_not_recursed` in
`crates/cfml-config/src/env.rs`. See also `docs/configuration.md`.

---

# Part D — Implemented, with documented edges 🏗

The feature works. What follows are the known corners, scoping decisions and "by design"
boundaries — not gaps to fix.

<a id="5"></a>

## 5. Server-level keys are not application-level 🏗

The entire cfconfig `server.*` section (host, welcomeFiles, maxRequestBodySize, …) is a
**server/environment** concern and is intentionally **not** overlaid from a per-app
`.cfconfig.json`. There is deliberately **no `port` key** — the listening port is set
via `--port`; pages read `cgi.server_port`. (This is by design, not a gap.)

<a id="9"></a>

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

<a id="10"></a>

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

<a id="11"></a>

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

<a id="12"></a>

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

<a id="13"></a>

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
| Bare column scope | Columns are merged into `variables`, so a page variable sharing a column's name is shadowed for the duration of the loop. `<cfloop query>` bodies perform the same merge (GitHub PR #318), so bare column refs resolve there too. |

<a id="14"></a>

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

<a id="16"></a>

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

<a id="18"></a>

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

<a id="22"></a>

## 22. Within-request template freshness — the process's own writes are picked up, external mid-request edits are not 🏗 *(GH [#284](https://github.com/RustCFML/RustCFML/issues/284))*

In serve-mode **dev**, each request builds a fresh VM and carries a request-scoped
freshness memo (`request_validated_files`): once a template's on-disk mtime has been
validated during a request, repeat `include`s of it skip the per-load `stat`. This is a
deliberate v0.511.0 optimisation — a live Preside profile showed ~33% of all CPU in
`exists()`/`canonicalize()` syscalls — and the memo is dropped at request end, so the
**next** request always re-checks (Lucee `inspectTemplate` per-request parity).

The consequence: if a `.cfm`/`.cfc` is changed **by a process other than rustcfml**
partway through a request, the change is not observed until the next request. Lucee with
`inspectTemplates="always"` re-stats on every access and would observe it mid-request. This
is **by design** — the common case is many repeat includes of unchanged templates, and
paying a `stat` on each to catch a rare external mid-request edit is exactly the cost the
memo exists to avoid.

**A template rewritten by rustcfml *itself* mid-request (`fileWrite`/`fileAppend`/
`fileCopy`/`fileMove`/`fileDelete`, including the `<cffile>` forms) IS picked up on a
subsequent `include` in the same request** — the write flushes that file's freshness memo
and shared bytecode-cache entry by canonical path identity (fixes the v0.511.0 regression
that broke Wheels' `?reload=true` / `$reincludeGlobals` hot-reload flow). Only rustcfml's
own writes trigger the flush; external edits still defer to the next request as above.

**Production mode behaves the same way as of v0.545.0**: it still never re-stats (immutable
tree, restart to reload), but rustcfml's *own* mid-request write does flush, exactly as in
dev. v0.521.0 gated the flush on `!production_mode`, which left production silently serving
the stale compiled unit after a `fileWrite` + re-`include` in the same request — and left
the GH #284 regression test red in `--serve --production` for 23 releases, unnoticed because
the release gate only ever served the runner in dev. The immutable-tree contract covers
*external* edits; a template this process just rewrote is not one.

<a id="26"></a>

## 26. Locale is request state; the `ls*` locale table is hand-maintained 🏗 *(GH [#304](https://github.com/RustCFML/RustCFML/issues/304))*

`getLocale()`/`setLocale()` were inert stubs and cfconfig `runtime.locale` had no
consumer, so the whole `ls*` family behaved as US English regardless of what the
application or config asked for. Locale is now per-request VM state, seeded from
`runtime.locale`, mutated by `setLocale()` (which returns the **previous** locale, in
code form — Lucee's save-and-restore contract) and read by `getLocale()` and the
formatters. An unresolvable locale is an **error**, not a silent fall back to `en_US`.

**Gap:** the number/currency conventions live in a hand-maintained table
(`cfml-common/src/locale.rs`), not a full CLDR/ICU dataset. It covers the ~32 locales
RustCFML names, and an unlisted locale falls back to its language's conventions and then
to `en_US`. Locale-specific **date** formatting (month/day names, date field order) is
**not** yet driven by the locale — `lsDateFormat`/`lsTimeFormat`/`lsParseDateTime` still
behave as `en_US`. Extend the table rather than letting a caller's locale be dropped.

---

<a id="38"></a>

## 38. Database exception members — `sqlState` is driver-dependent 🏗 *(GH [#295](https://github.com/RustCFML/RustCFML/issues/295))*

Database failures now carry the structured detail Lucee/ACF attach, so
`catch( database e )` can branch on **why** a query failed instead of
substring-matching the driver's message: `sqlState`, `nativeErrorCode`,
`errorCode`, `sql`, `queryError`, `datasource`, `where`, and the `additional`
struct. Every member always exists (empty string when unknown), so an unguarded
`e.sqlState` read can never raise a secondary "variable is undefined" the way it
could before GH #250.

Reference behaviour was captured from Lucee 7.0.4 driving pgjdbc, MariaDB
Connector/J and mssql-jdbc. Two things there are worth knowing, because both
contradict the intuitive reading: Lucee's **`errorCode` is a literal `0`** for
every driver — the vendor number lives in **`nativeErrorCode`** — and Lucee's
**`detail` is empty** for database errors on every driver, so ours is too.

**Gaps:**

| Gap | Detail |
|---|---|
| `sqlState` is empty on **SQL Server** | SQL Server's TDS protocol carries no SQLSTATE; `TokenError`'s `state` byte is the TDS error state, an unrelated field. Lucee reports one (`S0002`) only because mssql-jdbc synthesises legacy ODBC states no other driver produces. `nativeErrorCode` carries the real vendor number (`208`, `2627`, …) and is what SQL Server code should branch on. |
| `sqlState` is empty on **SQLite** | SQLite defines no SQLSTATE at any layer. `nativeErrorCode` carries the extended result code (`1555` = `SQLITE_CONSTRAINT_PRIMARYKEY`, `787` = `…_FOREIGNKEY`, …), which is finer-grained than a SQLSTATE would be. Lucee bundles no SQLite driver, so there is no reference behaviour to match. |
| `additional.DatabaseVersion` / `additional.DriverVersion` are empty | Both would need a live round-trip to the server, and this is the error path — the connection that would answer is frequently the thing that just failed. The other four `additional` members (`SQL`, `Datasource`, `DriverName`, `DatabaseName`) are populated. |
| `cfcatch.message` keeps its `queryExecute: ` prefix | Lucee reports the driver's message verbatim. Ours is prefixed with the operation, which is more useful in a log but is a textual divergence. Anything matching on message text was already engine-specific; that is the reason `sqlState` exists. |
| Connection failures are `database`-typed | Lucee surfaces an unreachable server as `java.io.IOException`, not `database`, and attaches none of these members. We deliberately keep the `database` typing from GH #293 (frameworks catch `database` around first-run connectivity probes); such errors carry empty `sqlState`/`nativeErrorCode`, since no server ever answered. |

`datasource` reports the datasource **name**; a datasource passed as an inline
struct reports Lucee's `__temp__` sentinel, and one passed as a raw URL has its
`user:pass@` userinfo stripped — an exception struct routinely ends up in a log
or on an error page and must not carry a password there.

Covered by `tests/database/test_query_error_sqlstate_members.cfm` (SQLite
control gated on driver availability so it skips on Lucee; PostgreSQL, MySQL and
SQL Server legs gated on `RUSTCFML_TEST_PG_DS` / `_MYSQL_DS` / `_MSSQL_DS`).

---

# Part E — Environment-specific 🌍

Restrictions that apply only on a particular target (wasm, CLI vs serve).

<a id="8"></a>

## 8. Environment-specific 🌍

| Feature | Restriction |
|---|---|
| `<cfdirectory>` | Not supported on `wasm32` (no filesystem). |
| `<cfzip>` | Not supported on `wasm32`. |
| `<cflock>` | No-op in CLI mode (no server state); enforced in serve mode. |
| `<cfcache>` | No-op today (could emit Cache-Control in serve mode). |
| `runAsync` / `_schedule` — `delayMs` | On `wasm32` (and other no-real-threads builds) `delayMs` is ignored: the closure runs inline immediately rather than being scheduled. With real threads it is honoured. |
| `_schedule` — `everyMs` / `spacedMs` | Honoured with real threads since v0.572.0 (GitHub #314): `everyMs` is fixed-rate (period measured from each run's start, missed ticks **skipped** rather than burst-replayed), `spacedMs` is fixed-delay (measured from each run's end); `everyMs` wins if both are given. A run that throws is not rescheduled, and `cancel()` stops the schedule and the run in flight. On `wasm32` (and other no-real-threads builds) they are still ignored along with `delayMs` — the closure runs inline exactly once. |
| `java.util.Collections.unmodifiable*` / `synchronized*` shims | Identity no-ops — they return the same collection with no true immutability / synchronization. |

---

# Part F — Resolved (upgrade notes only) ✅

Fixed and shipped. Retained because each one changed behaviour an application could have
been relying on, and because the numbers are cited from commits and issues. **Nothing in
this part is an open problem.**

<a id="19"></a>

## 19. Mixed-in view helper could not resolve the host Renderer's implicit accessors ✅ *(fixed in v0.440.0, GH [#259](https://github.com/RustCFML/RustCFML/issues/259))*

A ColdBox/Preside **view helper** mixed into the Renderer and then calling one of the
Renderer's **implicit accessors** (e.g. `getController()`) got `null` back, so Preside's
admin sitetree page died with `cannot call method [renderViewlet] on a null value` from
`system/helpers/presideProxies.cfm`. The original diagnosis here blamed a missing `this`;
the real cause was narrower and had nothing to do with `this`:

1. **A bare call from an included template dropped page variables.**
   `build_call_parent_scope` picked its inherited-key filter from the nearest *pushed*
   function frame (`call_stack.last()`) — but `__main__`/template frames are never
   pushed, so a bare call from inside an `include`d view (ColdBox's
   `RendererEncapsulator.includeWrapper`) adopted an *outer* method's inherited set
   (`renderViewComposite`) and discarded page vars the template legitimately held,
   `controller` among them. So the mixed-in `getController()` read null. Fixed with a
   `frame_ctx` stack recording the inherited set + is-template flag for **every** frame,
   templates included, so a bare call filters against its true immediate caller. This
   matches Railo's `UDFImpl._call` (never swaps the variables scope) and BoxLang's
   `FunctionBoxContext` (see-through when not `isInClass()`).
2. **`return x = expr` returned null** — a cascade blocker that only surfaced once (1)
   was fixed. The return-value position never requested the assignment's value, so
   Preside's `return alerts = obj.selectData( … )` in `getCriticalAlerts` returned null
   and `criticalAlerts.recordCount` then threw undefined. Fixed in codegen.

Regression tests, both cross-engine verified against Lucee 7:
`tests/tags/test_seethrough_udf_variables.cfm` and
`tests/core/test_return_assignment_expression.cfm`. Re-probed at v0.557.0: the minimal
repro (helper `include`d into a component, called without `this`, reading an implicit
accessor) returns the controller, and the GH #259 suite passes 2/2.

<a id="27b"></a>

## 27b. Tag-attribute whitelists already removed ✅ *(v0.543.0–v0.555.0)*

The counterpart to §27 — the tags whose whitelist has been deleted, in the order they
were taken. Every expectation was probed against
Lucee 7.0.4 before being implemented, and the regression tests
(`tests/tags/test_tags_cfhttp_name_file.cfm`,
`tests/tags/test_tags_cfloop_query_window_group.cfm`,
`tests/tags/test_script_transaction_attrs.cfm`, plus the lowering tests in
`tag_parser.rs`) pass on **both** engines:

- **`<cfhttp>`** — the ten-key whitelist is gone; every attribute is forwarded, as
  `<cfquery>` already does. `name=` now parses the response body into a query
  (`delimiter`, `textQualifier`, `firstRowAsHeaders`, `columns`; blank lines skipped,
  a doubled qualifier is a literal one, a row whose field count differs from the
  column count raises Lucee's own `Invalid CSV line size…` `application` error, and
  cells stay strings so `007` survives). `file=`/`path=` write the body to disk —
  filename derived from the URL when only `path=` is given, next to the calling
  template when only `file=` is, an existing file overwritten, and a missing parent
  directory reported as Lucee's `java.io.IOException`. `throwOnError` now raises the
  `application`-typed `404 Not Found` Lucee raises instead of a generic `runtime`
  error. `redirect`/`port`/`proxyPort`/`encodeURL` were already implemented and now
  simply arrive. **One divergence remains:** `getMetadata(q).typeName` reports
  `VARCHAR` for a numeric-looking column where Lucee reports `DOUBLE` — RustCFML
  infers column types from cell values and has nowhere to record a declared type.
  The cells themselves match Lucee exactly (strings).
- **`<cfloop query=…>`** — `startrow`/`endrow` (and `maxrows`, which Lucee honours here
  too) bound the iteration; a window past the end of the recordset yields nothing
  rather than everything. `group=` does a real control break, with a **bare** nested
  `<cfloop>` as the per-group detail block, recursing for multi-level grouping.
  Grouping applies *after* the row window. This also corrected `groupCaseSensitive`,
  which RustCFML defaulted to `true` (ACF's documented default) for `<cfoutput
  group=>`: Lucee 7.0.4 merges `eng` with `ENG` unless `groupCaseSensitive="true"` is
  given, so the default is now case-**in**sensitive on both tags.
- **script-form `transaction`** — the block form (`transaction datasource="x" { … }`)
  dropped `isolation` and `datasource` while the statement and tag forms forwarded
  them. All three now emit `__cftransaction_start(action, isolation, datasource)` with
  every position filled: they used to emit only the attributes present, which shifted
  `isolation` into the datasource slot, so `<cftransaction isolation="serializable">`
  with no datasource tried to open a connection to a datasource named *serializable*.
  An inline datasource **struct** is also read properly now (as `queryExecute` reads
  one) rather than stringified into a `{class: …}` blob that `parse_datasource` then
  treated as a SQLite file name. Two notes: `datasource=` is a RustCFML **extension** —
  Lucee rejects it at compile time on both forms (`valid attribute names are
  [isolation, savepoint, action]`) — and Lucee's `savepoint=` is accepted-and-ignored
  here, as `isolation` still is (§7).
- **`<cfdump>`** — every attribute now reaches `writeDump`, not just
  var/label/expand/top. `output="console"` keeps the dump out of the HTTP response
  (only the script form did), `output="<path>"` writes the plain-text rendering to a
  file and **appends** to an existing one (path resolved like ExpandPath — a relative
  one against the *base* request template's directory, which is what Lucee does even
  when the tag sits in an included file), a missing parent directory is Lucee's
  `application`-typed `Parent directory for [x] doesn't exist`, and `abort="true"`
  emits the dump and then ends the request. The plain-text dump *layout* still differs
  from Lucee's (`Struct (2) / a = 1` vs `Struct / A number 1`) — a renderer
  difference, not a dropped attribute.
- **`<cffile action="copy"/"move">`** — `nameConflict=` is honoured: `overwrite` (also
  the default), `skip` (destination untouched, no error), `error` (Lucee's
  `application`-typed `Destination file [x] already exists`), and `makeunique` (leaves
  the destination and writes `name-<unique>.ext` beside it). It rides as a third
  argument to `fileCopy`/`fileMove`; a plain two-argument call still overwrites.
- **`<cfinvoke webservice=…>`** — used to travel as a *method argument* with the
  component resolving to `""`. There is no SOAP client in RustCFML, so it now throws
  and says that. Deliberately a **runtime** throw: Lucee compiles the tag happily, so
  an app that merely contains an unreached SOAP call must still start.
- **`<cffile charset=>`** — this one was never only a lowering gap: the
  `fileRead`/`fileWrite`/`fileAppend` BIFs ignored a charset argument too, and
  `charsetEncode`/`charsetDecode` were pass-through no-ops, so *everything* was UTF-8
  whatever the caller asked for. There is now a real encoding layer
  (`cfml-common/src/charset.rs`) covering UTF-8, UTF-16 (BOM), UTF-16BE/LE,
  ISO-8859-1, windows-1252 and US-ASCII, wired through all three file BIFs, the tag,
  and `charsetEncode`/`charsetDecode`. Byte-for-byte Lucee 7.0.4 parity: `utf-16`
  writes an `FE FF` BOM then big-endian units; the BE/LE forms write none; an
  unmappable character becomes `?` on the single-byte encodings; a **BOM wins over the
  requested charset** on read (which is what lets a `utf-16` file be read by a caller
  who asks for `utf-8`, or for nothing); undecodable bytes become U+FFFD rather than
  raising; `fileAppend` appends the full encoding, second BOM included. An
  **unrecognised charset name is now an error** rather than a silent UTF-8 fallback.
  Two things this surfaced along the way:
    - `<cffile action="write"/"append">` appends the platform line separator **by
      default** and takes `addNewLine="false"` to suppress it (Lucee writes 4 bytes for
      `abc` from the tag, 3 from the `fileWrite()` BIF). RustCFML did neither — it wrote
      3 from both and ignored `addNewLine` — so every file written through the tag was
      a separator short of Lucee's. Fixed; the BIFs still write exact bytes.
    - `fileReadBinary()` returns a `Binary`, where Lucee returns a byte **array**. So
      `len()` agrees on both engines but `arrayLen()`/`b[1]` only work on Lucee. Not
      touched here — it is a value-model difference, not an encoding one. 🌟

`<cflock>` used to head this list — with `scope=` and `throwOnTimeout=` both discarded,
every scope lock collapsed onto the single name `"default"` (unrelated scopes, and
unrelated applications, serializing against each other) and a contended lock always
threw. Fixed in v0.553.0: see §31. Note that the loss was in the **runtime**, not the
lowering — all three `cflock` lowerings already forwarded every attribute, and
`__cflock_start` simply never read them. Attribute plumbing is worth checking at both
ends.

<a id="24"></a>

## 24. `writeLog` / `<cflog>` — file logging ✅ *(fixed in v0.528.0, GH [#286](https://github.com/RustCFML/RustCFML/issues/286))*

Resolved. `<cflog>` / `writeLog()` now write to `<log-dir>/<name>.log` through cached,
rotating file appenders, in Lucee 7's exact line layout:

```
"Severity","ThreadID","Date","Time","Context","Application","Message"
"ERROR","tokio-rt-worker","07/26/2026","22:56:48","http://127.0.0.1:8500","MyApp","boom"
```

Log directory: `logging.logsDirectory` from `.cfconfig.json`, else `<webroot>/logs`
under `--serve`, else `./logs` under the CLI. The resolved path is readable from CFML as
`server.cfconfig.logging.logsDirectory`.

Semantics verified against Lucee 7.0.4 and pinned in
`tests/tags/test_cflog_file_logging.cfm`: `type=` → log4j2 severity (an unknown type is
an error); no `file=`/`log=` targets the `application` log; `log=` names a *configured*
logger and falls back to `application` when unknown, whereas `file=` creates the file; a
path separator in `file=` is an error; `application="false"` blanks the Application
column (the attribute defaults to true).

Config knobs (all under `logging`): `logsDirectory`, `cfmlLevel` (default threshold),
`loggers.<name>.level` (per-log threshold, `off` to mute), `maxFileSize` (default 10 MB),
`maxFiles` (default 10 rotated generations), `flushEachLine` (default `true`, log4j2's
`immediateFlush`; `false` batches until request end), `echoToStderr` (default `false` —
Lucee doesn't echo to the console either; the `RUSTCFML_LOG_STDERR` env var forces it on).

Rotation is size-based, rolling to `<name>.log.<n>.bak` at 10 MB — the naming and
threshold Lucee's resource appender produces (confirmed by overflowing a log on Lucee
7.0.4). Remaining gap: no time-based (daily) rolling policy.

<a id="25"></a>

## 25. Member-function dispatch — unknown members now throw ✅ *(fixed in v0.549.0, GH [#307](https://github.com/RustCFML/RustCFML/issues/307))*

`call_member_function` used to end in a bare `Ok(CfmlValue::Null)`. Components (GH #220)
and plain structs (GH #285) had each been tightened to throw, but **Array, String, Query,
numeric, Boolean, Binary and TimeSpan receivers stayed lenient** — so every gap in the
member tables was a *silent* no-op rather than an error.

That is what made GH #307 dangerous: `filtered.add(x)` appended nothing and threw nothing,
so calling code took the success path with an empty array (Preside's
`TaskManagerService.listTasks()` returned `[]` for every call). Assigning the resulting
Null also trips the PR #112 null-delete guard, so the failure typically resurfaced far
away as a misleading `Variable 'X' is undefined`.

Unknown members on those receivers now throw
`The function [x] does not exist in the <Type>.`, matching Lucee. The missing members
themselves were also wired up: the `java.util.List` passthroughs on arrays
(`add`/`get`/`remove`/`removeAll`/`retainAll`/`subList`/`containsAll`/`indexOf`/
`lastIndexOf`), the `java.util.Map` passthroughs on structs
(`put`/`putIfAbsent`/`remove`/`containsKey`/`containsValue`/`keySet`/`values`/`entrySet`),
the `java.lang.String` passthroughs (`charAt`/`substring`/`concat`/`equals`/
`equalsIgnoreCase`/`compareTo`/`hashCode`/`replaceAll`/`isBlank`), and the CFML array
members whose BIFs already existed but were never mapped (`pop`/`shift`/`unshift`/`swap`/
`resize`/`set`/`splice`/`mid`/`median`/`toStruct`/`removeDuplicates`/`indexExists`/…).

**Behaviour changes to be aware of when upgrading:**

- `indexOf`/`lastIndexOf` on **both arrays and strings** are the Java methods:
  **0-based, returning −1 when absent**. They were previously aliased onto `find`
  (1-based, `0` when absent), which silently made `if ( x.indexOf(v) >= 0 )` always true
  and every hit off by one. `find()` itself is unchanged and still 1-based.
- `arrayResize` fills with **null**, not empty strings (Lucee parity).
- `arraySet`/`arraySwap` **throw** on an under-supplied call instead of returning `false`
  and mutating nothing.

Remaining divergences in this area (all minor, all verified against Lucee 7.0.4):

| Member | RustCFML | Lucee |
|---|---|---|
| `struct.values()` / `.keySet()` / `.entrySet()` | CFML array (iterable, castable) | live `java.util.Collection` views; the `Values` view can be neither cast nor looped |
| `array.deleteNoCase(v)` | returns boolean "was it found" | returns the array |
| `array.unshift(v)` | returns the array | returns the new length |
| `date.noSuchMember()` | reports type `String` | reports type `Datetime` (RustCFML dates are strings) |

<a id="28"></a>

## 28. Unclosed body tags — refused, not erased ✅ *(fixed in v0.556.0)*

A body-bearing tag with no closing tag used to make the preprocessor return an empty
string for it, so **the tag *and its entire body* vanished from the compiled output** —
a compile-time construct that quietly deleted code (the same failure mode as the
`<cfloop>` fallback fixed in v0.550.0). `<cfsavecontent>` with a missing
`</cfsavecontent>` dropped the content and never set its variable; `<cfmail>` sent an
empty message; `<cfquery>` leaked its SQL into the *page* and ran nothing.

Which tags require closing was probed per tag on Lucee 7.0.4 — the original inventory
here was wrong in both directions:

| Tag | Lucee | RustCFML now |
|---|---|---|
| `<cfoutput>`, `<cfsilent>`, `<cfstatic>`, `<cflock>`, `<cftransaction>`, `<cfsavecontent>`, `<cfmail>`, `<cfswitch>`, `<cfloop query=…>` | compile error: `No matching end tag found for tag [X]` | the same error, same wording |
| `<cfquery>` | **compiles**, then fails at runtime — a cfquery with no body has no SQL: `You need to define the attribute [SQL] or define the SQL in the body of the tag.` | the same runtime error (so a template that merely *contains* the mistake still compiles) |
| `<cfhttp>`, `<cfexecute>`, `<cfmodule>`, `<cfthread>` | **legal unclosed** — the tag runs attribute-only and the body stays page content | unchanged; already matched. These were wrongly listed as broken here |
| `<cfscript>` | `invalid construct` | `Unclosed <cfscript> tag: missing </cfscript>` — errors, wording differs |

Two things still outstanding:

- `<cfif>`, `<cfloop>` (the non-query forms), `<cffunction>` and `<cftry>` lower to a
  bare `{` opener, so an unclosed one surfaces as the script parser's generic
  `Parse error` rather than Lucee's `No matching end tag found for tag [cfif]`. It
  fails loudly either way — the message is just less useful. 🏗
- `<cfspreadsheet action="write">` could not be probed: the reference Lucee here has no
  spreadsheet extension installed (`undefined tag [cfspreadsheet]`). RustCFML currently
  treats an unclosed one as attribute-only, like the `<cfhttp>` family. 🏗

<a id="29"></a>

## 29. Declared function types — now enforced ✅ *(fixed in v0.557.0)*

`param_type` and `return_type` used to be carried through the parser and codegen into
`BytecodeFunction` and then read by nothing but `getMetadata()` (plus a component-type
check on arguments), so a declared primitive type was a comment:

```cfml
function f( required numeric n ) { return n; }
f( "notanumber" );                    // -> "notanumber", no error (Lucee throws)

function g() returntype="numeric" { return "abc"; }
g();                                  // -> "abc", no error
```

Both are now `expression` errors, with Lucee's own wording, in argument and return
position, for script and tag declarations, for closures and arrow functions, for
named / positional / `argumentCollection` calls, and for an applied default. The
rules live in `crates/cfml-vm/src/type_check.rs`; every one was probed against Lucee
7.0.4 first and `tests/functions/test_fn_type_enforcement.cfm` (95 assertions) passes
on both engines. Two properties of the reference behaviour are worth knowing:

- **Validation, not coercion.** A `numeric` parameter given `"123"` receives the
  *string* `"123"`. Nothing is converted, in either direction, ever.
- **A type name with no cast target is treated as a component path**, so it rejects
  *every* value: `integer`, `int`, `long`, `short`, `byte`, `char`, `float`, `double`,
  `decimal`, `email`, `creditcard`, `url`, `base64`, `usdate`, `eurodate`, `hex`,
  `path`, `node`, `closure`, `lambda` and `udf` all throw unconditionally —
  `function f( integer i )` throws on `f( 5 )`, and `email` throws on `"a@b.com"` —
  while Lucee's own `isValid( "integer", 5 )` says true. This is mirrored
  deliberately: on Lucee such a call is unconditionally fatal, so no Lucee-tested app
  can contain a reachable one, and diverging would mean accepting code the reference
  engine rejects. The names that DO have a cast target: `any`, `string`, `numeric`,
  `number`, `boolean`, `bool`, `date`/`datetime`/`time`, `timespan`, `array`,
  `struct`, `query`, `binary`, `xml`, `function`, `uuid`, `guid`, `variablename`,
  `component`, `object`, `void`, a CFC/interface path, and `T[]` (validated
  element-by-element, recursively).

Some acceptance cells are surprising and are Lucee's, not ours: a boolean satisfies
`numeric`; any number satisfies `boolean`; a numerically-keyed (or empty) struct
satisfies `array` while `{ a : 1 }` does not; a component satisfies `struct`; binary
satisfies both `string` and `array`; and a numeric *string* satisfies `date`.

Three things this surfaced on the way, all fixed here:

- **Dotted return types were mangled.** `pkg.sub.Res function make( any a )` captured
  its return type as `pkgsubResmakeany` — the capture loop peeked by index while
  advancing the cursor, so it spliced in the function name and its first parameter
  type. Invisible while the value only reached `getMetadata()`; fatal the moment the
  type is enforced. A method declaring a package-qualified return type of its own
  package now also accepts the instance it built (resolution may name that instance
  webroot-relative rather than mapping-qualified; leaf names are compared in both
  directions).
- **Two declaration forms were being dropped.** `numeric function f()` at page scope
  (the prefix form *without* an access modifier) lost its return type entirely, as did
  `function f() returntype="numeric" {}` (the post-paren attribute form) and any
  closure's `returntype=`. All three now carry it — so all three also report it in
  `getMetadata()`, which they did not before.
- **`isXml()` accepted an unclosed element** (`isXml("<a>")` was true, Lucee says
  false) and **`isDate()` rejected slash-separated ISO order** (`2020/1/2`, which
  Lucee accepts). Both are load-bearing for `xml`- and `date`-typed parameters.

Remaining divergences, both consequences of the value model rather than of the check
🌟:

- RustCFML dates are **strings**, so a date value satisfies `string` and `binary`
  (Lucee's DateTime does not) but not `numeric` (Lucee's does), and a date named in a
  mismatch message reads `String [2026-08-03 22:36:31]` where Lucee reads
  `Object type [DateTime]`.
- A query column reached by dot notation is an **Array** here and a scalar there, so
  `q.name` passed to a typed parameter is described as `Object type [Array]` rather
  than by its single value.

Engine-**generated** property accessors are exempt, as they are on Lucee: a generated
`getNum()` on `property name="num" type="numeric"` reports `numeric` in metadata and
still returns `""` happily, and a generated `setX()` reports `void` while returning
`this` for chaining. Enforcing either would break CFCs that are legal on the reference
engine. `cfparam`/`param` `type=` enforcement is separate — see §14.

<a id="31"></a>

## 31. `<cflock>` `scope=` and `throwOnTimeout=` ✅ *(fixed in v0.553.0)*

Both attributes reached the compiler and were then dropped by the runtime, which read
only `name`, `type` and `timeout`. Consequences: every `scope=` lock fell back to the
literal lock name `"default"`, so `scope="application"` in one app serialized against
`scope="session"` in another (a concurrency-correctness bug, not a missing option), and
`throwOnTimeout="false"` still threw on contention.

Now each scope gets its own lock, discriminated by *which* application / session /
request it belongs to:

| Form | Lock identity |
|---|---|
| `name="x"` | The name verbatim, process-wide (unchanged). |
| `scope="server"` | One lock for the whole process, shared by every application. |
| `scope="application"` | Per application — two apps do not contend. |
| `scope="session"` | Per session, within the application. |
| `scope="request"` | Per request — keyed on the request scope's backing store, so `<cfthread>` children (which share that scope) contend with their parent, and separate requests do not. |
| neither | A single lock named `"default"`, distinct from every scope lock. Lucee also treats the bare form as its own lock. |

`name=` and `scope=` are mutually exclusive and now raise Lucee's own error
(`type="lock"`, "invalid attribute combination"), and a timeout raises a `lock`-typed
exception with Lucee's wording — `a timeout occurred after 1 second trying to acquire a
exclusive lock with name [x].` / `… a read-only [application] scope lock.` A sub-second
timeout is expressed in milliseconds, as Lucee expresses it. Previously both were
generic `runtime` errors, so `catch( lock e )` could not see them.

Every one of those behaviours was probed against Lucee 7.0.4 before being implemented;
`tests/tags/test_tags_cflock_scope.cfm` passes 7/7 on both engines.

Two things to know:

- **`scope="session"` with session management off** has no session to key on, so it
  degrades to the application lock rather than silently becoming process-global. 🏗
- **`throwOnTimeout="false"` skips the body.** That is the reference behaviour, but it
  means the guarded work silently does not happen — the acquire result is not surfaced
  to CFML, so there is no way to branch on it. Lucee has the same shape.

<a id="32"></a>

## 32. Page-scope variables holding a function ✅ *(fixed in v0.558.0)*

A page-level variable whose value was a function EXPRESSION could not be reached from
inside any function body — not bare, not as `variables.x`, from a named function or from
a closure:

```cfml
cl = function( x ) { return "called:" & x; };

outer = function()      { return cl( "v" ); };            // Variable 'cl' is undefined
function reader()       { return cl( "n" ); }             // Variable 'cl' is undefined
function scopedReader() { return variables.cl( "n" ); }   // Variable 'cl' is undefined
```

All three now resolve, as they always did on Lucee 7, including when the callee closure
is declared *after* the function that calls it (the read happens at call time). So the
very common "define helpers as closures at the top of a `.cfm`, use them lower down
inside other functions" style works.

The frame seed in `execute_function_body` skipped every `CfmlValue::Function` that
carried a captured scope, reasoning that it was already reachable through
`user_functions`. That is true of a *declared* function, but a function expression is
registered under its synthetic `__closure_N` name rather than the variable it was
assigned to, so it was simply dropped. Declared functions are still skipped, so a
component with many methods does not re-seed them into every method frame.

Carrying them exposed a second, opposite bug in `build_call_parent_scope`: its
helper-function carve-out let a caller's OWN `var`-scoped function local past the
caller-locals filter, which is dynamic scoping — Lucee reports `isDefined` false in the
callee and throws on a bare call. It is now limited to the captured-scope-stripped
values `closure_env_capture_value` produces (PR #198), which are genuinely unreachable
otherwise. `tests/functions/test_page_function_vars.cfm` passes 13/13 on both engines.

<a id="33"></a>

## 33. Java `Object` methods on simple values ✅ *(fixed in v0.558.0)*

Lucee boxes a CFML simple value as a Java object, so the `java.lang.Object` /
`Comparable` methods are callable on it. RustCFML implemented them for `String` only;
on every other receiver `equals`, `hashCode` and `compareTo` threw ("The function
[equals] does not exist in the Numeric."). The methods were always missing — v0.549.0
(§25) only made the gap loud by making unknown members throw instead of returning null.

That cost **8 tests in TestBox's own suite**. `equalize()` compares numerics, arrays and
structs itself and only falls through to `actual.equals( expected )` once the two have
already differed — i.e. exactly the `isNotEqual` path — so a throw where Lucee returns
`false` failed the assertion instead of passing it. TestBox's own suite is back to
**410 pass / 0 fail / 0 error / 22 intentional skips**, its v0.493.0 baseline.

`crates/cfml-vm/src/java_shims.rs` now carries Java-exact `java_equals` /
`java_hash_code` / `java_compare_to`: type-strict equality with no CFML coercion (so
`1.equals("1")` and `true.equals(1)` are both false, and a whole-number double is not an
int), `Long`/`Double`/`Boolean`/`String` hashing, `java.util.List` hashing for `Array`,
and `java.util.Map` hashing for `Struct` — the sum of per-entry `keyHash ^ valueHash`
with keys hashed UPPER-cased, the casing Lucee's case-insensitive `Struct` stores them
in, so `{a:1}.hashCode()` is 64 and not 96. Every value was read off Lucee 7.0.4 first;
`Query` and `TimeSpan` are deliberately excluded rather than guessed, and components keep
the Java IDENTITY semantics they already had (§25, ColdBox's `BaseProxy`).
`tests/functions/test_java_object_methods.cfm` passes 48/48 on both engines.

Two residual divergences, both deliberate: 🌟

- **`compareTo` on mixed numerics compares numerically.** Lucee throws a raw JVM
  `ClassCastException` ("class java.lang.Long cannot be cast to class java.lang.Double")
  for `x = 1.5; x.compareTo( 2 )`, so this only affects inputs Lucee refuses outright.
- **`hashCode` of a negative or large-magnitude integer literal differs**, because Lucee
  boxes `-1` and `4294967296` as `Double`s while boxing `1` as a `Long`; RustCFML keeps
  them integral. Each engine is self-consistent and Java-correct for its own boxing, so
  only a hash compared *across* engines sees this.

<a id="34"></a>

## 34. `createUUID()` — random from the first call, and v4-shaped ✅ *(fixed in v0.558.0)*

The first `createUUID()` in a process always returned a UUID whose first block was
`00000000` (`00000000-CFC5-A584-879E7B7161971634`); every later call was random. So two
processes that each generated exactly one could collide, and a caller using the leading
block as a shard/prefix key got a hotspot. Separately, no UUID carried the RFC 4122
version-4 nibble that Lucee's do, so nothing inspecting the version saw a v4 UUID.

The zeroed block was a self-cancelling XOR, not weak entropy. `cfml_random()` lazily
seeded from `now_unix_nanos()` and returned the raw seed as `(next >> 11) / 2^53`, so the
first value of the stream multiplied by `u32::MAX` came out to exactly `nanos >> 32` —
precisely the word `fn_create_uuid` XORed it against. Later calls were unaffected because
`xorshift64` had already advanced the state off the clock.

Seeding now mixes the clock through splitmix64, together with a per-thread distinguisher
and a process-global counter so threads and processes starting inside the same tick still
diverge, and advances once before first use. `createUUID` draws two full 64-bit words and
stamps the version and variant bits, so its output is v4-shaped like Lucee's.
`createUniqueID` shared the construction — its first four bytes collapsed the same way,
showing up as a leading `AAAAA` — and now shares the generator. `randomize( seed )`
reproducibility is untouched; only the lazy path changed.

`tests/stdlib/test_uuid_shape.cfm` passes 14/14 on both engines. The
first-call-in-a-process property cannot be observed from a suite that has already drawn
from the PRNG, so it is pinned in `cfml-stdlib`'s `uuid_tests`, which spawns a fresh
thread to get a fresh thread-local PRNG.

One unrelated divergence this surfaced: `createUniqueID( "counter" )` advances on both
engines but is **encoded** differently — Lucee base-36s the counter (`2q`, `2r`) where
RustCFML emits decimal (`1`, `2`). 🌟

<a id="35"></a>

## 35. §29 type enforcement rejected two legitimate values ✅ *(fixed in v0.560.0)*

Enforcing declared types (§29, v0.557.0) turned two long-standing internal
representations into hard errors. Neither was a new bug — both had been invisible for
as long as declared types were comments — and together they stopped **Preside booting**
on the first release where a user actually ran an enforced build.

**A self-typed method called from a pseudo-constructor.** A component's `this` carries
the parser's `Anonymous` placeholder for as long as its pseudo-constructor body runs;
the real name is stamped onto the finished instance afterwards. So a method declared to
return its own type, *called from the pseudo-constructor*, returned an instance that
could not name itself:

```cfml
component {
    reset();                                      // called during construction
    LogBoxConfig function reset() { return this; }
}
```

→ `The function [reset] has an invalid return value , [Cannot cast Object type
[Component Anonymous] to a value of type [LogBoxConfig]]`. That is ColdBox's
`LogBoxConfig` verbatim. `getMetadata()` already compensated from the in-construction
name stack (GH #212); the type checker now consults it too, and only for a value that
cannot name itself — a component that *can* is matched normally, so an unrelated type
still cannot slip through.

**A query column against a simple declared type.** `q.col` is a `QueryColumn` — a proxy
standing in for its current row's cell, not a collection. `isArray( q.col )` is false on
Lucee 7 and here, and every scalar context (comparison, coercion, `Len`) already treated
it as that one cell. The type checker was the one place that did not, so:

```cfml
string function getDbVersion() {
    return versionRecord.version_hash;            // -> Object type [Array]
}
```

was refused — Preside's `SqlSchemaVersioning.getDbVersion`. The checker now resolves a
`QueryColumn` to the cell it proxies for every target except `array`, which keeps
accepting the raw column exactly as before, so nothing that passed previously fails now.
The mismatch *message* was wrong too: it named a `QueryColumn` "Object type [Array]", a
type the value does not have. It is now named by the cell it stands for.

Pinned in `tests/functions/test_pc_self_type.cfm` and
`tests/types/test_querycolumn_declared_types.cfm` (11 assertions, green on both engines).

The lesson for the remaining §29 surface: type enforcement is only as correct as the
engine's internal value model is honest, and an internal representation that *behaves*
like a scalar everywhere else has to satisfy a scalar declaration too. A representation
that leaks through the checker will present as a Lucee incompatibility even though the
declaration is being read correctly.

<a id="36"></a>

## 36. Built-in scope names are reserved for a bare read ✅ *(fixed in v0.561.0, GH [#312](https://github.com/RustCFML/RustCFML/issues/312))*

A bare read of a built-in scope name inside a function resolved to a same-named
parameter or `var` local instead of the scope. On Lucee 7 the scope names are **fully
reserved**: bare `request` is always the request scope, and the shadowing value is
reachable only through its explicit qualifier (`arguments.request`). Verified uniform
across `request`, `cookie`, `url`, `form`, `cgi`, `session`, `application`, `server`
and `variables`, and for a `var` / `local.` declaration as well as a parameter — there
is no per-scope exception.

This was an **ACF-vs-Lucee fork where RustCFML had taken the ACF route**, and it left
two internal resolvers contradicting each other, which is worse than either answer:

```cfml
request.wheels = { tenant = { id = "FROM-SCOPE" } };

function handler( required struct request ) {          // shadows the request scope
    isDefined( "request.wheels.tenant" )              // true   — answered from the SCOPE
    request.wheels.tenant.id                          // THROWS — answered from the ARGUMENT
}
```

So a correctly guarded read still blew up. That is the Wheels middleware pipeline
exactly — it hands core handlers a `required struct request` — and it accounted for
**all 7** remaining non-passing Wheels specs (5 × `Variable 'tenant' is undefined`,
2 × a cleanup `StructDelete` operating on the wrong object). Wheels is now **2740 pass
/ 0 fail / 0 error**.

Two things the fix had to keep intact:

- **§256's `arguments.<name>` behaviour.** That issue — an omitted defaulted parameter
  binding the live scope instead of its default — is a *different* path, and both
  engines already agreed on it. Its store-side half stays too: `var cookie = …` writes
  to `local`, never into the live scope, which is also what Lucee does. Note §256's
  stated premise ("a scope name has no special meaning in a parameter list") is only
  half true for Lucee 7 — true for `arguments.<name>`, false for a bare read. The
  reporter was describing ACF, which is how the wrong fork got taken.
- **The default-argument preamble.** Codegen seeded `arguments.<name>` from an applied
  default by storing the value and then reading it back with `LoadLocal(param.name)`.
  Once a bare scope name resolved to the scope, that read-back handed
  `arguments.cookie` the live cookie scope. Now the freshly-evaluated value is kept on
  the stack (`Dup`/`Swap`) instead of being re-read — fixed at all three sites that
  emit the preamble (declared functions, closures, arrow functions).

`tests/core/test_scope_named_default_param.cfm` was rewritten: it had asserted the ACF
behaviour, and it **passed here while erroring on Lucee 7** (`Can't cast Complex Object
Type [COOKIE scope] to String`). It now covers both rules and passes 33/33 on both
engines — a test that fails on the reference engine being the clearest possible signal
that the wrong fork was taken.

<a id="37"></a>

## 37. A non-SELECT statement returns an empty query ✅ *(fixed in v0.562.0)*

`queryExecute()` on an INSERT / UPDATE / DELETE returned the **mutation-metadata
struct** (`{recordCount, cached, sql, executionTime [, generatedKey]}`) as its value.
Lucee returns an **empty query** — verified on Lucee 7.0.4 over MySQL, where all three
statement kinds hand back `QUERY(recordCount=0)` and the affected-row count and
generated key are exposed *only* through the `result=` struct.

So a `query`-declared function wrapping a mutation failed §29 type enforcement:

```cfml
private query function _deleteSessionRecord( required string sessionId ) {
    return sqlRunner.runSql(
          sql = "delete from psys_session_storage where id = :id"
        , dsn = _getSessionStorageDsn()
        , params = [ { type="cf_sql_varchar", value=arguments.sessionId, name="id" } ]
    );
}
```

→ `The function [_deleteSessionRecord] has an invalid return value , [Cannot cast Object
type [Struct] to a value of type [query]]`. That is Preside's `SessionStorage`, and it
broke the admin route — the third §29 casualty after §35's two, and the same root shape:
an internal representation that no declaration had ever been able to see.

The conversion happens at the `queryExecute` return boundary rather than in the four
drivers, because that struct is the internal carrier the `result=` / `name=` delivery
reads `recordCount` and `generatedKey` out of — it has to survive until those are built.
A `returntype="struct"` SELECT is also a struct and is deliberately *not* caught: the
discriminator is the `executionTime` + `recordCount` + `cached` triple, which only the
mutation metadata carries.

`result=` is unchanged and still reports rows affected, which
`tests/database/test_dml_returns_empty_query.cfm` pins alongside the return shapes
(16 assertions). It runs on SQLite so no server is needed; Lucee ships no SQLite JDBC
driver, so it skips there with a single informational pass rather than spraying false
reds — the cross-engine evidence was taken on MySQL, where the two engines agree
exactly, including `result.recordCount`.

---

*This list is not exhaustive — it captures gaps identified to date. A periodic audit
sweep (e.g. parallel search for "not supported" / accepted-but-unused config keys /
ignored tag attributes) should refresh it. The most recent such sweep was 2026-08-02;
its findings have been merged into the sections above, and everything it identified as
already-fixed or since-fixed (v0.549.0–v0.551.0) has been dropped rather than carried
forward.*

**Last re-probe: 2026-08-04 (v0.557.0)** — §2, §3 and §4 re-verified against the code
and all rows still hold (`maxConcurrentRequests`, `http2`, `trustedCache`,
`showExecutionTime`, `connectionLimit`, `idleTimeout`, `evictionPolicy` and
`loggers[].appender` have no consumer outside the schema; the SMTP transport sets port
and credentials but never a timeout; `security_flags()` is still a process-wide
`OnceLock`; `onCFCRequest` is attached as a lifecycle name and never dispatched). Four
corrections came out of it: §19 had been fixed for 117 releases (moved to Part F),
`setTimeZone()` and `fileUpload()`/`fileUploadAll()` are no longer no-ops, and
`writeDump(output="<path>")` writes the file (all three rows corrected or dropped from
§7). One previously undocumented no-op was added to §3 — nothing enforces a request
timeout by any route.

> A caution learned from that sweep: an audit's "what Lucee does" column is a claim, not
> a fact. Three of its entries were wrong (`System.arraycopy` — Lucee throws
> `ArrayStoreException` rather than copying, because a CFML array is not a Java array;
> `Optional.orElseGet` and `Files.write` — Lucee cannot express either call with CFML
> types), and its largest section described work that had already shipped. Probe the
> reference engine before acting on a compatibility claim.