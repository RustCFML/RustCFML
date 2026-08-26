# Native (Rust) Modules

> **There is now a second way to do this.** A `.rcx` **extension** is
> precompiled Rust that a *stock* `rustcfml` binary loads at start-up — no
> `--build`, no engine checkout to compile against, and no need to match the
> engine's rustc. See [extensions.md](extensions.md). This page documents the
> statically linked route, which is still supported and is the better choice
> when you want a single self-contained binary.


[← Back to README](../README.md)

Self-contained binaries can include user-authored Rust code that surfaces as first-class CFML built-in functions and classes. When `rustcfml --build` finds a `native/<crate>/Cargo.toml` inside your app dir, it generates a Cargo workspace and compiles your modules into the binary alongside the CFML.

```
myapp/
├── main.cfm
└── native/
    └── greeter/
        ├── Cargo.toml
        └── src/lib.rs   — pub fn register(vm: &mut Vm)
```

In `src/lib.rs`:

```rust
use rustcfml_cli::{CfmlNative, CfmlResult, Value, Vm};

pub fn register(vm: &mut Vm) {
    vm.register_native_fn("rustGreet", |args| {
        let name = args.get(0).map(|v| v.as_string()).unwrap_or_default();
        Ok(Value::String(format!("Hello, {}", name)))
    });
    vm.register_native_class("Tally", tally_new);
}
```

In your CFML:

```cfml
writeOutput(rustGreet("Alex"));         // Hello, Alex
counter = createObject("rust", "Tally");
counter.bump();
```

## Building

```bash
rustcfml --build ./myapp -o myapp --mode cli --entry main.cfm
```

When a project contains `native/<crate>/Cargo.toml`, the build runs the "cocktail" path: it generates a synthetic Cargo workspace under `.rustcfml-cocktail/`, path-deps on `rustcfml-cli` plus each user module, shells out to `cargo build --release`, then appends the application VFS archive to the produced binary. Plain CFML apps with no `native/` directory keep the toolchain-free bundling path.

**Requirements:** `cargo`/`rustc` on `PATH` at build time (the standard Rust toolchain — install from [rustup.rs](https://rustup.rs/)). End users running the produced binary need nothing extra.

## CFC inheritance from a Rust class

A CFC can extend a registered Rust class:

```cfml
component extends="rust:Tally" {
    function init() { super(); }
}
```

`super(args)` re-runs the registered constructor; `super.method()` and unqualified method fall-through both reach the native parent; and `this.X` reads/writes route through the native object's property accessors when the CFC has no such key.

## Reference

- Working example: [`examples/native_module_demo/`](../examples/native_module_demo/) — includes the full module-author contract in its README.
- The `CfmlNative` trait and registration API are defined in `cfml-common/src/dynamic.rs` and `cfml-vm/src/lib.rs`.
