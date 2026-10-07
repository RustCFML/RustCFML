//! Cycle collector.
//!
//! RustCFML's reference-typed containers (`CfmlStruct`, `CfmlArray`, `CfmlQuery`),
//! closure capture scopes (`Arc<RwLock<ValueMap>>`) and flyweight component
//! instances are `Arc`-refcounted, so a reference *cycle* (`a.other = b;
//! b.other = a`, a closure stored into the scope it captures, every component's
//! `this → variables → this`) is never reclaimed by refcounting alone — its
//! internal refs keep `strong_count > 0` even after every external root is gone.
//! In a long-lived `--serve` process that builds cyclic graphs (Preside, ColdBox,
//! WireBox), this leaks on every request and RSS climbs without bound.
//!
//! This module reclaims those cycles with **trial deletion** (Bacon–Rajan) over
//! a log of the containers a request allocated. It does NOT replace refcounting:
//! the acyclic garbage is still freed eagerly, on-thread, with zero pause. It
//! never walks the whole heap and never stops other requests. User-facing
//! description: `docs/memory.md`.
//!
//! ## When it runs
//! * **Mid-request** — `collect_incremental`, from component construction, once
//!   the young log reaches its budget: a MINOR sweep over the young entries
//!   promotes survivors to an OLD generation; a MAJOR sweep covers both once the
//!   old generation has doubled.
//! * **Request end / `cfthread` body end** — `collect` over everything logged,
//!   deferred (`defer_current_log`) while a thread the request started is still
//!   running, because the two share scopes.
//! * **Across requests** — `collect` carries live survivors into a
//!   process-wide, `Weak`-held set, swept once it has doubled. This frees what
//!   became garbage AFTER the request that made it.
//! * **On displacement** — overwriting or deleting a key in a `persistent_scope`
//!   struct (the application scope, static scopes, class structs) re-enters the
//!   displaced graph into the log (`relog_cycle_nodes`); a large one schedules a
//!   cross-request sweep (`sweep_if_displaced`). This is what frees a framework
//!   reload's old generation.
//!
//! ## How it stays correct without tracing the persistent scopes
//! The `Arc::strong_count` itself is the oracle. A survivor that is still
//! referenced from outside the set being swept (application/session/server
//! scope, a live frame, a seed) has a strong count greater than the number of
//! references it gets from inside the set; a pure cycle does not. So we compute,
//! per node `n`:
//!
//! ```text
//! external(n) = strong_count(n) − 1 (our own probe handle) − internal_in(n)
//! ```
//!
//! `external(n) > 0` ⟺ `n` has an owner outside the set ⟹ `n` is a live root.
//! We mark the transitive closure of the roots live, and everything else is an
//! unreachable cycle: we clear its backing (dropping its outgoing refs) so the
//! whole subgraph's counts fall to zero and it frees. A node whose lock can't be
//! taken without waiting has its edges skipped, which can only under-collect.
//!
//! ## Safety w.r.t. threads
//! Reading `strong_count` is only stable if no other thread is concurrently
//! cloning/dropping the same `Arc`. A truly-internal cycle (the only thing we
//! ever collect) is unreachable from any other request's thread by construction;
//! anything shared across threads escaped to a shared scope and thus reads as a
//! live root. The one case to guard is *this* request's own `cfthread`s, which
//! share `application`/`request` scope by Arc — so a request or thread body that
//! still has a running thread defers its log instead of collecting it.

use crate::dynamic::{
    CfmlClosureBody, CfmlFunction, CfmlQueryData, CfmlStatement, CfmlValue, StructInner, ValueMap,
};
#[cfg(feature = "component-instance")]
use crate::component::Instance;
use parking_lot::RwLock as PlRwLock;
use std::cell::RefCell;
use std::collections::HashMap;

/// Pointer-keyed set/map used throughout the collector. The keys are `Arc`
/// addresses (already well distributed), so `FxHash` beats SipHash here and the
/// collector is hash-bound: several lookups per node and per edge in a pass.
type PtrSet = rustc_hash::FxHashSet<usize>;
type PtrMap<V> = rustc_hash::FxHashMap<usize, V>;
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

/// Nanoseconds spent collecting, process-wide, across every sweep (request-end,
/// incremental, deferred). The engine's counterpart to a JVM collector's
/// `CollectionTime`, which is how the metrics endpoint reports it.
static COLLECTION_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Cumulative time spent collecting, in milliseconds.
pub fn collection_time_ms() -> u64 {
    COLLECTION_NANOS.load(Ordering::Relaxed) / 1_000_000
}

/// Sweeps run, process-wide.
static SWEEPS_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Bytes freed by sweeps, process-wide, and by the most recent one. Measured by
/// the allocation accounting (`mem_account`), so zero while it is off.
static FREED_BYTES_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FREED_BYTES_LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Number of sweeps run so far.
pub fn sweeps_total() -> u64 {
    SWEEPS_TOTAL.load(Ordering::Relaxed)
}

/// Cycle nodes reclaimed so far.
pub fn reclaimed_nodes_total() -> u64 {
    COLLECTED_TOTAL.load(Ordering::Relaxed) as u64
}

/// Bytes freed by sweeps so far, and by the latest sweep, when allocation
/// accounting is on.
pub fn freed_bytes() -> Option<(u64, u64)> {
    crate::mem_account::is_enabled().then(|| {
        (
            FREED_BYTES_TOTAL.load(Ordering::Relaxed),
            FREED_BYTES_LAST.load(Ordering::Relaxed),
        )
    })
}

/// Adds its lifetime to [`COLLECTION_NANOS`], and the bytes the pass freed on
/// this thread to the freed-bytes totals, on every exit path.
struct CollectionTimer(std::time::Instant, crate::mem_account::ThreadTotals);

impl CollectionTimer {
    fn start() -> Self {
        Self(std::time::Instant::now(), crate::mem_account::thread_totals())
    }
}

impl Drop for CollectionTimer {
    fn drop(&mut self) {
        COLLECTION_NANOS.fetch_add(self.0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        SWEEPS_TOTAL.fetch_add(1, Ordering::Relaxed);
        // NET of what the pass allocated itself: its node map, membership set
        // and counts (~160 B per entry walked) are freed before it returns, and
        // counting those frees reported a boot sweep that reclaimed 6.5k nodes
        // as "120 MB freed".
        let freed = if crate::mem_account::is_enabled() {
            let now = crate::mem_account::thread_totals();
            let freed = now.freed.saturating_sub(self.1.freed);
            let scratch = now.allocated.saturating_sub(self.1.allocated);
            let freed = freed.saturating_sub(scratch);
            FREED_BYTES_TOTAL.fetch_add(freed, Ordering::Relaxed);
            FREED_BYTES_LAST.store(freed, Ordering::Relaxed);
            freed
        } else {
            0
        };
        let _ = REQUEST_GC.try_with(|g| {
            let (n, b) = g.get();
            g.set((n + 1, b + freed));
        });
    }
}

/// Sweeps run on this thread since the request began (see `reset_request_census`
/// callers). Lets the VM tell whether a `collect_incremental()` call swept.
pub fn request_sweeps() -> u64 {
    REQUEST_GC.with(|g| g.get().0)
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
    /// Per-request allocation log. `Some` only while a request body or a
    /// `cfthread` body is executing on this thread (`enable` is called for both);
    /// `None` everywhere else (CLI, between requests). Taking the log out
    /// (`collect`) also leaves it `None`, so the collector's own allocations are
    /// never logged.
    static ALLOC_LOG: RefCell<Option<Vec<TrackedAlloc>>> = const { RefCell::new(None) };
    /// Monotonic count of tracked containers this REQUEST has allocated, unlike
    /// the log itself (which sweeps drain) and unaffected by the log cap. Read by
    /// [`crate::mem_guard`] to decide which in-flight request built the heap when
    /// `--max-memory`'s hard tier has to choose one to abort. A plain `Cell`
    /// bump; the guard publishes it to an atomic only at safe points.
    static ALLOC_TOTAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The OLD generation: survivors promoted by a minor sweep. Only re-walked by
    /// a MAJOR sweep (when it has doubled since the last one) or at request end.
    /// See `collect_incremental`.
    static OLD_LOG: RefCell<Vec<TrackedAlloc>> = const { RefCell::new(Vec::new()) };
    /// Backing pointers currently in `OLD_LOG`, so a node re-entered into the
    /// young log by the relog hook and re-promoted is not pushed twice.
    static OLD_SET: RefCell<PtrSet> = RefCell::new(PtrSet::default());
    /// Old-generation size at which the next sweep is a major one.
    static NEXT_MAJOR: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
    /// A drained log's buffer, kept for the next log on this thread. Every
    /// request began with `Some(Vec::new())` and every minor sweep left a
    /// zero-capacity `Vec` behind, so the log regrew by doubling through its
    /// ~8k pushes on every Preside render — the largest steady-state source of
    /// fresh mimalloc pages in the warm profile (~2.7%), for a buffer whose
    /// size is the same request after request. Bounded by `SPARE_CAP`.
    static SPARE_LOG: RefCell<Vec<TrackedAlloc>> = const { RefCell::new(Vec::new()) };
}

/// Largest buffer `SPARE_LOG` keeps between requests (entries, 16 bytes each),
/// and the largest capacity the old-generation log and the pointer sets keep
/// across `enable()`. A warm Preside render logs ~8k entries, so 64k covers the
/// steady state with room to spare. It was 1M (16 MB): the BOOT request filled
/// it, and every worker thread that ever served a boot-sized request then held
/// its 16 MB spare (plus the old log's and sets' boot-sized capacity, which
/// `clear()` keeps) for the life of the process — ~13 MB of a booted Preside
/// heap. A larger request simply regrows its buffers.
const SPARE_CAP: usize = 1 << 16;

/// Drop a thread-local buffer whose capacity grew past `SPARE_CAP` (a big
/// request) so the next request starts small; otherwise just clear it.
fn trim_vec<T>(v: &mut Vec<T>) {
    if v.capacity() > SPARE_CAP {
        *v = Vec::new();
    } else {
        v.clear();
    }
}

fn trim_set(set: &mut PtrSet) {
    if set.capacity() > SPARE_CAP {
        *set = PtrSet::default();
    } else {
        set.clear();
    }
}

/// A cleared buffer for a new log: the thread's spare if it has one, else empty.
#[inline]
fn fresh_log() -> Vec<TrackedAlloc> {
    SPARE_LOG.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// Return a drained log's buffer for reuse (the larger of it and the current spare).
#[inline]
fn recycle_log(mut v: Vec<TrackedAlloc>) {
    v.clear();
    if v.capacity() == 0 || v.capacity() > SPARE_CAP {
        return;
    }
    SPARE_LOG.with(|s| {
        let mut s = s.borrow_mut();
        if s.capacity() < v.capacity() {
            *s = v;
        }
    });
}

/// Drain BOTH generations of this thread's log into one vector (young first),
/// leaving the thread not logging. `None` when the thread was not logging.
fn take_full_log() -> Option<Vec<TrackedAlloc>> {
    let young = ALLOC_LOG.with(|c| c.borrow_mut().take())?;
    let mut old = OLD_LOG.with(|c| std::mem::take(&mut *c.borrow_mut()));
    OLD_SET.with(|c| trim_set(&mut c.borrow_mut()));
    if old.is_empty() {
        return Some(young);
    }
    old.reserve(young.len());
    old.extend(young);
    Some(old)
}

/// Soft cap on the per-request allocation log, as a pure MEMORY safety valve —
/// NOT a functional gate. A real framework request (Preside, ColdBox, Wheels)
/// routinely allocates well over a million containers, so the old 1M cap caused
/// every such request to "overflow and skip collection", which is exactly the
/// runaway serve-mode leak this collector exists to prevent. The cap is now set
/// far above real request sizes, and — critically — overflowing it no longer
/// abandons collection: the log is first COMPACTED (dead and duplicate entries
/// dropped, see `log_push`), and only if it is still nearly full does logging
/// stop (bounding the log's own memory to ~`LOG_CAP * sizeof(Weak)` ≈ 16 bytes
/// each) while `collect()` still reclaims every cycle among the allocations
/// logged BEFORE the cap was reached.
///
/// 4M entries (~64 MB of bookkeeping). It was 16M while the log could fill with
/// duplicates (§81); with de-duplication the largest request measured (the
/// Wheels suite, 2,737 specs) peaks at ~1.2M distinct entries between sweeps.
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
const LOG_CAP_DEFAULT: usize = 4_000_000;

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
    // Every request and cfthread body starts here, so this is where a thread's
    // allocation totals get their exit flush (see `mem_account::arm_thread`).
    crate::mem_account::arm_thread();
    ALLOC_LOG.with(|c| *c.borrow_mut() = Some(fresh_log()));
    ALLOC_TOTAL.with(|n| n.set(0));
    OLD_LOG.with(|c| trim_vec(&mut c.borrow_mut()));
    OLD_SET.with(|c| trim_set(&mut c.borrow_mut()));
    NEXT_SWEEP.with(|c| c.set(incremental_threshold()));
    NEXT_MAJOR.with(|c| c.set(incremental_threshold()));
    LOG_PAUSED.with(|c| c.set(false));
    RELOG_SEEN.with(|c| trim_set(&mut c.borrow_mut()));
    reset_request_census();
}

thread_local! {
    /// Set once this request's log genuinely holds more DISTINCT live nodes
    /// than the cap can take even after compaction; logging stops for the rest
    /// of the request (the historical overflow behaviour, now reached only when
    /// the request really is that large — see `log_push`).
    static LOG_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Nodes the relog hook has already entered into the log during the current
    /// sweep interval — see [`relog_first_sight`].
    static RELOG_SEEN: RefCell<PtrSet> = RefCell::new(PtrSet::default());
}

/// Whether `ptr` has NOT yet been re-logged during the current sweep interval,
/// marking it as seen. The relog hook (`CfmlValue::relog_cycle_nodes`) enters a
/// displaced subgraph into the log so trial deletion can evaluate it, and every
/// overwrite of a key that holds a large graph enters the SAME nodes again: a
/// node only needs to be in the log once, but the Wheels suite pushed the same
/// ~260k nodes until the log held 16M entries and hit its cap — and with logging
/// paused, everything the rest of the request allocated was invisible to the
/// collector, so its cycles outlived the request (3.3 GB retained per run).
/// Cleared whenever the log is swept, because a sweep rebuilds the log from the
/// distinct survivors and a node displaced after that must be entered anew.
#[inline]
pub fn relog_first_sight(ptr: usize) -> bool {
    RELOG_SEEN.with(|c| c.borrow_mut().insert(ptr))
}

/// Stop logging and drop the log without collecting.
pub fn disable_and_clear() {
    ALLOC_LOG.with(|c| *c.borrow_mut() = None);
    OLD_LOG.with(|c| trim_vec(&mut c.borrow_mut()));
    OLD_SET.with(|c| trim_set(&mut c.borrow_mut()));
}

/// Current length of this thread's allocation log (`None` if not logging). For
/// diagnostics only.
pub fn log_len() -> Option<usize> {
    ALLOC_LOG.with(|c| c.borrow().as_ref().map(|v| v.len()))
        .map(|n| n + OLD_LOG.with(|c| c.borrow().len()))
}

/// Tracked containers allocated by the request running on this thread, counted
/// from `enable()` and never reduced by a sweep or the log cap. See `ALLOC_TOTAL`.
#[inline]
pub fn alloc_total() -> u64 {
    ALLOC_TOTAL.with(|n| n.get())
}

/// Composition of this thread's allocation log by container type
/// `(structs, arrays, queries, closure_scopes)`. Diagnostics only — answers
/// "what are the N tracked allocations a request made?" without a heap profiler.
pub fn log_type_breakdown() -> (usize, usize, usize, usize) {
    ALLOC_LOG.with(|c| {
        let b = c.borrow();
        let mut t = (0usize, 0usize, 0usize, 0usize);
        let old = OLD_LOG.with(|o| o.borrow().clone());
        if let Some(v) = b.as_ref() {
            for a in v.iter().chain(old.iter()) {
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
    let log = take_full_log();
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
    ALLOC_TOTAL.with(|n| n.set(n.get().wrapping_add(1)));
    ALLOC_LOG.with(|c| {
        let mut b = c.borrow_mut();
        if let Some(v) = b.as_mut() {
            if LOG_PAUSED.with(|c| c.get()) {
                // Genuine overflow (see below): drop `t`, keep the log as it is.
                return;
            }
            if v.len() >= log_cap() {
                // At the cap. The log is a LOG, not a set: the relog hook and the
                // closure-scope sites can enter one node many times, and entries
                // whose node has already been freed by refcounting are dead
                // weight. Compact it — drop dead entries, keep one per distinct
                // live node — and carry on logging. No graph walk and no
                // `upgrade()`: a `Weak::strong_count` read and a pointer per
                // entry, so this is cheap enough to run exactly when it is
                // needed. Only if the compacted log STILL nearly fills the cap
                // does the request genuinely hold that many distinct containers,
                // and only then does logging pause for the rest of the request:
                // collecting a partial log is conservative (an unlogged node
                // reads as an external root and is never over-collected), but
                // every cycle minted after the pause survives the request — so
                // the pause must be the last resort, not the first.
                let before = v.len();
                compact_log(v);
                let after = v.len();
                let debug = std::env::var("RUSTCFML_GC_DEBUG").is_ok();
                if after >= log_cap() / 4 * 3 {
                    LOG_PAUSED.with(|c| c.set(true));
                    if !OVERFLOW_WARNED.swap(true, Ordering::Relaxed) && debug {
                        eprintln!(
                            "[cycle_gc] log reached cap={} with {} distinct live nodes after \
                             compaction — logging paused for this request; partial \
                             (conservative) collection will still run",
                            log_cap(),
                            after
                        );
                    }
                    return;
                }
                if debug {
                    eprintln!(
                        "[cycle_gc] log reached cap={} — compacted {} entries to {} distinct live node(s)",
                        log_cap(),
                        before,
                        after
                    );
                }
            }
            v.push(t);
        }
    });
}

/// Drop dead entries and duplicate entries from a log in place, keeping the
/// first entry for each distinct live node. Order is preserved.
fn compact_log(v: &mut Vec<TrackedAlloc>) {
    let mut seen: PtrSet = PtrSet::with_capacity_and_hasher(v.len() / 8, Default::default());
    v.retain(|t| t.is_alive() && seen.insert(t.ptr()));
    v.shrink_to_fit();
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
        count_created(KIND_STRUCT);
        maybe_sample();
    }
}

#[inline]
pub fn log_array(arc: &Arc<PlRwLock<Vec<CfmlValue>>>) {
    if is_armed() {
        log_push(TrackedAlloc::Array(Arc::downgrade(arc)));
        count_created(KIND_ARRAY);
        maybe_sample();
    }
}

#[inline]
pub fn log_query(arc: &Arc<PlRwLock<CfmlQueryData>>) {
    if is_armed() {
        log_push(TrackedAlloc::Query(Arc::downgrade(arc)));
        count_created(KIND_QUERY);
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
        if count_created(KIND_INSTANCE) {
            if let Some(i) = arc.try_read() {
                let name = i.class.name.clone();
                let _ = CLASS_CREATED.try_with(|m| *m.borrow_mut().entry(name).or_insert(0) += 1);
            }
        }
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
        count_created(KIND_SCOPE);
    }
    arc
}

// ── Per-request object census (debug mode) ──────────────────────────────────
//
// While allocation accounting is on (`mem_account`, i.e. debugging is
// enabled), each request counts the containers it creates by kind, and
// component instances by class. [`request_census`] adds what of that is still
// alive, from the request's own allocation log, for the debug footer's memory
// panel. Counting is off when accounting is, and a relog (a displaced graph
// re-entered into the log) is not a creation.

const KIND_STRUCT: usize = 0;
const KIND_ARRAY: usize = 1;
const KIND_QUERY: usize = 2;
const KIND_SCOPE: usize = 3;
const KIND_INSTANCE: usize = 4;

thread_local! {
    static CREATED: std::cell::Cell<[u64; 5]> = const { std::cell::Cell::new([0; 5]) };
    static CLASS_CREATED: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    static IN_RELOG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Sweeps this request ran and the bytes they freed.
    static REQUEST_GC: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count one creation; true when counted.
#[inline]
fn count_created(kind: usize) -> bool {
    if !crate::mem_account::is_enabled() || IN_RELOG.with(|r| r.get()) {
        return false;
    }
    CREATED.with(|c| {
        let mut v = c.get();
        v[kind] += 1;
        c.set(v);
    });
    true
}

/// Mark this thread as relogging (re-entering a displaced graph) for the
/// guard's lifetime, so those entries are not counted as creations.
pub(crate) struct RelogGuard(bool);

impl RelogGuard {
    pub(crate) fn new() -> Self {
        Self(IN_RELOG.with(|r| r.replace(true)))
    }
}

impl Drop for RelogGuard {
    fn drop(&mut self) {
        IN_RELOG.with(|r| r.set(self.0));
    }
}

fn reset_request_census() {
    CREATED.with(|c| c.set([0; 5]));
    CLASS_CREATED.with(|m| m.borrow_mut().clear());
    REQUEST_GC.with(|g| g.set((0, 0)));
}

/// One kind of container in [`RequestCensus`].
#[derive(Debug, Clone, Default)]
pub struct KindCensus {
    /// Created by this request.
    pub created: u64,
    /// Still alive (of those it logged) when the census was taken.
    pub alive: u64,
    /// Estimated size of the alive ones, each counted without the containers
    /// it holds (those are counted under their own kind).
    pub alive_bytes: u64,
}

/// What one request created and still holds, by kind of container.
#[derive(Debug, Clone, Default)]
pub struct RequestCensus {
    pub structs: KindCensus,
    pub arrays: KindCensus,
    pub queries: KindCensus,
    pub scopes: KindCensus,
    pub instances: KindCensus,
    /// Component instances by class: `(class, census)`, most alive bytes first.
    pub classes: Vec<(String, KindCensus)>,
    /// Sweeps this request ran, and the bytes they freed.
    pub sweeps: u64,
    pub swept_bytes: u64,
    /// The log hit its cap and stopped recording, so the alive figures are a
    /// lower bound.
    pub incomplete: bool,
}

fn scalar_bytes(v: &CfmlValue, strings: &mut PtrSet) -> u64 {
    match v {
        CfmlValue::String(st) => {
            if strings.insert(Arc::as_ptr(st) as *const () as usize) {
                24 + st.len() as u64
            } else {
                0
            }
        }
        CfmlValue::Binary(b) => b.len() as u64,
        CfmlValue::Function(_) => 96,
        _ => 0,
    }
}

fn map_bytes<'a>(entries: impl Iterator<Item = (&'a str, &'a CfmlValue)>, strings: &mut PtrSet) -> u64 {
    let mut n = 32;
    for (k, v) in entries {
        n += 40 + k.len() as u64 + scalar_bytes(v, strings);
    }
    n
}

/// This thread's census: what the running request created, and what of it is
/// still alive. Walks the request's allocation log (young and old generation),
/// so call it while the request runs; it is empty once the log has been
/// collected. Sizes are estimates.
pub fn request_census() -> RequestCensus {
    let created = CREATED.with(|c| c.get());
    let mut out = RequestCensus {
        structs: KindCensus { created: created[KIND_STRUCT], ..Default::default() },
        arrays: KindCensus { created: created[KIND_ARRAY], ..Default::default() },
        queries: KindCensus { created: created[KIND_QUERY], ..Default::default() },
        scopes: KindCensus { created: created[KIND_SCOPE], ..Default::default() },
        instances: KindCensus { created: created[KIND_INSTANCE], ..Default::default() },
        ..Default::default()
    };
    let (sweeps, swept) = REQUEST_GC.with(|g| g.get());
    out.sweeps = sweeps;
    out.swept_bytes = swept;
    out.incomplete = LOG_PAUSED.with(|p| p.get());
    let mut classes: HashMap<String, KindCensus> = HashMap::new();
    CLASS_CREATED.with(|m| {
        for (k, n) in m.borrow().iter() {
            classes.entry(k.clone()).or_default().created = *n;
        }
    });
    let mut entries: Vec<TrackedAlloc> = Vec::new();
    ALLOC_LOG.with(|l| {
        if let Ok(l) = l.try_borrow() {
            if let Some(v) = l.as_ref() {
                entries.extend(v.iter().cloned());
            }
        }
    });
    OLD_LOG.with(|l| {
        if let Ok(l) = l.try_borrow() {
            entries.extend(l.iter().cloned());
        }
    });
    let mut seen = PtrSet::default();
    let mut strings = PtrSet::default();
    for e in &entries {
        match e {
            TrackedAlloc::Struct(w) => {
                let Some(a) = w.upgrade() else { continue };
                if !seen.insert(Arc::as_ptr(&a) as *const () as usize) {
                    continue;
                }
                let b = a
                    .try_read()
                    .map(|g| map_bytes(g.map.iter().map(|(k, v)| (k.as_str(), v)), &mut strings))
                    .unwrap_or(32);
                out.structs.alive += 1;
                out.structs.alive_bytes += b;
            }
            TrackedAlloc::Array(w) => {
                let Some(a) = w.upgrade() else { continue };
                if !seen.insert(Arc::as_ptr(&a) as *const () as usize) {
                    continue;
                }
                let b = a
                    .try_read()
                    .map(|v| 32 + v.len() as u64 * 24 + v.iter().map(|x| scalar_bytes(x, &mut strings)).sum::<u64>())
                    .unwrap_or(32);
                out.arrays.alive += 1;
                out.arrays.alive_bytes += b;
            }
            TrackedAlloc::Query(w) => {
                let Some(a) = w.upgrade() else { continue };
                if !seen.insert(Arc::as_ptr(&a) as *const () as usize) {
                    continue;
                }
                let b = a
                    .try_read()
                    .map(|d| {
                        32 + d.columns.iter().map(|c| c.len() as u64 + 24).sum::<u64>()
                            + d.data
                                .iter()
                                .map(|col| col.len() as u64 * 24 + col.iter().map(|x| scalar_bytes(x, &mut strings)).sum::<u64>())
                                .sum::<u64>()
                    })
                    .unwrap_or(32);
                out.queries.alive += 1;
                out.queries.alive_bytes += b;
            }
            TrackedAlloc::Scope(w) => {
                let Some(a) = w.upgrade() else { continue };
                if !seen.insert(Arc::as_ptr(&a) as *const () as usize) {
                    continue;
                }
                let b = a
                    .try_read()
                    .map(|m| map_bytes(m.iter().map(|(k, v)| (k.as_str(), v)), &mut strings))
                    .unwrap_or(32);
                out.scopes.alive += 1;
                out.scopes.alive_bytes += b;
            }
            #[cfg(feature = "component-instance")]
            TrackedAlloc::Instance(w) => {
                let Some(a) = w.upgrade() else { continue };
                if !seen.insert(Arc::as_ptr(&a) as *const () as usize) {
                    continue;
                }
                let Some(i) = a.try_read() else { continue };
                let mut b = 64;
                for m in [&i.this_members, &i.variables_members] {
                    b += m.with_read(|g| map_bytes(g.iter().map(|(k, v)| (k.as_str(), v)), &mut strings));
                }
                out.instances.alive += 1;
                out.instances.alive_bytes += b;
                let c = classes.entry(i.class.name.clone()).or_default();
                c.alive += 1;
                c.alive_bytes += b;
            }
        }
    }
    let mut classes: Vec<(String, KindCensus)> = classes.into_iter().collect();
    classes.sort_by(|a, b| {
        b.1.alive_bytes
            .cmp(&a.1.alive_bytes)
            .then(b.1.created.cmp(&a.1.created))
            .then(a.0.cmp(&b.0))
    });
    out.classes = classes;
    out
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
    fn for_each_child_node(&self, in_set: &PtrSet, emit: &mut impl FnMut(usize)) {
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
/// One-line shape of a value, for the diagnostic reports only.
/// Diagnostic tally of nodes the pass could not lock. A `try_read` that fails
/// makes the pass SKIP that node's outgoing edges, which inflates its children's
/// external count and pins them — conservative, but a leak. Counted so "is lock
/// contention pinning the graph?" is answerable instead of arguable.
static LOCK_SKIPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn note_lock_skip() {
    LOCK_SKIPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn describe_value(v: &CfmlValue) -> String {
    match v {
        CfmlValue::Struct(s) => {
            let keys: Vec<String> = s
                .with_read(|m| m.keys().take(5).map(|k| k.to_string()).collect());
            format!("Struct [{}]", keys.join(", "))
        }
        CfmlValue::Array(a) => format!("Array len={}", a.with_read(|x| x.len())),
        CfmlValue::Function(f) => format!("Function {}", f.name),
        CfmlValue::Closure(_) => "Closure".to_string(),
        CfmlValue::Query(_) => "Query".to_string(),
        CfmlValue::NativeObject(n) => n
            .read()
            .map(|g| format!("Native {}", g.class_name()))
            .unwrap_or_else(|_| "Native <locked>".to_string()),
        CfmlValue::String(s) => format!("String {:?}", &s.chars().take(40).collect::<String>()),
        other => format!("{:?}", std::mem::discriminant(other)),
    }
}

/// Emit the edges a function-like body carries.
///
/// `CfmlClosureBody` is not a leaf: both of its arms hold `CfmlValue`s, and a
/// `Statements` body holds one per statement. A body is therefore a perfectly
/// ordinary edge into the tracked graph, and leaving it out means anything
/// reachable ONLY through a function body reads as externally owned — a pinned
/// root whose whole transitive closure is then marked live. Walked from
/// [`classify`] so the count phase and the mark phase descend identically (the
/// invariant that keeps over-counting `internal_in` safe).
fn classify_body(body: &CfmlClosureBody, in_set: &PtrSet, emit: &mut impl FnMut(usize)) {
    match body {
        CfmlClosureBody::Expression(v) => classify(v, in_set, emit),
        CfmlClosureBody::Statements(sts) => {
            for st in sts {
                match st {
                    CfmlStatement::Expression(v) | CfmlStatement::Assignment(_, v) => {
                        classify(v, in_set, emit)
                    }
                    CfmlStatement::Return(Some(v)) => classify(v, in_set, emit),
                    CfmlStatement::Return(None) => {}
                }
            }
        }
    }
}

/// Emit the edges a `CfmlFunction` carries: its captured scope, its DEFAULT
/// PARAMETER VALUES (`CfmlParam::default` is a `CfmlValue` like any other) and
/// its body. All three were previously invisible except the scope.
fn classify_function(f: &CfmlFunction, in_set: &PtrSet, emit: &mut impl FnMut(usize)) {
    if let Some(sc) = &f.captured_scope {
        let p = Arc::as_ptr(sc) as *const () as usize;
        if in_set.contains(&p) {
            emit(p);
        }
    }
    for prm in &f.params {
        if let Some(d) = &prm.default {
            classify(d, in_set, emit);
        }
    }
    classify_body(&f.body, in_set, emit);
}

fn classify(v: &CfmlValue, in_set: &PtrSet, emit: &mut impl FnMut(usize)) {
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
        CfmlValue::Function(f) => classify_function(f, in_set, emit),
        CfmlValue::Component(c) => {
            for pv in c.properties.values() {
                classify(pv, in_set, emit);
            }
            for m in c.methods.values() {
                classify_function(m, in_set, emit);
            }
        }
        CfmlValue::Closure(c) => {
            for cv in c.captured_vars.values() {
                classify(cv, in_set, emit);
            }
            classify_body(&c.body, in_set, emit);
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
    let Some(log) = take_full_log() else {
        return 0;
    };
    // Survivors are carried into the cross-request set rather than abandoned —
    // see [`PersistentSet`] for why dropping them was a permanent loss of
    // tracking, and for the sweep's cost and correctness argument.
    let mut live: Vec<(usize, TrackedAlloc)> = Vec::new();
    let reclaimed = collect_from_log_carrying(log, Some(&mut live));
    RELOG_SEEN.with(|c| trim_set(&mut c.borrow_mut()));
    reclaimed + carry_survivors(live)
}

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
/// 25,000 was the knee: ~32x less memory on churn and ~5x on a large live set,
/// with CPU at or below the sweeping-off arm on BOTH. 10,000 buys a little more
/// memory back but starts paying for the extra passes on the live workload.
///
/// That table was measured when every sweep re-walked every survivor, so the
/// budget also had to bound the re-walk. With the generational sweep (young
/// entries only; survivors promoted and re-walked only by a major) the young
/// budget sets only how much young garbage may accumulate between minors, so a
/// larger one trades a little transient memory for fewer passes. Wheels suite
/// (2,737 specs), one run each: 25k → 590 minors, 5.8 s of sweeps, wall 49 s,
/// peak 760 M; 50k → 256 / 6.1 s / 46 s / 757 M; 100k → 119 / 4.1 s / 45 s /
/// 764 M. Peak did not move; 100k it is.
const INCREMENTAL_DEFAULT: usize = 100_000;

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
    /// Backing-pointer identity of the tracked node (the same key the collector
    /// de-duplicates survivors by). Valid whether or not the node is alive.
    fn ptr(&self) -> usize {
        match self {
            TrackedAlloc::Struct(w) => w.as_ptr() as *const () as usize,
            TrackedAlloc::Array(w) => w.as_ptr() as *const () as usize,
            TrackedAlloc::Query(w) => w.as_ptr() as *const () as usize,
            TrackedAlloc::Scope(w) => w.as_ptr() as *const () as usize,
            #[cfg(feature = "component-instance")]
            TrackedAlloc::Instance(w) => w.as_ptr() as *const () as usize,
        }
    }

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
/// **Why it is generational.** Re-walking every survivor on every pass is
/// quadratic in the live set: a request holding 60k live components went from
/// 3.4 s (sweeping off) to over TEN MINUTES at a flat 10k threshold. So a MINOR
/// sweep walks only the young entries and promotes their survivors to the old
/// generation, and the young budget stays at the fixed base. The old generation
/// is re-walked only by a MAJOR sweep, due once it has doubled since the last
/// one (`NEXT_MAJOR`, the classic "collect again when the heap has doubled"
/// rule). A churn workload (nothing survives) runs only minors; a workload that
/// holds N objects live runs O(log N) majors, so total work stays linear-ish.
///
/// Must not be called while an `ALLOC_LOG` borrow is held.
pub fn collect_incremental() -> usize {
    let base = incremental_threshold();
    if base == 0 {
        return 0;
    }
    let budget = NEXT_SWEEP.with(|c| c.get());
    let budget = if budget == usize::MAX { base } else { budget };
    let young = ALLOC_LOG.with(|c| {
        let mut b = c.borrow_mut();
        match b.as_mut() {
            Some(v) if v.len() >= budget => Some(std::mem::replace(v, fresh_log())),
            _ => None,
        }
    });
    let Some(young) = young else { return 0 };
    // A sweep is the natural moment to tell `mem_guard` how much this request has
    // allocated: it happens every `base` allocations regardless of what the
    // request is doing, so the `--max-memory` watchdog sees a fresh odometer for
    // every in-flight request without anything being published from the hot path.
    crate::mem_guard::publish_progress();
    let t0 = std::time::Instant::now();
    // GENERATIONAL. A minor sweep runs trial deletion over the YOUNG entries
    // only (allocated since the last sweep); a node still owned from outside
    // that set — including from an old-generation node — reads as external and
    // is kept, so a minor sweep is conservative in exactly the way a partial
    // log is. Survivors are PROMOTED to the old generation, which is re-walked
    // only by a MAJOR sweep (young + old together) once it has doubled since
    // the last major, or by `collect()` at request end. Before this, every
    // sweep re-walked every survivor: on the Wheels suite that was ~100 sweeps
    // over a ~300k-node live set, ~12 s of a 65 s run, to reclaim young cycles
    // that a walk of the young entries alone finds just as well.
    let next_major = NEXT_MAJOR.with(|c| c.get());
    let next_major = if next_major == usize::MAX { base } else { next_major };
    // Promoted survivors are mostly acyclic request-lifetime data that refcounting
    // frees soon after; their entries stay in the old generation as dead weight
    // and would trigger a major on COUNT alone. When the count reaches the
    // budget, first drop the dead entries (a `strong_count` read each, no graph
    // walk) and only call a major if the LIVE old set really has doubled.
    // Measured on the Wheels suite before this: 15 majors per run re-walking
    // ~19M entries to reclaim 490k nodes — half of all sweep time.
    let old_len = OLD_LOG.with(|c| {
        let mut o = c.borrow_mut();
        if o.len() >= next_major {
            o.retain(|t| t.is_alive());
            OLD_SET.with(|s| {
                let mut s = s.borrow_mut();
                s.clear();
                s.extend(o.iter().map(|t| t.ptr()));
            });
        }
        o.len()
    });
    let major = old_len >= next_major;
    let (log, kind) = if major {
        let mut log = OLD_LOG.with(|c| std::mem::take(&mut *c.borrow_mut()));
        OLD_SET.with(|c| c.borrow_mut().clear());
        log.reserve(young.len());
        log.extend(young);
        (log, "major")
    } else {
        (young, "minor")
    };
    let taken = log.len();
    // The survivors come back DE-DUPLICATED by backing pointer — one entry per
    // distinct live node — and must not be re-entered from the raw log: doing
    // so once inflated `live` to 15.8M for ~260k nodes and stopped all sweeps
    // for the rest of a request (known-issues §81).
    let mut survivors: Vec<(usize, TrackedAlloc)> = Vec::new();
    let reclaimed = collect_from_log_carrying(log, Some(&mut survivors));
    let promoted = survivors.len();
    let old_now = OLD_LOG.with(|c| {
        let mut o = c.borrow_mut();
        OLD_SET.with(|s| {
            let mut s = s.borrow_mut();
            for (p, t) in survivors {
                if s.insert(p) {
                    o.push(t);
                }
            }
        });
        o.len()
    });
    if major {
        // Doubling from the live count keeps the amortised cost of majors
        // linear: a fixed threshold re-walks the same live nodes over and over
        // (quadratic — and a clamp that put the budget BELOW the live count
        // once made every frame exit run a 330 ms sweep). Clamped to the log
        // cap so a sweep still fires when the log fills.
        let next = std::cmp::max(base, old_now.saturating_mul(2)).min(log_cap());
        NEXT_MAJOR.with(|c| c.set(next));
    }
    // Young budget: a fixed `base` — survivors are no longer re-walked by a
    // minor, so the cost of minors is linear in allocations regardless of the
    // live set.
    NEXT_SWEEP.with(|c| c.set(base));
    // The relog de-dup set describes the log that was just replaced.
    RELOG_SEEN.with(|c| c.borrow_mut().clear());
    if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!(
            "[cycle_gc] incremental {} sweep over {} reclaimed {} node(s); {} promoted, old={} next major at {} ({} ms)",
            kind,
            taken,
            reclaimed,
            promoted,
            old_now,
            NEXT_MAJOR.with(|c| c.get()),
            t0.elapsed().as_millis()
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
    seen: PtrSet,
    next_sweep: usize,
}

static PERSISTENT: parking_lot::Mutex<Option<PersistentSet>> = parking_lot::Mutex::new(None);

/// Floor for the cross-request sweep budget, so a small application does not
/// sweep on every request just because its live set is tiny. Overridable with
/// `RUSTCFML_GC_PERSISTENT` (`0` disables carrying survivors forward entirely,
/// restoring the drop-on-request-end behaviour).
const PERSISTENT_BASE_DEFAULT: usize = 50_000;

thread_local! {
    /// True only while [`sweep_entries`] is running. The orphan-sweep experiment
    /// must apply ONLY there: a request-end or mid-request pass runs while the
    /// probe roots are stale (they are installed at the END of a request) and
    /// while the running request's own locals — which no probe covers — are
    /// live, so suppressing unreachable roots in those passes frees data that is
    /// genuinely in use.
    static IN_SWEEP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Diagnostic: sweep the cross-request set at EVERY request end instead of on
/// the doubling rule (`RUSTCFML_GC_PERSISTENT_ALWAYS=1`). The budget exists so a
/// steady-state request pays nothing, which is right for production and wrong
/// for answering "is this reload's generation being reclaimed or pinned?" — with
/// this on, every request prints its own reclaimed/still-tracked line, and
/// pairing it with `RUSTCFML_GC_ROOTS=N` names whatever is doing the pinning.
/// Diagnostic: `RUSTCFML_GC_UNREACHABLE_REPORT=1`. During a cross-request sweep,
/// list (by shape) every survivor that NO installed probe root can reach. It
/// touches nothing.
///
/// Why it exists: a dead generation that will not collect shows its hub objects
/// with a small external refcount surplus and no visible holder. Two different
/// defects produce that picture — a holder the walk cannot see (the Preside
/// reload case: allocations made on cfthreads were never logged), or an edge
/// type the walk cannot follow. This report separates them: anything the engine
/// still USES that appears here is reachable in reality but not by our walk.
/// An earlier variant also SUPPRESSED those roots to test the hypothesis; that
/// mode is gone, because the probes do not enumerate every legitimate root (a
/// running request's frame, for one) and it freed live data.
fn unreachable_report() -> bool {
    use std::sync::OnceLock;
    static O: OnceLock<bool> = OnceLock::new();
    *O.get_or_init(|| std::env::var("RUSTCFML_GC_UNREACHABLE_REPORT").is_ok())
}

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
            seen: PtrSet::default(),
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

/// A generation-sized subgraph has been DISPLACED from a persistent scope since
/// the last cross-request sweep (an application restart, in practice). Set by
/// [`note_displacement`], consumed by [`sweep_if_displaced`].
///
/// Why this exists: the cross-request sweep otherwise runs on a doubling budget
/// (`next_sweep = 2 × live`), so after a reload the dead generation is not
/// re-examined until two or three MORE reloads have accumulated — and mimalloc
/// keeps the high-water mark, so the footprint records the peak of three
/// generations (~1.3G on Preside) instead of two. The displacement itself is the
/// precise moment a generation became garbage; sweeping then, and again once the
/// reload's threads have finished (they legitimately hold the old generation
/// while they run), frees it within seconds instead of reloads later.
static DISPLACED_PENDING: AtomicBool = AtomicBool::new(false);

/// Thread bodies currently executing (see `spawn_cfthread`). Maintained by
/// [`body_started`] / [`body_finished`]; the last one out runs the pending sweep.
static RUNNING_BODIES: AtomicUsize = AtomicUsize::new(0);

/// Smallest displaced subgraph that counts as "a generation" for the purposes
/// of [`DISPLACED_PENDING`]. `application.counter++` displaces one node and must
/// not trigger a sweep over a 300k-node set; `application.cbController = new`
/// displaces ~100k. `RUSTCFML_GC_DISPLACE_SWEEP_MIN` overrides (0 disables).
fn displace_sweep_min() -> usize {
    use std::sync::OnceLock;
    static M: OnceLock<usize> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("RUSTCFML_GC_DISPLACE_SWEEP_MIN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000)
    })
}

/// Record that `nodes` tracked nodes were just displaced from a persistent
/// scope. Called by the relog hook with the size of the subgraph it re-entered.
pub fn note_displacement(nodes: usize) {
    let min = displace_sweep_min();
    if min > 0 && nodes >= min && !DISPLACED_PENDING.swap(true, Ordering::Relaxed) {
        DISPLACE_ATTEMPTS.store(0, Ordering::Relaxed);
        // Remember how big the displacement was: a sweep only counts as having
        // freed it if it reclaims a comparable amount, not just the request's
        // ordinary churn. The relog walk is budget-capped, so this is a floor.
        DISPLACE_SIZE.store(nodes, Ordering::Relaxed);
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            eprintln!(
                "[cycle_gc] generation displaced ({} nodes re-entered): sweep pending",
                nodes
            );
        }
    }
}

/// A cfthread body has started executing.
pub fn body_started() {
    RUNNING_BODIES.fetch_add(1, Ordering::AcqRel);
}

/// A cfthread body has finished (its VM is dropped). A reload's threads are what
/// hold its predecessor generation alive after the request ends, so each exit
/// is a chance the generation has just become free: retry the pending sweep,
/// rate-limited (see [`sweep_if_displaced`]).
pub fn body_finished() {
    RUNNING_BODIES.fetch_sub(1, Ordering::AcqRel);
    sweep_if_displaced();
}

/// Attempts made at the current pending displacement sweep, and when the last
/// one ran. An application with continuous background threads (Preside's
/// heartbeats) never reaches "no body running", so the sweep cannot wait for
/// that; instead it runs, and if a thread still held the generation (nothing
/// generation-sized was reclaimed) it is retried on later body exits — no more
/// often than [`DISPLACE_RETRY_SECS`], and no more than [`DISPLACE_MAX_ATTEMPTS`]
/// times, so a displacement that turns out to be genuinely live cannot turn into
/// a sweep every few seconds forever.
static DISPLACE_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
/// Size (tracked nodes re-entered, a floor) of the pending displacement.
static DISPLACE_SIZE: AtomicUsize = AtomicUsize::new(0);
static DISPLACE_LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const DISPLACE_RETRY_SECS: u64 = 5;
const DISPLACE_MAX_ATTEMPTS: usize = 6;

fn now_ms() -> u64 {
    crate::clock::now_unix_millis() as u64
}

/// Run the pending displacement sweep, if there is one and the retry policy
/// allows. Returns nodes reclaimed. The flag clears once a sweep reclaims at
/// least a generation's worth ([`displace_sweep_min`]) — the displaced graph is
/// gone — or after [`DISPLACE_MAX_ATTEMPTS`].
pub fn sweep_if_displaced() -> usize {
    if !DISPLACED_PENDING.load(Ordering::Relaxed) {
        return 0;
    }
    // Rate limit: the first attempt runs immediately (request end), retries wait.
    let now = now_ms();
    let last = DISPLACE_LAST_MS.load(Ordering::Relaxed);
    let attempts = DISPLACE_ATTEMPTS.load(Ordering::Relaxed);
    if attempts > 0 && now.saturating_sub(last) < DISPLACE_RETRY_SECS * 1000 {
        return 0;
    }
    // Claim this attempt; a concurrent caller that loses the race skips.
    if DISPLACE_ATTEMPTS
        .compare_exchange(attempts, attempts + 1, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return 0;
    }
    DISPLACE_LAST_MS.store(now, Ordering::Relaxed);
    let n = sweep_persistent();
    // "Freed it" means reclaiming at least half of what was displaced — a
    // request's ordinary end-of-life churn (~1,500 nodes on Preside) must not
    // clear a 20,000+-node displacement and cancel the retries that would have
    // caught the generation once the reload's threads finished.
    let target = std::cmp::max(displace_sweep_min(), DISPLACE_SIZE.load(Ordering::Relaxed) / 2);
    let done = n >= target || attempts + 1 >= DISPLACE_MAX_ATTEMPTS;
    if done {
        DISPLACED_PENDING.store(false, Ordering::Relaxed);
    }
    if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
        eprintln!(
            "[cycle_gc] displacement sweep #{} reclaimed {} node(s){}",
            attempts + 1,
            n,
            if done { "; done" } else { "; still held — will retry" }
        );
    }
    n
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
    let t0 = std::time::Instant::now();
    IN_SWEEP.with(|c| c.set(true));
    let reclaimed = collect_from_log_carrying(entries, Some(&mut still_live));
    IN_SWEEP.with(|c| c.set(false));
    let took = t0.elapsed();
    let live_count = still_live.len();
    {
        let mut guard = PERSISTENT.lock();
        let set = guard.get_or_insert_with(|| PersistentSet {
            entries: Vec::new(),
            seen: PtrSet::default(),
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
             next sweep at {} ({} ms)",
            reclaimed,
            live_count,
            std::cmp::max(base, live_count.saturating_mul(2)),
            took.as_millis()
        );
    }
    reclaimed
}

/// The collection pass over an explicit allocation log (the live request's,
/// drained by `collect`, or a previously-deferred one). Identical algorithm
/// either way; factored out so deferred logs can be collected after their
/// spawning request's threads finish. Safe to run concurrently with unrelated
/// requests: the cycles it touches are internal to one finished request and
/// unreachable from anywhere else, so their `strong_count`s are stable.
fn collect_from_log(log: Vec<TrackedAlloc>) -> usize {
    collect_from_log_carrying(log, None)
}

fn collect_from_log_carrying(
    mut log: Vec<TrackedAlloc>,
    carry: Option<&mut Vec<(usize, TrackedAlloc)>>,
) -> usize {
    let _timer = CollectionTimer::start();
    if log.is_empty() {
        recycle_log(log);
        return 0;
    }

    // 1. Upgrade survivors; one strong probe handle per distinct backing.
    let mut nodes: PtrMap<NodeHandle> = PtrMap::with_capacity_and_hasher(log.len(), Default::default());
    for t in log.drain(..) {
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
    recycle_log(log);
    if nodes.is_empty() {
        return 0;
    }

    let in_set: PtrSet = nodes.keys().copied().collect();

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
    let blueprints: PtrMap<std::sync::Arc<crate::component::ClassBlueprint>> = {
        let mut bps = PtrMap::default();
        for h in nodes.values() {
            if let NodeHandle::Instance(a) = h {
                match a.try_read() {
                    Some(g) => {
                        let bp = g.class.clone();
                        bps.entry(Arc::as_ptr(&bp) as *const () as usize)
                            .or_insert(bp);
                    }
                    None => note_lock_skip(),
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
    let method_tables: PtrMap<Arc<ValueMap>> = {
        let mut t = PtrMap::default();
        for h in nodes.values() {
            match h {
                NodeHandle::Struct(a) => match a.try_read() {
                    Some(g) => {
                        if let Some(mt) = g.method_table.as_ref() {
                            t.entry(Arc::as_ptr(mt) as *const () as usize)
                                .or_insert_with(|| Arc::clone(mt));
                        }
                    }
                    None => note_lock_skip(),
                },
                // A flyweight Instance's data maps carry the SAME per-class
                // method table, and they are usually UNTRACKED — so gathering
                // tables only from tracked `Struct` nodes misses every instance
                // whose scopes were partitioned into plain data maps, which is
                // the common case. The table's entries are `CfmlFunction`s whose
                // captured scopes reach back into the instance graph; left
                // uncounted those scopes read as externally owned, become pinned
                // roots, and mark the whole generation live. Deduplicated by Arc
                // identity with the Struct-sourced ones, so a table reachable
                // both ways is still counted exactly once.
                #[cfg(feature = "component-instance")]
                NodeHandle::Instance(a) => match a.try_read() {
                    Some(g) => {
                        for m in [g.public_map_handle(), g.private_map_handle()] {
                            if let Some(mt) = m.method_table() {
                                t.entry(Arc::as_ptr(&mt) as *const () as usize)
                                    .or_insert(mt);
                            }
                        }
                    }
                    None => note_lock_skip(),
                },
                _ => {}
            }
        }
        t
    };

    // 2. internal_in[n] = number of references to n from other survivors, plus
    //    the references held by the carrier blueprints (counted once each).
    let mut internal_in: PtrMap<usize> = PtrMap::with_capacity_and_hasher(nodes.len(), Default::default());
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
    let mut bp_internal: PtrMap<usize> = PtrMap::with_capacity_and_hasher(blueprints.len(), Default::default());
    #[cfg(feature = "component-instance")]
    for h in nodes.values() {
        if let NodeHandle::Instance(a) = h {
            if let Some(g) = a.try_read() {
                let p = Arc::as_ptr(&g.class) as *const () as usize;
                *bp_internal.entry(p).or_insert(0) += 1;
            }
        }
    }

    // Probe reachability computed UP FRONT, but only for the
    // unreachable report (see `unreachable_report`), which needs it before roots
    // are chosen rather than after the mark.
    if unreachable_report() && IN_SWEEP.with(|c| c.get()) {
        let probes = PROBE_ROOT.lock().clone();
        let mut seen: PtrSet = PtrSet::default();
        let mut stack: Vec<usize> = Vec::new();
        for (_, pr) in probes.iter() {
            classify(pr, &in_set, &mut |c| {
                if seen.insert(c) {
                    stack.push(c);
                }
            });
        }
        while let Some(p) = stack.pop() {
            if let Some(h) = nodes.get(&p) {
                h.for_each_child_node(&in_set, &mut |c| {
                    if seen.insert(c) {
                        stack.push(c);
                    }
                });
                #[cfg(feature = "component-instance")]
                if let NodeHandle::Instance(a) = h {
                    if let Some(bp) = a.try_read().map(|g| g.class.clone()) {
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
        {
            let mut would: HashMap<String, usize> = HashMap::new();
            for (&p, h) in &nodes {
                if !seen.contains(&p) {
                    *would.entry(h.describe()).or_insert(0) += 1;
                }
            }
            let mut v: Vec<(String, usize)> = would.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            let total: usize = v.iter().map(|(_, n)| *n).sum();
            eprintln!(
                "[cycle_gc] UNREACHABLE-FROM-PROBES ({} node(s)) — anything the \
                 engine still USES in this list is reached by an edge our walk \
                 cannot follow:",
                total
            );
            for (d, n) in v.iter().filter(|(d, _)| d.starts_with("Instance")).take(30) {
                eprintln!("    n={:<5} {}", n, d);
            }
        }
    }

    // 3. Roots = survivors with an owner OUTSIDE the survivor set.
    //    external(n) = strong_count − 1 (probe handle) − internal_in(n).
    let mut live: PtrSet = PtrSet::with_capacity_and_hasher(nodes.len(), Default::default());
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
    let mut root_ext: PtrMap<usize> = PtrMap::default();
    // Index built once (not per root) so the diagnostic stays linear: a pass with
    // 250k survivors can have tens of thousands of roots.
    #[cfg(feature = "component-instance")]
    let data_map_owners: PtrMap<String> = if root_debug > 0 {
        let mut m = PtrMap::default();
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
        PtrMap::default()
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
                root_ext.insert(p, external);
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
    let mut live_bps: PtrSet = PtrSet::default();
    let mut live_tables: PtrSet = PtrSet::default();
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
            // A live component scope makes its shared method table live too —
            // whether that scope is a tracked Struct node or one of a live
            // Instance's untracked data maps (the mirror of how the tables are
            // gathered for `internal_in`; the two walks must agree or the mark
            // phase under-marks exactly what the count over-charged).
            let mut mark_table = |mt: Arc<ValueMap>,
                                  live: &mut PtrSet,
                                  worklist: &mut Vec<usize>| {
                if live_tables.insert(Arc::as_ptr(&mt) as *const () as usize) {
                    for v in mt.values() {
                        classify(v, &in_set, &mut |child| {
                            if live.insert(child) {
                                worklist.push(child);
                            }
                        });
                    }
                }
            };
            if let NodeHandle::Struct(a) = h {
                let mt = a.try_read().and_then(|g| g.method_table.clone());
                if let Some(mt) = mt {
                    mark_table(mt, &mut live, &mut worklist);
                }
            }
            #[cfg(feature = "component-instance")]
            if let NodeHandle::Instance(a) = h {
                let maps = a
                    .try_read()
                    .map(|g| (g.public_map_handle(), g.private_map_handle()));
                if let Some((this_m, vars_m)) = maps {
                    for m in [this_m, vars_m] {
                        if let Some(mt) = m.method_table() {
                            mark_table(mt, &mut live, &mut worklist);
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
        let mut probe_reachable: PtrSet = PtrSet::default();
        {
            let probes = PROBE_ROOT.lock();
            if !probes.is_empty() {
                // Cumulative: each named carrier is charged only with what no
                // EARLIER carrier already reached, so the numbers sum to the
                // covered total and one dominant holder stands out.
                let seen = &mut probe_reachable;
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
        // ORPHAN ANCHORS. A generation no persistent scope can reach is alive only
        // because some node in it carries an external reference. Those nodes are
        // the anchors — and they are what a leak hunt needs, because the plain
        // retainer ranking is dominated by the CURRENT generation, which is
        // legitimately held and looks identical on every reload.
        let mut anchors: Vec<(usize, usize)> = retention
            .iter()
            .filter(|(_, r)| !probe_reachable.is_empty() && !probe_reachable.contains(r))
            .map(|(n, r)| (*n, *r))
            .collect();
        anchors.sort_by(|a, b| b.0.cmp(&a.0));
        anchors.truncate(root_debug);

        let targets: PtrSet = retention
            .iter()
            .take(root_debug)
            .map(|(_, r)| *r)
            .chain(anchors.iter().map(|(_, r)| *r))
            .collect();
        let mut holders: PtrMap<Vec<String>> = PtrMap::default();
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
        // THE UNHELD SET. The ranked anchor list above is ordered by how much
        // each node RETAINS, which is dominated by whichever few nodes sit near
        // the top of a generation's object graph — and those are held by other
        // dead nodes, so chasing them is chasing downstream symptoms. The nodes
        // that actually keep a dead generation alive are the ones carrying an
        // external reference that NO tracked node accounts for: something in
        // Rust owns an `Arc` the collector cannot see, so trial deletion can
        // never zero them. This lists exactly that set, complete and grouped by
        // shape rather than truncated to a top-N, because the whole question is
        // how many distinct carriers there are and what they look like.
        if !probe_reachable.is_empty() {
            let mut tracked_holders: PtrSet = PtrSet::default();
            for h in nodes.values() {
                h.for_each_child_node(&in_set, &mut |child| {
                    tracked_holders.insert(child);
                });
            }
            #[cfg(feature = "component-instance")]
            for bp in blueprints.values() {
                blueprint_values(bp, |v| {
                    classify(v, &in_set, &mut |c| {
                        tracked_holders.insert(c);
                    })
                });
            }
            for mt in method_tables.values() {
                for v in mt.values() {
                    classify(v, &in_set, &mut |c| {
                        tracked_holders.insert(c);
                    })
                }
            }
            let mut unheld: HashMap<String, (usize, usize)> = HashMap::new();
            for (&p, h) in &nodes {
                if probe_reachable.contains(&p) || tracked_holders.contains(&p) {
                    continue;
                }
                let ext = root_ext.get(&p).copied().unwrap_or(0);
                if ext == 0 {
                    continue;
                }
                // An `Array len=1` tells you nothing about which array it is.
                // Group by the shape of what it CONTAINS as well, because that
                // is what identifies the carrier in engine terms.
                let inner = match h {
                    NodeHandle::Array(x) => x
                        .read()
                        .iter()
                        .take(4)
                        .map(describe_value)
                        .collect::<Vec<_>>()
                        .join(", "),
                    NodeHandle::Struct(x) => x
                        .read()
                        .map
                        .values()
                        .next()
                        .map(describe_value)
                        .unwrap_or_else(|| "<empty>".to_string()),
                    _ => String::new(),
                };
                let e = unheld
                    .entry(format!("{}  -> [{}]", h.describe(), inner))
                    .or_insert((0, 0));
                e.0 += 1;
                e.1 += ext;
            }
            // How big is the pinned set, and how much of it carries an external
            // reference at all? If only a handful of orphans report `ext > 0`
            // the generation is pinned by those few and they are the whole
            // hunt; if nearly ALL of them do, `internal_in` is undercounting
            // and the bug is in the edge walk, not in some carrier.
            let orphan_total = nodes.keys().filter(|p| !probe_reachable.contains(p)).count();
            let orphan_ext = nodes
                .keys()
                .filter(|p| {
                    !probe_reachable.contains(p) && root_ext.get(*p).copied().unwrap_or(0) > 0
                })
                .count();
            eprintln!(
                "[cycle_gc] lock skips this pass: {}",
                LOCK_SKIPS.swap(0, std::sync::atomic::Ordering::Relaxed)
            );
            eprintln!(
                "[cycle_gc] orphans: {} unreachable from any probe, {} of them carry \
                 an external ref ({} have none)",
                orphan_total,
                orphan_ext,
                orphan_total - orphan_ext
            );
            // The complement of the UNHELD set: orphan roots that DO have a
            // tracked holder yet still report an external reference. These are
            // the ones whose arithmetic does not close — `strong_count` exceeds
            // `1 + internal_in` — so something outside every walked carrier owns
            // them, and their transitive closure is what marks a dead generation
            // live. Listed COMPLETE, not top-N: the set is small and the whole
            // question is what is in it.
            let mut held: HashMap<String, (usize, usize)> = HashMap::new();
            for (&p, h) in &nodes {
                if probe_reachable.contains(&p) || !tracked_holders.contains(&p) {
                    continue;
                }
                let ext = root_ext.get(&p).copied().unwrap_or(0);
                if ext == 0 {
                    continue;
                }
                let e = held.entry(h.describe()).or_insert((0, 0));
                e.0 += 1;
                e.1 += ext;
            }
            let mut hv: Vec<(String, (usize, usize))> = held.into_iter().collect();
            hv.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
            let htotal: usize = hv.iter().map(|(_, (n, _))| *n).sum();
            eprintln!(
                "[cycle_gc] HELD-BUT-EXTERNAL orphans — tracked holder AND an \
                 unaccounted external ref ({} node(s)):",
                htotal
            );
            for (desc, (n, ext)) in hv.iter().take(60) {
                eprintln!("    n={:<5} ext={:<5} {}", n, ext, desc);
            }
            let mut v: Vec<(String, (usize, usize))> = unheld.into_iter().collect();
            v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            let total: usize = v.iter().map(|(_, (n, _))| *n).sum();
            eprintln!(
                "[cycle_gc] UNHELD orphans — external ref, no tracked holder \
                 ({} node(s), {} shape(s)):",
                total,
                v.len()
            );
            for (desc, (n, ext)) in v.into_iter().take(25) {
                eprintln!("    n={:<6} ext={:<6} {}", n, ext, desc);
            }
        }
        if !anchors.is_empty() {
            eprintln!(
                "[cycle_gc] ORPHAN anchors — retained but unreachable from any \
                 persistent scope:"
            );
            for (n, r) in anchors.iter() {
                let what = nodes.get(r).map(|h| h.describe()).unwrap_or_default();
                eprintln!(
                    "    retains={:<8} ext={:<4} {}",
                    n,
                    root_ext.get(r).copied().unwrap_or(0),
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
                        let shown: usize = v.iter().map(|(_, c)| **c).sum();
                        for (desc, c) in v.iter().take(12) {
                            eprintln!("        held by x{:<4} {}", c, desc);
                        }
                        eprintln!(
                            "        -> {} tracked holder edge(s) in total from {} distinct shape(s)",
                            shown,
                            v.len()
                        );
                    }
                    None => {
                        eprintln!("        held by      <nothing tracked — Rust-side state>")
                    }
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

    /// The request census counts what was created by kind and what is still
    /// alive, and a relog is not a creation.
    #[test]
    fn request_census_counts_created_and_alive() {
        std::thread::spawn(|| {
            crate::mem_account::enable();
            arm();
            enable();
            let keep = CfmlStruct::new(ValueMap::default());
            keep.insert("s".to_string(), CfmlValue::string("x".repeat(100)));
            for _ in 0..9 {
                drop(CfmlStruct::new(ValueMap::default()));
            }
            let arr = CfmlValue::array(vec![CfmlValue::Int(1)]);
            let c = request_census();
            assert_eq!(c.structs.created, 10);
            assert_eq!(c.structs.alive, 1, "nine were dropped");
            assert!(c.structs.alive_bytes >= 100, "the kept struct holds a 100-char string");
            assert_eq!(c.arrays.created, 1);
            assert_eq!(c.arrays.alive, 1);
            // Re-entering a graph into the log is not creating it.
            CfmlValue::Struct(keep.clone()).relog_cycle_nodes();
            assert_eq!(request_census().structs.created, 10);
            drop((keep, arr));
            disable_and_clear();
        })
        .join()
        .unwrap();
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
        // The survivor was PROMOTED to the old generation, so the sweep that
        // finds it is a major one (or request-end `collect()`).
        drop(live_a);
        drop(live_b);
        let _ = CfmlStruct::new(ValueMap::default());
        NEXT_SWEEP.with(|c| c.set(1));
        NEXT_MAJOR.with(|c| c.set(1));
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
        // First sweep is a minor: promotes the 80 live nodes to the old
        // generation. Then force a major and check ITS budget backed off.
        NEXT_SWEEP.with(|c| c.set(1));
        collect_incremental();
        let _ = CfmlStruct::new(ValueMap::default());
        NEXT_SWEEP.with(|c| c.set(1));
        NEXT_MAJOR.with(|c| c.set(1));
        collect_incremental();
        let after = NEXT_MAJOR.with(|c| c.get());
        assert!(
            after >= 80,
            "major budget must rise to ~2x the live set (>=80 for 40 live cycles), got {after}"
        );
        drop(live);
        disable_and_clear();
    }

    /// The log is a LOG, not a set: the relog hook and the closure-scope sites can
    /// enter one node many times. The sweep must carry each distinct survivor back
    /// ONCE. Carrying every raw entry inflated `live` (15.8M for ~260k nodes on the
    /// Wheels suite), pushed the doubled budget above the log cap, and stopped all
    /// further sweeps in that request — see known-issues §81.
    #[test]
    fn duplicate_log_entries_are_carried_once() {
        arm();
        enable();
        // 80 live nodes (closure-capture scopes: logged once at creation).
        let live: Vec<Arc<RwLock<ValueMap>>> =
            (0..80).map(|_| tracked_scope(ValueMap::default())).collect();
        // Enter every node another 50 times, the way repeated relogs do.
        for _ in 0..50 {
            for sc in &live {
                log_scope(sc);
            }
        }
        assert!(log_len().unwrap() >= 80 * 51, "precondition: the log holds duplicates");
        NEXT_SWEEP.with(|c| c.set(1));
        collect_incremental();
        assert_eq!(
            log_len(),
            Some(80),
            "after a sweep the log must hold exactly one entry per distinct live node"
        );
        drop(live);
        disable_and_clear();
    }

    /// At the cap the log is compacted — dead entries dropped, one entry kept per
    /// distinct live node — rather than logging being paused outright.
    #[test]
    fn compaction_keeps_one_entry_per_distinct_live_node() {
        let keep: Vec<Arc<RwLock<ValueMap>>> =
            (0..10).map(|_| Arc::new(RwLock::new(ValueMap::default()))).collect();
        let mut log: Vec<TrackedAlloc> = Vec::new();
        for _ in 0..7 {
            for sc in &keep {
                log.push(TrackedAlloc::Scope(Arc::downgrade(sc)));
            }
        }
        // Dead entries: nodes that no longer exist.
        for _ in 0..25 {
            let dead = Arc::new(RwLock::new(ValueMap::default()));
            log.push(TrackedAlloc::Scope(Arc::downgrade(&dead)));
        }
        assert_eq!(log.len(), 95);
        compact_log(&mut log);
        assert_eq!(log.len(), 10, "compaction must leave one live entry per node");
        assert!(log.iter().all(|t| t.is_alive()));
        drop(keep);
    }

    /// The relog hook's de-dup set is per sweep interval: a node entered since the
    /// last sweep is not entered again, and a sweep (or a new request) resets it,
    /// because the sweep rebuilds the log from the distinct survivors.
    #[test]
    fn relog_first_sight_is_per_sweep_interval() {
        arm();
        enable();
        assert!(relog_first_sight(0x1000));
        assert!(!relog_first_sight(0x1000), "second sight within an interval must be skipped");
        NEXT_SWEEP.with(|c| c.set(1));
        let _ = CfmlStruct::new(ValueMap::default());
        collect_incremental();
        assert!(relog_first_sight(0x1000), "a sweep must reset the de-dup set");
        assert!(!relog_first_sight(0x1000));
        enable();
        assert!(relog_first_sight(0x1000), "a new request must start with an empty set");
        disable_and_clear();
    }
}
