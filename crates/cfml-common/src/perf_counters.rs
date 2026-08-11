//! Process-global diagnostic counters for sizing performance levers before
//! designing them (the counter-first rule: never size a lever from a profile
//! share). Increments are single relaxed atomic adds — cheap enough to leave
//! compiled in unconditionally. Nothing is ever printed unless the embedder
//! asks for a [`report`] (the CLI does so at graceful shutdown when
//! `RUSTCFML_COUNTERS=1`).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// `CfmlStruct::new` calls (cycle-GC-tracked structs: scopes, literals, CFCs).
pub static STRUCT_NEW: AtomicU64 = AtomicU64::new(0);
/// `CfmlStruct::new_untracked` calls (frame-confined scopes, Lever C opt 1).
pub static STRUCT_NEW_UNTRACKED: AtomicU64 = AtomicU64::new(0);

/// Entries into `resolve_component_template`.
pub static RESOLVE_CALLS: AtomicU64 = AtomicU64::new(0);
/// Resolutions answered by the two-layer path cache (request or production).
pub static RESOLVE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
/// Resolutions that fell through to the candidate-path probe walk.
pub static RESOLVE_PROBE_WALKS: AtomicU64 = AtomicU64::new(0);

/// Existence memo answers served without touching the filesystem.
pub static EXISTS_MEMO_HITS: AtomicU64 = AtomicU64::new(0);
/// Actual VFS existence probes (each is ≥1 `stat` syscall).
pub static EXISTS_FS_PROBES: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Slot-locals coverage (perf plan T3.1 / P2 sizing). The COMPILE-side counters
// classify every finalized function once; the RUNTIME pair weights those
// classes by how often frames of each kind are actually entered. Sizing P2
// needs both: static coverage says what fraction of code is reachable, frame
// counts say whether that code is the code that runs.
// ---------------------------------------------------------------------------
/// Functions that came out of `assign_local_slots` with ≥1 slot.
pub static SLOT_FN_SLOTTED: AtomicU64 = AtomicU64::new(0);
/// Functions disqualified WHOLESALE because the body defines a closure.
pub static SLOT_FN_DISQ_CLOSURE: AtomicU64 = AtomicU64::new(0);
/// Functions disqualified wholesale for any other reason (include, dynamic
/// var set, `structDelete(local)`, the reflective `evaluate` family).
pub static SLOT_FN_DISQ_OTHER: AtomicU64 = AtomicU64::new(0);
/// Per-reason split of `SLOT_FN_DISQ_OTHER` (functions / static ops / ops
/// before the first offending op — the prefix a spill-instead-of-disqualify
/// design would recover).
pub static SLOT_FN_DISQ_INCLUDE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_INCLUDE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_INCLUDE_PREFIX: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_DYNVAR: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_DYNVAR: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_REFLECTIVE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_REFLECTIVE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_DISQ_DELSCOPE: AtomicU64 = AtomicU64::new(0);
/// Eligible functions that simply had no slottable name (no `var`, or every
/// candidate excluded per-name).
pub static SLOT_FN_NO_CANDIDATES: AtomicU64 = AtomicU64::new(0);
/// Split of the above: bodies with no `var` declaration AT ALL (nothing stage 1
/// could ever slot — these are the params-and-`variables`-only methods that
/// P2 item 2 targets) vs bodies whose every declared name was excluded per-name.
pub static SLOT_FN_NO_DECLARES: AtomicU64 = AtomicU64::new(0);
pub static SLOT_FN_ALL_EXCLUDED: AtomicU64 = AtomicU64::new(0);
/// Declared params living in functions that ended up with no slots — the
/// upper bound on what "params into slots" would add.
pub static SLOT_PARAMS_UNSLOTTED_FNS: AtomicU64 = AtomicU64::new(0);
/// Frame entries into an unslotted function that HAS declared params (i.e. the
/// entries P2 item 2 could convert), and into one with none (unreachable by
/// any slot widening).
pub static FRAMES_UNSLOTTED_WITH_PARAMS: AtomicU64 = AtomicU64::new(0);
/// Op counts for the same three classes (static body length — the same
/// weighting the 2026-08 JIT admission scan used).
pub static SLOT_OPS_SLOTTED: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_CLOSURE: AtomicU64 = AtomicU64::new(0);
pub static SLOT_OPS_DISQ_OTHER: AtomicU64 = AtomicU64::new(0);
/// Of `SLOT_OPS_DISQ_CLOSURE`, the ops that sit BEFORE the body's first
/// `DefineFunction`. That prefix is all a spill-on-DefineFunction design can
/// recover; the remainder needs per-name exclusion instead.
pub static SLOT_OPS_CLOSURE_PREFIX: AtomicU64 = AtomicU64::new(0);
/// Frame entries into a function that has slots vs one that has none.
pub static FRAMES_SLOTTED: AtomicU64 = AtomicU64::new(0);
pub static FRAMES_UNSLOTTED: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn add(c: &AtomicU64, n: u64) {
    c.fetch_add(n, Relaxed);
}

#[inline]
pub fn bump(c: &AtomicU64) {
    c.fetch_add(1, Relaxed);
}

/// Snapshot every counter as a one-block human-readable report.
pub fn report() -> String {
    let g = |c: &AtomicU64| c.load(Relaxed);
    #[cfg(feature = "alloc-sizing")]
    {
        let mut out = report_totals(g);
        out.push('\n');
        out.push_str(&alloc_sites::report(60));
        return out;
    }
    #[cfg(not(feature = "alloc-sizing"))]
    report_totals(g)
}

fn report_totals(g: impl Fn(&AtomicU64) -> u64) -> String {
    format!(
        "=== RUSTCFML_COUNTERS ===\n\
         struct_new (tracked):        {:>12}\n\
         struct_new_untracked:        {:>12}\n\
         resolve_component calls:     {:>12}\n\
           .. path-cache hits:        {:>12}\n\
           .. candidate probe walks:  {:>12}\n\
         exists memo hits:            {:>12}\n\
         exists FS probes (stats):    {:>12}\n\
         --- slot-locals coverage (functions / static ops) ---\n\
         slotted:                     {:>12} {:>12}\n\
         disq. closure-defining:      {:>12} {:>12}\n\
           .. ops before 1st closure: {:>25}\n\
         disq. other:                 {:>12} {:>12}\n\
           .. include:                {:>12} {:>12}\n\
           .... ops before 1st incl:  {:>25}\n\
           .. dynamic var set:        {:>12} {:>12}\n\
           .. reflective builtin:     {:>12} {:>12}\n\
           .. structDelete(local):    {:>12}\n\
         eligible, no candidates:     {:>12}\n\
           .. no `var` at all:        {:>12}\n\
           .. all names excluded:     {:>12}\n\
           .. params in unslotted fns:{:>12}\n\
         frame entries slotted:       {:>12}\n\
         frame entries unslotted:     {:>12}\n\
           .. of those, has params:   {:>12}",
        g(&STRUCT_NEW),
        g(&STRUCT_NEW_UNTRACKED),
        g(&RESOLVE_CALLS),
        g(&RESOLVE_CACHE_HITS),
        g(&RESOLVE_PROBE_WALKS),
        g(&EXISTS_MEMO_HITS),
        g(&EXISTS_FS_PROBES),
        g(&SLOT_FN_SLOTTED),
        g(&SLOT_OPS_SLOTTED),
        g(&SLOT_FN_DISQ_CLOSURE),
        g(&SLOT_OPS_DISQ_CLOSURE),
        g(&SLOT_OPS_CLOSURE_PREFIX),
        g(&SLOT_FN_DISQ_OTHER),
        g(&SLOT_OPS_DISQ_OTHER),
        g(&SLOT_FN_DISQ_INCLUDE),
        g(&SLOT_OPS_DISQ_INCLUDE),
        g(&SLOT_OPS_INCLUDE_PREFIX),
        g(&SLOT_FN_DISQ_DYNVAR),
        g(&SLOT_OPS_DISQ_DYNVAR),
        g(&SLOT_FN_DISQ_REFLECTIVE),
        g(&SLOT_OPS_DISQ_REFLECTIVE),
        g(&SLOT_FN_DISQ_DELSCOPE),
        g(&SLOT_FN_NO_CANDIDATES),
        g(&SLOT_FN_NO_DECLARES),
        g(&SLOT_FN_ALL_EXCLUDED),
        g(&SLOT_PARAMS_UNSLOTTED_FNS),
        g(&FRAMES_SLOTTED),
        g(&FRAMES_UNSLOTTED),
        g(&FRAMES_UNSLOTTED_WITH_PARAMS),
    )
}

/// Dynamic op census (`op-census` builds only) — how many times each bytecode
/// op is actually EXECUTED, as opposed to how many times it appears in compiled
/// bodies. The 2026-08 JIT admission scan weighted ops statically; a Tier-0
/// design has to be sized against the dynamic mix, because loops mean a
/// function's ops do not execute once per frame entry. Indexed by
/// `BytecodeOp::census_index()`; the VM's dispatch loop bumps one relaxed add
/// per op, which is why this lives behind a feature and never ships on.
#[cfg(feature = "op-census")]
pub mod op_census {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// Room for every `BytecodeOp` variant (121 today) plus headroom, so adding
    /// an op cannot silently start writing out of bounds.
    pub const SLOTS: usize = 192;

    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicU64 = AtomicU64::new(0);
    pub static COUNTS: [AtomicU64; SLOTS] = [ZERO; SLOTS];

    #[inline]
    pub fn bump(idx: usize) {
        if let Some(c) = COUNTS.get(idx) {
            c.fetch_add(1, Relaxed);
        }
    }

    /// Snapshot every non-zero slot as `(index, count)`, so callers can diff two
    /// snapshots to isolate one request's mix from cumulative boot totals.
    pub fn snapshot() -> Vec<(usize, u64)> {
        COUNTS
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.load(Relaxed)))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    /// Human-readable descending report. `names` comes from
    /// `BytecodeOp::CENSUS_NAMES` (cfml-common cannot depend on cfml-codegen,
    /// so the embedder passes it in).
    pub fn report(names: &[&str]) -> String {
        let mut rows = snapshot();
        let total: u64 = rows.iter().map(|(_, n)| *n).sum();
        rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut out = format!(
            "--- dynamic op census: {} ops executed across {} distinct opcodes ---",
            total,
            rows.len()
        );
        let mut cum = 0u64;
        for (i, n) in &rows {
            cum += n;
            out.push_str(&format!(
                "\n{:>14} {:>6.2}% {:>6.2}%cum  {}",
                n,
                *n as f64 / total.max(1) as f64 * 100.0,
                cum as f64 / total.max(1) as f64 * 100.0,
                names.get(*i).copied().unwrap_or("<unknown>"),
            ));
        }
        out
    }
}

/// Phase attribution for the UDF call prologue (`call-phases` builds only).
///
/// The 2026-08-11 op census established that a UDF call costs ~700 ns of frame
/// machinery against ~60 ns for a builtin dispatch, i.e. ~640 ns of pure setup,
/// and that this — not interpreter dispatch — is the warm-request lever. Sizing
/// the rework needs to know WHICH of the prologue's phases owns that budget, so
/// this accumulates nanoseconds per phase across every call.
///
/// The clock itself is not free (`Instant::now()` is tens of ns on macOS), so
/// [`CLOCK_CAL_NS`] accumulates the cost of a back-to-back pair of reads on the
/// same path. Subtract `CLOCK_CAL_NS / CALLS` per phase boundary before reading
/// any phase total as a real cost.
#[cfg(feature = "call-phases")]
pub mod call_phases {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub const N: usize = 14;

    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicU64 = AtomicU64::new(0);
    /// Cumulative nanoseconds per phase.
    pub static NS: [AtomicU64; N] = [ZERO; N];
    /// Frames measured (every `execute_function_body` entry).
    pub static CALLS: AtomicU64 = AtomicU64::new(0);
    /// Cumulative cost of one extra clock read per call — the calibration term.
    pub static CLOCK_CAL_NS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_calls() {
        CALLS.fetch_add(1, Relaxed);
    }

    /// Frames that built the `arguments` scope EAGERLY vs took the lazy path
    /// (Lever A). Phase 4 is 338 ns/frame on live Preside, so which side of this
    /// branch the real workload sits on decides whether the fix is "make eager
    /// cheaper" or "make more calls lazy".
    pub static ARGS_EAGER: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_LAZY: AtomicU64 = AtomicU64::new(0);
    /// Frames whose `Return` arm did the component-method `this`/`variables`
    /// write-back (phase 7's expensive branch) vs those that skipped it.
    pub static RET_THIS_WRITEBACK: AtomicU64 = AtomicU64::new(0);
    pub static RET_PLAIN: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_calls_args(eager: bool) {
        if eager { ARGS_EAGER.fetch_add(1, Relaxed) } else { ARGS_LAZY.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_ret(has_this: bool) {
        if has_this { RET_THIS_WRITEBACK.fetch_add(1, Relaxed) } else { RET_PLAIN.fetch_add(1, Relaxed) };
    }

    pub fn branch_report() -> String {
        let g = |c: &AtomicU64| c.load(Relaxed);
        let (e, l) = (g(&ARGS_EAGER), g(&ARGS_LAZY));
        let (t, pl) = (g(&RET_THIS_WRITEBACK), g(&RET_PLAIN));
        format!(
            "--- call-path branch split ---\n\
             arguments scope eager:  {:>12}  ({:.1}%)\n\
             arguments scope lazy:   {:>12}  ({:.1}%)\n\
             Return with this-wb:    {:>12}  ({:.1}%)\n\
             Return plain:           {:>12}  ({:.1}%)",
            e, e as f64 / (e + l).max(1) as f64 * 100.0,
            l, l as f64 / (e + l).max(1) as f64 * 100.0,
            t, t as f64 / (t + pl).max(1) as f64 * 100.0,
            pl, pl as f64 / (t + pl).max(1) as f64 * 100.0,
        )
    }

    #[inline]
    pub fn add(phase: usize, ns: u64) {
        if let Some(c) = NS.get(phase) {
            c.fetch_add(ns, Relaxed);
        }
    }

    pub fn report(labels: &[&str]) -> String {
        let calls = CALLS.load(Relaxed).max(1);
        let cal = CLOCK_CAL_NS.load(Relaxed) as f64 / calls as f64;
        let tot: u64 = NS.iter().map(|c| c.load(Relaxed)).sum();
        let mut out = format!(
            "--- UDF call-prologue phases: {} frames, {:.1} ns/frame measured \
             (clock read ≈ {:.1} ns, already subtracted below) ---",
            calls,
            tot as f64 / calls as f64,
            cal
        );
        for (i, label) in labels.iter().enumerate() {
            let raw = NS[i].load(Relaxed) as f64 / calls as f64;
            let net = (raw - cal).max(0.0);
            out.push_str(&format!("\n{:>9.1} ns/frame  {}", net, label));
        }
        out
    }
}

/// Call-site attribution for `CfmlStruct` construction (`alloc-sizing` builds
/// only). Every constructor in the chain (`new`/`new_untracked` and their thin
/// wrappers) carries `#[track_caller]`, so `Location::caller()` here resolves
/// to the real VM/stdlib call site, not the wrapper.
#[cfg(feature = "alloc-sizing")]
pub mod alloc_sites {
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::panic::Location;

    /// site → [tracked, untracked] construction counts (cumulative).
    static SITES: Mutex<Option<HashMap<&'static Location<'static>, [u64; 2]>>> =
        Mutex::new(None);

    #[inline]
    #[track_caller]
    pub fn record(tracked: bool) {
        let loc = Location::caller();
        let mut g = SITES.lock();
        let m = g.get_or_insert_with(HashMap::new);
        m.entry(loc).or_default()[if tracked { 0 } else { 1 }] += 1;
    }

    /// Cumulative top-`n` sites, sorted by total count descending. Diff two
    /// consecutive reports to isolate a single request's shape.
    pub fn report(n: usize) -> String {
        let g = SITES.lock();
        let Some(m) = g.as_ref() else {
            return String::from("--- struct-alloc sites: none recorded ---");
        };
        let mut rows: Vec<_> = m.iter().map(|(l, c)| (*l, *c)).collect();
        rows.sort_by_key(|(_, c)| std::cmp::Reverse(c[0] + c[1]));
        let mut out = String::from("--- struct-alloc sites (cumulative; tracked/untracked) ---");
        for (loc, [t, u]) in rows.into_iter().take(n) {
            out.push_str(&format!(
                "\n{:>12} {:>12}   {}:{}",
                t,
                u,
                loc.file(),
                loc.line()
            ));
        }
        out
    }
}

/// True when `RUSTCFML_COUNTERS=1` — memoized once per process.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RUSTCFML_COUNTERS").map(|v| v == "1").unwrap_or(false))
}
