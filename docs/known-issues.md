# Known Issues & Unsupported Behaviour

What RustCFML **does not fully do**, as of **v0.610.0**.

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
a separate change with a much wider blast radius than the dispatch gate, and one
that would also have to cover the JIT's member-access inline caches to stay
consistent. Treated as a follow-up rather than folded into the dispatch fix.

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

---

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
| `_schedule` — `everyMs` / `spacedMs` | Honoured with real threads: `everyMs` is fixed-rate (period measured from each run's start, missed ticks **skipped** rather than burst-replayed), `spacedMs` is fixed-delay (measured from each run's end); `everyMs` wins if both are given. A run that throws is not rescheduled, and `cancel()` stops the schedule and the run in flight. On `wasm32` (and other no-real-threads builds) they are still ignored along with `delayMs` — the closure runs inline exactly once. |
| `java.util.Collections.unmodifiable*` / `synchronized*` shims | Identity no-ops — they return the same collection with no true immutability / synchronization. |

---

