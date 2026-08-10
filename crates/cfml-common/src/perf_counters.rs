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
         exists FS probes (stats):    {:>12}",
        g(&STRUCT_NEW),
        g(&STRUCT_NEW_UNTRACKED),
        g(&RESOLVE_CALLS),
        g(&RESOLVE_CACHE_HITS),
        g(&RESOLVE_PROBE_WALKS),
        g(&EXISTS_MEMO_HITS),
        g(&EXISTS_FS_PROBES),
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
