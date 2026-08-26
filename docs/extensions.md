# Extensions — precompiled Rust, loaded at runtime

An **extension** is a `.rcx` file: precompiled Rust that a stock `rustcfml`
binary loads at start-up and that adds built-in functions and classes to every
request. No engine rebuild, no toolchain on the server, no `--build`.

```sh
rustcfml ext new mything          # scaffold a crate that already compiles
cd mything && rustcfml ext build .
rustcfml ext install mything-0.1.0.rcx --user
```

```cfml
writeOutput( mythingGreet( "there" ) );
t = mythingTally();
writeOutput( t.bump( by = 5 ).value() );
```

> **Naming.** The shipped artifact is an **extension** (`.rcx`). The Rust crate
> you write is a **native module**. An extension contains one or more native
> modules plus optional CFML. The older statically linked route is still
> supported and documented in [native-modules.md](native-modules.md).

---

## Why you would want one

Some capabilities are too big to put in the engine for everyone. Typst-backed
PDF authoring is +31.6 MB of binary; a browser engine is far more. Making those
extensions means the base binary stays small and you opt in by installing a
file.

The other reason is distribution. An extension is a single artifact that can
carry libraries for several platforms at once, and it installs into a directory
the engine already searches — including one you can check into your project.

## Which delivery mode

| | `.rcx` extension | `--build` cocktail |
|---|---|---|
| Engine binary | stock | bespoke, built by you |
| Needs Rust on the target | no | no (built elsewhere) |
| Needs an engine checkout to build the module | **no** | yes |
| Must match the engine's rustc | **no** | yes |
| Deployment | drop in a file | ship a new binary |
| Call overhead | ~18 ns/call over a compiled-in builtin | none |

Both are supported and both are tested. Prefer `.rcx` unless you specifically
want one self-contained binary.

---

## Writing one

`rustcfml ext new` writes a crate that compiles as-is. The shape:

```rust
use rustcfml_module::{module, Ctx, Error, NativeClass, Result, Value};

fn greet<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let who = match args.first() {
        Some(v) if !v.is_null() => v.to_string(),
        _ => "World".to_string(),
    };
    Ok(ctx.string(format!("Hello, {who}")))
}

module! {
    name: "mything",
    version: "0.1.0",
    bifs: { "mythingGreet" => greet },
    classes: { Tally },
    on_load: warm_caches,
}
```

### `Value` is a borrowed handle

Your function never sees a Rust value the engine owns. `Value` is a handle; you
pay one indirect call per field you actually touch, and a 10,000-row query
passed as an argument is never copied. The lifetime ties a `Value` to the call
it came from, so it cannot be stored across requests.

```rust
let total: i64 = (0..arr.len()?).map(|i| arr.get(i).as_i64().unwrap_or(0)).sum();
let name = row.key( "name" ).to_string();
```

For a query, prefer `query_column(i)` over looping `query_cell(r, c)`: it
materialises a whole column in **one** crossing rather than one per row.

### Classes take `&self`

```rust
impl NativeClass for Tally {
    const CLASS_NAME: &'static str = "Tally";

    fn new(_ctx: &Ctx, _args: &[Value]) -> Result<Self> { … }

    fn method_params(method: &str) -> Option<&'static str> {
        match method { "bump" => Some("by"), "value" => Some(""), _ => None }
    }

    fn call<'a>(&self, ctx: &'a Ctx, method: &str, args: &[Value<'a>]) -> Result<Value<'a>> {
        match method {
            "bump"  => Ok(ctx.int(self.count.fetch_add(1, SeqCst) + 1)),
            "reset" => { self.count.store(0, SeqCst); Ok(ctx.this()) }
            other   => Err(Error::new(format!("no method [{other}]"))),
        }
    }
}
```

Three things to notice:

- **`&self`, not `&mut self`.** Interior mutability is required from day one,
  because it is what lets the engine dispatch your methods without holding an
  exclusive lock once CFML re-entry arrives. Use a `Mutex` or an atomic.
- **`ctx.this()`** returns the *receiver*, so a fluent mutator hands back the
  same object rather than a copy. Your module has no handle to itself; the
  engine substitutes one. Returning it from a plain BIF is an error, not a null.
- **`method_params`** declares the parameter names so a named call
  (`t.bump( by = 5 )`) binds by name. Returning `None` makes the engine
  **refuse** named arguments for that method rather than binding them by
  position — which would be a silent wrong answer.

An instance is an ordinary native object as far as the engine is concerned, so
`createObject( "rust", "Tally" )`, `component extends="rust:Tally"`,
`super.method()` and `this.X` fall-through all work with no extra plumbing.

### Errors and panics

Return `Err(Error::…)` and it becomes a CFML error your caller can `cfcatch`.
`Error::custom( "my.type", "…" )` sets a custom type. A **panic** inside an
extension is caught at the boundary and reported as a CFML error naming the
function — an extension bug must not take the process down.

### `on_load`

Runs **once per process**, after the host's service table is installed:
thread pools, font enumeration, caches. `register` runs once per VM — every
request and every `cfthread` child — so nothing expensive may happen there.

---

## Seeing the running application (tier 2)

An extension can read and write the engine's scopes, take the same locks CFML
takes, and keep values alive across requests.

```rust
fn memoise<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let app = ctx.scope( "application" );

    // Reads take the scope's read lock, so this can never see a half-written
    // value — no ceremony required.
    let hit = app.get( "answer" )?;
    if !hit.is_null() { return Ok( hit ); }

    // Writes to a SHARED scope require a lock you are holding.
    let guard = ctx.lock( "application", true, 10_000 )?;
    let hit = app.get( "answer" )?;          // re-check: without this every
    if !hit.is_null() { return Ok( hit ); }  // racing caller still computes
    let computed = ctx.int( 42 );
    app.set( "answer", computed )?;
    drop( guard );
    Ok( computed )
}
```

### Scopes

`ctx.scope(name)` reaches `variables`, `request`, `session`, `application`,
`server`, and read-only `cgi` / `url` / `form` / `cookie`. It offers `get`,
`set`, `has`, `remove` and `snapshot`. `ctx.var(key)` is an **unqualified** read
that goes through the engine's own resolver, so it sees exactly what an
unprefixed read in CFML source would.

`snapshot()` returns a **copy**, and there is deliberately no live iterator:
walking a live shared scope key by key while another request writes it is a
race, not a convenience.

### Writing a shared scope requires its lock

`application`, `session` and `server` are live and shared across concurrent
requests, so an unlocked write is refused:

```
writing [application] requires holding its lock — take ctx.lock("application", …) first
```

This is **stricter than CFML itself**, on purpose. CFML lets you write
`application.x` unlocked; an extension may not, because it can do so from a
thread the application never thinks about, and because the failure mode is
corruption rather than an error. `variables` and `request` belong to one request
and need no lock.

### Locks are the engine's locks

`ctx.lock(scope, exclusive, timeout_ms)` and `ctx.lock_named(...)` take locks
from **the same registry `<cflock>` uses**. A `<cflock scope="application">` in a
CFML page and a native write mutually exclude — a separate native lock table
would look correct and protect nothing. Same reentrancy (a nested acquire by the
same request succeeds), the same `timeout_ms = 0` means wait forever, and a
timeout raises the engine's own `lock`-typed error carrying
`LockOperation = "Timeout"`.

Guards are **call-scoped**: anything still held when your call returns is
force-released and logged, because a module holding a lock into the next request
is a hang, not a bug report. Drop the guard when you are done anyway, so CFML
code waiting on the scope is not blocked for the rest of your function.

### Values that outlive a call

```rust
let rooted = ctx.root( value )?;   // keep it
let back   = rooted.get( ctx );    // in a later call
                                   // dropped => released
```

Rooted values are visible to the cycle collector, so a cache cannot be collected
out from under you. The flip side: a `Rooted` you never drop is a leak for the
life of the process.

### Checking the host is new enough

An extension built against tier 2 and loaded by a tier-1 engine gets a legible
error the first time it reaches for a scope, not a jump through an unfilled
vtable slot — that is what `size` and `ctx`-from-day-one bought. Declare the
tier you need in `module!` and the loader refuses a too-old host up front.

---

## Running CFML (tier 3)

```rust
// A closure the page handed you — the mechanism behind
// `thing.onEvent( function(e){ … } )`.
let doubled = callback.call_as_fn( &[ ctx.int( 21 ) ] )?;

// A builtin, or any UDF in scope.
let shouted = ctx.call( "ucase", &[ ctx.string( "hi" ) ] )?;

// A component: construct, inject, invoke.
let svc = ctx.new_component( "services.Mailer", &[] )?;
svc.set_property( "transport", my_transport )?;
svc.invoke( "send", &[ message ] )?;

// What annotation-driven DI is built on.
let meta = ctx.component_metadata( "services.Mailer" )?;

ctx.write_output( "straight to the page" )?;
ctx.include( "/views/partial.cfm" )?;
```

### Re-entrancy is the whole difficulty

An extension method that calls CFML which calls back into **the same object** used
to deadlock: dispatch held the object's exclusive lock for the entire call, and
the re-entry waited for it. A dependency container resolving a bean whose
provider resolves another bean from the same container is exactly that shape, so
this is the main line, not a corner case.

The fix is an opt-out. `CfmlNative::needs_exclusive()` defaults to `true`, so
**every class implemented inside the engine is untouched** — it still gets
`&mut self` and the guard. An extension class returns `false`: dispatch takes
only a shared lock and calls `call_method_shared`, so several frames of the same
object can be live at once. That is sound because the ABI already requires a
module to manage its own synchronisation, and the wrapper's `&self` method
signature enforces it. Nothing takes the exclusive lock for such an object, so
nested shared acquisitions cannot be starved by a waiting writer.

`ctx.this()` is a value, not just a return: passing it to `invoke` is how a
method re-enters itself through the engine.

### `set_property` writes `variables`, not `this`

Injecting a dependency means writing the component's **`variables`** scope,
because that is what its own methods read. Writing the public member instead
compiles, runs, and leaves the component seeing nothing — a silent no-op rather
than an error. This is the one place the ABI deliberately does not mirror
`struct_set`.

### `include` is not `<cfinclude>`

`<cfinclude>` merges the included file's new variables back into the *calling
frame's* locals. An extension has no calling frame to merge into, so
`ctx.include` runs the template with a fresh scope and discards what it defines;
its output goes to the buffer. Well defined, rather than guessing whose
`variables` scope to touch.

### Errors from re-entrant CFML

When CFML you called throws, that error is what your caller sees — the module's
own `Err` on the way out does not replace it. First error wins, deliberately: a
precise "no such component" or a lock timeout is worth more than a generic
"[newComponent] failed" wrapped around it.

---

## SQL functions

An extension can add functions to Query-of-Queries:

```rust
module! {
    …
    qoq_scalars:    { "slugify" => sql_slugify },
    qoq_aggregates: { "median"  => sql_median  },
}
```

```cfml
queryExecute( "SELECT SLUGIFY( title ) AS slug FROM posts", {}, { dbtype = "query" } );
queryExecute( "SELECT MEDIAN( views ) AS mid FROM posts",   {}, { dbtype = "query" } );
```

A **scalar** is called once per row with that row's values. An **aggregate** is
called once per partition, and each SQL argument arrives as an **array** of that
argument's value across every row — the part people get wrong. Both are also
registered as ordinary BIFs, so `slugify( "…" )` works from CFML too.

An extension's SQL function is never assumed pure, so it is not hoisted out of a
row loop: it may read a scope or call CFML.

## Shipping CFML with an extension

A `.rcx` can carry a `cfml/` directory, which the engine mounts as
`/<extension-name>/`:

```
demo-0.1.0.rcx
└── cfml/Formatter.cfc      →  createObject( "component", "demo.Formatter" )
```

Nothing in the application declares that mapping. This is what makes a `.rcx` an
*extension* rather than a plugin: a Rust core can present a CFML facade, which
is usually the nicer API anyway.

The mapping is **server-level and re-applied after anything that replaces the
mapping set** — `Application.cfc`'s `this.mappings`, the application lifecycle, a
thread seed, `application action="update"`. All four replace wholesale, and an
extension's CFCs are not the application's to drop.

## Discovering what an extension provides

`getFunctionList()` includes extension functions, with the providing extension's
name as the value (compiled-in functions have an empty string). Without that, an
extension's BIFs are callable but invisible to anything enumerating the engine's
functions — an answer that looks authoritative and is wrong.

---

## The command line

```
rustcfml ext new <name> [dir]      scaffold an extension crate
rustcfml ext build [dir]           build the cdylib and package the .rcx
rustcfml ext install <file.rcx> [--user | --dir D]
rustcfml ext list                  installed extensions and their load status
rustcfml ext remove <name>
```

`ext build` reads the built library's own declaration to write the manifest, so
the manifest can never disagree with the code — including the extension's name,
which comes from `module!`, not from the crate name.

Running `ext build` on a second platform **merges** into an existing `.rcx`
rather than replacing it, which is how a single file ends up carrying macOS,
Linux and Windows libraries.

## Where extensions are found

First hit wins per extension name:

1. `--extensions <dir>` on the command line
2. `extensions.directory` from the server `.cfconfig.json`
3. `extensions/` in the application directory — per-app, checked into the
   project, and the common case
4. `~/.rustcfml/extensions/`
5. `extensions/` beside the `rustcfml` binary — system or container image

`rustcfml --verbose` prints what loaded and from where; `rustcfml ext list`
shows the same without starting anything. Anything that fails to load is
reported on stderr, never skipped silently.

## Configuring them — `.cfconfig.json`

```json
{
  "extensions": {
    "directory": "lib/extensions",
    "enabled":  [],
    "disabled": ["browser"],
    "settings": {
      "typst": { "fontDir": "/usr/share/fonts" }
    }
  }
}
```

| key | meaning |
|---|---|
| `directory` | an extra directory, searched before the built-in locations |
| `enabled` | when non-empty, **only** these load, by declared name |
| `disabled` | names to skip; applied after `enabled` |
| `settings` | per-extension config, handed to that extension's `on_load` as a struct |

`settings` is the delivery mechanism for anything an extension needs to know at
start-up — a font directory, a pool size, an endpoint:

```rust
fn on_load(_ctx: &Ctx, settings: Value) -> Result<()> {
    if let Ok(dir) = settings.key( "fontDir" ).as_str() { … }
    Ok(())
}
```

**This is read from the SERVER-level `.cfconfig.json` only.** Extensions load
once per process, before anything is compiled; by the time a per-application
config is resolved the extension is already in the process, and there is no
unload. A per-app `extensions` block is therefore ignored rather than
half-honoured.

> Lucee uses this same key for its `.lex` extension list, which is an *array*.
> A config exported from Lucee or CommandBox still parses — that shape is
> accepted and ignored.

## Inside a `.rcx`

```
typst-0.1.0.rcx
├── module.json          name, version, abi_major, tier, declared bifs/classes,
│                        exclusive capabilities, sha256 of each library
├── lib/
│   ├── aarch64-apple-darwin/libtypst.dylib
│   └── x86_64-unknown-linux-gnu/libtypst.so
├── cfml/                optional CFCs and custom tags shipped with it
└── README.md, LICENSE
```

Because the manifest declares what the extension provides, the engine can report
a conflict (`greet is provided by both A and B`) **without loading anything**,
and can refuse a second provider of an exclusive capability — which matters for
anything that initialises a process-global runtime, where two providers is a
crash rather than an ambiguity.

---

## The contract, and what it costs

The boundary is a **C ABI over opaque handles**. Your crate links neither the
engine nor its allocator, sees no Rust type layout, and does not care which
rustc built the engine. The compatibility token is just
`{ABI_MAJOR}|{target-triple}`; a mismatch is refused at load with both values
printed.

The host's service table is **size-versioned and append-only**, and every entry
point takes a `ctx` from day one. Scope access (tier 2) and CFML execution
(tier 3) will therefore *append* entries rather than change any signature: an
extension published today keeps working, and an extension built against a newer
ABI degrades on an older engine with a legible message instead of jumping
through uninitialised memory.

Measured on an M-series Mac, per call:

| | ns/call |
|---|---|
| compiled-in builtin (bound at compile time) | ~130 |
| **extension BIF** | **~148** |
| CFML user-defined function | ~490 |

**An extension's BIF is bound at compile time, exactly like a compiled-in one.**
Extensions load once, at process start, before anything is compiled, and are
never unloaded — so codegen can emit a direct `CallBuiltin` for their names and
skip the whole generic dispatch path (a `LoadGlobal`, the
locals/`variables`/globals chain walk, a per-call `to_lowercase`, and the
intercept chain). That is worth **~180 ns per call**: before it, an extension
BIF cost ~325 ns; the ABI crossing was never the expensive part.

What is left is the crossing itself: **~18–34 ns fixed, plus ~16 ns per
argument you actually read**. If a compiled template ever runs in a process
without the extension it was compiled against, the `CallBuiltin` handler falls
back to generic resolution — so the worst case is the old speed, never a wrong
answer.

## Limits you should know about

- **Extensions are never unloaded.** Their function pointers live in VM
  registries and their objects can outlive any request, so `dlclose` while
  either is alive is undefined behaviour. `ext install` says "restart to
  activate" and means it.
- **An extension is trusted code**, exactly like a Lucee `.lex`: arbitrary
  native code with full process privilege. The manifest digests protect against
  a corrupted download, not a hostile author. This is not a sandbox.
- **Tier 3 today** — the full surface. An extension can compute over CFML values,
  hold Rust state, read and write scopes, take the locks CFML takes, keep values
  alive across requests, and **run CFML**: call functions and closures,
  instantiate components, invoke methods, inject dependencies, read metadata,
  write page output and include templates.
- **No signing yet.** Manifest digests catch a corrupted download, not a hostile
  author.
- **macOS:** downloaded libraries are quarantined by Gatekeeper and would fail
  to `dlopen` with no explanation; `ext install` clears the attribute for you.
