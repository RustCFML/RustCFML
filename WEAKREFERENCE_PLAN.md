# Plan — shim `java.lang.ref.WeakReference` (next Preside boot blocker)

> Handoff doc (untracked). Engine @ v0.369.0. This is the next Preside serve-boot
> blocker after the JSON-schema validator (v0.369). It is a **trivial mirror** of
> the existing `SoftReference`/`ReferenceQueue` shim (v0.355, GH #218).

## The blocker
Clean-DB boot of `readyintelligencewebsite` (vs `pcms_ritest`) now 500s on:
```
createObject: Java class [java.lang.ref.WeakReference] is not supported.
```

## Where Preside uses it
Two identical `_createWeakReference()` helpers, both:
```cfml
return CreateObject( "java", "java.lang.ref.WeakReference" ).init( arguments.target );
```
- `system/services/webflow/spec/WebflowSpecLibrary.cfc:344`
- `system/services/datamanagerflow/spec/DatamanagerWorkflowSpecLibrary.cfc:121`

Usage pattern: construct → `.init(referent)` → store → later `.get()` to read the
referent back (and possibly `.clear()`). Semantically **identical to
SoftReference** — a reference holder. We hold the referent strongly and never
clear it on our own (no GC), exactly like the SoftReference shim.

## The model to mirror
`handle_java_softreference` (`crates/cfml-vm/src/java_shims.rs:1252`) already
implements every method WeakReference needs:
`init` (stores `__referent`), `get` (returns it), `clear` (nulls it),
`isEnqueued`/`enqueue` (false), `hashCode` (stable per-instance via the struct's
Arc backing pointer). WeakReference behaviour is the same.

## Implementation (4 wiring points + 1 handler)

### 1. Handler — `crates/cfml-vm/src/java_shims.rs`
Avoid copy-paste: extract the SoftReference body into a shared inner that takes
the class name, then have both call it.
```rust
fn handle_java_reference(class: &str, method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // ... existing softreference body, but use `class` for __java_class on init ...
}
pub fn handle_java_softreference(method, args, object) -> CfmlResult {
    handle_java_reference("java.lang.ref.softreference", method, args, object)
}
pub fn handle_java_weakreference(method, args, object) -> CfmlResult {
    handle_java_reference("java.lang.ref.weakreference", method, args, object)
}
```
(Minimal alternative if you don't want the refactor: a `handle_java_weakreference`
that is a verbatim copy with `"java.lang.ref.weakreference"` as the stored class.)

### 2. Construction arm — `crates/cfml-vm/src/lib.rs` (~line 11510, beside softreference)
```rust
"java.lang.ref.weakreference" => {
    java_shims::handle_java_weakreference("init", empty_args, &CfmlValue::Null)
}
```

### 3. Member-dispatch arm — `crates/cfml-vm/src/lib.rs` (~line 16251, beside softreference)
```rust
"java.lang.ref.weakreference" => {
    java_shims::handle_java_weakreference(&m, all_args, object)
}
```

### 4. `map_getter_owns_null` guard — `crates/cfml-vm/src/lib.rs` (~line 16304)
`WeakReference.get()` may legitimately return null (a null/cleared referent); mark
that null authoritative so it isn't treated as "method unhandled" and fall
through. Add:
```rust
|| (java_class == "java.lang.ref.weakreference" && method_lower == "get")
```

## Test (regression)
`tests/java_shims/test_classloader_shims.cfm` already covers SoftReference — add a
WeakReference block there (or a new `tests/java_shims/test_weakreference.cfm`),
RustCFML-gated (no JVM on the Lucee box):
```cfml
ref = createObject( "java", "java.lang.ref.WeakReference" ).init( { a = 1 } );
assert( "WeakReference.get() returns the referent", ref.get().a, 1 );
ref.clear();
assertTrue( "WeakReference.get() is null after clear()", isNull( ref.get() ) );
```
Register it in `tests/runner.cfm`.

## Verify
1. `cargo build --release`
2. Full gate: `cargo run -- tests/runner.cfm` (CLI), serve cold+warm, `cargo test
   --workspace` (+ JIT 76), wasm32, `wasm-pack build crates/wasm --target web`.
3. Re-boot Preside (reset `pcms_ritest`, boot `readyintelligencewebsite`) and
   confirm it advances PAST WeakReference to the next blocker.

## Effort / risk
~15 min, very low risk — it reuses a proven shim. Expect the boot to advance to
the next createObject/JVM blocker (keep peeling: this session went BCrypt →
snakeyaml → json-schema+putAll → WeakReference). Watch for the next one and
decide shim-vs-out-of-scope case by case (pure-data/algorithm libs are shimmable;
anything that loads & executes real JVM bytecode, e.g. the CronUtil OSGi JAR
loader, is genuinely out of scope).
