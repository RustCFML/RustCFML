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
enum TrackedAlloc {
    Struct(Weak<PlRwLock<StructInner>>),
    Array(Weak<PlRwLock<Vec<CfmlValue>>>),
    Query(Weak<PlRwLock<CfmlQueryData>>),
    Scope(Weak<RwLock<ValueMap>>),
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
}

impl NodeHandle {
    #[inline]
    fn strong_count(&self) -> usize {
        match self {
            NodeHandle::Struct(a) => Arc::strong_count(a),
            NodeHandle::Array(a) => Arc::strong_count(a),
            NodeHandle::Query(a) => Arc::strong_count(a),
            NodeHandle::Scope(a) => Arc::strong_count(a),
        }
    }

    /// Enumerate the immediate child *nodes* (members of `in_set`) without
    /// cloning any `Arc` (so refcounts are undisturbed) — terminal at node
    /// types, descending through non-node carriers (Function/Component/Closure/
    /// QueryColumn). Holds a read guard for the duration; the callback only
    /// records ids and never locks another node, so this cannot deadlock.
    fn for_each_child_node(&self, in_set: &HashSet<usize>, emit: &mut impl FnMut(usize)) {
        match self {
            NodeHandle::Struct(a) => {
                let g = a.read();
                for v in g.map.values() {
                    classify(v, in_set, emit);
                }
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
        // A flyweight component `Instance` (`Arc<RwLock<Instance>>`) is a NON-tracked
        // owner — treat it as OPAQUE / an external root, exactly like `NativeObject`.
        // Its `this_members`/`variables_members` are owned by the Instance's `Arc`, so
        // their `strong_count` already carries an EXTERNAL reference (from the
        // non-tracked Instance) and they are protected by refcounting. Do NOT emit
        // internal edges to them here: this collector computes
        // `external(n) = strong_count − 1 − internal_in(n)` and collects nodes with
        // `external == 0`, so emitting edges to the data maps would INFLATE their
        // `internal_in`, drop `external` to 0, and OVER-COLLECT live component data.
        // (An earlier "fix" that emitted those ptrs did exactly this: Preside's
        // cached `EventHandlerBean` lost `variables.viewDispatch` on a warm request,
        // 500ing the admin login page. Verified by bisection 2026-07-22.)
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(_) => {}
        _ => {}
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
    collect_from_log(log)
}

/// The collection pass over an explicit allocation log (the live request's,
/// drained by `collect`, or a previously-deferred one). Identical algorithm
/// either way; factored out so deferred logs can be collected after their
/// spawning request's threads finish. Safe to run concurrently with unrelated
/// requests: the cycles it touches are internal to one finished request and
/// unreachable from anywhere else, so their `strong_count`s are stable.
fn collect_from_log(log: Vec<TrackedAlloc>) -> usize {
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
        }
    }
    if nodes.is_empty() {
        return 0;
    }

    let in_set: HashSet<usize> = nodes.keys().copied().collect();

    // 2. internal_in[n] = number of references to n from other survivors.
    let mut internal_in: HashMap<usize, usize> = HashMap::with_capacity(nodes.len());
    for h in nodes.values() {
        h.for_each_child_node(&in_set, &mut |child| {
            *internal_in.entry(child).or_insert(0) += 1;
        });
    }

    // 3. Roots = survivors with an owner OUTSIDE the survivor set.
    //    external(n) = strong_count − 1 (probe handle) − internal_in(n).
    let mut live: HashSet<usize> = HashSet::with_capacity(nodes.len());
    let mut worklist: Vec<usize> = Vec::new();
    for (&p, h) in &nodes {
        let internal = *internal_in.get(&p).unwrap_or(&0);
        let external = h.strong_count().saturating_sub(1).saturating_sub(internal);
        if external > 0 && live.insert(p) {
            worklist.push(p);
        }
    }

    // 4. Mark the transitive closure of the roots live (a node reachable from a
    //    live root is live even if its own external count is 0).
    while let Some(p) = worklist.pop() {
        if let Some(h) = nodes.get(&p) {
            h.for_each_child_node(&in_set, &mut |child| {
                if live.insert(child) {
                    worklist.push(child);
                }
            });
        }
    }

    // 5. Everything not live is an unreachable cycle: clear it to break the
    //    cycle, then dropping the probe handles frees the whole subgraph.
    let survivors = nodes.len();
    let mut collected = 0usize;
    for (&p, h) in &nodes {
        if !live.contains(&p) {
            h.clear();
            collected += 1;
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
