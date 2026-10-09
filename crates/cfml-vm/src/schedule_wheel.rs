//! One timer thread for ALL scheduled tasks, plus a small pool of runners.
//!
//! Each `_schedule` used to get a thread of its own that slept toward its
//! deadline in 50 ms steps so a `cancel()` would be noticed promptly. That cost
//! one thread per schedule AND a wake-up every 50 ms per thread, whether or not
//! anything was due.
//!
//! Neither reference engine does that. Lucee gives each task a thread but waits
//! on a monitor for the WHOLE remaining interval (`ScheduledTaskThread.sleepEL`
//! → `Object.wait(millis)`), waking it with `notify()` on cancel. BoxLang has no
//! per-task thread at all — `ScheduledTask.start()` hands the work to a shared
//! `ScheduledThreadPoolExecutor`, whose `DelayedWorkQueue` parks one leader
//! thread in `Condition.awaitNanos()` on the earliest deadline only.
//!
//! This is BoxLang's shape:
//!
//! * one **timer** thread owns a deadline-ordered heap and parks on the
//!   earliest entry — indefinitely when the heap is empty, so an idle server
//!   with any number of schedules costs one parked thread and no periodic
//!   wake-ups. [`wake`] makes `cancel()` immediate rather than up to 50 ms late.
//! * a pool of **runner** threads takes due tasks off a queue, runs the body and
//!   re-arms the schedule. Runners are REUSED between firings and park on a
//!   condvar when there is nothing to do.
//!
//! Two things measured on a live Preside server during this rewrite, both worth
//! knowing before "simplifying" either one:
//!
//! 1. Spawning a runner per firing cost MORE than the polling it replaced
//!    (3.9% → 5.35% of a core): a thread is not cheap to create here, each
//!    reserving 64 MB of stack address space, and Preside's heartbeats fire
//!    every few hundred milliseconds. Hence the pool.
//! 2. Sharing one `mpsc::Receiver` between runners behind a `Mutex` was worse
//!    still: `recv()` blocks while HOLDING the lock, so one runner waits and
//!    every other spins on the mutex. Hence the explicit queue and condvar
//!    below — `pending` is drained under the lock, which is released before the
//!    body runs.

use crate::async_kernel::{ScheduleRelayGuard, ScheduledPermits};
use crate::{ThreadHandle, ThreadResult, ThreadSeed, ThreadSpawnFn};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Ceiling on runner threads. Past this a due task waits for one to free up,
/// which is what a `ScheduledThreadPoolExecutor` does at its core pool size.
const MAX_RUNNERS: usize = 64;

/// Everything a schedule needs to fire again, held by the timer while it waits
/// and moved to a runner when it is due.
struct Task {
    seed: ThreadSeed,
    spawn: ThreadSpawnFn,
    /// `(period_ms, fixed_rate)`; `None` for a one-shot.
    period: Option<(i64, bool)>,
    permits: Option<ScheduledPermits>,
    cancel: Arc<AtomicBool>,
    tx: Sender<ThreadResult>,
    /// The start of the run this schedule is phased from — fixed-RATE advances
    /// from here, so a body that overran does not produce a catch-up burst.
    run_at: Instant,
    /// Still waiting on the first run. Only a ONE-SHOT resolves its future on
    /// that run; see `run_once`.
    first: bool,
    /// Census of live schedules (`RUSTCFML_GC_DEBUG`). Each pins a
    /// `ThreadSeed`, so the count is also a memory figure.
    _guard: ScheduleRelayGuard,
}

#[derive(Default)]
struct State {
    /// Min-heap of `(due, id)` — `Reverse` because `BinaryHeap` is a max-heap.
    heap: BinaryHeap<Reverse<(Instant, u64)>>,
    /// Schedules WAITING for their deadline. A task that is due or running has
    /// been taken out; a runner puts it back when it re-arms.
    tasks: HashMap<u64, Task>,
    /// Due tasks waiting for a free runner.
    pending: VecDeque<(u64, Task)>,
    next_id: u64,
    timer_started: bool,
    idle_runners: usize,
    live_runners: usize,
}

struct Inner {
    state: Mutex<State>,
    /// Signals the timer: a new schedule, a re-arm, or a cancel.
    timer_cv: Condvar,
    /// Signals the runners: a task is due.
    work_cv: Condvar,
}

type Shared = Arc<Inner>;

static SCHED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SCHED.get_or_init(|| {
        Arc::new(Inner {
            state: Mutex::new(State::default()),
            timer_cv: Condvar::new(),
            work_cv: Condvar::new(),
        })
    })
}

fn terminated() -> ThreadResult {
    ThreadResult { status: "TERMINATED".to_string(), ..Default::default() }
}

/// Wake the timer. Call after setting a schedule's cancel flag: the timer is
/// parked until its next deadline, which for an hourly schedule is an hour
/// away, so without this a cancel would not be noticed until it fired.
pub fn wake() {
    if let Some(sh) = SCHED.get() {
        let _g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        sh.timer_cv.notify_all();
    }
}

/// Arm a schedule. Returns the handle whose `rx` the caller's future resolves
/// from; there is no per-schedule thread to join, so `join` is `None` (every
/// consumer takes it through `if let Some`).
pub fn schedule(
    seed: ThreadSeed,
    spawn: ThreadSpawnFn,
    delay_ms: i64,
    period: Option<(i64, bool)>,
    permits: Option<ScheduledPermits>,
    cancel: Arc<AtomicBool>,
) -> ThreadHandle {
    let (tx, rx) = std::sync::mpsc::channel::<ThreadResult>();
    let due = Instant::now() + Duration::from_millis(delay_ms.max(0) as u64);
    let sh = shared();
    {
        let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = g.next_id;
        g.next_id += 1;
        g.tasks.insert(
            id,
            Task {
                seed,
                spawn,
                period,
                permits,
                cancel: Arc::clone(&cancel),
                tx,
                run_at: due,
                first: true,
                _guard: ScheduleRelayGuard::new(),
            },
        );
        g.heap.push(Reverse((due, id)));
        if !g.timer_started {
            g.timer_started = true;
            let sh2 = Arc::clone(sh);
            // If the timer cannot start, the schedules it would drive never
            // fire — put the flag back and say so, rather than silently
            // swallowing every schedule from here on.
            if let Err(e) = std::thread::Builder::new()
                .name("rustcfml-scheduler".to_string())
                .spawn(move || timer_loop(sh2))
            {
                g.timer_started = false;
                eprintln!("[scheduler] could not start the timer thread: {}", e);
            }
        }
        sh.timer_cv.notify_all();
    }
    ThreadHandle { name: String::new(), rx, cancel, join: None, result: None }
}

/// The one parked thread. Waits on the earliest deadline, or indefinitely when
/// nothing is scheduled.
fn timer_loop(sh: Shared) {
    loop {
        let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        // Retire anything cancelled while we were parked, so a cancelled
        // schedule resolves its future now rather than at its next deadline.
        let dead: Vec<u64> = g
            .tasks
            .iter()
            .filter(|(_, t)| t.cancel.load(Ordering::Relaxed))
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            if let Some(t) = g.tasks.remove(&id) {
                let _ = t.tx.send(terminated());
            }
        }
        // `heap` can hold ids whose task has gone; drop those as we look.
        let now = Instant::now();
        let next_due = loop {
            match g.heap.peek() {
                None => break None,
                Some(Reverse((_, id))) if !g.tasks.contains_key(id) => {
                    g.heap.pop();
                }
                Some(Reverse((d, _))) => break Some(*d),
            }
        };
        match next_due {
            None => {
                let _unused = sh.timer_cv.wait(g).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            Some(d) if d > now => {
                let _unused =
                    sh.timer_cv.wait_timeout(g, d - now).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            Some(_) => {}
        }
        let Some(Reverse((_, id))) = g.heap.pop() else { continue };
        let Some(task) = g.tasks.remove(&id) else { continue };
        if task.cancel.load(Ordering::Relaxed) {
            let _ = task.tx.send(terminated());
            continue;
        }
        // Queue it and make sure someone can pick it up. A runner also re-arms
        // the schedule, so a slow body — or one parked on a permit — holds up
        // only its own schedule.
        g.pending.push_back((id, task));
        if g.idle_runners == 0 && g.live_runners < MAX_RUNNERS {
            let sh2 = Arc::clone(&sh);
            match std::thread::Builder::new()
                .name("rustcfml-schedule-runner".to_string())
                .spawn(move || runner_loop(sh2))
            {
                Ok(_) => g.live_runners += 1,
                Err(e) => eprintln!("[scheduler] could not start a runner: {}", e),
            }
        }
        drop(g);
        sh.work_cv.notify_one();
    }
}

/// A pooled runner: takes due tasks off `pending` for the life of the process.
fn runner_loop(sh: Shared) {
    loop {
        let (id, task) = {
            let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = g.pending.pop_front() {
                    break job;
                }
                // Park on the condvar, which RELEASES the lock while we wait —
                // unlike blocking in `recv()` with the lock held, which left
                // every other runner spinning on it.
                g.idle_runners += 1;
                g = sh.work_cv.wait(g).unwrap_or_else(|e| e.into_inner());
                g.idle_runners = g.idle_runners.saturating_sub(1);
            }
        };
        run_once(&sh, id, task);
    }
}

/// One firing: take a permit, run the body, then either resolve the future or
/// put the schedule back on the heap.
fn run_once(sh: &Shared, id: u64, mut task: Task) {
    if let Some(ref pm) = task.permits {
        if !pm.acquire(&task.cancel) {
            // Cancelled while queued behind a sibling run: the schedule is over.
            let _ = task.tx.send(terminated());
            return;
        }
    }
    let inner = (task.spawn)(task.seed.clone());
    let res = inner.rx.recv().ok();
    if let Some(j) = {
        let mut h = inner;
        h.join.take()
    } {
        let _ = j.join();
    }
    if let Some(ref pm) = task.permits {
        pm.release();
    }
    let terminated_run = res.as_ref().map(|r| r.status == "TERMINATED").unwrap_or(true);

    // A PERIODIC schedule's future must NOT resolve on the first run. The JVM's
    // ScheduledFuture stays pending for the life of the schedule — `isDone()` is
    // how callers ask "is this heartbeat still running?". Publishing the first
    // result made isDone() true after one tick, so Preside's
    // AbstractHeartBeat.start() saw its own heartbeat as stopped and scheduled
    // ANOTHER one on every call: schedules piled up until the adhoc-task
    // heartbeat was firing hundreds of times a second. One-shots keep first-run
    // semantics.
    if task.first && task.period.is_none() {
        if let Some(r) = res {
            let _ = task.tx.send(r);
        }
        return;
    }
    task.first = false;

    // One-shot, or a run that threw: a periodic task that fails is NOT
    // rescheduled (same as ScheduledExecutorService — otherwise a permanently
    // broken body spins forever).
    let Some((period_ms, fixed_rate)) = task.period else { return };
    if terminated_run || task.cancel.load(Ordering::Relaxed) {
        let _ = task.tx.send(terminated());
        return;
    }

    let period_dur = Duration::from_millis(period_ms.max(0) as u64);
    let next = if fixed_rate {
        // Fixed-rate: advance from the previous run's START. If the body
        // overran its period, SKIP the missed ticks rather than firing a
        // catch-up burst (the classic scheduleAtFixedRate surprise); the
        // schedule stays on its phase.
        let mut n = task.run_at + period_dur;
        let now = Instant::now();
        while n <= now {
            n += period_dur;
        }
        n
    } else {
        // Fixed-delay: measured from the END of the run that just finished.
        Instant::now() + period_dur
    };
    task.run_at = next;

    let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
    g.tasks.insert(id, task);
    g.heap.push(Reverse((next, id)));
    drop(g);
    sh.timer_cv.notify_all();
}

/// Schedules currently waiting on a deadline, for tests and diagnostics.
pub fn live_count() -> usize {
    SCHED
        .get()
        .map(|sh| sh.state.lock().unwrap_or_else(|e| e.into_inner()).tasks.len())
        .unwrap_or(0)
}
