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
                if let Some((this_m, vars_m)) = maps {
                    this_m.with_read(|m| {
                        for v in m.values() {
                            classify(v, in_set, emit);
                        }
                    });
                    vars_m.with_read(|m| {
                        for v in m.values() {
                            classify(v, in_set, emit);
                        }
                    });
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
    for (&p, h) in &nodes {
        let internal = *internal_in.get(&p).unwrap_or(&0);
        let external = h.strong_count().saturating_sub(1).saturating_sub(internal);
        if root_debug > 0 && external > 0 {
            root_report.push((external, h.describe()));
        }
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
    if root_debug > 0 && !root_report.is_empty() {
        root_report.sort_by(|a, b| b.0.cmp(&a.0));
        root_report.truncate(root_debug);
        eprintln!("[cycle_gc] pinned roots (external refs, descending):");
        for (ext, what) in &root_report {
            eprintln!("    ext={:<6} {}", ext, what);
        }
    }
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
