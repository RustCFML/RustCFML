//! Request-boundary cycle collector.
//!
//! RustCFML's reference-typed containers (`CfmlStruct`, `CfmlArray`, `CfmlQuery`)
//! and closure capture scopes (`Arc<RwLock<ValueMap>>`) are `Arc`-refcounted, so
//! a reference *cycle* (`a.other = b; b.other = a`, or a closure stored into the
//! scope it captures) is never reclaimed by refcounting alone — its internal
//! refs keep `strong_count > 0` even after every external root is gone. In a
//! long-lived `--serve` process that builds cyclic per-request graphs (Preside,
//! ColdBox, WireBox), this leaks a little on every request and RSS climbs without
//! bound.
//!
//! This module reclaims those cycles with a **request-scoped trial-deletion**
//! pass (Bacon–Rajan, bounded to one request's allocations). It does NOT replace
//! refcounting: the ~99% acyclic garbage is still freed eagerly, on-thread, with
//! zero pause. The collector only ever processes the small set of containers a
//! request allocated that are *still alive* at request end — never the whole
//! heap, never the resident persistent scopes — so there is no global
//! stop-the-world pause.
//!
//! ## How it stays correct without tracing the persistent scopes
//! The `Arc::strong_count` itself is the oracle. After the request's transient
//! roots (page `variables`, request scope, thread scope) are cleared, a survivor
//! that is still referenced from a *persistent* root (application/session/server
//! scope — which Arc-share the objects that escaped into them) has a strong count
//! greater than the number of references it gets from inside the survivor set; a
//! pure cycle does not. So we compute, per survivor `n`:
//!
//! ```text
//! external(n) = strong_count(n) − 1 (our own probe handle) − internal_in(n)
//! ```
//!
//! `external(n) > 0` ⟺ `n` has an owner outside the request's cyclic garbage ⟹
//! `n` is a live root. We mark the transitive closure of the roots live, and
//! everything else in the survivor set is an unreachable cycle: we clear its
//! backing (dropping its outgoing refs) so the whole subgraph's counts fall to
//! zero and it frees.
//!
//! ## Safety w.r.t. threads
//! Reading `strong_count` is only stable if no other thread is concurrently
//! cloning/dropping the same `Arc`. A truly-internal cycle (the only thing we
//! ever collect) is unreachable from any other request's thread by construction;
//! anything shared across threads escaped to a shared scope and thus reads as a
//! live root. The one case to guard is *this* request's own `cfthread`s, which
//! share `application`/`request` scope by Arc — the VM caller MUST skip
//! collection while `live_threads` is non-empty. See `CYCLE_GC_PLAN.md`.

use crate::dynamic::{CfmlQueryData, CfmlValue, StructInner, ValueMap};
#[cfg(feature = "component-instance")]
use crate::component::Instance;
use parking_lot::RwLock as PlRwLock;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, Weak};

/// Process-wide arm switch. `false` (default) makes every allocation hook a
/// single predictable-false relaxed load — CLI, tests, and wasm pay essentially
/// nothing. Set true once at `--serve` startup (unless `RUSTCFML_NO_CYCLE_GC`).
static GC_ARMED: AtomicBool = AtomicBool::new(false);

/// Total cycle nodes reclaimed across the process, for observability.
static COLLECTED_TOTAL: AtomicUsize = AtomicUsize::new(0);

/// Arm the collector (serve mode). Idempotent.
pub fn arm() {
    GC_ARMED.store(true, Ordering::Relaxed);
}

/// Disarm globally (e.g. `RUSTCFML_NO_CYCLE_GC=1`).
pub fn disarm() {
    GC_ARMED.store(false, Ordering::Relaxed);
}

#[inline]
pub fn is_armed() -> bool {
    GC_ARMED.load(Ordering::Relaxed)
}

/// Cumulative count of cycle nodes reclaimed (for the debug footer / logs).
pub fn collected_total() -> usize {
    COLLECTED_TOTAL.load(Ordering::Relaxed)
}

/// One logged allocation, held weakly so the log never extends an object's
/// lifetime (a dead object's `Weak` simply fails to upgrade at collection time).
#[derive(Clone)]
enum TrackedAlloc {
    Struct(Weak<PlRwLock<StructInner>>),
    Array(Weak<PlRwLock<Vec<CfmlValue>>>),
    Query(Weak<PlRwLock<CfmlQueryData>>),
    Scope(Weak<RwLock<ValueMap>>),
    /// A flyweight component `Instance`. The COLLECTIBLE node is the Instance Arc
    /// itself; its `this_members`/`variables_members` are untracked and owned by
    /// this Arc (the collector walks their values via the Instance node — see
    /// `classify` / `NodeHandle::Instance`). Tracking the Arc (not the maps) is
    /// what makes `Instance↔Instance` cycles reclaimable without the earlier
    /// over-collection of live component data.
    #[cfg(feature = "component-instance")]
    Instance(Weak<PlRwLock<Instance>>),
}

impl NodeHandle {
    /// Downgrade a survivor back to a tracking entry, so a node that outlived
    /// its request can stay under observation without being kept alive by the
    /// collector's own bookkeeping.
    fn downgrade(&self) -> TrackedAlloc {
        match self {
            NodeHandle::Struct(a) => TrackedAlloc::Struct(Arc::downgrade(a)),
            NodeHandle::Array(a) => TrackedAlloc::Array(Arc::downgrade(a)),
            NodeHandle::Query(a) => TrackedAlloc::Query(Arc::downgrade(a)),
            NodeHandle::Scope(a) => TrackedAlloc::Scope(Arc::downgrade(a)),
            #[cfg(feature = "component-instance")]
            NodeHandle::Instance(a) => TrackedAlloc::Instance(Arc::downgrade(a)),
        }
    }
}

thread_local! {
    /// Per-request allocation log. `Some` only while a TOP-LEVEL request body is
    /// executing on this worker thread; `None` everywhere else (CLI, between
    /// requests, and inside `cfthread` child threads — so child-thread allocs are
    /// never logged and never accumulate). Taking the log out (`collect`) also
    /// leaves it `None`, so the collector's own allocations are never logged.
    static ALLOC_LOG: RefCell<Option<Vec<TrackedAlloc>>> = const { RefCell::new(None) };
}

/// Soft cap on the per-request allocation log, as a pure MEMORY safety valve —
/// NOT a functional gate. A real framework request (Preside, ColdBox, Wheels)
/// routinely allocates well over a million containers, so the old 1M cap caused
/// every such request to "overflow and skip collection", which is exactly the
/// runaway serve-mode leak this collector exists to prevent. The cap is now set
/// far above real request sizes, and — critically — overflowing it no longer
/// abandons collection: logging simply STOPS (bounding the log's own memory to
/// ~`LOG_CAP * sizeof(Weak)` ≈ 16 bytes each) while `collect()` still reclaims
/// every cycle among the allocations logged BEFORE the cap was reached.
///
/// Collecting a partial log is provably conservative: any allocation that was
/// never logged is absent from the survivor set, so edges to it are counted as
/// external ownership (a live root) and its subgraph is protected. Thus a
/// partial pass may under-collect (leak a little, that request only) but can
/// NEVER over-collect a live object. Acyclic garbage is freed eagerly by
/// refcounting regardless. The cap therefore only ever trades a little extra
/// retained memory on a pathological alloc-churning request for a hard bound on
/// the collector's transient bookkeeping — it never silently disables the
/// collector the way the old threshold did.
const LOG_CAP_DEFAULT: usize = 16_000_000;

/// Effective per-request log cap. Overridable via `RUSTCFML_GC_LOG_CAP` (read
/// once) so the bound can be tuned/experimented with at runtime without a
/// rebuild. Falls back to `LOG_CAP_DEFAULT`.
fn log_cap() -> usize {
    use std::sync::OnceLock;
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("RUSTCFML_GC_LOG_CAP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(LOG_CAP_DEFAULT)
    })
}

/// Begin logging allocations for a request. Call at the very top of a top-level
/// request execution (serve mode only).
pub fn enable() {
    ALLOC_LOG.with(|c| *c.borrow_mut() = Some(Vec::new()));
    NEXT_SWEEP.with(|c| c.set(incremental_threshold()));
}

/// Stop logging and drop the log without collecting.
pub fn disable_and_clear() {
    ALLOC_LOG.with(|c| *c.borrow_mut() = None);
}

/// Current length of this thread's allocation log (`None` if not logging). For
/// diagnostics only.
pub fn log_len() -> Option<usize> {
    ALLOC_LOG.with(|c| c.borrow().as_ref().map(|v| v.len()))
}

/// Composition of this thread's allocation log by container type
/// `(structs, arrays, queries, closure_scopes)`. Diagnostics only — answers
/// "what are the N tracked allocations a request made?" without a heap profiler.
pub fn log_type_breakdown() -> (usize, usize, usize, usize) {
    ALLOC_LOG.with(|c| {
        let b = c.borrow();
        let mut t = (0usize, 0usize, 0usize, 0usize);
        if let Some(v) = b.as_ref() {
            for a in v {
                match a {
                    TrackedAlloc::Struct(_) => t.0 += 1,
                    TrackedAlloc::Array(_) => t.1 += 1,
                    TrackedAlloc::Query(_) => t.2 += 1,
                    TrackedAlloc::Scope(_) => t.3 += 1,
                    // Instances are tracked nodes but not surfaced in this
                    // struct/array/query/scope diagnostic tuple.
                    #[cfg(feature = "component-instance")]
                    TrackedAlloc::Instance(_) => {}
                }
            }
        }
        t
    })
}

// --- Deferred collection (requests that end with a thread still running) -----
//
// A request may end while a `cfthread` it spawned is STILL executing (true
// fire-and-forget background work that outlives the response — explicitly
// allowed by CFML). We must not collect then: a running thread can hold and
// mutate Arcs into the request's graph, so `strong_count` reads would race, and
// joining it would wrongly block the response. We also must not DISCARD the log
// (that would leak the request's cycles forever — nothing else records them).
//
// Instead we DEFER: stash the request's log together with the still-running
// threads' join handles in a small global queue. Later — at every request
// boundary and on a periodic sweep — we collect each entry whose threads have
// ALL finished. A finished thread has returned from its body and dropped every
// Arc it held (verified: the spawn closure drops its child VM, sends-or-drops
// its result, and drops its sender before `is_finished()` flips true), so the
// entry's pure cycles then have stable, internal-only refcounts and collect
// safely. This guarantees there is no scenario in which unused data is never
// collected.

/// One deferred request log plus the join handles of the threads whose
/// completion gates its collection.
struct DeferredEntry {
    log: Vec<TrackedAlloc>,
    joins: Vec<std::thread::JoinHandle<()>>,
}

/// Global queue of deferred logs. Small: one entry per in-flight
/// background-thread-spawning request, drained as those threads finish.
/// `parking_lot::Mutex::new` is const, so this needs no lazy init.
static DEFERRED: parking_lot::Mutex<Vec<DeferredEntry>> = parking_lot::Mutex::new(Vec::new());

/// Number of deferred logs currently awaiting their threads (observability).
pub fn deferred_pending() -> usize {
    DEFERRED.lock().len()
}

/// Take this thread's current allocation log and defer its collection until the
/// given still-running threads finish. Call this INSTEAD of `collect` +
/// `disable_and_clear` when a request ends with a thread still executing. If the
/// log is empty/absent there is nothing to track — the join handles are simply
/// dropped (detaching the threads, which keep running as before).
pub fn defer_current_log(joins: Vec<std::thread::JoinHandle<()>>) {
    let log = ALLOC_LOG.with(|c| c.borrow_mut().take());
    match log {
        Some(log) if !log.is_empty() && !joins.is_empty() => {
            DEFERRED.lock().push(DeferredEntry { log, joins });
        }
        // No cycles logged, or no still-running threads to wait on: nothing to
        // defer. Dropping `joins` just detaches (the default for cfthread).
        _ => {}
    }
}

/// Sweep the deferred queue: collect every entry whose threads have all
/// finished, leaving the rest. Cheap when the queue is empty (one uncontended
/// lock + length check). Called at each request boundary and by the periodic
/// sweep so deferred logs are reclaimed even on an otherwise-idle server.
/// Returns the number of cycle nodes reclaimed this sweep.
pub fn collect_ready_deferred() -> usize {
    // Phase 1: under the lock, move out the entries whose threads are all done.
    // Keep the lock hold short — do the actual (potentially heavy) collection
    // outside it. Each ready entry is owned by exactly one sweeping thread.
    let ready: Vec<DeferredEntry> = {
        let mut q = DEFERRED.lock();
        if q.is_empty() {
            return 0;
        }
        let mut ready = Vec::new();
        let mut i = 0;
        while i < q.len() {
            if q[i].joins.iter().all(|j| j.is_finished()) {
                ready.push(q.swap_remove(i));
            } else {
                i += 1;
            }
        }
        ready
    };

    let mut total = 0;
    for entry in ready {
        // Join the finished threads to release their OS resources (returns
        // immediately — they have already completed).
        for j in entry.joins {
            let _ = j.join();
        }
        total += collect_from_log(entry.log);
    }
    if total > 0 && std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!("[cycle_gc] deferred sweep reclaimed {} node(s)", total);
    }
    total
}

#[inline]
fn log_push(t: TrackedAlloc) {
    ALLOC_LOG.with(|c| {
        let mut b = c.borrow_mut();
        if let Some(v) = b.as_mut() {
            if v.len() >= log_cap() {
                // Overflow: STOP logging further allocations for this request, but
                // KEEP what we have so `collect()` still reclaims the cycles among
                // the logged subset. Collecting a partial log is conservative
                // (unlogged objects read as external roots → never over-collect),
                // so this caps the collector's bookkeeping memory without ever
                // disabling collection. The skip stays quiet unless debugging.
                if !OVERFLOW_WARNED.swap(true, Ordering::Relaxed)
                    && std::env::var("RUSTCFML_GC_DEBUG").is_ok()
                {
                    eprintln!(
                        "[cycle_gc] log reached cap={} — logging paused for this request; \
                         partial (conservative) collection will still run",
                        log_cap()
                    );
                }
                // Leave the log in place (do not null it out); just drop `t`.
            } else {
                v.push(t);
            }
        }
    });
}

/// One-shot guard so the cap-reached notice is printed at most once per process
/// (it is otherwise per-allocation noise once a request crosses the cap).
static OVERFLOW_WARNED: AtomicBool = AtomicBool::new(false);

// --- Sampling allocation profiler (diagnostics; off unless env-enabled) -------
//
// Set `RUSTCFML_GC_SAMPLE=N` to capture a backtrace on 1-in-N struct/array
// allocations, aggregate by call site, and print the top sites at each request
// end (see cli `request end` handler). Per-request + thread-local, so it scopes
// to one steady-state request and skips boot noise. Build with
// `--profile profiling` for symbol names. Zero cost when the env var is unset
// (one OnceLock load returning 0 → the hot path never captures).

fn sample_rate() -> usize {
    use std::sync::OnceLock;
    static RATE: OnceLock<usize> = OnceLock::new();
    *RATE.get_or_init(|| {
        std::env::var("RUSTCFML_GC_SAMPLE")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
    })
}

thread_local! {
    /// `(counter, site -> count)` for the current request when sampling is on.
    static SAMPLES: RefCell<(usize, HashMap<String, usize>)> =
        RefCell::new((0, HashMap::new()));
}

#[inline]
fn maybe_sample() {
    let rate = sample_rate();
    if rate == 0 {
        return;
    }
    SAMPLES.with(|c| {
        let mut s = c.borrow_mut();
        s.0 += 1;
        if s.0 % rate != 0 {
            return;
        }
        // Capture + symbolize a backtrace, then key by the first CFML engine
        // frame below the allocation hooks (the actual allocating call site).
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let site = bt
            .lines()
            .map(|l| l.trim())
            .find(|l| {
                (l.contains("cfml_vm")
                    || l.contains("cfml_stdlib")
                    || l.contains("cfml_codegen")
                    || l.contains("cfml_compiler"))
                    && !l.contains("cycle_gc")
                    && !l.contains("maybe_sample")
                    && !l.contains("log_struct")
                    && !l.contains("log_array")
                    && !l.contains("::strukt")
                    && !l.contains("CfmlValue::array")
                    && !l.contains("CfmlArray::new")
                    && !l.contains("CfmlStruct::new")
            })
            .map(|l| {
                // strip the leading "N: " frame index and trailing hash
                let l = l.splitn(2, ": ").nth(1).unwrap_or(l);
                l.split("::h").next().unwrap_or(l).to_string()
            })
            .unwrap_or_else(|| "<unresolved>".to_string());
        *s.1.entry(site).or_insert(0) += 1;
    });
}

/// Drain and format the top-`k` sampled allocation sites for this request.
/// Returns `None` when sampling is disabled. Resets the per-request state.
pub fn drain_top_sites(k: usize) -> Option<Vec<(String, usize)>> {
    if sample_rate() == 0 {
        return None;
    }
    SAMPLES.with(|c| {
        let mut s = c.borrow_mut();
        let mut v: Vec<(String, usize)> = s.1.drain().collect();
        s.0 = 0;
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(k);
        Some(v)
    })
}

// --- Allocation hooks (called from the container constructors) ---------------
// Each is gated by `is_armed()` so the disarmed path is a single relaxed load.

/// Diagnostic: `RUSTCFML_GC_TRACK_ALL=1` logs even the allocations that are
/// DELIBERATELY untracked ([`crate::dynamic::CfmlStruct::new_untracked`] and the
/// component data maps).
///
/// Those are untracked for good reasons — they cannot escape their frame, or they
/// are owned solely by an `Instance` Arc — and leaving them out is what keeps the
/// collector's hot path cheap. But it also makes them INVISIBLE as holders: a
/// pinned root reports `external = 1` with no way to say what the 1 is, because
/// the holder is not a node. Turning them all on trades throughput for the
/// ability to name that holder, which is exactly the trade a leak hunt wants.
pub fn track_all() -> bool {
    use std::sync::OnceLock;
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| std::env::var("RUSTCFML_GC_TRACK_ALL").is_ok())
}

#[inline]
pub fn log_struct(arc: &Arc<PlRwLock<StructInner>>) {
    if is_armed() {
        log_push(TrackedAlloc::Struct(Arc::downgrade(arc)));
        maybe_sample();
    }
}

#[inline]
pub fn log_array(arc: &Arc<PlRwLock<Vec<CfmlValue>>>) {
    if is_armed() {
        log_push(TrackedAlloc::Array(Arc::downgrade(arc)));
        maybe_sample();
    }
}

#[inline]
pub fn log_query(arc: &Arc<PlRwLock<CfmlQueryData>>) {
    if is_armed() {
        log_push(TrackedAlloc::Query(Arc::downgrade(arc)));
    }
}

/// Track a flyweight component `Instance` Arc as a cycle node. Call once at
/// Instance creation (`make_instance_value` / `duplicate`). Held weakly, so a
/// short-lived Instance freed by refcounting before request end simply fails to
/// upgrade at collection time. No-op unless the collector is armed.
#[cfg(feature = "component-instance")]
#[inline]
pub fn log_instance(arc: &Arc<PlRwLock<Instance>>) {
    if is_armed() {
        log_push(TrackedAlloc::Instance(Arc::downgrade(arc)));
    }
}

/// Track an ALREADY-ALLOCATED closure-capture scope as a cycle node. Sibling of
/// [`log_struct`] / [`log_array`] for the one node type that has no constructor
/// of its own here — [`tracked_scope`] allocates and logs in one step, but a
/// scope reached by walking an existing graph (see
/// [`CfmlValue::relog_cycle_nodes`](crate::dynamic::CfmlValue::relog_cycle_nodes))
/// must be entered after the fact. Logging the same scope twice is harmless: the
/// collector de-duplicates survivors by backing pointer.
#[inline]
pub fn log_scope(arc: &Arc<RwLock<ValueMap>>) {
    if is_armed() {
        log_push(TrackedAlloc::Scope(Arc::downgrade(arc)));
    }
}

/// Allocate a closure-capture scope, tracking it as a cycle node. Use this in
/// place of `Arc::new(RwLock::new(map))` for every `captured_scope`/`closure_env`
/// so closure↔scope cycles are reclaimable.
#[inline]
pub fn tracked_scope(map: ValueMap) -> Arc<RwLock<ValueMap>> {
    let arc = Arc::new(RwLock::new(map));
    if is_armed() {
        log_push(TrackedAlloc::Scope(Arc::downgrade(&arc)));
    }
    arc
}

// --- The collection pass -----------------------------------------------------

/// A strong handle to one survivor, holding exactly ONE reference (subtracted as
/// the "probe handle" when computing external ownership).
enum NodeHandle {
    Struct(Arc<PlRwLock<StructInner>>),
    Array(Arc<PlRwLock<Vec<CfmlValue>>>),
    Query(Arc<PlRwLock<CfmlQueryData>>),
    Scope(Arc<RwLock<ValueMap>>),
    #[cfg(feature = "component-instance")]
    Instance(Arc<PlRwLock<Instance>>),
}

impl NodeHandle {
    #[inline]
    fn strong_count(&self) -> usize {
        match self {
            NodeHandle::Struct(a) => Arc::strong_count(a),
            NodeHandle::Array(a) => Arc::strong_count(a),
            NodeHandle::Query(a) => Arc::strong_count(a),
            NodeHandle::Scope(a) => Arc::strong_count(a),
            #[cfg(feature = "component-instance")]
            NodeHandle::Instance(a) => Arc::strong_count(a),
        }
    }

    /// Enumerate the immediate child *nodes* (members of `in_set`) without
    /// disturbing any TRACKED node's refcount — terminal at node types,
    /// descending through non-node carriers (Function/Component/Closure/
    /// QueryColumn). Holds a read guard for the duration; the callback only
    /// records ids and never locks another node, so this cannot deadlock. (The
    /// `Instance` arm is the one place a handle is cloned — the two UNTRACKED
    /// data maps, whose refcounts the collector never inspects — so the
    /// "refcounts undisturbed" guarantee still holds for every tracked node.)
    fn for_each_child_node(&self, in_set: &HashSet<usize>, emit: &mut impl FnMut(usize)) {
        match self {
            NodeHandle::Struct(a) => {
                let g = a.read();
                for v in g.map.values() {
                    classify(v, in_set, emit);
                }
                // NOTE: `method_table` (the shared per-class `Arc<ValueMap>` hung
                // off a component's scope structs) is deliberately NOT walked here.
                // It is the blueprint's `method_values`, and the blueprint carrier
                // walks it EXACTLY ONCE per pass. Walking it from each holder would
                // count every one of its edges once per instance of the class,
                // deflating its children's external count — the double-count that
                // over-collects live data.
            }
            NodeHandle::Array(a) => {
                let g = a.read();
                for v in g.iter() {
                    classify(v, in_set, emit);
                }
            }
            NodeHandle::Query(a) => {
                let g = a.read();
                for col in &g.data {
                    for v in col.iter() {
                        classify(v, in_set, emit);
                    }
                }
            }
            NodeHandle::Scope(a) => {
                if let Ok(g) = a.read() {
                    for v in g.values() {
                        classify(v, in_set, emit);
                    }
                }
            }
            // The Instance's OWN outgoing edges: walk both data-map value sets so
            // edges to OTHER tracked nodes (other Instances, structs, arrays,
            // closure scopes) are surfaced EXACTLY ONCE — here, on the Instance
            // node — never re-walked by each holder of the Instance (`classify`'s
            // Instance arm is terminal). This is what keeps `internal_in` accurate
            // and avoids the double-count that would deflate a shared child's
            // external count and over-collect it. The data maps themselves are
            // untracked, so we never emit their backing ptrs (they can't be in
            // `in_set`); we only classify the VALUES they hold.
            //
            // `try_read` + skip-if-locked (a lingering finished cfthread could hold
            // the lock): skipping under-counts this node's outgoing internal edges,
            // which can only INFLATE its children's external counts (protecting
            // them) — conservative, never over-collects. Handles are cloned out and
            // the Instance lock released before touching the maps (no nested lock).
            #[cfg(feature = "component-instance")]
            NodeHandle::Instance(a) => {
                let maps = a
                    .try_read()
                    .map(|g| (g.public_map_handle(), g.private_map_handle()));
                // A CFC extending a Rust class holds the parent OBJECT here; it
                // is a plain `CfmlValue` field of the Instance, walked nowhere
                // else.
                if let Some(g) = a.try_read() {
                    if let Some(np) = g.native_parent.as_ref() {
                        classify(np, in_set, emit);
                    }
                }
                if let Some((this_m, vars_m)) = maps {
                    for m in [this_m, vars_m] {
                        // A data map is USUALLY untracked and owned solely by this
                        // Arc, so its values are walked from here. But it is not
                        // always: a component that defines a closure keeps its LIVE
                        // `variables` scope (the closure captured it) instead of the
                        // partitioned copy, and that scope IS a tracked node. When
                        // it is, emit the MAP — the Instance genuinely references
                        // it, and leaving that edge uncounted made the map's own
                        // external count read 1, turning every such instance's
                        // scope into a pinned root and marking its whole object
                        // graph live. On a Preside `?fwreinit=true` that stranded a
                        // complete generation per reload.
                        //
                        // Emitting it is also why the values must NOT be walked in
                        // that case: the map is its own survivor and walks them
                        // itself, so doing both would double-count `internal_in`
                        // for every shared child, deflate its external count and
                        // over-collect live data (the double-walk that dropped a
                        // live `EventHandlerBean`'s `viewDispatch`, 2026-07-22).
                        let p = m.backing_ptr();
                        if in_set.contains(&p) {
                            emit(p);
                        } else {
                            m.with_read(|mm| {
                                for v in mm.values() {
                                    classify(v, in_set, emit);
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    /// A short human description used by the pinned-roots diagnostic: the node
    /// kind plus, for maps, its first few keys — which is what identifies the
    /// object to a CFML developer.
    fn describe(&self) -> String {
        fn keys_of(m: &ValueMap) -> String {
            let ks: Vec<String> = m.iter().take(6).map(|(k, _)| k.to_string()).collect();
            format!("{} keys [{}]", m.len(), ks.join(", "))
        }
        match self {
            NodeHandle::Struct(a) => match a.try_read() {
                Some(g) => format!("Struct {}", keys_of(&g.map)),
                None => "Struct <locked>".to_string(),
            },
            NodeHandle::Array(a) => match a.try_read() {
                Some(g) => format!("Array len={}", g.len()),
                None => "Array <locked>".to_string(),
            },
            NodeHandle::Query(_) => "Query".to_string(),
            NodeHandle::Scope(a) => match a.try_read() {
                Ok(g) => format!("ClosureScope {}", keys_of(&g)),
                Err(_) => "ClosureScope <locked>".to_string(),
            },
            #[cfg(feature = "component-instance")]
            NodeHandle::Instance(a) => match a.try_read() {
                Some(g) => format!("Instance of {}", g.class.name),
                None => "Instance <locked>".to_string(),
            },
        }
    }

    /// Break this node's cycle by clearing its contents (drops its outgoing refs).
    fn clear(&self) {
        match self {
            NodeHandle::Struct(a) => a.write().map.clear(),
            NodeHandle::Array(a) => a.write().clear(),
            NodeHandle::Query(a) => {
                let mut g = a.write();
                g.data.clear();
                g.columns.clear();
            }
            NodeHandle::Scope(a) => {
                if let Ok(mut g) = a.write() {
                    g.clear();
                }
            }
            // Break the Instance's cycle by clearing its data maps (drops the Arcs
            // it holds to other cycle members). We do NOT drop the Instance Arc
            // itself — the probe handles in `nodes` are dropped after this pass and
            // the strong count falls to zero naturally. `try_write`: a node we
            // reached here is non-live (no external owner), so nothing should hold
            // its lock; skip-if-contended rather than block (defensive — a stuck
            // lock would only leak this one cycle for one request).
            #[cfg(feature = "component-instance")]
            NodeHandle::Instance(a) => {
                if let Some(g) = a.try_write() {
                    g.clear_all_members();
                }
            }
        }
    }
}

/// Record any child *nodes* reachable from `v`. Node types (Struct/Array/Query,
/// and the Scope behind a Function's `captured_scope`) are terminal — emitted but
/// not descended (each is processed as its own survivor). Non-node carriers
/// (Component/Closure boxes, QueryColumn) are descended into, since they are not
/// separately collectible. `NativeObject` is opaque and treated as an external
/// owner (anything it holds stays protected — conservative, never over-collects).
fn classify(v: &CfmlValue, in_set: &HashSet<usize>, emit: &mut impl FnMut(usize)) {
    match v {
        CfmlValue::Struct(s) => {
            let p = s.backing_ptr();
            if in_set.contains(&p) {
                emit(p);
            }
        }
        CfmlValue::Array(a) => {
            let p = a.backing_ptr();
            if in_set.contains(&p) {
                emit(p);
            }
        }
        CfmlValue::Query(q) => {
            let p = q.backing_ptr();
            if in_set.contains(&p) {
                emit(p);
            }
        }
        CfmlValue::Function(f) => {
            if let Some(sc) = &f.captured_scope {
                let p = Arc::as_ptr(sc) as *const () as usize;
                if in_set.contains(&p) {
                    emit(p);
                }
            }
        }
        CfmlValue::Component(c) => {
            for pv in c.properties.values() {
                classify(pv, in_set, emit);
            }
            for m in c.methods.values() {
                if let Some(sc) = &m.captured_scope {
                    let p = Arc::as_ptr(sc) as *const () as usize;
                    if in_set.contains(&p) {
                        emit(p);
                    }
                }
            }
        }
        CfmlValue::Closure(c) => {
            for cv in c.captured_vars.values() {
                classify(cv, in_set, emit);
            }
        }
        CfmlValue::QueryColumn(col, _) => {
            for cv in col.iter() {
                classify(cv, in_set, emit);
            }
        }
        // A native object is not a node of its own, but it CAN hold CfmlValues —
        // a Future's result, an executor's queued task bodies. Descend through it
        // exactly like a Component or Closure box, or everything it holds reads as
        // externally owned and is pinned forever (see `CfmlNative::visit_values`).
        // A native reachable from several survivors has its edges counted once per
        // holder; over-counting `internal_in` is corrected by the mark phase (a
        // child reachable from any LIVE holder is marked live through this same
        // descent), so it can only under-collect, never over-collect.
        CfmlValue::NativeObject(n) => {
            if let Ok(g) = n.read() {
                g.visit_values(&mut |v| classify(v, in_set, emit));
            }
        }
        // A flyweight component `Instance` (`Arc<RwLock<Instance>>`) is a TRACKED,
        // collectible node (`TrackedAlloc::Instance` / `NodeHandle::Instance`), so
        // it is TERMINAL here exactly like Struct/Array/Query: emit the Instance
        // ptr if it is a survivor and STOP. We must NOT descend into its data maps
        // from this arm — that descent is done once, by the Instance node's own
        // `for_each_child_node`. Descending here would make every holder of the
        // Instance re-walk its members, double-counting `internal_in` for shared
        // children, deflating their external count, and OVER-COLLECTING live data.
        // (That double-walk — plus the earlier variant that tracked the data maps
        // directly — is what 500'd Preside's cached `EventHandlerBean` by dropping
        // `variables.viewDispatch` on a warm request; bisected 2026-07-22.)
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => {
            let p = Arc::as_ptr(inst) as *const () as usize;
            if in_set.contains(&p) {
                emit(p);
            }
        }
        _ => {}
    }
}

/// Enumerate every `CfmlValue` a class blueprint holds.
///
/// A blueprint is `Arc<ClassBlueprint>`, NOT a `CfmlValue`, so it is invisible
/// to [`classify`] — and it is held by every `Instance` of its class. Left out
/// of the graph, each of these fields reads as an EXTERNAL reference into the
/// tracked set and pins its whole transitive closure, while the blueprint itself
/// is kept alive by the very instances it is pinning. That is a cycle straddling
/// an untracked node: refcounting cannot break it and trial-deletion never sees
/// it. On a Preside `?fwreinit=true` it stranded ~111,000 nodes per reload —
/// one blueprint set per class per request, so every reload leaked a generation.
#[cfg(feature = "component-instance")]
fn blueprint_values(bp: &crate::component::ClassBlueprint, mut f: impl FnMut(&CfmlValue)) {
    f(&bp.metadata);
    for v in bp.method_values.values() {
        f(v);
    }
    for v in [
        &bp.static_scope,
        &bp.super_handle,
        &bp.super_map,
        &bp.source_names,
        &bp.properties,
    ]
    .into_iter()
    .flatten()
    {
        f(v);
    }
    // `try_read`: skipping a contended lock can only UNDER-count this carrier's
    // outgoing edges, which inflates its children's external count and protects
    // them — conservative, never over-collects.
    if let Some(g) = bp.metadata_cache.try_read() {
        if let Some(v) = g.as_ref() {
            f(v);
        }
    }
}

/// Run the request-scoped cycle collection. Drains this thread's allocation log,
/// reclaims unreachable cycles among the request's surviving allocations, and
/// returns the number of nodes reclaimed.
///
/// PRECONDITIONS (the VM caller must establish these):
///  1. No `cfthread` is still *running* (finished-but-lingering handles are OK —
///     a request that ends with a thread still executing must instead
///     `defer_current_log` so its log is collected later, not discarded).
///  2. Persistent scopes already written back to `ServerState`.
///  3. Transient roots (page `variables`, request scope, thread scope) cleared
///     — in practice satisfied by dropping the VM before calling this.
pub fn collect() -> usize {
    let Some(log) = ALLOC_LOG.with(|c| c.borrow_mut().take()) else {
        return 0;
    };
    // Survivors are carried into the cross-request set rather than abandoned —
    // see [`PersistentSet`] for why dropping them was a permanent loss of
    // tracking, and for the sweep's cost and correctness argument.
    let mut live: Vec<(usize, TrackedAlloc)> = Vec::new();
    let reclaimed = collect_from_log_carrying(log, Some(&mut live));
    reclaimed + carry_survivors(live)
}

/// The collection pass over an explicit allocation log (the live request's,
/// drained by `collect`, or a previously-deferred one). Identical algorithm
/// either way; factored out so deferred logs can be collected after their
/// spawning request's threads finish. Safe to run concurrently with unrelated
/// requests: the cycles it touches are internal to one finished request and
/// unreachable from anywhere else, so their `strong_count`s are stable.
/// Number of tracked allocations after which a MID-REQUEST sweep is allowed.
/// `0` disables incremental sweeping (end-of-request only, the historical
/// behaviour). Override with `RUSTCFML_GC_INCREMENTAL`.
///
/// Why this exists: `collect()` used to run ONLY at request end, so every cycle
/// a request minted was retained for the whole request. A CFC instance is
/// inherently cyclic (the body keeps the instance in a local named after the
/// component, which lands in `variables`, closing
/// `this -> __variables -> variables -> this`), so a request constructing many
/// components grew without bound: 100k constructions of an 86-method CFC held
/// **1.8 GB**, 400k held **7.2 GB**, with `survivors=300001` at 100k — three
/// uncollectable cycles per construction, every one of them reclaimable.
/// Chosen by measurement (86-method CFC, warm serve, both directions tested):
///
/// | base   | 200k discarded ctors | 60k LIVE components |
/// |--------|----------------------|---------------------|
/// | 0 (off)| 3.6 G / 16.0 s       | 1.1 G / 4.8 s       |
/// | 10,000 | 61 M / 15.5 s        | 201 M / **5.1 s**   |
/// | 25,000 | **113 M / 13.1 s**   | **218 M / 3.5 s**   |
/// | 50,000 | 177 M / 11.1 s       | 221 M / 3.5 s       |
///
/// 25,000 is the knee: ~32x less memory on churn and ~5x on a large live set,
/// with CPU at or below the sweeping-off arm on BOTH. 10,000 buys a little more
/// memory back but starts paying for the extra passes on the live workload.
const INCREMENTAL_DEFAULT: usize = 25_000;

fn incremental_threshold() -> usize {
    use std::sync::OnceLock;
    static T: OnceLock<usize> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("RUSTCFML_GC_INCREMENTAL")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(INCREMENTAL_DEFAULT)
    })
}

impl TrackedAlloc {
    /// Whether the tracked allocation is still alive (its `Weak` upgrades).
    fn is_alive(&self) -> bool {
        match self {
            TrackedAlloc::Struct(w) => w.strong_count() > 0,
            TrackedAlloc::Array(w) => w.strong_count() > 0,
            TrackedAlloc::Query(w) => w.strong_count() > 0,
            TrackedAlloc::Scope(w) => w.strong_count() > 0,
            #[cfg(feature = "component-instance")]
            TrackedAlloc::Instance(w) => w.strong_count() > 0,
        }
    }
}

thread_local! {
    /// Log length at which the NEXT mid-request sweep is allowed. Reset to the
    /// base threshold when a request arms the log, then raised adaptively — see
    /// [`collect_incremental`].
    static NEXT_SWEEP: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}

/// Run a collection pass NOW if this request's log has grown past the current
/// adaptive budget. Returns the number of nodes reclaimed.
///
/// **Correctness** is the same conservative argument the end-of-request pass
/// uses: an object still referenced from outside the logged set (the VM stack, a
/// frame's locals, an outer scope) has an external strong reference and is
/// classified live, so a mid-request pass can never over-collect. The extra
/// obligation is that a survivor stays VISIBLE to later passes — it may become
/// cyclic garbage later in the same request — so still-alive handles are
/// re-registered before returning.
///
/// **Why the budget is adaptive, not a fixed count.** Re-registering survivors
/// means each pass rescans everything still alive, so a fixed threshold is
/// quadratic in the live set: a request holding 60k live components went from
/// 3.4 s (sweeping off) to over TEN MINUTES at a flat 10k threshold. The budget
/// is therefore raised to twice the surviving live set after every pass — the
/// classic "collect again when the heap has doubled" rule. A churn workload
/// (nothing survives) keeps sweeping at the base threshold and stays flat; a
/// workload that genuinely holds N objects live sweeps O(log N) times, so the
/// total rescan work stays linear-ish instead of quadratic.
///
/// Must not be called while an `ALLOC_LOG` borrow is held.
pub fn collect_incremental() -> usize {
    let base = incremental_threshold();
    if base == 0 {
        return 0;
    }
    let budget = NEXT_SWEEP.with(|c| c.get());
    let budget = if budget == usize::MAX { base } else { budget };
    let log = ALLOC_LOG.with(|c| {
        let mut b = c.borrow_mut();
        match b.as_mut() {
            Some(v) if v.len() >= budget => Some(std::mem::take(v)),
            _ => None,
        }
    });
    let Some(log) = log else { return 0 };
    let carry = log.clone();
    let reclaimed = collect_from_log(log);
    let mut live = 0usize;
    ALLOC_LOG.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            for t in carry {
                if t.is_alive() {
                    live += 1;
                    v.push(t);
                }
            }
        }
    });
    NEXT_SWEEP.with(|c| c.set(std::cmp::max(base, live.saturating_mul(2))));
    if reclaimed > 0 && std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!(
            "[cycle_gc] incremental sweep reclaimed {} node(s); {} live, next sweep at {}",
            reclaimed,
            live,
            std::cmp::max(base, live.saturating_mul(2))
        );
    }
    reclaimed
}

/// --- The cross-request survivor set -----------------------------------------
///
/// A request's log is drained by [`collect`] and then GONE. Everything in it
/// that was still alive — anything that escaped into application, session or
/// server scope, and everything those graphs reach — therefore stopped being
/// tracked the moment that request ended, and nothing ever looked at it again.
/// On a live Preside request that is ~206,000 nodes abandoned per request. If
/// any of them later became cyclic garbage, no pass would ever find it:
/// refcounting cannot free a cycle, and the collector only ever saw containers
/// the CURRENT request allocated.
///
/// [`CfmlValue::relog_cycle_nodes`](crate::dynamic::CfmlValue::relog_cycle_nodes)
/// patched one shape of that hole (a value displaced from a scope struct flagged
/// persistent), but it is a hook on specific mutations — it cannot cover a
/// displacement one level down (`application.cache.x = y`, where `cache` is a
/// plain struct), a session expiring, or any of the other ways a persistent
/// graph becomes garbage. The general fix is not to stop tracking.
///
/// So survivors are carried forward here instead of being dropped:
///
///  * **De-duplicated by backing pointer**, so a steady-state application whose
///    survivors are the same nodes every request adds nothing after the first.
///    The set grows only when genuinely new long-lived objects appear.
///  * **Swept on the doubling rule**, exactly like [`collect_incremental`]: a
///    pass runs when the set has grown to twice its last live size. A normal
///    request therefore pays a hash probe per survivor and nothing else; the
///    sweep lands on the request that actually created a new generation (a
///    framework reload), which is precisely the request that made the garbage.
///  * **Weakly**, so the set never keeps anything alive and entries whose object
///    was freed by refcounting fall out at the next sweep.
///
/// Correctness is the same conservative argument the request-scoped pass uses,
/// and it does not depend on other requests being idle: a node still owned from
/// outside the set — a live request's stack, a frame local, an untracked
/// container, a `NativeObject` — has an external strong reference, is classified
/// live, and its whole transitive closure is marked live with it. A concurrent
/// mutation can only add references (protecting more), and a reference MOVED
/// between two set members is still found by the mark phase via whichever member
/// is live. Only a genuine cycle with no external owner is ever reclaimed.
struct PersistentSet {
    entries: Vec<TrackedAlloc>,
    seen: HashSet<usize>,
    next_sweep: usize,
}

static PERSISTENT: parking_lot::Mutex<Option<PersistentSet>> = parking_lot::Mutex::new(None);

/// Floor for the cross-request sweep budget, so a small application does not
/// sweep on every request just because its live set is tiny. Overridable with
/// `RUSTCFML_GC_PERSISTENT` (`0` disables carrying survivors forward entirely,
/// restoring the drop-on-request-end behaviour).
const PERSISTENT_BASE_DEFAULT: usize = 50_000;

/// Diagnostic: sweep the cross-request set at EVERY request end instead of on
/// the doubling rule (`RUSTCFML_GC_PERSISTENT_ALWAYS=1`). The budget exists so a
/// steady-state request pays nothing, which is right for production and wrong
/// for answering "is this reload's generation being reclaimed or pinned?" — with
/// this on, every request prints its own reclaimed/still-tracked line, and
/// pairing it with `RUSTCFML_GC_ROOTS=N` names whatever is doing the pinning.
fn persistent_always() -> bool {
    use std::sync::OnceLock;
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| std::env::var("RUSTCFML_GC_PERSISTENT_ALWAYS").is_ok())
}

fn persistent_base() -> usize {
    use std::sync::OnceLock;
    static B: OnceLock<usize> = OnceLock::new();
    *B.get_or_init(|| {
        std::env::var("RUSTCFML_GC_PERSISTENT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(PERSISTENT_BASE_DEFAULT)
    })
}

/// A value the collector should treat as a NAMED probe root for diagnostics —
/// in practice the live `application` scope, handed over by the request loop
/// just before `collect()`.
///
/// It answers the one question that separates an engine leak from an application
/// one: of the nodes that survive a request, how many are reachable from the
/// application scope? Nodes reachable from it are retained BY THE APPLICATION and
/// the engine is behaving. Nodes that survive while unreachable from it are held
/// by something else — an untracked engine carrier — and that is ours to fix.
static PROBE_ROOT: parking_lot::Mutex<Vec<(String, CfmlValue)>> =
    parking_lot::Mutex::new(Vec::new());

/// Install the diagnostic probe root (see [`PROBE_ROOT`]). Cheap no-op unless
/// `RUSTCFML_GC_ROOTS` is set; the value is dropped again after each pass.
pub fn set_probe_root(v: Vec<(String, CfmlValue)>) {
    *PROBE_ROOT.lock() = v;
}

/// Number of nodes currently carried across requests (observability).
pub fn persistent_tracked() -> usize {
    PERSISTENT.lock().as_ref().map_or(0, |p| p.entries.len())
}

/// Carry this pass's still-live survivors into the cross-request set, then run a
/// sweep over that set if it has doubled since the last one. Returns the nodes
/// reclaimed by the sweep (0 if none ran).
fn carry_survivors(live: Vec<(usize, TrackedAlloc)>) -> usize {
    let base = persistent_base();
    if base == 0 {
        return 0;
    }
    let due = {
        let mut guard = PERSISTENT.lock();
        let set = guard.get_or_insert_with(|| PersistentSet {
            entries: Vec::new(),
            seen: HashSet::new(),
            next_sweep: base,
        });
        for (ptr, t) in live {
            if set.seen.insert(ptr) {
                set.entries.push(t);
            }
        }
        if persistent_always() || set.entries.len() >= set.next_sweep {
            // Take the whole set out under the lock; the pass itself runs
            // unlocked, and the surviving remainder is put back below.
            set.seen.clear();
            Some(std::mem::take(&mut set.entries))
        } else {
            None
        }
    };

    let Some(entries) = due else { return 0 };
    sweep_entries(entries, base)
}

/// Force a cross-request sweep now, ignoring the doubling budget. Returns the
/// nodes reclaimed. Intended for an idle server (nothing is arriving to trip the
/// budget) and for the collector's own tests.
pub fn sweep_persistent() -> usize {
    let base = persistent_base();
    if base == 0 {
        return 0;
    }
    let entries = {
        let mut guard = PERSISTENT.lock();
        match guard.as_mut() {
            Some(set) if !set.entries.is_empty() => {
                set.seen.clear();
                std::mem::take(&mut set.entries)
            }
            _ => return 0,
        }
    };
    sweep_entries(entries, base)
}

/// The sweep itself: collect over the carried set, then put the remainder back
/// and re-arm the budget. Runs UNLOCKED — see [`PersistentSet`] for why that is
/// safe against concurrent requests.
fn sweep_entries(entries: Vec<TrackedAlloc>, base: usize) -> usize {
    let mut still_live: Vec<(usize, TrackedAlloc)> = Vec::new();
    let reclaimed = collect_from_log_carrying(entries, Some(&mut still_live));
    let live_count = still_live.len();
    {
        let mut guard = PERSISTENT.lock();
        let set = guard.get_or_insert_with(|| PersistentSet {
            entries: Vec::new(),
            seen: HashSet::new(),
            next_sweep: base,
        });
        for (ptr, t) in still_live {
            if set.seen.insert(ptr) {
                set.entries.push(t);
            }
        }
        set.next_sweep = std::cmp::max(base, live_count.saturating_mul(2));
    }
    if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!(
            "[cycle_gc] cross-request sweep reclaimed {} node(s); {} still tracked, \
             next sweep at {}",
            reclaimed,
            live_count,
            std::cmp::max(base, live_count.saturating_mul(2))
        );
    }
    reclaimed
}

fn collect_from_log(log: Vec<TrackedAlloc>) -> usize {
    collect_from_log_carrying(log, None)
}

fn collect_from_log_carrying(
    log: Vec<TrackedAlloc>,
    carry: Option<&mut Vec<(usize, TrackedAlloc)>>,
) -> usize {
    if log.is_empty() {
        return 0;
    }

    // 1. Upgrade survivors; one strong probe handle per distinct backing.
    let mut nodes: HashMap<usize, NodeHandle> = HashMap::with_capacity(log.len());
    for t in log {
        match t {
            TrackedAlloc::Struct(w) => {
                if let Some(a) = w.upgrade() {
                    nodes
                        .entry(Arc::as_ptr(&a) as *const () as usize)
                        .or_insert(NodeHandle::Struct(a));
                }
            }
            TrackedAlloc::Array(w) => {
                if let Some(a) = w.upgrade() {
                    nodes
                        .entry(Arc::as_ptr(&a) as *const () as usize)
                        .or_insert(NodeHandle::Array(a));
                }
            }
            TrackedAlloc::Query(w) => {
                if let Some(a) = w.upgrade() {
                    nodes
                        .entry(Arc::as_ptr(&a) as *const () as usize)
                        .or_insert(NodeHandle::Query(a));
                }
            }
            TrackedAlloc::Scope(w) => {
                if let Some(a) = w.upgrade() {
                    nodes
                        .entry(Arc::as_ptr(&a) as *const () as usize)
                        .or_insert(NodeHandle::Scope(a));
                }
            }
            #[cfg(feature = "component-instance")]
            TrackedAlloc::Instance(w) => {
                if let Some(a) = w.upgrade() {
                    nodes
                        .entry(Arc::as_ptr(&a) as *const () as usize)
                        .or_insert(NodeHandle::Instance(a));
                }
            }
        }
    }
    if nodes.is_empty() {
        return 0;
    }

    let in_set: HashSet<usize> = nodes.keys().copied().collect();

    // 1b. The class blueprints held by the surviving instances, DE-DUPLICATED by
    //     Arc identity. A blueprint is a CARRIER, not a node: it participates in
    //     the counts and the mark so that the values it holds are not mistaken
    //     for externally-owned roots, but it is never cleared — breaking the
    //     instances that hold it drops it by refcounting. De-duplication is
    //     load-bearing: walking one blueprint once per instance would count each
    //     of its edges N times, deflate its children's external count and
    //     OVER-COLLECT live data (the failure mode that dropped a live
    //     `EventHandlerBean`'s `viewDispatch` when the Instance data maps were
    //     double-walked).
    #[cfg(feature = "component-instance")]
    let blueprints: HashMap<usize, std::sync::Arc<crate::component::ClassBlueprint>> = {
        let mut bps = HashMap::new();
        for h in nodes.values() {
            if let NodeHandle::Instance(a) = h {
                if let Some(g) = a.try_read() {
                    let bp = g.class.clone();
                    bps.entry(Arc::as_ptr(&bp) as *const () as usize)
                        .or_insert(bp);
                }
            }
        }
        bps
    };

    // 1c. The shared per-class METHOD TABLES hung off component scope structs.
    //     Like a blueprint this is an `Arc<ValueMap>` carrier, not a node: its
    //     entries are `CfmlFunction`s whose captured scopes reach back into the
    //     instance graph. Walking it from each holder would count every edge once
    //     per instance of the class (the double-count that over-collects), and
    //     NOT walking it leaves those edges uncounted — which reads as external
    //     ownership and pins the graph. De-duplicated by Arc identity, it is
    //     counted exactly once, like `blueprints` above.
    let method_tables: HashMap<usize, Arc<ValueMap>> = {
        let mut t = HashMap::new();
        for h in nodes.values() {
            if let NodeHandle::Struct(a) = h {
                if let Some(g) = a.try_read() {
                    if let Some(mt) = g.method_table.as_ref() {
                        t.entry(Arc::as_ptr(mt) as *const () as usize)
                            .or_insert_with(|| Arc::clone(mt));
                    }
                }
            }
        }
        t
    };

    // 2. internal_in[n] = number of references to n from other survivors, plus
    //    the references held by the carrier blueprints (counted once each).
    let mut internal_in: HashMap<usize, usize> = HashMap::with_capacity(nodes.len());
    for h in nodes.values() {
        h.for_each_child_node(&in_set, &mut |child| {
            *internal_in.entry(child).or_insert(0) += 1;
        });
    }
    #[cfg(feature = "component-instance")]
    for bp in blueprints.values() {
        blueprint_values(bp, |v| {
            classify(v, &in_set, &mut |child| {
                *internal_in.entry(child).or_insert(0) += 1;
            })
        });
    }
    for mt in method_tables.values() {
        for v in mt.values() {
            classify(v, &in_set, &mut |child| {
                *internal_in.entry(child).or_insert(0) += 1;
            });
        }
    }

    // 2b. A blueprint's own ownership: held by each surviving instance of its
    //     class (internal) and by anything else — a live request's blueprint
    //     cache, an instance outside this set (external). `-1` is our own clone.
    #[cfg(feature = "component-instance")]
    let mut bp_internal: HashMap<usize, usize> = HashMap::with_capacity(blueprints.len());
    #[cfg(feature = "component-instance")]
    for h in nodes.values() {
        if let NodeHandle::Instance(a) = h {
            if let Some(g) = a.try_read() {
                let p = Arc::as_ptr(&g.class) as *const () as usize;
                *bp_internal.entry(p).or_insert(0) += 1;
            }
        }
    }

    // 3. Roots = survivors with an owner OUTSIDE the survivor set.
    //    external(n) = strong_count − 1 (probe handle) − internal_in(n).
    let mut live: HashSet<usize> = HashSet::with_capacity(nodes.len());
    let mut worklist: Vec<usize> = Vec::new();
    // RUSTCFML_GC_ROOTS=N reports the N largest PINNED ROOTS — survivors whose
    // external count is non-zero, i.e. the nodes something outside this
    // collection set still points at. Their transitive closure is what gets
    // marked live, so when a sweep keeps far more than expected, these names
    // are the answer to "held by what?". Structs report their first keys, which
    // identifies them in CFML terms rather than as addresses.
    static ROOT_DEBUG: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let root_debug = *ROOT_DEBUG.get_or_init(|| {
        std::env::var("RUSTCFML_GC_ROOTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    });
    let mut root_report: Vec<(usize, String)> = Vec::new();
    let mut deferred_roots: Vec<usize> = Vec::new();
    // Index built once (not per root) so the diagnostic stays linear: a pass with
    // 250k survivors can have tens of thousands of roots.
    #[cfg(feature = "component-instance")]
    let data_map_owners: HashMap<usize, String> = if root_debug > 0 {
        let mut m = HashMap::new();
        for oh in nodes.values() {
            if let NodeHandle::Instance(a) = oh {
                if let Some(g) = a.try_read() {
                    m.insert(
                        g.public_map_handle().backing_ptr(),
                        format!("this-map of {}", g.class.name),
                    );
                    m.insert(
                        g.private_map_handle().backing_ptr(),
                        format!("variables-map of {}", g.class.name),
                    );
                }
            }
        }
        m
    } else {
        HashMap::new()
    };
    for (&p, h) in &nodes {
        let internal = *internal_in.get(&p).unwrap_or(&0);
        let external = h.strong_count().saturating_sub(1).saturating_sub(internal);
        if root_debug > 0 && external > 0 {
            // Is this pinned root one of the surviving instances' OWN data maps?
            // Those are deliberately untracked and owned by the Instance Arc, so
            // if a tracked struct turns out to BE one, the Instance's reference
            // to it is an uncounted external ref — which pins it and everything
            // it reaches.
            #[allow(unused_mut)]
            let mut owner = String::new();
            #[cfg(feature = "component-instance")]
            if let Some(what) = data_map_owners.get(&p) {
                owner = format!(" [== {}]", what);
            }
            root_report.push((
                external,
                format!(
                    "[strong={} internal={}] {}{}",
                    h.strong_count(),
                    internal,
                    h.describe(),
                    owner
                ),
            ));
        }
        if external > 0 {
            if root_debug > 0 {
                // Attribution mode seeds one root at a time (below) so each can
                // be charged with what it alone keeps alive.
                deferred_roots.push(p);
            } else if live.insert(p) {
                worklist.push(p);
            }
        }
    }

    // 3b. A blueprint owned from outside this set — by a concurrent request's
    //     blueprint cache, or by an instance that is not a survivor here — keeps
    //     everything it holds alive. One whose only owners ARE survivors here is
    //     carried by the mark instead: it goes live exactly when one of its
    //     instances does (step 4).
    #[cfg(feature = "component-instance")]
    let mut live_bps: HashSet<usize> = HashSet::new();
    let mut live_tables: HashSet<usize> = HashSet::new();
    #[cfg(feature = "component-instance")]
    for (&p, bp) in &blueprints {
        let internal = *bp_internal.get(&p).unwrap_or(&0);
        let external = Arc::strong_count(bp)
            .saturating_sub(1)
            .saturating_sub(internal);
        if external > 0 && live_bps.insert(p) {
            blueprint_values(bp, |v| {
                classify(v, &in_set, &mut |child| {
                    if live.insert(child) {
                        worklist.push(child);
                    }
                })
            });
        }
    }

    // 3c. RETENTION ATTRIBUTION (diagnostics only; identical final live set).
    //     Seeding every root at once answers "what is pinned", which is the wrong
    //     question when thousands of roots are individually harmless: a function's
    //     process-lifetime `params_marker` array is a root forever and retains
    //     nothing but its own strings. The question that ends a leak hunt is which
    //     root is RESPONSIBLE for the bulk of the retained graph. Seeding one root
    //     at a time and charging it with the nodes its own walk newly marks answers
    //     exactly that, for the cost of the mark phase we were doing anyway (each
    //     node is still marked at most once). Shared nodes are charged to whichever
    //     root reaches them first, which is fine: one dominant retainer still
    //     dominates.
    let mut retention: Vec<(usize, usize)> = Vec::new(); // (nodes retained, root ptr)
    if root_debug > 0 {
        for &r in &deferred_roots {
            if !live.insert(r) {
                continue; // already reached from an earlier root
            }
            let before = live.len();
            worklist.push(r);
            while let Some(p) = worklist.pop() {
                if let Some(h) = nodes.get(&p) {
                    h.for_each_child_node(&in_set, &mut |child| {
                        if live.insert(child) {
                            worklist.push(child);
                        }
                    });
                    #[cfg(feature = "component-instance")]
                    if let NodeHandle::Instance(a) = h {
                        let bp = a.try_read().map(|g| g.class.clone());
                        if let Some(bp) = bp {
                            let bpp = Arc::as_ptr(&bp) as *const () as usize;
                            if live_bps.insert(bpp) {
                                blueprint_values(&bp, |v| {
                                    classify(v, &in_set, &mut |child| {
                                        if live.insert(child) {
                                            worklist.push(child);
                                        }
                                    })
                                });
                            }
                        }
                    }
                }
            }
            retention.push((live.len() - before + 1, r));
        }
    }

    // 4. Mark the transitive closure of the roots live (a node reachable from a
    //    live root is live even if its own external count is 0). A live Instance
    //    also makes its blueprint live — it holds it — so everything that
    //    blueprint carries is live with it.
    while let Some(p) = worklist.pop() {
        if let Some(h) = nodes.get(&p) {
            h.for_each_child_node(&in_set, &mut |child| {
                if live.insert(child) {
                    worklist.push(child);
                }
            });
            #[cfg(feature = "component-instance")]
            if let NodeHandle::Instance(a) = h {
                let bp = a.try_read().map(|g| g.class.clone());
                if let Some(bp) = bp {
                    let bpp = Arc::as_ptr(&bp) as *const () as usize;
                    if live_bps.insert(bpp) {
                        blueprint_values(&bp, |v| {
                            classify(v, &in_set, &mut |child| {
                                if live.insert(child) {
                                    worklist.push(child);
                                }
                            })
                        });
                    }
                }
            }
            // A live component scope makes its shared method table live too.
            if let NodeHandle::Struct(a) = h {
                let mt = a.try_read().and_then(|g| g.method_table.clone());
                if let Some(mt) = mt {
                    if live_tables.insert(Arc::as_ptr(&mt) as *const () as usize) {
                        for v in mt.values() {
                            classify(v, &in_set, &mut |child| {
                                if live.insert(child) {
                                    worklist.push(child);
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    // 5. Everything not live is an unreachable cycle: clear it to break the
    //    cycle, then dropping the probe handles frees the whole subgraph.
    if root_debug > 0 && !root_report.is_empty() {
        // Ranked by COUNT, not by size. A leak shows up as tens of thousands of
        // roots of ONE shape — sampling the twenty largest just re-lists the
        // application's legitimately-live singletons every time, which is what
        // made the first two rounds of this hunt so slow. The shape whose count
        // grows by a generation per reload is the leak.
        let mut by_shape: HashMap<String, (usize, usize)> = HashMap::new();
        for (ext, what) in &root_report {
            // Strip the per-node arithmetic prefix so identical shapes group.
            let shape = what
                .split_once("] ")
                .map(|(_, rest)| rest)
                .unwrap_or(what.as_str());
            let e = by_shape.entry(shape.to_string()).or_insert((0, 0));
            e.0 += 1;
            e.1 += ext;
        }
        let mut shapes: Vec<(String, (usize, usize))> = by_shape.into_iter().collect();
        shapes.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        shapes.truncate(root_debug);
        eprintln!(
            "[cycle_gc] {} pinned roots, by shape (count, total ext):",
            root_report.len()
        );
        for (shape, (n, ext)) in &shapes {
            eprintln!("    n={:<7} ext={:<7} {}", n, ext, shape);
        }
        // Engine-or-application: how much of the surviving graph hangs off the
        // application scope?
        {
            let probes = PROBE_ROOT.lock();
            if !probes.is_empty() {
                // Cumulative: each named carrier is charged only with what no
                // EARLIER carrier already reached, so the numbers sum to the
                // covered total and one dominant holder stands out.
                let mut seen: HashSet<usize> = HashSet::new();
                for (name, pr) in probes.iter() {
                    let before = seen.len();
                    let mut stack: Vec<usize> = Vec::new();
                    classify(pr, &in_set, &mut |c| {
                        if seen.insert(c) {
                            stack.push(c);
                        }
                    });
                    while let Some(p) = stack.pop() {
                        if let Some(h) = nodes.get(&p) {
                            h.for_each_child_node(&in_set, &mut |c| {
                                if seen.insert(c) {
                                    stack.push(c);
                                }
                            });
                            // Follow the same blueprint edge the MARK phase does,
                            // or the probe under-reports what a scope really keeps
                            // alive and blames the engine for the application.
                            #[cfg(feature = "component-instance")]
                            if let NodeHandle::Instance(a) = h {
                                let bp = a.try_read().map(|g| g.class.clone());
                                if let Some(bp) = bp {
                                    blueprint_values(&bp, |v| {
                                        classify(v, &in_set, &mut |c| {
                                            if seen.insert(c) {
                                                stack.push(c);
                                            }
                                        })
                                    });
                                }
                            }
                        }
                    }
                    eprintln!(
                        "[cycle_gc]   reachable from {:<28} {}",
                        name,
                        seen.len() - before
                    );
                }
                eprintln!(
                    "[cycle_gc] of {} live nodes, {} reached by the probes, {} NOT \
                     (held by something else)",
                    live.len(),
                    seen.len(),
                    live.len().saturating_sub(seen.len())
                );
            }
        }
        // The ranking that actually names a leak: who RETAINS the most.
        retention.sort_by(|a, b| b.0.cmp(&a.0));
        eprintln!(
            "[cycle_gc] top retainers (nodes kept alive, of {} live):",
            live.len()
        );
        // WHO HOLDS IT. A pinned root reports `external > 0`; this names the
        // survivors that actually reference it, which is the question every round
        // of a leak hunt ends on. Reverse edges are built ONLY for the top
        // retainers (one extra pass over the graph, under the flag), because a
        // full reverse index of a 450k-node graph is not worth building to answer
        // a question about ten nodes.
        //
        // A root whose holders are listed here is held by TRACKED data; a root
        // that reports none is held by something outside the collector's world —
        // Rust-side state, or an allocation deliberately excluded from the log
        // (run again with `RUSTCFML_GC_TRACK_ALL=1` to make those visible too).
        let targets: HashSet<usize> = retention
            .iter()
            .take(root_debug)
            .map(|(_, r)| *r)
            .collect();
        let mut holders: HashMap<usize, Vec<String>> = HashMap::new();
        if !targets.is_empty() {
            for (&p, h) in &nodes {
                h.for_each_child_node(&in_set, &mut |child| {
                    if targets.contains(&child) {
                        holders.entry(child).or_default().push(
                            nodes.get(&p).map(|n| n.describe()).unwrap_or_default(),
                        );
                    }
                });
            }
            #[cfg(feature = "component-instance")]
            for bp in blueprints.values() {
                let name = format!("ClassBlueprint of {}", bp.name);
                blueprint_values(bp, |v| {
                    classify(v, &in_set, &mut |child| {
                        if targets.contains(&child) {
                            holders.entry(child).or_default().push(name.clone());
                        }
                    })
                });
            }
            for mt in method_tables.values() {
                for v in mt.values() {
                    classify(v, &in_set, &mut |child| {
                        if targets.contains(&child) {
                            holders
                                .entry(child)
                                .or_default()
                                .push("shared method table".to_string());
                        }
                    });
                }
            }
        }
        for (n, r) in retention.iter().take(root_debug) {
            let what = nodes.get(r).map(|h| h.describe()).unwrap_or_default();
            let strong = nodes.get(r).map(|h| h.strong_count()).unwrap_or(0);
            let internal = *internal_in.get(r).unwrap_or(&0);
            eprintln!(
                "    retains={:<8} ext={:<4} strong={:<4} internal={:<4} {}",
                n,
                strong.saturating_sub(1).saturating_sub(internal),
                strong,
                internal,
                what
            );
            match holders.get(r) {
                Some(hs) => {
                    let mut counts: HashMap<&str, usize> = HashMap::new();
                    for h in hs {
                        *counts.entry(h.as_str()).or_insert(0) += 1;
                    }
                    let mut v: Vec<(&&str, &usize)> = counts.iter().collect();
                    v.sort_by(|a, b| b.1.cmp(a.1));
                    for (desc, c) in v.into_iter().take(4) {
                        eprintln!("        held by x{:<4} {}", c, desc);
                    }
                }
                None => eprintln!(
                    "        held by      <nothing tracked — Rust-side state, or an \
                     untracked allocation; retry with RUSTCFML_GC_TRACK_ALL=1>"
                ),
            }
        }
    }
    let survivors = nodes.len();
    #[cfg(feature = "component-instance")]
    if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        let insts = nodes
            .values()
            .filter(|h| matches!(h, NodeHandle::Instance(_)))
            .count();
        eprintln!(
            "[cycle_gc]   survivors by kind: instances={} blueprints={} of {} nodes",
            insts,
            blueprints.len(),
            nodes.len()
        );
    }
    let mut collected = 0usize;
    for (&p, h) in &nodes {
        if !live.contains(&p) {
            h.clear();
            collected += 1;
        }
    }
    // Hand the LIVE survivors back to the caller so they can stay tracked. Only
    // live ones: a node cleared above has no external owner, so dropping the
    // probe handles below frees it and a `Weak` to it would never upgrade again.
    if let Some(carry) = carry {
        carry.reserve(live.len());
        for (&p, h) in &nodes {
            if live.contains(&p) {
                carry.push((p, h.downgrade()));
            }
        }
    }
    drop(nodes);

    if collected > 0 {
        COLLECTED_TOTAL.fetch_add(collected, Ordering::Relaxed);
    }
    if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!(
            "[cycle_gc] survivors={} live={} collected={}",
            survivors,
            live.len(),
            collected
        );
    }
    collected
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use crate::dynamic::{CfmlStruct, CfmlValue, ValueMap};

    /// Build a 2-node reference cycle of tracked structs and return it. Dropping
    /// the returned handles leaves the cycle unreachable but NOT freed by
    /// refcounting — exactly the shape a CFC instance has.
    fn make_cycle() -> (CfmlStruct, CfmlStruct) {
        let a = CfmlStruct::new(ValueMap::default());
        let b = CfmlStruct::new(ValueMap::default());
        a.insert("b".to_string(), CfmlValue::Struct(b.clone()));
        b.insert("a".to_string(), CfmlValue::Struct(a.clone()));
        (a, b)
    }

    /// A mid-request sweep must reclaim unreachable cycles WITHOUT waiting for
    /// request end — the whole point of `collect_incremental`.
    #[test]
    fn incremental_sweep_reclaims_unreachable_cycles() {
        arm();
        enable();
        // Base threshold of 1 so the very first check sweeps.
        NEXT_SWEEP.with(|c| c.set(1));
        for _ in 0..50 {
            let (a, b) = make_cycle();
            drop(a);
            drop(b);
        }
        let reclaimed = collect_incremental();
        assert!(
            reclaimed > 0,
            "an incremental sweep must reclaim unreachable cycles mid-request, got {reclaimed}"
        );
        disable_and_clear();
    }

    /// The safety property: a sweep must NEVER collect something still reachable,
    /// and the survivor must stay VISIBLE to a later sweep (it can become garbage
    /// later in the same request). Without re-registration the second sweep below
    /// would find nothing and the object would leak for the rest of the request.
    #[test]
    fn incremental_sweep_keeps_live_and_re_registers_it() {
        arm();
        enable();
        let (live_a, live_b) = make_cycle();
        NEXT_SWEEP.with(|c| c.set(1));
        collect_incremental();
        // Still fully intact and readable after the sweep.
        assert!(
            matches!(live_a.get("b"), Some(CfmlValue::Struct(_))),
            "a reachable cycle must survive an incremental sweep"
        );
        // Now drop the only external handles; a later sweep must still see it.
        drop(live_a);
        drop(live_b);
        NEXT_SWEEP.with(|c| c.set(1));
        let reclaimed = collect_incremental();
        assert!(
            reclaimed > 0,
            "a survivor must be re-registered so a LATER sweep can still reclaim it"
        );
        disable_and_clear();
    }

    /// The budget must back off with the live set. A fixed threshold rescans
    /// every survivor on every pass, which is quadratic: a request holding 60k
    /// live components went from 3.4 s to over ten minutes at a flat threshold.
    #[test]
    fn budget_backs_off_with_the_live_set() {
        arm();
        enable();
        let mut live = Vec::new();
        for _ in 0..40 {
            live.push(make_cycle());
        }
        NEXT_SWEEP.with(|c| c.set(1));
        collect_incremental();
        let after = NEXT_SWEEP.with(|c| c.get());
        assert!(
            after >= 80,
            "budget must rise to ~2x the live set (>=80 for 40 live cycles), got {after}"
        );
        drop(live);
        disable_and_clear();
    }
}
