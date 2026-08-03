//! Runtime-armed **sampling heap profiler** — the memory-side counterpart to
//! the `--profile` CPU sampler in [`crate::pprof_profile`].
//!
//! # Why this exists
//!
//! The pre-existing heap tool is the `dhat-heap` feature, which swaps in DHAT's
//! global allocator. That is exact but unconditional: every allocation is
//! instrumented from process start, at a multiple-x slowdown, and the only
//! output is a dump on clean shutdown. It answers "where did the bytes go" only
//! if you are willing to run a purpose-built, badly-degraded binary for the
//! whole session.
//!
//! This module takes the approach jemalloc, Go and TCMalloc all take instead:
//! a global allocator that is **inert until armed at runtime**, and which then
//! samples roughly one allocation per `R` bytes (default 256 KiB) rather than
//! recording all of them. Disabled cost is one `Relaxed` atomic load plus a
//! predictable branch per allocator call. Armed cost is a sharded-mutex lookup
//! per allocator call plus a backtrace on the ~1-in-R sampled ones.
//!
//! # What you get
//!
//! Two profiles, both in standard pprof protobuf so they load in
//! `go tool pprof`, speedscope, Pyroscope, and the repo-root `pprof_top.py` /
//! `pprof_callers.py` scripts:
//!
//! * **inuse** (`*-inuse.pb`) — bytes still live at dump time, attributed to the
//!   stack that allocated them. This is the "where is my RSS" view, and at
//!   process end it is a leak report.
//! * **alloc** (`*-alloc.pb`) — every sampled allocation ever made, live or not.
//!   This is the "what is churning" view, and it drives allocator pressure even
//!   when it never shows up in RSS.
//!
//! Each is also written as a `.folded` collapsed-stack text file (inferno /
//! flamegraph.pl format, and trivial to roll up categorically with `awk`).
//!
//! # Statistical correctness
//!
//! Sampling is Poisson over the allocated-byte stream: a thread-local counter
//! is decremented by each allocation's size and a sample is taken when it goes
//! non-positive, at which point the counter is redrawn from an exponential
//! distribution with mean `R`. An object of size `s` is therefore sampled with
//! probability `1 - exp(-s/R)`, so each sample is scaled up by the inverse of
//! that to give an unbiased estimate of the true byte total. Small objects are
//! rarely sampled but count for a lot when they are; large ones are always
//! sampled and count for themselves. Totals converge on the truth; any single
//! rare stack may not.
//!
//! # Triggering a dump
//!
//! * `SIGUSR2` — dumps without stopping the process. The signal handler only
//!   sets an atomic; a watcher thread does the (allocating) work. Send repeatedly
//!   to get a time series; each dump gets its own sequence number.
//! * Graceful shutdown (Ctrl+C) — a final dump.
//!
//! Unix-only, because the on-demand dump is signal-driven.

#![cfg(all(feature = "memprofile", unix))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Mean bytes between samples. Override with `RUSTCFML_MEMPROFILE_RATE`.
/// 256 KiB keeps the live table in the low tens of thousands of entries for a
/// multi-hundred-MB heap while still resolving anything holding >~1 MB.
const DEFAULT_SAMPLE_RATE: usize = 256 * 1024;

/// Max native frames captured per sample. Deep CFML recursion goes through the
/// VM's own stack, not the Rust stack, so the Rust stack stays shallow; 64 is
/// comfortably past the deepest interpreter path.
const MAX_FRAMES: usize = 64;

/// Number of independently-locked shards in the live-object table. Deallocation
/// takes a shard lock on every call while armed, so this needs to be well past
/// the worker-thread count to keep contention off the hot path.
const SHARDS: usize = 64;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// The master switch. Checked on every allocator call; `Relaxed` because a
/// missed allocation either side of the arm/disarm edge is irrelevant.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Set by the `SIGUSR2` handler, polled by the watcher thread.
static DUMP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Sequence number so successive dumps don't overwrite each other.
static DUMP_SEQ: AtomicUsize = AtomicUsize::new(0);

static SAMPLE_RATE: AtomicUsize = AtomicUsize::new(DEFAULT_SAMPLE_RATE);

/// Unsampled running totals — cheap counters that give an exact live-byte
/// figure to sanity-check the sampled estimate against.
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static TOTAL_ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static TOTAL_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

// Self-diagnostics, reported in every dump.
//
// A sampling profiler that silently under-samples still produces a
// plausible-looking report — the shape looks right and the totals are simply
// wrong — so the sample accounting is made visible rather than assumed correct.
// This is not paranoia: during development a bug here made the sampler take 20
// samples where it should have taken ~1000, and the only reason it was caught
// was `sampled` being visibly absurd next to `due`.
//
// Read them as: `due` should track `sampled` closely, `reentry_skip` and
// `tls_err` should be ~0, and the reported effective rate should land near the
// configured one. Any large divergence means the profile is not trustworthy.
static N_DUE: AtomicU64 = AtomicU64::new(0);
static N_SAMPLED: AtomicU64 = AtomicU64::new(0);
static N_REENTRY_SKIP: AtomicU64 = AtomicU64::new(0);
static N_TLS_ERR: AtomicU64 = AtomicU64::new(0);
static SUM_INTERVAL: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Re-entrancy guard. The profiler itself allocates (backtrace capture, the
    /// hash maps, the dump writer); without this, recording a sample would
    /// recurse into the allocator and deadlock on a shard lock we already hold.
    static IN_PROFILER: Cell<bool> = const { Cell::new(false) };
    /// Bytes remaining until the next sample. Starts at 0, so the first
    /// allocation on each thread after arming takes a sample and redraws.
    static BYTES_TO_SAMPLE: Cell<i64> = const { Cell::new(0) };
    /// Per-thread xorshift state, lazily seeded.
    static RNG: Cell<u64> = const { Cell::new(0) };
}

/// RAII re-entrancy guard.
struct Reentry(bool);

impl Reentry {
    /// Returns `None` if we're already inside the profiler on this thread, or
    /// if TLS is unavailable (during thread teardown).
    fn enter() -> Option<Self> {
        match IN_PROFILER.try_with(|f| {
            if f.get() {
                false
            } else {
                f.set(true);
                true
            }
        }) {
            Ok(true) => Some(Reentry(true)),
            _ => None,
        }
    }
}

impl Drop for Reentry {
    fn drop(&mut self) {
        if self.0 {
            let _ = IN_PROFILER.try_with(|f| f.set(false));
        }
    }
}

// ---------------------------------------------------------------------------
// Sample bookkeeping
// ---------------------------------------------------------------------------

/// A sampled, still-live allocation.
#[derive(Clone, Copy)]
struct LiveEntry {
    stack: u32,
    /// Inverse-probability-weighted byte estimate, not the raw size.
    weight: u64,
}

#[derive(Default)]
struct Stacks {
    /// Interned unresolved instruction pointers → stack id. Symbol resolution
    /// is deferred to dump time; it is far too slow to do per sample.
    ids: HashMap<Box<[usize]>, u32>,
    frames: Vec<Box<[usize]>>,
    /// Cumulative sampled allocations per stack: (weighted bytes, weighted count).
    alloc_totals: Vec<(u64, u64)>,
}

static STACKS: Mutex<Option<Stacks>> = Mutex::new(None);

fn live_shards() -> &'static [Mutex<HashMap<usize, LiveEntry>>; SHARDS] {
    use std::sync::OnceLock;
    static LIVE: OnceLock<[Mutex<HashMap<usize, LiveEntry>>; SHARDS]> = OnceLock::new();
    LIVE.get_or_init(|| std::array::from_fn(|_| Mutex::new(HashMap::new())))
}

#[inline]
fn shard_of(ptr: usize) -> usize {
    // Pointers are allocator-aligned, so the low bits are near-constant —
    // mix before masking or every allocation lands in a handful of shards.
    let mut h = ptr as u64;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    (h as usize) % SHARDS
}

#[inline]
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A panic in an unrelated thread must not disable the profiler.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Draw the next sampling interval from Exp(1/rate).
fn next_interval(rate: usize) -> i64 {
    let r = RNG.try_with(|c| {
        let mut x = c.get();
        if x == 0 {
            // Seed from the thread id and an address; no clock needed.
            let seed = (&x as *const u64 as u64) ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9);
            x = seed | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        c.set(x);
        x
    });
    let bits = match r {
        Ok(x) => x,
        Err(_) => return rate as i64,
    };
    // Uniform in (0,1], then inverse-CDF.
    let u = ((bits >> 11) as f64 + 1.0) / ((1u64 << 53) as f64);
    let interval = -(rate as f64) * u.ln();
    // Clamp: an unlucky draw near 0 would sample every allocation.
    interval.clamp(1.0, (rate * 64) as f64) as i64
}

/// Unbiased inverse-probability weight for an object of `size` under mean
/// sampling interval `rate`: `size / (1 - exp(-size/rate))`.
fn weigh(size: usize, rate: usize) -> u64 {
    if size == 0 {
        return 0;
    }
    let s = size as f64;
    let p = 1.0 - (-s / rate as f64).exp();
    if p <= f64::EPSILON {
        rate as u64
    } else {
        (s / p) as u64
    }
}

/// Capture the current native stack, intern it, and return its id.
fn capture_stack() -> u32 {
    let mut ips: [usize; MAX_FRAMES] = [0; MAX_FRAMES];
    let mut n = 0usize;
    backtrace::trace(|frame| {
        if n >= MAX_FRAMES {
            return false;
        }
        ips[n] = frame.ip() as usize;
        n += 1;
        true
    });
    // Drop the profiler's own frames from the leaf end so the reported leaf is
    // the real caller. The exact count varies with inlining, so trim by name at
    // resolve time instead of guessing here; keep the raw stack intact.
    let key: Box<[usize]> = ips[..n].to_vec().into_boxed_slice();

    let mut guard = lock(&STACKS);
    let stacks = guard.get_or_insert_with(Stacks::default);
    if let Some(&id) = stacks.ids.get(&key) {
        return id;
    }
    let id = stacks.frames.len() as u32;
    stacks.frames.push(key.clone());
    stacks.alloc_totals.push((0, 0));
    stacks.ids.insert(key, id);
    id
}

/// Hot path: called on every allocation while armed.
#[inline]
fn on_alloc(ptr: *mut u8, size: usize) {
    // Bail before touching any counter if this allocation is the profiler's
    // own. Symbol resolution at dump time parses DWARF out of the binary and
    // allocates tens of MB; counting that would corrupt every reported total
    // and make the dump look like a program-side allocation storm.
    if IN_PROFILER.try_with(|f| f.get()).unwrap_or(true) {
        return;
    }
    LIVE_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    TOTAL_ALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    TOTAL_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);

    let due = BYTES_TO_SAMPLE.try_with(|c| {
        let remaining = c.get() - size as i64;
        c.set(remaining);
        remaining <= 0
    });
    match due {
        Ok(true) => {}
        Ok(false) => return,
        Err(_) => {
            N_TLS_ERR.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    N_DUE.fetch_add(1, Ordering::Relaxed);

    let Some(_guard) = Reentry::enter() else {
        N_REENTRY_SKIP.fetch_add(1, Ordering::Relaxed);
        return;
    };
    N_SAMPLED.fetch_add(1, Ordering::Relaxed);
    let rate = SAMPLE_RATE.load(Ordering::Relaxed);
    let iv = next_interval(rate);
    SUM_INTERVAL.fetch_add(iv as u64, Ordering::Relaxed);
    let _ = BYTES_TO_SAMPLE.try_with(|c| c.set(iv));

    let weight = weigh(size, rate);
    let stack = capture_stack();

    {
        let mut guard = lock(&STACKS);
        if let Some(stacks) = guard.as_mut() {
            let slot = &mut stacks.alloc_totals[stack as usize];
            slot.0 += weight;
            slot.1 += 1;
        }
    }

    let mut shard = lock(&live_shards()[shard_of(ptr as usize)]);
    shard.insert(ptr as usize, LiveEntry { stack, weight });
}

/// Hot path: called on every deallocation while armed.
#[inline]
fn on_dealloc(ptr: *mut u8, size: usize) {
    // Symmetric with `on_alloc`: profiler-internal frees must not decrement the
    // program's live-byte counter, or the two sides stop balancing.
    if IN_PROFILER.try_with(|f| f.get()).unwrap_or(true) {
        return;
    }
    LIVE_BYTES.fetch_sub(size as u64, Ordering::Relaxed);

    let Some(_guard) = Reentry::enter() else { return };
    let mut shard = lock(&live_shards()[shard_of(ptr as usize)]);
    shard.remove(&(ptr as usize));
}

// ---------------------------------------------------------------------------
// The allocator
// ---------------------------------------------------------------------------

/// A `System` passthrough that samples the heap when [`arm`] has been called.
///
/// Install it in the binary crate:
/// ```ignore
/// #[global_allocator]
/// static ALLOC: rustcfml_cli::memprofile::SamplingAlloc = rustcfml_cli::memprofile::SamplingAlloc;
/// ```
pub struct SamplingAlloc;

unsafe impl GlobalAlloc for SamplingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if ARMED.load(Ordering::Relaxed) && !p.is_null() {
            on_alloc(p, layout.size());
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if ARMED.load(Ordering::Relaxed) && !p.is_null() {
            on_alloc(p, layout.size());
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            on_dealloc(ptr, layout.size());
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    /// Overridden rather than left to the `GlobalAlloc` default, which would
    /// degrade every `Vec` growth to alloc+copy+free even when the system
    /// allocator could have grown the block in place.
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let armed = ARMED.load(Ordering::Relaxed);
        if armed {
            on_dealloc(ptr, layout.size());
        }
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if armed && !p.is_null() {
            on_alloc(p, new_size);
        }
        p
    }
}

// ---------------------------------------------------------------------------
// Arming / dumping
// ---------------------------------------------------------------------------

/// Arm the profiler, install the `SIGUSR2` dump handler, and start the watcher
/// thread. `out_prefix` names the output files.
pub fn arm(out_prefix: &str) {
    if let Ok(v) = std::env::var("RUSTCFML_MEMPROFILE_RATE") {
        if let Ok(n) = v.parse::<usize>() {
            if n > 0 {
                SAMPLE_RATE.store(n, Ordering::Relaxed);
            }
        }
    }
    let rate = SAMPLE_RATE.load(Ordering::Relaxed);

    // Pre-create the shard array and stack table before arming, so the first
    // sampled allocation isn't the one that has to allocate them.
    let _ = live_shards();
    {
        let mut g = lock(&STACKS);
        g.get_or_insert_with(Stacks::default);
    }

    // Confirm `SamplingAlloc` is actually the process allocator before promising
    // a profile. The `#[global_allocator]` hookup lives in the binary crate, so a
    // downstream binary built against this library (e.g. one produced by
    // `rustcfml --build` with its own generated `main.rs`) can enable the feature
    // yet never install it — and would then write an empty profile that looks
    // like "nothing allocated" rather than "not wired up".
    ARMED.store(true, Ordering::SeqCst);
    let before = TOTAL_ALLOC_COUNT.load(Ordering::Relaxed);
    {
        let probe: Vec<u8> = Vec::with_capacity(64 * 1024);
        std::hint::black_box(&probe);
    }
    if TOTAL_ALLOC_COUNT.load(Ordering::Relaxed) == before {
        ARMED.store(false, Ordering::SeqCst);
        eprintln!(
            "memprofile: SamplingAlloc is not installed as #[global_allocator]; \
             refusing to arm (would produce an empty profile)"
        );
        return;
    }

    install_sigusr2();

    let prefix = out_prefix.to_string();
    let watch_prefix = prefix.clone();
    std::thread::Builder::new()
        .name("memprofile-watch".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if DUMP_REQUESTED.swap(false, Ordering::SeqCst) {
                dump(&watch_prefix);
            }
        })
        .ok();

    ARMED.store(true, Ordering::SeqCst);
    eprintln!(
        "memprofile: armed, sampling ~1 per {} KiB → {prefix}-N-{{inuse,alloc}}.{{pb,folded}}",
        rate / 1024
    );
    eprintln!(
        "memprofile: send SIGUSR2 (kill -USR2 {}) to dump without stopping the process",
        std::process::id()
    );
}

/// Stop sampling and write a final dump.
pub fn finish(out_prefix: &str) {
    ARMED.store(false, Ordering::SeqCst);
    dump(out_prefix);
}

extern "C" fn sigusr2_handler(_sig: libc::c_int) {
    // Async-signal-safe: an atomic store and nothing else. The watcher thread
    // does the allocating work.
    DUMP_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_sigusr2() {
    unsafe {
        libc::signal(
            libc::SIGUSR2,
            sigusr2_handler as *const () as libc::sighandler_t,
        );
    }
}

/// Resolved frame list for one stack, leaf first, profiler frames trimmed.
fn resolve_stack(ips: &[usize]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for &ip in ips {
        let mut names: Vec<String> = Vec::new();
        backtrace::resolve(ip as *mut std::ffi::c_void, |sym| {
            let n = sym
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("{ip:#x}"));
            names.push(n);
        });
        if names.is_empty() {
            names.push(format!("{ip:#x}"));
        }
        for n in names {
            out.push((ip, n));
        }
    }
    // Trim the profiler's own frames from the leaf end. This must only consume a
    // CONTIGUOUS PREFIX: `out` is leaf-first, and scanning for the last match
    // anywhere would drain the entire stack whenever an allocator frame happens
    // to appear near the root (thread startup does exactly that), leaving only
    // thread-preamble frames and collapsing every distinct stack into a handful.
    let cut = out
        .iter()
        .position(|(_, n)| !is_profiler_frame(n))
        .unwrap_or(out.len());
    out.drain(..cut);
    out
}

/// Frames belonging to the profiler itself or to the bare allocator shim, which
/// carry no information about *who* allocated. Deliberately does not include
/// `RawVec`/`raw_vec` — those are the first genuinely informative frames.
fn is_profiler_frame(n: &str) -> bool {
    n.contains("memprofile")
        || n.contains("SamplingAlloc")
        || n.contains("backtrace::")
        || n.contains("__rust_alloc")
        || n.contains("__rust_realloc")
        || n.contains("__rust_dealloc")
        || n.contains("alloc::alloc::alloc")
        || n.contains("alloc::alloc::realloc")
        || n.contains("alloc::alloc::Global")
        || n.contains("std::alloc::")
        || n.contains("<alloc::alloc::Global as core::alloc::Allocator>")
}

/// Strip the trailing `::h<hash>` rustc appends to symbol names.
fn clean(name: &str) -> String {
    let b = name.as_bytes();
    if let Some(pos) = name.rfind("::h") {
        if b.len() - pos >= 11 && b[pos + 3..].iter().all(|c| c.is_ascii_hexdigit()) {
            return name[..pos].to_string();
        }
    }
    name.to_string()
}

/// Write the inuse + alloc profiles.
pub fn dump(out_prefix: &str) {
    let Some(_guard) = Reentry::enter() else { return };
    let seq = DUMP_SEQ.fetch_add(1, Ordering::SeqCst) + 1;

    // Snapshot every counter FIRST. Anything read later would include work done
    // by this dump on other threads and describe the profiler, not the program.
    let exact_live = LIVE_BYTES.load(Ordering::Relaxed);
    let exact_alloc = TOTAL_ALLOC_BYTES.load(Ordering::Relaxed);
    let exact_count = TOTAL_ALLOC_COUNT.load(Ordering::Relaxed);
    let n_sampled = N_SAMPLED.load(Ordering::Relaxed);
    let n_due = N_DUE.load(Ordering::Relaxed);
    let n_skip = N_REENTRY_SKIP.load(Ordering::Relaxed);
    let n_tls_err = N_TLS_ERR.load(Ordering::Relaxed);
    let sum_interval = SUM_INTERVAL.load(Ordering::Relaxed);

    // Snapshot the live table. Each shard is locked only for the length of its
    // own copy, so request threads keep running.
    let mut inuse: HashMap<u32, (u64, u64)> = HashMap::new();
    let mut live_sampled_objs: u64 = 0;
    for shard in live_shards().iter() {
        let g = lock(shard);
        for entry in g.values() {
            let slot = inuse.entry(entry.stack).or_insert((0, 0));
            slot.0 += entry.weight;
            slot.1 += 1;
            live_sampled_objs += 1;
        }
    }

    // Snapshot the stack table.
    let (frames, alloc_totals) = {
        let g = lock(&STACKS);
        match g.as_ref() {
            Some(s) => (s.frames.clone(), s.alloc_totals.clone()),
            None => (Vec::new(), Vec::new()),
        }
    };

    // Resolve symbols once, shared by both profiles.
    let resolved: Vec<Vec<(usize, String)>> = frames.iter().map(|f| resolve_stack(f)).collect();

    let inuse_samples: Vec<(u32, u64, u64)> =
        inuse.iter().map(|(&s, &(b, c))| (s, b, c)).collect();
    let alloc_samples: Vec<(u32, u64, u64)> = alloc_totals
        .iter()
        .enumerate()
        .filter(|(_, &(b, _))| b > 0)
        .map(|(i, &(b, c))| (i as u32, b, c))
        .collect();

    let est_inuse: u64 = inuse_samples.iter().map(|s| s.1).sum();
    let est_alloc: u64 = alloc_samples.iter().map(|s| s.1).sum();

    write_profile(
        &format!("{out_prefix}-{seq}-inuse"),
        &resolved,
        &inuse_samples,
        "inuse_space",
        "inuse_objects",
    );
    write_profile(
        &format!("{out_prefix}-{seq}-alloc"),
        &resolved,
        &alloc_samples,
        "alloc_space",
        "alloc_objects",
    );

    eprintln!("── memprofile dump #{seq} ─────────────────────────────");
    eprintln!(
        "  live (exact counter):   {:>10}",
        human(exact_live)
    );
    eprintln!(
        "  live (sampled estimate):{:>10}   from {live_sampled_objs} sampled live objects",
        human(est_inuse)
    );
    eprintln!(
        "  total allocated:        {:>10}   over {exact_count} allocations",
        human(exact_alloc)
    );
    eprintln!(
        "  total alloc (estimate): {:>10}",
        human(est_alloc)
    );
    eprintln!("  distinct stacks:        {:>10}", frames.len());
    eprintln!(
        "  sampler: due={n_due} sampled={n_sampled} reentry_skip={n_skip} tls_err={n_tls_err}"
    );
    if n_sampled > 0 {
        eprintln!(
            "  sampler: mean drawn interval {}  (configured {})",
            human(sum_interval / n_sampled),
            human(SAMPLE_RATE.load(Ordering::Relaxed) as u64)
        );
        eprintln!(
            "  sampler: effective 1 sample per {}",
            human(exact_alloc / n_sampled)
        );
    }
    eprintln!("  wrote {out_prefix}-{seq}-{{inuse,alloc}}.{{pb,folded}}");
}

fn human(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

/// Write one profile as both pprof protobuf and collapsed-stack text.
fn write_profile(
    path_prefix: &str,
    resolved: &[Vec<(usize, String)>],
    samples: &[(u32, u64, u64)],
    type_name: &str,
    count_name: &str,
) {
    // ── collapsed stacks (root-first, `;`-joined, value = bytes) ──
    let folded_path = format!("{path_prefix}.folded");
    match std::fs::File::create(&folded_path) {
        Ok(mut f) => {
            let mut buf = String::new();
            for &(stack, bytes, _) in samples {
                let Some(frames) = resolved.get(stack as usize) else {
                    continue;
                };
                if frames.is_empty() {
                    continue;
                }
                let joined: Vec<String> =
                    frames.iter().rev().map(|(_, n)| clean(n)).collect();
                buf.push_str(&joined.join(";"));
                buf.push(' ');
                buf.push_str(&bytes.to_string());
                buf.push('\n');
            }
            if let Err(e) = f.write_all(buf.as_bytes()) {
                eprintln!("memprofile: folded write failed: {e}");
            }
        }
        Err(e) => eprintln!("memprofile: cannot create {folded_path}: {e}"),
    }

    // ── pprof protobuf ──
    let pb = encode_pprof(resolved, samples, type_name, count_name);
    let pb_path = format!("{path_prefix}.pb");
    if let Err(e) = std::fs::File::create(&pb_path).and_then(|mut f| f.write_all(&pb)) {
        eprintln!("memprofile: pprof write failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Minimal pprof protobuf encoder
// ---------------------------------------------------------------------------
//
// Hand-rolled rather than pulled from the `pprof` crate's generated types: we
// need only a handful of fields, the wire format for them is stable, and this
// keeps the encoder available in builds that don't enable `obs-pprof`.
//
// perftools.profiles.Profile field numbers:
//   1 sample_type(ValueType) 2 sample(Sample) 4 location(Location)
//   5 function(Function) 6 string_table(string) 12 period 11 period_type

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_tag(out: &mut Vec<u8>, field: u32, wire: u32) {
    put_varint(out, ((field << 3) | wire) as u64);
}

fn put_varint_field(out: &mut Vec<u8>, field: u32, v: u64) {
    put_tag(out, field, 0);
    put_varint(out, v);
}

fn put_bytes_field(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    put_tag(out, field, 2);
    put_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

fn put_packed(out: &mut Vec<u8>, field: u32, vals: &[u64]) {
    let mut inner = Vec::new();
    for &v in vals {
        put_varint(&mut inner, v);
    }
    put_bytes_field(out, field, &inner);
}

/// String interner for the pprof string_table.
struct Strings {
    table: Vec<String>,
    idx: HashMap<String, u64>,
}

impl Strings {
    fn new() -> Self {
        // pprof requires string_table[0] == "".
        let mut s = Strings {
            table: vec![String::new()],
            idx: HashMap::new(),
        };
        s.idx.insert(String::new(), 0);
        s
    }
    fn get(&mut self, v: &str) -> u64 {
        if let Some(&i) = self.idx.get(v) {
            return i;
        }
        let i = self.table.len() as u64;
        self.table.push(v.to_string());
        self.idx.insert(v.to_string(), i);
        i
    }
}

fn encode_pprof(
    resolved: &[Vec<(usize, String)>],
    samples: &[(u32, u64, u64)],
    type_name: &str,
    count_name: &str,
) -> Vec<u8> {
    let mut s = Strings::new();
    let s_bytes = s.get("bytes");
    let s_count = s.get("count");
    let s_type = s.get(type_name);
    let s_ctype = s.get(count_name);

    // Assign ids. A "location" here is one (ip, symbol) pair — encoding inlined
    // frames as separate locations rather than as multiple Lines on one
    // location keeps the encoder simple and reads identically in pprof.
    let mut func_ids: HashMap<String, u64> = HashMap::new();
    let mut loc_ids: HashMap<(usize, String), u64> = HashMap::new();
    let mut funcs: Vec<(u64, u64)> = Vec::new(); // (id, name_str_idx)
    let mut locs: Vec<(u64, usize, u64)> = Vec::new(); // (id, address, func_id)

    for frames in resolved {
        for (ip, name) in frames {
            let key = (*ip, name.clone());
            if loc_ids.contains_key(&key) {
                continue;
            }
            let cleaned = clean(name);
            let fid = *func_ids.entry(cleaned.clone()).or_insert_with(|| {
                let id = funcs.len() as u64 + 1;
                let nidx = s.get(&cleaned);
                funcs.push((id, nidx));
                id
            });
            let lid = locs.len() as u64 + 1;
            locs.push((lid, *ip, fid));
            loc_ids.insert(key, lid);
        }
    }

    let mut out = Vec::new();

    // sample_type: [bytes, count]
    for (t, u) in [(s_type, s_bytes), (s_ctype, s_count)] {
        let mut vt = Vec::new();
        put_varint_field(&mut vt, 1, t);
        put_varint_field(&mut vt, 2, u);
        put_bytes_field(&mut out, 1, &vt);
    }

    // samples
    for &(stack, bytes, count) in samples {
        let Some(frames) = resolved.get(stack as usize) else {
            continue;
        };
        if frames.is_empty() {
            continue;
        }
        // pprof wants leaf first, which is the order `resolved` is already in.
        let ids: Vec<u64> = frames
            .iter()
            .filter_map(|(ip, n)| loc_ids.get(&(*ip, n.clone())).copied())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let mut sm = Vec::new();
        put_packed(&mut sm, 1, &ids);
        put_packed(&mut sm, 2, &[bytes, count]);
        put_bytes_field(&mut out, 2, &sm);
    }

    // locations
    for (id, addr, fid) in &locs {
        let mut lm = Vec::new();
        put_varint_field(&mut lm, 1, *id);
        put_varint_field(&mut lm, 3, *addr as u64);
        let mut line = Vec::new();
        put_varint_field(&mut line, 1, *fid);
        put_bytes_field(&mut lm, 4, &line);
        put_bytes_field(&mut out, 4, &lm);
    }

    // functions
    for (id, nidx) in &funcs {
        let mut fm = Vec::new();
        put_varint_field(&mut fm, 1, *id);
        put_varint_field(&mut fm, 2, *nidx);
        put_varint_field(&mut fm, 3, *nidx);
        put_bytes_field(&mut out, 5, &fm);
    }

    // string_table
    for st in &s.table {
        put_bytes_field(&mut out, 6, st.as_bytes());
    }

    // period_type + period: the mean sampling interval, so pprof knows the
    // values are already scaled estimates rather than raw counts.
    let mut pt = Vec::new();
    put_varint_field(&mut pt, 1, s_type);
    put_varint_field(&mut pt, 2, s_bytes);
    put_bytes_field(&mut out, 11, &pt);
    put_varint_field(&mut out, 12, SAMPLE_RATE.load(Ordering::Relaxed) as u64);

    out
}
