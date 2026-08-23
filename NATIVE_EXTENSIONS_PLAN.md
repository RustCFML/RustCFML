# Native Extensions Plan — precompiled Rust modules, loaded at runtime

**Goal:** the Lucee-`.lex` experience for RustCFML. Ship a Rust module as a *file* that a stock
`rustcfml` binary loads at startup — no engine recompile, no special binary. Architecture-locked
(same triple as the host) is accepted up front.

**Decided:** a **stable C ABI with opaque value handles** (§3 option B), not the Rust-ABI approach.
**Decided:** dynamic loading is the primary delivery mode; the compiled-in cocktail path stays as
a secondary option.
**Decided:** modules that touch shared live scopes both **take** locks and **observe** locks —
one lock registry shared with `<cflock>`, explicit guard required for writes (§5.2b).
**Decided:** tier 1 exposes the **full** `CfmlValue` surface (§4.3), and **`.rcx` packaging ships
before scope access** (§7) — the ABI's nouns are settled before anyone publishes against them.

**The single most important design constraint** is in §4.2: every entry point takes a `ctx` handle
from day one, even though phase 1 gives that ctx almost no capabilities. That reserved seam is what
lets scope access and CFML re-entry arrive later **without breaking a single published extension**.

---

## 1. Where we are today

The native-module system exists and works; the *delivery* is static and the *capability* is narrow.

| Piece | Location | Shape |
|---|---|---|
| `CfmlNative` trait | `cfml-common/src/dynamic.rs:1378` | `Send + Sync + Debug`; `class_name`, `call_method`, `get_property`, `set_property` |
| Native object value | `cfml-common/src/dynamic.rs:1470` | `CfmlValue::NativeObject(Arc<RwLock<dyn CfmlNative>>)` (`std::sync::RwLock`) |
| BIF registration | `cfml-vm/src/lib.rs:4433` | `register_native_fn(&mut self, name: &str, f: BuiltinFunction)` |
| Class registration | `cfml-vm/src/lib.rs:4385` | `register_native_class(&mut self, name: &str, ctor: NativeConstructor)` |
| QoQ registration | `cfml-vm/src/lib.rs:4456` | `register_native_qoq_fn(&mut self, name, f: QoQFn, kind)` |
| Function types | `lib.rs:122`, `:2451`, `cfml-qoq/src/function.rs:22` | all three are `fn(Vec<CfmlValue>) -> CfmlResult` — **bare fn pointers** |
| BIF storage | `lib.rs:1646` + index at `:1663`, `:4405` | `HashMap<String, BuiltinFunction>`, `.copied()` at ~8 dispatch sites |
| Module contract | `examples/native_module_demo/native/greeter/src/lib.rs:13` | `pub fn register(vm: &mut Vm)`, crate-type `rlib` |
| Registrar hook | `cli/src/lib.rs:122-161` | `static REGISTRAR: OnceLock<Box<dyn Fn(&mut Vm) + Send + Sync>>` |
| Registrar applied | `cli/src/lib.rs:852`, in `register_vm_runtime` | after the stdlib, **once per VM** — every request and every cfthread child VM |
| Delivery | `--build` cocktail (`cli/src/lib.rs:3280`) | `discover_native_modules` → generate `.rustcfml-cocktail/` → path-dep `rustcfml-cli` → `cargo build --release` → append VFS archive |

### 1.1 The capability gap, stated plainly

**A native module today cannot touch the running application.** `BuiltinFunction` and
`CfmlNative::call_method` are pure functions over values — no `&mut Vm`. So a module cannot read
`application`, cannot write `session`, cannot call a CFML closure, cannot instantiate a CFC, cannot
read component metadata. Scope and re-entry are reserved for engine-internal "VM-intercepted"
builtins in `lib.rs::call_function`, which have `&mut self`.

This is a **pre-existing limitation of the static system**, not something dynamic loading
introduces. Any ambition beyond leaf computation — a caching layer, a DI container, a framework
kernel — needs a VM context handed to native code. That work is described in §5 and is genuinely
the larger half of this plan.

### 1.2 What the seam already gives us for free

`set_registrar` stores a `Box<dyn Fn(&mut Vm) + Send + Sync>` in a `OnceLock`, and
`register_vm_runtime` (`cli/src/lib.rs:852`) applies it to every VM it constructs. **The dylib
loader needs no new hook in the VM** — it calls `set_registrar` with a closure over the loaded
modules, exactly as the generated cocktail `main.rs` does today. Both delivery modes converge on
one code path.

Nothing in the tree does dynamic loading today (`libloading` appears only transitively under
`clang-sys`). This is greenfield.

---

## 2. Why Lucee gets this for free and we don't

A `.lex` is a zip of JARs. The JVM has a **stable ABI** (class files) and **OSGi** gives versioned
bundles, isolation, and hot deploy. Rust has none of the three: no stable ABI, no runtime linker
that understands Rust semantics, no safe unload.

The C ABI approach buys back the first. The other two we do not get:

- **No hot deploy.** Extensions load once at boot; `ext install` says "restart to activate" (§6.2).
- **No write-once-run-anywhere.** A `.rcx` carries per-triple payloads (§4.9); wasm is the eventual
  portable tier (§3 option C).

Both belong in the docs up front, not discovered later.

---

## 3. Mechanism options and the decision

| | How | Perf | Portability | Verdict |
|---|---|---|---|---|
| A. cdylib, Rust ABI, compatibility-token gated | plugin sees real `CfmlValue`, `Vec`, `dyn CfmlNative` | native, zero marshalling | breaks on any rustc bump; layout is the contract | **rejected** |
| **B. cdylib, C ABI with opaque value handles** | plugin never sees Rust layout; host accessor vtable | ~1 indirect call per field touched | survives toolchain bumps; `ABI_MAJOR` is the only gate | ✅ **chosen** |
| C. Wasm (wasmtime / component model) | `wasm32` payload, sandboxed | ~2–5× boundary cost; no native deps or threads | true write-once-run-anywhere | complementary tier, later |
| D. Out-of-process IPC actor | subprocess + pipe | per-call IPC kills fine-grained BIFs | total | only for `!Send` monsters (cf. Obscura browser actor) |

**Why B is the right call here**, recorded so we don't relitigate it:

- The perf argument for A is weaker than it looks. B costs roughly one indirect call per value
  *touched* (~2–5ns) against a VM already spending ~164ns per op. On a BIF worth writing in Rust
  that is noise; on `rustAdd(a,b)` it's maybe 10%, and `rustAdd` was never the point.
- B's ergonomic cost is **recoverable by a wrapper crate** (§4.7). The raw `extern "C"` surface is
  not what module authors write.
- B **eliminates three of the five landmines** the Rust-ABI design carried:
  - *Allocator split* (host runs mimalloc via `#[global_allocator]` at `cli/src/main.rs:40`; a cdylib
    is its own crate graph and would default to `System` → cross-allocator free → heap corruption).
    Under B every `CfmlValue` is host-allocated and host-owned. Nothing crosses ownership. Gone.
  - *Duplicated statics / `TypeId` mismatch* — the module links neither `cfml-common` nor `cfml-vm`. Gone.
  - *`dyn CfmlNative` vtable layout as contract* — replaced by a `#[repr(C)]` vtable the module
    supplies (§4.6). Adding a defaulted method to `CfmlNative` stops being a breaking change. Gone.
- For a long-lived, complex module (a framework kernel), being independent of the exact rustc
  version is worth far more than the marshalling it costs.

**Consequence worth naming:** B also removes the need for `LAYOUT_REV` and for `cfml-common` to
become a shared dylib. Two previously-open questions are simply answered by the choice.

---

## 4. The design

### 4.1 The ABI crate

New crate `crates/cfml-module-abi` — `#[repr(C)]` types, **zero dependencies, zero mutable statics**
(enforce with a test). Depended on by both host and module.

```rust
pub const ABI_MAJOR: u32 = 1;

/// Opaque, host-owned value. Index + generation tag: a stale handle is caught
/// and reported, never dereferenced (§4.4).
#[repr(C)] #[derive(Copy, Clone)]
pub struct ValueHandle { pub slot: u32, pub gen: u32 }

/// Opaque per-call VM context. Valid ONLY for the duration of the call that
/// received it. Carries its own generation tag so a stored-and-reused ctx is a
/// clean error rather than UB.
#[repr(C)] pub struct Ctx { _private: [u8; 0] }
```

### 4.2 The `ctx` seam — the load-bearing decision

**Every module entry point takes `ctx` as its first parameter, from phase 1, even though phase 1's
ctx can do almost nothing.**

```rust
pub type ModuleFn = unsafe extern "C-unwind" fn(
    ctx: *mut Ctx,
    args: *const ValueHandle,
    argc: usize,
) -> ValueHandle;
```

And the host vtable is **size-versioned and append-only**:

```rust
#[repr(C)]
pub struct HostVtable {
    pub size: usize,        // sizeof(HostVtable) as the HOST sees it
    pub abi_major: u32,
    pub tier: u32,          // capability tier this host implements (§4.3)
    // --- tier 1: values, errors, registration ---
    pub val_type:    unsafe extern "C" fn(*mut Ctx, ValueHandle) -> u32,
    pub val_i64:     unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut i64) -> u32,
    pub val_f64:     unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut f64) -> u32,
    pub val_str:     unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut *const u8, *mut usize) -> u32,
    pub new_string:  unsafe extern "C" fn(*mut Ctx, *const u8, usize) -> ValueHandle,
    pub new_int:     unsafe extern "C" fn(*mut Ctx, i64) -> ValueHandle,
    pub arr_len:     unsafe extern "C" fn(*mut Ctx, ValueHandle, *mut usize) -> u32,
    pub arr_get:     unsafe extern "C" fn(*mut Ctx, ValueHandle, usize) -> ValueHandle,
    pub struct_get:  unsafe extern "C" fn(*mut Ctx, ValueHandle, *const u8, usize) -> ValueHandle,
    // … ~60 entries: the FULL CfmlValue surface (§4.3) plus throw()
    // --- tier 2 appended here later: scope facade (§5.2) ---
    // --- tier 3 appended here later: CFML execution (§5.3) ---
}
```

The module checks `host.size >= offset_of!(needed_entry) + size_of::<fn>()` before using any entry
beyond tier 1, and the wrapper crate turns that into `ctx.application()?` returning a clean
"this host does not provide tier 2" error. An old extension on a new host works untouched; a new
extension on an old host degrades with a legible message.

**This is the entire answer to "can the VM-access work be deferred?"** — yes, because appending
vtable entries is not a breaking change, and because `ctx` is already in every signature. Ship
tier 1 without it and adding it later is an `ABI_MAJOR` bump that invalidates every extension ever
published.

### 4.3 The three capability tiers

| Tier | Capability | Engine work | Re-entrancy risk |
|---|---|---|---|
| **1. Values** | pure functions over CFML values; native classes holding their own Rust state; QoQ functions | value slab + foreign registries (§5.1) | none — no CFML runs |
| **2. Scope facade** | read/write `application`, `session`, `request`, `server`, `variables`, `cgi`, `url`, `form`, `cookie`; take/release the same locks `<cflock>` uses; rooted handles that outlive a request | scope accessors + rooted slab + GC participation (§5.2) | **none** — still no CFML executes |
| **3. CFML execution** | call BIFs/UDFs/closures, instantiate CFCs, invoke methods, read component metadata, write the output buffer, include | re-entrancy redesign (§5.3) | the hard part |

**Tier 1 exposes the full `CfmlValue` surface — decided.** Every variant, including `Query`,
`Binary`, `Component`, `Function`, `DateTime` and `TimeSpan`, lands in phase 1 rather than being
appended later:

- Packaging ships at phase 2 (§7), so third parties start publishing `.rcx` files early. A vtable
  that grows by 35 entries *after* people have published against it is technically non-breaking but
  practically messy — better to settle the value surface before distribution opens.
- `Query` is the hardest type to model over handles (column-major storage, typed columns, row
  cursors). Doing it while the ABI is still soft is much cheaper than retrofitting it.
- `Component`/`Function` handles are **not** dead weight before tier 3. *Calling* them needs tier 3,
  but accepting a CFC or closure as an argument, rooting it, passing it around and handing it back
  is useful on its own — and it means a tier-3 upgrade adds verbs, not nouns.

Cost: phase 1 grows from ~25 to ~60 vtable entries and the wrapper crate grows to match. Most of
those entries are mechanical (`x_len`/`x_get`/`x_set`/`new_x`); the modelling risk is concentrated
in `Query`, which the phase-0 spike should cover explicitly.

Tier 2 is the sweet spot for "a module that can see the running app": it gives real capability
(memoise into `application`, read config, coordinate across requests, hold state that survives a
request) while **executing no CFML at all**, so none of the deadlock/re-entrancy problems in §5.3
apply. It is a small, separable, high-value increment and should land right after the MVP.

Tier 3 is what a framework kernel needs (§8) and is where the engine surgery lives.

### 4.4 Value handles, lifetime, and rooting

Two slabs, both owned by the host:

- **Call slab** — a `Vec<CfmlValue>` kept on the VM and truncated (not freed) per call. Arguments
  are pre-loaded; module-created values push onto it. On return the result is moved out and the
  slab is truncated. **Zero allocation per call in steady state.** Handles carry a generation tag
  bumped on truncate, so using a handle after its call returns is a clean error.
- **Root slab** (tier 2+) — `ctx.root(h) -> RootHandle` / `ctx.unroot(r)`. Explicitly released,
  process-lifetime, and **visible to the cycle GC** (this is mandatory — RustCFML has cycle
  collection, and an invisible root is either a leak or, worse, a collected-then-used value).

Rules the wrapper crate enforces so authors can't get them wrong: a `Value<'call>` borrows the ctx
lifetime and cannot escape the call; a `Rooted` is an owning RAII type that unroots on drop.

### 4.5 Errors

`CfmlError` (`cfml-common/src/vm.rs:13`) is `{ message: String, error_type: CfmlErrorType,
stack_trace: Vec<StackFrame>, extras: Option<Box<ValueMap>> }`. None of it crosses:

```
ctx.throw(type_code, custom_type_ptr/len, message_ptr/len, extras: ValueHandle) -> ()
```
then return `ValueHandle::NULL`. The host builds the `CfmlError`; `stack_trace` stays host-side and
is filled in by the VM as it already is. `extras` is just a struct handle, so a module can carry
structured detail onto `cfcatch` the same way the DB drivers do (GH #295).

### 4.6 Native classes over the C ABI

The module supplies data + a vtable; the **host** implements `CfmlNative` and forwards.

```rust
#[repr(C)]
pub struct NativeClassVtable {
    pub size: usize,
    pub class_name:   unsafe extern "C" fn(*mut c_void, *mut *const u8, *mut usize),
    pub call_method:  unsafe extern "C-unwind" fn(*mut Ctx, *mut c_void, *const u8, usize,
                                                  *const ValueHandle, usize) -> ValueHandle,
    pub get_property: unsafe extern "C-unwind" fn(*mut Ctx, *mut c_void, *const u8, usize) -> ValueHandle,
    pub set_property: unsafe extern "C-unwind" fn(*mut Ctx, *mut c_void, *const u8, usize, ValueHandle) -> u32,
    pub drop_fn:      unsafe extern "C" fn(*mut c_void),
}
```

Host side: `struct ForeignNative { data: *mut c_void, vtable: &'static NativeClassVtable }` with
`impl CfmlNative for ForeignNative`. `Send + Sync` is asserted by contract — the module is
responsible for its own interior synchronisation, which is exactly what makes §5.3's re-entrancy
fix cheap.

This means `component extends="rust:Name"`, `super.method()`, and `this.X` fall-through
(`lib.rs:29862`, `ops/locals.rs:839/868`, `lib.rs:5123`…) all keep working unchanged — they see an
ordinary `CfmlNative`.

### 4.7 What module authors actually write

`rustcfml-module` is a safe wrapper crate over the vtable — **publishable to crates.io** (no deps,
no engine source). The raw `extern "C"` surface is generated by a macro, never hand-written:

```rust
use rustcfml_module::{module, Ctx, Value, Result, Error};

#[rustcfml_module::bif]
fn rust_greet(ctx: &Ctx, args: &[Value]) -> Result<Value> {
    let name = args.get(0).map(|v| v.as_str()).transpose()?.unwrap_or("World");
    Ok(ctx.string(format!("Hello, {name} (from Rust)")))
}

#[rustcfml_module::class]
impl Tally {
    fn new(_ctx: &Ctx, _args: &[Value]) -> Result<Self> { Ok(Tally { count: 0 }) }
    fn bump(&self, _ctx: &Ctx) -> Result<i64> { Ok(self.count.fetch_add(1, SeqCst) + 1) }
    fn value(&self, _ctx: &Ctx) -> Result<i64> { Ok(self.count.load(SeqCst)) }
}

module! { name: "greeter", version: "0.1.0", bifs: [rust_greet], classes: [Tally] }
```

Deliberate choices:

- **Handles + an owned escape hatch.** `Value` is a borrowed handle; you pay one host call per
  field you actually touch, and a 10k-row query argument is never copied. `v.to_owned()?` marshals
  into a plain mirror enum when the author wants to `match`, which is opt-in rather than the
  default toll on every call.
- **`&self`, not `&mut self`, on class methods.** Forces authors into interior mutability from day
  one, which is what makes tier 3 re-entrancy possible without a later breaking change.
- **Both link modes from one source.** A `static` cargo feature on `rustcfml-module` binds the same
  module source directly against the host's Rust API instead of the vtable, so the cocktail
  `--build` path keeps working with zero source changes and zero marshalling. Same crate, two
  bindings, one API.

### 4.8 Compatibility token

Much simpler than the Rust-ABI design needed:

```
token = "{ABI_MAJOR}|{target_triple}"     e.g. "1|aarch64-apple-darwin"
```

No rustc version, no layout revision, no feature hash. Checked at load; mismatch refuses loudly
with both tokens printed. `ABI_MAJOR` bumps only on a genuinely breaking vtable change — which,
given append-only growth, should be approximately never.

### 4.9 Package format — `.rcx`

A zip, because a Lucee-shaped format is a Lucee-shaped format:

```
greeter-0.1.0.rcx
├── module.json      name, version, abi_major, min tier required, declared BIF/class names,
│                    author, licence, sha256 of each payload
├── lib/
│   ├── aarch64-apple-darwin/libgreeter.dylib
│   ├── x86_64-unknown-linux-gnu/libgreeter.so
│   └── x86_64-pc-windows-msvc/greeter.dll
├── wasm/greeter.wasm     (optional, later tier)
├── cfml/                 (optional) CFCs / custom tags / mappings shipped with the module
└── LICENSE, README.md
```

Multi-triple in one file = the "fat extension", published once. Declaring BIF/class names in the
manifest lets the engine report conflicts (`rustGreet is provided by both greeter and legacy-greeter`)
**without loading anything**, and lets `getFunctionList`/`writeDump` attribute a builtin to its
module. Declaring the required tier lets an old host refuse a too-new extension cleanly.

Shipping CFML alongside native code is what makes this an *extension* rather than a *plugin* — a
module can present a fluent CFC facade over its Rust core, which is how these should be designed
anyway.

### 4.10 Discovery, install, config

Resolution order, first hit wins per module name:

1. `--extensions <dir>` (explicit)
2. `extensions/` in the app dir — per-app, checked into the project, the common case
3. `~/.rustcfml/extensions/`
4. `<binary-dir>/extensions/` — system / container image
5. VFS-embedded, for `--build` self-contained binaries (extracted to cache at boot)

```
rustcfml ext new <name>            scaffold a module crate
rustcfml ext build [--target ...]  build cdylib(s), package the .rcx
rustcfml ext install <file|url>    verify token + sha256, place, clear macOS quarantine
rustcfml ext list                  installed, versions, tier, load status
rustcfml ext remove <name>
```

`.cfconfig.json` gets an `extensions` block: enable/disable list, per-extension settings struct
(handed to `on_load` — Lucee-style extension config), `requireSigned` flag.

### 4.11 Load lifecycle

```
process start (once)
  └─ resolve extension dirs
  └─ for each *.rcx, newest-version-wins per module name:
       ├─ read manifest, pick lib/<host-triple>/, verify sha256
       ├─ extract to ~/.rustcfml/ext-cache/<sha256>/   — dlopen needs a real path
       ├─ libloading::Library::new(path)
       ├─ dlsym rustcfml_module_decl -> *const ModuleDecl
       ├─ check abi_major + token + required tier   ← refuse loudly on mismatch
       ├─ decl.on_load(&vtable, config)   — one-time setup, thread pools, caches
       └─ std::mem::forget(library)       ← NEVER unloaded (§6.2)
  └─ set_registrar(move |vm| for m in loaded { m.register(vm) })

per VM (existing path, unchanged)
  register_vm_runtime → stdlib → apply_native_modules → our closure → decl.register(...)
```

`on_load` (once per process) is split from `register` (once per **VM** — every request and every
cfthread child VM, per `cli/src/lib.rs:852`) precisely because `register` is on a per-request path
and must do nothing but register names.

---

## 5. Engine changes required, by tier

This is the part that is genuinely new work in the VM, and the answer to "how much of this can be
deferred".

### 5.1 Tier 1 — required for the MVP

**(a) Foreign registries.** `self.builtins` is `HashMap<String, fn(Vec<CfmlValue>) -> CfmlResult>`
(`lib.rs:1646`), read via `.copied()` at ~8 sites (`:13000`, `:15312`, `:20873`, `:21078`,
`:24356`, …). **A bare `fn` pointer cannot close over "which module, which entry point"**, so
foreign BIFs cannot live in that map. Same for `native_classes: HashMap<String, NativeConstructor>`
(`:2243`) and the QoQ registry.

Required: a parallel `foreign_builtins: HashMap<String, ForeignBuiltin>` consulted after `builtins`,
where `ForeignBuiltin { entry: ModuleFn, module: ModuleId }`; `refresh_builtin_index` /
`is_builtin_name_ci` / `builtin_names_lc` extended to include foreign names. Best done by funnelling
the ~8 `.copied()` sites through one resolution helper first — a mechanical, testable refactor.

> *Escape hatch if we want the MVP to avoid VM churn:* a fixed pool of monomorphised trampolines
> (`fn tramp<const N: usize>(args) -> CfmlResult` looking up slot `N` in a static table) yields real
> bare `fn` pointers with **zero engine changes**, capped at a fixed count. It works — but a bare
> `fn` has no `&mut Vm`, so it cannot carry `ctx` and is a dead end at tier 2. Given tier 2 is
> wanted, **do the registry once**; the trampoline is a fallback only if tier 1 must ship in
> isolation.

**(b) The value slab + ctx.** `Ctx` wraps `&mut CfmlVirtualMachine` plus the call slab. The foreign
dispatch arm lives in `call_function`, which already has `&mut self` — so ctx costs no new plumbing.

**(c) The loader** — `crates/cli`: libloading, token/tier check, `on_load`, `set_registrar`.

**(d) `ForeignNative`** — the host-side `impl CfmlNative` forwarding to the module vtable (§4.6).

### 5.2 Tier 2 — the scope facade

**(a) Scope accessors on the vtable.** Read/write by scope name, plus an unqualified read that
honours the standard resolution order. Values in and out are handles; no Rust layout crosses.

**(b) Locking — decided policy: modules take locks, and observe locks.**

`application` is live and shared across concurrent requests as of v0.593.0, so a module touching a
shared scope is a concurrent writer like any other. The rules:

- **One lock registry, shared with `<cflock>`.** Native locks and CFML locks live in the same
  namespace, so a `<cflock scope="application">` in a CFML page blocks a native write and vice
  versa. A separate native lock table would be worse than no locking at all — it would look correct
  and protect nothing.
- **Writes require an explicitly acquired guard.** `ctx.lock(scope|name, type, timeout) ->
  LockGuard`; a write attempted without a live guard is a clean error, not a silent success.
  Stricter than CFML itself, deliberately: a native module writing a shared live scope unlocked is
  a bug factory, and unlike CFML code it can do so from a thread the user never thinks about.
- **Reads take the underlying read lock per call.** So an unqualified `ctx.app_get(k)` is always
  consistent with an in-flight exclusive CFML `<cflock>`, without the author having to think.
- **Guards are call-scoped and force-released.** RAII on the module side, plus the host releases
  anything still held when the call returns. A module must not be able to hold a lock across
  requests — that is a hang, not a bug report.
- **Timeout semantics match the engine exactly**: `timeout=0` means wait forever (Lucee semantics,
  per the v0.592.0/v0.593.0 lock work), and a timeout surfaces `LockOperation="Timeout"` on the
  error. Reimplementing these from scratch on the native side is how we would reintroduce the
  Preside reload-flag bug that cost 241 lock errors a burst.

**(c) Rooted handles + cycle-GC participation** (§4.4). This is the substantive part: a module that
caches across requests holds roots, and the collector must see them.

**(d) ctx lifetime enforcement.** `ctx` is per-call; a module storing it and using it on a later
request must get a clean generation-tag error, not UB. Cheap to add, essential to have.

Notably, **tier 2 executes no CFML**, so it needs none of §5.3. That is what makes it a small,
independently shippable increment.

### 5.3 Tier 3 — CFML execution and the re-entrancy problem

**The blocker:** method dispatch at `lib.rs:22490` takes `obj.write()` and holds that guard for the
whole `call_method`. Any native method that calls back into CFML which touches the same object
deadlocks. A DI container resolving a bean whose provider closure resolves another bean from the
same container is precisely that shape — so this is not a corner case for a kernel, it is the
main line.

**The cheap fix, and why B makes it cheap:** add an opt-out on the dispatch path for
internally-synchronised natives (e.g. `fn needs_exclusive(&self) -> bool { true }` on `CfmlNative`,
overridden to `false` by `ForeignNative`), and dispatch those without taking the guard. Sound
because §4.6 already requires the module to manage its own locking, and §4.7's `&self` method
signature already forces authors into interior mutability. **The existing Rust `CfmlNative` trait
and every current implementor keep `&mut self` and the guard, untouched.** Re-entrancy is solved
for foreign modules without a risky change to the existing component model.

**Also needed at tier 3:** `call_fn(name, args)`, `call_value(fn_handle, args)` (UDFs and closures),
`new_component(path, args)`, `invoke_method(component, name, named_args)`, `set_component_property`
(DI injection), `get_component_metadata(path)` — metadata matters more than it looks, because
annotation-driven DI is metadata-driven — plus `write_output` and `include`.

---

## 6. Remaining landmines

Three of the original five died with the C ABI choice (§3). What's left:

### 6.1 Unwinding across the boundary
All entry points are `extern "C-unwind"` **and** `catch_unwind` inside the host-side shim, so a
panicking module produces a clean CFML error rather than an abort. Both sides need
`panic = "unwind"` (we use the default; the release profile does not set `panic = "abort"` — keep
it that way).

### 6.2 Unloading is not on the table
Foreign fn pointers live in the registries and `ForeignNative` objects may outlive any request.
`dlclose` while any of that is alive is UB. `mem::forget` the `Library`, never unload,
`ext install` prints "restart to activate". Real functional gap versus OSGi; document it.

### 6.3 Platform specifics
- **macOS:** Gatekeeper quarantines downloaded dylibs and `dlopen` then fails — `ext install` must
  clear the quarantine xattr, and redistribution needs ad-hoc signing. `dlopen` also needs a real
  path (no in-memory load), hence extract-to-cache.
- **Windows:** load by absolute path via `LoadLibraryEx` semantics (libloading covers it) to avoid
  DLL-hijack search ordering.
- **Linux:** glibc vs musl is already in the target triple, so the token separates them.
- **PGO/fat-LTO:** no ABI interaction (`repr(C)` + fn pointers). Just don't let the module build
  inherit `-C profile-use` pointing at our profdata.

### 6.4 Security
A `.rcx` is arbitrary native code with full process privilege — same as a `.lex`. Position it as
trusted code. Escalating: manifest sha256 pinning (phase 1), `requireSigned` with detached
signatures (phase 3), org allowlist. Do not pretend to sandbox; if sandboxing is the requirement,
that's the wasm tier and should be sold as such.

---

## 7. Phasing

| Phase | Content | Rough size |
|---|---|---|
| **0. Spike** | `cfml-module-abi` + a hand-built cdylib loaded by a test binary. Prove: dlopen on macOS **and** Linux, ctx threading, the value slab, one BIF and one native class round-tripping. **Model `Query` over handles** — the one piece with real design risk. Measure per-call handle overhead on a realistic BIF to confirm §3's arithmetic. | ~2 days |
| **1. Tier 1 MVP** | ABI crate with the **full value surface** (~60 entries), `rustcfml-module` wrapper + macros, foreign registries (§5.1a), value slab, loader, `ForeignNative`, token check, `extensions/` discovery, port `examples/native_module_demo` to build **both** ways, docs. Loose `.dylib`/`.so`/`.dll`. | ~7–9 days |
| **2. `.rcx` packaging** | Manifest, fat multi-triple extensions, `ext new/build/install/list/remove`, CFML-alongside-native, `.cfconfig.json` block, VFS-embedded extensions, conflict reporting, `getFunctionList` attribution, sha256 pinning, macOS quarantine handling. **Extensions become distributable here.** | ~4–5 days |
| **3. Tier 2 scope facade** | Scope accessors, lock integration (§5.2b), rooted handles + GC participation, ctx lifetime enforcement. *The "Rust can see the running app" milestone.* | ~4–5 days |
| **4. Tier 3 CFML execution** | Re-entrancy opt-out, call/instantiate/metadata/output vtable entries. Unlocks §8. | ~5–8 days |
| **5. Hot-method substitution** | Per-instance `variables` scope access, the override registry + dispatch hook, decline-return, drift guard. **See §11 — and §11.6's precondition: fix the engine pathology before writing an override.** | ~5–7 days |
| **6. Hardening / wasm** | Signing, richer host services (cache, datasource, logging), wasm payload tier. | as needed |

Ordering rationale: distribution before capability. Phase 2 makes an extension a shippable artifact
others can install, and gets the manifest's tier-declaration machinery in place *before* tier 2
changes what a manifest must declare. The full-value-surface decision in §4.3 pairs with this —
the ABI's nouns are settled before anyone publishes against them, so later tiers append verbs only.

Phases 1–3 deliver a complete, distributable extension system for leaf and
stateful-but-not-re-entrant work. Phase 4 is what a framework kernel needs and can be scheduled
independently.

---

## 8. Case study A — a Rust Preside/ColdBox kernel

Target: a pure-Rust kernel combining WireBox/ColdBox/CacheBox internals, presenting an interface
that looks like ColdBox to Preside. **Requires tier 3** (phase 4).

### 8.1 Read the existing profiling honestly before committing

The measured picture (see the perf memories, and `PERFORMANCE_ROADMAP.md`):

| Fact | Implication for a Rust kernel |
|---|---|
| Request decomposes as **0.57ms engine floor / 2.88ms + Preside framework / 38.40ms homepage** ⇒ ~92% is content execution | ⚠️ **Framework plumbing is ~7.5% of a real page.** Taken at face value, a kernel that replaces only framework plumbing has a small ceiling. |
| **Uniform 2.2× per-op gap vs Lucee, no hot spot** | No single component to replace. Wins have to come from removing *work*, not speeding one function. |
| **8,112 frames/req = 24% of a warm request** (P6 call path) | Framework code creates a lot of frames. A Rust kernel removes them wholesale — this is the strongest pro-kernel signal. |
| **Eager `arguments` = 42% boot / 61% warm struct allocs**; **1.07M struct allocs/boot** | A Rust kernel allocates no CFML structs at all — attacks CPU *and* memory together. |
| **~22ms (63%) of a warm request is unattributed "BIF bodies"** | Those BIFs are already Rust. **A tier-1 module cannot help here** — you cannot beat the existing implementation by reimplementing it in the same language. |
| **JIT does ~0% of Preside**; mimalloc −15.6%, PGO −14.16% | The biggest wins so far came from swapping a *subsystem*, not optimising call sites. A kernel is the same kind of move. |

Two conclusions worth stating plainly:

1. **Tier-1 leaf BIFs will not move Preside.** The hot builtins are already native. The value of
   tiers 1–2 for Preside is *packaging and extensibility*, not speed. If Preside performance is the
   only justification, phases 1–3 do not pay for themselves — phase 4 does.
2. **The kernel thesis is unproven, and the existing decomposition mildly argues against it.**
   The 2.88ms framework figure was measured on a *near-trivial* request; it is a floor, not the
   framework's share of a real page render, where interceptors fire, `getModel` runs repeatedly and
   caching is consulted per widget. The 8,112-frames figure hints the real share is much higher.
   **These two numbers are not reconciled, and reconciling them is the go/no-go.**

### 8.2 The go/no-go measurement — do this before phase 4

Cheap, decisive, and it should gate ~5–8 days of the riskiest work in this plan:

> On a **warm** Preside homepage render in **serve mode**, attribute CFML frames and CPU time to
> **framework code** (ColdBox / WireBox / CacheBox / Preside's own service layer) versus
> **application code** (handlers, views, widgets). Use the existing `call-phases` instrument and the
> `ab_cpu.py` / `ab_suite.py` methodology — **CPU time, not wall clock**, on a frame-dense workload.

- If framework CFML is a **large** share of those 8,112 frames → the kernel is the right lever and
  phase 4 is justified.
- If it is **small**, and the frames are Preside's own handlers and views → a kernel replaces the
  cheap part. The honest response is to **not build it**, and to spend the time on the "BIF bodies"
  census that the perf memories already flag as the next instrument.

Do not skip this because the kernel is the exciting option. Every large perf win in this repo so far
came from measuring first (`project_p61_writeback_argset_fixed` is the standing lesson: the
diagnosis was wrong until a phase was split and measured).

### 8.3 If it goes ahead — two architectural rules

**Keep kernel state in Rust, not in CFML scopes.** The singleton map, the cache, the
event/interceptor registry are real Rust structures inside the module — never `application.wirebox`
structs. Expose a small number of `NativeObject` facades into `application`
(`application.cbController` and friends); `application.cbController.getRequestService()` then
dispatches straight into Rust. This inverts the traffic — the scope holds a handle *to* the kernel
rather than the kernel marshalling scope structs — so the handle-ABI overhead lands nowhere that
matters, and it is what makes the struct-allocation win real.

**Scope the surface deliberately.** "Looks like ColdBox to Preside" spans controller, request
context, interceptor service, module service, handler service, WireBox injector and binder. A full
reimplementation is a very large project. Build a **Rust fast-path for the hot subset** — singleton
resolution, handler dispatch, cache — with the rest still CFML. That version can be measured early
and abandoned cheaply.

**Extra engine requirement:** a Rust kernel dispatching a handler must not synthesise CFML frames
for its own internal steps, or it gives back the 24% it was built to reclaim. Tier 3's
`invoke_method`/`call_value` need a frame-free path for module-internal dispatch.

---

## 9. Case study B — Obscura as a dynamic extension

Question: does this design make `OBSCURA_BROWSER_PLAN.md` achievable as a loadable extension,
**including the V8 tier**?

**Yes — and it is a better fit than compiling in.** The reasoning:

### 9.1 Why it fits

- **Obscura's actor design already isolates everything `!Send`.** The plan's §4 puts `Browser`,
  `Page` and the V8 isolate on a dedicated service thread with its own current-thread tokio runtime
  + `LocalSet`, reached by channel; the CFML-side objects are `{id, tx}`. **That thread is spawned
  by the module, inside the module, in `on_load`** (once per process — exactly what `on_load` is
  for). Nothing `!Send` ever approaches the host boundary.
- **The CFML-facing API is values in, values out.** `goto`, `text`, `links`, `extract`, `attr`,
  `count`, `markdown`, `evaluate` — all `CfmlValue` → `CfmlValue`. Wrapped as `ForeignNative`
  objects (§4.6) holding `{id, tx}`. **Obscura's core is a tier-1 module.** It does not need scope
  access and it does not need to call CFML.
- **CDP and MCP need no host route registration.** Both are standalone listeners on their own ports
  (`obscura_cdp::start_with_options` on 9222, `obscura_mcp::http::run` on 3000) — not routes on the
  CFML server. The module binds them itself from `on_load`, gated on config. So the "host services"
  gap I expected turns out not to exist for this case.
- **V8's ~50MB and the static-lib download leave the base binary.** This is the biggest win.
  Browser support becomes an opt-in 50MB `.rcx` instead of bloating every `rustcfml` binary, and
  the three feature tiers (`browser` lite / `browser-stealth` / `browser-js`) become three
  published extension builds with the tier declared in `module.json` — arguably cleaner than cargo
  features, and it removes the wasm-exclusion awkwardness the Obscura plan §3 has to work around.
- **dlopen is lazy-paged**, so a 50MB dylib costs little at startup unless used.
- **No engine source needed.** An Obscura extension deps on `rustcfml-module` (crates.io) +
  `deno_core`. Compare with the cocktail path's `rustcfml_source_root()` requirement.
- `panic = "unwind"` is required by both plans — consistent, no conflict.

### 9.2 The three things to design for

1. **V8 must be initialised exactly once per process.** Two extensions each linking `deno_core`
   would each try to initialise the V8 platform → crash. Mitigation: `module.json` declares
   `provides: ["v8"]`; the host refuses to load a second provider of the same exclusive capability
   and says which extension already owns it. Cheap, and needed before any V8-bearing `.rcx` ships.
2. **CFML callbacks need tier 3.** Fluent interception (`page.onRequest(function(req){...})`) and
   preload scripts driven by CFML closures require `call_value` — phase 4. The core browser API
   does not, so Obscura can ship at tier 1 and gain callbacks later. This is exactly the
   append-only-vtable story working as designed.
3. **Config delivery to `on_load`.** The Obscura plan wants `--browser-cdp-port` / `--browser-mcp-port`
   style switches. Under the extension model these become `.cfconfig.json` `extensions.browser.*`
   settings passed to `on_load` (§4.10), not new CLI flags in `crates/cli`. Slightly different
   ergonomics from the original plan; worth reconciling when Obscura is picked up.

### 9.3 What this implies for sequencing

Obscura is a **much better first real extension than the Preside kernel**: it is tier 1, it has an
obvious binary-size justification for being dynamic, and it exercises the awkward parts of the ABI
(native classes, long-lived module state, a module-owned thread, `Query`-free but `Struct`/`Array`-heavy
values) without needing any of phase 4. If phases 1–3 need a proving workload, this is it.

---

## 10. Executing this from a cold start

Everything a fresh session needs. **Read this section first.**

### 10.1 Orientation

- Plan doc: `NATIVE_EXTENSIONS_PLAN.md` (this file) — **untracked by design**; do not commit it
  without asking.
- Related plans: `OBSCURA_BROWSER_PLAN.md` (§9 above), `PERFORMANCE_ROADMAP.md` (§8 above).
- Existing native-module docs: `docs/native-modules.md`; worked examples
  `examples/native_module_demo/`, `examples/native_markdown/`.
- Project conventions and the **release verification gate**: `CLAUDE.md`. Read it. A red *or
  skipped* test in any suite is a blocker.

### 10.2 The facts this plan rests on (re-verify if the tree has moved)

| Claim | Where | Why it matters |
|---|---|---|
| `BuiltinFunction`/`NativeConstructor`/`QoQFn` are all `fn(Vec<CfmlValue>) -> CfmlResult` | `cfml-vm/src/lib.rs:122`, `:2451`, `cfml-qoq/src/function.rs:22` | bare fn pointers can't carry module identity → §5.1a |
| `builtins: HashMap<String, BuiltinFunction>`, `.copied()` at ~8 sites | `lib.rs:1646`; `:13000`, `:15312`, `:20873`, `:21078`, `:24356`, `:7839`, `:10082`, `:24356` | the foreign registry work |
| builtin name index | `lib.rs:1663`, `:4405` `refresh_builtin_index`, `:4422` `is_builtin_name_ci` | must include foreign names |
| `native_classes: HashMap<String, NativeConstructor>` (lowercased keys) | `lib.rs:2243`, registered `:4385`, consulted `:15650` | ditto for classes |
| `CfmlNative` trait | `cfml-common/src/dynamic.rs:1378-1405` | the four methods `ForeignNative` must implement |
| `CfmlValue::NativeObject(Arc<RwLock<dyn CfmlNative>>)` — `std::sync::RwLock` | `dynamic.rs:1470` | equality is `Arc::ptr_eq` (`:2333`) |
| method dispatch holds `obj.write()` for the whole call | `lib.rs:22490` | the tier-3 re-entrancy blocker |
| `set_registrar` / `apply_native_modules` / `run_with_registrar` | `cli/src/lib.rs:122-161` | the loader's injection point |
| registrar applied **once per VM**, after stdlib | `cli/src/lib.rs:852` in `register_vm_runtime` | why `on_load` ≠ `register` |
| `CfmlError { message, error_type, stack_trace, extras }` | `cfml-common/src/vm.rs:13` | §4.5 error convention |
| host installs mimalloc as `#[global_allocator]` | `cli/src/main.rs:40` | why the C ABI matters (§3) |
| cocktail build driver | `cli/src/lib.rs:3280` `build_self_contained`, `:3423` `discover_native_modules`, `:3542` `build_cocktail_binary`, `:3491` `rustcfml_source_root` | the static path to keep working |
| release pins rustc 1.93.0 for PGO | `.github/workflows/release.yml:82` | context only; the C ABI removes the dependency |

### 10.3 Phase-by-phase task list

**Phase 0 — spike (~2 days).** Goal: de-risk, not to build production code. Throwaway is fine.
- [ ] `crates/cfml-module-abi` with `ValueHandle`, `Ctx`, `HostVtable` (tier-1 subset), `ModuleDecl`.
- [ ] A hand-written cdylib exporting `rustcfml_module_decl`, no wrapper crate yet.
- [ ] A test host binary: libloading → token check → `on_load` → `register` → call one BIF and one
      native class method.
- [ ] **Prove `Query` over handles** — the one real design risk. Try both a full accessor set and a
      borrowed columnar view; write down which wins and why.
- [ ] Verify on **macOS and Linux** (§6.3). Windows can wait for phase 2.
- [ ] **Measure**: per-call handle overhead on a realistic BIF, CPU time, serve mode. Compare
      against the same BIF compiled in. This validates or kills §3's arithmetic.
- **Exit criteria:** a number for the overhead, a decided `Query` model, and dlopen working on two
  platforms.

**Phase 1 — tier-1 MVP (~7–9 days).**
- [ ] `crates/cfml-module-abi` productionised: full ~60-entry vtable (§4.3), `#[repr(C)]`,
      zero deps, **zero mutable statics** (add a test asserting this).
- [ ] `crates/rustcfml-module` — safe wrapper + `#[bif]` / `#[class]` / `module!` macros (§4.7),
      plus the `static` feature that binds against the host Rust API for the cocktail path.
- [ ] **Refactor first:** funnel the ~8 `builtins.get().copied()` sites through one resolution
      helper. Own commit, full gate green, no behaviour change.
- [ ] `foreign_builtins` / `foreign_classes` / foreign QoQ registries + index integration (§5.1a).
- [ ] Value slab on the VM (call-scoped, truncate-not-free, generation-tagged) (§4.4).
- [ ] `ForeignNative` host-side `impl CfmlNative` (§4.6).
- [ ] Loader in `crates/cli`: libloading, token + tier check, `on_load`, `set_registrar`,
      `mem::forget`, `extensions/` discovery (§4.10 order).
- [ ] `catch_unwind` shims at every host-side entry (§6.1).
- [ ] Port `examples/native_module_demo` to build **both** ways from one source.
- [ ] Tests: CFML suite additions under `tests/native/`, plus Rust tests for the slab and
      generation-tag rejection.
- [ ] `docs/extensions.md`; update `docs/native-modules.md` to point at it.
- **Exit criteria:** a loose `.dylib` in `extensions/` provides BIFs, a native class, and a QoQ
  function; the cocktail path still works unchanged; full gate green.

**Phase 2 — `.rcx` packaging (~4–5 days).**
- [ ] Format + `module.json` schema (§4.9), incl. `provides:` exclusive capabilities (§9.2).
- [ ] `rustcfml ext new|build|install|list|remove` (§4.10).
- [ ] Fat multi-triple extensions; extract-to-cache; sha256 verify; macOS quarantine clearing.
- [ ] CFML-alongside-native payloads + mapping registration.
- [ ] `.cfconfig.json` `extensions` block, config delivered to `on_load`.
- [ ] Conflict reporting from the manifest **without loading**; `getFunctionList`/`writeDump`
      attribution.
- [ ] VFS-embedded extensions for `--build`.
- **Exit criteria:** an `.rcx` built on one machine installs and runs on another of the same triple.

**Phase 3 — tier-2 scope facade (~4–5 days).**
- [ ] Scope accessors on the vtable (append only — do not reorder tier-1 entries).
- [ ] Lock integration per the decided policy in §5.2b — **one registry shared with `<cflock>`**,
      explicit guard for writes, reads take the read lock, guards force-released at call end,
      `timeout=0` = wait forever, `LockOperation="Timeout"`.
- [ ] Rooted handle slab + **cycle-GC participation** (the substantive risk here).
- [ ] ctx generation-tag enforcement (stored-and-reused ctx = clean error).
- [ ] Tests including a concurrency test that a native write and a CFML `<cflock>` mutually exclude.
- **Exit criteria:** a module memoises into `application` correctly under concurrent load; no leak
  and no premature collection of rooted values.

**Phase 4 — tier-3 CFML execution (~5–8 days).** Gate on §8.2 first if the justification is Preside.
- [ ] `needs_exclusive()` opt-out on `CfmlNative`; `ForeignNative` returns `false`; dispatch path at
      `lib.rs:22490` honours it. **Existing implementors keep `&mut self` and the guard.**
- [ ] `call_fn`, `call_value`, `new_component`, `invoke_method`, `set_component_property`,
      `get_component_metadata`, `write_output`, `include`.
- [ ] Frame-free internal dispatch path (§8.3).
- [ ] Re-entrancy tests: native → CFML → same native object, several levels deep.

### 10.4 Gates — run these, every phase

Per `CLAUDE.md`, all of the following must be green before tagging; a skipped test is a blocker:

```bash
cargo build
cargo test --workspace                    # incl. the 76 JIT integration tests
cargo run -- tests/runner.cfm             # CLI
# serve mode, cold AND warm, BOTH:
cargo run --release -- --serve
cargo run --release -- --serve --production
cargo build -p cfml-worker -p rustcfml-wasm --target wasm32-unknown-unknown
wasm-pack build crates/wasm --target web  # before pushing to main
```

Extension-specific additions to that list:
- The cocktail `--build` path still produces a working binary (`examples/native_module_demo`).
- The wasm targets still build — `cfml-module-abi` must not leak into the wasm crate graph.
- Cap build parallelism (`-j 4`) per the standing preference.

### 10.5 Standing constraints from project memory

- **Never** no-op or stub to get past a blocker without flagging it and getting approval; any
  approved no-op goes in `docs/known-issues.md`.
- **Lucee is the reference** for any CFML-visible semantics (locking, error members, scope
  behaviour). Never accept a divergence.
- Never park a red test as "pre-existing" — prove causation, then fix the engine.
- Perf claims use **CPU time on a frame-dense representative workload**, measured in **serve mode**,
  interleaved A/B — never 40 warm requests, never wall clock on a busy box.
- Commit fixes direct to `main`; ask before `git push`; never chain a push after a commit or tag.
- No `Co-Authored-By` or any AI authorship credit anywhere.

---

## 11. Hot-method substitution — overriding individual CFC methods with Rust

**The idea:** leave the framework and all its wiring in CFML, but let a loaded extension replace
*individual hot methods* on *individual CFCs* with a compatible Rust implementation. The CFML source
is never edited, so Preside still runs unmodified on Lucee; a RustCFML deployment can additionally
load a `.rcx` that targets only the handful of methods where it matters.

This is the most attractive shape in this plan. Compared with the kernel (§8) it is **incremental**
(one method at a time), **measurable** (each override is its own A/B), **abandonable** (drop the
extension, get the CFML back), and it carries **no compatibility debt** in the framework repo.

### 11.1 Mechanism

- **Registry:** `(component path, method name) → foreign fn`, populated from the module's
  `register` and keyed case-insensitively like everything else.
- **Dispatch hook:** consulted where a CFC method call resolves to a function body. Miss = today's
  path exactly, so the cost when no extension is loaded must be provably zero (a single
  `is_empty()` guard on the registry, hoisted).
- **Signature:** the override receives `ctx`, the instance handle, and the argument handles. It is a
  normal foreign fn (§4.2) — no new ABI shape.
- **Decline return:** a distinguished result meaning *"I did not handle this; run the CFML body."*
  This is what makes the feature safe to adopt gradually — implement the hot 90% in Rust and fall
  through for every edge case you haven't covered. Without it, an override is all-or-nothing and
  every unhandled input is a production bug.

### 11.2 Capability requirements

| Need | Tier | Note |
|---|---|---|
| Read/write the instance's `variables` scope | **new** | DI'd dependencies and per-instance caches live here. This is *per-instance* scope access, distinct from tier 2's shared scopes — it needs its own vtable entries keyed off the instance handle. |
| Call other CFC methods / closures | **3** | Most candidate methods delegate. A few are leaf-ish and would work without it. |
| Read component metadata | **3** | Needed by anything metadata-driven (`getInheritedMetaData`-shaped work). |
| Decline / fall through to CFML | **new** | §11.1 — small, but it must exist from the first version. |

So this lands **after phase 4**, and adds one capability beyond it. Realistically a phase 5 item —
but the value is high enough that it is worth letting it shape phase 4's API rather than being
retrofitted.

### 11.3 The drift guard — non-negotiable

An override silently implementing *last year's semantics* after a framework upgrade is the failure
mode that would make this feature a liability rather than an asset.

- At load, hash the target method's source (or its AST) and compare against a hash recorded in
  `module.json`.
- **Mismatch refuses the override** and logs loudly, naming the component, method, expected and
  actual hash. The CFML runs — degraded performance, correct behaviour. Never the other way round.
- `rustcfml ext build` computes the hashes from the framework version being targeted, so an
  extension is explicitly pinned to the framework releases it was validated against.
- `rustcfml ext list` reports which overrides are active and which were refused on drift.

### 11.4 Correctness surface to get right

More than it first appears, and worth enumerating before any code:

- Access modifiers (`public`/`private`/`package`), `output=false`, declared return-type coercion.
- Named arguments, `argumentCollection`, defaulted and `required` arguments.
- Inheritance: is the override keyed to the declaring component or the runtime instance type? A
  method inherited from a parent must not be overridden accidentally on every subclass — or, if it
  should be, that has to be explicit in the manifest.
- **Runtime mixins.** ColdBox and Preside inject methods at runtime — `injectMixin`,
  `injectPropertyMixin`, `getVariablesMixin` appear thousands of times in the attached profile. A
  method present on an instance may not exist in the component's source at all. Override keying has
  to account for mixed-in methods, and the drift guard has to hash the *source of record*, wherever
  that is.
- `super.` calls, `onMissingMethod` interaction, and `cfinterface` conformance.
- **Observability:** an overridden method must still appear in the debug output, attributed to the
  module that replaced it. A method that silently vanishes from the profile is how you lose a day.

### 11.5 Candidates from the 2026-08-13 profile

Captured from a 457.140 ms Preside admin request (412.575 ms application, 44.565 ms query,
40,804 executed templates/tags).

⚠️ **This request is cold or first-hit, not steady state.** `registerNewInstance` ×14,
`buildCFC` ×43, `createCache` ×3, `EventHandler.init` ×14, `Mapping.process` ×14,
`readExportersFromDirectories` ×1 are DI construction and cache setup that largely vanish on
subsequent requests. Splitting by call count separates the two populations:

**Warm-path (recurs every request — where throughput lives):**

| Method | ms | calls | avg ms |
|---|---|---|---|
| `HandlerService.getHandlerBean()` | 69.4 | 312 | **0.222** |
| `RequestContextDecorator.*` (aggregate) | 17.8 | 4278 | 0.004 |
| `ResourceBundleService.getResource()` | 15.4 | 371 | 0.041 |
| `DelayedInjectorDsl.process()` | 11.6 | 171 | 0.067 |
| `FeatureService.isFeatureEnabled()` | 9.4 | 745 | 0.013 |
| `InterceptorService.processState()` | 9.1 | 1431 | 0.006 |
| `DelayedInjector.*` | 8.7 | 2680 | 0.003 |
| `InterceptorState.*` | 5.9 | 224 | 0.026 |
| `RequestContext.getValue()` | 4.8 | 1956 | 0.002 |
| `Logger.canDebug()` | 3.3 | 1468 | 0.002 |
| `RequestService.getContext()` | 2.6 | 2920 | 0.001 |

**Cold-path (instantiation — affects boot and first hit):** `Builder.buildCFC()` 42.7/43,
`Util.getInheritedMetaData()` **21.1/14 = 1.5 ms each**, `Injector.registerNewInstance()` 5.9/14,
`LogBox.getLogger()` 13.6/34, `CacheFactory.createCache()` 11.5/3, `EventHandler.init()` 5.8/14.

(`SqlRunner.runSql()` 44.4/21 is almost entirely the 43.1 ms of query time — that is the database,
not CFML, and not a candidate.)

### 11.6 The precondition — find the engine bug before writing the override

**0.222 ms for a handler-bean lookup and 1.5 ms for `getInheritedMetaData` do not look like
inherent CFML cost.** They look like engine pathologies.

This repo's history is almost entirely that shape: the canonicalize negative-caching stat storm, the
existence-memo bug on script-form deletes, the per-request regex recompile, the MySQL statement
cache wiped every request. Each presented as "Preside is slow here" and resolved as an engine
defect — fixed once, benefiting every application, and measurable against Lucee.

So the order of work is:

1. **Instrument into the top warm-path methods** — `getHandlerBean` first, then
   `getInheritedMetaData` on the cold path — and find out *why* the per-call cost is what it is.
2. Fix whatever engine-level pathology that exposes.
3. Re-profile. Only what survives is a candidate for a Rust override.

**A Rust override that masks an engine bug is the same failure mode as a no-op**: it looks like a
fix, it ships, and the real defect stays in the engine for every other user. Overrides are for cost
that is genuinely irreducible in CFML — not for cost the engine should never have charged.

---

## 12. Settled by default, and what's genuinely still open

Applied unless overridden:

- **Naming.** The shipped artifact is an **extension** (`.rcx`); the Rust crate you write is a
  **native module**; an extension contains one or more native modules plus optional CFML. Keeps
  `docs/native-modules.md` valid and adds `docs/extensions.md` above it.
- **The cocktail `--build` path stays.** Strictly better for single-binary deployment, and §4.7's
  `static` feature means it shares module *source* with the dynamic path. Cost: two delivery modes
  to document and test.
- **Foreign BIF count is unbounded** — the proper registry (§5.1a) has no fixed cap. The count only
  mattered under the trampoline escape hatch, which we are not taking.

Genuinely open, and best answered by the phase-0 spike rather than by argument:

1. **How is `Query` modelled over handles?** Column-major storage with typed columns and row
   cursors is the one place the handle abstraction might not sit cleanly. Options range from a
   full accessor set (`query_cols`/`query_cell`/`query_col_type`/…) to a borrowed columnar view
   for bulk work. Decide with a working spike, not on paper.
2. **What does the per-call handle overhead actually measure?** §3 argues ~2–5ns per field touched
   against ~164ns/op. If a realistic BIF measures materially worse, the wrapper's `to_owned()`
   escape hatch may want to become the default for small argument lists.
