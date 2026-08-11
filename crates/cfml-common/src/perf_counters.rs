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
