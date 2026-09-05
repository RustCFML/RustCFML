//! Async kernel — Layer 0.
//!
//! Native runtime support for the `coldbox.system.async.*` port (WireBox/
//! ColdBox dependency). Three primitives:
//!
//! - `runAsync(closure)` — VM intercept; spawns the closure via the cfthread
//!   spawner, wraps the resulting `ThreadHandle` in a `FutureNative`, returns
//!   it as a `CfmlValue::NativeObject`. Inline-runs and returns a resolved
//!   `FutureNative` on wasm / when `real-threads` is off.
//!
//! - `_schedule(closure, delayMs[, everyMs|spacedMs])` — VM intercept; spawns
//!   a worker that sleeps `delayMs` then runs the closure (one-shot, fixed-
//!   rate, or fixed-delay-after-completion). Returns a `FutureNative` ticket
//!   whose `cancel()` flips the cancel flag.
//!
//! - `Future` — `impl CfmlNative` holding the `ThreadHandle` + cached result.
//!   Method dispatch goes through `call_member_function` in `lib.rs`.
//!
//! Critical: `CfmlNative::call_method` has no `&mut VM`. The native object's
//! `RwLock` is held in write mode for the entire call. So `get()` must take
//! ownership of its channel (`Option::take`), not block on shared locked
//! state — a second concurrent method call on the same Future would deadlock
//! otherwise.
//!
//! Anything that *runs* a CFML closure (composing futures, firing
//! continuations) cannot live here — those must be intercepted BIFs with
//! `&mut VM`. The CFML async port composes via `runAsync(() => cb(prev.get()))`
//! instead.

use crate::{ThreadHandle, ThreadResult, ThreadSeed, ThreadSpawnFn};
use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use std::sync::{Arc, Mutex, RwLock};

/// A Future wrapping one spawned async task. Either holds a live
/// `ThreadHandle` (waiting/running) or a cached `ThreadResult` (resolved).
///
/// Identity is by `Arc` pointer-equality (the surrounding `Arc<RwLock<…>>`),
/// matching how other NativeObjects compare in the VM. The handle is
/// `Option::take`n on first `get()` so the receiver's ownership leaves the
/// locked struct before we block on it — avoids the documented re-entrancy
/// deadlock.
pub struct FutureNative {
    /// `ThreadHandle` holds an `mpsc::Receiver` which is `!Sync`; we wrap in
    /// a `Mutex` so `FutureNative` satisfies `CfmlNative: Sync`. In practice
    /// the surrounding `Arc<RwLock<dyn CfmlNative>>` already serializes
    /// method calls, so this inner lock is essentially uncontended.
    handle: Mutex<Option<ThreadHandle>>,
    result: Option<ThreadResult>,
    /// `True` when the task was inline-run (wasm / `real-threads` off) and
    /// the result was injected at construction. `cancel()` is a no-op then.
    inline_resolved: bool,
    /// `True` for a `java.util.concurrent` task, where the SAM's return value
    /// IS the future value — including when that value is null. A `Runnable`
    /// returns nothing and `Future.get()` must yield null; the cfthread-style
    /// fallbacks below (thread.result, then the whole `thread` scope) would
    /// otherwise hand back whatever the AMBIENT thread scope happened to hold.
    /// That is not hypothetical: under TestBox it returned the runner's own
    /// `thread` vars (testResults/suiteStats/target/closures/suite/spec).
    strict_value: bool,
}

impl std::fmt::Debug for FutureNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FutureNative")
            .field("done", &self.result.is_some())
            .field(
                "status",
                &self.result.as_ref().map(|r| r.status.clone()).unwrap_or_default(),
            )
            .finish()
    }
}

impl FutureNative {
    /// Wrap a freshly-spawned `ThreadHandle` (the live, async path).
    pub fn from_handle(handle: ThreadHandle) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
            result: None,
            inline_resolved: false,
            strict_value: false,
        }
    }

    /// As `from_handle`, but with JVM `Future` value semantics — see
    /// [`FutureNative::strict_value`].
    pub fn from_handle_strict(handle: ThreadHandle) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
            result: None,
            inline_resolved: false,
            strict_value: true,
        }
    }

    /// Pre-resolved future (inline-run path: wasm / `real-threads` off).
    pub fn resolved(result: ThreadResult) -> Self {
        Self {
            handle: Mutex::new(None),
            result: Some(result),
            inline_resolved: true,
            strict_value: false,
        }
    }

    /// Pre-resolved future with JVM `Future` value semantics.
    pub fn resolved_strict(result: ThreadResult) -> Self {
        Self {
            handle: Mutex::new(None),
            result: Some(result),
            inline_resolved: true,
            strict_value: true,
        }
    }

    /// Block until the underlying task completes (or `timeout_ms` elapses;
    /// 0 = forever). Returns:
    /// - `Ok(value)` — completed normally. The thread body's *return value*
    ///   is the future value. We can't recover that from `ThreadResult`
    ///   (which only carries status/output/error/thread_vars), so v1 returns
    ///   `thread.result` when the body set it, else the `thread` scope as a
    ///   struct, else Null.
    /// - `Err(...)` — the body threw; the error message is preserved.
    ///
    /// On timeout: leaves the handle in place and returns Null without
    /// error (matches `threadJoin` semantics — caller checks `isDone()`).
    fn await_result(&mut self, timeout_ms: i64) -> CfmlResult {
        if self.result.is_none() {
            // Take the handle out of the Mutex slot so we don't hold the
            // inner lock across the blocking recv. If we time out we put it
            // back; on completion it stays None.
            let mut taken: Option<ThreadHandle> = {
                let mut slot = self.handle.lock().unwrap();
                slot.take()
            };
            if let Some(mut handle) = taken.take() {
                let recv = if timeout_ms > 0 {
                    handle
                        .rx
                        .recv_timeout(std::time::Duration::from_millis(timeout_ms as u64))
                        .ok()
                } else {
                    handle.rx.recv().ok()
                };
                match recv {
                    Some(res) => {
                        if let Some(j) = handle.join.take() {
                            let _ = j.join();
                        }
                        self.result = Some(res);
                    }
                    None => {
                        // Timeout — restore the handle and return Null.
                        let mut slot = self.handle.lock().unwrap();
                        *slot = Some(handle);
                        return Ok(CfmlValue::Null);
                    }
                }
            }
        }
        let r = self.result.as_ref().unwrap();
        if !r.error.is_empty() {
            return Err(CfmlError::runtime(r.error.clone()));
        }
        // The closure's return value is the future value. Fall back to
        // `thread.result` (set inside the body), then the whole `thread`
        // scope, then Null — matches the convention CFML users expect.
        if let Some(v) = &r.return_value {
            if !matches!(v, CfmlValue::Null) {
                return Ok(v.clone());
            }
        }
        // JVM Future: the SAM's value is the answer, null included.
        if self.strict_value {
            return Ok(CfmlValue::Null);
        }
        if let Some(v) = r.thread_vars.get("result") {
            return Ok(v.clone());
        }
        if !r.thread_vars.is_empty() {
            return Ok(CfmlValue::strukt(r.thread_vars.clone()));
        }
        Ok(CfmlValue::Null)
    }

    fn is_done(&mut self) -> bool {
        if self.result.is_some() {
            return true;
        }
        // Non-blockingly drain the channel: if the body has already
        // published, cache the result so subsequent get()/isDone calls are
        // O(1). Without this, `isDone()` would lie until someone called
        // `get()` and `anyOf`/poll-style loops would spin forever.
        let taken: Option<ThreadHandle> = {
            let mut slot = self.handle.lock().unwrap();
            slot.take()
        };
        if let Some(mut handle) = taken {
            match handle.rx.try_recv() {
                Ok(res) => {
                    if let Some(j) = handle.join.take() {
                        let _ = j.join();
                    }
                    self.result = Some(res);
                    return true;
                }
                Err(_) => {
                    // Not ready — put the handle back.
                    let mut slot = self.handle.lock().unwrap();
                    *slot = Some(handle);
                    return false;
                }
            }
        }
        false
    }

    fn is_cancelled(&self) -> bool {
        if let Some(r) = &self.result {
            return r.status == "TERMINATED";
        }
        let slot = self.handle.lock().unwrap();
        match &*slot {
            Some(h) => h.cancel.load(std::sync::atomic::Ordering::Relaxed),
            None => false,
        }
    }

    fn error_message(&self) -> String {
        self.result
            .as_ref()
            .map(|r| r.error.clone())
            .unwrap_or_default()
    }

    fn status_str(&self) -> String {
        if let Some(r) = &self.result {
            return r.status.clone();
        }
        let slot = self.handle.lock().unwrap();
        if slot.is_some() {
            "RUNNING".to_string()
        } else {
            "UNKNOWN".to_string()
        }
    }
}

impl CfmlNative for FutureNative {
    fn class_name(&self) -> &str {
        "Future"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_ascii_lowercase().as_str() {
            "get" => {
                let timeout = args
                    .get(0)
                    .map(|v| v.as_string().parse::<i64>().unwrap_or(0))
                    .unwrap_or(0);
                self.await_result(timeout)
            }
            "isdone" => Ok(CfmlValue::Bool(self.is_done())),
            "iscancelled" | "iscanceled" => Ok(CfmlValue::Bool(self.is_cancelled())),
            "cancel" => {
                if self.inline_resolved || self.result.is_some() {
                    return Ok(CfmlValue::Bool(false));
                }
                let slot = self.handle.lock().unwrap();
                if let Some(h) = &*slot {
                    h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Ok(CfmlValue::Bool(true));
                }
                Ok(CfmlValue::Bool(false))
            }
            "error" => Ok(CfmlValue::string(self.error_message())),
            "status" => Ok(CfmlValue::string(self.status_str())),
            other => Err(CfmlError::runtime(format!(
                "Future has no method [{}]",
                other
            ))),
        }
    }

    fn get_property(&self, name: &str) -> Option<CfmlValue> {
        // Allow `future.done` / `future.status` / `future.error` as property
        // reads — same shape WireBox uses elsewhere.
        match name.to_ascii_lowercase().as_str() {
            // Property read is &self only; report the cached state without
            // polling. Callers wanting authoritative "done" should invoke
            // the isDone() method (which can poll).
            "done" => Some(CfmlValue::Bool(self.result.is_some())),
            "status" => Some(CfmlValue::string(self.status_str())),
            "error" => Some(CfmlValue::string(self.error_message())),
            _ => None,
        }
    }
}

/// The completion queue behind `java.util.concurrent.ExecutorCompletionService`.
///
/// This has to be a NativeObject rather than a plain struct key: the completion
/// service is passed around BY VALUE in CFML (cfconcurrent hands the same
/// service to `AbstractCompletionTask` while keeping its own reference, and
/// `AbstractCompletionTask.run()` polls through its copy). A struct + method
/// writeback gives each holder its own private queue, so tasks submitted
/// through one copy are invisible to the poller — which is exactly why
/// `poll()` used to return nothing. An `Arc` inside the struct makes every
/// copy share one queue, matching the JVM's object identity.
#[derive(Debug, Default)]
pub struct CompletionQueueNative {
    /// Futures submitted but not yet handed out by poll()/take().
    pending: Vec<CfmlValue>,
}

impl CompletionQueueNative {
    pub fn new_value() -> CfmlValue {
        CfmlValue::NativeObject(Arc::new(RwLock::new(Self::default())))
    }

    /// Is this future finished? Asks the Future itself, so a task that
    /// completed on its own thread is visible without anyone calling get().
    fn future_done(f: &CfmlValue) -> bool {
        match f {
            CfmlValue::NativeObject(o) => o
                .write()
                .ok()
                .and_then(|mut g| g.call_method("isDone", vec![]).ok())
                .map(|v| v.is_true())
                .unwrap_or(false),
            _ => false,
        }
    }
}

impl CfmlNative for CompletionQueueNative {
    fn class_name(&self) -> &str {
        "CompletionQueue"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        match name.to_ascii_lowercase().as_str() {
            "add" => {
                if let Some(f) = args.into_iter().next() {
                    self.pending.push(f);
                }
                Ok(CfmlValue::Bool(true))
            }
            // poll() — non-blocking: the first COMPLETED future, else null.
            // Completion order, not submission order, per the JVM contract.
            "poll" => {
                if let Some(i) = self.pending.iter().position(Self::future_done) {
                    return Ok(self.pending.remove(i));
                }
                Ok(CfmlValue::Null)
            }
            // take() — blocks until one is available.
            "take" => loop {
                if let Some(i) = self.pending.iter().position(Self::future_done) {
                    return Ok(self.pending.remove(i));
                }
                if self.pending.is_empty() {
                    return Ok(CfmlValue::Null);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            },
            "size" => Ok(CfmlValue::Int(self.pending.len() as i64)),
            "clear" => {
                self.pending.clear();
                Ok(CfmlValue::Null)
            }
            other => Err(CfmlError::runtime(format!(
                "CompletionQueue has no method [{}]",
                other
            ))),
        }
    }
}


// ─────────────────────────────────────────────
// Bounded executor pool
//
// `ThreadPoolExecutor(corePoolSize, maxPoolSize, …, workQueue, factory, policy)`
// is not just a place to fling threads: the JVM runs at most `maxPoolSize`
// tasks at once, holds the overflow in a queue of bounded capacity, and applies
// a RejectedExecutionHandler when that queue is full. Running every submitted
// task immediately on its own detached thread gets the happy path right and
// every back-pressure property wrong — an app that submits 10k tasks would
// spawn 10k threads instead of queueing 10k and discarding the overflow.
// ─────────────────────────────────────────────

/// What to do with a task submitted to a pool whose queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectPolicy {
    /// Silently drop the new task (java: DiscardPolicy).
    Discard,
    /// Drop the longest-waiting queued task, then enqueue this one.
    DiscardOldest,
    /// Throw RejectedExecutionException.
    Abort,
    /// Run it on the submitting thread.
    CallerRuns,
}

impl RejectPolicy {
    pub fn from_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("discardoldest") {
            RejectPolicy::DiscardOldest
        } else if n.contains("abort") {
            RejectPolicy::Abort
        } else if n.contains("callerruns") {
            RejectPolicy::CallerRuns
        } else {
            RejectPolicy::Discard
        }
    }
}

/// A task waiting for a worker.
struct PendingTask {
    seed: ThreadSeed,
    tx: std::sync::mpsc::Sender<ThreadResult>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

struct PoolShared {
    max_concurrent: usize,
    queue_capacity: usize,
    policy: RejectPolicy,
    /// Cancel flags for every PERIODIC schedule this executor started. The JVM
    /// cancels periodic tasks on `shutdown()`
    /// (ContinueExistingPeriodicTasksAfterShutdownPolicy defaults to false), and
    /// we must too: a relay that outlives its executor keeps its `ThreadSeed`
    /// alive, and a seed owns a clone of the whole compiled program, the globals
    /// snapshot and the application scope. Preside rebuilds its executors on
    /// every `?fwreinit=true`, so leaking them cost ~100MB per reload.
    scheduled_cancels: Vec<Arc<std::sync::atomic::AtomicBool>>,
    queue: std::collections::VecDeque<PendingTask>,
    /// Worker threads alive right now (each runs at most one task at a time,
    /// so this is also the count of concurrently-executing tasks).
    live_workers: usize,
    /// Permits held by scheduled runs, which bypass the queue (the JVM's
    /// ScheduledThreadPoolExecutor uses its own delayed queue) but are still
    /// bounded by the pool size.
    scheduled_running: usize,
    spawn_fn: Option<ThreadSpawnFn>,
}

/// The pool behind an executor shim. Held as a NativeObject so every CFML copy
/// of the executor struct shares one pool (same reasoning as the completion
/// queue: cfconcurrent passes executors around by value).
pub struct ExecutorPoolNative {
    inner: Arc<(std::sync::Mutex<PoolShared>, std::sync::Condvar)>,
}

/// An executor that goes out of scope WITHOUT `shutdown()` must still stop its
/// periodic schedules.
///
/// A relay keeps its `ThreadSeed` alive for the life of the schedule, and a seed
/// owns the task body — for a `java.util.concurrent` submission that body is the
/// `{__async_invoke_target, __async_invoke_method}` sentinel holding the RECEIVER
/// COMPONENT, which reaches the framework's whole object graph. So an abandoned
/// executor does not merely leak a thread: it pins an entire generation of the
/// application, forever, and keeps ticking.
///
/// `shutdown_all` covered the explicit call. Nothing covered the far more common
/// case of a framework simply REPLACING its executors — Preside rebuilds its
/// heartbeats and thread pools and drops the old handles on the floor, so every
/// rebuild stranded another generation. Measured on a live Preside, one pinned
/// generation of ~111,000 nodes per abandoned executor.
///
/// Safe as a `Drop`: this handle is never cloned (the workers and relays share
/// `PoolShared` through its own `Arc`, not through this struct), so the drop
/// happens exactly when CFML lets go of the executor.
impl Drop for ExecutorPoolNative {
    fn drop(&mut self) {
        // Cancel schedules only — queued one-shot work is left to finish, which
        // is what `shutdown()` (as opposed to `shutdownNow()`) means.
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            let n = self
                .inner
                .0
                .lock()
                .map(|g| g.scheduled_cancels.len())
                .unwrap_or(0);
            eprintln!("[executor] abandoned executor dropped; cancelling {} schedule(s)", n);
        }
        self.shutdown_all(false);
    }
}

impl std::fmt::Debug for ExecutorPoolNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.0.lock().unwrap();
        f.debug_struct("ExecutorPool")
            .field("maxConcurrent", &g.max_concurrent)
            .field("queued", &g.queue.len())
            .field("active", &g.live_workers)
            .finish()
    }
}

/// A failed result: the task could not run for a reason the caller should see
/// as an error from `Future.get()`.
fn terminated(reason: &str) -> ThreadResult {
    ThreadResult {
        status: "TERMINATED".to_string(),
        output: String::new(),
        error: reason.to_string(),
        elapsed: 0,
        thread_vars: ValueMap::default(),
        return_value: None,
    }
}

/// A task that was deliberately dropped (a rejection policy, or cancelled
/// before it ran). The JVM leaves such a future permanently incomplete, so
/// `get()` blocks forever; we resolve it as CANCELLED instead — `isCancelled()`
/// is true and `get()` returns null. Deadlocking a request to be bug-compatible
/// with the JVM would be the worse trade. No `error` is set, because nothing
/// went wrong: the pool did exactly what its policy asked.
fn discarded() -> ThreadResult {
    ThreadResult {
        status: "TERMINATED".to_string(),
        output: String::new(),
        error: String::new(),
        elapsed: 0,
        thread_vars: ValueMap::default(),
        return_value: None,
    }
}

impl ExecutorPoolNative {
    pub fn new_value(
        max_concurrent: usize,
        queue_capacity: usize,
        policy: RejectPolicy,
        spawn_fn: Option<ThreadSpawnFn>,
    ) -> CfmlValue {
        let pool = ExecutorPoolNative {
            inner: Arc::new((
                std::sync::Mutex::new(PoolShared {
                    max_concurrent: max_concurrent.max(1),
                    queue_capacity: queue_capacity.max(1),
                    policy,
                    scheduled_cancels: Vec::new(),
                    queue: std::collections::VecDeque::new(),
                    live_workers: 0,
                    scheduled_running: 0,
                    spawn_fn,
                }),
                std::sync::Condvar::new(),
            )),
        };
        CfmlValue::NativeObject(Arc::new(RwLock::new(pool)))
    }

    /// Track a periodic schedule so `shutdown()` can stop it.
    pub fn register_schedule(&self, cancel: Arc<std::sync::atomic::AtomicBool>) {
        let mut g = self.inner.0.lock().unwrap();
        // Drop flags whose relay has already exited, so a long-lived executor
        // that reschedules repeatedly does not accumulate dead entries.
        g.scheduled_cancels
            .retain(|c| !c.load(std::sync::atomic::Ordering::Relaxed));
        g.scheduled_cancels.push(cancel);
    }

    /// `shutdown()` / `shutdownNow()`: stop every periodic schedule this
    /// executor started, and (for shutdownNow) discard anything still queued.
    pub fn shutdown_all(&self, drop_queued: bool) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock().unwrap();
        for c in g.scheduled_cancels.drain(..) {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if drop_queued {
            for t in g.queue.drain(..) {
                t.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = t.tx.send(discarded());
            }
        }
        cv.notify_all();
    }

    pub fn max_concurrent(&self) -> usize {
        self.inner.0.lock().unwrap().max_concurrent
    }

    /// Enqueue `seed` and return the handle its Future will resolve from.
    /// `Err` is only produced by the Abort policy.
    pub fn submit(&self, seed: ThreadSeed) -> Result<ThreadHandle, CfmlError> {
        let cancel = seed.cancel_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel::<ThreadResult>();
        let handle = ThreadHandle {
            name: String::new(),
            rx,
            cancel: cancel.clone(),
            join: None,
            result: None,
        };

        let mut seed = Some(seed);
        let mut caller_runs_seed: Option<ThreadSeed> = None;
        let mut caller_tx: Option<std::sync::mpsc::Sender<ThreadResult>> = None;
        {
            let (lock, cv) = &*self.inner;
            let mut g = lock.lock().unwrap();

            if g.queue.len() >= g.queue_capacity {
                match g.policy {
                    RejectPolicy::Discard => {
                        let _ = tx.send(discarded());
                        return Ok(handle);
                    }
                    RejectPolicy::DiscardOldest => {
                        if let Some(old) = g.queue.pop_front() {
                            let _ = old.tx.send(discarded());
                        }
                    }
                    RejectPolicy::Abort => {
                        return Err(CfmlError::new(
                            "Task rejected: executor queue is full".to_string(),
                            cfml_common::vm::CfmlErrorType::Custom(
                                "java.util.concurrent.RejectedExecutionException".to_string(),
                            ),
                        ));
                    }
                    RejectPolicy::CallerRuns => {
                        caller_runs_seed = seed.take();
                        caller_tx = Some(tx.clone());
                    }
                }
            }

            if let Some(seed) = seed.take() {
                g.queue.push_back(PendingTask { seed, tx, cancel });
                // Start a worker only while we are under the concurrency limit.
                if g.live_workers < g.max_concurrent {
                    g.live_workers += 1;
                    let inner = Arc::clone(&self.inner);
                    if std::thread::Builder::new()
                        .name("rustcfml-pool-worker".to_string())
                        .spawn(move || Self::worker_loop(inner))
                        .is_err()
                    {
                        g.live_workers -= 1;
                    }
                }
                cv.notify_all();
            }
        }

        // CallerRuns happens outside the lock — it runs the body here, on the
        // submitting thread, exactly as the JVM policy does.
        if let (Some(seed), Some(tx)) = (caller_runs_seed, caller_tx) {
            let spawn = { self.inner.0.lock().unwrap().spawn_fn };
            if let Some(spawn_fn) = spawn {
                let inner_handle = spawn_fn(seed);
                if let Ok(res) = inner_handle.rx.recv() {
                    let _ = tx.send(res);
                } else {
                    let _ = tx.send(terminated("Task produced no result"));
                }
            } else {
                let _ = tx.send(terminated("No thread spawner available"));
            }
        }
        Ok(handle)
    }

    fn worker_loop(inner: Arc<(std::sync::Mutex<PoolShared>, std::sync::Condvar)>) {
        loop {
            let (task, spawn) = {
                let (lock, _cv) = &*inner;
                let mut g = lock.lock().unwrap();
                match g.queue.pop_front() {
                    Some(t) => (t, g.spawn_fn),
                    None => {
                        // Check-and-exit must be atomic with the decrement, or a
                        // concurrent submit could see a worker that is leaving.
                        g.live_workers -= 1;
                        return;
                    }
                }
            };

            if task.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = task.tx.send(discarded());
                continue;
            }
            match spawn {
                Some(spawn_fn) => {
                    let mut h = spawn_fn(task.seed);
                    let res = h.rx.recv().ok();
                    if let Some(j) = h.join.take() {
                        let _ = j.join();
                    }
                    let _ = task
                        .tx
                        .send(res.unwrap_or_else(|| terminated("Task produced no result")));
                }
                None => {
                    let _ = task.tx.send(terminated("No thread spawner available"));
                }
            }
        }
    }

    /// Block until a scheduled run may start, honouring the pool size. The
    /// JVM's ScheduledThreadPoolExecutor bounds concurrent runs by corePoolSize
    /// too — Preside builds its heartbeat pool with exactly 1 — so periodic
    /// tasks must not each get a free thread.
    pub fn acquire_scheduled(&self) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock().unwrap();
        while g.scheduled_running >= g.max_concurrent {
            g = cv.wait(g).unwrap();
        }
        g.scheduled_running += 1;
    }

    pub fn release_scheduled(&self) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock().unwrap();
        g.scheduled_running = g.scheduled_running.saturating_sub(1);
        cv.notify_all();
    }
}

impl CfmlNative for ExecutorPoolNative {
    fn class_name(&self) -> &str {
        "ExecutorPool"
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn call_method(&mut self, name: &str, _args: Vec<CfmlValue>) -> CfmlResult {
        let g = self.inner.0.lock().unwrap();
        match name.to_ascii_lowercase().as_str() {
            // Pool statistics the JVM exposes and cfconcurrent surfaces.
            "getactivecount" => Ok(CfmlValue::Int(g.live_workers as i64)),
            "getqueuesize" => Ok(CfmlValue::Int(g.queue.len() as i64)),
            "getcorepoolsize" | "getmaximumpoolsize" => {
                Ok(CfmlValue::Int(g.max_concurrent as i64))
            }
            other => Err(CfmlError::runtime(format!(
                "ExecutorPool has no method [{}]",
                other
            ))),
        }
    }
}

/// Read a numeric option from a CFML struct (case-insensitive). Returns
/// `None` when the key is absent or unparseable.
pub fn struct_get_i64(s: &ValueMap, key: &str) -> Option<i64> {
    s.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| match v {
            CfmlValue::Int(i) => Some(*i),
            CfmlValue::Double(d) => Some(*d as i64),
            CfmlValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            other => other.as_string().parse::<i64>().ok(),
        })
}

/// Live count of periodic schedule relays, for the leak diagnostics.
///
/// A relay holds a `ThreadSeed` for the life of its schedule, and a seed owns the
/// task body. For a `java.util.concurrent` submission that body is the
/// `{__async_invoke_target, __async_invoke_method}` sentinel holding the RECEIVER
/// COMPONENT, which reaches the framework's entire object graph — so each live
/// relay pins one generation of the application. A count that climbs on an idle
/// server therefore means schedules are being started faster than they are
/// cancelled, and says so in units of "generations retained".
pub static LIVE_SCHEDULE_RELAYS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// RAII counter for [`LIVE_SCHEDULE_RELAYS`] — a relay exits down several paths
/// (cancelled, body threw, one-shot completed), and a manual decrement on each
/// is exactly the kind of thing that goes stale.
pub struct ScheduleRelayGuard;

impl ScheduleRelayGuard {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let n = LIVE_SCHEDULE_RELAYS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            eprintln!("[executor] schedule relay started; {} live", n);
        }
        ScheduleRelayGuard
    }
}

impl Drop for ScheduleRelayGuard {
    fn drop(&mut self) {
        let n = LIVE_SCHEDULE_RELAYS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) - 1;
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            eprintln!("[executor] schedule relay ended; {} live", n);
        }
    }
}
