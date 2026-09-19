# RustCFML vs Lucee — cross-engine test differences

The test suite (`tests/runner.cfm`) runs on both RustCFML and Lucee 7.0.4 (pin
that version — `lucee@be` / `lucee@7` resolve to a broken 8.0.0-ALPHA whose
CommandBox cfconfig provider fails to load).

A small number of assertions cover RustCFML-specific features, deliberate
extensions, or by-design deltas. Those are wrapped in `if (isRustCFML())`
(see `tests/harness.cfm`) so they exercise RustCFML but are skipped on Lucee,
keeping a clean cross-engine bar. They are catalogued below for transparency,
followed by the **one genuinely unresolved divergence** that needs a decision.

`isRustCFML()` detects the engine via `server.coldfusion.productname`
(`"RustCFML"` here, `"Lucee"` on Lucee).

---

## Skipped-on-Lucee (intentional) — catalogue

These are *not* bugs; they are guarded only so the shared suite stays green.

### A. RustCFML-specific config / features the Lucee test server lacks
- **`disallowedFunctions` security policy** (`tests/config/test_cfconfig_security.cfm`)
  — enforced from RustCFML's `.cfconfig.json`; the CommandBox Lucee server has no
  equivalent loaded.
- **`this.datasources` in-memory sqlite datasources** (`tests/config/test_app_datasources.cfm`)
  — `rc_app_mem` / `rc_app_mem_str` / `rc_app_bad` are declared in
  `tests/Application.cfc` and backed by RustCFML's in-memory sqlite; they don't
  exist on Lucee.

### B. Deliberate RustCFML extensions beyond Lucee
- **`dateFormat()` single-quote literals** (`tests/stdlib/test_date_functions.cfm`)
  — RustCFML honours Java SimpleDateFormat-style `'...'` literals and `''` escapes
  in `dateFormat` masks (e.g. `dateFormat(d, "yyyy' year:'mmmm")` → `2026 year:May`).
  Lucee 7.0.4 honours them in `dateTimeFormat` but not `dateFormat`.
- **`createUniqueID("counter")`** (`tests/stdlib/test_create_unique_id.cfm`)
  — RustCFML adds a `"counter"` form returning an incrementing per-instance
  integer. Standard CF / Lucee ignore the argument.

### B1. Timezone display names — verified table, not full CLDR
RustCFML backs `getTimeZoneInfo()`, `setTimeZone()`/`getTimeZone()`,
`dateConvert()` and the `java.text.DateFormat` shim's `z`/`zzzz` fields with the
IANA database (`chrono-tz`). Offsets, DST transitions and instant↔wall-clock
conversion are faithful for **every** IANA zone. The four *display-name* fields
(`shortName`/`shortNameDST`/`name`/`nameDST`, and the `z`/`zzzz` pattern fields)
are CLDR data `chrono-tz` does not carry — Java even synthesises a *theoretical*
DST name for zones that never observe DST (e.g. `JDT` / "Japan Daylight Time").
RustCFML therefore serves these from a table captured **byte-for-byte from Lucee
7.0.4 / OpenJDK 21** (`crates/cfml-vm/src/tz.rs` `display_names`), covering the
common world zones. A valid zone that is **not** tabulated has full numeric
facts but no verified names, so name-bearing calls **fail loudly** (consistent
with the "Lucee-verified or fail loud" rule) rather than guess. Adding a zone is
a one-line table entry, ground-truthed against Lucee; the eventual full-coverage
path is `icu4x` (CLDR) behind an optional feature. `Z`/`X`/`O` numeric-offset
pattern fields are computed directly and need no table.

### C. Ordered-struct semantics (by design — `IndexMap` everywhere)
- **Auto-vivified struct key order** (`tests/core/test_subscript_autovivify.cfm`)
  and **struct-literal key order with a member-inc value**
  (`tests/core/test_member_index_incdec.cfm`) — RustCFML structs always preserve
  insertion order; Lucee's plain structs don't guarantee it in these cases.

### D. Implementation-defined
- **`csrfGenerateToken()` length** (`tests/config/test_cfconfig_security.cfm`)
  — RustCFML emits a 64-char hex token, Lucee 7.0.4 a 40-char one. cfdocs does
  not fix the length. (`csrfVerifyToken` round-trips on both.)

---

## E. RESOLVED — numeric-subscript auto-vivification makes a struct

**Status:** resolved in GH #429. RustCFML now matches Lucee: a numeric subscript
assigned into an *undefined* root vivifies a **struct**, not an array. Nothing is
guarded RustCFML-only any more; the case runs on both engines.

**Files:** `tests/core/test_subscript_autovivify.cfm`,
`tests/core/test_numeric_subscript_vivifies_struct.cfm`

**The case:**
```cfml
// rcfmlAutoVivArray is undefined here
rcfmlAutoVivArray[3] = "c";      // both engines: struct { "3": "c" }
```

RustCFML used to vivify a 1-based auto-growing array (`[null,null,"c"]`), which
was invisible to the immediate read and only surfaced once something inspected
the container — `isArray`/`isStruct`, `arrayLen` vs `structCount`,
`serializeJSON`, or a later `arrayAppend`.

**What made it more than a one-line change:** Lucee's array BIFs accept such a
struct, so the idiom `for(i=1;i<=n;i++){ u[i]=…; }` still works with `arrayLen`,
`arrayAppend`, `arrayMap` and the rest. Lucee implements that with
`StructAsArray`, a **positional** view over the keys `"1".."n"`. RustCFML now
provides the same view (`struct_as_positional_array` in `cfml-common`), applied
at the array-BIF argument boundary in `cfml-stdlib` and at the higher-order
intercept seam in `cfml-vm`.

**Where we deliberately do NOT follow Lucee.** `StructAsArray` is only coherent
when the keys are exactly 1..n, and Lucee's own behaviour outside that is
self-contradictory. Measured on Lucee 7.1.0.204:

| case | Lucee 7.1.0.204 | RustCFML | why |
|---|---|---|---|
| `s={20:…,4:…,13:…}; arrayToList(s)` | `,,` (three empty strings) | same positional rule, missing slots read null | we keep the rule, not the garbage |
| `s={20:…}; arrayFirst(s)` | throws `key [1] doesn't exist` | null for the empty slot | an engine-internal leak, not a semantic |
| `arrayClear({1:10,2:20})` | leaves `{"2":20}` | leaves `{}` | a cleared array holding an element is a bug |
| `arraySort(struct)` | throws | throws (same wording) | matched |

A struct with any non-numeric key is refused on both engines with Lucee's
wording: `can't cast struct to an array, key [A] is not a number`.

The mutators follow Lucee's key-based rules exactly: append at `max key + 1`
(floored at 1), prepend shifts existing keys up by one, and a removal refuses an
absent position key rather than renumbering. `arrayClear` is the only divergence.

**Key order is not part of this.** Lucee's `serializeJSON` of a vivified struct
prints in Java `HashMap` bucket order, not numerically — `y[3]=…; y[100]=…`
prints `{"100":…,"3":…}`, and `{20,4,13}` prints `13,4,20`. That is an
implementation artifact, so RustCFML keeps its insertion order (IndexMap) and the
positional *array view* is what carries the ordering guarantee.

**Note on `string`-key auto-viv:** the sibling case `x["alpha"] = 1` (string key
→ struct) was never in dispute — both engines make a struct, and always did.
