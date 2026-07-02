# Preside Test Suite Compatibility on RustCFML

> Status of running **Preside-CMS's own TestBox unit suite** on RustCFML.
> Snapshot: **2026-06-29, engine v0.357.0**, Preside `chrono-port` branch.
> This is an **untracked working doc** — do not commit.

Distinct from the *serve-boot* campaign (which boots a live Preside site). This is
Preside's `tests/` TestBox suite — the one that produced GH issues #201–#210.

---

## TL;DR

`scope=quick` now **runs to completion** against MariaDB:

```
[Passed: 1263] [Failed: 167] [Errors: 178] [Skipped: 2]
[Bundles/Suites/Specs: 109/467/1610]
```

Up from the 06-27 baseline of `1175 / 184 / 261`. Of the **345 non-passing specs**:

| Category | Count | Share | Actionable? |
|---|---:|---:|---|
| JVM / Java-library hard deps | ~118 | 34% | Only via per-class shims (out of scope) |
| Image-function gap (ImageInfo/ImageNew/reader) | ~14 | 4% | Feature gap — pure-Rust image crate could close it |
| **Real RustCFML compat differences** | **~213** | **62%** | **Yes — the actual work** |

---

## How to run it

**Database** — the suite needs a MySQL-protocol DB. Two options:

- **MariaDB** (used for this snapshot): `127.0.0.1:3306`, user `root`, password `freeze`,
  database `preside_test`. MariaDB uses native-password auth so the RustCFML MySQL
  driver connects cleanly.
- **Docker MySQL 8.4** (`preside_test_mysql` container): works *only* if you pre-register
  the datasource with a real password via `--cfconfig` — MySQL 8.4 defaults to
  `caching_sha2_password`, and the RustCFML driver gets "Access denied" with an empty
  password. MariaDB avoids this.

Ensure the DB exists: `CREATE DATABASE IF NOT EXISTS preside_test;`

**cfconfig** (`<scratch>/preside_test.cfconfig.json`) — only needs:

```json
{ "runtime": { "reportAsLucee": true }, "debugging": { "enabled": true } }
```

No datasource block required: `Application.cfc` self-registers it from env vars
(this works as of v0.357.0 — see fix #1 below).

**Serve & run:**

```bash
cd Preside-CMS/tests
PRESIDETEST_DB_HOST=127.0.0.1 PRESIDETEST_DB_PORT=3306 PRESIDETEST_DB_NAME=preside_test \
PRESIDETEST_DB_USER=root PRESIDETEST_DB_PASSWORD=freeze \
rustcfml --serve --port 8599 --cfconfig <scratch>/preside_test.cfconfig.json

# quick scope (excludes 5 heaviest bundles incl. presideObjects):
curl -s "http://127.0.0.1:8599/runtests.cfm?reporter=text&scope=quick" -o /tmp/pq.txt
```

**Getting a tally past the jsoup abort.** `unit.api.email.EmailStyleInlinerTest` does
`createObject("java","org.jsoup.Jsoup")` at construction. TestBox rethrows that as a
fatal `BundleRunnerMajorException` (TestBox.cfc:742) and the bundle loop
(TestBox.cfc:436) does **not** catch it, so the whole run 500s. To get a tally, drop a
throwaway runner in `tests/` that copies `runtests.cfm`'s TestBox call but adds the
JVM-dependent bundle basenames (e.g. `EmailStyleInlinerTest`) to the filter excludes,
then `curl` it. Remove the runner afterwards. (A proper engine option would be for
TestBox to record a construction-time bundle error and continue, but that's a TestBox
behaviour, not ours.)

---

## Engine fixes shipped to get here (v0.357.0, committed `6a92c41`, NOT pushed)

Both are **enablers** — they don't change pass/fail counts, they make the suite run
through Preside's documented configuration path:

1. **`System.getenv()` (no-arg) now returns a `java.util` Map shim**, not a plain
   struct. Preside's `_getEnvironmentVariable()` reads config via
   `System.getenv().get("X")`; on a plain struct that `.get()` didn't dispatch and
   silently returned `null`, so the DB env vars were never read (→ empty password →
   `nodsn` 500). Fix in `crates/cfml-vm/src/java_shims.rs` (`handle_java_system`).
2. **`dbinfo` reports `database_productname = "MySQL"` for the MySQL driver even
   against MariaDB** (the MariaDB build string stays in `database_version`). Preside
   whitelists `"MySQL"` for both datasource validation *and* DB-adapter selection, so
   without this it rejected the connection as `invalidDsn`. Matches a Lucee/ACF
   deployment using the MySQL JDBC connector. Fix in `crates/cfml-stdlib/src/dbinfo.rs`.

Regression test: `tests/java_shims/test_system.cfm` (getEnv get/containsKey/keySet).
Earlier same-day fix v0.356.0 (`b9c438e`, pushed): case-insensitive chained-CFC
writeback guard (GH #219) — unblocked TestBox's async BDDRunner path.

---

## The 345 non-passing specs, categorised

Counted from the text reporter's per-spec markers (`X Error:` = thrown exception,
`! Failure:` = assertion mismatch) in the captured run.

### 1. JVM / Java-library hard dependencies — 118 errors (~34%)

All are `createObject: Java class [X] is not supported. RustCFML has no JVM`. By
frequency of the class requested:

| Java class | count | notes |
|---|---:|---|
| `org.yaml.snakeyaml.Yaml` | 38 | YAML parsing — **pure-Rust shimmable** (serde_yaml) |
| `lucee.loader.engine.CFMLEngineFactory` | 36 | Lucee engine introspection — RustCFML-specific stubbing |
| `org.jsoup.Jsoup` | 23 | HTML parsing/inlining — large surface |
| `java.io.ByteArrayOutputStream` | 9 | stdlib I/O — shimmable |
| `net.glxn.qrgen.javase.QRCode` | 3 | QR generation |
| `org.mindrot.jbcrypt.BCrypt` | 2 | **pure-Rust shimmable** (bcrypt crate) — also a serve-boot blocker |
| `java.security.SecureRandom` | 2 | shimmable |
| `com.adobe.xmp.XMPMetaFactory` | 2 | XMP image metadata |
| `org.owasp.validator.html.AntiSamy` | 1 | HTML sanitiser |
| `java.io.FileWriter` / `FileInputStream` / `util.regex.Matcher` | 1 each | stdlib — shimmable |

Highest value-for-effort shims: **snakeyaml** (38) and **BCrypt** (2, also blocks the
live boot). jsoup/antisamy/qrgen are larger third-party surfaces.

### 2. Image-function gap — ~14 errors (~4%)

`Variable 'ImageInfo'/'ImageNew' is undefined` (9) + image `reader` (5). These are CFML
image BIFs, JVM-backed in Lucee but a genuine RustCFML feature gap (no "no JVM" throw).
Could be implemented with a pure-Rust image crate.

### 3. Real RustCFML compat differences — ~213 (~62%) — the actual work

**167 assertion failures** (pure behaviour diffs, never JVM):

- **~45 — empty struct/array received** (`fieldsets: []`, `received [{}]`):
  **FIXED v0.358.0** by the XML named-child-access fix — Preside form
  definitions are XML (`<tab><fieldset><field>`) read via named-child
  traversal. Re-run at v0.358 (06-29): all 19 fieldset specs pass, the
  `fieldsets: []`/`received [{}]` signature is gone suite-wide.
- ~~**~20 — `__arguments_scope` leak**~~ **FIXED v0.359.0.** The CFML
  `arguments` scope is a hybrid array/struct on Lucee — `isArray(arguments)` is
  true (positional, named, empty alike). Ours returned false, so TestBox's
  `equalize()` (Assertion.cfc:1324, branches on `isArray(actual) && isArray(expected)`)
  fell through to the struct branch and dumped the sentinel keys. Fix: `isArray()`
  + `arrayIsDefined()` treat a Struct carrying `__arguments_scope` as an array
  (`arrayLen` already did); JIT mirror updated. Quick suite **1295→1313 passed,
  142→124 failed**; leak 40→4 (the 4 remaining are a separate REST-routing diff).
- **10 — generated DDL nullable**: `change \`col\` … null` vs `not null` in Preside's
  ALTER TABLE builder (a column-not-null flag lost somewhere in metadata → SQL).
- **5 — component path short vs full**: e.g. `MySqlAdapter` vs
  `preside.system.services.database.adapters.MySqlAdapter` (getMetadata/getComponentName
  reporting the leaf name instead of the dotted path).
- remainder — struct key-ordering serialize diffs (known-issue family) and assorted
  value/boolean mismatches.

**~46 undefined-variable errors** (genuine engine bugs, non-image):

| Variable | count |
|---|---:|
| `isOriginAllowed` | 6 |
| `ValueArray` | 5 (unimplemented Lucee BIF) |
| `taskId` | 5 |
| `rendered` | 5 |
| `$errorLogService` | 4 |
| `result` / `detail` | 3 each |
| `$raiseError` / `newId` | 2 each |
| misc (`tester`, `newFormId`, `itemId`, `headers`, `actionId`, `$getWebsiteLoggedInUserId`) | 1 each |
| misc non-undefined (email-to missing, invalid JSON, fileDelete, len-on-null) | ~5 |

Likely the unscoped-var / closure-capture / missing-BIF families (`ValueArray` is just
an unimplemented BIF; the `$`-prefixed ones look like injected/mixin methods not
resolving).

---

## Suggested priority order for the compat work

1. ~~Form-definition empty-fieldsets bug (45 fails)~~ — **DONE v0.358.0** (XML
   named-child access).
2. ~~`__arguments_scope` leak (~18 fails)~~ — **DONE v0.359.0** (arguments scope
   isArray=true hybrid).
3. **Undefined-var cluster** (~46) — triage `isOriginAllowed`/`taskId`/`rendered`/
   `$errorLogService`; probably a couple of shared root causes (mixin/injected method
   resolution + a few missing BIFs like `ValueArray`).
3. **DDL nullable** (10) — localised to Preside's SQL adapter + the not-null metadata flag.
4. **Component-path reporting** (5) — getMetadata name vs full path.
5. Cheap JVM shims with outsized payoff: **snakeyaml** (38 errors) and **BCrypt** (2,
   also unblocks live boot).

Excluded from `quick` and not covered here: `presideObjects` (own campaign),
`CsrfProtectionServiceTest`, `LoginServiceTest`, `AuditServiceTest`, `SiteServiceTest`.
