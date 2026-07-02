# Preside-on-RustCFML boot — next-steps plan (handoff, engine v0.350.0)

> **UPDATE (v0.350.0 — cbi18n constructor-injection blocker CLEARED):**
> Boot now advances PAST cbi18n. The blocker the v0.349 handoff called
> "`controller.settingExists()` on NULL at Renderer.cfc:919" was a RED HERRING —
> that line never ran. **The real Preside site is the REPO checkout
> `/Users/alexskinner/Repos/opensource/Preside-CMS` (chrono-port), NOT the
> vendored `website/preside`** (the `/preside` mapping resolves to the repo;
> instrumenting the vendored copy showed nothing). A gated NULLCALL stack-dump
> in the engine pinned it to `cbi18n/models/ResourceService.cfc` init —
> `arguments.controller` (`inject="coldbox"`) was NULL.
>
> Root cause: the tag preprocessor (`scan_cfargument_tags`) emitted only the bare
> param NAMES for a `<cffunction>` signature, dropping every `<cfargument>`
> attribute — type, required, AND custom annotations like WireBox's `inject=`. So
> `getMetadata().functions[].parameters[].inject` was absent; ColdBox/WireBox
> `Builder.buildArgumentCollection` reads that annotation to discover constructor
> args (Mapping.cfc:890), found none → built every tag-based CFC with **0 ctor
> args** → injected deps null. FIXED v0.350.0 (committed main @ dc77f5b, NOT
> pushed/tagged — ask first). Now emits full CFScript param decls
> `[required] type name attr="v"...` (always-emit `type` default `any` to
> disambiguate annotation-vs-default in the script grammar). Lucee-verified;
> gate GREEN (workspace+JIT 76, CLI 5097, serve cold+warm 5147, wasm32+wasm-pack).
> Regression test: `tests/oop/PresideFixTagArgInject.cfc` + block H in
> `tests/oop/test_preside_serve_fixes.cfm`. Broad win — fixes ctor injection for
> ALL tag-based CFCs, not just cbi18n.
>
> **NEW immediate blocker → BCrypt JVM dependency:**
> `createObject("java", "org.mindrot.jbcrypt.BCrypt")` (Preside password hashing)
> 500s — no JVM. Per the JVM-`createObject("java",…)` convention this is normally
> out of scope, BUT bcrypt is a well-specified algorithm with a mature pure-Rust
> crate (`bcrypt`); a **native shim** (like the curated java.* set) is feasible
> here, unlike `URLClassLoader`. That's the next candidate task: either shim
> `org.mindrot.jbcrypt.BCrypt` (hashpw/checkpw/gensalt) in Rust, or confirm with
> the user whether Preside core boot can proceed without it.
>
> ---
> <details><summary>(prior v0.349.0 — cbjavaloader / ISSUE 3 CLEARED)</summary>
>
> Boot now advances
> **past** cbjavaloader. Three engine changes, gate fully green (workspace serial
> incl. JIT, CLI 5089, serve cold+warm 5139/5139, wasm32 + wasm-pack), committed?
> NO — ask before push.
> 1. **Silent `createObject("java", X)` no-op → now throws** for any class outside
>    the shimmed set (was a silent `Null` that surfaced downstream as a confusing
>    "Variable X is undefined"). Documented in known-issues §6.
> 2. **cbjavaloader Java tower shimmed** so ColdBox's JavaLoader boots: real shims
>    for `java.lang.Class`(+`forName`), `java.lang.reflect.Array`(newInstance/set/
>    get/getLength), `java.lang.Runtime` (CacheBoxProvider eager-init), `array
>    .iterator()`/`hasNext`/`next`, `java.io.File.toURL()`; **deferred** shims for
>    `java.net.URLClassLoader` / `coldfusion.runtime.java.JavaProxy` / loaded
>    classes — classloader *plumbing* (loadClass/addURL) succeeds so boot proceeds,
>    but invoking a genuinely-loaded class throws (no JVM). Tests:
>    `tests/java_shims/test_classloader_shims.cfm`.
> 3. **THE real cbjavaloader blocker: the `server` scope was READ-ONLY** (a silent
>    no-op — writes dropped; it was synthetic & rebuilt per access). cbjavaloader
>    caches its classloader in `server[key]` and reads it back. Now the `server`
>    scope is a persistent live `CfmlStruct` handle (Lucee/ACF write-through
>    parity, cross-request via `ServerState.server_scope`). Tests:
>    `tests/core/test_server_scope_write.cfm`. **This is the bigger, reusable
>    fix** — any Lucee code caching in `server.*` was silently broken before.
>
> **(this blocker — "`controller.settingExists()` on NULL at Renderer.cfc:919" —
> was a RED HERRING; see the v0.350.0 update at top for the real cause &
> fix.)** Also note: the committed v0.348.0 had a latent
> workspace-vs-dep version mismatch (deps pinned `0.347.0`, workspace `0.348.0`) —
> it only built off a stale Cargo.lock; the v0.349.0 bump fixes the dep strings.
>
> </details>
>
> **(prior, v0.348.0):** ISSUE 1 (`unknownTranslation`) RESOLVED — root cause
> was a silent no-op `array.addAll()` (the java.util.List passthrough). ColdBox
> `ModuleService.rebuildModuleRegistry` does
> `modLocations.addAll( ModulesExternalLocation )`; with addAll a no-op only
> `/coldbox/system/modules` + `/app/modules` were scanned, so cbi18n (and EVERY
> `/preside/system/modules` module) was never discovered → its `unknownTranslation`
> ColdBox setting was never registered → i18n-service autowire 500'd. Fixed +
> tested + full gate green. Committed (v0.348.0, NOT pushed — ask before push).
> ALSO confirmed end-to-end: fresh-DB `psys_page` now builds with **46 columns**
> (was 11), closing the ISSUE 2 live-schema TODO.
>
> **NEW immediate blocker → ISSUE 3 (`cbjavaloader` / JVM dependency):** boot now
> advances through i18n config and several module activations, then 500s in
> `cbjavaloader` `JavaLoader.cfc:ensureNetworkClassLoaderOnServerScope` with
> `Variable 'Array' is undefined`. Root cause: `createObject("java","java.lang.Class")`
> and `createObject("java","java.lang.reflect.Array")` both return **NULL** (no JVM,
> classes unshimmed) → `var Array = <null>` → later `Array.newInstance()` reports the
> var undefined. cbjavaloader's job is to build a `java.net.URLClassLoader` and load
> JARs from disk at runtime — a fundamental JVM dependency. Per campaign convention,
> JVM-dependent `createObject("java",…)` is **out of scope**. Options for next session:
> (a) make cbjavaloader degrade gracefully / no-op its classloader setup so boot
> proceeds; (b) decide whether to shim just enough reflection to no-op (URLClassLoader
> itself can't work, so shimming reflect.Array buys nothing on its own); (c) confirm
> with the user whether Preside core boot can skip cbjavaloader. Secondary engine
> nit worth noting: `createObject("java",unknownClass)` returning NULL silently (Lucee
> throws) produces a confusing downstream "Variable undefined" — consider a clear
> "java class not supported" error (flag-failures-loudly), but that risks other paths.
>
> **(prior, v0.347.0):** ISSUE 2 (page object 11/39 props) RESOLVED (tag comments in
> script .cfc bodies; recursive nested string interpolation) — see its section below.


Working/handoff doc. **Untracked — do NOT `git add`/commit** (per project convention for planning docs).

## TL;DR
Booting the ReadyIntelligence Preside site on RustCFML against a **fresh empty DB** now
builds the **complete schema** (180 tables, 128 FKs, sync completes) and drives into
ColdBox to a single config blocker, **`unknownTranslation`**. Behind that sits a more
serious latent bug: RustCFML assembles the core **`page`** object with only **11 columns
vs Lucee's 46** (page.cfc declares 39 properties; RustCFML drops ~28), so Preside boots
but page features will fail at runtime. Two issues to clear, below.

The earlier cross-engine schema-hash divergence (canonical-hash work) is **parked** — only
relevant if a DB must be shared between Lucee and RustCFML. For a RustCFML-owned DB it is
irrelevant (RustCFML is internally self-consistent). See
`memory/project_preside_dbsync_crossengine_divergence.md`.

---
## Environment & run loop (reproduce from cold)

- **Engine binary:** `/Users/alexskinner/Repos/opensource/CFMLs/RustCFML/target/release/rustcfml` (build with `cargo build --release`).
- **Docroot:** `/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website`
- **Preside checkout (via `/preside` mapping):** `/Users/alexskinner/Repos/opensource/Preside-CMS`, branch **`chrono-port`** (pristine). NB the canonical-hash prototype lives on branch `rustcfml-deterministic-schema-hash` (uncommitted working-tree edits were discarded; recreate from the memory file if needed).
- **DB:** MariaDB in Docker container **`busy_shtern`** (root/`freeze`, port 3306). Start Docker Desktop (`open -a Docker`; wait for `docker info`), then `docker start busy_shtern`.
- **Fresh test DB:** `pcms_ritest` (already built this session; recreate empty with
  `docker exec busy_shtern mariadb -uroot -pfreeze -e "DROP DATABASE IF EXISTS pcms_ritest; CREATE DATABASE pcms_ritest CHARACTER SET utf8mb4;"` to start clean).

### cfconfig GOTCHAS (cost time this session — read before booting)
1. **The docroot `.cfconfig.json` auto-discovery OVERRIDES `--cfconfig` for the datasource.**
   So to point at a test DB you must edit the **docroot** `.cfconfig.json` `database` field
   (don't rely on `--cfconfig` alone — moving the docroot file aside made datasource
   resolution flaky → SQLite fallback "unable to open database file: preside").
2. **`runtime.reportAsLucee` MUST be `true`** or ColdBox picks the dead-end ACF mapping
   helper and the boot fails early (surfaced as "Directory does not exist" / `/HTMLHelper`).
3. Set `debugging.enabled=true` or serve-mode 500s show only the message, not the stack.

This session left the docroot `.cfconfig.json` pointed at `pcms_ritest` with
`reportAsLucee`+`debugging` added. **Pristine original saved at
`<session-scratchpad>/docroot_cfconfig_ORIGINAL.json`** — restore it when done (it normally
points at `pcms_readyintelligencewebsite`). If that scratchpad is gone, the only diff from
the live file is `database` back to `pcms_readyintelligencewebsite` and removing the
`runtime`/`debugging` blocks.

### Boot + capture
```bash
DOCROOT=/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website
BIN=/Users/alexskinner/Repos/opensource/CFMLs/RustCFML/target/release/rustcfml
pkill -f "rustcfml --serve"; sleep 1
cd "$DOCROOT"
"$BIN" --serve --port 8601 > /tmp/serve.log 2>&1 &     # docroot .cfconfig.json must have DB+reportAsLucee+debugging
until grep -q "server running on" /tmp/serve.log; do sleep 0.5; done
curl -s -o /tmp/resp.html -w "HTTP %{http_code}\n" --max-time 400 http://127.0.0.1:8601/
grep -i "ERROR rustcfml\|dbsync.error\|CFML error" /tmp/serve.log | tail
```
**Preside's friendly-error template hides the real cause.** To see it, temp-patch the
throwing CFC to fold `e.message`/`e.detail`/SQL into the thrown `message` (it reaches
serve.log), then `git checkout --` the file after. (Used repeatedly this session.)

### Lucee ground truth (for cross-engine comparison; never needed to fix, only to compare)
`box server start name=riclean serverConfigFile=<server.json pointing webroot at docroot, cfengine lucee@7, cfconfig.file at a clean cfconfig>` then curl. **Lucee 7 only — never `@be`.**
Lucee builds a clean reference DB (e.g. `pcms_riclean_lucee`) with 47-col `psys_page`.

---
## ISSUE 1 — `unknownTranslation` setting (immediate boot blocker)

**Symptom:** `Runtime Error: The setting unknownTranslation does not exist.` 500 during ColdBox
boot, after schema sync completes.

**What it is:** Preside `system/services/i18n/i18n.cfc` (extends cbi18n) declares
`property name="unknownTranslation" inject="coldbox:setting:unknownTranslation"`. The
ColdBox/cbi18n setting `unknownTranslation` is absent from `variables.configSettings` when
WireBox autowires the i18n service → `getSetting("unknownTranslation")` throws.

**Likely class:** same family as the earlier `default_locale` blocker (fixed v0.251.0 fix #1)
— a setting written by a module/Config that isn't surviving into `configSettings`. cbi18n
module config (or Preside's i18n config) sets `unknownTranslation` (default usually `""`); it
either isn't being read from the module's `ModuleConfig.cfc` `settings` struct, or the
unscoped `settings.x=v` write isn't surviving (cf. the v0.262 classic-localmode auto-viv fix
and v0.253 case-insensitive nested-dotted-assign fix).

**Investigation steps:**
1. Find where `unknownTranslation` is set: `grep -rn "unknownTranslation" /Users/alexskinner/Repos/opensource/Preside-CMS/system` and in the cbi18n module (`.../modules/cbi18n/ModuleConfig.cfc` or `config/Config.cfc`). Confirm the default and which config layer sets it.
2. Add a throw-trace (temp) at the `getSetting("unknownTranslation")` site (ColdBox
   `coldboxModifications/Controller.cfc` `getSetting`, gate on the key) to capture the
   autowire stack and which interceptor/service triggers it.
3. Determine whether the setting is (a) never written, (b) written to a module settings
   struct that RustCFML isn't merging, or (c) written but lost (scope/auto-viv). Compare
   `getApplicationSettings()`/configSettings contents vs a Lucee boot at the same point.

**Likely fix:** mirror the `default_locale` resolution — ensure the cbi18n/i18n module
settings (`unknownTranslation`, and check siblings `defaultLocale`, `localePaths`) are merged
into `configSettings`. May be a module-settings-merge gap rather than a new engine bug.

**Test:** new assertions in `tests/oop/test_preside_serve_fixes.cfm` (the established home for
Preside-boot engine fixes) reproducing the missing-module-setting scenario; cross-check vs
Lucee 7.

---
## ISSUE 2 — RESOLVED v0.347.0 (committed NOT pushed) ✅

**Root cause was NEITHER candidate in the original plan.** The regex extraction
(candidate 1) returns all 39 names correctly; getMetaData was the symptom, not
the cause. The real cause: page.cfc's body uses a `<!--- properties --->`
tag-style comment on line 12. `tag_parser::has_cfml_tags` treated ANY `<!---`
as a tag trigger, so the whole script-based .cfc was forced through the tag
preprocessor, which echoed `component {...}` as literal text and dropped every
property — Preside then fell back to the inherited SystemPresideObject columns
(≈11). Two engine fixes:
1. `has_cfml_tags` now SKIPS `<!--- --->` comments instead of treating them as a
   tag trigger (a `<cf...>` commented out inside one no longer forces tag mode
   either); the CFScript lexer now strips `<!--- --->` tag comments directly.
2. **Exposed a second bug** once page.cfc reached script parsing: deeply nested
   string interpolation (outer `#...#` → nested string → inner `#...#` with
   further nested quotes, as in `updateChildHierarchyHelpers`) misparsed because
   nested-string consumption was flat. Made it recursive (lexer
   `consume_string_literal_into` / `consume_interpolation_into`).

Standalone repro confirms `getMetaData(page).properties` = 39 with
`labelfield="title"` (no spurious `label` column). Cross-checked vs Lucee 7:
property count/order, labelfield, and the built SQL string all match exactly.
Tests: `tests/oop/PresideFixTagCommentProps.cfc` + assertions F/G in
`tests/oop/test_preside_serve_fixes.cfm`. Full gate green; CFML suite 5104/5104
serve cold+warm.

**Still TODO (end-to-end confirmation):** boot the real Preside site against a
fresh DB and verify `pcms_*.psys_page` now builds with the full ~46 columns
(was 11). The standalone + cross-engine evidence is strong, but the live schema
build hasn't been re-run since the fix.

---
## ISSUE 2 (ORIGINAL PLAN — superseded by the resolution above) — `page` object only gets 11 of 39 declared properties

**Symptom (confirmed this session):** fresh-built `pcms_ritest.psys_page` has **11 columns**;
Lucee's `psys_page` has **46**. `system/preside-objects/core/page.cfc` declares **39**
`property` tags (`component extends="preside.system.base.SystemPresideObject"
labelfield="title" siteFiltered=true useDrafts=true`). So RustCFML is dropping ~28 of
page.cfc's **own declared** properties (plus a spurious extra `label` column — see below).
Schema sync does NOT error on this (it just builds a thin table), so it's a **latent
landmine**: page rendering / sitetree / navigation / versioning / access control will fail
later with "unknown column" once those fields are queried.

**Why page specifically:** page.cfc is unusually large (39 props, many likely multi-line
declarations with many attributes) and is a `SystemPresideObject` with `useDrafts`,
`siteFiltered`, hierarchy. The bug likely also affects other big/complex objects (asset=31
worked, but page is the extreme).

**Root-cause candidates (in priority order):**
1. **Property extraction in `PresideObjectReader._getOrderedPropertiesInAHackyWayBecauseLuceeGivesThemInRandomOrder`** (`system/services/presideObjects/PresideObjectReader.cfc:477`). It regex-scans the .cfc **source** via `$reSearch( 'property\s+[^;/>]*name="([a-zA-Z_\$][a-zA-Z0-9_\$]*)"', cfcContent )` (a `ReFindNoCase` loop, line 499). **Prime suspect:** RustCFML's `ReFindNoCase` behavior on this pattern differs from Lucee — esp. `[^;/>]*` across **newlines** (multi-line `property` declarations) or how the start-position loop advances. If this returns only 11 names, `_mergeProperties` only keeps 11. (Note: a `<cfc>$props.json` sidecar short-circuits the regex — check none exists for page.)
2. **`getMetaData(new page()).properties`** is itself short — i.e. the parser/`getMetaData`
   drops properties (multi-line `property` tags, or attributes that confuse the reader).
3. **`_mergeProperties`** (`PresideObjectReader.cfc:192`) intersects regex-order names with
   getMetaData props; if EITHER is short, props drop. Also `_mergeExtendedObjectMeta` (line 170).

**Investigation steps (definitive, ~30 min):**
1. Standalone repro, no Application.cfc. With the `/preside` mapping set, on **RustCFML CLI**
   and **Lucee** run:
   ```cfml
   m = getMetaData( createObject("component","preside.system.preside-objects.core.page") );
   writeOutput("getMetaData props = " & arrayLen(m.properties) & chr(10));
   ```
   - If RustCFML `< 39` and Lucee `= 39` → bug is in **parser/getMetaData** (candidate 2). Fix in the CFC parser (multi-line `property` tag handling).
   - If both `= 39` → bug is in the **regex extraction** (candidate 1). Reproduce `$reSearch` standalone with page.cfc content on both engines; compare the `$1` array length. Then fix RustCFML `ReFindNoCase` multiline/negated-class behavior (compare against Rust `regex` vs `fancy-regex` path; v0.341 added the lookaround fallback — this pattern has none so it's the standard path).
2. Once located, dump the FIRST property name that diverges to see what trips it (likely a
   multi-line declaration or an attribute value containing `/`, `>`, or `;` before `name=`).

**Spurious `label` column:** page has `labelfield="title"`, so Preside should NOT add a `label`
column (`_addDefaultsToProperties`/`_mergeSystemPropertyDefaults` gate on
`labelField == "label"`). RustCFML added one → `meta.labelField` isn't being read as "title"
for page. Likely the same property/attribute-reading gap, OR `labelfield` component attribute
not surfacing in merged meta. Verify `getMetaData(page).labelfield == "title"` on RustCFML.

**Proposed fix:** whichever of candidate 1/2 is confirmed. If regex (`ReFindNoCase`): align
multiline negated-char-class semantics with Lucee. If parser: handle multi-line `property`
declarations / the attribute syntax page.cfc uses. Either is a real engine fix with broad
benefit (affects any large/multi-line-property object).

**Test:** add a fixture CFC with multi-line `property` declarations + `labelfield="x"` under
`tests/oop/`; assert `getMetaData().properties` count, property names, and labelfield. Plus a
Preside-specific assertion (page-object column completeness) if feasible. Cross-check vs Lucee 7.

---
## Verification gate (MUST be green before tagging — see CLAUDE.md)
- `cargo test --workspace` (incl. JIT `jit_numeric` 76; if parallel flakes, rerun `-- --test-threads=1`)
- `cargo run -- tests/runner.cfm` (CLI) AND serve-mode cold+warm
- `cargo build -p cfml-worker -p rustcfml-wasm --target wasm32-unknown-unknown`
- `wasm-pack build crates/wasm --target web` (before pushing to main)
- Commit style: `vX.Y.Z: …`, **NO Co-Authored-By**, ask before push.

## Cleanup checklist when pausing
- Restore docroot `.cfconfig.json` (from `docroot_cfconfig_ORIGINAL.json`, → DB
  `pcms_readyintelligencewebsite`, drop the temp `runtime`/`debugging`).
- `git checkout --` any temp throw-trace patches in the Preside checkout (keep it pristine `chrono-port`).
- Test DBs on `busy_shtern` (safe to drop): `pcms_ritest`, `pcms_riclean_lucee`,
  `pcms_riclean_rcfml`, `pcms_rcfml_fresh`.
- `pkill -f "rustcfml --serve"`; `box server stop name=riclean/annotest/proptest/metatest`.
