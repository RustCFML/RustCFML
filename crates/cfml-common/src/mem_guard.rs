//! The hard tier of `--max-memory`: abort the in-flight request that is taking
//! the process over its ceiling.
//!
//! The soft tier (`cli::memory_limit`) refuses NEW requests with 503 once the
//! footprint crosses 85% of the limit, and sheds. That protects a process against
//! a rising tide of ordinary traffic, but it cannot help against the case it was
//! really built for: ONE request that allocates without bound. Nothing arrives to
//! be refused, and the request that is already inside keeps going until the OOM
//! killer arrives — which takes down every other request with it.
//!
//! This module is the counterpart. A watchdog in the CLI polls the footprint; at
//! 95% of the limit it asks for a victim, and the request that has allocated the
//! most since it started is aborted with an uncatchable error, the same class of
//! error as `requestTimeout` (see `is_request_timeout_error` in the VM). Every
//! other request finishes normally.
//!
//! **It never aborts an innocent request.** Selection only considers requests
//! that have themselves allocated more than [`VICTIM_FLOOR`] tracked containers.
//! Memory held by the application scope, the caches, or the allocator's retained
//! pages belongs to no request, and killing an arbitrary one would not free it —
//! so when nothing clears the floor the watchdog reports that and leaves the soft
//! tier (503 + shed) to do its work.
//!
//! **Cost when no limit is configured:** one relaxed atomic load per checkpoint,
//! guarded by [`enabled`]. The per-request odometer is a thread-local counter in
//! [`crate::cycle_gc`] bumped where allocations are already being logged.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Tracked containers a request must have allocated before it can be chosen as
/// the victim. A request under this has not built the heap, so aborting it would
/// free nothing and lose real work. ~100k containers is far above an ordinary
/// page (a Preside homepage logs a few thousand) and far below a runaway.
const VICTIM_FLOOR: u64 = 100_000;

/// Whether a memory limit is configured at all. Everything here short-circuits
/// on this, so a server with no `--max-memory` pays one relaxed load per
/// checkpoint and nothing else.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Set while the footprint is over the hard line. Read by checkpoints so the
/// common (healthy) path does not touch the registry lock.
static PRESSURE: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static ABORTED: AtomicUsize = AtomicUsize::new(0);

/// One in-flight request. Shared between its own thread (which publishes its
/// odometer and polls `abort`) and the watchdog (which reads odometers and sets
/// `abort` on exactly one of them).
#[derive(Debug)]
pub struct Slot {
    pub id: u64,
    /// Tracked containers this request has allocated, published at checkpoints.
    odometer: AtomicU64,
    abort: AtomicBool,
    /// Request line, for the log and the error message.
    label: String,
}

impl Slot {
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn allocations(&self) -> u64 {
        self.odometer.load(Ordering::Relaxed)
    }
}

static REGISTRY: Mutex<Vec<Arc<Slot>>> = Mutex::new(Vec::new());

thread_local! {
    /// The slot for the request running on this thread, if any.
    static CURRENT: std::cell::RefCell<Option<Arc<Slot>>> =
        const { std::cell::RefCell::new(None) };
}

/// Arm the hard tier. Called once by the CLI when `--max-memory` is configured.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Tell the guard whether the process is over the hard line. Called by the
/// watchdog; flipping it to `false` does not un-abort an already-armed victim.
pub fn set_pressure(on: bool) {
    PRESSURE.store(on, Ordering::Release);
}

pub fn aborted_total() -> usize {
    ABORTED.load(Ordering::Relaxed)
}

/// RAII registration for one in-flight request. Dropping it deregisters, so a
/// panicking or early-returning request cannot leave a stale slot behind.
pub struct RequestGuard {
    slot: Arc<Slot>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut reg) = REGISTRY.lock() {
            reg.retain(|s| s.id != self.slot.id);
        }
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}

impl RequestGuard {
    pub fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }
}

/// Register the request about to run on this thread. `None` when no limit is
/// configured, so the caller pays nothing.
pub fn register(label: impl Into<String>) -> Option<RequestGuard> {
    if !enabled() {
        return None;
    }
    let slot = Arc::new(Slot {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        odometer: AtomicU64::new(0),
        abort: AtomicBool::new(false),
        label: label.into(),
    });
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.push(Arc::clone(&slot));
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(Arc::clone(&slot)));
    Some(RequestGuard { slot })
}

/// A safe point. Publishes this request's allocation odometer so the watchdog can
/// compare it with the others, and reports whether this request has been chosen
/// for abort.
///
/// Cheap by construction: without a limit, one relaxed load. With one but no
/// pressure, one more load plus a thread-local read to publish. The registry lock
/// is never taken here.
#[inline]
pub fn checkpoint() -> bool {
    if !enabled() {
        return false;
    }
    CURRENT.with(|c| {
        let b = c.borrow();
        let Some(slot) = b.as_ref() else {
            return false;
        };
        slot.odometer
            .store(crate::cycle_gc::alloc_total(), Ordering::Relaxed);
        PRESSURE.load(Ordering::Acquire) && slot.abort.load(Ordering::Acquire)
    })
}

/// Publish this request's allocation odometer, without checking for abort.
/// Called from the collector's sweep (see `cycle_gc::collect_incremental`), which
/// is the one place that runs at a predictable allocation interval whatever the
/// request is doing.
#[inline]
pub fn publish_progress() {
    if !enabled() {
        return;
    }
    CURRENT.with(|c| {
        if let Some(slot) = c.borrow().as_ref() {
            slot.odometer
                .store(crate::cycle_gc::alloc_total(), Ordering::Relaxed);
        }
    });
}

/// Whether this thread's request has been armed for abort, WITHOUT publishing.
/// For poll sites that are not allocation checkpoints (the blocking calls the
/// request timeout also fires at).
#[inline]
pub fn should_abort() -> bool {
    if !enabled() || !PRESSURE.load(Ordering::Relaxed) {
        return false;
    }
    CURRENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|s| s.abort.load(Ordering::Acquire))
    })
}

/// What the watchdog decided.
pub enum Victim {
    /// This request was armed: id, label, tracked allocations.
    Armed(u64, String, u64),
    /// Over the hard line, but no in-flight request has allocated enough to be
    /// responsible — the memory belongs to the application, the caches, or the
    /// allocator. Aborting anything would free nothing.
    NoneResponsible { in_flight: usize },
    /// A victim is already armed and has not finished unwinding yet.
    AlreadyArmed,
}

/// Choose and arm the largest allocator among the in-flight requests. Called by
/// the watchdog when the footprint is over the hard line.
pub fn arm_victim() -> Victim {
    let reg = match REGISTRY.lock() {
        Ok(r) => r,
        Err(e) => e.into_inner(),
    };
    if reg.iter().any(|s| s.abort.load(Ordering::Acquire)) {
        return Victim::AlreadyArmed;
    }
    let worst = reg
        .iter()
        .filter(|s| s.allocations() >= VICTIM_FLOOR)
        .max_by_key(|s| s.allocations());
    match worst {
        Some(s) => {
            s.abort.store(true, Ordering::Release);
            ABORTED.fetch_add(1, Ordering::Relaxed);
            Victim::Armed(s.id, s.label.clone(), s.allocations())
        }
        None => Victim::NoneResponsible {
            in_flight: reg.len(),
        },
    }
}

/// In-flight request count and the largest odometer among them (diagnostics).
pub fn census() -> (usize, u64) {
    let reg = match REGISTRY.lock() {
        Ok(r) => r,
        Err(e) => e.into_inner(),
    };
    (
        reg.len(),
        reg.iter().map(|s| s.allocations()).max().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the tests: they share the process-wide registry and flags.
    fn reset() {
        ENABLED.store(true, Ordering::Release);
        PRESSURE.store(false, Ordering::Release);
        if let Ok(mut r) = REGISTRY.lock() {
            r.clear();
        }
        CURRENT.with(|c| *c.borrow_mut() = None);
    }

    fn slot_with(label: &str, allocs: u64) -> Arc<Slot> {
        let s = Arc::new(Slot {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            odometer: AtomicU64::new(allocs),
            abort: AtomicBool::new(false),
            label: label.to_string(),
        });
        REGISTRY.lock().unwrap().push(Arc::clone(&s));
        s
    }

    /// The point of the whole module: of several in-flight requests, the one
    /// that built the heap is the one that is stopped.
    #[test]
    fn the_largest_allocator_is_the_victim() {
        let _l = crate::mem_guard::tests::LOCK.lock();
        reset();
        let small = slot_with("/small", 150_000);
        let big = slot_with("/runaway", 9_000_000);
        match arm_victim() {
            Victim::Armed(id, label, allocs) => {
                assert_eq!(id, big.id, "the biggest allocator must be chosen");
                assert_eq!(label, "/runaway");
                assert_eq!(allocs, 9_000_000);
            }
            _ => panic!("a request over the floor must be armed"),
        }
        assert!(big.abort.load(Ordering::Acquire));
        assert!(
            !small.abort.load(Ordering::Acquire),
            "a smaller request must be left alone"
        );
    }

    /// Memory held by the application scope or the allocator belongs to no
    /// request; killing one would lose work and free nothing.
    #[test]
    fn an_innocent_request_is_never_aborted() {
        let _l = crate::mem_guard::tests::LOCK.lock();
        reset();
        let a = slot_with("/one", 12);
        let b = slot_with("/two", VICTIM_FLOOR - 1);
        match arm_victim() {
            Victim::NoneResponsible { in_flight } => assert_eq!(in_flight, 2),
            _ => panic!("no request is over the floor, so none may be aborted"),
        }
        assert!(!a.abort.load(Ordering::Acquire));
        assert!(!b.abort.load(Ordering::Acquire));
    }

    /// Only one victim at a time: the watchdog polls every 250ms, and a large
    /// request takes longer than that to unwind.
    #[test]
    fn a_second_poll_does_not_arm_a_second_victim() {
        let _l = crate::mem_guard::tests::LOCK.lock();
        reset();
        slot_with("/a", 500_000);
        slot_with("/b", 400_000);
        assert!(matches!(arm_victim(), Victim::Armed(..)));
        assert!(matches!(arm_victim(), Victim::AlreadyArmed));
    }

    /// The abort is only visible to the request it was armed on, and only while
    /// the process is actually over the line.
    #[test]
    fn abort_is_seen_only_by_its_own_request_under_pressure() {
        let _l = crate::mem_guard::tests::LOCK.lock();
        reset();
        let guard = register("/mine").expect("enabled");
        slot_with("/other", 8_000_000);
        assert!(!should_abort(), "nothing armed yet");
        assert!(matches!(arm_victim(), Victim::Armed(..)));
        set_pressure(true);
        assert!(
            !should_abort(),
            "the other request was armed, not this one"
        );
        guard.slot().abort.store(true, Ordering::Release);
        assert!(should_abort());
        set_pressure(false);
        drop(guard);
        assert!(!should_abort(), "a deregistered request cannot abort");
    }

    pub(super) static LOCK: Mutex<()> = Mutex::new(());
}
