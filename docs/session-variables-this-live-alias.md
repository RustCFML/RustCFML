# Dedicated session: the `variables.this` live-alias gap

> **Self-contained handoff.** A fresh Claude Code session should be able to read
> this + the linked memory files and execute without re-deriving anything.
> Written 2026-06-24, against RustCFML **v0.279.0** (`d6c455c`, pushed).

---

## 0. TL;DR

In Lucee/ACF, a component's `variables` scope carries a **live alias** to the
component's public `this` scope: mutating `variables.this` (e.g.
`StructAppend(variables.this, fns)`) mutates the live object, and the new keys are
immediately callable/visible as public members. **RustCFML's `variables.this` is a
detached snapshot** — appends to it vanish.

**Goal of this session:** make `variables.this` a live alias of the component's
public scope (Lucee-faithful) **without reintroducing the per-request Arc-cycle
memory leak that v0.185.0 fixed.** That tension is the whole difficulty.

**Payoff:** clears the Wheels `pluginsSpec` mixin cluster (~8 specs:
`$GlobalTestMixin`/`$MixinForModels`/`$DirPluginMixin`/`$helper01`/`$$pluginOnlyMethod`/
`$$completelyOverridden`) and is a foundational correctness fix likely to ripple
into other framework code that relies on `variables.this`.

---

## 1. Load this context first

Read these memory files (in `~/.claude/projects/.../memory/`) before touching code:

- **`bug_application_cfc_lifecycle_arc_cycle_leak.md`** — ⚠️ THE risk. The exact
  cycle `template → __variables → <self-alias> → template` that leaks ~10-17 KB/req
  in serve mode; fixed in v0.185.0 by *breaking* the self-alias. Any "make
  variables.this live" change must not reintroduce this.
- **`project_wheels_clusters_post_v279.md`** — the cluster map this session came
  from, and the **arrayContains-was-a-misdiagnosis** trap (verify against Lucee
  BEFORE implementing — an agent's high-confidence root cause was wrong and the
  fix diverged from Lucee).
- **`bug_closure_unscoped_reassign_lost_stored_dispatch.md`** — the v0.279.0
  closure-scope fix; same scope-machinery neighbourhood (`StoreLocal` /
  `scope_aware_store` / `__variables`), useful background.

Then read the **release gate** in `CLAUDE.md` (cargo test --workspace incl JIT,
runner.cfm CLI+serve, wasm32 + wasm-pack) — this fix touches the component scope
core, so the FULL gate is mandatory, plus a **serve-mode `leaks` check** (§7) and
**Lucee parity** (§7).

---

## 2. Symptom & failing specs

Wheels `pluginsSpec.cfc` "Tests that injection" describe block. Failing on RustCFML
v0.279.0 (pass via Lucee 7):

- `works for Global method` — `key [$GlobalTestMixin] does not exist in the target object`
- `works for Component specific` — `key [$MixinForModels] does not exist`
- `injects mixins from directory-based plugins` — `key [$DirPluginMixin] does not exist`
- `overrides a framework method` — `expected [$$completelyOverridden] but received [hahahah]`
- `calls plugin methods from other methods` / `via $invoke` / `via $simplelock` /
  `via $doublecheckedlock` — `Component [...Test] has no function with name [$helper01]`
- `is running plugin only method` — `no function [$$pluginOnlyMethod]`

Note the **two distinct failure shapes**, both rooted in the same gap:
1. **`toHaveKey`/`StructKeyExists(obj, "$X")` fails** — the mixin never reached the
   PUBLIC scope (it only landed in private `__variables`). Direct consequence of
   `StructAppend(variables.this, mixins)` not propagating.
2. **`obj.$helper01()` "no function"** — even though the engine has a `__variables`
   method-dispatch fallback (so a function in `__variables` *is* normally
   callable, see §5), these mixins are "not found". Leading hypothesis: the mixins
   were **never injected at all**, because Wheels resolves the target class name via
   `GetMetadata(variablesScope.this)` (Plugins.cfc:800) — and if `variablesScope.this`
   is a detached/empty snapshot, `className` resolves wrong, so
   `StructKeyExists(application.wheels.mixins, className)` is false and the whole
   inject block (Plugins.cfc:812-827) is skipped. **Confirm this with tracing (§6)
   before assuming.**

---

## 3. Verified minimal repro (no Wheels)

Three files. Recreate them (scratchpad is wiped between sessions):

`Injector.cfc` — mimics `wheels.Plugins.$initializeMixins(variablesScope)`:
```cfc
component {
    public any function inject(required struct variablesScope) {
        var fns = { mixedViaVars = function(){ return "VIA_VARS"; } };
        StructAppend(arguments.variablesScope, fns, true);
        if (StructKeyExists(arguments.variablesScope, "this")) {
            var fns2 = { mixedViaThis = function(){ return "VIA_THIS"; } };
            StructAppend(arguments.variablesScope.this, fns2, true);
        }
        return true;
    }
}
```

`Target.cfc` — a component that injects mixins into ITSELF via a foreign object,
exactly like `Model.cfc:646` does `new wheels.Plugins().$initializeMixins(variables)`:
```cfc
component {
    function init() {
        new Injector().inject(variables);   // pass OUR variables scope out
        return this;
    }
    function probe() {
        var out = [];
        arrayAppend(out, "varHasMixedViaVars=" & StructKeyExists(variables, "mixedViaVars"));
        arrayAppend(out, "thisHasMixedViaThis=" & StructKeyExists(this, "mixedViaThis"));
        try { arrayAppend(out, "mixedViaVars()=" & mixedViaVars()); } catch(any e){ arrayAppend(out,"mixedViaVars ERR:"&e.message); }
        try { arrayAppend(out, "this.mixedViaThis()=" & this.mixedViaThis()); } catch(any e){ arrayAppend(out,"mixedViaThis ERR:"&e.message); }
        return arrayToList(out, " | ");
    }
}
```

`runinject.cfm`:
```cfc
<cfscript>
t = new Target();
writeOutput("inside method: " & t.probe() & chr(10));
try { writeOutput("external t.mixedViaThis()=" & t.mixedViaThis() & chr(10)); } catch(any e){ writeOutput("external mixedViaThis ERR: " & e.message & chr(10)); }
try { writeOutput("external t.mixedViaVars()=" & t.mixedViaVars() & chr(10)); } catch(any e){ writeOutput("external mixedViaVars ERR: " & e.message & chr(10)); }
</cfscript>
```

### Current RustCFML output (the bug)
```
inside method: varHasMixedViaVars=true | thisHasMixedViaThis=false | mixedViaVars()=VIA_VARS | mixedViaThis ERR:Component [Target] has no function with name [mixedViaThis]
external mixedViaThis ERR: Component [Target] has no function with name [mixedViaThis]
external t.mixedViaVars()=VIA_VARS
```

### Lucee 7 output (the TARGET — what the fix must produce)
```
inside method: varHasMixedViaVars=true | thisHasMixedViaThis=true | mixedViaVars()=VIA_VARS | this.mixedViaThis()=VIA_THIS
external t.mixedViaThis()=VIA_THIS
external t.mixedViaVars()=VIA_VARS
```

**Diff:** `StructAppend(variablesScope, fns)` already works on RustCFML (lands in
`__variables`, callable — the v0.272.0 fix). Only `StructAppend(variablesScope.this,
fns2)` is broken: `variablesScope.this` is a detached snapshot.

This repro IS the regression test. When green on RustCFML *and* Lucee, ship it as
`tests/core/test_variables_this_live_alias.cfm` (+ `Injector.cfc`/`Target.cfc`
fixtures, wired into `tests/runner.cfm`).

---

## 4. Real-world trigger (Wheels)

Every Model/Controller/Test object, in `init`, runs:
`new wheels.Plugins().$initializeMixins(variables)` (e.g. `vendor/wheels/Model.cfc:646`,
`Controller.cfc:216`, `Test.cfc:797`, `Dispatch.cfc:670`).

`vendor/wheels/Plugins.cfc:790` `$initializeMixins(required struct variablesScope)`:
- line 800: `$wheels.metaData = GetMetadata(variablesScope.this)` → derives `className`.
- line 818: `StructAppend(variablesScope, mixins[className], true)` → private scope (works).
- line 821: `StructAppend(variablesScope.this, mixins[className], true)` → PUBLIC scope (broken).
- line 824-825: same into `variablesScope.core.this`.

So `variablesScope.this` must be a live alias of the constructing component's public
scope, both for the `GetMetadata` class-name resolution AND the public mixin append.

---

## 5. Root cause + engine map

`variables.this` in a CFC method resolves like this today:
- A component instance is a `CfmlValue::Struct` whose keys are `{__name, __variables,
  __source_file, <methods...>, <public props...>}`. The PRIVATE scope is the nested
  `__variables` struct; the PUBLIC scope is the instance struct itself (= `this`).
- Inside a method, `locals` holds separate `this` and `__variables` entries
  (inserted at e.g. `lib.rs:6407`, `:14834`, `:10180`…).
- `LoadLocal("variables")` (`lib.rs` ~2909-2936) returns the `__variables` struct
  (Arc clone — shares backing, which is why appends to `variables` propagate).
- The `__variables` struct *contains a `this` key* (the repro's
  `StructKeyExists(variablesScope, "this")` is true) — but that stored `this` is a
  **snapshot** of the public scope captured at construction, NOT a live handle that
  shares the instance struct's Arc backing. Appends to it don't reach the instance.

Anchors to read:
- `lib.rs` LoadLocal `"variables"` arm (~2909-2936) — how `variables` is materialised.
- The `this`-key insertion sites (grep `insert("this"` — ~2 dozen): find which one
  seeds `__variables["this"]` and whether it stores a snapshot vs the live instance Arc.
- `call_member_function` method-dispatch + the `__variables` fallback that already
  makes private-scope functions callable (`lib.rs:14703-14717`, with the comment
  explicitly admitting *"Lucee exposes the live `variables.this` alias, which
  RustCFML doesn't"*).
- `StructAppend` impl: `cfml-stdlib/src/builtins.rs` (search `fn fn_struct_append`)
  — it mutates the target `CfmlStruct` in place via the shared Arc, so the fix is
  about making `variables.this` BE the shared Arc, not about StructAppend.
- `cfml-common/src/dynamic.rs` `CfmlStruct` — `ptr_eq`, `backing_ptr`, interior
  `RwLock`. Identity/aliasing primitives.

---

## 6. Investigation plan (do this first, before coding)

1. **Reproduce** §3 on RustCFML (confirm the bug) and Lucee (confirm the target).
   `box server start cfengine=lucee@7 port=8585` then serve the repro under the
   webroot (Lucee serves from project root; put files somewhere servable). NEVER
   `cfengine=lucee@be`.
2. **Locate the snapshot.** Add a gated `eprintln` (env var, e.g.
   `RUSTCFML_DBG_THIS`) at: (a) where `__variables["this"]` is seeded at
   construction, printing `this`'s `backing_ptr()` vs the instance struct's
   `backing_ptr()`; (b) the `LoadLocal("variables")` arm. Confirm whether
   `__variables["this"]` shares the instance's backing or is a detached copy. This
   tells you WHERE the snapshot is taken.
3. **Confirm the Wheels method-not-found shape (§2 item 2).** Briefly instrument
   `vendor/wheels/Plugins.cfc` `$initializeMixins` (a `systemOutput(..., true)` of
   `className`, `StructKeyExists(application.wheels.mixins, className)`, and the
   `GetMetadata(variablesScope.this)` result) and run just that bundle. **Caveat
   learned:** in serve mode the framework `.cfc` may be **bytecode-cached** — edits
   may not reload (touch the file / restart server / avoid `--production`), and
   `format=json` + `testBundles=` returned the HTML report not JSON. Prefer a
   standalone `.cfm` harness that drives the model layer directly, or restart the
   server after each vendor edit. **Revert vendor edits afterwards.**

---

## 7. The hard part: don't regress the Arc-cycle leak

A naive fix — store the live instance Arc as `__variables["this"]` — recreates the
exact cycle v0.185.0 killed: `instance(this) → __variables → this(=instance) →
__variables → …`. CFC method `Function` values also hold `captured_scope` pointing
back at the body scope. The cycle is unreachable at request end and leaks
~10-17 KB/req in serve mode (see `bug_application_cfc_lifecycle_arc_cycle_leak`).

Candidate approaches (evaluate, pick, justify):
- **(A) Resolve `variables.this` live on read, never stored.** Don't keep a `this`
  key inside `__variables`. Instead, make `LoadLocal("variables")` / the member-read
  path return a `variables` view whose `this` resolves to the frame's live `this`
  binding (`locals["this"]`, the instance struct, shared Arc). Mutations via
  `variables.this.X = …` / `StructAppend(variables.this, …)` then hit the instance.
  Pro: no stored back-edge → no cycle. Con: requires the `variables` value to carry
  a live handle to `this`; if `variables` is passed to another object (the Wheels
  case!), that handle must survive the hand-off — verify the repro, where
  `variablesScope` crosses into `Injector`.
- **(B) Store `this` in `__variables` as a Weak ref** (`Weak<RwLock<…>>`), upgraded
  on access. Breaks the strong cycle → no leak. Con: CfmlStruct/CfmlValue don't model
  Weak today; intrusive to the value type.
- **(C) Keep the snapshot but make it the SAME Arc as the instance** and add an
  explicit teardown of `__variables`/captured scopes at request/scope end (like
  v0.185's self-alias skip, applied to the `this` back-edge). Con: teardown timing
  is fiddly for objects that outlive the request.

(A) is most promising (no new strong references → structurally leak-free) but needs
care so the cross-object hand-off in the repro keeps the live handle. Whatever you
pick, the **serve-mode `leaks` check is part of DoD.**

---

## 8. Verification gate (mandatory — this touches the scope core)

1. **`cargo test --workspace`** (Rust + the 76 JIT tests).
2. **`cargo run -- tests/runner.cfm`** (CLI) + **serve cold+warm** (serve from the
   PROJECT ROOT, readiness-probe a LIGHTWEIGHT path like `/tests/harness.cfm` — NOT
   `/tests/runner.cfm`, which runs the whole suite per probe; that bit me).
3. **wasm32 build** (`cargo build -p cfml-worker -p rustcfml-wasm --target
   wasm32-unknown-unknown`) **and `wasm-pack build crates/wasm --target web`**.
4. **Lucee 7 parity** on the new regression test (§3) — MUST match. Re-confirm any
   behavioural assertion against Lucee BEFORE shipping (the arrayContains lesson).
5. **Serve-mode leak check** — the differentiator for THIS fix:
   `MallocStackLogging=1 ./target/release/rustcfml --serve <wheels>/public --port N`
   then drive ~100 `/ping`-style requests and `leaks <pid>` → must report **0 ROOT
   CYCLE / 0 leaked bytes**. Compare RSS plateau before/after. (dhat-heap feature +
   `RUSTCFML_MEMPROBE` tooling described in the Arc-cycle memory if needed.)
6. **Wheels delta** — build a no-fix release, run the full suite both ways, diff the
   failing-spec lists. Expect pluginsSpec mixin specs to clear with **zero new
   failures**. Recipe: `WHEELS_CI=true RUSTCFML_JIT=0 rustcfml --serve
   <wheels>/public --port N`, then `curl
   ".../index.cfm/wheels/core/tests?db=sqlite&reload=true&format=json"` (output has
   ~31KB leading template whitespace — strip to first `{` before JSON parse).
   Kill the server by PID (don't `pkill -f "rustcfml --serve"` if other serve tests
   are running concurrently — they clobber each other).

---

## 9. Definition of done

- The §3 repro produces the Lucee output on RustCFML (`thisHasMixedViaThis=true`,
  `this.mixedViaThis()=VIA_THIS`, externally callable).
- Regression test added + wired + green on RustCFML AND Lucee 7.
- Wheels pluginsSpec mixin specs clear; **zero regressions** across all 2756 specs.
- **Serve-mode `leaks` = 0 ROOT CYCLE** (no reintroduced Arc-cycle leak); RSS plateaus.
- Full release gate green (workspace+JIT, CLI+serve, wasm32+wasm-pack).
- Version bump (workspace root `Cargo.toml` `version = "0.280.0"` AND the seven
  `[workspace.dependencies]` `cfml-* version = "..."` pins — both, or the build
  fails). Commit direct to main, NO Co-Authored-By line, **ask before push**, tag +
  push as separate discrete steps.

---

## 10. Quick command reference

```bash
# build
cargo build                 # debug (fast iterate; test-file changes need no rebuild)
cargo build --release

# repro (recreate the 3 files from §3 in scratchpad)
./target/release/rustcfml runinject.cfm

# Lucee 7 (parity) — from project root
box server start cfengine=lucee@7 port=8585     # NEVER lucee@be
# ... serve repro under webroot, curl it ...
box server stop

# full CFML suite
./target/release/rustcfml tests/runner.cfm | grep -E '^SUMMARY'

# leaks check (the key differentiator)
MallocStackLogging=1 ./target/release/rustcfml --serve <wheels>/public --port 8601 &
# drive ~100 requests, then:
leaks <pid> | grep -iE 'ROOT CYCLE|leaked'
```

Engine at v0.279.0; working tree clean except the untracked
`docs/observability-*.md`. Good luck.
