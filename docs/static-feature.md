# `static` in components — implementation & follow-up

Status as of this writing: **implemented and cross-engine validated against
Lucee 7.0.4**. Closes GitHub issue #170 (TestBox `ConsoleUtil.cfc` static block)
and goes beyond it to full Lucee/BoxLang `static` parity. One runtime-lifetime
nuance is deferred (see [Deferred](#deferred-work)).

**Nothing is committed yet** — all changes are unstaged in the working tree.

---

## What `static` is (Lucee & BoxLang semantics)

Class-level state, not instance state. Initialized **once** before the type is
first used, **shared across all instances**, and (on the reference engines)
persists for the **application lifetime**.

Authoritative docs consulted:
- Lucee static scope: https://docs.lucee.org/recipes/static-scope-in-components.html
- Lucee `getComponentStaticScope`: https://docs.lucee.org/reference/functions/getcomponentstaticscope.html
- BoxLang static constructs: https://boxlang.ortusbooks.com/boxlang-language/classes/static-constructs

Feature surface and where each lands in this codebase:

| Construct | Example | Status |
|---|---|---|
| Script `static { }` block | `static { COLORS = {...}; }` | ✅ |
| `static.X` read/write in instance methods | `return static.COLORS[k]` | ✅ |
| `static function` reading `static.X` | `static function f(){ return static.x }` | ✅ (callable on instance and via `::`) |
| `::` operator (no instance) | `Comp::member`, `Comp::method(args)` | ✅ |
| `<cfstatic>` tag form | `<cfstatic><cfset static.x=1></cfstatic>` | ✅ |
| `getComponentStaticScope(name)` | Lucee BIF, string name | ✅ (also accepts an instance as a convenience) |
| Static inheritance | child reads parent's static; `Child::ParentMember` | ✅ (copy-merge, see caveat) |
| App-lifetime persistence in serve mode | `static.counter++` across HTTP requests | ❌ deferred |

---

## Design / architecture

The static scope for a component type is a shared `CfmlStruct`
(`Arc<RwLock<IndexMap>>`, reference-typed) so all instances see the same
backing. Two VM-level caches hold it (both per-VM; see Deferred):

- `static_stores: HashMap<String /* cfc source path */, CfmlStruct>` — the scope
  itself, keyed by the component's source file path. Built once on first
  instantiation; reused after.
- `static_holders: HashMap<String /* lowercased name */, CfmlValue>` — a built
  template instance (whose `__variables.__static` is the shared scope) used by
  the `::` operator so static members/methods are reachable **without** running
  `new`. `init()` is intentionally **not** run for `::`.

The shared handle is injected as `__static`:
- into the `__cfc_static_init__` frame (so the init block's writes land there),
- into the pseudo-constructor (`__cfc_body__`) scope,
- into every instance's `__variables`.

`static` is a reserved/auto-viv scope name, so `static.X` reads route through
`LoadLocal("static")` → `find_static_scope`, and `static.X = v` writes route
through the scope-aware store. `find_static_scope(locals)` looks for
`locals["__static"]`, else `locals["__variables"]["__static"]`.

### The compile-once mechanism
Codegen emits the `static { }` body as a standalone function named
`__cfc_static_init__` (one per component; appended to `program.functions`). At
resolve time the VM finds it by name, runs it with `__static` seeded, and
captures its locals. **Unscoped** assignments inside the block (`X = v`) land in
locals and are merged into the handle; **scoped** assignments (`static.X = v`)
mutate the seeded handle directly. Both end up in the same scope.

This required teaching the frame-capture machinery that
`__cfc_static_init__` is a "template frame" like `__main__`/`__cfc_body__`
(captures locals, no closure-writeback leak).

### `::` operator
Lexed as `Token::ColonColon`. Parser builds `Expression::StaticMember` (read) or
`Expression::StaticCall` (call). Codegen extracts the static class name from the
LHS (`static_class_name`: identifier or dotted-identifier chain) and emits:
- `StaticMember` → `LoadStaticHolder(name)` + `GetStaticProperty(member)`
- `StaticCall`   → `LoadStaticHolder(name)` + args + `CallMethod(...)` (reuses
  normal method dispatch, so `static.X` resolves inside the called method)

### Inheritance
When resolving a child type's static scope, the parent's static scope is
resolved (recursively) and copy-merged into the child's handle first; child
declarations override. **Caveat:** inherited members are copied by value —
struct/array members stay shared by reference, but a child scalar reassignment
(`static.parentScalar = 9`) does not propagate back to the parent's own scope.
True per-declaring-class slots (Java-style) would need a chain-walking lookup.

### Error reporting (issue #170 part 2)
A parse/tag error inside an **existing** component file used to surface as the
misleading `Could not find the component [X]`. Now the real
`Parse error in '...' [line, col]: ...` is stashed
(`last_component_compile_error`) and surfaced via `component_load_error`.
NOTE: `new X()` errors are **not catchable** in CFML `try/catch` in this engine
(pre-existing behavior, independent of static) — so this is verified manually,
not in the CFML suite.

---

## File map (line numbers approximate — grep the symbol)

**`crates/cfml-compiler/src/token.rs`**
- `Token::ColonColon` (~L62)

**`crates/cfml-compiler/src/lexer.rs`**
- `:` → `::` lexing (~L114-120)

**`crates/cfml-compiler/src/ast.rs`**
- `Component.static_body: Vec<Statement>` (~L38)
- `Expression::StaticCall` (~L305), `Expression::StaticMember` (~L306)
- `struct StaticMember` (~L407), pre-existing `struct StaticCall` (~L395)

**`crates/cfml-compiler/src/parser.rs`**
- `static { }` block collection in `parse_component` (~L3224-3232, `static_body`)
- `::` postfix parsing in `parse_call` (~L4505)

**`crates/cfml-compiler/src/tag_parser.rs`**
- `<cfstatic>` → `static { }` conversion (~L1320)

**`crates/cfml-codegen/src/compiler.rs`**
- `"static"` added to `is_reserved_scope_name` x2 (~L49, ~L613) and
  `is_autoviv_scope_root` (~L629)
- `static_class_name` helper (~L598)
- `BytecodeOp::LoadStaticHolder` (~L265), `BytecodeOp::GetStaticProperty` (~L268)
- `Expression::StaticMember`/`StaticCall` codegen (~L3078-3120)
- `__cfc_static_init__` emission in `compile_component` (~L2717-2740)
- NOTE: removed a now-unreachable `_ => Null` arm at the end of
  `compile_expression` (the match became exhaustive once both `Static*` arms
  were added). If you add a new `Expression` variant, this match now requires it.

**`crates/cfml-vm/src/lib.rs`**
- fields `static_stores` (~L1055), `static_holders` (~L1059),
  `last_component_compile_error` (~L1063); init in `new` (~L1305)
- `is_template_frame` includes `__cfc_static_init__` (~L2390)
- `LoadLocal "static"` arm (~L2617)
- early-return capture for `__cfc_static_init__` (~L4658) and normal-exit capture
  (~L6591)
- ops `LoadStaticHolder`/`GetStaticProperty` (~L5115-5126)
- `"getcomponentstaticscope"` intercept handler (~L8631) + name in the intercept
  list (search `"getcomponentstaticscope"`)
- helpers: `resolve_static_holder` (~L11255), `resolve_static_scope_by_name`
  (~L11267), `read_static_member` (~L11286), `find_static_scope` (~L11297)
- `scope_aware_load` "static" arm (~L11336), `scope_aware_store` "static" arm
  (~L11526)
- `component_load_error` (~L14096); reset (~L14111) + stash (~L14260)
- static-scope build block in `resolve_component_template` (~L14316-14377);
  `__static` injected into pseudo-constructor scope and into final
  `__variables`
- `stack_effect` entries for the two new ops (search `LoadStaticHolder`)

**`crates/cfml-stdlib/src/builtins.rs`**
- `getComponentStaticScope` registration (~L463) + stub `fn_get_component_static_scope` (~L5541)

**Tests**
- `tests/oop/test_static.cfm` (16 assertions; wired in `tests/runner.cfm`)
- fixtures: `tests/oop/StaticConsole.cfc`, `StaticKid.cfc` (inheritance),
  `StaticTagForm.cfc` (`<cfstatic>`)

---

## How to test

```bash
# Whole CFML suite (CLI). Static suite should show "16/16 passed".
cargo run -- tests/runner.cfm 2>/dev/null | grep -E "Static blocks|SUMMARY"

# Full Rust gate
cargo test --workspace

# wasm targets (host build skips them)
cargo build -p cfml-worker -p rustcfml-wasm --target wasm32-unknown-unknown

# Serve-mode cold+warm
./target/debug/rustcfml --serve --port 8773 &
curl -s http://127.0.0.1:8773/tests/runner.cfm | grep -E "Static blocks|SUMMARY"

# Cross-engine (Lucee 7) — plain start; server.json pins lucee@7.
# NEVER pass cfengine=lucee@be (resolves to 8-alpha, fails on this box).
box server start
curl -s http://127.0.0.1:8585/tests/runner.cfm -o /tmp/lucee_out.txt
grep -E "Static blocks|SUMMARY" /tmp/lucee_out.txt   # expect Static 15/15 (1 guarded)
box server stop
```

Last verified results: workspace 0 failures; CLI 4121/4121 (static 16/16); serve
cold+warm 4155/4155; wasm exit 0; Lucee 7.0.4 full suite 3896/3896 (static
15/15). The only ERROR anywhere is the known-flaky `stdlib/test_cfhttp.cfm`
(httpbin, environmental).

Cross-engine note: the instance-form `getComponentStaticScope(obj)` is a
RustCFML/BoxLang convenience and is guarded behind `isRustCFML()` in the test
(Lucee documents only the string-name signature) — hence 15 on Lucee vs 16 on
RustCFML.

---

## Deferred work

### 1. Serve-mode application-lifetime persistence (the one real gap)
`static_stores`/`static_holders` live on the VM, which is per-request in
`--serve`. Lucee/BoxLang persist statics for the whole application lifetime.
Effect: a `static.counter++` does not carry across HTTP requests (reads are
still correct within a request). The test uses **relative** counter assertions
so it's green regardless.

To close it: move the static store onto `ServerState` (alongside
`applications`/`sessions`/`bytecode_cache`), keyed by application + component
path, and resolve through it in `resolve_component_template` /
`resolve_static_holder`. Mind cross-request mutation safety (the `CfmlStruct`
RwLock already serializes) and the wasm/worker build, which has no `ServerState`
in the same shape.

### 2. Faithful per-declaring-class inheritance
Current inheritance copy-merges parent static into the child at build time.
A child scalar reassignment doesn't write back to the parent's slot. Faithful
behavior is a chain-walking `static` lookup (resolve a member by walking
child → parent → … and writing to the declaring class's handle).

### 3. Multi-component-per-file
`__cfc_static_init__` is matched by name; a single source file with multiple
components would only wire the first one's static block. `.cfc` files are
one-component, so this is an edge case — but if it matters, name the init
function per-component and match accordingly.

---

## Follow-up checklist for a new session
1. Decide whether to commit current work (version bump; project convention is
   commit direct to `main`, **ask before push**, never chain push, no
   Co-Authored-By lines).
2. File / address the serve-mode persistence follow-up (gap #1).
3. GitHub issue #170 can be closed once committed; mention the `::` /
   `<cfstatic>` / `getComponentStaticScope` / inheritance extras.
