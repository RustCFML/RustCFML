# Extensions — precompiled Rust, loaded at runtime

An **extension** is a `.rcx` file: precompiled Rust that a stock `rustcfml`
binary loads at start-up and that adds built-in functions and classes to every
request. No engine rebuild, no toolchain on the server, no `--build`.

Most people meet one by **using** something somebody else published. That is two
commands and a restart:

```sh
rustcfml ext install typst-0.1.0-linux-x86_64.rcx --user
rustcfml ext list          # what is installed, and whether it loaded
```

Its functions and classes are then ordinary built-ins, available in every
request with nothing to import:

```cfml
pdf = Document().heading( text = "Invoice INV-1042", level = 1 )
                .toBinary();
```

That path — installing, placing and configuring an extension — is the first half
of this page. [Writing one](#writing-one) is the second half, and you do not need
it to use one.

> **Naming.** The shipped artifact is an **extension** (`.rcx`). The Rust crate
> you write is a **native module**. An extension contains one or more native
> modules plus optional CFML. The older statically linked route is still
> supported and documented in [native-modules.md](native-modules.md).

---

## Why you would want one

Some capabilities are too big to put in the engine for everyone. Typst-backed
PDF authoring is +31.6 MB of binary; a browser engine is far more. Making those
extensions means the base binary stays small and you opt in by installing a
file. [RustCFML-Extension-Typst](https://github.com/RustCFML/RustCFML-Extension-Typst)
is the worked example throughout this page: a published, per-platform extension
you can install and read.

The other reason is distribution. An extension is a single artifact that can
carry libraries for several platforms at once, and it installs into a directory
the engine already searches — including one you can check into your project.

## Installing one

```
rustcfml ext install <file.rcx> [--user | --dir D]
rustcfml ext list                  installed extensions and their load status
rustcfml ext remove <name>
```

`--user` installs into `~/.rustcfml/extensions/`; `--dir` puts it wherever you
say. The alternative to installing at all is to **drop the `.rcx` into your
application's `extensions/` directory and check it into the project**, which
makes the dependency explicit and travels with a deploy. Both are first-class —
see [Where extensions are found](#where-extensions-are-found) for the full
search order.

Two things that are easy to trip over:

- **Restart the engine.** Extensions load once, at process start, and are never
  unloaded. `ext install` says "restart to activate" and means it.
- **Take the archive for your platform.** A published extension ships one
  archive per platform (`…-macos-aarch64.rcx`, `…-linux-x86_64.rcx`, and so on).
  Installing the wrong one fails by name and lists what the archive actually
  contains, rather than loading and misbehaving.

On macOS a downloaded library is quarantined by Gatekeeper and would fail to
`dlopen` with no explanation; `ext install` clears the attribute for you.

`rustcfml ext list` shows what is installed and whether it loads without
starting anything, and `rustcfml --verbose` prints what loaded and from where at
boot. Anything that fails to load is reported on stderr, never skipped
silently.

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

## Discovering what an extension provides

`getFunctionList()` includes extension functions, with the providing extension's
name as the value (compiled-in functions have an empty string). Without that, an
extension's BIFs are callable but invisible to anything enumerating the engine's
functions — an answer that looks authoritative and is wrong.

---

## Which delivery mode

*Everything above is about using an extension someone else built. From here on
you are the author.*

The first decision is whether you want an extension at all, or the older
statically linked route:

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

Scaffold, build, install — the whole loop:

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

`Rooted` is an id, so it is `Send + Sync`: keep it in a class field, a `static`,
a map, and drop it on any thread — releasing needs no `ctx`. Bringing the value
back does: `rooted.get( ctx )` hands you a `Value` for the call in progress,
like any other. (A value that has been released comes back as a null.)

#### Keeping a closure to call later

The value can be a closure, or a component instance — which is how
`page.on( "console", fn )`, a SQL function backed by CFML, or an agent's tool
callback is built. A tier-3 extension keeps it in one call and calls it in a
later one, possibly in a later request:

```rust
"on" => {                                     // emitter.on( fn )
    self.handlers.lock().unwrap().push( ctx.root( args[0] )? );
    Ok( ctx.this() )
}
"emit" => {                                   // emitter.emit( payload ), any later call
    let handlers: Vec<Value> =
        self.handlers.lock().unwrap().iter().map( |h| h.get( ctx ) ).collect();
    for h in handlers {                       // the lock is released before CFML runs:
        h.call_as_fn( &[ args[0] ] )?;        // a handler may call back into this object
    }
    Ok( ctx.null() )
}
```

What the closure sees when it runs is what the same closure kept in
`application` scope sees, because it is the same value:

- **Its own captures are live.** A function's `local` variables it closed over,
  a component's `variables` and `this`: shared with every other holder of the
  closure, and every call sees the last one's changes. They stay alive until
  the last `Rooted` (and every other reference) is dropped.
- **The dynamic scopes are the caller's.** `request`, `session`, `url`, `form`,
  `cgi` and `cookie` belong to the request whose `ctx` calls the closure, not
  the one that created it; `application` is the live application. That is what
  Lucee does when an extension keeps a `UDF` and calls `udf.call( pc, … )` with
  the current `PageContext`, and the only answer that cannot leak one user's
  request into another's.
- **It runs on the calling thread, which is a CFML thread.** A `ctx` exists
  only inside a call, on the thread running that call, and cannot be sent to
  another (`Ctx` is neither `Send` nor `Sync`). So there is no way to run CFML
  from an extension's own threads. Work on another thread — an async runtime, a
  C library's callback — hands the call back to the CFML thread that is waiting
  for it, over a channel, and that thread makes the call. A `cfthread` body
  runs on a CFML thread like any other, with its request's scopes.
- **Its body survives its request.** A function value carries only an id; its
  bytecode lives in the template cache, the request, or the application. A
  root pins the bytecode of every function reachable from the value, so a kept
  closure still runs after the request that compiled it has gone and its
  template has been recompiled or evicted — as one kept in `application` does.

Concurrent calls of one kept closure from several requests run concurrently,
exactly as they would through `application` scope: unsynchronised updates to its
captured variables race in the same way, and `<cflock>` inside the closure (or
around the call) is the remedy, as it is in CFML.

Two places where RustCFML's closures differ from Lucee's apply here as they do
in CFML (they are not specific to extensions): a closure defined at **page
level** loses the page's `variables` when that page's request ends
([known-issues §122](known-issues.md#122)), and a closure called from inside
another closure defined in the same function can lose its own captures
([§123](known-issues.md#123)). Build kept closures in a function — as above, or
in `onApplicationStart` — rather than at page level.

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

When CFML you called throws, the call returns `Err` and the CFML error is left
**pending** in your call. A pending error is what your caller sees when your
call returns — whatever you return, `Err` or `Ok`. First error wins,
deliberately: a precise "no such component" or a lock timeout is worth more than
a generic message wrapped around it, so `?` always propagates the CFML error
exactly as CFML raised it, type, detail and tag context included.

The `Err` itself carries the CFML error's message and type (`custom_type` is
`my.tool` for a `throw( type = "my.tool" )`) on an engine that provides tier 4;
an older one says `[function value] failed`.
Ignoring the `Err` and returning normally does **not** handle the error: it is
still pending, and still raised. To handle it, catch it (tier 4, below).

The error reaches the CFML `catch` around your call, too: if the page that called
your function sits inside `try`, the CFML you called back into throws, and you
propagate, that `catch` runs. (Earlier engines lost the error there when the
extension function was compile-time bound, which every installed one is: the
callee consumed the outer `try`'s handler from inside its own frame and
execution carried on as if nothing had been thrown.)

## Catching CFML errors, named arguments (tier 4)

### Catching an error as data

```rust
match tool.try_call_as_fn( &[ input ] ) {
    Ok( v )  => Ok( v ),
    // The CFML `throw()` becomes data, and the run carries on.
    Err( e ) => Ok( ctx.string( format!( "Error: {} ({})", e.message, e.error_type ) ) ),
}
```

A `try_` call — `ctx.try_call`, `value.try_call_as_fn`, `value.try_invoke` —
returns a CFML error as a `CfmlException` instead of leaving it pending.
Holding one means the error is **handled**: it will not be raised when your call
returns. It carries `error_type`, `message`, `detail`, `extended_info` and
`error_code`, and `info()` is the full struct a CFML `catch ( any e )` would get —
`tagContext`, `stackTrace`, `rootCause` and anything a `throw()` attached. That
is the shape an agent's tool loop needs: a tool that throws becomes a result the
model reads, not the end of the run.

Three ways out of a caught exception:

```rust
Err( e ) => Ok( describe( e ) ),                        // handled
Err( e ) => Err( e.rethrow() ),                         // raised again, unchanged
Err( e ) => Err( Error::custom( "my.ext", "…" ) ),      // replaced by your own
```

`rethrow()` — and `?`, which does the same — puts the ORIGINAL error back exactly
as CFML threw it, so propagating a caught exception loses nothing.

Anything that calls CFML can be caught: `ctx.catching( || … )` runs a closure and
catches what it raises, for the calls with no `try_` twin (`new_component`,
`include`, `component_metadata`, a lock timeout, the named calls below):

```rust
let made = ctx.catching( || ctx.new_component( "services.Mailer", &[] ) );
```

`ctx.take_error()` is the primitive under both: it takes whatever is pending —
say after a plain call whose `Err` you have decided to handle — and returns
`None` when nothing is.

**A request ending is not an exception.** An `abort`, a `cflocation` redirect or
a request timeout cannot be taken: as `catch( any )` cannot stop one in CFML, an
extension cannot keep an aborted or timed-out request running. The `try_` call
still returns `Err` — check `e.ends_request()` — and the request ends when your
call returns, whatever you return. Return promptly.

**Output capture is put back.** If the CFML you called opened a `savecontent`
(or `silent`, or a custom tag) and threw inside it, the capture is closed when
the error reaches you — where Lucee's `finally` blocks would close it — so what
you or the page write afterwards goes where it should.

**Lucee parity.** A Java extension calling `udf.call( pc, … )` gets a
`PageException` it can catch, inspect and swallow; this is the same. For CFML
the only change is the fix above — an error thrown through an extension reaches
the `catch` around it. An extension that does not catch raises exactly what it
raised before, and an extension built before tier 4, which cannot catch,
behaves as it did.

### Named arguments

```rust
// args: { "city": "Paris", "days": 2 } — from a model, a form, a config file
tool.call_as_fn_named( args )?;            // function( city, days = 1 )
service.invoke_named( "forecast", args )?; // a method, the same way
```

The ABI form of `fn( argumentCollection = args )` and of
`invoke( object, method, args )`: a struct's keys bind to parameter names
whatever order they arrive in, an omitted optional takes its default, a key
that matches no parameter lands in `arguments`, and a positive-integer key is
positional, as in CFML. An array is positional. Lucee's equivalent is
`udf.callWithNamedValues`. Named arguments need a user-defined function or
closure; a built-in is refused with a clear error. Catch a named call's errors
with `ctx.catching`.

### Declaring tier 4

`module! { …, tier: abi::tier::EXCEPTIONS, … }`. An engine older than that
refuses the extension at load, naming both tiers. An extension that declares
tier 3 and uses these calls anyway still runs on an older engine: `take_error()`
returns `None` and the `try_` calls behave like the plain ones (the error stays
pending), and a named call returns an error saying which tier it needs.

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

## Database drivers

An extension can back a **datasource**, so a database RustCFML does not bundle
reaches `queryExecute`, `<cfquery>`, `cfqueryparam`, `transaction { }` and
`<cfdbinfo>` through the paths every other datasource uses:

```rust
use rustcfml_module::{module, Ctx, DbDriver, Result, Value};

struct Greptime { /* a client, a pool, whatever it needs */ }

impl DbDriver for Greptime {
    // The driver KIND, as a datasource's `driver` key names it.
    const DRIVER: &'static str = "greptimedb";

    fn new( ctx: &Ctx, args: &[Value] ) -> Result<Self> {
        // args[0] is the datasource's settings: host, port, database, …
        Ok( Greptime { } )
    }

    fn execute<'a>( &self, ctx: &'a Ctx, sql: &str, params: Value<'a>, return_type: &str )
        -> Result<Value<'a>>
    {
        let q = ctx.query( &[ "id", "name" ] )?;
        // … run the statement, add a row per result …
        Ok( q )
    }
}

module! {
    name: "greptime",
    version: "0.1.0",
    drivers: { Greptime },
}
```

```cfml
// Configured, with its own settings — any number of them:
this.datasources.metrics = { driver = "greptimedb", host = "…", database = "…" };
queryExecute( "select * from cpu", [], { datasource = "metrics" } );

// Or straight away, with no configuration, under the driver's own name:
queryExecute( "select * from cpu", [], { datasource = "greptimedb" } );
```

`params` arrives as an array of plain values — the engine flattens
`cfqueryparam` structs first — and `return_type` is `"query"`, `"array"` or
`"struct:<columnKey>"`. **Build the result with the engine's own helpers**
rather than by hand: `build_query_result_columnar`, `build_mutation_result` and
`dedup_result_columns` are what give a bundled driver its Lucee-shaped
`{recordCount, sql, executionTime, generatedKey}` and its duplicate-column
collapse, and hand-rolling those is how a driver quietly diverges.

**Everything past `execute` is optional, and each piece is independent.**
Implement `begin` and `transaction { }` works; leave it out and a transaction
against that datasource says so rather than silently running the statements
unprotected. Implement `dbinfo` and `<cfdbinfo>` works; decline an individual
`type=` by returning `None` and the engine answers it. A driver that only
answers `version`, `tables` and `columns` is a reasonable place to stop — those
are what ORMs actually call.

If your transactions have no savepoints, leave `supports_savepoints` at its
default. A nested `transaction { }` needs none: it joins the transaction around
it, as on Lucee. Only an explicit `action="setsavepoint"` (or
`transactionSetSavepoint()`) then errors. After an in-block `action="commit"`
or `"rollback"`, the engine calls `begin` on your driver again before the next
statement in the block.

Two drivers claiming the same kind is reported before either library is opened,
because the kind is declared in `module.json` — the same pre-load conflict check
that covers BIFs and classes.

A driver instance is built once and shared by every request, so it is
`Send + Sync` and owns its own synchronisation. It is also the one place the
`ctx` is not inside a request: a driver gets tier-1 values and cannot read
scopes or call CFML, which is what you want from something a connection pool
calls.

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

A hyphenated extension name is legal and mounts fine, but the mapping it creates
cannot be used in a bare `new` — `new wheels-core.Formatter()` parses as a
subtraction, not a path (Lucee behaves the same). Use the quoted form, or
`createObject`:

```cfml
o = new "wheels-core.Formatter"();
o = createObject( "component", "wheels-core.Formatter" );
```

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

## Publishing — one archive per platform

Running `ext build` on a second platform **merges** into an existing `.rcx`
rather than replacing it, which is how a single file ends up carrying macOS,
Linux and Windows libraries.

That "fat" shape is right for a file you carry between machines you own, and
wrong for one you publish. Every download then carries four libraries so that
one can be used, and the extensions most likely to be published are the ones
whose size is the reason they are extensions at all — `rustcfml-typst` packages
to 15.3 MB for one platform, so four is ~61 MB of which 75% is unusable on any
given host. Publish **one archive per platform**: a fresh checkout has no prior
`.rcx`, so a per-platform CI job packages exactly its own without needing a flag.
The file name is yours to choose — the loader keys on the triples inside
`module.json`, not on what the file is called — and installing the wrong one
fails by name, listing what the archive does contain.
[the Typst extension's release workflow](https://github.com/RustCFML/RustCFML-Extension-Typst/blob/main/.github/workflows/release.yml)
is the worked example.

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
point takes a `ctx` from day one. Scope access (tier 2), CFML execution (tier 3)
and catchable errors with named arguments (tier 4) therefore *appended* entries
rather than changing any signature: an extension published earlier keeps
working, and an extension built against a newer ABI degrades on an older engine
with a legible message instead of jumping through uninitialised memory.

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
- **Tier 4 today** — the full surface. An extension can compute over CFML values,
  hold Rust state, read and write scopes, take the locks CFML takes, keep values
  — closures included — alive across requests, and **run CFML**: call functions
  and closures (by position or by name), instantiate components, invoke methods,
  inject dependencies, read metadata, write page output and include templates,
  and catch, inspect and handle or re-raise the errors that CFML raises.
- **No signing yet.** Manifest digests catch a corrupted download, not a hostile
  author.
- **macOS:** downloaded libraries are quarantined by Gatekeeper and would fail
  to `dlopen` with no explanation; `ext install` clears the attribute for you.
