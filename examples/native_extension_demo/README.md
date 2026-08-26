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

## The two things worth copying

**Bulk reads.** `query_column(i)` materialises a whole column in one crossing;
looping `query_cell(r, c)` costs one per row. On a 10,000-row report that is the
difference between three calls and thirty thousand.

**Declare your parameter names.** `method_params` is what makes
`t.bump( by = 5 )` bind by name. Without it the engine refuses named arguments
for that method — deliberately, because binding them in call-site order instead
is a silent wrong answer.
