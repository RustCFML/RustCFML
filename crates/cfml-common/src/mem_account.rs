//! Allocation accounting: how many bytes each thread allocates and frees.
//!
//! Off by default. The release binary's global allocator (`AccountingAlloc` in
//! the CLI crate) calls [`note_alloc`] / [`note_free`] on every allocation, and
//! while accounting is off each call is a single relaxed load of [`ENABLED`].
//! It is switched on when debugging is enabled (`debugging.enabled` in
//! `.cfconfig.json`), whether or not a page shows the debug footer, and stays
//! on for the life of the process.
//!
//! What it gives:
//!
//! * **Per request** — the bytes a request allocated, the highest its in-use
//!   memory reached, and what it still held at the end ([`RequestMeter`]). This
//!   is FusionReactor's per-request allocation figure (the JVM's per-thread
//!   allocated-bytes counter), measured the same way.
//! * **Process-wide** — the live heap: bytes allocated minus bytes freed, over
//!   every thread ([`live_heap_bytes`]). Only meaningful when accounting was on
//!   from startup ([`enable_at_startup`]); switched on later, frees of memory
//!   allocated before it would drive the total below zero, so it is withheld.
//!
//! The counters are per thread and need no lock: const-initialised `Cell`s with
//! no destructor, so touching them from inside the allocator never allocates.
//! Each thread folds its net change into one process-wide atomic once it has
//! moved by [`FLUSH_BYTES`], so the live-heap figure is exact to within that
//! per thread.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// Whether allocations are being counted. One-way: once on, it stays on.
pub static ENABLED: AtomicBool = AtomicBool::new(false);
/// Set when accounting started before the server served anything, so the
/// process-wide live total covers every allocation still alive.
static FROM_START: AtomicBool = AtomicBool::new(false);
/// Process-wide net bytes (allocated − freed), as flushed by each thread.
static LIVE: AtomicI64 = AtomicI64::new(0);

/// How far a thread's unflushed net change may drift before it is added to
/// [`LIVE`]. Bounds the live-heap error to this much per running thread.
pub const FLUSH_BYTES: i64 = 64 * 1024;

thread_local! {
    /// Bytes this thread has allocated since accounting started.
    static ALLOCATED: Cell<u64> = const { Cell::new(0) };
    /// Bytes this thread has freed since accounting started (including memory
    /// another thread allocated).
    static FREED: Cell<u64> = const { Cell::new(0) };
    /// The highest `ALLOCATED − FREED` has reached since [`RequestMeter::start`]
    /// last reset it.
    static PEAK: Cell<i64> = const { Cell::new(i64::MIN) };
    /// Net change not yet added to [`LIVE`].
    static PENDING: Cell<i64> = const { Cell::new(0) };
    /// Set once this thread's [`ExitFlush`] has run: from then on every change
    /// goes straight to [`LIVE`], since nothing will flush it later.
    static DIRECT: Cell<bool> = const { Cell::new(false) };
    /// Flushes [`PENDING`] when the thread exits (see [`arm_thread`]).
    static EXIT_FLUSH: ExitFlush = const { ExitFlush };
}

/// A thread's unflushed change is lost when the thread exits, and the loss is
/// not random: a thread that runs a background task (a `cfthread`, a scheduled
/// task tick) frees data another thread allocated for it, so it exits with
/// frees not yet subtracted. On a Preside site, whose heartbeats run a fresh
/// thread per tick, that pushed the live heap past the footprint within an
/// hour. Dropped at thread exit, this adds what is left.
struct ExitFlush;

impl Drop for ExitFlush {
    fn drop(&mut self) {
        let _ = DIRECT.try_with(|d| d.set(true));
        let _ = PENDING.try_with(|p| {
            LIVE.fetch_add(p.get(), Ordering::Relaxed);
            p.set(0);
        });
    }
}

/// Make sure this thread's unflushed change reaches the process total when
/// the thread exits. Registering a thread-local destructor may allocate, so
/// the allocator itself cannot do it; it is done here, at the points every
/// request and `cfthread` body pass through (request start, collector start).
/// Cheap after the first call on a thread.
pub fn arm_thread() {
    if is_enabled() {
        let _ = EXIT_FLUSH.try_with(|_| {});
    }
}

#[inline]
fn net() -> i64 {
    ALLOCATED.with(|a| a.get()) as i64 - FREED.with(|f| f.get()) as i64
}

#[inline]
fn flush_if_due(delta: i64) {
    if DIRECT.try_with(|d| d.get()).unwrap_or(true) {
        LIVE.fetch_add(delta, Ordering::Relaxed);
        return;
    }
    PENDING.with(|p| {
        let v = p.get() + delta;
        if v >= FLUSH_BYTES || v <= -FLUSH_BYTES {
            LIVE.fetch_add(v, Ordering::Relaxed);
            p.set(0);
        } else {
            p.set(v);
        }
    });
}

/// Record an allocation of `size` bytes. Called by the global allocator: must
/// not allocate, and must be cheap while accounting is off.
///
/// Only the flag test is inlined. Link-time optimisation inlines the global
/// allocator into every allocation site in the binary, so an inlined counting
/// body was copied thousands of times: +8 MB of code and ~2% on an
/// allocation-heavy request, with accounting OFF. The counting itself lives in
/// one out-of-line function.
#[inline(always)]
pub fn note_alloc(size: usize) {
    if ENABLED.load(Ordering::Relaxed) {
        count_alloc(size);
    }
}

/// Record a free of `size` bytes. Called by the global allocator.
#[inline(always)]
pub fn note_free(size: usize) {
    if ENABLED.load(Ordering::Relaxed) {
        count_free(size);
    }
}

#[cold]
#[inline(never)]
fn count_alloc(size: usize) {
    // `try_with`: a TLS slot being torn down at thread exit is skipped, not a
    // panic inside the allocator.
    let _ = ALLOCATED.try_with(|a| a.set(a.get() + size as u64));
    let n = net();
    let _ = PEAK.try_with(|p| {
        if n > p.get() {
            p.set(n)
        }
    });
    flush_if_due(size as i64);
}

#[cold]
#[inline(never)]
fn count_free(size: usize) {
    let _ = FREED.try_with(|f| f.set(f.get() + size as u64));
    flush_if_due(-(size as i64));
}

/// Start counting, at server startup. The process-wide live total is then
/// published, because it covers every allocation still alive.
pub fn enable_at_startup() {
    if !ENABLED.swap(true, Ordering::Relaxed) {
        FROM_START.store(true, Ordering::Relaxed);
    }
    arm_thread();
}

/// Start counting, from now (when a request's own configuration enables
/// debugging). Per-request figures work from the next request on; the
/// process-wide live total stays withheld.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Bytes allocated and not yet freed, across every thread, when accounting has
/// been on since startup. Exact to within [`FLUSH_BYTES`] per thread.
pub fn live_heap_bytes() -> Option<u64> {
    if !FROM_START.load(Ordering::Relaxed) {
        return None;
    }
    Some(LIVE.load(Ordering::Relaxed).max(0) as u64)
}

/// This thread's totals so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadTotals {
    pub allocated: u64,
    pub freed: u64,
}

pub fn thread_totals() -> ThreadTotals {
    ThreadTotals {
        allocated: ALLOCATED.with(|a| a.get()),
        freed: FREED.with(|f| f.get()),
    }
}

/// One request's (or thread body's) memory, measured on the thread running it.
#[derive(Debug, Clone, Copy)]
pub struct RequestMeter {
    start: ThreadTotals,
}

/// What a [`RequestMeter`] read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestMemory {
    /// Bytes allocated since the meter started.
    pub allocated: u64,
    /// Bytes freed since the meter started.
    pub freed: u64,
    /// The highest in-use memory reached above the starting point.
    pub peak: u64,
    /// In use now above the starting point: allocated − freed (zero if the
    /// request freed more than it allocated).
    pub retained: u64,
}

impl RequestMeter {
    /// Start measuring on this thread, or `None` while accounting is off. Resets
    /// the thread's peak, so one meter runs per thread at a time.
    pub fn start() -> Option<Self> {
        if !is_enabled() {
            return None;
        }
        arm_thread();
        let start = thread_totals();
        PEAK.with(|p| p.set(start.allocated as i64 - start.freed as i64));
        Some(Self { start })
    }

    pub fn read(&self) -> RequestMemory {
        let now = thread_totals();
        let allocated = now.allocated.saturating_sub(self.start.allocated);
        let freed = now.freed.saturating_sub(self.start.freed);
        let base = self.start.allocated as i64 - self.start.freed as i64;
        let peak = PEAK.with(|p| p.get()).saturating_sub(base).max(0) as u64;
        RequestMemory {
            allocated,
            freed,
            peak,
            retained: allocated.saturating_sub(freed),
        }
    }
}

// ── Process figures the embedder supplies ───────────────────────────────────
//
// The footprint is measured by the server (cgroup / RSS / phys_footprint, in
// the CLI crate's `memory_limit`), which the VM cannot depend on. The server
// registers its meter here so the debug footer can show the same number
// `--max-memory` acts on.

static FOOTPRINT_FN: std::sync::OnceLock<fn() -> Option<u64>> = std::sync::OnceLock::new();
static LIMIT_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Register the process-footprint meter. First registration wins.
pub fn set_footprint_fn(f: fn() -> Option<u64>) {
    let _ = FOOTPRINT_FN.set(f);
}

/// The process's physical footprint in bytes, when a meter is registered.
pub fn footprint_bytes() -> Option<u64> {
    FOOTPRINT_FN.get().and_then(|f| f())
}

/// Record the `--max-memory` limit (bytes; 0 = none).
pub fn set_limit_bytes(bytes: u64) {
    LIMIT_BYTES.store(bytes, Ordering::Relaxed);
}

pub fn limit_bytes() -> Option<u64> {
    match LIMIT_BYTES.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thread that frees what another allocated, then exits with the change
    /// still below the flush threshold, must not lose it.
    #[test]
    fn a_thread_exit_flushes_what_it_had_not_added() {
        enable_at_startup();
        // Other tests move LIVE too; this one only needs its own delta, so it
        // runs its two threads alone and compares across them.
        let before = LIVE.load(Ordering::Relaxed);
        std::thread::spawn(|| {
            arm_thread();
            note_free(10_000); // well under FLUSH_BYTES
        })
        .join()
        .unwrap();
        let after = LIVE.load(Ordering::Relaxed);
        assert_eq!(after - before, -10_000, "the exiting thread's frees reach the total");
    }

    // The real allocator is not wired up in unit tests, so these drive the
    // hooks by hand. Each test runs on its own thread (fresh thread-locals).
    #[test]
    fn meter_reads_allocated_freed_peak_and_retained() {
        std::thread::spawn(|| {
            enable();
            let m = RequestMeter::start().expect("enabled");
            note_alloc(1000);
            note_alloc(500);
            note_free(800);
            note_alloc(100);
            let r = m.read();
            assert_eq!(r.allocated, 1600);
            assert_eq!(r.freed, 800);
            assert_eq!(r.peak, 1500, "the high-water mark, before the free");
            assert_eq!(r.retained, 800);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn freeing_more_than_allocated_retains_nothing() {
        std::thread::spawn(|| {
            enable();
            let m = RequestMeter::start().unwrap();
            note_free(4096);
            let r = m.read();
            assert_eq!((r.allocated, r.freed, r.peak, r.retained), (0, 4096, 0, 0));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_new_meter_resets_the_peak() {
        std::thread::spawn(|| {
            enable();
            let m = RequestMeter::start().unwrap();
            note_alloc(10_000);
            note_free(10_000);
            assert_eq!(m.read().peak, 10_000);
            let m2 = RequestMeter::start().unwrap();
            note_alloc(10);
            assert_eq!(m2.read().peak, 10);
        })
        .join()
        .unwrap();
    }
}
