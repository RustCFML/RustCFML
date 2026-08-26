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

1. `--extensions <dir>`
2. `extensions/` in the application directory — per-app, checked into the
   project, and the common case
3. `~/.rustcfml/extensions/`
4. `extensions/` beside the `rustcfml` binary — system or container image

`rustcfml --verbose` prints what loaded and from where. Anything that fails to
load is reported on stderr, never skipped silently.

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
- **Tier 1 today.** An extension can compute over CFML values and hold its own
  Rust state. It cannot yet read `application`, take a `<cflock>`, or call back
  into CFML. Those are tiers 2 and 3.
- **macOS:** downloaded libraries are quarantined by Gatekeeper and would fail
  to `dlopen` with no explanation; `ext install` clears the attribute for you.
