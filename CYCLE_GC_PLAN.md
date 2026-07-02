# Plan — request-boundary cycle collector (fix the serve-mode memory leak)

> Engine @ v0.372.0. Root cause CONFIRMED (repro below): `CfmlStruct`/`CfmlArray`/
> `CfmlQuery` and closure `captured_scope` are `Arc`-refcounted, so **reference
> cycles are never reclaimed**. Preside builds cyclic per-request graphs → RSS
> grows on every hit. JVM engines (Lucee/ACF/BoxLang) don't leak because they
> trace. MatchBox (Ortus's Rust BoxLang, sibling repo) avoids it with a
> generational tracing GC over a NaN-boxed handle heap — but it can only do that
> because it's single-threaded (fibers); RustCFML uses real OS threads for
> `cfthread`, which is exactly why it's on `Arc`.

## Goal / constraints (user priorities)
Reliability (no leak, never free a live object), speed (hot path untouched), and
**no big GC pauses**. → Keep `Arc` refcounting (eager-frees the ~99% acyclic
garbage instantly, on-thread, zero pause). Add a collector that only ever touches
*suspected cycles*, bounded per request, never a global stop-the-world.

## Repro (regression target)
`--serve` an app; hit endpoints N×; watch server RSS. `plain.cfm`/`nocycle.cfm`
(same alloc, no cycle) stay flat. `mutual.cfm` (`a.other=b; b.other=a`) and
`cycle.cfm` (struct holds a closure capturing the struct) grow ~11 KB/request.
After the fix all four must be flat. (Scratch app already built under the
session scratchpad `/leak`.)

## Design — request-scoped trial deletion (Bacon–Rajan, bounded to one request)

The refcount IS the oracle: a survivor with an external owner (persistent scope)
has `strong_count` greater than its internal (cycle) edges; a pure cycle does not.
So we never trace the big persistent scopes (application/session/server) — we read
their effect through the count.

### Tracked node types (the only Arc-shared, interior-mutable, cycle-capable backings)
- `CfmlStruct`  = `Arc<PlRwLock<StructInner>>`  — ctor `CfmlStruct::new` (dynamic.rs:317, single chokepoint)
- `CfmlArray`   = `Arc<PlRwLock<Vec<CfmlValue>>>` — ctor `CfmlArray::new` (dynamic.rs:94, single chokepoint)
- `CfmlQuery`   = `Arc<PlRwLock<CfmlQueryData>>`  — ctor `CfmlQuery::from_data` (dynamic.rs:1698)
- `captured_scope` = `Arc<std::sync::RwLock<ValueMap>>` (closure env). Sites in
  cfml-vm: 2332, 3740, 3769, 6440, 6902, 16026 (grep `Arc::new(.*RwLock::new` +
  `captured_scope`/`closure_env`). Route through a `tracked_scope()` helper.

Non-node carriers we DESCEND THROUGH (not separately collectible): `Function`
(→ its `captured_scope`), `Component`(Box → properties + each method
`captured_scope`), `Closure`(Box → `captured_vars`), `QueryColumn`(Arc<Vec>,
immutable). `NativeObject` = opaque external (a node it references stays
protected — conservative, safe).

### Allocation log (serve-only, near-zero cost otherwise)
- Global `static GC_ARMED: AtomicBool` — set true once at serve start (unless
  `RUSTCFML_NO_CYCLE_GC=1`). CLI/tests: false → ctors pay one relaxed load.
- thread-local `ALLOC_LOG: RefCell<Option<Vec<TrackedAlloc>>>` — `Some` only while
  a **top-level** request body runs (NOT child cfthreads → their TLS stays `None`,
  no accumulation). `TrackedAlloc` holds a `Weak` per allocation.
- Ctor hook: `if GC_ARMED { ALLOC_LOG.with(|c| if let Some(v)=…{ v.push(Weak) }) }`.

### Collection (`cfml_common::cycle_gc::collect`) — runs at request end, on the request thread
Preconditions (enforced by cfml-vm caller):
1. `live_threads.is_empty()` — else SKIP (a running cfthread shares Arcs; counts
   would race). Conservative; documented gap (cycles in thread-spawning requests
   not reclaimed). Truly-internal objects are unreachable by other requests'
   threads, so this guard is sufficient for safety.
2. Persistent data already written back to ServerState (app ~22886 / session
   ~22770) — escapees thus have a real external owner.
3. Transient roots cleared so their refs don't mask cycles: `globals` (page
   `variables`), `page_thread_scope`, leftover operand stack/frames.
   (`request_scope` already cleared ~22896.) Do NOT clear application/session/
   server scopes — they are the external-owner signal.

Algorithm:
1. Drain log; upgrade each `Weak`. Survivors `S` = `HashMap<backing_ptr, Node>`,
   each `Node` holding exactly ONE strong handle (the upgraded Weak).
2. `internal_in[n]` = Σ edges m→n for m,n ∈ S. Enumerate each survivor's child
   *nodes* by borrowing (`with_read`, `backing_ptr()` — NO Arc clones, so counts
   are undisturbed), descending through carriers, terminal at node types.
3. `external(n) = strong_count(n) − 1 (our handle) − internal_in(n)`.
   `roots = { n : external(n) > 0 }`.
4. Mark-live = transitive closure of `roots` over child-node edges (a node
   reachable from a live root is live even if its own external==0 — this is what
   protects `L`-live → `G` where `G`'s only ref is from `L`).
5. `garbage = S \ live`. Clear each garbage backing's contents (drops outgoing
   Arcs). Drop handles → garbage frees by cascade.

Correctness spot-checks: `mutual` → all external 0, no roots, all collected.
Escapee `application.foo=a` → ServerState's stored ValueMap holds `a` (external≥1)
→ root → live (and marks children live). Live `L`→`G` → `G` marked live in step 4.

## STATUS UPDATE (2026-06-30): collector shipped v0.373 was INERT on real Preside — FIXED in working tree

The v0.373 collector (below) was verified only against a lightweight *framework-
like stand-in* app, never the real Preside site (which had a separate boot 500 at
the time). On the **real ReadyIntelligence Preside site** (now booting 200 OK on a
fresh `pcms_ritest` DB) it reclaimed **nothing** — RSS grew **+289 MB on EVERY
request**, 525 MB → 6.7 GB in ~23 hits. Both of the collector's "give up" guards
fired on every request:

1. **`live_threads` guard (the binding one).** A Preside frontend request spawns
   ~17 fire-and-forget `cfthread`s and never joins them, so `vm.live_threads` is
   never empty at request end → the guard skipped collection every time. But all
   17 are **finished** (`is_finished() == true`) — they've returned from their
   bodies and dropped every Arc they held, so `strong_count` reads are perfectly
   stable. The guard tested map *non-emptiness* instead of whether any thread is
   *actually still running*. **FIX (cli/lib.rs):** skip only when some handle is
   `!is_finished()`; lingering-but-finished handles no longer block collection.
2. **`LOG_CAP` (1M) overflow.** A real Preside request allocates **~1.25 M**
   tracked containers — just over the 1 M cap — so the log overflowed, was
   dropped, and collection skipped, on EVERY request (not just cold boot). The cap
   was set *below* a normal framework request size. **FIX (cycle_gc.rs):** (a)
   raise default cap to 16 M (env `RUSTCFML_GC_LOG_CAP`), well above real request
   sizes; (b) on overflow, **stop logging but KEEP the logged subset and still
   collect it** — partial collection is provably conservative (unlogged objects
   read as external roots ⟹ never over-collect), so the cap is now a pure memory
   safety-valve, never a functional "collection disabled" switch.

**VERIFIED on real Preside (default config, this fix):** boot → 581 MB; 20+
sequential `/` hits hold flat at **~650–675 MB** (per-request Δ collapsed from
+289 MB to ~0, occasionally negative). ~145,697 cycle nodes reclaimed per request
(`survivors≈145,724 live≈27 collected≈145,697`). Same-session (cookie jar) RSS is
dead flat. New-session-per-request shows ~6 MB/req of *legitimate* session-scope
growth (live data until session timeout/reaper — Lucee retains it too; NOT a GC
leak). Response body constant at 69,357 bytes (no over-collection). Gate: CLI
5278/5278; serve cold+warm 5314/5314; wasm32 + wasm-pack green; `cargo test
--workspace` [pending].

Diagnostics retained: `RUSTCFML_GC_DEBUG=1` prints per-request
`live_threads/any_running/log_len` + `survivors/live/collected`; `cycle_gc::log_len()`.

---

## (prior) STATUS: IMPLEMENTED & VERIFIED (engine working tree, not yet committed)

Final design differs from the original sketch in two important ways, both found
during real-Preside testing:

1. **Drop the VM *before* collecting** (in `compile_and_run`, cli/lib.rs). The
   response is pulled out of the VM via `std::mem::take` (VM has no `Drop` impl,
   so it stays valid), then `drop(vm)` releases EVERY transient root at once
   (page `variables`, request/thread scopes, call frames, the per-request
   application-scope wrapper). `collect()` then runs purely off the log +
   refcounts. This was essential: manually clearing named scopes left phantom
   external refs (arguments scopes, the app-scope wrapper) that mis-marked
   request-local cycles as "live" and leaked them. Dropping the VM also means a
   closure cached in `ServerState` (persistent) keeps a real external ref to its
   captured scope → correctly protected (NOT collected). Verified: a framework-
   like app caching a closure-bearing singleton in `application` scope serves
   3000 requests all 200, singleton intact, RSS flat.

2. **`LOG_CAP` (1M) on the per-request log.** A cold framework boot allocates
   millions of objects; an unbounded log + survivor map ballooned Preside's boot
   to ~6 GB. Overflowing the cap drops the partial log and skips collection for
   that one request (safe; one-off). With the cap, Preside boot settles at
   ~384 MB.

VERIFIED GREEN: controlled repro flat (mutual/cycle/comp ~0 KB/req, was
+10–11/req); CLI 5273/5273; serve cold+warm 5309/5309; `cargo test --workspace`
serial 0-failures incl. JIT 76 (parallel had 2 flaky websocket ConnectionResets
that pass serially — documented pattern); wasm32 + wasm-pack build; perf neutral
(ab keepalive: 585 vs 584 req/s GC on/off on a 2500-alloc/req page); framework-
like app correctness+leak test green.

NOTE: local Preside-on-RustCFML has a PRE-EXISTING post-first-request 500
(`Bootstrap.cfc:143`, reproduces with `RUSTCFML_NO_CYCLE_GC=1` → not this change)
that blocks a clean repeated-200 RSS measurement on Preside itself; the
framework-like test stands in for it.

## Implementation steps
1. New `crates/cfml-common/src/cycle_gc.rs`: `GC_ARMED`, `arm()`, `ALLOC_LOG`,
   `enable()/disable_and_clear()`, `on_alloc_*`, `tracked_scope()`, `collect()` +
   child-node enumeration + `clear` per node type. Export from `lib.rs`.
2. Hook the 3 ctors (dynamic.rs) + route the ≤6 captured_scope sites through
   `tracked_scope`.
3. cfml-vm wiring: `cycle_gc::arm()` in serve startup; `enable()` at top of a
   top-level `execute_with_lifecycle` (serve, not child thread); at request end
   (after writeback, guard on `live_threads.is_empty()`) clear transient roots →
   `collect()` → `disable_and_clear()`.
4. Optional `RUSTCFML_NO_CYCLE_GC` kill-switch + a `collected`-count stat (debug
   footer / log).

## Verify (release gate)
- Repro: all four endpoints flat over 3000 hits.
- `cargo run -- tests/runner.cfm` (CLI), serve cold+warm, `cargo test --workspace`
  (+JIT 76), wasm32, `wasm-pack build crates/wasm --target web`.
- Boot Preside; hammer a URL; confirm RSS plateaus (was: unbounded).
- Cross-check no false collection: full suites green + Preside functionally intact.

## Out of scope / follow-ups
- Cross-thread cycles & thread-spawning requests (skipped via the live_threads
  guard) — refine later (global active-thread counter / deferred re-scan).
- Long-running CLI scripts (logging off in CLI) — add a periodic incremental pass
  later if needed.
- **GH #221** (component-body `A = B = new CFC()` gives leftmost a COPY) is a
  SEPARATE, still-open chained-assignment/pseudo-constructor codegen bug
  (the v0.240.0 Dup fix addressed only the general want_value facet). It is
  ORTHOGONAL to this GC work (different code paths) — this change neither fixes
  nor regresses it. Track/fix separately.
