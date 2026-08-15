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

/// Case-insensitive builtin resolutions served by the lowercased index.
pub static BUILTIN_LOOKUP_CI: AtomicU64 = AtomicU64::new(0);
/// Of those, the ones whose call-site spelling differed from the registry key
/// (`Len()` vs the registry's `len`) — i.e. how often the case-insensitive
/// fallback is taken. Before v0.596 each of these was an unmemoized linear
/// scan over all ~730 builtins plus a fresh `Arc<CfmlFunction>`; sizing that on
/// a real app is what this pair exists to answer.
pub static BUILTIN_LOOKUP_CI_MISCASED: AtomicU64 = AtomicU64::new(0);
/// Resolutions that could NOT use the index because an embedder inserted into
/// the public `builtins` field without calling `refresh_builtin_index()` —
/// these still pay the O(n) scan and should be zero in a healthy process.
pub static BUILTIN_LOOKUP_CI_SCAN: AtomicU64 = AtomicU64::new(0);

/// (`probe-sites` builds only — these sit on the hottest path in the engine,
/// so the shipped binary must not pay even a relaxed atomic for them.)
/// Keyed lookups whose probe carried a PRE-COMPUTED hash — a `Key` or a
/// bytecode `Name` (interned at compile time). These do no hashing at all.
pub static PROBE_PRECOMPUTED: AtomicU64 = AtomicU64::new(0);
/// Keyed lookups that had to fold-hash a `&str`/`String` probe at the call
/// site. The ratio of these two is what says how much of the interned-key
/// lever has actually been captured.
pub static PROBE_HASHED: AtomicU64 = AtomicU64::new(0);

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
    #[allow(unused_mut)]
    let mut out = report_totals(g);
    #[cfg(feature = "alloc-sizing")]
    {
        out.push('\n');
        out.push_str(&alloc_sites::report(60));
    }
    #[cfg(feature = "probe-sites")]
    {
        out.push('\n');
        out.push_str(&probe_sites::totals());
        out.push('\n');
        out.push_str(&probe_sites::report(40));
    }
    #[cfg(feature = "exists-census")]
    {
        out.push('\n');
        out.push_str(&exists_census::report(25));
    }
    out
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
         builtin CI lookups:          {:>12}\n\
           .. miscased (CI fallback): {:>12}\n\
           .. index-stale O(n) scans: {:>12}\n\
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
        g(&BUILTIN_LOOKUP_CI),
        g(&BUILTIN_LOOKUP_CI_MISCASED),
        g(&BUILTIN_LOOKUP_CI_SCAN),
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

    pub const N: usize = 24;

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

    /// Calls whose callee carried a captured scope, so the caller pre-call
    /// (phase 8) had to CLONE the whole captured env map before merging the
    /// caller's locals into it. Paired with the key count, this says whether
    /// that clone is worth removing with a layered scope view.
    pub static ENV_CLONE_CALLS: AtomicU64 = AtomicU64::new(0);
    /// Keys copied by those clones (env size + the caller locals merged in).
    pub static ENV_CLONE_KEYS: AtomicU64 = AtomicU64::new(0);
    /// Calls that passed the caller's locals straight through (no clone).
    pub static ENV_PASSTHROUGH_CALLS: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_env_clone(keys: u64) {
        ENV_CLONE_CALLS.fetch_add(1, Relaxed);
        ENV_CLONE_KEYS.fetch_add(keys, Relaxed);
    }

    #[inline]
    pub fn bump_env_passthrough() {
        ENV_PASSTHROUGH_CALLS.fetch_add(1, Relaxed);
    }

    /// Frames that built the `arguments` scope EAGERLY vs took the lazy path
    /// (Lever A). Phase 4 is 338 ns/frame on live Preside, so which side of this
    /// branch the real workload sits on decides whether the fix is "make eager
    /// cheaper" or "make more calls lazy".
    pub static ARGS_EAGER: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_LAZY: AtomicU64 = AtomicU64::new(0);
    /// Which disjunct of `build_arguments_eager` fired (first-wins order:
    /// template frame, overflow args, body actually references `arguments`).
    /// Decides whether widening laziness is even possible.
    pub static ARGS_EAGER_TEMPLATE: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_EAGER_OVERFLOW: AtomicU64 = AtomicU64::new(0);
    pub static ARGS_EAGER_REFERENCED: AtomicU64 = AtomicU64::new(0);
    /// Frames whose classic-localMode parent write-back diff ran, and of those,
    /// how many actually produced any write-back (phase-7 split item 16 is
    /// 44% of the Return arm — if almost nothing is written back, the whole diff
    /// is avoidable work).
    pub static CLOSURE_WB_SCANNED: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_NONEMPTY: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_KEYS_SCANNED: AtomicU64 = AtomicU64::new(0);
    pub static CLOSURE_WB_KEYS_WRITTEN: AtomicU64 = AtomicU64::new(0);
    /// Frames whose `Return` arm did the component-method `this`/`variables`
    /// write-back (phase 7's expensive branch) vs those that skipped it.
    pub static RET_THIS_WRITEBACK: AtomicU64 = AtomicU64::new(0);
    pub static RET_PLAIN: AtomicU64 = AtomicU64::new(0);
    /// Inside the write-back diff: how many keys the `argument_scope_key_set`
    /// HashSet lowercases and allocates per frame (a FIXED per-frame cost paid
    /// before the key loop), and how many scanned locals actually reach that
    /// set's `contains` probe — i.e. survive the cheap `__`/param/declared
    /// filters. If almost none reach it, building the set eagerly is pure waste.
    pub static ARGSET_BUILT: AtomicU64 = AtomicU64::new(0);
    pub static ARGSET_KEYS: AtomicU64 = AtomicU64::new(0);
    pub static ARGSET_PROBED: AtomicU64 = AtomicU64::new(0);
    /// Worst case seen on any SINGLE frame. The allocation-free probe scans the
    /// `arguments` keys per probing local, so its worst-case work is
    /// probes x argKeys on one frame — where a build-once HashSet would have won.
    /// These maxima say whether that tail is real or hypothetical.
    pub static WB_MAX_ARGKEYS: AtomicU64 = AtomicU64::new(0);
    pub static WB_MAX_PROBES: AtomicU64 = AtomicU64::new(0);
    pub static WB_MAX_PRODUCT: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn bump_calls_args(eager: bool) {
        if eager { ARGS_EAGER.fetch_add(1, Relaxed) } else { ARGS_LAZY.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_ret(has_this: bool) {
        if has_this { RET_THIS_WRITEBACK.fetch_add(1, Relaxed) } else { RET_PLAIN.fetch_add(1, Relaxed) };
    }

    #[inline]
    pub fn bump_eager_reason(template: bool, overflow: bool) {
        if template {
            ARGS_EAGER_TEMPLATE.fetch_add(1, Relaxed);
        } else if overflow {
            ARGS_EAGER_OVERFLOW.fetch_add(1, Relaxed);
        } else {
            ARGS_EAGER_REFERENCED.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn record_closure_wb(keys_scanned: u64, keys_written: u64) {
        CLOSURE_WB_SCANNED.fetch_add(1, Relaxed);
        CLOSURE_WB_KEYS_SCANNED.fetch_add(keys_scanned, Relaxed);
        CLOSURE_WB_KEYS_WRITTEN.fetch_add(keys_written, Relaxed);
        if keys_written > 0 {
            CLOSURE_WB_NONEMPTY.fetch_add(1, Relaxed);
        }
    }

    #[inline]
    pub fn record_wb_argset(keys: u64) {
        ARGSET_BUILT.fetch_add(1, Relaxed);
        ARGSET_KEYS.fetch_add(keys, Relaxed);
    }

    #[inline]
    pub fn bump_wb_argset_probe() {
        ARGSET_PROBED.fetch_add(1, Relaxed);
    }

    /// Per-frame maxima for the tail analysis above.
    pub fn record_wb_frame(argkeys: u64, probes: u64) {
        WB_MAX_ARGKEYS.fetch_max(argkeys, Relaxed);
        WB_MAX_PROBES.fetch_max(probes, Relaxed);
        WB_MAX_PRODUCT.fetch_max(argkeys * probes, Relaxed);
    }

    pub fn branch_report() -> String {
        let g = |c: &AtomicU64| c.load(Relaxed);
        let (e, l) = (g(&ARGS_EAGER), g(&ARGS_LAZY));
        let (t, pl) = (g(&RET_THIS_WRITEBACK), g(&RET_PLAIN));
        format!(
            "--- caller pre-call scope handling (phase 8) ---\n\
             env CLONED (closure callee):  {:>12}\n\
               .. keys copied:             {:>12}\n\
             locals passed through:        {:>12}\n\
             --- call-path branch split ---\n\
             arguments scope eager:  {:>12}  ({:.1}%)\n\
             arguments scope lazy:   {:>12}  ({:.1}%)\n\
             Return with this-wb:    {:>12}  ({:.1}%)\n\
             Return plain:           {:>12}  ({:.1}%)",
            g(&ENV_CLONE_CALLS), g(&ENV_CLONE_KEYS), g(&ENV_PASSTHROUGH_CALLS),
            e, e as f64 / (e + l).max(1) as f64 * 100.0,
            l, l as f64 / (e + l).max(1) as f64 * 100.0,
            t, t as f64 / (t + pl).max(1) as f64 * 100.0,
            pl, pl as f64 / (t + pl).max(1) as f64 * 100.0,
        ) + &format!(
            "\n  eager because template frame:  {:>12}\n\
               eager because overflow args:   {:>12}\n\
               eager because body uses it:    {:>12}\n\
             classic-localMode wb diffs run:  {:>12}\n\
               .. that wrote ANY key:         {:>12}  ({:.1}%)\n\
               .. keys scanned / written:     {:>12} / {}\n\
               .. argset builds / keys lc'd:  {:>12} / {}\n\
               .. locals reaching argset:     {:>12}\n\
               .. WORST single frame: argKeys {:>4}, probes {:>4}, product {:>6}",
            g(&ARGS_EAGER_TEMPLATE),
            g(&ARGS_EAGER_OVERFLOW),
            g(&ARGS_EAGER_REFERENCED),
            g(&CLOSURE_WB_SCANNED),
            g(&CLOSURE_WB_NONEMPTY),
            g(&CLOSURE_WB_NONEMPTY) as f64 / g(&CLOSURE_WB_SCANNED).max(1) as f64 * 100.0,
            g(&CLOSURE_WB_KEYS_SCANNED),
            g(&CLOSURE_WB_KEYS_WRITTEN),
            g(&ARGSET_BUILT),
            g(&ARGSET_KEYS),
            g(&ARGSET_PROBED),
            g(&WB_MAX_ARGKEYS),
            g(&WB_MAX_PROBES),
            g(&WB_MAX_PRODUCT),
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

/// Call-site census for keyed lookups that fold-hash a `&str` at the probe
/// site (`probe-sites` builds only).
///
/// The [`PROBE_PRECOMPUTED`]/[`PROBE_HASHED`] pair says how much of the
/// interned-key lever is captured; this says WHICH call sites still owe. A
/// site listed here is one `&str` probe away from being free — convert it to
/// take a `Name`/`Key` and its whole count moves to the precomputed column.
#[cfg(feature = "probe-sites")]
pub mod probe_sites {
    /// Totals for the two probe classes, reported alongside the site list.
    pub fn totals() -> String {
        use super::{Relaxed, PROBE_HASHED, PROBE_PRECOMPUTED};
        use std::sync::atomic::Ordering;
        let _ = Relaxed;
        let (p, h) = (
            PROBE_PRECOMPUTED.load(Ordering::Relaxed),
            PROBE_HASHED.load(Ordering::Relaxed),
        );
        let tot = (p + h).max(1);
        format!(
            "--- key probes: {p} precomputed ({:.1}%) / {h} hashed at site ({:.1}%) ---",
            p as f64 / tot as f64 * 100.0,
            h as f64 / tot as f64 * 100.0
        )
    }

    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::panic::Location;

    static SITES: Mutex<Option<HashMap<&'static Location<'static>, u64>>> = Mutex::new(None);

    #[inline]
    #[track_caller]
    pub fn record() {
        let loc = Location::caller();
        let mut g = SITES.lock();
        *g.get_or_insert_with(HashMap::new).entry(loc).or_default() += 1;
    }

    /// Cumulative top-`n` hashing probe sites, most frequent first.
    pub fn report(n: usize) -> String {
        let g = SITES.lock();
        let Some(m) = g.as_ref() else {
            return String::from("--- hashed probe sites: none recorded ---");
        };
        let mut rows: Vec<_> = m.iter().map(|(l, c)| (*l, *c)).collect();
        rows.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        let total: u64 = rows.iter().map(|(_, c)| c).sum();
        let mut out = format!(
            "--- hashed probe sites (cumulative; {total} total) ---"
        );
        for (loc, c) in rows.into_iter().take(n) {
            out.push_str(&format!("\n{:>12}  {:>5.1}%   {}:{}", c,
                c as f64 / total as f64 * 100.0, loc.file(), loc.line()));
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

/// Existence-probe outcome census (`exists-census` builds only) — the sizing
/// instrument for known-issues §45.
///
/// The shipped [`EXISTS_MEMO_HITS`] / [`EXISTS_FS_PROBES`] pair says the memo
/// takes more real syscalls than it serves hits, but it cannot say *what to
/// build*, because `request_exists_cache` holds POSITIVES ONLY: a negative
/// probe can never be a hit, so it is invisible to a hit-rate. The question
/// that decides the design is how many probes are REPEATS of a path already
/// probed — recoverable by a cache — versus genuine first looks, which no cache
/// can remove. This splits every probe six ways to answer exactly that:
///
/// | class | what would recover it |
/// |---|---|
/// | `pos_first` / `neg_first` | nothing — irreducible filesystem truth |
/// | `pos_repeat_req` | **only reachable after a wholesale memo clear** — measures the cost of the blanket `.clear()` |
/// | `pos_repeat_xreq` | a cross-request positive layer (already measured dead: −1% homepage, 0% admin) |
/// | `neg_repeat_req` | a request-scoped negative memo |
/// | `neg_repeat_xreq` | an application-lifetime negative memo |
///
/// Retains every probed path in a `Mutex`'d map (that is also how it reports
/// the distinct-path count, i.e. what a negative cache would have to hold), so
/// this is a sizing build only.
#[cfg(feature = "exists-census")]
pub mod exists_census {
    use super::{AtomicU64, Relaxed};
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// What we have already seen for one (path, probe-kind) pair.
    #[derive(Default)]
    struct Seen {
        /// Epoch of the most recent probe that found the path.
        last_pos_epoch: Option<u64>,
        /// Epoch of the most recent probe that did NOT find the path.
        last_neg_epoch: Option<u64>,
        /// Total probes that did not find it — ranks the worst offenders.
        neg_probes: u64,
    }

    #[derive(Default)]
    struct Tally {
        memo_hits: u64,
        pos_first: u64,
        pos_repeat_req: u64,
        pos_repeat_xreq: u64,
        neg_first: u64,
        neg_repeat_req: u64,
        neg_repeat_xreq: u64,
    }

    static STATE: Mutex<Option<(HashMap<(String, u8), Seen>, Tally)>> = Mutex::new(None);

    /// Monotonic request epoch. The VM takes one per `Vm` construction, which in
    /// serve mode is one per request, and passes it back into [`record`] — so
    /// "same request" is decided by the caller rather than by a thread-local,
    /// and stays correct under concurrent requests.
    static EPOCH: AtomicU64 = AtomicU64::new(0);

    /// Claim the next request epoch.
    pub fn next_epoch() -> u64 {
        EPOCH.fetch_add(1, Relaxed)
    }

    /// Names of builtins that retired the cached negatives, with counts — the
    /// list that decides which creators are worth attributing to a specific path
    /// instead of firing the coarse global retirement.
    static CREATORS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

    /// One builtin call that retired the negatives.
    pub fn record_creator(name_lower: &str) {
        let mut g = CREATORS.lock();
        let m = g.get_or_insert_with(HashMap::new);
        *m.entry(name_lower.to_string()).or_insert(0) += 1;
    }

    /// A probe answered from the memo without touching the filesystem.
    pub fn record_memo_hit() {
        let mut g = STATE.lock();
        let (_, t) = g.get_or_insert_with(Default::default);
        t.memo_hits += 1;
    }

    /// Classify one real filesystem probe of `path` under `bit`.
    pub fn record(path: &str, bit: u8, found: bool, epoch: u64) {
        let mut g = STATE.lock();
        let (m, t) = g.get_or_insert_with(Default::default);
        // `raw_entry` is unstable, so the miss path pays one allocation for the
        // key. This is a sizing build; correctness of the count is what matters.
        let seen = m.entry((path.to_string(), bit)).or_default();
        if found {
            match seen.last_pos_epoch {
                // Only reachable if something wiped the memo mid-request: the
                // memo would otherwise have served this as a hit.
                Some(e) if e == epoch => t.pos_repeat_req += 1,
                Some(_) => t.pos_repeat_xreq += 1,
                None => t.pos_first += 1,
            }
            seen.last_pos_epoch = Some(epoch);
        } else {
            match seen.last_neg_epoch {
                Some(e) if e == epoch => t.neg_repeat_req += 1,
                Some(_) => t.neg_repeat_xreq += 1,
                None => t.neg_first += 1,
            }
            seen.last_neg_epoch = Some(epoch);
            seen.neg_probes += 1;
        }
    }

    /// The census block, plus the top-`n` most-re-probed absent paths.
    pub fn report(n: usize) -> String {
        let g = STATE.lock();
        let Some((m, t)) = g.as_ref() else {
            return String::from("--- exists census: nothing recorded ---");
        };
        let epochs = EPOCH.load(Relaxed).max(1);
        let probes = t.pos_first
            + t.pos_repeat_req
            + t.pos_repeat_xreq
            + t.neg_first
            + t.neg_repeat_req
            + t.neg_repeat_xreq;
        let neg = t.neg_first + t.neg_repeat_req + t.neg_repeat_xreq;
        // What each candidate cache would actually have removed.
        let recoverable = t.neg_repeat_req + t.neg_repeat_xreq + t.pos_repeat_req + t.pos_repeat_xreq;
        let pct = |v: u64| {
            if probes == 0 {
                0.0
            } else {
                v as f64 * 100.0 / probes as f64
            }
        };
        let distinct_absent = m.values().filter(|s| s.last_neg_epoch.is_some()).count();
        let mut out = format!(
            "--- exists census ({} request epochs; {} probes, {} memo hits) ---\n\
             {:>12}  positive, first ever          {:>5.1}%\n\
             {:>12}  positive, repeat SAME request {:>5.1}%  <- cost of the wholesale memo clear\n\
             {:>12}  positive, repeat later req    {:>5.1}%  <- cross-request positive layer (measured dead)\n\
             {:>12}  negative, first ever          {:>5.1}%  <- irreducible\n\
             {:>12}  negative, repeat SAME request {:>5.1}%  <- request-scoped negative memo\n\
             {:>12}  negative, repeat later req    {:>5.1}%  <- application-lifetime negative memo\n\
             {:>12}  negative probes total         {:>5.1}%\n\
             {:>12}  RECOVERABLE by some cache     {:>5.1}%\n\
             {:>12}  distinct absent paths (a negative cache must hold these)\n\
             {:>12.1}  probes per request epoch",
            epochs,
            probes,
            t.memo_hits,
            t.pos_first,
            pct(t.pos_first),
            t.pos_repeat_req,
            pct(t.pos_repeat_req),
            t.pos_repeat_xreq,
            pct(t.pos_repeat_xreq),
            t.neg_first,
            pct(t.neg_first),
            t.neg_repeat_req,
            pct(t.neg_repeat_req),
            t.neg_repeat_xreq,
            pct(t.neg_repeat_xreq),
            neg,
            pct(neg),
            recoverable,
            pct(recoverable),
            distinct_absent,
            probes as f64 / epochs as f64,
        );
        let mut rows: Vec<_> = m
            .iter()
            .filter(|(_, s)| s.neg_probes > 1)
            .map(|((p, b), s)| (s.neg_probes, *b, p.as_str()))
            .collect();
        rows.sort_by_key(|(c, _, _)| std::cmp::Reverse(*c));
        {
            let g = CREATORS.lock();
            if let Some(m) = g.as_ref() {
                let mut rows: Vec<_> = m.iter().map(|(k, v)| (*v, k.as_str())).collect();
                rows.sort_by_key(|(c, _)| std::cmp::Reverse(*c));
                out.push_str("\n--- builtins that RETIRED the cached negatives (count, name) ---");
                for (c, n) in rows.into_iter().take(20) {
                    out.push_str(&format!("\n{:>12}  {}", c, n));
                }
            }
        }
        out.push_str("\n--- most-re-probed ABSENT paths (probes, kind bit, path) ---");
        for (c, b, p) in rows.into_iter().take(n) {
            out.push_str(&format!("\n{:>12}  {}  {}", c, b, p));
        }
        out
    }
}

/// True when `RUSTCFML_COUNTERS=1` — memoized once per process.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RUSTCFML_COUNTERS").map(|v| v == "1").unwrap_or(false))
}
