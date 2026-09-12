# Known Issues & Unsupported Behaviour

What RustCFML **does not fully do**, as of **v0.672.0**.

Sections are grouped by *what it means for you*, not by when they were found. Section
numbers (`§1`, `§27`, …) are permanent IDs — they are cited from commits and issues, so
they are never renumbered or reused, which is why the numbering inside each group is not
sequential.

| Tag | Meaning |
|---|---|
| 🔇 **silent** | accepted, no error, no effect — the dangerous class, and the priority list |
| 🛑 **loud** | not implemented, but throws a clear message |
| 🌟 **divergence** | works, but deliberately differs from Lucee |
| 🏗 **by design / edges** | implemented; the note records a scoping decision or a known corner |
| 🌍 **environment** | restricted on a specific target (wasm, CLI) |

Compatibility target is **Lucee 7** (BoxLang where Lucee is silent). Anything not marked
*by design* is a gap against that target.

> Maintenance: when you implement around a gap, or skip an attribute or setting, add it
> here **in the same change**, in the group that matches its status. When it is fixed,
> **delete the section in the same change as the fix** — this document describes only what
> is broken *now*. The history is not lost: section numbers are permanent, so a deleted
> §n stays cited from its commit, and the tagged release commit carries the detail.
> For the positive "what *is* supported" view see `docs/configuration.md` and `docs/status.md`.

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
| [109](#109) | `a &= "X"` on a parameter under `localmode="modern"` does not write through to `arguments` | 🌟 to fix |
| [110](#110) | Bare write to a DELETED parameter stays frame-local (Lucee: `variables`) | 🌟 deferred |

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
| [16](#16) | Sampling profiler vs JIT'd numeric leaves | ✅ resolved (v0.653.0) |
| [18](#18) | Image functions — not pixel-identical to Java2D | 🏗 edges |
| [22](#22) | Within-request template freshness (GH #284) | 🏗 by design |
| [26](#26) | Locale table is hand-maintained (GH #304) | 🏗 edges |
| [38](#38) | Database exception `sqlState` is driver-dependent (GH #295) | 🏗 edges |
| [41](#41) | `application` scope is live in-process, not across cluster nodes | 🏗 by design |
| [46](#46) | Member-function dispatch lowercases the method name per call | 🏗 edges |
| [47](#47) | Surplus built-in function arguments accepted, not rejected | 🏗 edges |
| [48](#48) | Elvis operator accepts any left operand (Lucee restricts it) | 🏗 edges |
| [49](#49) | `fileOpen( f, "write" )` does not create the file | 🏗 edges |
| [50](#50) | AntiSamy sanitiser — cosmetic divergences from the Java library | 🏗 edges |
| [51](#51) | Tag-mode parsing — two constructs compile here that Lucee rejects | 🏗 edges |
| [53](#53) | `private`/`package` methods are gated on CALLS, not on member reads | 🏗 edges |

**Part E — Environment-specific 🌍**

| § | Item | Status |
|---|---|---|
| [8](#8) | wasm / CLI restrictions | 🌍 |


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

`this.timezone` and `this.locale` seed the same request state the cfconfig
`runtime.*` keys use; Application.cfc overrides the server baseline, and `setTimeZone()`/`setLocale()` still override Application.cfc later in
the request. An unusable id is ignored rather than fatal, which is Lucee's verified
behaviour. Pinned in `tests/lifecycle/test_application_timezone_locale.cfm`
(12 assertions, green on both engines).

Accepted but **ignored** (no error, no effect):

| Setting | Notes |
|---|---|

| `this.applicationTimeout` | Per-app value ignored — **and so is the cfconfig `runtime.applicationTimeout`**. The key parses and is seeded into thread contexts, but nothing ever reads it: applications do not time out. |
| `this.scriptProtect` | No script-protection filtering of scopes. |
| `this.secureJSON` / `this.secureJSONPrefix` | Per-app value ignored. cfconfig `security.secureJSON*` IS applied (process-global — see §4). |
| `this.nullSupport` / `this.enableNullSupport` | Per-app value ignored — **and so is the cfconfig `runtime.nullSupport`**. The key parses and is seeded into thread contexts but has no consumer; with `"nullSupport": true` an unset variable still throws `expression` rather than returning null. |
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
| `onError` | ✅ invoked. An uncaught exception in the target page / `onRequest` / `onRequestStart` is handed to `onError(exception, eventName)` (`eventName` is `""` for a target-page error, otherwise the running event method). If `onError` returns normally it owns the response (the engine's default error page is suppressed); if absent the error surfaces as the default error page. When `onError` handles an error, `onRequestEnd` is skipped. |
| `onMissingTemplate` | ✅ invoked (serve mode). A request for a `.cfm`/`.cfc` template that doesn't exist on disk calls `onMissingTemplate(targetPage)` (`targetPage` is the web-root-relative requested path) after `onApplicationStart`/`onSessionStart`. Returning `true` (or nothing) handles the request and suppresses the default 404; returning `false` — or having no handler — falls through to the default 404. `onRequestStart`/`onRequest`/`onRequestEnd` are skipped (Adobe semantics). A throw inside the handler routes to `onError`. Non-CFML 404s (`.html`, images, directory requests) bypass the engine and never reach the handler. The cfconfig front-controller `fallback` remains available as an alternative. |
| `onAbort` | ✅ invoked on `<cfabort>` / `abort` — fired in place of `onRequestEnd`. `<cfabort showError="msg">` is a *catchable* error and is routed to `onError` instead (Adobe/Lucee parity), not `onAbort`. |
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
| `datasources[].connectionLimit` / `idleTimeout` / `timezone` | Pool tuning / per-DS timezone not applied. (`connectionTimeout` **is** applied — it reaches the pool builder.) |
| `mailServers[].timeout` | Carried but not applied during send. |
| `caches[].properties.maxObjects` / `defaultTimeout` / `evictionPolicy` | Region **defaults** not applied: a cache has no capacity bound and no eviction policy, and an entry stored with no explicit TTL never expires. A per-entry TTL — `cachePut( id, value, timespan )` — **is** honoured and does expire the entry, so this is narrower than it reads: it bites code that relies on the region's `defaultTimeout`/`maxObjects` instead of passing a TTL per put. |
| `logging.format` | Only `"text"`; other values warn and fall back. |
| `logging.loggers[].appender` | Logger name used; appender ignored. |

**`server.requestTimeout` is enforced.** It, `<cfsetting requestTimeout=N>` and
`getPageContext().setRequestTimeout()` all set the limit. An overrunning request
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
| `fileUpload()` / `fileUploadAll()` | `accept` | **Implemented** — VM-intercepted, it reads the form scope's `tempFilePath`/`clientFile`, creates the destination directory, honours `nameConflict=makeunique`, and reports the real `serverFile`/`fileWasSaved`. The remaining gap is `accept`: the MIME/extension allow-list is parsed and discarded, so an upload is never rejected on content type. |
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
attribute never reaches the runtime — no "unknown option" either.

| Tag | Survives lowering | Silently dropped |
|---|---|---|
| `<cfqueryparam>` | `value`, `cfsqltype`, `list`, `null` | `maxLength`, `scale` — precision/truncation not applied. |
| `<cfstoredproc>` | `procedure`, `datasource` | `returnCode`, `result`, `blockFactor`, `cachedWithin`; a second and subsequent `<cfprocresult>`, and `resultSet=` — only the first result set is bound. |

Both rows are blocked on the same thing: a database the reference Lucee can also
reach, so the expected precision/OUT-param behaviour can be probed rather than guessed.

<a id="30"></a>

## 30. Java shims — remaining gaps 🛑/🔇

A shim signals "this shim does not implement that method" out-of-band rather than as
`Ok(null)`, so a shim's `null` is believed. These operations do real work (StringBuilder
mutators,
`ConcurrentHashMap.replace`, `Collections.sort` on numbers, `TimeZone` offsets, `Date`
comparisons, `File.renameTo`, `Files` I/O, `Optional.orElse*`, `GregorianCalendar`
mutators, `Queue.contains`/`drainTo`, `InetAddress` resolution). What
remains:

| Shim | Status |
|---|---|
| `ConcurrentHashMap.compute` / `computeIfAbsent` / `computeIfPresent` / `merge` | 🛑 Throws. They take a remapping function, and these handlers are free functions with no VM handle, so a CFML closure cannot be invoked. Needs the VM-intercept treatment the higher-order builtins get. |
| `Queue.take()` | 🛑 Throws. It blocks until an element is available; the shim backs both `ConcurrentLinkedQueue` (no `take()` in Java) and the blocking queues (where it must block) and cannot tell them apart. Use `poll()`. |
| `ChronoUnit.X.between(a, b)` | 🛑 Throws. `ChronoUnit` constants are plain strings, so `.between()` dispatches on a String. Making it work means representing the tokens as shims, which would break code comparing them as strings. |
| `ProcessBuilder` / `Runtime.exec` | 🔇 `directory()`, `environment()`, `redirectOutput()`, `redirectErrorStream()`, `inheritIO()` are ignored; `Process.getInputStream()`/`getErrorStream()` return null so child stdout is unreadable and leaks to the engine console; `Runtime.exec` never launches. Implementing these is a new capability (process spawning with redirected stdio), not a bug fix — deliberately not done. |
| `new SimpleDateFormat(pattern)` | 🔇 The pattern argument is discarded; `.format()` emits the Java MEDIUM style (`Jan 1, 1970`) regardless. |
| `HttpServletRequest.setAttribute` / `getAttribute` / `getSession` | 🔇 Attributes are silently discarded — there is no real servlet state behind the bridge (see §11). |
| Unknown method on a **known** shim class | 🔇 Still returns null rather than throwing. The shim correctly reports "not mine" and falls through to generic dispatch — which must stay, so property access like `system.out` keeps working — but a `__java_shim` struct whose member resolves nowhere does not reach the undefined-member error a plain struct gets. Making that loud is the remaining half of the D2 work. |

Shims that **work** are not listed here — this document groups by status, so they
appear under Part D with whatever edges they have: see [§50](#50) for the AntiSamy
sanitiser's divergences from the Java library. For the full shimmed surface —
which classes and methods exist at all — see `docs/java-shims.md`.

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
| `createObject("java", "…")` for a class outside the shimmed set | Throws "Java class […] is not supported" (RustCFML has no JVM; only a curated set of `java.*` standard-library classes are shimmed). |
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

<a id="109"></a>
## 109. `a &= "X"` on a parameter under `localmode="modern"` does not write through to `arguments` 🌟 *(to fix)*

Probed on Lucee 7.1: in modern localmode a bare `a = …`, `a += 1` and `a++` on a
parameter are LOCAL writes that leave `arguments.a` alone (we match, §102), but
`a &= "X"` DOES update `arguments.a`. We lower `&=` exactly like `a = a & "X"`,
so the argument stays at its passed value. Matching needs `&=` to stop lowering
as a plain reassign (its own op or a marker) — decided 2026-09-12: match Lucee,
inconsistency included.

<a id="110"></a>
## 110. A bare write to a DELETED parameter stays frame-local; Lucee sends it to `variables` 🌟 *(deferred)*

After `structDelete( arguments, "a" )` (§106) a later bare `a = "x"` is no longer
a parameter write on either engine. In classic localmode Lucee therefore stores
it in the `variables` scope; we keep it in the frame's locals. Observable only as
`variables.a` after the call. Closing it means the classic-mode store routing
(eight `func.params` checks in the frame prologue) consulting a per-frame
"detached parameter" set — a hot-path change that needs its own A/B (see §102's
2% note), so parked.

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
| Expiry touch | throttled (skips the write until ~25% of the timeout elapses with no data change) | kills per-request write amplification; semantically invisible. Change detection hashes the record's CONTENT (`variables`, auth, timeout) and deliberately ignores `last_accessed_secs`/`created_secs` — including them made this throttle dead code until v0.629.0 (GH #361) |
| Reads per request | one `SELECT` for an established session | the VM memoises the record for the life of the request (`SessionStore::reads_are_cheap() == false`), so the start-of-request touch, the live-scope attach, every `isUserLoggedIn`/`isUserInRole`/`getAuthUser`/`sessionGetMetadata` call and the end-of-request persist all serve from one read. Before v0.629.0 a small logged-in page cost 8 `SELECT`s + 2 `UPDATE`s — ~185 ms on a remote Postgres before any CFML ran (GH #361) |
| App partition | single logical `app_name` per store | multi-app isolation via distinct datasources/tables in v1 |
| DDL denied by grants | clear error telling you to pre-create the documented schema | the store then just uses it |
| Verified driver | SQLite (bundled) end-to-end; MySQL/PostgreSQL/MSSQL portable-by-construction | MSSQL may need a manual schema (`TEXT` is deprecated there) |
| `client` scope storage | not implemented (explicit non-goal for v1) | the schema extends with a scope discriminator if ever wanted |

### 12b. Lazy session creation is the engine-wide default 🌟 *(divergence)*

No session record, no `CFID` cookie, and no `onSessionStart` fire until code
**writes** to the `session` scope. A request that only reads session (or never
touches it) mints nothing — so crawlers and `curl` hits neither persist an empty
session nor receive a tracking cookie.

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

Expiry does not ride on request handling. Two independent mechanisms:

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
Cloudflare Worker handler, so the two cannot drift. Per-application overrides via
`this.sessioncookie` are honoured on both runtimes:

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
  (`on`/`off`).

Lucee's spec default is `secure:false` everywhere, so the Worker-on default is a
**deliberate divergence** — but confined to the *unspecified* case: an explicit
`this.sessioncookie.secure = false` is honoured verbatim on both runtimes.
`SameSite=None` forces `Secure` on (browsers reject it otherwise).

<a id="13"></a>

## 13. `<cfoutput query>` / grouped output — implemented, with edges 🏗

`<cfoutput query="q">` drives row iteration. Supported: per-row looping, `startrow`/`maxrows`, bare column refs (`#name#`,
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

## 16. Sampling profiler — JIT-compiled numeric leaves not attributed ✅ RESOLVED (v0.653.0)

Small hot numeric functions the JIT compiled to native code bypassed the
interpreter loop, so they neither pushed a call frame nor fired the sampling
hook, and their time was folded into the caller's self-time.

Resolved by removing the JIT (see §77): every frame is interpreted now, so the
profiler attributes all of them.

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
and shared bytecode-cache entry by canonical path identity. Only rustcfml's
own writes trigger the flush; external edits still defer to the next request as above.

**Production mode behaves the same way**: it never re-stats (immutable tree, restart to
reload), but rustcfml's *own* mid-request write does flush, exactly as in dev. The
immutable-tree contract covers *external* edits; a template this process just rewrote is
not one.

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

<a id="51"></a>

## 51. Tag-mode parsing: two constructs compile here that Lucee rejects 🏗

### 51a. Bracketed comparisons in tag mode

A tag ends at the first `>` that is not inside a string, a CFML comment, or a
bracketed sub-expression. Lucee has no such bracket clause: in tag mode a bare
`>` terminates the construct unconditionally, in an attribute *and* inside
`#…#`, so a parenthesised comparison is a **compile error** there.

| tag-mode source | Lucee 7.0.4 | RustCFML |
|---|---|---|
| `<cfset big = a GT b>` | `true` | `true` |
| `<cfset big = a > b>` | tag ends at `>`; `big = a`, ` b>` emitted as text | identical |
| `<cfif a > b>YES</cfif>` | tag ends at `>`; ` b>` emitted, then `YES` | identical |
| `<cfif cond>=5</cfif>` | outputs `=5` | identical |
| `<cfset f = (x) => x * 2>` | ok | ok |
| `<cfset t = arr.reduce((s, r) => s + r, 0)>` | ok | ok |
| `<cfset big = (a > b)>` | **compile error** — `Invalid Syntax Closing [)] not found` | `true` |
| `#(a > b)#` in a tag-mode body | **compile error**, same message | `true` |

So everything about the bare-`>` rule is shared with Lucee — including the
`<cfif cond>=5</cfif>` resolution, where the tag still ends at `>` and `=5` is
body text. Only the bracket clause is ours. The scanner ignores a `>` while any
`(`/`[` is open, and ignores a literal `=>` anywhere (guarded so `>=`, `==>`,
`!=>` and `<=>` are unaffected); the arrow handling matches Lucee, the
bracketing does not.

The consequence is one-directional and benign, like §48: tag-mode source written
for Lucee always compiles here, but source written here may not compile on
Lucee. **Write `<cfset big = a GT b>`** — the word operator is the only spelling
both engines accept. Do not reach for brackets to disambiguate a tag-mode `>`;
that is RustCFML-only.

### 51b. `#`, `"` or `'` inside an *unquoted* attribute value

An unquoted tag attribute value is a literal string on both engines, terminated
by whitespace, `>` or `/>` — `<cfparam name="z" default=a.b>` yields the three
characters `a.b`, not a read of `a.b`. Lucee then makes a `#`, `"` or `'`
*inside* such a value a hard compile error (`Simple attribute value can't
contain [#]`); we accept it, interpolating `#…#` and quoting the rest:

| unquoted attribute | Lucee 7.0.4 | RustCFML |
|---|---|---|
| `<cfparam name="z" default=a.b>` | `z = "a.b"` | `z = "a.b"` |
| `<cfparam name="z" default=http://x/?a=>` | `z = "http://x/?a="` | same |
| `<cfparam name="z" default=#a.b#>` | one whole `#…#` is an expression | same |
| `<cfparam name="z" default=x#a.b#y>` | **compile error** | interpolates → `xSURPRISEy` |
| `<cfparam name="z" default=len('ab')>` | **compile error** | literal `len('ab')` |

Same one-directional shape as 51a: source written for Lucee always compiles
here, source written here may not compile there. **Quote any attribute value
that contains a `#` or a quote** — `default="x#a.b#y"` — which is portable.

Covered by `tests/tags/test_tag_unquoted_attr_literal.cfm`, which is green on
both engines.

Measured against Lucee 7.0.4.34 (2026-08-18).

<a id="41"></a>

## 41. The `application` scope is live in-process, but NOT across cluster nodes 🏗 *(divergence, by construction)*

The `application` scope is a genuinely shared live structure: a write
by one request is immediately visible to every other in-flight request on that
node, matching Lucee. Guard-once idioms
(`if ( !StructKeyExists( application, "x" ) ) { expensive(); … }`) are therefore
safe **within a process**.

They are **not** safe across nodes when a serialising application store is in use —
Cloudflare KV, the Durable Object store, or any future cluster backend. Those hold
a snapshot per node and publish it at request end (`ApplicationStore::publish_variables`),
so two nodes can both observe a key as absent and both run the expensive branch,
and one node's write can overwrite another's.

There is no way to paper over this at the trait level: a live shared scope needs
shared memory, which distinct isolates/nodes do not have. If you need
exactly-once initialisation across nodes, use an external mutex (a database row,
a Durable Object, a distributed lock) rather than an application-scope key.
Note the Durable Object backend serialises all requests through a single instance,
so it is closer to correct than plain KV, but still snapshot-based.

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

<a id="46"></a>

## 46. Member-function dispatch lowercases the method name per call 🏗

`obj.method()` dispatch normalises the method name with `to_lowercase()` on the
call path (`crates/cfml-vm/src/lib.rs:24597`).

The remaining allocation is small individually but the call counts are not. One
profiled Preside admin request showed `RequestService.getContext()` invoked 2,920
times and `RequestContextDecorator` methods 4,278 times — thousands of
`String` allocations per request purely to normalise a name for comparison.

⚠️ **Measure before changing this.** The obvious fix — borrow with `Cow` when the
name is already lowercase — was implemented and measured on the *bare-name*
resolution path during the v0.596.0 work and was **slower**: the callee compares
the normalised name against ~30 string literals, and routing each comparison
through the `Cow` discriminant cost more than the allocation it saved
(registry-cased `len` 186→210 ms/1M, `structKeyExists` 262→290 ms/1M). It was
reverted, and a comment left at the site. The same trap may well apply here.

<a id="47"></a>

## 47. Surplus built-in function arguments are accepted, not rejected 🏗

RustCFML tolerates extra positional arguments to a built-in where Lucee raises a
**compile-time** error:

| call | Lucee 7.0.4 | RustCFML |
|---|---|---|
| `duplicate( x, false, "extra" )` | compile error, "Too many arguments (2:3)" | accepted, extra ignored |
| `struct.copy( false )` | error, "too many arguments for function [structcopy] call" | accepted, arg ignored |

This is general BIF arity tolerance rather than anything specific to those two
functions — making arity strict across ~730 built-ins is a
separate semantics project with a much wider blast radius, and a real risk of
breaking working user code that happens to pass a stray argument.

<a id="48"></a>

## 48. The Elvis operator accepts any left operand, where Lucee restricts it 🏗

Lucee constrains the left operand of `?:` **grammatically, at compile time**:

```
lucee.runtime.exp.TemplateException: left operand of the Elvis operator
has to be a variable or a function call
```

| expression | Lucee 7.0.4 | RustCFML |
|---|---|---|
| `noSuchVar ?: "d"` | ok | ok |
| `someFn() ?: "d"` | ok | ok |
| `( 1 / 0 ) ?: "d"` | **compile error** | accepted, evaluates the operand |

Measured while fixing the elvis *error scope* divergence (GH
[#329](https://github.com/RustCFML/RustCFML/issues/329), v0.597.0), where Lucee's
`?:` was found to absorb any exception raised by its left operand rather than
only an undefined read. The runtime behaviour was brought to parity; this
compile-time grammar restriction was deliberately **not** adopted, because
adopting it would reject source we currently compile and run, with no
correctness benefit to code that already works.

The practical consequence is one-directional and benign: source written for
Lucee always compiles here, but source written here may not compile on Lucee.
Anything relying on an arithmetic or comparison expression to the left of `?:`
is RustCFML-only.

Covered from the runtime side by `tests/functions/test_elvis_error_scope.cfm`,
which deliberately avoids the restricted shape so the suite compiles on both
engines.

<a id="49"></a>

## 49. `fileOpen( f, "write" )` does not create the file 🏗

Lucee's `fileOpen()` in a write mode creates the target immediately, so
`fileExists()` is true before anything is written and the handle can be closed
without ever producing content. RustCFML defers creation until the first
`fileWrite()` on the handle, so:

```cfml
h = fileOpen( f, "write" );
fileClose( h );
fileExists( f );   // Lucee: true.  RustCFML: false — nothing was created
```

Confirmed to be genuine absence rather than a stale cached answer: after the
sequence above the path is invisible to `directoryList()` (a different code path
from the existence memo) and absent on disk. `fileClose()` is itself a no-op stub
in RustCFML, which is why nothing flushes a zero-byte file on close.

The existence cache is unaffected either way, since it never caches an answer the
filesystem does not agree with. Fixing it means giving handles a real
create-on-open, which also wants `fileClose()` to stop being a stub.

<a id="50"></a>

## 50. AntiSamy sanitiser: cosmetic divergences from the Java library 🏗

`org.owasp.validator.html.AntiSamy` and `sanitizeHtml()` run a native Rust
sanitiser (`crates/cfml-sanitize`) rather than the Java library. The shimmed
surface is listed in `docs/java-shims.md`; the remaining gaps in other shims are
[§30](#30). Output was
diffed against the real **AntiSamy 1.5.3 jar on Lucee 7.0.4** across 77 inputs ×
the six shipped 1.4.4 policies (preside, tinymce, slashdot, ebay, myspace,
anythinggoes) — 462 comparisons.

**No security-relevant divergence remains**: every input where the two engines
disagree produces output that is equally inert, and no OWASP filter-evasion
vector survives on either engine. The differences that do remain are cosmetic:

| Divergence | Cause |
|---|---|
| Output is not pretty-printed | The policies' `formatOutput=true` makes AntiSamy re-indent with newlines; we emit the markup as parsed |
| No `<!DOCTYPE …>` is emitted | `omitDoctypeDeclaration=false` (tinymce et al.) makes AntiSamy prepend an XHTML doctype to a *fragment*; we never do |
| `<style>` CSS is not reformatted | AntiSamy re-serialises through batik (`p {\n\tcolor: red;\n}`); we keep the declarations as written. Both filter identically — only the layout differs |
| An emptied `<style>` reads `<![CDATA[/* */]]>` vs AntiSamy's `p {\n}` | Same cause: we drop the emptied rule, AntiSamy keeps the empty block |
| `<scr<script>ipt>alert(1)</script>` leaves the text `ipt&gt;alert(1)` | html5ever and neko disagree about where the malformed tag ends. Both remove the script; we keep the leftover text, AntiSamy discards it |

Attribute **order** used to diverge too — the parser alphabetised attributes,
so `<img src alt>` came back as `<img alt src>`. It no longer does: source order
is preserved (scraper's `deterministic` feature), which also keeps a
parse/serialise round trip through `HtmlDocument()` from rewriting the caller's
markup.

Two behaviours are deliberate and will not change:

- **At-rules inside `<style>` are dropped wholesale.** `@import` must never be
  honoured (`embedStyleSheets=false`, and a sanitiser that fetched remote
  stylesheets would be a request-forgery primitive), and `@media`/`@supports`
  nest further rule blocks that would need a full stylesheet parser to filter
  safely. This loses some legitimate styling.
- **`CleanResults.getErrorMessages()`/`getNumberOfErrors()` throw** rather than
  reporting zero. We do not track per-change messages, and answering "no errors"
  would falsely imply nothing was removed.

Also worth knowing: `<tags-to-encode>` tags are **unwrapped, not encoded** —
`<g>x</g>` becomes `x`. That reads backwards against the section name, but it is
what the 1.5.3 jar does (measured across `<g>a</g>b`, `x<g>y`, `<g/>` and a
nested case), and matching the library beats matching the label.

<a id="53"></a>

## 53. `private`/`package` methods are gated on CALLS, not on member reads 🏗

Access modifiers are enforced on component-method dispatch (GH
[#330](https://github.com/RustCFML/RustCFML/issues/330)): `obj.priv()`,
`obj["priv"]()`, `invoke( obj, "priv" )` and `<cfinvoke>` all report a
`private`/`package` method as **absent** from outside the class, exactly as Lucee
does — including the same fall-through to `onMissingMethod`. `tests/oop/test_method_access_gate.cfm`
pins 26 scenarios that pass on both engines.

What is *not* gated is reading the method as a **value**:

| | Lucee 7.0.4 | RustCFML |
|---|---|---|
| `obj.priv()` from outside | throws "has no function with name [priv]" | throws (same shape) |
| `f = obj.priv` from outside | throws "has no accessible Member with name [PRIV]" | returns the function |

So an external caller can still reach a private method by extracting the
reference first (`f = obj.priv; f()`). Closing that means gating member reads
(`GetProperty`/`GetIndex` and the other `Instance::get_member` callers), which are
the hottest ops in the engine and have no caller context threaded to them today —
a separate change with a much wider blast radius than the dispatch gate.
Treated as a follow-up rather than folded into the dispatch fix.

Also worth knowing: the refusal is raised as error type `Runtime`, where Lucee
uses `expression`. That is the type of every "no such method" error in this
engine, not something specific to the access gate.

---

<a id="54"></a>

## 54. Codec divergences: malformed input is tolerated 🏗

RustCFML's base64/hex decoders accept malformed input and do something
reasonable with it; Lucee rejects it. Measured against **Lucee 7.1.0.204**:

| Expression | Lucee | RustCFML |
|---|---|---|
| `binaryDecode( "DEADBEE", "hex" )` (odd length) | throws `lucee.runtime.coder.CoderException` | drops the trailing nibble → 3 bytes |
| `binaryDecode( "DEADBEZZ", "hex" )` (non-hex char) | throws `lucee.runtime.coder.CoderException` | decodes the bad char as `0` → `DEADBE00` |
| `toBinary( "QU*D" )` (non-alphabet char) | 2 bytes (`4140`) | 3 bytes (`414003`) |

These rows predate the v0.611.0 codec rewrite (a pure speed change — `toBinary`
went from a linear alphabet scan to a 256-entry reverse table, ~7.9x faster on a
28KB blob — which preserved the tolerant behaviour deliberately so no app's
output moved).

Everything the two engines *do* agree on is pinned by
`tests/stdlib/test_base64_hex_codec.cfm`, which passes on both. The rows above
are deliberately **not** asserted there: writing either answer into the suite
would freeze one engine's behaviour as correct before the call is made.

Note for whoever picks this up: making the decoders throw is a behaviour change,
not a bug fix, for any app relying on the tolerance — `binaryDecode` currently
never throws on content.

*(The URL-encoder half of this section was resolved in GH
[#336](https://github.com/RustCFML/RustCFML/issues/336): `urlEncode`,
`urlEncodedFormat` and `encodeForURL` now carry three distinct character
policies, verified character-by-character against Lucee and pinned by
`tests/stdlib/test_url_encoder_policies.cfm`.)*

---

## 55. Presigned S3 URLs spell a key's own leading slash differently 🏗

An object key that itself begins with a slash — what `objectName="//a/b.txt"`
addresses, since exactly one leading slash is stripped — is spelled differently
in the signed path by the two engines:

| | signed path |
|---|---|
| Lucee 7 (S3 extension) | `/%2Fa/b.txt` |
| RustCFML | `//a/b.txt` |

Both URL-decode to the same key (`/a/b.txt`) and both are valid signed URLs, so
they fetch the same object; only the byte shape differs. RustCFML builds and
signs the canonical URI through the AWS SDK, which treats the key's leading
slash as a path character; matching Lucee byte-for-byte would mean hand-rolling
SigV4 presigning instead. Everything else about the URL — virtual-host
addressing, key normalisation, `httpMethod`, and the `X-Amz-Expires` window —
matches, and is pinned by `tests/s3/test_s3_presigned_url_lucee_compat.cfm`.

## 56. XML: a named child is an ARRAY, where Lucee reports a single node 🏗

`x.Root.Kid` is an array of every `<Kid>` child here. Lucee returns one object
(`XMLMultiElementStruct`) that wraps the same list and delegates member reads to
the first element, so it reports as a struct while still indexing like an array.

Member reads and indexing now agree on both engines — `x.Root.Kid.xmlText`,
`x.Root.Kid[2].xmlText` and `arrayLen( x.Root.Kid )` all return the same thing
(GH [#343](https://github.com/RustCFML/RustCFML/issues/343)). What still differs
is the value's *type identity* and its key list:

| | Lucee 7 | RustCFML |
|---|---|---|
| `isArray( x.Root.Kid )` | `false` | `true` |
| `isStruct( x.Root.Kid )` | `true` | `false` |
| `structKeyList( x.Root )` | `Kid,Kid,Solo` (one entry per child) | `xmlName,xmlType,xmlText,xmlChildren,xmlAttributes,Kid` |

So the reserved node properties are real keys here and virtual on Lucee, and
`for ( k in node )` iterates them before reaching the child names. Closing this
means giving XML nodes their own value type rather than modelling them as plain
structs; the read paths that matter behave the same in the meantime.

---

## 57. `structGet()` cannot create a path rooted in `local` or `arguments` 🏗

`structGet( p )` resolves `p` and, when it does not exist, creates it and returns
the new struct (Lucee's `StructGet`, GH
[#346](https://github.com/RustCFML/RustCFML/issues/346)). Creation writes into
the scope the read chain would look in — `variables`, `request`, `application`,
`server`, `session` or the component's `variables`.

Two inputs throw here instead of creating:

  * a path rooted in the CALLING frame's `local` or `arguments` scope, which a
    builtin cannot write to from inside a function (the same limit runtime
    `param` has, and it reports the same way rather than creating the path in
    some other scope);
  * a path with an array subscript (`structGet( "a[3].b" )`), since creating it
    would have to invent elements 1..2 as well.

Resolution of both forms is unaffected — only creation-on-miss is. Erroring is
deliberate: returning a detached struct is exactly how the original no-op stayed
invisible for 300 releases.

---

## 59. `querySort()` infers numeric-vs-text ordering from the values, not a declared column type 🏗

Lucee decides whether a query column sorts numerically or as text from the
column's declared SQL type. We store no column types — `queryNew`'s type argument
is accepted and ignored — so `querySort` infers it: a column sorts numerically
when every non-empty cell parses as a number, and as text otherwise.

The two rules agree everywhere except one case — a column declared as a string
type that happens to hold only numeric strings:

```cfml
q = queryNew( "a", "varchar", [ [ "10" ], [ "9" ], [ "100" ] ] );
querySort( q, "a" );
valueList( q.a )   // Lucee: 10,100,9   RustCFML: 9,10,100
```

An untyped column of the same values sorts `9,10,100` on both engines, as does a
column with any non-numeric value in it (`["10","b","2"]` → `10,2,b` on both).
Everything else about the sort matches Lucee 7.1.0.204: stability, empty/null
first ascending, case-sensitive text order, multi-column tie-breaks, and the
three `database`-typed error messages.

Closing this means storing column types on `CfmlQueryData` — worth doing for
`getMetaData()` too, which currently infers `typeName` the same way — but it is a
schema change across `queryNew`, the DB result-set builders and QoQ, so it is
tracked rather than bundled into GH
[#345](https://github.com/RustCFML/RustCFML/issues/345).

## 60. `throw( object=e, … )` merges the explicit attributes; Lucee discards the object 🏗 *(GH [#352](https://github.com/RustCFML/RustCFML/issues/352))*

**Deliberate divergence.** Measured against Lucee 7.1.0.204.

```cfml
try { throw( type="Custom.T2", message="m2", detail="d2" ); } catch ( any e ) { orig = e; }
```

| call | Lucee 7.1.0.204 | RustCFML |
|---|---|---|
| `throw( object=orig )` | `Custom.T2` / `m2` / `d2` | same — agrees |
| `throw( object=orig, message="overridden" )` | `application` / `overridden` / *(empty)* | `Custom.T2` / `overridden` / `d2` |
| `throw( object=orig, type="New.T" )` | `Custom.T2` / `m2` — the `type=` is ignored | `New.T` / `m2` |

We **merge**: the object supplies the base and any explicit attribute overrides
it. Lucee gives `message` outright precedence over `object`, and `object`
outright precedence over `type`/`detail`.

Lucee's behaviour is a deliberate ordering, not an accident — `Throw.java`'s
`doStartTag()` runs `_doStartTag(message)` *before* `_doStartTag(object)`, and
the first non-empty one throws a **fresh** `CustomTypeException` built from the
tag's own attributes, whose `type` defaults to `"application"`. The caught
exception's type and detail are simply never consulted.

Copying it was considered and rejected: silently discarding a caught exception's
type and detail because the caller also wanted to reword the message loses
information for no stated benefit, and no code has been found that depends on
the reset. Our merge is a superset — `throw( object=e )` alone is identical on
both engines, so the only programs affected are those that pass an object *and*
an override, which on Lucee cannot be doing anything deliberate with the object.

`tests/tags/test_throw_object_rootcause.cfm` guards the three overriding
assertions with `isRustCFML()` so the cross-engine run stays green while the
shared behaviour keeps its cross-engine value.

## 61. A binary participates in the READ-ONLY array BIFs only 🏗 *(GH [#340](https://github.com/RustCFML/RustCFML/issues/340))*

A `Binary` is a Java `byte[]` on Lucee, so the array BIFs operate on it and its
elements are signed bytes. That now holds here for every **read**: `arrayLen`,
`isArray`, `b[1]`, `arrayToList`/`Slice`/`Mid`/`Find`/`Reverse`/`First`/`Last`/
`Min`/`Max`/`Sum`/`Avg`/`ToStruct`/`Merge`/`IsEmpty`/`IsDefined`/`IndexExists`,
`for ( x in b )`, and `arrayMap`/`Filter`/`Reduce`/`Each` — all measured against
Lucee 7.1.0.204, with `0xFF` reading back as `-1` on both.

Two **write** behaviours still diverge, because the conversion produces a copy
rather than a view onto the binary's bytes:

| | Lucee 7.1.0.204 | RustCFML |
|---|---|---|
| `b[1] = 99` | mutates the byte in place; `b` stays a 3-byte binary | the write is dropped; `b` is unchanged |
| `arrayAppend( b, 68 )` | no-op — a `byte[]` is fixed size, so `b` stays a 3-byte binary | `b` becomes a 1-element ARRAY |

The mutating array BIFs are deliberately excluded from the conversion for the
second reason: coercing there would silently turn a binary into a real array,
which is further from Lucee than leaving them alone. Closing this properly means
a byte-backed array view rather than a per-call copy.

Two neighbouring divergences found while measuring this were NOT specific to
binaries and were tracked separately as GH #358 and GH #359 — both **fixed in
v0.630.0**: `arrayContains`/`arrayContainsNoCase` now return the 1-based index
like Lucee, and `serializeJSON( binary )` now yields the base64 string.

## 62. `pageEncoding` is accepted and ignored 🏗

`<cfprocessingdirective pageEncoding="…">` — and its script forms
`cfprocessingdirective( pageEncoding=… )` / `processingdirective pageEncoding=…;`
— parse and run, but the attribute has no effect: the engine reads every source
file as UTF-8. `suppressWhiteSpace` IS honoured in both the tag and the script
forms.

This is not new behaviour; it is recorded here because GH #357 added the script
spellings, and the bare statement form previously parsed as an identifier plus an
assignment — doing nothing *and* leaving a stray `pageencoding` page variable
behind. It is now a clean no-op that matches the tag.

## 63. `new Mail()` / `<cfmail>`: `async` is ignored — every send is synchronous 🏗 *(GH [#356](https://github.com/RustCFML/RustCFML/issues/356))*

Lucee spools a message when `async`/`spoolEnable` is set and delivers it from a
background task, so the request returns before the SMTP dialogue happens. Here
the attribute is accepted and ignored: `send()` always talks to the server
inline and the request waits for it. A slow or unreachable SMTP server therefore
shows up as request latency rather than as a queued message.

The rest of the surface the engine-bundled `Mail` shim exposes IS wired through:
`to`/`cc`/`bcc`/`replyTo` (comma- **or** semicolon-delimited), `failTo` as a
`Return-Path`, `addParam( name=, value= )` as a custom header,
`addParam( file=, remove= )` as an attachment deleted after a successful send,
`addPart` as a `multipart/alternative`, and `useSSL`/`useTLS`.

## 64. Lucee's OSGi bundle plumbing is inert 🏗

`lucee.runtime.osgi.OSGiUtil` and `lucee.loader.engine.CFMLEngineFactory` are
shimmed so that a CFML library which ships its own jars can complete its
bundle-loading ceremony. Nothing is loaded: there is no JVM, no OSGi container
and no jar to install.

* `getBundleLoaded()` reports **every** bundle as already present, so callers
  take their "nothing to do" path instead of building a `Resource` for a jar
  that will not be read. `installBundle()` accepts and does nothing.
* This is deliberately **not** a claim that the bundle's classes exist. The
  `createObject( "java", className, bundleName, version )` that follows the
  ceremony is answered on its own merits — natively if the engine models the
  class (see §65), and otherwise with the usual "Java class […] is not
  supported" error, naming the class.

Without the shim, the `init()` of any such library is a hard error, so the only
reachable states were "throws at construction" and "reaches the class request".

## 65. Apache POI is an adapter over the native spreadsheet engine, not POI 🏗

Libraries that drive POI's object graph directly — `lucee-spreadsheet`
(`spreadsheetCFML`), which Preside vendors as `spreadsheetLib` — run against an
adapter that maps that graph onto the `Spreadsheet*` builtins. A `Sheet` is a
workbook plus a sheet index, a `Row` adds a row, a `Cell` adds a column, and
each mutation is the matching builtin. POI's 0-based indexing is converted at
that boundary.

Where the two models genuinely differ:

| POI | Here |
|---|---|
| `new XSSFWorkbook()` has no sheets | The engine always has one, so the adapter keeps POI's view of the sheet list and **reuses** that sheet on the first `createSheet()`. Later ones add normally. |
| `CellStyle`/`Font` are configure-then-assign, and a style is a live workbook object | A style is an **accumulator**; `setCellStyle()`/`setRowStyle()` is where the formatting is applied. Mutating a style *after* assigning it does not retroactively restyle the cells it already touched. |
| `setFont()` replaces the style's font | It **merges**, which is what makes the library's clone-the-current-font-then-modify-it idiom compose correctly without a shared font table. |
| Fonts have defaults (Calibri, 11pt, black) | An **unset** font property reads as `null`, and every setter ignores a `null`. Answering POI's defaults would stamp them onto every cell a cloned-from-empty style touched. |
| `Font.setCharSet()` / `setTypeOffset()` | Accepted and ignored — neither the format struct nor the engine models them, and refusing would break a `cloneFont()` that never set them. |
| `Workbook.write( OutputStream )` streams anywhere | Requires a **file-backed** stream (`java.io.FileOutputStream`); the adapter writes through the engine, which needs a path. Anything else throws `java.io.IOException`. |
| `new HSSFWorkbook()` writes legacy binary `.xls` | The engine READS `.xls` but cannot write it, so the workbook is **backed by xlsx and written as xlsx** — under whatever filename the caller chose, `.xls` included. Every such write prints a `[POI]` warning to stderr naming the file. `getClass()` still reports `HSSFWorkbook`/`HSSFCellStyle`, because libraries branch on that to pick their style and colour classes and those branches must stay self-consistent; only the bytes differ. **This is a deliberate format substitution** — spreadsheet applications sniff content and open it, a strict `.xls` consumer will not. It exists so Preside's form-builder export (which asks for `.xls`) keeps working until that is changed upstream. |
| `Row.cellIterator()` yields physically-created cells | Approximated by "has a value", the only distinction the engine records. |

The substitution is **write-side only**: `SpreadsheetRead()` still picks its
reader from the file extension, so reading such a file back through its `.xls`
name does not work. Callers that need the round trip should name the file
`.xlsx`.

Anything outside the adapter's reach **throws, naming the class and the method**
rather than returning a plausible default — a spreadsheet that silently loses a
column is worse than one that fails.

## 66. jsoup is an adapter over `HtmlDocument()` 🏗

`org.jsoup.*` maps onto the `HtmlDocument()` builtin's mutable DOM. An `Element`
is the shared document handle plus an integer node handle, so a mutation through
one element is visible through every other and in the document's output — which
is what the mutate-then-serialise callers (email click-tracking, CSS inlining)
need.

| jsoup | Here |
|---|---|
| `Document.OutputSettings` — charset, pretty-print, escape mode, indent | **Accepted and ignored**, fluently, so a caller's chain still runs. The serialiser has none of those knobs: it emits the document as parsed, in UTF-8. |
| `Jsoup.clean( html, whitelist )` | **Refused.** It is a sanitiser with jsoup's Whitelist policy model, which has no equivalent here; quietly running AntiSamy instead would apply rules the caller never asked for. Use `sanitizeHtml( html, policyPath )`. |
| `Element.toString()` on a *handle* coerced to a string | The element's outer HTML **as at selection time**. Every live read goes through a method (`toString()`, `html()`, `attr()`), which re-reads the document; only string coercion of the handle itself sees the snapshot. |
| `Elements` | A plain CFML array, so indexing, `ArrayLen` and `for…in` behave as they do for a `java.util.List` on Lucee. |

`Element.hashCode()` is the node handle — stable for one element, distinct
between two, which is what callers grouping by it require.

**An orphan table cell loses its tag.** `HtmlDocument( "<td>x</td>" )` yields
`x`, because a `<td>` outside a table has no valid insertion point under the HTML
parsing algorithm — a browser does the same with `innerHTML`. Wrap such
fragments (`<table><tbody><tr>…`) before parsing, as Preside's
`EmailStyleInliner` already does.

## 67. QRGen and Batik are adapters over `qrCodeGenerate()` / `imageReadSvg()` 🏗

| Behaviour | Note |
|---|---|
| `QRCode.…withSize( w, h )` with `w != h` | A QR symbol is square. The **smaller** edge is used, so the code stays inside the box the caller reserved for it, rather than being stretched into something a scanner may reject. |
| `QRCode.…stream()` | Returns a stream whose `toByteArray()` is a **Binary**, not the signed-byte array `java.io.ByteArrayOutputStream`'s shim yields. Both answers are needed elsewhere — the array form is what `String.getBytes()` and the TOTP path rely on — so QRGen's stream is its own type instead of either being made wrong. |
| `QRCode.…file()` | **Refused**: it writes to a JVM temp `File`. Use `.stream().toByteArray()` and `fileWrite()`. |
| `QRCode.…withCharset()` / `withHint()` | Accepted and ignored. The encoder emits UTF-8, which is QRGen's own default and what scanners expect. |
| `PNGTranscoder` / `JPEGTranscoder` / `TIFFTranscoder` | Supported. `KEY_WIDTH`/`KEY_HEIGHT` (and the `KEY_MAX_*` forms, treated as a target when no exact size is set) are honoured; other hints are recorded and ignored. |
| `transcode()` | **File to file.** `TranscoderInput` takes a `file:` URI or path, `TranscoderOutput` must wrap a `java.io.FileOutputStream` — the adapter goes through the image builtins, which need paths. A non-file output raises `TranscoderException`. |

## 68. PDF reading and page rasterisation 🏗

`PdfRead` / `Pdf` / `PdfInfo` / `PdfPageCount` / `PdfToImage`, and the
`org.apache.pdfbox.*` adapter over them, are backed by
[`hayro`](https://github.com/LaurenzV/hayro) — pure Rust, no C.

| Behaviour | Note |
|---|---|
| Reading only | Pages can be read and rasterised. **Authoring, merging, splitting and form-filling are not supported** and say so; `PDDocument.save()` throws rather than writing an empty file. |
| The `resolution` argument | PDFBox's `PDFImageWriter.writeImage(…, resolution)` is DPI, and the adapter honours it as DPI. Callers that pass a *pixel width* there (Preside does, then resizes) get a higher-resolution render that downsamples to a better thumbnail. |
| Page indices | The `Pdf*` builtins are **1-based**, like the rest of CFML. PDFBox's `PDFRenderer` is **0-based**, and the adapter converts. |
| Output size | Capped at 40 megapixels regardless of what the page declares or the caller asks for — thumbnailing usually means rendering files the public uploaded. Over the cap the scale is reduced to fit rather than the call failing. |
| Unsupported by hayro | Knockout groups, and PDFs whose CID fonts are not embedded. |

Native-only (the `pdf` cargo feature), like `svg` and `spreadsheet`.

`imageReadSvg()` is **native-only** (the `svg` cargo feature, absent from the
wasm builds), because rendering text in an SVG needs real fonts and the only
honest source is the operating system's font database. Dropping text support to
gain a wasm build would render any SVG containing text as silently missing
content, which is worse than the BIF not being there.

Sizing follows the vector-graphics convention rather than the raster one: with
one dimension given the other follows the aspect ratio, and with both the art is
scaled to **fit inside** that box and centred. Stretching vector art to an
arbitrary rectangle is almost never what "make it 200x100" means.

---

## 69. `loop file=` streams — RESOLVED in v0.640.0 *(GH [#367](https://github.com/RustCFML/RustCFML/issues/367))*

Kept as a pointer because the entry above it shipped in two halves and the
first half read like the whole fix.

Both spellings iterate a file line by line:

```cfml
<cfloop file="#p#" index="line"> ... </cfloop>     <!--- tag form --->
loop file=p item="line" { ... }                    // script form (added for GH #367)
```

v0.636.0 made the script form *work* — it previously fell through to the
infinite-loop fallback and threw "Variable 'line' is undefined". It did not
make either form bound its memory: `__cfloop_file_lines` read the file whole
and materialised an array of every line before the first iteration, so peak
cost was the file size plus one `String` per line — *worse* than the
`fileRead()` + `listToArray()` workaround the construct is meant to replace.

v0.640.0 closes that half. `Vfs::open_lines` returns a `VfsLines` cursor
(`RealFs` buffers over the open file; in-memory implementations keep the eager
read, which is optimal for them and forwarded by the delegating ones), and both
lowerings converge on a pump — open, `next` until it yields Null, close — in
place of `for (x in <array>)`. Measured on a 214MB / 4M-row CSV: peak RSS 1038MB
eager → 241MB streaming, which is the engine's own baseline footprint; peak is
flat from a 12MB file to a 214MB one. CPU is unchanged to slightly better.

Two properties worth knowing rather than rediscovering:

- **The cursor holds an OS file descriptor, so every exit path must close it.**
  The loop is lowered as `try { while(true){…} } finally { close }` rather than
  as hand-emitted bytecode precisely for this: the body can leave by running
  out, by `break`, by `return`, or by throwing. A first cut covered the first
  three and leaked on the fourth, which is not academic — a function that
  returns from inside the loop leaks one descriptor per call, so a few thousand
  calls in one request exhaust a default 1024-fd limit and fail with EMFILE far
  from the cause. Regression cover: 40,000 early exits (returns and throws)
  under a deliberately-set 256-fd limit.
- **Undecodable bytes become U+FFFD, one per bad byte, rather than ending the
  loop.** That is Lucee's lenient decoder, and it is what makes a
  mis-declared file degrade instead of aborting. (Until v0.651.0 the streaming
  reader used `BufRead::lines`, which *errored* on invalid UTF-8 — so a
  Latin-1 file with no `charset=` threw "stream did not contain valid UTF-8"
  where Lucee returned text.)

### The line window (`startLine`/`endLine`, a.k.a. `from`/`to`)

Reading a file's header to validate it is the case the streaming loop still
handled badly: the only way to stop early was `break`, and stopping early is
the whole point. Both bounds are honoured as of v0.651.0, in both spellings —
`startLine`/`endLine` are Lucee's documented attribute names and `from`/`to`
are aliases, with the documented pair winning if both appear:

```cfml
loop file=p item="line" from=1 to=1 { validateHeader( line ); }   // reads ONE line
<cfloop file="#p#" index="line" startline="2" endline="10"> ... </cfloop>
```

Behaviour matches Lucee 7.1 on every edge the alias pair has (all asserted
cross-engine in `tests/tags/test_loop_file_lines.cfm`): a start below 1 clamps
to line 1, a start past EOF or an `endLine` below the start runs the body no
times rather than erroring, a fractional bound truncates, and a non-numeric one
throws an `expression` error instead of reading as 0 — which would look like an
empty file. The window is enforced on the cursor, so `to=1` on a 214MB file
still reads one line; slicing a materialised array would have handed back the
residency the cursor exists to avoid.

Note the trap the guard exists for: `from`/`to` **with** `index` is also the
shape of the counted index loop, and that lowering ignores `file` entirely. Both
lowerings therefore check `file=` first — before the guard,
`<cfloop file="f" index="l" from="3" to="5">` bound `l` to the numbers 3, 4, 5
and never opened the file.

### `characters=` and `charset=`

Both were accepted and **ignored** before v0.651.0 — a `characters=` loop
yielded whole lines, and a file in a single-byte encoding came back with
replacement characters (or threw, see above). Both now work, streaming, and
both compose with the window:

```cfml
loop file=p item="chunk" characters=8192 { … }              // 8192-char chunks
loop file=p item="line" charset="iso-8859-1" { … }          // decoded, per line
<cfloop file="#p#" index="c" charset="utf-16" characters="6"> … </cfloop>
```

`characters=N` yields exactly N **characters** — not bytes, so a multi-byte
character counts once — with line terminators included verbatim and the last
chunk holding the remainder. `charset=` takes the same names as
`fileRead`/`fileWrite` (`charset::resolve`), and a byte-order mark still wins
over the declared charset, as it does everywhere else in the engine.

**The window counts iterations, so with `characters=` it counts chunks**, not
lines: `characters=7 from=3 to=4` yields the 3rd and 4th 7-character chunk.
Probed against Lucee 7.1, which does the same.

Two deliberate divergences, both trading a Lucee defect for an error:
`characters=0` loops **forever** there, yielding `""` until the request is
killed, and a negative value throws a raw
`java.lang.NegativeArraySizeException`. Here an unusable chunk size is
rejected with an `expression` error. (The cross-engine test for `characters=0`
is guarded `isRustCFML()` — running it on Lucee hangs the suite.)

Implementation note: decoding is incremental (`charset::StreamDecoder`, a
Rust unit test asserts it is byte-for-byte equivalent to `charset::decode` of
the whole file at every block size), and consumed text is dropped once per
16KB refill rather than per chunk. Both matter: the naive versions of each
cost the million-line loop ~10% of its wall clock, and a "streaming" reader
that is 20% slower than the array walk it replaces is a poor trade. Measured
on an 84MB / 2M-row CSV: line loop 985ms vs 997ms for the pre-charset reader,
`characters=8192` 368ms, peak RSS flat at the engine's baseline, and
`from=1 to=1` returns in 0ms.

## 70. An operator word used as a plain variable is read as the operator 🏗

A reserved word may **name** a variable on Lucee, and RustCFML now agrees for
every position that matters — declarations (`var case = 1`), argument names,
struct keys, member access (`case.label`), and, since the fix for the Preside
boot failure below, the for-in loop variable:

```cfml
for ( var case in caseQuery ) { out &= case.label; }   // Preside app services do this
```

The one remaining gap is READING a variable whose name is an *infix operator
word* — `contains`, `eq`, `is`, `mod`, `and`, `or`, `xor`, `gt`, `lt`, `imp`:

```cfml
for ( var contains in [ "a" ] ) { out &= contains; }   // Lucee: "a"   RustCFML: ""
```

The loop variable is assigned correctly; it is the *read* in operand position
that our parser resolves as the operator rather than the identifier. Assigning,
passing, and member access all work — only the bare read diverges. Rename the
variable, or read it through a scope (`local.contains`).

Note this is distinct from the words that are also LITERALS (`true`, `false`,
`null`) or statement keywords (`return`): those read back as the literal or fail
on **both** engines, so they are not a divergence.

## 71. `cfzipparam` covers `action="zip"` only 🏗

`<cfzipparam>` / `cfzipparam(...)` contributes an entry to the enclosing
`<cfzip>` — `source` (a file or a directory), `entrypath`, `prefix`, `filter`,
`recurse`, and literal `content` written at `entrypath`. That is the whole of
the `action="zip"` surface.

Lucee also accepts child params on the other actions (a per-entry `entrypath`
filter for `action="unzip"`/`"delete"`). Those are NOT implemented, and rather
than drop them silently `cfzip` raises when a param is supplied with any action
other than `zip`.

---

## 72. `throw()` mixing named and positional arguments — a deliberate superset 🏗

```cfml
throw( type="my.type", "the message" );   // named, then positional
```

Lucee refuses to **compile** the file that contains this ("Invalid argument for
function [ throw ], You can't mix named and unNamed arguments"), so the whole
component is unloadable there. RustCFML compiles it and raises at the call
instead — the mixed-arguments error every other function call already produces,
typed `expression`.

The reason for the divergence is blast radius: a shipped Preside extension
contains this line, and failing at parse time takes every service in the file
with it. Accepting the file keeps the rest of the component usable while the bad
line still fails when it runs. Nothing that works on Lucee behaves differently
here — only code Lucee rejects outright.

---

## 73. `cgi` is read-only — but a refused DELETE or APPEND still happens 🏗 *(GH [#372](https://github.com/RustCFML/RustCFML/issues/372))*

Writing to the `cgi` scope is refused, matching Lucee:

```cfml
cgi.qtest = "x";   // Expression: can't set key [QTEST] to struct, struct is readonly
```

The mark rides on the scope STRUCT rather than on the name, so an alias is
refused identically (`local.c = cgi; local.c.x = 1`) — which is what Lucee does
too. `url`, `form` and `cookie` stay writable on both engines.

Where we differ is the two operations Lucee *lets through*. Verified against
Lucee 7.1.0+204:

| operation | Lucee | RustCFML |
|---|---|---|
| `cgi.x = v`, `cgi["x"] = v`, `structInsert`/`structUpdate`, `cgi.insert()`, `cgi.x = nullValue()` | throws | throws |
| `structClear( cgi )` | throws (`can't clear struct…`) | throws |
| `structDelete( cgi, k )`, `cgi.delete( k )` | returns success, **does nothing** | performs the delete |
| `structAppend( cgi, s )`, `cgi.append( s )` | returns success, **does nothing** | performs the append |

Lucee's read-only struct throws from `put`/`clear` but leaves `remove`/`putAll`
as no-ops, so it reports a delete that never happened. Copying that would be
shipping a silent no-op; throwing instead would be a RESTRICTIVE divergence — the
kind that can break an app that works on Lucee. We do neither, and leave those
two operations working as they always did. Code that relies on either is already
broken on Lucee, in the quieter direction.

---

---

# Part E — Environment-specific 🌍

Restrictions that apply only on a particular target (wasm, CLI vs serve).

<a id="8"></a>

## 74. Form file uploads stream to disk — RESOLVED in v0.647.0 *(GH [#384](https://github.com/RustCFML/RustCFML/issues/384), [#385](https://github.com/RustCFML/RustCFML/issues/385))*

A `multipart/form-data` upload is parsed off the wire and each file part is
written straight to its own temp file. Nothing about the CFML surface changed —
`form.<field>.tempFilePath` and `cffile action="upload"` work as before — but
three things behind it did, and all three are worth knowing.

**The body is no longer buffered.** It used to be read whole by
`axum::body::to_bytes` before the handler ran, then split, then written to
disk: three copies of every uploaded byte, with the peak charged per concurrent
request. Measured on a 300MB upload: peak RSS growth 745MB → 3.1MB, and flat
from 300MB to 900MB. `server.maxRequestBodySize` is still enforced (413 on the
way past it) but is now a policy limit rather than a per-request memory
reservation. Hosts with no filesystem — the Cloudflare worker, wasm — still
buffer, which is what `web::RequestBody` distinguishes; Lucee streams the same
way (`FormImpl.initializeMultiPart` → `FileItemIterator` → `IOUtil.copy`).

**The temp filename no longer comes from the client.** It was
`cfupload_{filename}` interpolated with no sanitising at all, which was two bugs
in one string: `filename="../../etc/thing"` wrote outside the temp directory,
and two concurrent uploads of `avatar.png` shared one path and clobbered each
other. Temp files are now `cfupload_<pid>_<counter>.upload` — nothing
client-derived — and the original name reaches CFML as `clientFile`/`serverFile`
reduced to a bare basename. That last part matters beyond the temp directory:
`cffile action="upload"` joins `clientFile` onto its destination, so an
unsanitised name escaped *that* directory too. Lucee names its temp files
`tmp-<counter>.upload` for the same reason.

**`getHttpRequestData().content` follows Lucee's rule now.** It used to be
`String::from_utf8_lossy(body)` for every request that had a body, whatever the
content type — a second full-size copy, and for binary input a *lossy* one, so
each invalid byte became a 3-byte U+FFFD and the value could be larger than the
body it had already corrupted beyond recovery. Per
`ReqRspUtil.getRequestBody`, a body is now text when the content type is
absent, form-urlencoded, or a text mime type (`HTTPUtil.isTextMimeType`, which
counts anything containing `xml`, `json`, `rss`, `atom` or `text`), and
`CfmlValue::Binary` otherwise. A multipart request exposes an empty `content`,
matching Lucee's `FormImpl.getInputStream()` — and a streamed upload never had
the bytes to expose in any case.

> **Not yet done: temp files are never cleaned up** — GH
> [#386](https://github.com/RustCFML/RustCFML/issues/386). They accumulate in
> the system temp directory for the life of the host. Deliberately left out of
> this change rather than bundled into it, because the fix is a design decision
> rather than a detail: Lucee deletes them when the form scope is released
> (`FormImpl.release`), but skips that entirely for any request that used
> `cfthread` (`PageContextImpl.release` only calls `urlForm.release` in its
> non-`hasFamily` branch), so one thread anywhere in a request leaks its
> uploads permanently. That hole is the conservative answer to a real hazard —
> a `cfthread` can outlive its request and may have been handed
> `tempFilePath` — and our threads have the same shape. The plan in #386 is
> request-end deletion for requests that spawned no thread, plus an age-based
> reaper for the rest, which also covers what request-end deletion structurally
> cannot: crashes, `kill -9`, and files left by a previous run.

## 75. Template lookup is case-insensitive; file I/O is not — a deliberate superset *(GH [#387](https://github.com/RustCFML/RustCFML/issues/387))*

On a case-sensitive filesystem (Linux/ext4) the engine resolves a **component
or template** path case-insensitively, folding every segment: `SqlRunner.cfc`
answers `createObject("component","...database.sqlRunner")`, and an on-disk
`siteTree/` answers `...system.sitetree.SiteService`. macOS (APFS) and Windows
fold in the filesystem and never reach this code.

**This is a superset, not Lucee parity.** Verified against Lucee 7.1.0.204 on a
case-sensitive APFS volume: Lucee raises `invalid component definition, can't
find component [sqlRunner]` for both the relative and the mapped form. We are
deliberately more permissive, because ACF-on-Windows and macOS development have
made case-insensitive component lookup a de-facto part of the language, and a
codebase that runs everywhere else should not fail only on Linux.

**File I/O deliberately does NOT fold**, and that is where Lucee parity is
kept: `fileExists`, `fileRead`, `fileReadBinary`, `getFileInfo`,
`directoryExists` and `directoryList` all stay case-sensitive. Folding them was
tried (PR [#388](https://github.com/RustCFML/RustCFML/pull/388)) and is a trap:
with `filedelete`/`directorydelete`/`fileopen` in the same path-resolution
list, `fileDelete("./Foo.txt")` deletes an on-disk `foo.txt` and
`directoryDelete("./Cache", true)` recursively removes `cache/` — a file the
caller never named. It also splits `fileExists` from `fileWrite`: the former
reports a file present under a casing the latter will not write to, so
`if (!fileExists(p)) fileWrite(p, ...)` silently skips a legitimate write.
Component lookup has no such hazard: it only ever reads, and the worst case is
finding a file.

Cost is zero when nothing is mis-cased. Every resolution order runs exactly as
before, and the case-insensitive pass runs only once the whole order has missed
— which is also what keeps priority intact: an exact match in the last mapping
still beats a case-folded one in the first. The fold reads one directory
listing, cached per **directory** (not per path) behind the same two layers and
the same negative-answer generation as the existence memo, so a tree that
probes many absent override CFCs in one directory pays a single listing.

## 76. `java.util.concurrent` is a native executor, not a JVM thread pool 🏗

`ThreadPoolExecutor` / `ScheduledThreadPoolExecutor` / `ExecutorCompletionService`
and friends are backed by RustCFML's own threading, not by a JVM. The observable
contract is kept: `maxPoolSize` bounds how many tasks run at once, the work
queue's capacity is enforced, all four `RejectedExecutionHandler` policies
(`DiscardPolicy`, `DiscardOldestPolicy`, `AbortPolicy`, `CallerRunsPolicy`)
behave as documented, `invokeAll` blocks and cancels stragglers on timeout,
`invokeAny` raises `java.util.concurrent.TimeoutException`, and a periodic
`ScheduledFuture` stays pending until it is cancelled. cfconcurrent's own
TestBox suite passes 30/30, matching Lucee 7.1.0.204 spec for spec.

Two divergences are deliberate:

**A rejected or cancelled-before-it-ran task resolves as CANCELLED.** The JVM's
`DiscardPolicy` returns a `Future` that never completes, so `get()` blocks
forever. We resolve it instead: `isCancelled()` is true and `get()` returns
null. Being bug-compatible here would mean deadlocking a request thread on a
task the pool deliberately threw away.

**The pool bounds concurrency and queueing, it does not pool VMs.** Each task
still gets a fresh child VM, as `cfthread` does; worker threads gate execution
rather than being long-lived interpreters. Nothing observable from CFML depends
on this, but a task cannot leave state behind in "its" thread for the next task
on that thread to find — which is true of `cfthread` too.

`org.pixl8.cfconcurrent.LuceeRunnable` / `LuceeCallable` (Preside's 4.7KB helper
jar) are shimmed to the same proxy `createDynamicProxy` produces: RustCFML
reports `server.lucee.version` as `7.x`, so cfconcurrent's `_isLucee5()` gate
sends it down the jar path, and the jar's job — rebuilding a page context on the
worker thread — is already done by the engine's thread seed.

---

## 8. Environment-specific 🌍

| Feature | Restriction |
|---|---|
| `<cfdirectory>` | Not supported on `wasm32` (no filesystem). |
| `<cfzip>` | Not supported on `wasm32`. |
| `<cflock>` | No-op in CLI mode (no server state); enforced in serve mode. |
| `<cfcache>` | No-op today (could emit Cache-Control in serve mode). |
| `runAsync` / `_schedule` — `delayMs` | On `wasm32` (and other no-real-threads builds) `delayMs` is ignored: the closure runs inline immediately rather than being scheduled. With real threads it is honoured. |
| `_schedule` — `everyMs` / `spacedMs` | Honoured with real threads: `everyMs` is fixed-rate (period measured from each run's start, missed ticks **skipped** rather than burst-replayed), `spacedMs` is fixed-delay (measured from each run's end); `everyMs` wins if both are given. A run that throws is not rescheduled, and `cancel()` stops the schedule and the run in flight. On `wasm32` (and other no-real-threads builds) they are still ignored along with `delayMs` — the closure runs inline exactly once. |
| `java.util.Collections.unmodifiable*` / `synchronized*` shims | Identity no-ops — they return the same collection with no true immutability / synchronization. |
| `java.security.KeyPairGenerator.generateKeyPair()` | Not supported on `wasm32`: there is no OS entropy source, and inventing a key from a deterministic source would be worse than failing. Throws with that explanation. Generate the pair elsewhere and supply it as PEM. |
| `SHA512withECDSA` **signing** (P-521 only) | Not supported on `wasm32`, for the same reason: `p521` has no RFC 6979 implementation, so P-521 signing draws a random nonce. P-521 *verification*, and both signing and verification on P-256/P-384 (which are RFC 6979 deterministic), work everywhere. |

---


<a id="77"></a>

## 77. The Cranelift JIT was removed (v0.653.0) 📌

The optional Cranelift JIT (`--features jit`, ~10,500 lines under
`crates/cfml-vm/src/jit/`, five Cranelift crates) is gone. This records why, so
nobody re-derives it.

**It compiled almost nothing.** On a Preside boot plus 20 renders, with the
hotness threshold forced to 1 so every function attempted admission, **13 of
1,345** distinct functions were admitted — 1.0%. 99.0% were rejected by the body
analyser, and the top blocking opcodes were `CallMethod` (123) and
`CallMethodNamed` (43): **component method dispatch**, which is what a real CFML
application mostly is. Even a hand-written ideal numeric benchmark — a hot pure
arithmetic loop and a recursive `fib` — compiled **zero** functions, because a
`while` loop, `%`, `int()` and `arguments.n` are each outside the admitted
subset. A bare `function add(a,b){ return a+b; }` did compile.

**It was a net slowdown.** Warm Preside homepage, interleaved A/B, CPU time,
three rounds: JIT off was **1.1%, 7.0% and 3.2% faster**. You paid admission
checks and compilation for coverage that never materialised.

**It was a second, permanently-diverging semantics surface.** v0.649.0 found
three real divergences in one session (unchecked declared param types on the
compiled entry path; a compiled→compiled direct native call that bypassed the
dispatch-level check; a closure's writes to an enclosing `var` silently lost).
It also never polled the cooperative-cancellation flag the interpreter checks on
every loop back-edge, so `thread action="terminate"` and `Future.cancel(true)`
silently failed on hot loops — a JIT'd runaway loop could not be stopped. And a
CLI run can never catch a JIT bug (nothing gets hot in one request), so
`tests/runner.cfm` stayed green through all of it.

**Independent confirmation.** MatchBox (`ortus-boxlang/matchbox`) is the same
idea with more machinery — four tiers, real side-exit deopt, inline caches in
Tier-2 loops — and hits the identical exclusion list: no member/index access, no
`new`/array/struct literals, no dynamic calls, no exceptions. Its own docs say
"most effective for pure computational functions", it publishes no perf numbers,
and it ships opt-in and off in WASM/embedded builds.

Removed: the module, the `jit` feature on `cfml-vm` and `rustcfml-cli`, five
Cranelift dependencies (29 crates in the tree), and the `--no-jit`,
`--jit-threshold`, `--jit-stats`, `--jit-coverage` flags with their
`RUSTCFML_JIT*` env vars. Binary 62M → 59M. The numeric closed-form checks it
carried live on as `crates/cfml-vm/tests/numeric_semantics.rs`.

**What replaces it:** nothing, and deliberately. The measured cost centre is
frames and member access, not arithmetic — see
the inline-cache section of the performance roadmap (planning/PERFORMANCE_ROADMAP.md §2.2, a working doc) for the one idea worth taking from the JIT
(inline caches for member reads), sized against the interpreter instead.

## 78. Allocations made on a `cfthread` were invisible to the cycle collector (fixed v0.653.3) 📌

**Symptom.** Preside `?fwreinit=true` retained ~110 MB and exactly ~111,000
tracked nodes per reload — 1.0 G → 1.9 G over eight reloads, linear, never
reclaimed. The cross-request sweep walked every dead generation and freed ~1,500
nodes of it. Every hub object of a dead generation (the WireBox `Injector`,
`LogBox`, `Controller`, `Logger`) read `strong_count = 1 + internal_in + 1..26`:
a small refcount surplus that no tracked holder, blueprint, method table,
native, thread, VM, cache, session or app-config entry accounted for. The
surplus was real — an experiment that ignored it made memory flat and broke the
next reload with `Variable 'siteTemplate' is undefined`.

**Cause.** The collector's allocation log is **thread-local**, armed per thread by
`cycle_gc::enable()`, and only the request path (`compile_and_run`) ever called
it. `spawn_cfthread` never did. So `log_struct`/`log_array`/`log_scope` were
no-ops on every spawned thread, and nothing a cfthread allocated was in any
survivor set. Two consequences: a cycle built on a thread could never be freed,
and — the one that mattered — every reference such an allocation held read to the
collector as EXTERNAL ownership of its target, pinning the target's whole
transitive closure. Preside's reload spawns ~37 threads (task manager,
heartbeats, log listeners, module loaders) whose products land in the
application graph, so each generation's singletons carried a few invisible
holders and the whole generation stayed live.

**Fix.** `spawn_cfthread` now arms the log before the body and, when the body
returns, either `collect()`s (carrying survivors into the cross-request set) or
`defer_current_log` if a nested cfthread is still running — the same contract as
the request path. Measured: the sweep now reclaims whole generations
(112,333 / 112,336 / 341,512 in one pass under the production doubling rule);
tracked nodes oscillate between one and three generations instead of doubling;
footprint holds at 1.1–1.3 G over eleven reloads. Regression test:
`crates/cli/tests/thread_alloc_gc.rs` (verified non-vacuous).

**Also closed on the way, all real gaps in the "the two walks must mirror"
invariant, none of which moved the Preside number on its own:** the shared
`method_table` reached through an Instance's untracked data maps is now counted
and marked; `classify` walks function/closure bodies and `CfmlParam::default`;
`CompletionQueueNative` implements `visit_values`; closure captured scopes are
logged at creation (`log_scope` was previously only reached by the relog hook).

**Diagnostics added (env-gated, off by default).** `RUSTCFML_CACHE_CENSUS=1`
(sizes of every process-lifetime cache + live-VM count), `RUSTCFML_GC_ROOTS`
now also reports `orphans: N unreachable, M carry an external ref`, the complete
`HELD-BUT-EXTERNAL` / `UNHELD` orphan sets with contents, per-anchor total holder
edges, lock skips, and probes for sessions, app config and running thread
bodies. `RUSTCFML_GC_UNREACHABLE_REPORT=1` lists what no probe root can reach
during a sweep (report only — an earlier suppressing variant freed live data and
is gone).

**Follow-up (v0.653.4): the displacement sweep.** With the leak fixed, the
cross-request sweep still ran only on its doubling budget (`next_sweep = 2 ×
live`), so a reload's dead generation was not re-examined until two or three
MORE reloads had accumulated, and mimalloc keeps the high-water mark — footprint
plateaued at ~1.3 G with three generations resident. The relog hook already knows
the exact moment a generation is displaced, so it now tells the collector
(`cycle_gc::note_displacement`, fired on both the normal and the
budget-exhausted exit — the budget-exhausted one IS the generation-sized case).
The request loop sweeps at that request's end; if the reload's threads still
hold the old generation (they do, for ~15 s while the log listeners idle out),
each thread exit retries, no oftener than every 5 s and at most 6 times, until a
sweep reclaims at least half of what was displaced. Measured on Preside: retry
#4 reclaims the full generation ~15 s after each reload; tracked nodes return to
ONE resident generation (~117k) between reloads; footprint oscillates 840–970 M
over eight reloads instead of plateauing at 1.3 G. A sweep of ~230k nodes costs
170–370 ms. `RUSTCFML_GC_DISPLACE_SWEEP_MIN` (default 1,000 nodes; 0 disables)
is the size a displacement must reach to count. Regression test: the second
case in `crates/cli/tests/thread_alloc_gc.rs`, run with the relog budget capped
so the sweep — not the ordinary request-end collect — is what has to free it;
non-vacuous (with the trigger disabled the graph stays tracked).

**Measured, not a regression — render cost.** Isolated warm-render A/B on the
Preside homepage, published v0.653.2 vs v0.653.3 vs v0.653.4: p50 5.8 / 5.9 /
6.0 ms end-to-end with ±0.2 ms round-to-round noise; server-side footer 5.22 /
5.10 / 5.03 ms. On an ordinary page the displacement check is one atomic load
and no GC line is emitted.

**Measured, not a regression — the Wheels suite.** One suite request
(2,737 tests, `?reload=true`) peaks at ~6.4 G and successive runs read 6.4 →
10.0 → 13.9 G. Identical on the published v0.653.2 binary, so pre-existing. It
is not collector garbage: the mid-request sweep finds 8–14 M nodes live and
reclaims 0, and the request-end pass then sees only ~280k of the 16 M logged
entries still alive — the rest were acyclic request-lifetime data freed by
refcount when the test frames returned. The cross-run growth is mimalloc
retaining the peak. Two things were tried and rejected with numbers: clamping
the incremental budget to half the log cap (put the budget BELOW the live count
→ a 330 ms sweep at every frame exit, 270 sweeps in 90 s) and polling the
incremental sweep at frame exit (timely sweeps, nothing extra freed, run ~15%
slower). The incremental debug line now always prints, with the log size taken
and the pass duration, so this is visible next time.

**Still true.** The remaining oscillation is mimalloc's high-water mark plus the
~15 s window in which a reload's threads legitimately hold their predecessor. The
`SHARED_FN_REGISTRY` Vec is indexed by a monotonic id and grows ~2 MB per reload
in dead `Weak` slots — small, real, untouched.

## 79. `--max-memory`: a process-wide footprint limit with 503 back-pressure (v0.653.5) 📌

**Why.** A JVM engine runs under `-Xmx`, so a container that says "2 GB" can
hand the process 1.5 GB and know it will not be OOM-killed. RustCFML's footprint
is live data plus the pages mimalloc retains after a peak, and nothing bounded
it: one Wheels suite request peaked at 6.4 G and three in a row read 13.9 G.

**What it does (the soft tier).** `--max-memory 1.5G` (or `RUSTCFML_MAX_MEMORY`,
or `auto` = 75% of the cgroup limit). Above 85% of the limit the server refuses
NEW requests with **503 + `Retry-After: 2`** — a healthy back-pressure signal for
a load balancer or orchestrator — and sheds: the collector's cross-request sweep
plus `mi_collect(true)` to return retained pages. In-flight requests finish.
Admission reopens below 95% of the soft line (hysteresis: a process sitting on
the line otherwise flaps 503/200 on alternate requests). Shedding is rate-limited
to one pass per 2 s.

**What it measures.** The number the OS or container kills on: cgroup
`memory.current` when a cgroup limit exists, else `/proc/self/statm` resident
(Linux), `proc_pid_rusage` `ri_phys_footprint` (macOS), mimalloc's `current_rss`
as the last resort. ⚠️ mimalloc's RSS was tried first and is WRONG on macOS: it
counts pages already released with `MADV_FREE` (reclaimed lazily), read 763 M
against a real 585 M, refused traffic there was room for, and made shedding look
inert because the meter never moved.

**Sizing.** Leave room for the reload window: for ~15 s after a reload two
application generations are legitimately resident. Preside idles at ~600 M and
peaks ~950 M on reload, so a 700 M limit serves 503 for most of that window —
correct behaviour for a limit set at 1.15× steady state, but not what you want.
`auto` in a 2 G container gives 1.5 G, which is right for that app.

**Not yet: the hard tier** — aborting the in-flight request that has allocated
the most since it started, so a single runaway request (a Wheels suite run) gets
a clean 500 instead of taking the process to the limit. Needs the per-request
allocation accounting wired to the abort path.

Tests: `crates/cli/tests/max_memory.rs` (503 → finish → reopen, end to end) and
the unit tests in `crates/cli/src/memory_limit.rs`.

## 80. The regex caches were bounded by entry count, not by bytes — 1.3 GB of a Wheels run (fixed v0.653.6) 📌

**What the heap profiler showed.** Of ~2 GB live at the peak of a Wheels
test-suite request, 1.3 GB was compiled regular expressions: 676 MB in
cfml-stdlib's `REGEX_CACHE` (`reFind`/`reMatch`/`reReplace`) and 620 MB in the
`java.util.regex.Pattern` shim's cache. Both caps were 4,096 *entries* — but a
compiled `regex::Regex` carries a one-pass DFA of up to 1 MB and a lazy-DFA cache
of 2 MB by default, so the "hard memory ceiling" the comments promised was ~13 GB
per cache. The suite mints ~1,100 distinct patterns per run (interpolated values
make each one unique), and eviction was clear-all, which also threw away the
handful of patterns every request reuses.

**Fix.** Cap 256 entries per cache; **least-recently-used eviction** (a quarter
at a time, stamps refreshed under the read lock on a hit) so what stays is what
was reused; lazy-DFA cache per pattern 2 MB → 256 KB (a pattern that needs more
falls back to a slower engine, never fails). Wheels: footprint after run 1
6.4 G → 4.9 G, after run 2 10.0 G → 8.5 G, run time unchanged (37 s). Preside
warm render p50 6.1–6.3 ms vs 5.8–6.0 baseline (±0.2 noise), server-side 4.88 ms.
CFML suite 8827/8827.

**Deliberately NOT done.** An NFA `size_limit` was tried and removed: a pattern
over it fails to compile and silently falls back to backtracking on every call.
The one-pass DFA megabyte per pattern is not reachable through the `regex`
crate's builder; reclaiming it means building on `regex_automata::meta::Regex`
with `onepass_size_limit`, a refactor of every regex call site.

**Two measurement traps from this work.** (1) Timing a page without checking
the HTTP status: MySQL had gone away under a Docker restart and three "1.5 s
render regressions" were Preside's 500 error page. Always print `%{http_code}`
next to `%{time_total}`. (2) The remaining ~3.3 GB live after a Wheels request
looked like component-metadata deep copies that were "not in tracked containers"
because only ~21k tracked nodes survived. They were tracked containers — just
never logged. See §81.

## 81. The allocation log filled with duplicates, hit its cap, and paused — everything allocated afterwards leaked past the request (fixed v0.653.7) 📌

**Symptom.** After one Wheels test-suite request (2,737 specs) the process held
3.3 GiB of live heap that nothing referenced from CFML, and a second run stacked
another ~4 GB on top (footprint 4.7 G → 8.6 G). The heap profiler attributed it to
`resolve_component_template_impl` / `resolve_inheritance_chain` / `deep_copy`
— component instances and their scopes — while the collector reported only
~21k tracked nodes. The per-request VM was gone (`live_vms=1`).

**Cause: three collector behaviours that were each harmless alone.**

1. The allocation log is a *log*, not a set. The relog hook
   (`CfmlValue::relog_cycle_nodes`, which enters a displaced subgraph so trial
   deletion can evaluate it) and the closure-scope sites enter the SAME node
   every time a key holding a big graph is overwritten. Wheels' reload and
   test bookkeeping did that until ~260k distinct nodes occupied 16M entries.
2. `collect_incremental` de-duplicated the survivors by pointer when it
   collected them — but then re-entered every entry of the RAW log that still
   upgraded, and counted each as live: `15,808,984 live` for ~260k real nodes.
   Its doubling budget became 31M, above the 16M log cap, so no sweep ever ran
   again in that request.
3. At the cap, `log_push` *paused logging for the rest of the request* (by
   design: "collecting a partial log is conservative"). Conservative is right —
   nothing live is ever freed — but every cycle minted after the pause is
   invisible to the collector, is in no survivor set, and is never swept. A CFC
   instance is a cycle. The remaining ~50 s of the suite built ~3 GB of them.

`[cycle_gc] incremental sweep over 16000000 reclaimed 62057 node(s); 15808984
live, next sweep at 31617968` followed by `request end: log_len=Some(16000000)`
is the signature.

**Fix.**
- `collect_incremental` carries the DE-DUPLICATED survivors back into the log
  and takes `live` from that count; the next budget is clamped to the log cap.
- The relog hook consults a per-sweep-interval de-dup set
  (`cycle_gc::relog_first_sight`); a node already entered since the last sweep
  is in the log and is not entered again. Cleared by every sweep and at request
  end.
- At the cap, `log_push` **compacts** the log (drops dead entries, keeps one per
  distinct live node — a `strong_count` read and a pointer per entry, no graph
  walk) and carries on. Only if the compacted log still fills three quarters
  of the cap does the request genuinely hold that many distinct containers,
  and only then does logging pause, as before.

**Measured, Wheels suite (`?db=sqlite&reload=true`), same 2737/3/0/16 result:**

| | before (v0.653.6) | after |
|---|---|---|
| live heap 25 s after run 1 | 3.3 GiB | 122 MiB |
| footprint after run 1 / run 2 | 4.7 G / 8.6 G | 449 M / 393 M |
| peak footprint during a run | 7.9 G | 1.1 G |
| suite wall time | 71 s | 65–66 s |
| mid-request sweeps per run | 35, then paused | ~98, ~12 s total |

The log now never reaches the cap on this workload (no compaction fired). The
sweeps that used to be paused now run for the whole request and cost ~12 s of
the 65 s — yet the run is faster overall, because a 5 GB heap was costing more
than that in allocation and paging. That sweep share (~20 %) is the next lever:
a generational sweep that skips nodes which survived the previous pass would cut
most of it, since the suite's ~300k-node live set is re-walked ~100 times.

**The wrong conclusion this replaces.** An earlier pass on this workload
concluded "the 6 G peak is the suite's own live request data; the collector
sees 8–14 M nodes live and reclaims 0; nothing to take". The 8–14 M were
duplicate log entries of a few hundred thousand nodes, and the "0 reclaimed"
sweeps were the paused collector. When a sweep reports far more `live` than the
request could plausibly hold as distinct containers, suspect the log, not the
workload.

Tests: `crates/cfml-common` unit tests for the de-dup/compaction paths; the
Wheels numbers above are the end-to-end evidence.


## 82. `BytecodeOp` was 48 bytes wide — a payload added two days after the initial commit set the size of every instruction (32 B in v0.653.8, 24 B in v0.653.9) 📌

**What it costs.** Compiled CFML is a `Vec<BytecodeOp>` per function, and a Rust
enum is as wide as its widest variant, so `Add`, `Pop` and `Jump` each occupied
as many bytes as the fattest call op. Preside's ~790k compiled instructions were
36 MiB of the 74 MiB bytecode cache, itself the largest item in a 265 MiB live
heap after a reinit (§81's profile).

**How it got there** (`git log -S` on the enum):

| date | change | widest variant | size |
|---|---|---|---|
| 2026-02-21 | initial commit | `String(String)` | 32 B |
| 2026-02-21 | mutating-method write-back | `CallMethod(String, usize, Option<(String, Option<String>)>)` | ~80 B |
| 2026-02-23 | write-back path as a Vec | `CallMethod(String, usize, Option<Vec<String>>)` | 64 B |
| 2026-05-30 | perf-plan size probe added | ceiling set at 64 B | 64 B |
| 2026-08-09 | name interning (`Name` = 8 B) | `CallMethodNamed(Name, Box<Vec<String>>, usize, Option<Vec<String>>)` | 48 B |

The op doubled two days in, for the receiver write-back path carried inline on
every method call, and was only ever shrunk as a side effect of interning. The
probe's ceiling followed the size down; it never drove it.

**Fix.** The rare fat payloads move behind a pointer and the counts that never
need 64 bits are narrowed, so the widest variant's payload plus the tag fits in
32 bytes:

- `CallMethod` / `CallMethodNamed`: write-back path `Option<Vec<String>>` →
  `Option<Box<Vec<String>>>` (8 B, niche-optimised); arg count `usize` → `u32`.
  With four 8-byte fields `CallMethodNamed` was exactly 32 B of payload and
  forced a 40 B op — the count had to shrink too.
- `NewObjectNamed` / `CallNamed`: `Vec<String>` → `Box<Vec<String>>`.
- `ForLoopStep` / `ForSlotStep`: jump target `usize` → `u32`.

Nothing on a hot path gained an indirection: the boxed fields are read only on a
named-argument call or a write-back, and the loop ops copy `u32`s. The size probe
in `crates/cfml-codegen/src/compiler.rs` now asserts `≤ 32` and its message
records the history, so the next regression is caught at build time.

**Measured.** `size_of::<BytecodeOp>()` 48 → 32 B; instruction storage on
Preside 36 → 24 MiB (~12 MiB, ~4.5 % of the live heap). CFML suite 8827/8827 CLI
and 8940/8940 served (both modes), workspace and cfml-vm green, wasm32 and
wasm-pack built. Preside warm render, interleaved A/B with both servers alive (six alternating rounds of 40): v0.653.7 p50 6.14–6.60 ms, this build 5.96–6.72 ms, medians 6.32 vs 6.23 — no cost. A sequential A/B first read the new build 0.7 ms slower; that was run ordering, not the change — interleave.

**Second step (v0.653.9): 32 → 24 B.**

- `String` / `UnsetPath` / `Include` carry `Arc<String>` (8 B) instead of
  `String` (24 B). This is also a CPU change on a hot path: `op_string` used to
  build `Arc::new(s.to_string())` — a fresh allocation and copy — for EVERY
  literal executed (struct keys, string arguments, every `"..."`); it now
  clones the Arc the compiled function already holds, a refcount bump.
- The fused loop ops (`ForLoopStep`, `ForSlotStep`, `JumpIfLocalCmpConstFalse`,
  `JumpIfSlotCmpConstFalse`) carry their constants as `i32` and their jump
  target as `u32`. The matchers that detect a counted loop refuse a literal
  that does not fit an `i32`, so such a loop compiles to the generic shape;
  the VM widens to `i64` at the arm head, so the arithmetic is unchanged.
- `CallMethodNamed` keeps its argument names and write-back path in ONE boxed
  `NamedMethodCall` (`Name` + `Box` + `u32` = 20 B); the VM arm binds the
  common parts of both call variants explicitly instead of an or-pattern.

`size_of::<BytecodeOp>()` 32 → 24 B (probe ceiling 24). Preside instruction
storage 24 → 18 MiB. Interleaved warm-render A/B (six alternating rounds of 40,
both servers alive): v0.653.8 p50 medians 6.07 ms, this build 6.24 ms, rounds
split three and three — no measurable change either way; the literal-push
saving is real but below the noise of a ~6 ms page. Gates green (workspace
88 suites, cfml-vm 312, CFML 8827 CLI / 8940 served both modes, wasm32,
wasm-pack).

**Not taken: 16 B.** The floor is now the two-`Name` ops (`LoadLocalProperty`
and the slot-property family, 16–18 B) and the loop ops (~17 B). 16 B needs
every `Name` payload as a `u32` interner id, which turns every name-carrying op's
dispatch into an interner lookup; measure before attempting.

## 83. Where a booted Preside's memory actually is — the census that retired three "levers" (v0.653.10 diagnostics) 📌

`RUSTCFML_CACHE_CENSUS=1` now reports, at every request end, live `ClassBlueprint`s
per class and what they hold, and each application scope's approximate size with
a one-pass shape breakdown (`CfmlValue::approx_heap_bytes` — diagnostics only,
strings by length, containers by entry count, each shared backing counted once).
Measured on the Preside test site after boot and after `?fwreinit=true`, with the
sampling heap profiler alongside:

| | after boot | 35 s after a reinit |
|---|---|---|
| physical footprint | 456 M | 520 M (other runs: 687–750 M) |
| live heap (profiler exact counter) | 194 MiB | 206 MiB |
| tracked collector nodes | ~116k | ~116k |
| live blueprints | 518 for 483 classes | 525 (max 6 per class) |
| blueprint payload | metadata 139 K, getMetadata cache 3.9 MiB | same |
| application scope (est.) | 29.5 MiB, 99.9 % under `wireBox` | 29.7 MiB |
| …of which getMetadata-shaped structs | 456 structs, 8.1 MiB | same |
| …instances / functions / plain structs | 5,072 / 12,498 / 49,131 | same |

**What this settles.**

1. **Process-wide blueprint sharing is not a memory lever.** The duplication is
   42 blueprints out of 525 and their payload is ~4 MiB; the static-scope
   semantic change it needs is not worth that.
2. **Component-metadata copies are ~8 MiB of the app graph plus ~4 MiB on the
   blueprints, largely the same backings** (`instance_metadata` hands out the
   cached handle, not a copy). Sharing them further risks a framework that
   mutates the struct it was given (ColdBox's `getInheritedMetaData` does).
   Single-digit percent of the live heap; retired.
3. **The +240 MB a reinit adds to the footprint is allocator retention, not
   data.** Live heap grows ~12 MiB per reinit — 8 MiB of it is ONE extra pooled
   MySQL connection (mysql_common reserves two 4 MiB frame buffers per
   connection), the rest is metadata churn within sampling noise — while the
   footprint grows 64–290 MB. The reload allocates the new generation while the
   old is still live, interleaved in the same mimalloc segments; when the old
   one is freed 15 s later the segments are left partially used and cannot be
   returned. Measured: `MIMALLOC_PURGE_DELAY=0` and
   `MIMALLOC_ABANDONED_PAGE_PURGE=1` change nothing (687/735 → 697/748 →
   708/749 M over two reinits), so it is fragmentation, not un-purged free
   pages. It plateaus: the second reinit adds far less than the first, and
   earlier runs held 840–970 M over eleven reloads. That is ~1.6× the live
   heap — the same order as a JVM's heap headroom, and bounded.

**Where the 194 MiB live heap is** (post-boot, profiler): bytecode cache ~62 MiB
(1,913 files; instructions now 24 B/op), MySQL frame buffers 48 MiB *reserved*
(resident only once a result set has touched them), application graph ~30 MiB
(estimator; ~45 MiB by the profiler), regex caches ~15 MiB, interned keys/names
~12 MiB, persistent collector set ~6.5 MiB. The remaining levers, in order:
`Name` payloads as `u32` ids (16 B ops, ~6 MiB), and a generational sweep for
Wheels-style workloads (CPU, not memory).

**Trap.** A per-member drill-down with a fresh `seen` set is useless on a DI
graph: every member of the WireBox injector reaches the whole 29.7 MiB, so each
reports the total. Use one shared walk with a shape classifier, or a dominator
tree.

## 84. Generational mid-request sweep: survivors are promoted, not re-walked (v0.653.10) 📌

**Before.** Every mid-request sweep re-ran trial deletion over the whole log —
new entries AND every survivor of the previous sweep, carried forward. The
doubling budget kept that amortised-linear, but on the Wheels suite it meant
~100 sweeps re-walking a ~300k-node live set to find cycles among the new
entries: ~12 s of a 54 s run (§81).

**Now.** The log has two generations. A **minor** sweep runs trial deletion over
the YOUNG entries only (allocated since the last sweep); a node still owned from
outside that set — including from an old node — reads as external and is kept,
so a minor is conservative in exactly the way a partial log is (§81). Its
survivors are **promoted** to the old generation, which is re-walked only by a
**major** sweep (young + old) once its LIVE size has doubled since the last major,
or by `collect()` at request end, which always takes both generations. Two
details that mattered as much as the split:

- **Old-generation membership set.** The relog hook re-enters displaced old
  nodes into the young log every interval, and each minor re-promoted them, so
  the old generation held ~2× duplicates and majors fired on count: 14 majors
  per run over 8.6M entries reclaiming 250k nodes. With a pointer set on the
  old generation (and dead entries pruned before a major is called): 6 majors
  over 1.7M entries.
- **Young budget 25k → 100k.** The 25k knee was measured when survivors were
  re-walked; with minors linear in allocations it only sets how much young
  garbage accumulates between sweeps. Measured 25k / 50k / 100k: sweeps 5.8 /
  6.1 / 4.1 s, wall 49 / 46 / 45 s, peak 760 / 757 / 764 M. Peak did not move.

Pointer-keyed sets and maps in the collector use `FxHash` (keys are `Arc`
addresses); measured within noise on its own, kept because it is free.

**Measured, Wheels suite (`?db=sqlite&reload=true`, 2,737 specs, identical
results):**

| | v0.653.9 | this build |
|---|---|---|
| wall time | 53.8–54.0 s | 45–49 s |
| mid-request sweep time per run | ~12 s (~590 sweeps) | ~4.5 s (119 minors + 3 majors) |
| peak footprint | 860–920 M | 760–770 M |

Preside (interleaved, both servers alive, six rounds of 40, all 200s): v0.653.9 p50 6.59–8.08 ms, this build 6.61–8.29 ms, rounds split three and three — no change. Reinit footprint over three reloads: 700/776/782 M vs 513/750/823 M — the same fragmentation plateau (§83). A first attempt ran with MySQL down and read footprint 1.0 → 3.9 G on BOTH arms because every request re-booted the application; always check `%{http_code}` before reading a render or footprint number (§80).

**What a minor cannot see.** An old node whose last external reference is dropped
by a frame exit (no mutation hook fires) stays until the next major or request
end. That is the same retention the doubling rule always had; majors now fire
on live growth, so mid-request retention of old cyclic garbage is bounded by
2× the live old set, as before.

Tests: `crates/cfml-common` `incremental_tests` (minor reclaims young cycles,
promoted survivor reclaimed by a forced major, major budget backs off, duplicates
carried once, compaction, relog de-dup).

## 85. `--max-memory` hard tier: abort the in-flight request that is taking the process over (v0.653.12) 📌

**The gap.** §79 shipped the soft tier: above 85% of the limit, new requests get
503 + `Retry-After` and the server sheds. That protects against a rising tide of
ordinary traffic, but not against the case the limit exists for — ONE request
allocating without bound. Nothing arrives to be refused, and the request already
inside runs until the OOM killer takes the process, and every other request with
it. Measured on the fixture: with the hard tier disabled, a request building
structs in a loop reached **6.9 GB** and was still going 125 s later.

**The design.** A watchdog thread polls the footprint every 250 ms. Above 95% of
the limit it asks `cfml_common::mem_guard` for a victim: the in-flight request
with the largest allocation odometer. That request's next poll returns an
uncatchable error — the same class as `requestTimeout`, so a framework's
`catch( any )` cannot swallow it and let the heap keep filling — and every other
request continues.

- **Odometer.** A thread-local counter of tracked containers, bumped where the
  cycle collector already logs allocations, published to the request's registry
  slot at each poll point. Not bytes: bytes-per-request would need allocator
  instrumentation on the hot path, and "which request built the containers" is
  the question that actually identifies a runaway.
- **Poll points.** Every user-function frame entry, plus the blocking boundaries
  the request timeout uses. Frame entry costs one relaxed atomic load when no
  limit is configured. A tight loop calling no function is not interruptible —
  the same documented gap as `requestTimeout`.
- **95%, not 100%.** The abort itself has to run: build an error, a stack trace,
  a response. And a runaway can allocate a lot in one 250 ms tick.
- **500, not 503.** A runaway request must not be retried elsewhere.

**It never aborts an innocent request.** Only requests over a floor of 100,000
tracked containers are eligible. Memory held by the application scope, the
caches, or the allocator (§83: a Preside reload leaves a plateau of ~1.6× live
data) belongs to no request; killing one would lose work and free nothing. When
nothing clears the floor the watchdog logs that and sheds instead. The regression
test for this is the §79 hog: it holds 600 MB of *strings* in two containers, so
it takes the process over the hard line while staying far below the floor, and it
must finish normally.

**One trap worth recording.** The odometer was first published from the
collector's incremental sweep, on the reasoning that it runs at a predictable
allocation interval. It does not: that sweep runs at *component construction*
(§25), so a request building plain structs and arrays never published, its
odometer stayed at zero, and the watchdog could not tell it from an innocent
request — the 6.9 GB run above is exactly that bug. Publishing moved to the frame
-entry poll.

**Measured.** Preside warm render, interleaved A/B against v0.653.11, six rounds
of 40, all 200s: 6.01–7.12 ms vs 6.05–6.87 ms, medians 6.20 vs 6.28 — the
frame-entry poll costs nothing measurable. CFML suite 8827/8827 CLI and
8940/8940 served in both modes; workspace 88 suites; cfml-vm 312.

Tests: `crates/cli/tests/max_memory.rs` — the runaway is aborted and the server
still serves (verified non-vacuous: with `HARD_FRACTION` raised so the tier never
fires, the request reaches 6.9 GB and the test fails at the client read timeout);
a request that did not build the heap is not aborted; the soft tier's 503 and
reopen. Unit tests in `crates/cfml-common/src/mem_guard.rs` cover victim
selection, the floor, one-victim-at-a-time, and that an abort is visible only to
the request it was armed on.

## 86. `--serve` ignored SIGTERM, so every containerised stop waited out the grace period (fixed v0.653.14) 📌

Only `tokio::signal::ctrl_c()` (SIGINT) was awaited for graceful shutdown. That
is wrong in two different ways depending on where the process runs:

* **Outside a container** an unhandled SIGTERM takes the DEFAULT action and kills
  the process immediately. In-flight requests are cut off mid-response, the Unix
  socket file is left behind, and every `Drop` is skipped (the heap-profiler dump,
  the OTel flush). Measured: a 3s request signalled at t=1s returned a truncated
  read to the client.
* **As PID 1 in a container** it is worse. The kernel installs no default signal
  dispositions for PID 1, so an unhandled SIGTERM is simply IGNORED. SIGTERM is
  what `docker stop`, Kubernetes and systemd send, so every stop became "wait the
  full grace period, then SIGKILL". The reference image
  (`RustCFML-Docker`) carried `STOPSIGNAL SIGINT` purely to work around this.

**Fix.** A `shutdown_signal()` future that selects over SIGINT and (on unix)
SIGTERM, used by both the TCP and Unix-socket serve paths. `tokio`'s `signal`
feature was already enabled, so this is a handler, not a dependency. Registration
failure falls back to Ctrl+C rather than refusing to serve, and each signal logs
which one it was.

**Measured.** Idle server: graceful exit 118ms after SIGTERM. In-flight 3s
request signalled at t=1s: response delivered in full, server exited 2.0s later,
i.e. as soon as the request drained.

Tests: `crates/cli/tests/graceful_shutdown.rs`. Note which test does the work —
the idle-shutdown test passes with OR without the handler when run outside a
container (the default action kills the process anyway), so it pins the contract,
not the regression. The in-flight test is the real detector: with the handler
listening on a different signal it fails, the client getting a truncated read
(verified).

**For image authors.** `STOPSIGNAL SIGINT` is no longer needed from v0.653.14 and
is harmless to keep. Set the platform's grace period above your slowest request
so a draining server is never SIGKILLed.

## 87. `<cfflush>` streams and commits — what stops working afterwards, and where it degrades to a no-op 🏗 *(GH [#419](https://github.com/RustCFML/RustCFML/issues/419))*

Implemented in the `--serve` response pipeline, not just in the tag parser: the
first `<cfflush>` **commits** the response (status line + headers go out with
that chunk) and the rest of the page is delivered as chunked transfer-encoding
as it is produced. A page that never flushes takes exactly the buffered path it
always did, `Content-Length` and all — the streaming machinery is inert until a
flush actually happens.

**Flush targets the ROOT buffer, not the innermost capture — we follow Lucee,
not BoxLang.** Both engines were read at source:

* Lucee's `tag/Flush.java` calls `getRootOut().flush()`, and `getRootOut()` is
  `bodyContentStack.getBase()` — the base writer. A flush inside
  `<cfsavecontent>` therefore ships the page text written *before* the capture
  began and leaves the capture intact.
* BoxLang's `components/system/Flush.java` calls `context.flushBuffer(true)`,
  and its `BaseBoxContext.flushBuffer` drags *every* registered buffer out when
  `force` is set — so a flush inside a capture leaks the captured text to the
  client.

We match Lucee (and ACF). Verified live against Lucee 7.1:
`<cfsavecontent variable="cap">A<cfflush>B</cfsavecontent>` leaves `cap` as `AB`
on both engines.

**What raises after the response is committed.** This matrix was measured
against a running Lucee 7.1, not assumed — the servlet spec would suggest
`cfheader` is silently ignored on a committed response, but Lucee's
`Header.doStartTag` throws on `isCommitted()`. Our messages match Lucee's
exactly; only `cfcatch.type` casing differs, which is a pre-existing engine-wide
convention difference (Lucee lowercases engine-generated types) and is
immaterial because CFML type matching is case-insensitive.

| after `<cfflush>` | behaviour |
|---|---|
| another `<cfflush>` | fine |
| `<cfabort>` | fine |
| `<cfcookie>` | fine — silently ineffective, the header has gone |
| `<cfheader>` | raises `template`: `can't assign value to header, header is already committed` |
| `<cfcontent>` (**any**, not just `reset`) | raises `application`: `Content was already flushed` |
| `<cflocation>` | raises: `Response buffer is already flushed` |
| `<cfhtmlhead>` / `<cfhtmlbody>` | raises: `Page is already flushed` |

`isFlushed()` reports whether the response has been committed — Lucee's
`IsFlushed` is literally `pc.getHttpServletResponse().isCommitted()`, i.e. the
same flag the table above is keyed on, so it is the supported way to ask "can I
still set a header?". BoxLang has no equivalent. It is always `false` under the
CLI, where nothing is ever committed.

`interval="N"` auto-flushes once the buffer passes N bytes (Lucee's
`setBufferConfig(N, autoFlush=true)`). A non-numeric interval is a **cast**
error (`can't cast [abc] string to a number value`) raised regardless of
`throwonerror`, which covers only the flush itself; a negative or zero interval
is *not* an error on Lucee — it means "flush on every write" — and is accepted
here too.

**Where it degrades to buffering.** Two serve-mode paths build one buffered body
and cannot stream: the `onMissingTemplate` handler and the error-template path.
A `<cfflush>` reached from those keeps buffering — output still renders
correctly and in order, it simply arrives in one piece — and, importantly, does
**not** commit, so nothing that follows starts failing. It must never fall
through to the CLI's stdout path, which is the server's console, not the client.

In CLI mode the root output *is* stdout, so a flush genuinely prints what has
accumulated. Nothing is committed there because there are no response headers to
freeze, which is why `<cflocation>` and friends keep working after a CLI flush.

A flush from inside a `cfthread` body is a no-op: the thread writes to its own
captured buffer and has no client of its own (Lucee gives the thread a separate
`PageContext`, so its `getRootOut()` is not the request's either).

Tests: `tests/tags/test_cfflush.cfm` + `tests/tags/flush_target.cfm`. The
flushing cases run over HTTP against a target page rather than in the runner
itself, because a flush in the runner would freeze the runner's own headers and
break every later `cfheader`/`cflocation`/`cfcontent` test. 15/15 green on both
RustCFML and Lucee 7.1.

## 88. Component member reads were gated by OPCODE, not by reader — so the same rule denied insiders and served outsiders (fixed v0.657.0) 📌 *(GH [#420](https://github.com/RustCFML/RustCFML/issues/420), residual [#417](https://github.com/RustCFML/RustCFML/issues/417))*

**The reported symptom.** Inside a component, `this[ "priv" ]` returned NULL
while `this.priv` and `variables[ "priv" ]` both resolved the private method.
Lucee resolves all three. Same for a function injected onto the instance
(`obj.fn = udf; obj.fn()`). Outside the component all three were correctly
unreachable on both engines, so the divergence was dot-vs-bracket *inside*.

Preside's object merger is exactly that shape. `Merger.cfc` merges same-named
objects from different source folders — a core mechanism, not an edge case — by
walking `getMetaData( this ).functions` and re-homing each one:

```cfml
target.$addFunction( func.name, this[ func.name ] );   // NULL for every private method
```

so `$addFunction( required string name, required function func )` threw
*"The parameter [func] to function [$addFunction] is required but was not passed
in"*. `PresideObjectServiceTest` test053/054/055 (`objectsWithMerging`) went
green → erroring, v0.636.0 → v0.655.0.

**The cause, and why it was two bugs.** #417 gated four read paths onto the
public view. It gated them **per opcode**, and an opcode does not know who is
reading — so one rule was wrong in *both* directions:

* `GetIndex` was pinned to the public view and denied the **insider**. That is
  #420.
* `op_get_property`'s `Instance` arm was left on the FULL view and still served
  the **outsider**. `c.secret` looked gated only because the compiler fuses a
  bare local into `LoadLocalProperty`; a receiver it cannot fuse never reached
  that path. `arr[ 1 ].secret` and `st.k.secret` both read a `variables`-only
  member on v0.656.1 — the private-scope leak #417 declared closed was still
  half open, 20 releases later.

The lesson is the same one #417's own commit message drew and then half-applied:
these are four paths to one question, and any answer that differs between them
is a bug in whichever path is asked. The discriminator has to be the *reader*.

**The fix.** `CfmlVirtualMachine::instance_member_view( inst, name, frame )`.
All four paths (`op_get_index`, `op_get_property`, `lookup_property`,
`lookup_property_opt`) now thread the reading frame through it: inside the
receiver's class → full view, outside → public surface. The insider test reuses
the existing `caller_is_within`, so it is the same rule the method-access gate
(GH #330) already applies — including that `private` is CLASS-level, so a sibling
instance and a subclass both qualify, and a closure minted inside a method reads
as an insider because it captures `this`.

**Why it costs nothing on the hottest op in the engine.** It asks the public view
FIRST and consults the caller only on a public MISS. That is sound because the
public view is a strict subset of the full one that agrees with it wherever it
answers at all (`get_public_member` is `this_members.get_ci` plus a
declared-private-method gate; `get_member` probes `this_members` first too), so
asking it first cannot change the answer — only who pays. And it does pay: past
the `Arc::ptr_eq` fast path, `caller_is_within` clones both class source paths to
compare them, and *reading another component's public members from inside your
own* is the commonest shape in any framework. That shape now pays nothing; only
a private read or a genuine miss (already headed for a native-parent probe or an
undefined-variable throw) reaches the comparison.

**Still divergent, deliberately.** `isDefined( "this.priv" )` from INSIDE a
component still walks the public view and answers `false` while the read
resolves. #417 reasoned about that path explicitly (and its commit corrects a
contaminated probe that had suggested Lucee answers `true` — measure it on an
untouched instance) so it is left alone here rather than changed as a side
effect. Tracked, not forgotten.

Tests: `tests/oop/test_component_member_view_by_reader.cfm` (19 assertions,
cross-engine) with `MemberViewFixture` / `MemberViewForeign` /
`MemberViewChild`. It pins both directions, the `getMetaData( this ).functions`
re-homing walk, the foreign-class negative, inheritance and closures — and it
FAILS on a pre-fix binary, which is the check that matters for a regression test.

Verified: Preside's real `Merger.cfc`, run against two fixture CFCs, throws the
reported error pre-fix and merges every method (private included) post-fix. CLI
8907/8907; serve dev cold+warm and `--production` ×3 at 9035/9035; baseline with
the change stashed byte-identical at 8888/8888. Wheels core suite on sqlite, as
an interleaved A/B over three runs each: pass=3361 fail=5 err=0 skip=23 on BOTH
binaries, same two spec names failing, six runs identical; wall 33.37 s baseline
vs 32.95 s fixed — inside the baseline's own 2.4 s spread, i.e. no measurable
change, which is what the public-first ordering was for.

## 89. Component construction: parent pseudo-constructors ran 2^depth times, constructor values were deep-copied, and every instance carried a self-reference (fixed v0.658.0) 📌

**Four divergences, one construction path, all probed against Lucee 7.1.0.204
on 2026-09-09 before fixing.** They surfaced while working the CFC-construction
performance plan, whose flat-class benchmark could not see the first two: the
plan measured an 86-method class with no parent at ~54 µs per `new`, but a
**1-method child of that class cost 170 µs and an 86-method child 343 µs** —
inheritance, the shape every framework object has, was 3–6× worse than the flat
case the plan was optimising.

1. **A parent's pseudo-constructor ran 2^depth times per instantiation.**
   `resolve_component_template_impl` resolved (and constructed) the parent once
   to build the child's injected scope, `resolve_inheritance_chain` then resolved
   it again for the merge — at every level. With a `writeOutput` in each
   constructor, `new Leaf()` on a three-level chain logged
   `Root,Root,Mid,Root,Root,Mid,Leaf,Root,Mid,Root`. Lucee logs `Root,Mid,Leaf`.
   Any side effect in a parent constructor (a log line, a counter, a
   registration) repeated on every subclass instantiation. The static-scope
   block constructed the parent a further time on a class's first sight.
2. **The finalize deep-copied constructor-assigned values.** A constructor
   `variables.cfg = request.cfg` handed the instance a private copy, so a later
   `variables.cfg.x = 2` never reached `request.cfg` (Lucee: it does — CFML
   structs are references). Same for `this.cfg2 = request.cfg`. The copy existed
   only to detach the template from an alias it should never have had (below).
3. **A `component name="X"` file left its template in page globals.** The body's
   codegen ends with `StoreGlobal(<name>)`; nothing removed it. The resolver's
   own globals lookup then served that stale template to the next `new X()` —
   **no constructor run, `this` state shared between instances** — and a page
   variable called `X` was clobbered. (For the common unnamed `component {}` the
   key was `Anonymous`, which is why `isDefined("Anonymous")` was true on every
   page that had constructed one.)
4. **The body's class-name local leaked into `variables`.** Inside a component,
   `structKeyList(variables)` listed `Anonymous` — the template struct, i.e. a
   reference to the instance itself. Lucee has no such key. This is the
   per-instance reference cycle the collector has been paying for since the
   flyweight landed.

**The fix.** (1) The resolver keeps the parent it resolved, stashes it on the
child template under a hidden key, and `resolve_inheritance` merges against it
(`merge_child_onto_resolved_parent`, split out of the chain walk) instead of
resolving again; the static block reads the parent's store from `static_stores`
via that same resolved parent. Every pseudo-constructor now runs exactly once,
root first. (2)(3) The finalize snapshots the candidate global keys before the
body, TAKES the template out of globals afterwards (restoring any prior page
value), and no longer deep-copies either the template or the `variables`
values — nothing aliases them any more. (4) The class-name local is filtered out
of the captured body locals. `__variables` is now attached even when empty — it
was only ever non-empty before because of the leak, and an empty `component {}`
stopped being recognised as a component without it.

**Performance.** Two further changes rode along because the same profile showed
them: a new `DefineComponentMethods` op replaces the six-op-per-method sequence
the constructor body executed to attach each method (`DefineFunction` +
`StoreLocal`, `LoadLocal` + `DefineFunction` + `SetProperty` + `StoreLocal`), and
the inheritance merge's linear case-insensitive key scans and per-method
`remove_ci` (a `shift_remove`) became single hash probes. Same box, same session,
CLI, N=3000 warm, µs per `createObject().init()`:

| shape | v0.657.0 | v0.658.0 | Lucee 7.1 |
|---|---|---|---|
| 1 method | 5.3 | 4.1 | 1.2 |
| 20 methods | 15.8 | 8.3 | 2.0 |
| 86 methods | 54 | **24** | 6.1 |
| 200 methods | 119 | 53 | 18.3 |
| 1-method child of an 86-method base | 168 | **52** | — |
| 86-method child of an 86-method base | 343 | **86** | — |
| 60,000 live instances, peak RSS | 2.11 GB | **272 MB** | — |

The memory figure is the leaked self-reference plus the doubled parent
constructions plus the deep copies, all of which were retained per instance
until request end.

Tests: `tests/oop/test_component_construction_semantics.cfm` (19 assertions,
cross-engine, fixtures in `tests/oop/ctorsem/`) — 19/19 on RustCFML and Lucee,
**11/19 on the v0.657.0 binary** with the doubled constructor logs visible in
the failure output.

## 90. Component construction no longer scales with method count: a class's method tables are attached, not rebuilt, from its second construction on (v0.659.0) 📌

Follow-on to §89, which left the flat 86-method `new` at 24 µs with ~20 µs of it
spent materialising the 86 method entries into three per-instance maps (the
constructor's `variables` seed, the template `this`, the assembled `variables`)
and then stripping them all into the shared per-class table that already
existed from the first instance. Subclasses paid the same again in the parent
hand-off: `all_entries()` on the parent (quadratic in the method count), the
`super` struct rebuilt from those entries, and the merge's
`snapshot_with_methods()` of the parent — all class-invariant, all per instance.

**The change ("replay").** A class's first construction in a request is
unchanged and builds the tables as before. Every later construction of that
class (`class_method_tables` has its source file) attaches instead of copies:
the constructor's `variables` seed carries the class's shared `variables` table;
`DefineComponentMethods` attaches a per-class OWN-methods table to the template
`this` and writes nothing per method; the parent hand-off reuses a cached
`__is_super` struct and stages only the parent's data members; the merge copies
the parent's data and bookkeeping keys and takes the super struct from the
template. Class-invariant metadata (`__extends_chain`, `__super_map`, …) is now
shared in every mode, not production only — the cache lives on the per-request
VM, and a class cannot change within a request.

**Two things the first attempt got wrong, both caught by existing tests.**
Attaching the class's FINAL table (own + inherited) to the template `this` made
`getComponentMetaData(child).functions` list the parent's methods, and the leaf
metadata builder read the template map only, so it then listed nothing. Hence the
separate own-methods table, and the leaf builder now snapshots with methods.
Staging the parent's methods onto the child's `this` on a first construction
(for `structKeyExists(this, "inherited")` parity with Lucee, which answers true)
polluted the same metadata and was reverted — that read stays `false` on both
paths, a small pre-existing divergence, recorded in `planning/`.

Same rig as §89, µs per `createObject().init()`, best-of-4 warm on the Lucee arm:

| shape | v0.658.0 | v0.659.0 | Lucee 7.1 |
|---|---|---|---|
| 1 method | 4.0 | 3.4 | 0.57 |
| 86 methods | 23.2 | **4.0** | 5.9 |
| 200 methods | 51.9 | **4.6** | 15.5 |
| 1-method child of an 86-method base | 53.6 | **8.3** | 7.35 |
| 86-method child of an 86-method base | 84.3 | **8.8** | 14.0 |
| 60,000 live instances, peak RSS | 272 MB | 272 MB | — |

Construction cost is now flat in the method count and at or below Lucee for
every shape but the trivial 1-method class, whose ~3 µs fixed cost (compile
lookup, program swap, blueprint lookup, `init()` dispatch, Instance partition)
is the next target. The replay is per request: a class constructed once per
request still pays the full first-construction path, so the follow-on for
Preside-shaped workloads is caching the class-level result across requests,
keyed to the bytecode-cache entry.

Tests: `tests/oop/test_component_construction_semantics.cfm` grew to 26
cross-engine assertions, adding a replay block — metadata, public key list,
3-level `super` dispatch, override resolution and per-instance isolation must be
identical between a class's first and later constructions. 26/26 on RustCFML,
Lucee 7.1 and the v0.658.0 binary (the replay must be unobservable).

## 91. `cachePut`/`cacheGet` and component `static` scopes lived for one request (fixed v0.660.0) 📌

Found by auditing which caches live on the per-request VM (every serve request
gets a fresh VM) versus the shared server state. Nearly all class-level and
execution caches are per request — an accepted trade-off — but two things on
that list are not caches, they are state that must outlive the request:

- **The object cache.** `cachePut("k", v)` in one request; `cacheGet("k")` in
  the next returned null. The store was a plain map on the VM, so in serve mode
  the cache functions were inert: anything relying on them for cross-request
  caching silently recomputed every time. The store is now
  `ServerState::object_cache`, shared for the server's lifetime; the VM holds
  the shared handle when serving and a private map under the CLI.
- **Component `static` scopes.** `static { hits = 0; }` with a method doing
  `static.hits++` read 1 on every request; Lucee and ACF keep a class's static
  scope for the application's lifetime. The per-request store stays as the
  fast path, but it is now seeded from and written through to
  `ServerState::static_scopes`, keyed by source file. Each entry records the
  compile generation (the CFC `__main__`'s process-unique `global_id`), so
  editing the file in dev mode — a new compile, a new id — gives it a fresh
  static scope, as a redeploy would on the JVM engines.

Tests: `tests/oop/test_static_across_requests.cfm` and
`tests/stdlib/test_cache_across_requests.cfm`. Both need three separate
requests to observe, so they drive a target page over HTTP on `cgi.server_port`
and report a single skip under the CLI runner.

## 92. v0.659.0 regression: an Application.cfc extending a parent lost the parent's methods (fixed v0.660.1) 📌

Preside failed to boot on v0.659.0/v0.660.0: *"Variable '_pingCheck' is
undefined"* from `Bootstrap.cfc`'s `onRequestStart`, then the same for every
other Bootstrap helper. The Application.cfc path builds the application
component itself and merges it against its parent by resolving that parent a
**second** time in the request — so the parent arrived as a replayed template
(§90). A replayed template's finalize built a fresh `__variables` struct and
relied on `share_methods_into_table`, later in `resolve_inheritance`, to attach
the class table — but a template merged as a raw parent never gets there. Its
`variables` then held the methods in neither the map nor a table, and the merge
copied nothing. A replayed template now carries its `variables` table from the
finalize, and the merge folds table methods when the child is not itself a
replay. Test: `tests/oop/test_appcfc_extends_parent_methods.cfm` with its own
`tests/appcfc_parent/Application.cfc` (serve mode).

## 93. A class's construction tables were rebuilt by every request — the replay path (§90) never fired on a page that constructs each class once (fixed v0.661.0) 📌

§90 made the second and later constructions of a class in a request cheap by
attaching its shared method tables instead of rebuilding them. Those tables,
the class's own-method table, its `super` struct and its flyweight blueprint
all lived on the per-request VM, and serve mode builds a fresh VM per request.
So a page that constructs most of its classes once — a Preside render, a
framework boot — paid the full first-construction price for every class on
every request and never reached the replay path at all. Measured in serve
mode (`--production`), one construction of each shape per request, median of
300 requests:

| First construction in a request | v0.660.1 | v0.661.0 |
|---|---|---|
| 86-method class | 64 µs | **8 µs** |
| 200-method class | 125 µs | **9 µs** |
| 1-method child of an 86-method base | 66 µs | **11 µs** |
| 86-method child of an 86-method base | 189 µs | **19 µs** |
| 1-method class, constructed after the others | 6.9 µs | **4.1 µs** |

The tables now live on `ServerState::class_caches`, keyed by source file. A
request adopts a class's entry into its per-request maps when it resolves the
class, and publishes what it builds. Each entry carries a **chain generation**:
the CFC's own compile generation (the `__main__` `global_id`, as the static
scopes use) folded with its parent's. A child's `variables` table holds the
parent's methods too, so keying on the child's own compile would have kept
serving a stale table after a parent edit; with the chain generation an edit
anywhere in the `extends` chain misses and rebuilds on the next request, in
dev mode exactly as the bytecode cache does.

Two further per-construction costs fell out of the same profile:

- Every program swap re-registered each of the program's functions into the
  VM's function registry, on every `new` of the class. A program is registered
  whole, so its first function's id now marks it registered for the request.
- Component resolution walked **every** page global (the builtins included)
  with a case-insensitive comparison, twice per construction, looking for a
  `component name="X"` template. The finalize now takes the template out under
  the key the body filed it under — the declared `name=` or "Anonymous", read
  from the body's bytecode alongside `__extends` — so it never looks. The
  resolver's entry walk runs only while page globals hold at least one such
  template (a count `StoreGlobal` and the finalize maintain), which is
  normally never.
- The flyweight blueprint lookup formatted a `(source file, name)` string key
  on every construction; the file's first name is now filed under the bare
  path and probed without allocating. The template's reserved keys are
  inserted through the pre-hashed well-known `Key`s.

Trivial 1-method class, CLI loop of 300k, ns per construction: `createObject`
2,850 → **2,320**, `new` 3,200 → **2,600** (Lucee 570). What remains is spread
across the marker struct the body builds, the body frame's scope set-up, and the
instance partition — map inserts and allocations with no line above 1%.

Tests: `crates/cfml-vm/tests/class_cache_across_requests.rs` (adoption is
observable on the server state, the adopted instance is indistinguishable, and
a parent-only or child-only edit is visible on the next dev-mode request) and
`tests/oop/test_class_cache_across_requests.cfm` (serve mode, three requests
against a 3-level chain). Wheels core and the TestBox suite are identical per
spec to v0.660.1.

The ~12 µs a request's *first* component resolution pays (the trivial class
costs 16 µs constructed first and 4 µs constructed last) is spread across
first-touch growth of per-request maps and registries; no single line
dominates, and it is not addressed here.

## 94. Inside a subclass pseudo-constructor `this` did not show inherited methods; a relatively declared `extends` chain was never package-qualified; `structAppend`/`structKeyList` over a component were quadratic; every CLI start re-hashed the installed extensions (fixed v0.662.0) 📌

Four items left open by the construction work (§89–§93), each confirmed
against Lucee 7.1 before the change.

**Inherited methods on `this` during the body.** Lucee runs the parent chain's
pseudo-constructors first on the same `this`, so inside a child's body
`structKeyExists(this, "inheritedMethod")`, `isDefined("this.inheritedMethod")`,
`isNull(this.inheritedMethod)`, `f = this.inheritedMethod` and
`structKeyList(this)` all see the parent's methods. Here `this` was the bare
template — own methods only — on the first construction and on a replay (§90)
alike, so a body reading an inherited method by reference threw "Variable
'rootFn' is undefined". §90 had tried staging the parent's methods as entries
and reverted it because `getComponentMetaData(child).functions` then listed the
parent's. The template `this` now carries the class's FULL method table for the
body's duration only (on a replay the class's own shared table; on a first
construction the parent chain's methods, the same set the `super` struct is
built from) and the finalize puts it back — own table on a replay, none on a
first construction — before the inheritance merge and the metadata builders
read it. Leaf metadata still lists only the class's own functions. The same
key-list read inside a body also exposed the engine's `__name`/`__extends`
entries (the filter recognised a component by `__variables`, which a template
under construction does not have yet); it recognises `__name` now, the test
`isInstanceOf` already applied.

**Relative `extends` and `isInstanceOf`.** Lucee's `isInstanceOf(x, "pkg.Root")`
matches every class in the chain by its mapping-qualified full name. A chain
declared with relative names (`pkg/Leaf.cfc extends="Mid"`, `Mid extends="Root"`)
stored the raw `extends=` spelling, so `isInstanceOf(leaf, "pkg.Root")` was false
and `getMetadata(leaf).extends.name` was `Mid` where Lucee gives `pkg.Mid`.
The name a template gets (#229/#237's qualification) is now decided BEFORE its
parent is resolved and handed to the parent's resolution as an explicit anchor:
an unqualified parent found beside the child's file takes the child's package
(`pkg.Mid`), a parent elsewhere goes through the existing webroot derivation,
and each level anchors the next, so `pkg.Root` follows. `__extends_chain` is
built from the parent's resolved name rather than the `extends=` text. Probe on
both engines, `new pkg.Leaf()` from a page: `isInstanceOf` `pkg.Root`/`pkg.Mid`
true, `wrong.Root` false, `extends.name` `pkg.Mid`, `extends.extends.name`
`pkg.Root`.

**`structAppend(plain, cfc)` / `structKeyList(cfc)` were quadratic in the
method count** (GH #402's actual per-call cost). The flyweight instance's public
member walk re-scanned the collected keys with `eq_ignore_ascii_case` for every
class method, and the marker-struct table walks (`all_keys`, `all_entries`) did
the same — although `Key` already hashes and compares case-insensitively, so the
map probe alone decides shadowing. The source's pre-hashed keys are also kept
through the append instead of being re-interned from `String`s. Same box,
20k iterations, best of 5:

| `structAppend` | v0.661.0 | v0.662.0 | Lucee |
|---|---|---|---|
| plain ← 86-method CFC | 12.1 µs | **8.2 µs** | 2.9 µs |
| plain ← 86-method child of an 86-method base | 30.1 µs | **16.3 µs** | 6.2 µs |
| plain ← 86-key plain struct | 4.7 µs | **3.0 µs** | 2.1 µs |

What remains is the member walk itself: a snapshot of the instance's data map
plus a per-method `to_ascii_lowercase` for the access lookup and one `Key` insert.

**Extensions were inflated and SHA-256'd on every start.** `stage_library`
read the library out of the `.rcx` zip and hashed it to find its
content-addressed cache directory — even when that directory already held the
staged library from an earlier start. With the 38 MB browser extension installed
that was **0.30 s of CPU per CLI run** (`rustcfml trivial.cfm`: user 0.32 s →
0.00 s), i.e. the whole cost of a small script. The manifest's digest IS the
cache key, so when `~/.rustcfml/ext-cache/<digest>/<lib>` exists the archive is
not opened; only a manifest without a digest still takes the full path. An
archive with no `cfml/` half is remembered with a `.no-cfml` marker so its
central directory is not walked every start either.

Verification: CLI runner 8946/8946; served dev and `--production`, cold and
warm, 9088/9088; `cargo test --workspace`; wasm32 and wasm-pack builds; Wheels
core 2737/3/0/16 and TestBox own suite 415/0/0/22 with zero per-spec status
changes against v0.661.0. Trivial-class construction loop unchanged
(2,330 ns both). Tests: `tests/oop/test_component_construction_semantics.cfm`
(inherited visibility on the first and a replayed construction, qualified
`isInstanceOf`/metadata names, engine-key-free `structKeyList(this)`), green on
Lucee 7.1 too.

## 95. A CFC method call cost 3-4x Lucee's and grew ~50 ns per declared parameter — the frame's bookkeeping, not its body (v0.663.0) 📌

With construction at or near Lucee (§89–§94), the method CALL is where CFC
time goes: frames were 71% of a Preside request's CPU. A `call-phases` split
of `o.m1(1)` said the prologue phases were small and everything sat in the
"body" of a `return 1;` method, i.e. in the dispatch and return machinery the
marks did not cover. Line-level sampling (samply, `line-tables-only`) on the
instance-method dispatch and the frame prologue found the cost was almost
entirely allocation and re-hashing that Lucee never does:

- **Dispatch (`call_instance_method_impl`)** built its 3-entry parent map from
  `String` keys (two interns + a grow per call), read the instance lock twice,
  probed for `onMissingMethod` on every direct dispatch, swapped `source_file`
  with two `String` clones — which `call_function` then did AGAIN — and copied
  the argument `Vec` once more with `drain().collect()`.
- **Frame prologue.** Every declared parameter was inserted into a per-frame
  hash table of "inherited or param" names and, on the lazy `arguments` path,
  into a second `HashSet<Key>`; both allocated and re-hashed on growth every
  call, and each supplied value was cloned instead of moved. A 35-parameter
  method spent a quarter of its call there. The frame then copied the
  dispatch's parent map entry by entry into its own scope map through the
  carry filter.
- **Stack record.** `function_name`, `called_name` and `template` were three
  `String` clones per frame, plus `method.to_string()` at the dispatch.
- **Return.** Every method return scanned all declared params for a
  `CfmlValue::Component` by-reference write-back — a variant nothing in the
  engine constructs any more.
- **Named arguments** (`argumentCollection`) lower-cased every collection key,
  cloned every name, built a `HashSet<String>` of the explicit names and
  matched names to parameters with a quadratic case-insensitive string scan
  (35 params: ~600 compares).

**What changed.** The dispatch snapshots the instance once, tests
`onMissingMethod` only on the fallback paths, and hands the frame its scope
map READY-MADE from the pool (`pending_instance_frame`): the frame adopts it
instead of seeding a second one, and — since the flyweight instance's scopes
are live references — skips the `this`/`variables` return write-backs the
dispatch was discarding. Declared parameters are tracked as a bit per index
over the function's shared `param_keys` (`InheritedKeys::track_param`), the
first few non-structural inherited keys live inline, and the lazy supplied
set is a `u64`; argument values are moved out of the call's `Vec`. Call frames
record `Arc<str>`s (`BytecodeFunction::name_arc`/`source_file_arc`, the
`CallMethod` op's own key for the called name) and the VM's `source_file` is
an `Arc<str>`. Named-argument reordering keeps the collection's pre-hashed
keys, matches parameters by folded hash with a next-in-order guess, and
compares the (usually empty) explicit-name list directly. `__static` is a
structural inherited key, which keeps a method frame's inherited set
bits-only. The dead `Component`-variant scan returns early.

Same box, ns per call, CFC method `function mN(p0..pN) { return 1; }` called
300k times; Lucee 7.1 warm, best of 4:

| shape | v0.662.0 | v0.663.0 | Lucee |
|---|---|---|---|
| `o.m1(1)` | 534 | **323** | 126 |
| `o.m8(…)` | 899 | **488** | 199 |
| `o.m35(…)` positional | 2,209 | **1,069** | 518 |
| `o.m35(argumentCollection=ac)` | 3,914 | **1,473** | 1,421 |
| bare `leaf(i)` from a sibling method | 419 | **347** | 136 |
| `this.leaf(i)` | 563 | **335** | 133 |
| `variables.dep.m1(i)` | 585 | **363** | 155 |

Per declared parameter: ~50 ns → ~22 ns positional. Construction loops are
unchanged. What remains is the frame itself (operand stack, slot vector,
scope map insert per param, `frame_ctx`/call-stack pushes, pooled map
recycle) and the two hash probes of the bare-name lookup — no single line
above 3%.

**Only a PLAIN class method takes the ready-made frame.** A method injected
from another CFC — a TestBox custom matcher, a Wheels controller mixin — is a
UDF value carrying its defining CFC's captured scope, which the general seed
merges into the frame; the first cut skipped that and turned 6 TestBox and 3
Wheels specs red. Those dispatch the general way. Probed on Lucee 7.1 while
writing the regression test (`tests/oop/test_injected_method_frame.cfm`): an
injected plain UDF binds to the component it is INVOKED on (its `variables`
are the target's — both engines agree), and an injected CLOSURE keeps its
captured locals (both agree) but Lucee ALSO keeps `variables` bound to the
DEFINING component where we bound it to the target. That closure binding
divergence is fixed in v0.664.0 (§96).

**Tried and dropped:** seeding a bare sibling-method call's frame the same
owned way. It broke the private-method access gate and custom-tag `thisTag`
inheritance (the general carry filter does more than copy structure there)
and measured no faster; the structural `__static` alone gave the bare path its
gain.

Verification: CLI runner 8946/8946; served dev and `--production`, cold and
warm, 9088/9088; `cargo test --workspace`; wasm32 and wasm-pack builds;
Wheels core 2737/3/0/16 and TestBox own suite 415/0/0/22 with zero per-spec
status changes against v0.661.0.

## 96. A closure dispatched on another component was re-bound to it — Lucee keeps a closure lexically bound to the component that defined it (fixed v0.664.0) 📌

Found while writing §95's regression test and probed on Lucee 7.1 across every
call shape before changing anything. Lucee's rule is total: a closure or arrow
function keeps the scopes of the component it was DEFINED in — `variables`,
`this`, unscoped reads AND writes — wherever it is stored and however it is
invoked: `t.clo()`, `this.clo()` inside a method, a bare `clo()`,
`variables.clo()`, held in a plain struct and called as `s.f()`, nested
closures. A closure defined at page level and stored on a component sees the
page's `variables` and has NO `this` at all. A PLAIN UDF reference behaves the
opposite way and re-binds to the component it is invoked on (its `variables`
are the receiver's). A closure never sees its caller's locals or `arguments`,
and does see its defining frame's later declarations and mutations.

We matched the UDF rule and the `variables.clo()` path, and re-bound closures
everywhere else: `strip_instance_binding` removed the captured
`this`/`variables` on instance dispatch (so the receiver's won), the bare-call
and bare-read paths rebuilt a "foreign-bound" function's env around the
current component (the guard assumed a "genuine closure" captures no
`this`/`__variables` — ours capture their defining method frame, which has
both), and the fused frame seed carried the caller's structural scopes into a
closure's frame (which is how a page closure acquired a `this`). Each of those
three now applies only to a plain UDF value (`is_closure_value`: the
`__closure_`/`__arrow_` expression names), and a lexical callee's frame takes
its structural scopes from its captured env alone (`FusedParentPlan::lexical`).

Test: `tests/oop/test_closure_lexical_binding.cfm` — 23 assertions covering
all the shapes above, every expectation read off Lucee first; green on both
engines. Wheels core and TestBox identical per spec (WireBox's injected
`buildProviderMixer` provider and Preside's `decorated.onMissingMethod =
this.onMissingMethod` are plain UDF values and still re-bind).

## 97. Inside a closure, a `var` local sharing its name with a captured variable was overwritten after every nested closure call — a `for (var i…)` loop ran once, or never ended (fixed v0.665.0) 📌

Found by a cross-engine microbench that would not finish: a closure defined
inside another closure, called in a loop, with a 1,000-element array built
earlier on the page. The array was a red herring — the loop that built it
left a page-level `i` behind. After ANY closure call inside a closure the
frame reconciled its own captured env back into its locals
(`reconcile_closure_env_into_locals`, there so a closure's write to an
enclosing variable is visible when it was invoked behind an intermediate CFC
method frame), and it did so for every env key whose value differed from the
local — including keys the frame had declared with `var`. A `var i` in a slot
is never forward-synced to the env, so the env still held the enclosing
scope's `i`, and the loop counter was reset to it after each nested call: with
page `i = 5000` the loop ran once; with page `i = 201` it never terminated;
with page `t = 999999` the closure's running total became 1,000,001. A nested
UDF call did not trigger it. Identical on v0.661.0, so long-standing, and the
shape — `describe(function(){ … it(function(){ for (var i…) … }) })`, a
`for (var i…)` around a callback inside an `arrayEach` — is common.

The fix has to keep the case the reconcile exists for: `var groups` declared
BEFORE the closure and set inside it must still come back (two existing
suites cover it). The rule is value-based: a declared local or parameter is
skipped only when the env's value still equals what the enclosing scope
holds for that key — i.e. the env never saw this frame's variable and is
merely stale. A genuinely mutated local no longer matches and is reconciled,
including `var i = 0; each(function(){ i++ })` with a page `i` present. Test:
`tests/core/test_closure_var_shadows_captured_after_nested_call.cfm`.

## 98. `structKeyList`/`structKeyArray` over a component were quadratic in the method count, like §94's `structAppend` (fixed v0.665.0)

`instance_public_keys` copied the instance's data map and re-scanned the
collected keys case-insensitively per class method. It reads under the lock
and probes the own map (not the class table, which would report every method
as shadowed) instead. 86-method class: 8.9 → 3.3 µs (Lucee 2.1).

## 99. A page's `variables` scope listed every builtin (754 members on a page with three variables), and one `variables.x = …` made every later call from that page 70x slower (fixed v0.666.0) 📌

Found while sizing call costs against Lucee: a page-level UDF call measured
800 ns on a clean page and 59 µs after a single explicit `variables.x = 1`
(or `variables["x"] = 1`), growing with the number of page variables. Two
defects behind one mechanism:

- The page-scope `variables` view was built as a clone of the VM's globals
  map plus the frame's locals, and the globals map is also where every
  builtin and engine-registered function, the other scopes (`cgi`, `url`,
  `form`) and a component template awaiting its finalize live. So
  `structCount(variables)` was 754 where Lucee says 3, `structKeyExists(
  variables, "arrayLen")` and `isDefined("variables.arrayLen")` were true,
  and `structKeyList(variables)` began with `$sioBroadcast`.
- A scoped store (`variables.x = …`) compiles to load-the-view, set the
  member, store-the-view, and the store spliced EVERY entry of the view back
  into the page frame as an ordinary local. From then on the frame held ~750
  extra keys, and the fused seed carried all of them into every callee frame
  (counted: 750 caller keys per call, against 4 before).

The view now includes only members of the scope (no builtin function
entries, no `__` engine keys, no scopes, no templates), and the store splices
back only entries whose value differs from the page's current one. Verified
on Lucee 7.1: `structKeyList(variables)` on a page with `a`, `b` and a UDF
is exactly those three, four after `variables.c = 3`. Test:
`tests/core/test_page_variables_scope_members.cfm`, green on both engines.

Still open from the same sizing: a UDF called from a PAGE frame copies the
page's variables into its frame on every call (the template-caller carry
filter is "all"), so page-heavy code pays O(page variables) per call where
Lucee walks a reference chain. That is the next frame lever, together with
closure calls (10x Lucee) and higher-order functions (7.7x).

## 100. A call from a page copied every page variable into the callee and diffed it back; a closure call copied its whole captured env in and out; `variables.x` in a method cost 9x Lucee (fixed v0.667.0) 📌

The three frame levers left open by §99, sized against Lucee 7.1 on the same
box (warm, ns per call):

| shape | v0.666.0 | v0.667.0 | Lucee |
|---|---|---|---|
| page UDF call, 5 page variables | 1,406 | 456 | 196 |
| page UDF call, 50 page variables | 5,311 | 456 | 196 |
| page UDF call, 200 page variables | 21,494 | 456 | — |
| page UDF call, 1,000 page variables | 461,057 | 456 | — |
| page closure call | 1,419 | 451 | 146 |
| closure defined inside a closure | 1,053 | 547 | 175 |
| `arrayMap` per element | 728 | 315 | 153 |
| `variables.t += variables.i` in a method (per iteration) | 553 | 193 | 60 |
| `s &= "x"` (100k, per append) | 2,681 | 1,020 | 1,382 |
| `structKeyExists` | 173 | 154 | 105 |
| string BIF trio (`len(ucase(replace(…)))`) | 288 | 226 | 151 |

Three structural changes, each verified against Lucee before it was kept:

**The page `variables` scope is one shared struct.** A `__main__` frame that
inherited no `__variables` handle from its launcher gets a fresh one at entry,
and the frame reads and writes its variables through it — the routing every
component-scope frame already used. Before, the page frame's locals map WAS
the scope, so the call-parent seed carried every page variable into each
callee (the template-caller filter is "all") and the return path diffed them
back: O(page variables) per call, and worse than linear at 1,000. A callee is
now seeded with the one structural key and resolves page variables through it
exactly as a CFC method resolves its component's `variables`; an `include`d
template, a custom tag body, an `evaluate()` frame and a REPL line arrive with
a handle already seeded and share it. `localMode="modern"` is forced off for
a page frame (there is no `local` scope for a bare write to land in). Two
things that only worked because the page scope used to be a copy of the
globals map were fixed along the way: `setVariable("variables.x", …)` /
`setVariable("x", …)` and `<cfparam name="…">` with a variables-scoped or
runtime name wrote `self.globals` — they are delivered into the caller's frame
through the `queryExecute(result=)` channel and land on the handle. The page
frame's own loads and stores take a fast path (one pre-hashed probe on the
handle) keyed off a per-`Name` reserved-word flag computed at intern time; a
bare page loop is 180 ns per iteration against 113 before and Lucee's 60-77,
the one regression, taken for the flat call cost.

**A lexical closure references its env; it does not copy it.** The frame
holds the captured (defining) env under a reserved `__closure_frame_env__`
carrier; a bare name that misses `locals`/`arguments` walks that env and its
parent links LIVE, and a classic-mode write to a captured name updates the env
level that owns it — Lucee's closure scope chain. Retired per call: the copy
of every captured key into the frame, `refresh_env_from_parent_chain` (which
re-copied the ancestors' values into the env before every call, allocating a
`String` per key), and the return-time diff that wrote mutations back. The
defining frame still reconciles its env into its locals after a call, but
only when the env's version changed. The caller's carrier is never carried
into a callee — that clobbered a nested closure's own env, so it resolved its
captured names through the wrong scope — except into a STRIPPED closure (a
var-scoped function expression fetched from an env: the recursive `var fact =
function(n){ … fact(n-1) }`), whose lexical scope IS the caller's chain.
Semantics probed on Lucee 7.1 and matched on nine shapes (new-name writes go
to the defining scope's `variables`, not the closure's `local` nor the
enclosing function's; a captured var-local is updated in place; a deferred
closure sees the variable's last value; a parameter shadows a same-named
page variable; the definer's local beats the caller's; `arrayEach` callbacks
mutate the enclosing function's local): `tests/core/test_closure_scope_chain.cfm`.
A cfthread body written in a CFC method used to receive the component's
methods flattened into its env as loose entries; it now gets the scope as a
per-thread snapshot struct under `__variables`, so `variables.helper()` and a
sibling's sibling call resolve as they do in the method.

**`variables.x` reads and writes are one op at any depth.** `LoadVariablesKey`
(previously a page-only peephole) is emitted inside function bodies too and
resolves off the frame's `__variables` handle first; a new `StoreVariablesKey`
replaces load-handle + `SetProperty` + store-handle for `variables.x = …`,
`variables.x += …` and `variables.x++`. The old sequence allocated a fresh
`"__variables"` key string per write and stamped the `this` alias per read.

Smaller levers found in the same profiles: the env ∪ locals composition the
`Call`/`CallNamed` arms materialized as a merged copy before the frame seed
copied it again (~20% of a closure call) is folded into the seed; a
compiled-in builtin's entry point is cached on its interned `Name` after the
first call (the per-call hash into the builtin index — and, with any `.rcx`
extension installed, a SipHash probe of the extension table — was ~10% of a
small string BIF); `&` builds its result with one allocation instead of three
plus `format!`; `structKeyExists` is one probe instead of a case-insensitive
scan plus four; a callee's positional-overflow `arguments` keys (`"1"`, `"2"`
— every higher-order callback) come from a pre-interned table; a member write
that stores the very handle a variable already holds is skipped.

Gates: CLI runner 9,036/9,036 (41 new assertions in
`tests/core/test_page_scope_shared_struct.cfm` and
`tests/core/test_closure_scope_chain.cfm`), TestBox 415/0/0 (+22 skipped) and
Wheels core 2,737/3/0 unchanged at every step. The CFML runner caught three
closure regressions the two framework suites did not (the carrier clobber,
the stripped-closure chain, the cfthread flattening) — keep it in the gate.

Still open: the generic frame cost (a UDF-to-UDF call is 306 ns against
Lucee's 155; profiles are flat — dispatch, param binding, the `arguments`
struct, frame teardown), `queryAddRow` at 450 vs 232, struct key read/write at
248/330 vs 149/140 (the key string is built per access on both engines; ours
still allocates it twice), and the page bare-loop regression above.

## 101. The page bare loop got slower in v0.667.0; a classic method reading an unscoped component variable cost 2.2x Lucee; struct literals allocated every key twice; `queryAddRow` was 1.9x (fixed v0.668.0)

Follow-up to §100, taking its "still open" list in the order "where we got
slower first". Warm ns per operation; Lucee 7.1 on the same box (the box
carried ~12% external load during the v0.668.0 runs, so its absolute numbers
are slightly pessimistic — the unchanged `var`-local loop moved 57 → 64):

| shape | v0.667.0 | v0.668.0 | Lucee |
|---|---|---|---|
| page bare loop (`t += i`, no calls), per iteration | 180 | 105-111 | 60-77 |
| classic method, unscoped `t`/`i` routed to `variables`, per iteration | 229 | 111 | 104 |
| `variables.t += variables.i` in a method, per iteration | 193 | 153 | 60 |
| page UDF call, 50 page variables | 456 | ~400 | 196 |
| struct key write (`st["w" & k] = i`) | 331 | 243 | 140 |
| struct key read | 248 | 211 | 149 |
| `queryAddRow(q, {id: i, name: "n" & i})` | 450 | 345 | 232 |

**One frame scope cache.** Every plain-variable load, store and fused loop
op on a page frame or a component frame (a CFC method, a UDF called from a
page) goes straight to the frame's `variables` handle, fetched once per
change of the locals map: `FrameScopeCache` is validated by the map's
`version()`, which bumps on every insert and removal, so a replaced
`__variables` or `arguments` can never leave a stale handle in use. The fast
path takes the CFML resolution order into account rather than shortcutting
it: it is skipped when the frame carries a closure-env carrier (captured
names come first) or when the arguments scope holds a key beyond the declared
parameters (extras come first), and a later mutation of the arguments scope
shows up as a length change. A method referenced by bare name as a callback
(`items.each( record )`) still takes the generic path, which binds the
receiver at the load site. Before: the page frame's loop paid two probes per
access to find the handle it had just used; a method's unscoped read walked
locals → arguments (with a redundant case-insensitive scan of the whole
arguments scope on every miss — the keys already fold case) → the closure
chain → four web-scope compares → `__variables`.

**Fewer wasted probes.** The scope-name comparison chains in LoadLocal and
StoreLocal are gated on the per-`Name` reserved-word flag; the fused
`JumpIfLocalCmpConstFalse` resolves a counter that lives behind the handle
once and takes its numeric arms instead of the generic CFML comparison; a
member write's handle store-back (`st[k] = v` → `StoreLocal st`) is answered
with read probes at the top of StoreLocal when the variable already holds the
handle; a lexical closure whose env holds no data key and no parent link
(a page-level closure: env = `__variables`) gets no carrier, so its frame
takes the direct paths too.

**Struct literals.** `{ id: i, name: n }` compiles to `BuildStructStatic`
with its keys interned at compile time; `BuildStruct` popped each key as a
runtime String and built a `Key` from it — two allocations per key per
evaluation. Computed keys keep the pair form. `queryAddRow(q, struct)` moves
a uniquely owned literal's map into the row instead of cloning it, the row
insert probes each column once (the row's keys fold case), and the deferred-
intercept name check is a binary search instead of a scan of ~150 names on
every intercepted BIF call.

Gates: CLI runner 9,041/9,041, serve dev+prod cold+warm 9,183/9,183,
`cargo test --workspace` 714 passed, wasm32 + wasm-pack, TestBox 415/0/0 and
Wheels 2,737/3/0 unchanged. Two regressions were caught on the way, one by
each side of the gate. The runner: the direct read path handed a raw
method-table entry to a callback (`this` undefined inside `record`) — Function
values now fall through to the binding path. Wheels (mapperModernSpec, 9
specs): the write-back shortcut treated "some scope holds this handle" as
"the target variable holds it", so `local.routes = this.getRoutes()` — the
very array `variables.routes` holds — never created the local. The shortcut
now consults the scope handle and closure chain only for a name the store
would route there (classic localmode, not `var`-declared, not a parameter);
`tests/oop/test_local_alias_of_variables_member.cfm` pins the shape.
A third was caught by booting Preside (the ModuleService `each()` callback
that does `appRouter.getModuleRoutes( moduleName ).append( item )`): the
guard that checks what a variable held before a mutating member call writes
its result back used a runtime load that did not know the closure chain, saw
nothing for the captured `appRouter`, and let the array be written over the
captured component in the shared env — "The function [getModuleRoutes] does
not exist in the Array" on the second route. `scope_aware_load` now resolves
captured names through the chain; `tests/oop/test_closure_mutating_chain_root.cfm`
pins it. The framework suites did not see it either: with page and closure
scopes now shared by reference, a wrong write is visible to every later
reader, so "boot the real app" stays in the gate.

Still open: the generic frame cost (UDF-to-UDF 306 vs Lucee 155 — dispatch,
parameter binding, the `arguments` struct, teardown; the profile is flat),
`variables.x` at 153 vs 60 (each op still locks the handle), struct key
read/write at 211/243 vs 149/140 (the key string is built and hashed per
access on both engines; ours also allocates the `Key`).

## 102. `arguments.x` — the commonest idiom in framework code — forced the eager `arguments` struct onto 35% of all frames; in `localmode="modern"` a bare parameter rebind wrote through to the argument (fixed v0.669.0) 📌

Follow-up to §101's "still open" list, which named the generic frame cost
(UDF-to-UDF 306 vs Lucee 155) as the next target. The profile there is flat —
no item over 10% — and the instrumented `call-phases` build agreed. The item
worth having was one level out, and it was not on the list at all.

Warm ns, same box, Lucee 7.1 alongside:

| shape | v0.668.0 | v0.669.0 | Lucee |
|---|---|---|---|
| `arguments.a` vs bare `a`, 1 declared parameter | +103 | **-7** | ~0 |
| `arguments.a`+`.b`+`.c` vs bare, 3 declared parameters | +174 | **-13** | ~0 |
| frames taking the eager path, `tests/runner.cfm` | 57.6% | **22.4%** | — |

**Naming the scope was the whole cost.** `arguments.foo` lowered to
`LoadLocal("arguments")` + `GetProperty`, and that load is exactly what
`function_needs_arguments_scope` scans for — so a single `arguments.foo`
anywhere in a body put *every* call of that function on the eager path, which
allocates a `CfmlStruct` (an `Arc<RwLock<..>>` plus a cycle-GC log entry) and
copies every bound argument into it a second time. 43,211 of the 122,463 frames
in our own suite were eager for that reason alone. The precedent was already
here: `SeedArgumentKey` (§ the default-parameter preamble) exists because one
defaulted parameter used to force the same thing.

`LoadArgKey` / `TryLoadArgKey` read one parameter without naming the scope.
On a frame that is eager anyway — a template frame, overflow arguments, or a
body that genuinely observes the whole scope — they read the struct, exactly as
`GetProperty` did. Otherwise they read the parameter where it already lives, in
the frame's `locals` under its own name, gated on the supplied/defaulted bits so
an omitted parameter reads Null and never a same-named enclosing variable
(§ GH #240's hazard). Reading `arguments.a` is now marginally *cheaper* than
reading bare `a`, because it resolves one key instead of walking the scope chain.

This moves the two engines closer together rather than apart. Lucee's
`UndefinedImpl.get()` resolves a bare name as `local` first, then
`argument.getFunctionArgument(key)`: a parameter has **one** storage and
`arguments.x` is a view onto it. We keep two copies, which is also why
`structDelete(arguments,"a")` still leaves bare `a` readable here and nulls it
there (below).

**Three things keep the semantics, and each was caught by a gate rather than by
review.** `local.X` / `var X` and `arguments.X` are separate scopes inside one
frame (`tests/core/test_local_shadows_arguments.cfm`), so a function that
declares a local named like one of its `arguments.X` reads keeps the eager
struct — statically decided, and it costs the optimisation only for that
function. A miss must route through `raise_undefined_member`, which raises the
catchable `expression` error `GetProperty` raised; returning a plain runtime
error re-typed it to `Runtime` and only Wheels' `contentSpec` noticed. And the
default preamble's `SeedArgumentKey` now marks the parameter supplied on the
lazy path, or an applied default would read as absent.

**`localmode="modern"`: a bare rebind of a parameter is a local write.**
Verified against Lucee 7.1 across plain assignment, a self-referencing
assignment (`a = a & "X"`), `+=`, `++`, and assignments inside branches and
loops: all of them create or update the frame's `local` and leave the argument
at its passed value, where we wrote through to both. In `classic` — the default
on every frame in Preside, ColdBox and Wheels — the write *does* update the
argument on both engines, so this is gated on the mode. A modern-mode frame that
rebinds a parameter keeps its eager struct, that being the only place the
original argument still exists after the write.

One divergence is left here: `a &= "X"` writes through to `arguments` on Lucee,
while `a += 1` and `a++` do not. `&=` has no distinct opcode in our codegen — it
lowers exactly like `a = a & "X"`, which Lucee treats as local-only. Tracked as
§109; the decision (2026-09-12) is to match Lucee.

⚠️ **Where a clause sits in the frame prologue is worth 2%.** The modern-mode
test, added to the eager-arguments decision, made CFC method calls 2.0-2.4%
slower — outside the A-to-A spread, on frames where the flag is false and the
clause is a single already-loaded bool. Moving it behind the memoized
`arguments_scope_needed` returned it to baseline. `execute_function_body` is
large and its prologue is layout-sensitive; a three-arm interleaved run
(baseline / this change alone / both) is what attributed it, a two-arm A/B
blamed the wrong half.

Still open: the generic frame cost itself (UDF-to-UDF 306 vs Lucee 155),
`variables.x` at 153 vs 60, struct key read/write at 211/243 vs 149/140, and two
`arguments`-scope divergences this work surfaced but did not address —
`structCount`/`structKeyList` over `arguments` omit declared-but-omitted
parameters where Lucee lists them, and `structDelete(arguments, "a")` does not
clear the bare name.

---

## 103. Every struct write allocated the key twice and then threw both away; a nested dot write also re-stored the scope handle over itself, invalidating the frame scope cache (fixed v0.670.0) 📌

A CFML struct write does not need to build a key at all when the key is already
there, which is the common case: `st.foo = v` in a loop, a scope key rewritten
each request, a result struct assembled field by field. We built one anyway, and
usually built it twice.

| shape | before | after | Δ |
|---|---|---|---|
| `s.w7 = i` (local struct, dot form) | 38.6 | 9.3 | **−76%** |
| `s["w7"] = i` (local struct, literal key) | 78.1 | 25.4 | **−67%** |
| `request.zz.y = i` (scope path, 2 levels) | 117.3 | 83.4 | **−29%** |
| `variables.st["w7"] = i` | 134.1 | 91.2 | **−32%** |
| `o.a.b = i` (bare root, 2 levels) | 150.9 | 115.7 | **−23%** |
| `variables.st.w7 = i` (scope path) | 169.8 | 115.6 | **−32%** |
| `variables.st.deep.k = i` (scope path, 3 levels) | 191.8 | 137.8 | **−28%** |
| `st["w" & (i % 100)] = i` (computed key) | 200.1 | 181.5 | −9% |

(ns per iteration, net of the same loop with a slot-local target; medians of a
6-round interleaved ABBA, A-to-A spread 1.4-14.3 ns. Reads, `structKeyExists`,
scoped-variable loops and every frame shape in `shapes.cfm` stayed inside the
spread.)

### The three costs

**1. The key string was materialised eagerly.** `CfmlValue::String` is an
`Arc<String>`, and `op_set_index` called `index.into_string()` — a deep copy
whenever the value is shared, which it always is for a bytecode literal. The
dot-form paths were worse: `op_set_property` and the fused
`StoreLocalProperty`/`StoreSlotProperty` arms called `name.to_string()` on an
identifier whose interned `Key` was already sitting on the `Name`
(`Name::key()`, whose own doc comment says to use it for exactly this).

**2. `Key::from_string` copied it again.** `Arc<str>` cannot adopt a `String`'s
buffer — it needs the refcount header inline — so building the owned key is a
second malloc and a second copy.

**3. `IndexMap::insert` then dropped that key.** An insert over an existing
entry keeps the key already stored (that is what preserves CFML's
first-written-casing rule) and drops the one passed in. So both allocations were
freed immediately, having done nothing.

`IntoKey::insert_into` now probes with a borrowed, non-allocating `KeyRef`
first and replaces the value in place on a hit, building the owned `Key` only on
a genuine miss — and reusing the probe's already-computed fold hash when it
does. `Key`/`&Key` callers keep the old body: cloning an existing key is an
atomic increment, so probe-then-insert would only add a lookup on the miss path.
Frame seeding passes `Key`s and is unaffected (verified: every row of
`shapes.cfm` inside the A-to-A spread).

Behaviour is unchanged by construction — `get_mut` + replace and
`IndexMap::insert` agree on the stored key, the value and the insertion order.

### Also here: the read-only check stopped allocating an error it never raised

`check_struct_writable` needs the key only to NAME it in the message, so every
caller built the string — `index.as_string()`, `name.to_uppercase()` — on every
write to produce an error that is essentially never emitted (only `cgi` and
explicitly read-only structs are marked; GH #372). The hot sites now test
`CfmlValue::is_read_only_struct()` first and materialise the string only when
the error is actually about to be reported.

### What is still slower than Lucee here

The write path is now 90 ns for a scoped struct against a ~47 ns read, and the
gap is lock traffic, not allocation: a bracket write takes a read lock for the
read-only mark, another for the CFC declared-property propagation probe
(`__variables` + `properties`), and then the write lock for the insert itself.
Folding the first two into the insert's own guard is the next step and needs a
`CfmlStruct` API change, so it is not done here.

### The nested dot form was a different code path entirely

`variables.st.w7 = i` was 40% slower than the bracket form for a reason that had
nothing to do with `SetProperty`: **any assignment two or more levels below a
scope or a bare root compiles to a dotted STRING plus `SetDynamicVar`**
(`scope_rooted_nested_path` / `bare_rooted_nested_path` in the codegen), so
`variables.a.b = v`, `request.a.b = v` and `o.a.b = v` all go through
`store_runtime_path` — a runtime walk of a path the compiler had just finished
taking apart. Per write that path did:

* `path.split('.').collect::<Vec<_>>()` — a `Vec` allocation;
* `parts[1].to_uppercase()` — a `String` for the read-only error, again never
  emitted (now gated on `is_read_only_struct`);
* `scope.to_lowercase()` and, inside `scope_aware_store`, `name.to_lowercase()` —
  two more `String`s per write to compare a scope name against a fixed list, now
  `eq_ignore_ascii_case`;
* `(*leaf).to_string()` and `(*k).to_string()` for the in-place walk — the same
  double key allocation as above, now borrowed;
* and, worst of all, **`scope_aware_store("variables", …)` re-inserted the scope
  handle over itself.** The walk mutates the scope struct in place, so the value
  arriving there is the handle already stored — but the insert still bumped
  `locals.version()`, which invalidates the `FrameScopeCache`, so the frame's
  very next `variables.x` read had to rebuild it. That store is now skipped when
  `backing_ptr()` shows nothing moved (the same guard `StoreVariablesKey`
  already had).

The string path itself is still built and re-split at runtime; pre-splitting it
at codegen (the segments are known there) is the obvious next step and is not
done here.

---

## 104. A nested scope assignment re-split at runtime a path the compiler had just taken apart — fixed, but the split was NOT where the time went (v0.671.0) 📌

Following §103: any assignment two or more levels below a scope or a bare root
(`variables.a.b = v`, `request.a.b = v`, `o.a.b = v`) compiled to a dotted STRING
plus `SetDynamicVar` plus `Pop`, and `store_runtime_path` split that string again
on every execution. The segments are known at codegen. `BytecodeOp::SetScopePath`
now carries them pre-split, and `store_runtime_path_parts` is generic over the
segment type so the compiler's `Vec<String>` is used directly.

Removed per execution: the `String` operand push, the `Swap`, the `as_string()`
deep copy of the literal's `Arc<String>`, the `split('.')` `Vec`, the clone of
the stored value, and the `Pop` — four ops become one.

### It measured 2-3%, not the 30% the allocation count suggested

| shape | v0.670.0 | after | Δ |
|---|---|---|---|
| `request.zz.y = i` | 157.2 | 153.0 | −2.7% |
| `variables.st.w7 = i` | 188.8 | 185.3 | −1.9% |
| `variables.st.deep.k = i` | 211.7 | 208.1 | −1.7% |
| `o.a.b = i` (bare root) | 188.5 | 187.6 | −0.5% |
| every shape NOT using the op | — | — | inside the spread |

(ns per iteration; two independent 6-round interleaved ABBA batches, medians;
consistent in sign and magnitude across both, with the control rows at zero.)

**So the path handling was never the cost of a nested scope write.** What is
left — roughly 110 ns for `variables.st.w7 = i` after §103 — is
`scope_aware_load` resolving the root, the in-place walk through the
intermediates, and `scope_aware_store` putting the root back. That is where the
next attempt should go; do not rebuild this one expecting more.

### ⚠️ The trap: an operand inside a struct is invisible to op-level scans

Two scanners decide whether a frame needs the EAGER `arguments` struct
(`function_needs_arguments_scope`, `args_never_escapes`), and both do it by
looking for a `BytecodeOp::String` whose text names the scope — that is how
`isDefined("arguments.rc.status")` and `evaluate()` are caught. Moving the path
from a `String` operand into `SetScopePath`'s struct made it invisible to both,
so a frame containing `param default="" name="arguments.rc.status"` silently took
the LAZY path: `scope_aware_load("arguments", …)` rebuilt an empty scope, the
walk vivified a fresh `rc`, and the write never reached the caller's struct.
Masa and Mura `param` dozens of these per request.

`Self::path_names_arguments_scope` is now shared by both scanners and applied to
`SetScopePath`'s path as well as to `String`. The general rule: **when a new
opcode absorbs an operand that a static scan used to read off the stack, every
scan that read it has to learn the new shape.** Caught by
`tests/core/test_param_attr_order.cfm` and the nested-`arguments` `isDefined`
suite — nothing else, and no review would have found it.

---

## 105. The `arguments` scope omitted declared-but-unpassed parameters; `structAppend(…, false)` skipped a null-valued key; an error escaping a dynamic include inside `lock {}` surfaced as "No exception to rethrow" (fixed v0.671.0) 📌

Three Lucee divergences, found and fixed together because the first exposed the
other two. All verified against Lucee 7.1 with probes run on both engines.

### 1. The scope holds one entry per DECLARED parameter

`function f( a, b, c )` called as `f( 1 )`:

| | Lucee 7.1 | before | now |
|---|---|---|---|
| `structCount( arguments )` | 3 | 1 | **3** |
| `structKeyList` / `structKeyArray` / for-in | `a,b,c` | `a` | **`a,b,c`** |
| `arguments.len()` | 3 | 1 | **3** |
| `serializeJSON( arguments )` | `{"a":1,"b":null,"c":null}` | `{"a":1}` | **matches** |
| `structCopy` / `duplicate` / `structAppend` out | 3 keys | 1 | **3** |
| `structKeyExists( arguments, "b" )` | false | false | false |
| `isDefined( "arguments.b" )` | false | **true** (after the change, until fixed) | **false** |
| reading `arguments.b` | null — concatenates as `""`, `len()` 0 | null | null |

An omitted parameter is a key holding null, in declaration order; defaulted
parameters count too. The existence checks still say "absent". Two interlocks
moved with it: omission detection (`JumpIfArgPresent`) is now "absent **or
null**", or no default would ever apply; and `isDefined` treats a null-valued
key as undefined, which is also what `structKeyExists` already did.

**A probe artefact worth recording.** The first probe funnelled every read
through a helper — `out( label, arguments.b )` — and Lucee threw, so the read
was briefly changed to throw. That broke `test_preside_boot_lang_fixes.cfm`,
which asserts the opposite with a real Preside path behind it. Re-probing the
exact shape settled it: Lucee does **not** throw on the read; it throws when
that null is *passed as an argument* to another function, which is argument
binding, not the read. Reverted; the test file was right.

### 2. `structAppend( target, defaults, false )` must fill a null-valued key

Lucee counts a key holding null as *lacking* for `overwrite=false`, so the
Wheels idiom `$args( name, args=arguments )` → `structAppend( arguments,
defaults, false )` fills an omitted parameter's default. We tested presence
only, so the null rode through `save()` → `invokeWithTransaction` → `$invoke`
into `$save( required parameterize )` and every model save in the Wheels suite
died with "The parameter [parameterize] to function [$save] is required but was
not passed in". Reverse direction unchanged: `structAppend( d, arguments, true )`
copies the nulls through, as Lucee does.

### 3. A dynamic include's error path never set the exception register

Codegen synthesises a `Rethrow` for the cleanup arm of `lock {}` and `try {}
finally {}`. When an error escaped a **dynamic** include (`include "#path#"`),
the handler jumped into that arm without setting `last_exception`, so the
synthesised rethrow raised "No exception to rethrow" and the real message was
lost. Every other catch-entry path set the register; this one now does too.
This is what turned the Wheels failure above into an unreadable one — the
suite runs under `lock { include "#arguments.targetPage#"; }` in `onRequest`.

### 4. …and the real-app check found a fourth: a null entry must not SHADOW the chain

The suite gates were green and Preside would not boot: "cannot call method
[isIgnored] on a null value" at `HandlerService.getHandlerListing`, which
declares `any ignoreFileService`, is usually called without it, and expects the
bare name to reach `variables.ignoreFileService`. Lucee's `UndefinedImpl.get()`
walks local → arguments → variables and treats a null argument as "not here";
two of our paths did not:

* the bare-name chain (`lookup_name_in_scopes`) returned the null entry from
  the arguments scope instead of continuing to `variables`;
* the `arguments.x = v` write-back re-mirrors every arguments key into the
  frame's locals, so the null entry became a null bare *local* — which then
  shadowed everything for the rest of the body. That is the one that bit
  Preside (`arguments.directory = replace( … )` two lines above the read).

Both now skip null entries. Probed on Lucee 7.1 across page and CFC frames,
lazy and eager, with and without the write-back, from the pseudo-constructor
and after it: identical on every row. Pinned by four more assertions in
`test_arguments_scope_shape.cfm` (28 total). **A parity change to a scope's
shape has to be checked against every consumer of that shape, and the real
app found the one the suites did not.**

### How it was found

Bisected with an env gate on the one seeding line (off: Wheels 2737/3/0; on:
suite dead). Then eliminated in turn: the `isDefined` half, the rethrow
machinery, the catch-variable lookup (zero misses), a new exception (both
catch branches instrumented: 16 events off, **one** on — the rethrow itself),
for-in iteration (hidden, still dead), and five argument-forwarding shapes
(byte-identical). Tagging every one of the 23 jump-to-catch sites named the
dynamic-include path in one run; setting the register there produced the
real message; the message named `$args`; a five-row `structAppend` probe on
both engines showed row A.

Pinned by `tests/core/test_arguments_scope_shape.cfm` (28),
`test_struct_append_null_keys.cfm` (8) and
`test_rethrow_across_dynamic_include.cfm` (2). Gates: CLI 9077/9077 · serve
dev+prod cold+warm 9209/9209 ×4 · `cargo test --workspace` 714/0/5 · wasm32 +
wasm-pack · TestBox 415/0/0 +22 · Wheels 2737/3/0 +16.

Still open from the same list: error-message wording; `a &= "X"` write-through
under `localmode="modern"`; and the bracket read `st["missing"]`, which throws
on Lucee and returns quietly here. (`structDelete( arguments, "a" )` is §106.)

## 106. `structDelete( arguments, "a" )` / `structClear( arguments )` left the bare parameter name readable (fixed v0.672.0) 📌

On Lucee the `arguments` scope IS a parameter's storage: after
`structDelete( arguments, "a" )` a bare `a` throws (`variable [A] doesn't
exist`), `isDefined( "a" )` is false, the count drops, and a later bare
`a = "x"` is an ordinary variable write that does **not** put the key back
(`arguments.a = "x"` restores both views). `structClear( arguments )` does the
same for every parameter. We bind each parameter into the frame's locals under
its own name as well as into the scope struct, so the delete removed the scope
entry and left the mirror: bare `a` still read `1`, `isDefined` said true, and
the later bare write re-inserted the key (the classic-mode param store syncs
into the scope unconditionally).

Fix: `DeleteScopeKey("arguments")` (`op_delete_scope_key`) removes the scope
entry AND the parameter's local mirror (also dropping it from the frame's
inherited/param key set), unless the name was explicitly declared local
(`var a` / `local.a` are a separate slot on both engines and survive). Codegen
emits the same op with an empty key after a statement-level
`structClear( arguments )`, which clears every parameter mirror. The
param-name store syncs into the scope only while the key is still present —
a declared-but-omitted parameter has a Null entry and takes the write (Lucee
too: `b = 5` → `arguments.b = 5`), a deleted one does not. Both eager-arguments
scanners now treat the op as naming the scope. (Frames that call
`structDelete`/`structClear` are already excluded from local slots, so no slot
state is involved.)

**Residual, deliberately left:** on Lucee in classic localmode the bare write
AFTER the delete lands in the `variables` scope (it is no longer a parameter);
here it stays frame-local. Making it leak would need the classic-mode store
routing (eight `func.params` checks) to consult a per-frame "detached" set — a
hot-prologue change that needs its own A/B for an edge of an edge. Observable
only as `variables.a` after the call.

Probe trap re-learned twice: the bare write after the delete leaked into
Lucee's page `variables`, so the NEXT probe function's bare read of the same
name found it — use distinct parameter names per probe function, and never a
name another suite in the runner assigns at page scope (`c1` was).

Pinned by `tests/core/test_structdelete_arguments.cfm` (20, identical on
Lucee 7.1). Gates: CLI 9101/9101 · serve dev+prod cold+warm 9243/9243 ×4 ·
`cargo test --workspace` 714/0/5 · wasm32 + wasm-pack · TestBox 415/0/0 +22 ·
Wheels 2737/3/0 +16 · Preside boot + admin tour clean.

## 107 + 108. Bracket reads of a missing key / out-of-range index returned quietly; every "undefined" message used our own wording (fixed v0.673.0) 📌

Lucee 7.1 throws on `st["missing"]`, `st[k]`, `arr[9]`, `arr[0]`, `[][1]`,
`q["nocol"]` and `q.nocol`; we read Null/"" and carried on. The dot forms
already threw, but with `Variable 'x' is undefined` where Lucee says:

| Shape | Lucee wording (now ours) |
|---|---|
| bare `noSuchVar` | `variable [NOSUCHVAR] doesn't exist` |
| `st.missing`, `variables.x`, `local.x`, `url/form/application/server.x` | `key [MISSING] doesn't exist` (identifier upper-cased) |
| `st["MissingKey"]`, `st[k]`, `local["x"]` | `key [MissingKey] doesn't exist` (literal keeps its casing) |
| `request.x` / `request["x"]` | `key [X] doesn't exist in the request scope` |
| `arguments.x` / `arguments["x"]` / `arguments[5]` | `The key [X] doesn't exist in the arguments scope. The existing keys are [alpha, beta]` (all DECLARED params, §105) |
| `arr[9]`, `arr[0]` | `Array index [9] out of range, array size is [2]` (negative index still reads Null, as on Lucee) |
| `noSuchFn()` | `No matching function [NOSUCHFN] found` |
| `q.nocol` / `q["nocol"]` | `Column [NOCOL] not found in query`, type `database` |
| `st.noSuchMethod()` | `The function [noSuchMethod] does not exist in the Struct.` |

All `expression`-typed and catchable in-frame (`raise_expression_message`) or
across frames. Message builders live next to it (`msg_variable_missing`,
`msg_key_missing`, `msg_function_missing`, `msg_array_index_out_of_range`).

`GetIndex` now throws; its Null-tolerant twin `TryGetIndex` serves the `?:` /
`isNull()` operand path, compound-assign reads (`st["n"] += 1`) and nested
write-back loads (`st["a"]["b"] = v` auto-vivifies as before). The fused
`obj.prop` read (`lookup_property_opt`) treats a missing query column as a miss
instead of Null.

**What the throw exposed:** `for ( x in arr )` hoisted `len(arr)` once, so a
body that deleted from the array walked past its end — silently reading Null
before, throwing `Array index [4] out of range` now. Preside's
`FormsService` merges fieldsets exactly that way (`ArrayDelete( fields, mField )`
inside `for ( mField in fields )`) and every page 500'd. Lucee reads the size
LIVE each step: a delete skips the next element and ends early (`1,2,4` over
`[1,2,3,4]`), an append is iterated. New one-op `IterLen` in the loop condition
(same rules as `len()`); the hoisted temp is gone. **A lenient read that
becomes strict turns every consumer that leaned on the leniency into a
failure — the framework suites were all green; only the real app found it.**

Left as they were (not silent, or cosmetic): the struct-method message omits
Lucee's `Available functions are [...]` suffix; `"abc".foo` reads Null (Lucee:
`there is no property with name [FOO]  found in [string]`); `arr["x"]` reads
element 1 (Lucee: `cannot cast [x] string to a number value`); a missing method
on a component keeps our wording.

Pinned by `tests/core/test_error_wording_lucee.cfm` (43, identical on
Lucee 7.1). Gates: CLI 9142/9142 · serve dev+prod cold+warm 9286/9286 ×4 ·
`cargo test --workspace` 714/0/5 · wasm32 + wasm-pack · TestBox 415/0/0 +22 ·
Wheels 2737/3/0 +16 · Preside boot + 16 admin pages clean.

