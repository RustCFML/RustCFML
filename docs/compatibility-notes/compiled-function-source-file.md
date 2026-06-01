# Cached compiled functions should retain source file metadata

## Summary

This is a VM-internal compatibility issue found during the Moopa port. It is not
currently exposed as a clean CFML runner test.

When `compile_file_cached()` compiles a CFML/CFC file, functions in the returned
`BytecodeProgram` should retain the source file path they came from. Cached
component/application functions may later need that metadata when they are
restored or rebound into a request program.

## Observed Behavior

Functions produced through cached file compilation did not have `source_file`
populated. That left later function restoration/rebinding paths without the
original file identity for a cached function.

## Expected Behavior

After compiling a source file through `compile_file_cached(path, ...)`, every
function originating from that file should have:

```text
function.source_file == Some(path)
```

## Suggested Fix Shape

After `compiler.compile(ast)` returns the `BytecodeProgram`, iterate over the
program functions and assign the source path before caching/returning it:

```rust
let mut program = compiler.compile(ast);
for func in &mut program.functions {
    Arc::make_mut(func).source_file = Some(path.to_string());
}
```

## Moopa Port Context

This came up while debugging cached component/application function behavior.
Moopa exercises a lot of application-scoped functions and cached component
methods, so stale or incomplete function metadata can surface as incorrect
rebinding behavior after cached code is reused.

## Why This Is Markdown-Only

The useful assertion is against RustCFML VM metadata, not a direct CFML output.
I did not find a clean CFML-only runner test that observes this invariant
without depending on surrounding cache/rebinding internals.

This PR therefore includes a focused Rust integration test:

```text
crates/cfml-vm/tests/compiled_function_source_file.rs
```

The test compiles a CFC through `compile_file_cached()` and asserts that the
compiled `read` function has `source_file` set to the CFC path.
