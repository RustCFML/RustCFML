# Plan: AntiSamy on RustCFML — faithful Java shim + native BIF

> Goal: restore HTML/XSS sanitisation for Preside on RustCFML with **zero upstream
> changes to Preside core**, by shimming `org.owasp.validator.html.AntiSamy` /
> `.Policy` over a native Rust sanitiser — and expose the same core as a
> `sanitizeHtml()` BIF for every other CFML app.
>
> Supersedes the temporary no-op in `PRESIDE_BOOT_JAVA_NOOPS.md` §5.
> Consumption detail: `PRESIDE_JAVA_SURFACE.md` Part 2.

## Why shim-first, unlike the cfconcurrent plan

`PRESIDE_CFCONCURRENT_PLAN.md` deliberately chose **BIFs + Preside engine-detection**
over a Java shim. This plan inverts that, for three reasons specific to AntiSamy:

1. **The constraint is "no upstream Preside changes."** cfconcurrent's branch-on-engine
   approach requires a Preside PR; here we want the existing checkout to just work.
2. **The surface is tiny.** cfconcurrent would have needed `TimeUnit`,
   `ThreadPoolExecutor`, dynamic proxies and a jar. AntiSamy needs **three methods
   across two classes** (see contract below) — a shim is genuinely cheaper than a
   Preside-side migration.
3. **The security posture is worse today.** The no-op passes *all* untrusted request
   input through unsanitised (see Blast radius). Anything that closes that quickly
   wins over anything that needs an upstream release cycle.

The BIF still gets built (Part 3) — it's the honest public API and the migration
target if Preside upstream ever wants it. Both layers sit on one core.

## Blast radius (why this matters more than it looks)

`AntiSamyService.clean()` is **not** a rich-text-editor sanitiser in Preside. It is a
global request input filter — `system/handlers/General.cfc:110-128`:

```cfml
private void function _xssProtect( event, rc, prc ) {
    if ( IsTrue( antiSamySettings.enabled ?: "" ) ) {
        var bypass = adminBypass && ( event.isAdminUser() || event.isAdminRequest() );
        if ( !bypass ) {
            for( var key in rc ){
                if( IsSimpleValue( rc[ key ] ) ) {
                    rc[ key ] = antiSamyService.clean( rc[ key ], policy );
                }
            }
        }
        request[ "preside.path_info"    ] = antiSamyService.clean( ... );
        request[ "preside.query_string" ] = antiSamyService.clean( ... );
    }
}
```

Enabled by default (`system/config/Config.cfc:1096`, policy `preside`,
`bypassForAdministrators = true`). So:

- It runs on **every `rc` key of every non-admin request**. This is the hottest path
  in the application — performance is a correctness concern, not a nicety.
- While no-op'd, **every front-end request is completely unfiltered**. That is
  materially worse than "rich text isn't sanitised".
- Most `rc` values are short scalars with no markup. A **fast path** (no `<` and no
  `&` ⇒ return input unchanged, skip the parser) will skip the large majority of
  calls and should be implemented from day one, not as an optimisation later.

## What already exists in RustCFML (verified in source, this session)

- **`createObject("java", cls, …)` dispatches on `args[1]` alone** —
  `crates/cfml-vm/src/lib.rs` ~16600-16740, a `match` on the lowercased class name.
  **Extra arguments are ignored**, so Preside's 3-arg jar-path form
  (`CreateObject("java", "…AntiSamy", _listJars())`) needs no special handling. ✅
  *This was the main feasibility risk and it is resolved.*
- **Unsupported classes throw** with `createObject: Java class [x] is not supported.`
  — so today AntiSamy fails at construction and Preside's try/catch catches it.
- **The marker-shim pattern is well established** in `crates/cfml-vm/src/java_shims.rs`
  (6,290 lines). Directly analogous precedents to copy:
  - `org.mindrot.jbcrypt.BCrypt` → routes to native bcrypt builtins
  - `org.yaml.snakeyaml.Yaml` → routes to `yamlDeserialize`
  - `ca.vanmulligen.json.schema.validator` → routes to `validateJSON`
  - `org.apache.commons.imaging.Imaging` → static holder + result object
    (`make_commons_imaging_info`) — **closest analogue**, since AntiSamy also needs a
    result object (`CleanResults`) with one getter.
- **`java.io.File` is shimmed**, which `Policy.getInstance()` receives. The shim needs
  to read a path back off it.

## The contract — the complete Preside surface

From `system/services/security/AntiSamyService.cfc`. Nothing else is touched:

| # | Call | Site | Returns |
|---|---|---|---|
| 1 | `createObject("java","org.owasp.validator.html.AntiSamy", jarArray)` | `:51` | instance — **no `.init()`** |
| 2 | `createObject("java","org.owasp.validator.html.Policy", jarArray)` | `:62` | static holder |
| 3 | `Policy.getInstance( javaFileObj )` | `:65` | Policy — **opaque to Preside**, only passed back into `scan()` |
| 4 | `antiSamy.scan( dirtyHtml, policyObj )` | `:28`, `:82` | CleanResults |
| 5 | `cleanResults.getCleanHtml()` | `:29`, `:83` | string |

Because Policy is opaque, its internal representation is entirely ours — a handle/index
into a parsed-policy registry is fine.

Preside caches policies itself (`_policies` struct, `:57-69`), so `getInstance()` is
called at most six times per app lifetime. **Parse cost is irrelevant; scan cost is
everything.**

## Fidelity is cheaper than expected — `_removeUnwantedCleanses` self-calibrates

`AntiSamyService.cfc:80-93` looks like it hard-codes AntiSamy's entity behaviour. It
doesn't — it **probes the sanitiser at runtime** with a bare `&` and adapts:

```
input:      He said &quot;hi&quot;
→ masked:   He said &~~~quot;hi&~~~quot;        (:27, &quot; → &~~~quot;)
→ scan:     He said &amp;~~~quot;hi&amp;~~~quot;  (html5ever escapes the bare &)
→ probe:    scan("&") → "&amp;"  ⇒  replace all "&amp;" → "&"
            He said &~~~quot;hi&~~~quot;
→ unmask:   He said &quot;hi&quot;              ✓ round-trips
```

**Consequence:** the shim does **not** need to match AntiSamy byte-for-byte. It needs
to be *internally consistent* about escaping `&`. html5ever's serialiser is.

⚠️ This is load-bearing and non-obvious — the `&~~~quot;` marker only survives because
the ampersand pass runs first. **Pin it with a test before touching anything else**
(Part 4, T1). If a future serialiser change stops escaping bare `&`, this breaks
silently and quotes start vanishing from user input.

## The policy format — what the parser must handle

Six policy files ship with Preside in
`system/services/security/antisamylib/` (`preside`, `tinymce`, `ebay`, `myspace`,
`slashdot`, `anythinggoes`). `antisamy-preside-1.4.4.xml` is 2,611 lines. Only
`preside` is used by default; `tinymce`/`slashdot` are ~180-200 lines.

Verified top-level sections (all six files share the schema):

```
<directives>            9 in preside — omitXmlDeclaration, maxInputSize,
                        useXHTML, preserveSpace, embedStyleSheets, …
<common-regexps>        223 named regexps — e.g. colorName, onsiteURL, offsiteURL
<common-attributes>     184 attribute defs, each with <regexp-list> and/or
                        <literal-list>, optional onInvalid=
<global-tag-attributes> attributes allowed on any tag
<tags-to-encode>        2 in preside: g, grin
<tag-rules>             63 tags
<css-rules>             119 <property> defs, each with <literal-list>,
                        <regexp-list> (named refs), <shorthand-list>
```

Tag actions in `preside`:

| Action | Count | Tags | Ammonia mapping |
|---|---:|---|---|
| `validate` | 49 | a, b, div, img, p, span, style, table, … | `allowed_tags` + attribute rules |
| `remove` | 4 | `frame frameset iframe script` | `clean_content_tags` (drop element *and* contents) |
| `truncate` | 3 | `dd dl dt` | **no direct equivalent** — keep element, strip all attributes |

`onInvalid` values in use: `removeTag` ×4, `filterTag` ×2, `remove` ×1.

Note `<attribute name="style"/>` is declared with **no rules** and a comment saying
*"will be validated by an inline stylesheet scanner"* — i.e. the whole of `style=`
validation is delegated to `<css-rules>`. That is why CSS cannot be skipped.

---

# Part 1 — the sanitiser core *(the actual work)*

New module, suggested `crates/cfml-vm/src/antisamy.rs` (or a `cfml-sanitize` crate if
it grows). Three pieces:

### 1a. Policy model + XML parser

- `quick-xml` (already a plausible dep — check `Cargo.toml` before adding) → a
  `Policy` struct: tags, attributes, named regexps, CSS properties, directives.
- Resolve named references at parse time (`<regexp name="onsiteURL"/>` →
  the compiled regex from `<common-regexps>`); compile all regexps once, `regex` crate.
- ⚠️ **Regex dialect.** AntiSamy regexps are Java-flavour. The `regex` crate has no
  backreferences or lookaround. Audit all 223 for constructs it rejects — most are
  simple character classes and alternations, but this needs checking early because a
  hard incompatibility changes the design (fallback: `fancy-regex`).
- Store parsed policies in a registry; `getInstance(file)` returns a handle.

### 1b. HTML sanitisation — `ammonia`

Map the policy onto `ammonia::Builder`:

- `allowed_tags` ← `action="validate"` tags
- `clean_content_tags` ← `action="remove"` tags
- `allowed_attributes` / `allowed_attributes_per_tag` ← common + global-tag attributes
- `attribute_filter` callback ← per-attribute `<regexp-list>` / `<literal-list>`
  validation, plus `onInvalid` semantics (`removeTag` must drop the whole element, not
  just the attribute — the callback alone can't do that; needs a post-pass or a
  pre-parse with `html5ever` directly)
- `url_schemes` / `url_relative` ← onsiteURL/offsiteURL regexps
- `truncate` tags → custom pass stripping attributes on `dd`/`dl`/`dt`

⚠️ **`onInvalid="removeTag"` is the awkward one** — ammonia's `attribute_filter` can
only rewrite/drop the attribute, not the element. Four attributes use it. Options:
(a) post-process the ammonia output, (b) drive `html5ever` directly and skip ammonia's
Builder, (c) accept a documented divergence for those four. **Decide in Phase 1**;
option (b) is the honest one but costs the ammonia convenience layer.

### 1c. CSS validation — `lightningcss`

The gap ammonia does not cover. 119 properties with literal-lists, named-regexp lists
and shorthand expansion.

- Parse the `style` attribute value and any `<style>` element content.
- For each declaration: look up the property; accept if it matches a literal or a
  regexp; expand `<shorthand-list>` properties (e.g. `background` → `background-color`,
  `background-image`, …) and validate components.
- Drop non-matching declarations; re-serialise.
- Directives that bear on this: `embedStyleSheets=false` and `maxStyleSheetImports=3`
  mean **do not fetch remote stylesheets** — never make network calls here.

### 1d. Fast path

Before any parsing: if the input contains neither `<` nor `&`, return it unchanged.
This is the single highest-value performance decision given the per-`rc`-key call
pattern. Measure the hit rate on a real request mix.

# Part 2 — the Java shim *(zero Preside changes)*

`crates/cfml-vm/src/java_shims.rs` + two match arms in `lib.rs`:

```rust
"org.owasp.validator.html.antisamy" => java_shims::make_antisamy(),
"org.owasp.validator.html.policy"   => java_shims::make_antisamy_policy_static(),
```

Follow `make_commons_imaging_static()` / `make_commons_imaging_info()` exactly — the
shape is identical (static holder + opaque result object with getters).

Methods to route:
- `AntiSamy.scan( html, policyHandle )` → `CleanResults` marker carrying the result
- `CleanResults.getCleanHtml()` → string
- `Policy.getInstance( fileShim )` → read path off the `java.io.File` shim, parse (or
  hit the registry cache), return handle

Everything else on these objects should **throw**, matching the stricter third-party
shims (BCrypt, SnakeYAML, commons-imaging) rather than returning silent `null`.

**No Preside edit is required.** The existing `try/catch` in `_setupAntiSamy()` simply
stops catching, and the `IsSimpleValue()` guard in `clean()` stops short-circuiting.
Leave the jars on disk — `_listJars()` keeps working and the shim ignores the array.

# Part 3 — the BIF

`sanitizeHtml( html [, policy ] )` in `crates/cfml-stdlib/src/builtins.rs`, over the
same core. `policy` accepts a named built-in (`"preside"`, `"tinymce"`, …), a path to
an AntiSamy XML file, or a struct/JSON policy for non-Preside callers.

Not required for Preside to work — but it's the API worth having, it's what a future
upstream PR would target, and it makes the core testable without going through the
shim.

# Part 4 — validation

Cross-engine, per `docs/testing.md` conventions — the suite must pass on **Lucee with
real AntiSamy** and on RustCFML with the shim, comparing output.

- **T1 — the `&quot;` round-trip.** `clean('He said &quot;hi&quot;')` must return the
  input unchanged. Directly pins the `_removeUnwantedCleanses` interaction above.
  **Write this first.**
- **T2 — XSS corpus.** OWASP filter-evasion cheat-sheet vectors + the html5ever
  mutation-XSS cases. Assert *no* vector survives under the `preside` policy.
- **T3 — policy parity.** A fixture set of ~50 HTML snippets scanned under all six
  policies on both engines; diff the output. Expect *some* divergence — record it as
  known-divergence rather than chasing byte-identity.
- **T4 — CSS.** Per-property accept/reject fixtures across the 119 properties,
  including shorthand expansion and `url(javascript:…)` rejection.
- **T5 — `onInvalid` semantics.** Specifically the four `removeTag` attributes.
- **T6 — throughput.** Benchmark the `_xssProtect` pattern: ~20 short scalar `rc`
  values, no markup. Must be dominated by the fast path. Compare against Lucee to
  make sure we're not regressing a hot path.
- **T7 — no network.** Assert the CSS scanner never opens a socket, even given
  `@import` / remote `url()`.

## Sequencing

| Phase | Deliverable | Unblocks |
|---|---|---|
| **0** | Spike: 20-line shim returning input unchanged, confirm Preside constructs it and `scan()`/`getCleanHtml()` dispatch end-to-end. Write T1. | proves wiring; **do this before any sanitiser work** |
| **1** | Policy XML parser + regex-dialect audit + `onInvalid=removeTag` decision | the two design unknowns |
| **2** | ammonia integration + fast path; tags/attributes only, `style` **stripped entirely** | usable, secure-but-lossy sanitiser — could ship as an interim |
| **3** | CSS validation via lightningcss; restore `style` | feature-complete |
| **4** | BIF + docs (`docs/java-shims.md`, `docs/known-issues.md`) | public API |

Phase 2 is a legitimate stopping point if time is short: stripping `style` outright is
*safe*, just lossy for styled content — and infinitely better than today's no-op.

## Decisions (locked)

- **Shim is primary, BIF is secondary.** Driven by the no-upstream-changes constraint.
- **Self-consistency over byte-fidelity.** Justified by the runtime probe in
  `_removeUnwantedCleanses`; T3 records divergences rather than eliminating them.
- **Fail loud on unknown methods**, consistent with the other third-party shims.
- **Never fetch remote resources** during CSS validation.

## Open questions

1. `onInvalid="removeTag"` — ammonia post-pass, direct html5ever, or documented
   divergence? (Phase 1)
2. Do all 223 policy regexps compile under the `regex` crate, or is `fancy-regex`
   needed? (Phase 1)
3. Should `<style>` *element* content be validated, or just the `style` attribute?
   The policy has `<tag name="style" action="validate">`, so AntiSamy does both.
4. Is the six-policy set worth supporting up front, or start with `preside` + `tinymce`
   (the only two Preside actually configures) and throw clearly on the rest?
