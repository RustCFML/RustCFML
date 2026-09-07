//! Process-wide memory limit for `--serve` (`--max-memory 1.5G|auto`).
//!
//! Why this exists. A JVM engine runs under `-Xmx`, so a container that says
//! "2 GB" can hand the process 1.5 GB and know it will never be OOM-killed. Our
//! footprint is live data plus the pages mimalloc keeps around after a peak, and
//! nothing bounded it: one Wheels suite request peaked at 6.4 GB and three in a
//! row read 13.9 GB, most of it reclaimable pages. This module is the bound —
//! and it fails better than `-Xmx`, which throws `OutOfMemoryError` into
//! whichever thread allocated last after minutes of GC thrash.
//!
//! What it measures: the **real physical footprint**, from mimalloc's process
//! statistics — the number the container's OOM killer uses — not our own
//! tracked-node count.
//!
//! The soft tier (this module). Above `soft` (default 85% of the limit) the
//! server stops ADMITTING new requests — **503 + `Retry-After`**, so a load
//! balancer or orchestrator sees a healthy back-pressure signal — and sheds:
//! it runs the collector's cross-request sweep and asks mimalloc to return
//! retained pages to the OS (`mi_collect(true)`). In-flight requests finish
//! normally; admission reopens the moment the footprint is back under. Shedding
//! is rate-limited so a sustained overload does not turn into a sweep storm.
//!
//! The hard tier ([`cfml_common::mem_guard`], armed from here). The soft tier
//! cannot help against the case it was really built for: ONE request that
//! allocates without bound. Nothing new arrives to be refused, and the request
//! already inside runs until the OOM killer takes the whole process down. So a
//! watchdog thread polls the footprint, and above `hard` (95% of the limit) it
//! aborts the in-flight request that has allocated the most — with an
//! uncatchable error, like `requestTimeout` — leaving every other request
//! running. It never aborts a request that has not itself built the heap: when
//! the memory belongs to the application scope or the allocator, no request is
//! responsible and only the soft tier applies.
//!
//! `auto` reads the cgroup limit (v2 `memory.max`, v1 `memory.limit_in_bytes`)
//! and takes 75% of it, so a containerised deployment gets the right behaviour
//! with no flag at all; with no cgroup limit, `auto` leaves the limit off.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Fraction of the limit at which admission closes and shedding starts.
const SOFT_FRACTION: f64 = 0.85;
/// Fraction of a cgroup limit `auto` hands to this process.
const AUTO_FRACTION: f64 = 0.75;
/// Minimum gap between two shed passes: a sweep over a large persistent set is
/// hundreds of milliseconds, and a burst of refused requests must not each pay
/// for one.
const SHED_INTERVAL: Duration = Duration::from_secs(2);
/// Hysteresis: once shedding, admission reopens only below this fraction of the
/// soft limit, so a process sitting right at the line does not flap 503/200 on
/// alternate requests (measured on Preside with a limit ~1.15x its steady state).
const REOPEN_FRACTION: f64 = 0.95;
/// `Retry-After` handed to refused requests, in seconds.
pub const RETRY_AFTER_SECS: u32 = 2;
/// Fraction of the limit at which the hard tier aborts the largest in-flight
/// allocator. Below the ceiling the operator named, so the abort itself — an
/// error value, a stack trace, a response — has room to run, and above the soft
/// line so back-pressure is always tried first.
const HARD_FRACTION: f64 = 0.95;
/// How often the watchdog reads the footprint. A runaway request can allocate a
/// lot in 250ms, which is why `hard` leaves 5% of headroom; polling faster costs
/// a syscall per tick for no benefit on a process that is not in trouble.
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(250);

/// A configured limit. `max` is the hard ceiling the operator named; `soft` is
/// where admission closes.
#[derive(Debug, Clone, Copy)]
pub struct MemoryLimit {
    pub max: u64,
    pub soft: u64,
    /// Where the hard tier aborts the largest in-flight allocator.
    pub hard: u64,
}

impl MemoryLimit {
    pub fn new(max: u64) -> Self {
        Self {
            max,
            soft: (max as f64 * SOFT_FRACTION) as u64,
            hard: (max as f64 * HARD_FRACTION) as u64,
        }
    }
}

/// Parse `--max-memory`'s argument: a byte size (`1.5G`, `1536M`, `2048m`,
/// `1073741824`) or `auto`. `Ok(None)` means "no limit" (`auto` outside a
/// cgroup, or `0`/`off`).
pub fn parse_max_memory(arg: &str) -> Result<Option<MemoryLimit>, String> {
    let a = arg.trim();
    if a.eq_ignore_ascii_case("off") || a == "0" {
        return Ok(None);
    }
    if a.eq_ignore_ascii_case("auto") {
        return Ok(cgroup_limit_bytes().map(|b| MemoryLimit::new((b as f64 * AUTO_FRACTION) as u64)));
    }
    parse_size(a)
        .map(|b| Some(MemoryLimit::new(b)))
        .ok_or_else(|| format!("--max-memory: cannot parse '{arg}' (expected e.g. 1.5G, 1536M, or auto)"))
}

/// `1.5G` → bytes. Suffixes K/M/G/T (case-insensitive, optional trailing B or
/// iB); a bare number is bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let lower = lower
        .strip_suffix("ib")
        .or_else(|| lower.strip_suffix('b'))
        .unwrap_or(&lower);
    let (num, mult): (&str, f64) = match lower.chars().last()? {
        'k' => (&lower[..lower.len() - 1], 1024.0),
        'm' => (&lower[..lower.len() - 1], 1024.0 * 1024.0),
        'g' => (&lower[..lower.len() - 1], 1024.0 * 1024.0 * 1024.0),
        't' => (&lower[..lower.len() - 1], 1024.0 * 1024.0 * 1024.0 * 1024.0),
        _ => (lower, 1.0),
    };
    let n: f64 = num.trim().parse().ok()?;
    if !(n > 0.0) {
        return None;
    }
    Some((n * mult) as u64)
}

/// The cgroup memory limit this process runs under, if any. Linux only; `None`
/// elsewhere and when the cgroup says `max` (unlimited).
pub fn cgroup_limit_bytes() -> Option<u64> {
    for path in ["/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory/memory.limit_in_bytes"] {
        if let Ok(s) = std::fs::read_to_string(path) {
            let s = s.trim();
            if s == "max" {
                return None;
            }
            if let Ok(n) = s.parse::<u64>() {
                // cgroup v1 reports a huge sentinel when unlimited.
                if n > 0 && n < (1u64 << 60) {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Current physical footprint in bytes — the number the OS or container acts
/// on, in this order of preference:
///
/// * Linux in a cgroup: `memory.current` (v2) / `memory.usage_in_bytes` (v1),
///   exactly what the OOM killer compares against the limit.
/// * Linux otherwise: resident pages from `/proc/self/statm`.
/// * macOS: `proc_pid_rusage` → `ri_phys_footprint`, the figure Activity
///   Monitor and `vmmap -summary` report.
/// * Anything else: mimalloc's `current_rss` when the allocator is compiled in.
///
/// Why not mimalloc's RSS everywhere: on macOS it counted pages the allocator
/// had already released with `MADV_FREE` (reclaimed lazily by the kernel) and
/// read 763M against a real footprint of 585M — the server refused traffic it
/// had room for, and shedding appeared not to work because the meter did not
/// move. `None` means no metric is available and the limit cannot be enforced.
pub fn footprint_bytes() -> Option<u64> {
    os_footprint().or_else(mimalloc_rss)
}

/// Name of the metric in use, for the startup banner.
pub fn footprint_source() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        if cgroup_limit_bytes().is_some() {
            return "cgroup memory.current";
        }
        return "/proc/self/statm resident";
    }
    #[cfg(target_os = "macos")]
    {
        return "phys_footprint";
    }
    #[allow(unreachable_code)]
    "mimalloc rss"
}

#[cfg(target_os = "linux")]
fn os_footprint() -> Option<u64> {
    if cgroup_limit_bytes().is_some() {
        for path in ["/sys/fs/cgroup/memory.current", "/sys/fs/cgroup/memory/memory.usage_in_bytes"] {
            if let Some(n) = std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse::<u64>().ok()) {
                return Some(n);
            }
        }
    }
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf with a valid name has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        return None;
    }
    Some(resident_pages * page as u64)
}

#[cfg(target_os = "macos")]
fn os_footprint() -> Option<u64> {
    // <libproc.h> / <sys/resource.h>: RUSAGE_INFO_V2 carries ri_phys_footprint.
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV2 {
        ri_uuid: [u8; 16],
        ri_user_time: u64,
        ri_system_time: u64,
        ri_pkg_idle_wkups: u64,
        ri_interrupt_wkups: u64,
        ri_pageins: u64,
        ri_wired_size: u64,
        ri_resident_size: u64,
        ri_phys_footprint: u64,
        ri_proc_start_abstime: u64,
        ri_proc_exit_abstime: u64,
        ri_child_user_time: u64,
        ri_child_system_time: u64,
        ri_child_pkg_idle_wkups: u64,
        ri_child_interrupt_wkups: u64,
        ri_child_pageins: u64,
        ri_child_elapsed_abstime: u64,
        ri_diskio_bytesread: u64,
        ri_diskio_byteswritten: u64,
    }
    const RUSAGE_INFO_V2: i32 = 2;
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV2) -> i32;
    }
    let mut info = RusageInfoV2::default();
    // SAFETY: `info` is a correctly laid-out, writable RUSAGE_INFO_V2 buffer for
    // our own pid; the call writes it and returns 0 on success.
    let rc = unsafe { proc_pid_rusage(std::process::id() as i32, RUSAGE_INFO_V2, &mut info) };
    if rc == 0 && info.ri_phys_footprint > 0 {
        Some(info.ri_phys_footprint)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn os_footprint() -> Option<u64> {
    None
}

/// mimalloc's own `current_rss`. Fallback only — see [`footprint_bytes`].
fn mimalloc_rss() -> Option<u64> {
    #[cfg(feature = "mimalloc")]
    {
        let mut elapsed = 0usize;
        let mut user = 0usize;
        let mut system = 0usize;
        let mut rss = 0usize;
        let mut peak = 0usize;
        let mut commit = 0usize;
        let mut peak_commit = 0usize;
        let mut faults = 0usize;
        // SAFETY: plain out-parameters, all valid for writes for the call.
        unsafe {
            libmimalloc_sys::mi_process_info(
                &mut elapsed,
                &mut user,
                &mut system,
                &mut rss,
                &mut peak,
                &mut commit,
                &mut peak_commit,
                &mut faults,
            );
        }
        Some(rss as u64)
    }
    #[cfg(not(feature = "mimalloc"))]
    {
        None
    }
}

/// Ask the allocator to return retained pages to the OS. Targeted — only called
/// while shedding — because doing this eagerly on every request measured −31%
/// rps (GH #354).
pub fn release_retained_memory() {
    #[cfg(feature = "mimalloc")]
    // SAFETY: no preconditions; safe to call from any thread at any time.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
}

/// Live enforcement state, shared by the request handlers.
pub struct Enforcer {
    limit: MemoryLimit,
    /// True while admission is closed; flipping it logs once in each direction.
    shedding: AtomicBool,
    /// `Instant`-as-millis of the last shed pass (0 = never).
    last_shed_ms: AtomicU64,
    epoch: Instant,
    /// Requests refused so far (observability).
    refused: AtomicU64,
}

impl Enforcer {
    pub fn new(limit: MemoryLimit) -> Self {
        Self {
            limit,
            shedding: AtomicBool::new(false),
            last_shed_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            refused: AtomicU64::new(0),
        }
    }

    pub fn limit(&self) -> MemoryLimit {
        self.limit
    }

    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// Called before a request is admitted. `Some(footprint)` means REFUSE
    /// (respond 503); the caller need do nothing else — shedding has already
    /// been kicked if it was due.
    pub fn check_admission(&self) -> Option<u64> {
        let fp = footprint_bytes()?;
        let shedding = self.shedding.load(Ordering::Acquire);
        let threshold = if shedding {
            (self.limit.soft as f64 * REOPEN_FRACTION) as u64
        } else {
            self.limit.soft
        };
        if fp < threshold {
            if self.shedding.swap(false, Ordering::AcqRel) {
                eprintln!(
                    "[memory] footprint {} back under the soft limit {} — admitting requests again ({} refused)",
                    human(fp),
                    human(self.limit.soft),
                    self.refused.load(Ordering::Relaxed)
                );
            }
            return None;
        }
        if !self.shedding.swap(true, Ordering::AcqRel) {
            eprintln!(
                "[memory] footprint {} over the soft limit {} (max {}) — refusing new requests with 503 and shedding",
                human(fp),
                human(self.limit.soft),
                human(self.limit.max)
            );
        }
        self.refused.fetch_add(1, Ordering::Relaxed);
        self.shed();
        Some(fp)
    }

    /// Called at request end so a request that pushed the process over the
    /// soft limit triggers shedding immediately, rather than the next arrival
    /// paying for it with a 503.
    pub fn on_request_end(&self) {
        if let Some(fp) = footprint_bytes() {
            if fp >= self.limit.soft {
                self.shed();
            }
        }
    }

    /// One shed pass, rate-limited: sweep the collector's cross-request set,
    /// then return retained pages.
    /// Shed from outside the request path (the watchdog). Rate-limited exactly
    /// like the request-path shed it delegates to.
    pub fn shed_now(&self) {
        self.shed();
    }

    fn shed(&self) {
        let now = self.epoch.elapsed().as_millis() as u64;
        let last = self.last_shed_ms.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < SHED_INTERVAL.as_millis() as u64 {
            return;
        }
        if self
            .last_shed_ms
            .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return; // a concurrent caller is doing it
        }
        let before = footprint_bytes();
        let reclaimed = cfml_common::cycle_gc::sweep_persistent();
        release_retained_memory();
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            eprintln!(
                "[memory] shed: sweep reclaimed {} node(s); footprint {} -> {}",
                reclaimed,
                before.map(human).unwrap_or_default(),
                footprint_bytes().map(human).unwrap_or_default()
            );
        }
    }
}

/// The process-wide enforcer. One per process, because the limit is a property
/// of the process (it is what the container will kill), and because the two
/// places that consult it — request admission in the HTTP handler and the
/// request-end hook deep in the run loop — share no state otherwise.
static ENFORCER: std::sync::OnceLock<Enforcer> = std::sync::OnceLock::new();

/// Install the limit for this process. Later calls are ignored (first wins).
pub fn install(limit: MemoryLimit) {
    if ENFORCER.set(Enforcer::new(limit)).is_err() {
        return;
    }
    cfml_common::mem_guard::enable();
    start_watchdog(limit);
}

/// The hard tier's watchdog: poll the footprint, and above `hard` abort the
/// in-flight request that has allocated the most.
///
/// A thread rather than a check on the request path, because the request that
/// needs stopping is by definition not returning: nothing on the serving path
/// runs while it allocates. It exits with the process.
fn start_watchdog(limit: MemoryLimit) {
    std::thread::Builder::new()
        .name("rustcfml-memory-watchdog".to_string())
        .spawn(move || loop {
            std::thread::sleep(WATCHDOG_INTERVAL);
            let Some(fp) = footprint_bytes() else { continue };
            if fp < limit.hard {
                cfml_common::mem_guard::set_pressure(false);
                continue;
            }
            // Order matters: make the abort visible before arming, so the victim
            // cannot miss its own flag by polling between the two stores.
            cfml_common::mem_guard::set_pressure(true);
            match cfml_common::mem_guard::arm_victim() {
                cfml_common::mem_guard::Victim::Armed(id, label, allocs) => {
                    eprintln!(
                        "[max-memory] footprint {} is over the hard limit {} of {} — \
                         aborting request #{} [{}], the largest allocator ({} tracked \
                         containers). Other requests continue.",
                        human(fp),
                        human(limit.hard),
                        human(limit.max),
                        id,
                        label,
                        allocs
                    );
                }
                cfml_common::mem_guard::Victim::NoneResponsible { in_flight } => {
                    eprintln!(
                        "[max-memory] footprint {} is over the hard limit {} of {}, but \
                         none of the {} in-flight request(s) has allocated enough to be \
                         responsible — the memory is held by the application, the caches \
                         or the allocator. Not aborting anything; shedding instead.",
                        human(fp),
                        human(limit.hard),
                        human(limit.max),
                        in_flight
                    );
                    if let Some(e) = enforcer() {
                        e.shed_now();
                    }
                }
                cfml_common::mem_guard::Victim::AlreadyArmed => {}
            }
        })
        .ok();
}

/// The installed enforcer, if `--max-memory` was given.
pub fn enforcer() -> Option<&'static Enforcer> {
    ENFORCER.get()
}

/// `1610612736` → `1.5G`.
pub fn human(bytes: u64) -> String {
    const G: f64 = 1024.0 * 1024.0 * 1024.0;
    const M: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= G {
        format!("{:.2}G", b / G)
    } else if b >= M {
        format!("{:.0}M", b / M)
    } else {
        format!("{}K", bytes / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_with_and_without_suffixes() {
        assert_eq!(parse_size("1.5G"), Some(1_610_612_736));
        assert_eq!(parse_size("1536M"), Some(1_610_612_736));
        assert_eq!(parse_size("1536MB"), Some(1_610_612_736));
        assert_eq!(parse_size("2gib"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1073741824"), Some(1_073_741_824));
        assert_eq!(parse_size("512k"), Some(512 * 1024));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("lots"), None);
    }

    #[test]
    fn soft_limit_is_85_percent() {
        let l = MemoryLimit::new(1_000_000_000);
        assert_eq!(l.soft, 850_000_000);
        assert!(matches!(parse_max_memory("off"), Ok(None)));
        assert!(matches!(parse_max_memory("0"), Ok(None)));
        assert!(parse_max_memory("nope").is_err());
        assert_eq!(parse_max_memory("1G").unwrap().unwrap().max, 1 << 30);
    }

    #[test]
    fn footprint_reads_and_is_plausible() {
        let fp = footprint_bytes().expect("some footprint metric is available");
        assert!(fp > 1024 * 1024, "a running test process is bigger than 1MB: {fp}");
        assert!(fp < 64 * 1024 * 1024 * 1024, "and smaller than 64GB: {fp}");
        // The OS metric, where we have one, must not exceed mimalloc's RSS by
        // more than noise: RSS counts everything the footprint does and more.
        if let (Some(os), Some(rss)) = (os_footprint(), mimalloc_rss()) {
            assert!(os <= rss + 64 * 1024 * 1024, "os={os} rss={rss}");
        }
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(1_610_612_736), "1.50G");
        assert_eq!(human(300 * 1024 * 1024), "300M");
    }
}
