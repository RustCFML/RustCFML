# `.rcx` extension demo

A worked native extension: BIFs over every tier-1 value shape, a stateful class
with fluent chaining, and typed errors. See
[docs/extensions.md](../../docs/extensions.md) for the full guide.

```sh
rustcfml ext build examples/native_extension_demo
rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
rustcfml examples/native_extension_demo/demo.cfm
```

The engine running that last line is a **stock** `rustcfml` binary — nothing was
rebuilt, and this crate links neither the engine nor its allocator.

## What it demonstrates

| | |
|---|---|
| `demoGreet( [name] )` | the smallest useful entry point |
| `demoStats( array )` | reading a container element by element; returns a struct |
| `demoBuildQuery( rows )` | a query built on the module side |
| `demoSummariseQuery( q, col )` | the **bulk** query read — one crossing per column, not per row |
| `demoChecksum( binary )` | binary in, nothing stringified on the way |
| `demoFail()` | a custom-typed error, catchable by `<cfcatch type="demo.deliberate">` |
| `demoTally( [start] )` / `rust:Tally` | a class with state, fluent mutators and a `this.count` property |

## Tier 2 — seeing the running application

`tier2.cfm` and `tier2_race.cfm` need **serve mode**: the `application` scope and
the lock registry only exist there.

```sh
rustcfml --serve . --port 8500
#  /tier2.cfm       the guided tour
#  /tier2_race.cfm  hammer it concurrently
```

| | |
|---|---|
| `demoMemoise( key, value )` | memoise into `application` — read unlocked, then lock, **re-check**, write |
| `demoMemoiseComputations()` | how many times it actually computed; under load this stays at 1 |
| `demoUnlockedWrite( key )` | deliberately wrong, so the refusal is visible from CFML |
| `demoRequestVar( key )` | an unqualified read through CFML's own resolution order |

Note `Application.cfc`. Without one there is no application, so the
`application` scope does not persist between requests — for CFML *or* for an
extension. `probe_application_scope.cfm` checks that in plain CFML with no
extension involved, and is the first thing to run when a memoiser looks broken.

## Tier 3 — running CFML from Rust

`tier3.cfm`, also serve mode.

| | |
|---|---|
| `demoApply( callback, value )` | call a CFML closure the page handed over |
| `demoSort( array )` | Rust does the mechanics, CFML does the per-element work |
| `demoUseComponent( path, method )` | construct a CFC, **inject** into it, invoke it |
| `demoComponentAnnotations( path )` | read a CFC's metadata |
| `demoEmit( text )` | write straight to page output |

`Greeter.cfc` is an ordinary component the extension constructs and drives. Note
that its `hello()` reads `variables.injected` — injection writes the
**`variables`** scope, because that is what a component's own methods read.

The last block in `tier3.cfm` is the one that matters: CFML → extension → CFML →
extension. Under the old exclusive dispatch guard the nested extension call
deadlocked.

## The two things worth copying

**Bulk reads.** `query_column(i)` materialises a whole column in one crossing;
looping `query_cell(r, c)` costs one per row. On a 10,000-row report that is the
difference between three calls and thirty thousand.

**Declare your parameter names.** `method_params` is what makes
`t.bump( by = 5 )` bind by name. Without it the engine refuses named arguments
for that method — deliberately, because binding them in call-site order instead
is a silent wrong answer.
