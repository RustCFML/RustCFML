//! Phase 2 of the observability plan — a threshold-gated cooperative sampling
//! profiler (FusionReactor's "killer feature"), described in
//! `docs/observability-implementation-plan.md`.
//!
//! ## Why cooperative self-sampling
//! The CFML call stack lives inside the [`CfmlVirtualMachine`] owned by the
//! request's own (blocking) thread. A separate watchdog thread must **not**
//! reach across and read it — that would race the live `IndexMap` scopes. So
//! the watchdog only *asks* for a sample (flips an [`AtomicBool`]) and the VM
//! *provides* it at its next safe point (the `LineInfo` bytecode hook), snapshot
//! -ting its **own** `call_stack`. No cross-thread access to VM state.
//!
//! ## Cost model
//! * **Profiler off (config):** the VM never installs a handle; the per-line
//!   check is a `None` branch. Byte-identical to observability-off once inlined.
//! * **Armed, request under threshold:** the watchdog never flips the flag, so
//!   every `LineInfo` pays exactly one relaxed atomic load that returns `false`.
//! * **Actively sampling a slow request:** a stack snapshot every `intervalMs`
//!   — constant regardless of how much code the request runs.
//!
//! The whole module is behind the `observability` Cargo feature (host-only in
//! practice — the watchdog thread lives in the CLI and never ships to wasm).

#![cfg(feature = "observability")]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One frame of a captured stack sample: a function (or the top-level page)
/// executing at a source `line` in `template`.
#[derive(Clone, Debug)]
pub struct SampleFrame {
    pub function: String,
    pub template: String,
    pub line: usize,
}

/// The per-request handle the VM holds while a request is registered with the
/// [`ProfilerHub`]. Cheap to clone the `Arc`; the flag is the only thing the
/// watchdog thread touches.
pub struct RequestProfileHandle {
    /// Unique id of this in-flight request in the hub.
    pub id: u64,
    /// Set by the watchdog when it wants a sample; cleared by the VM once taken.
    pub want_sample: Arc<AtomicBool>,
    /// Hard cap on stored samples (bounds memory even if the watchdog misbehaves).
    pub max_samples: u32,
}

/// Raw samples collected for one request, plus the metadata needed to fold them
/// into a call tree and report percentages.
#[derive(Default)]
pub struct RequestSamples {
    /// Each entry is one stack snapshot, root → leaf (outermost call first).
    pub samples: Vec<Vec<SampleFrame>>,
}

impl RequestSamples {
    pub fn push(&mut self, frames: Vec<SampleFrame>) {
        self.samples.push(frames);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Fold the raw samples into an inverted call tree with self/total sample
    /// counts. Returns a synthetic `(request)` root whose children are the
    /// top-of-stack callees. `total_count` on a node = samples in which the node
    /// appears anywhere on the stack; `self_count` = samples in which the node
    /// was the leaf (actively executing). Self-time % = `self_count / total`.
    pub fn build_tree(&self) -> CallNode {
        let mut root = CallNode::new("(request)".to_string(), String::new(), 0);
        root.total_count = self.samples.len() as u64;
        for sample in &self.samples {
            let mut node = &mut root;
            let last = sample.len().saturating_sub(1);
            for (i, frame) in sample.iter().enumerate() {
                let idx = match node
                    .children
                    .iter()
                    .position(|c| c.function == frame.function && c.template == frame.template)
                {
                    Some(idx) => idx,
                    None => {
                        node.children.push(CallNode::new(
                            frame.function.clone(),
                            frame.template.clone(),
                            frame.line,
                        ));
                        node.children.len() - 1
                    }
                };
                node = &mut node.children[idx];
                node.total_count += 1;
                node.line = frame.line; // most-recent line seen for this node
                if i == last {
                    node.self_count += 1;
                }
            }
        }
        root.sort_by_total();
        root
    }
}

/// A node in the aggregated call tree.
#[derive(Clone, Debug)]
pub struct CallNode {
    pub function: String,
    pub template: String,
    pub line: usize,
    /// Samples in which this node was the actively-executing leaf.
    pub self_count: u64,
    /// Samples in which this node appears anywhere on the stack.
    pub total_count: u64,
    pub children: Vec<CallNode>,
}

impl CallNode {
    fn new(function: String, template: String, line: usize) -> Self {
        Self {
            function,
            template,
            line,
            self_count: 0,
            total_count: 0,
            children: Vec::new(),
        }
    }

    /// Recursively sort children by total sample count, hottest first, so the
    /// rendered tree reads top-down by cost.
    fn sort_by_total(&mut self) {
        self.children
            .sort_by(|a, b| b.total_count.cmp(&a.total_count));
        for c in &mut self.children {
            c.sort_by_total();
        }
    }
}

/// A finished profile published to the hub for the admin endpoint. Keeps a
/// small serialisable summary (not the raw samples).
#[derive(Clone, Debug)]
pub struct ProfileResult {
    pub id: u64,
    pub route: String,
    pub sample_count: u64,
    pub duration_ms: u64,
    pub tree: CallNode,
}

/// One in-flight request the watchdog is watching.
struct Inflight {
    started: Instant,
    want_sample: Arc<AtomicBool>,
    /// How many times the watchdog has already armed a sample (cadence + cap).
    armed_count: u32,
    /// When the watchdog last armed a sample (for interval pacing).
    last_armed: Option<Instant>,
    route: String,
    max_samples: u32,
}

/// Server-wide profiler registry, held on `ServerState`. The watchdog thread
/// scans `inflight`; request threads register/finish through it.
pub struct ProfilerHub {
    inflight: Mutex<Vec<(u64, Inflight)>>,
    /// Bounded ring of the most recent finished profiles (admin endpoint).
    recent: Mutex<VecDeque<ProfileResult>>,
    next_id: AtomicU64,
    /// Config snapshot (thresholds), copied at hub construction.
    pub threshold_ms: u64,
    pub interval_ms: u64,
    pub max_samples: u32,
    /// How many finished profiles to retain for the admin endpoint.
    recent_cap: usize,
}

impl ProfilerHub {
    pub fn new(threshold_ms: u64, interval_ms: u64, max_samples: u32) -> Self {
        Self {
            inflight: Mutex::new(Vec::new()),
            recent: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            threshold_ms,
            interval_ms,
            max_samples,
            recent_cap: 50,
        }
    }

    /// Register an in-flight request. Returns the handle the VM keeps for the
    /// life of the request (used to receive sample requests and to finish).
    pub fn register(&self, route: String) -> RequestProfileHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let want_sample = Arc::new(AtomicBool::new(false));
        let entry = Inflight {
            started: Instant::now(),
            want_sample: want_sample.clone(),
            armed_count: 0,
            last_armed: None,
            route,
            max_samples: self.max_samples,
        };
        if let Ok(mut v) = self.inflight.lock() {
            v.push((id, entry));
        }
        RequestProfileHandle {
            id,
            want_sample,
            max_samples: self.max_samples,
        }
    }

    /// Remove an in-flight request and, if it produced any samples, publish the
    /// folded call tree to the recent ring for the admin endpoint.
    pub fn finish(&self, id: u64, samples: &RequestSamples) {
        let route = if let Ok(mut v) = self.inflight.lock() {
            if let Some(pos) = v.iter().position(|(eid, _)| *eid == id) {
                Some(v.remove(pos).1.route)
            } else {
                None
            }
        } else {
            None
        };
        if samples.is_empty() {
            return;
        }
        let route = route.unwrap_or_default();
        let tree = samples.build_tree();
        let result = ProfileResult {
            id,
            route,
            sample_count: samples.len() as u64,
            duration_ms: 0,
            tree,
        };
        if let Ok(mut r) = self.recent.lock() {
            while r.len() >= self.recent_cap {
                r.pop_front();
            }
            r.push_back(result);
        }
    }

    /// Snapshot of recent finished profiles, newest first (admin endpoint).
    pub fn recent_profiles(&self) -> Vec<ProfileResult> {
        self.recent
            .lock()
            .map(|r| r.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// The watchdog tick: for every in-flight request past the threshold, arm a
    /// sample if the interval has elapsed and the per-request cap isn't hit.
    /// Runs on the watchdog thread; touches only the atomic flag + bookkeeping.
    pub fn tick(&self) {
        let now = Instant::now();
        let threshold = std::time::Duration::from_millis(self.threshold_ms);
        let interval = std::time::Duration::from_millis(self.interval_ms);
        if let Ok(mut v) = self.inflight.lock() {
            for (_, e) in v.iter_mut() {
                if now.duration_since(e.started) < threshold {
                    continue;
                }
                if e.armed_count >= e.max_samples {
                    continue;
                }
                let due = match e.last_armed {
                    None => true,
                    Some(t) => now.duration_since(t) >= interval,
                };
                if due {
                    e.want_sample.store(true, Ordering::Relaxed);
                    e.armed_count += 1;
                    e.last_armed = Some(now);
                }
            }
        }
    }

    /// Number of in-flight requests currently registered (test/introspection).
    pub fn inflight_count(&self) -> usize {
        self.inflight.lock().map(|v| v.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(f: &str) -> SampleFrame {
        SampleFrame {
            function: f.to_string(),
            template: "t.cfm".to_string(),
            line: 1,
        }
    }

    #[test]
    fn empty_samples_no_tree_publish() {
        let hub = ProfilerHub::new(3000, 200, 500);
        let h = hub.register("/x".to_string());
        let samples = RequestSamples::default();
        hub.finish(h.id, &samples);
        assert!(hub.recent_profiles().is_empty());
        assert_eq!(hub.inflight_count(), 0);
    }

    #[test]
    fn tree_aggregates_self_and_total() {
        // Two samples share a→b; one goes a→b→c (leaf c), the other a→b (leaf b).
        let mut s = RequestSamples::default();
        s.push(vec![frame("a"), frame("b"), frame("c")]);
        s.push(vec![frame("a"), frame("b")]);
        let root = s.build_tree();
        assert_eq!(root.total_count, 2);
        // root → a
        let a = &root.children[0];
        assert_eq!(a.function, "a");
        assert_eq!(a.total_count, 2);
        assert_eq!(a.self_count, 0);
        // a → b
        let b = &a.children[0];
        assert_eq!(b.function, "b");
        assert_eq!(b.total_count, 2);
        assert_eq!(b.self_count, 1); // leaf in the second sample
        // b → c
        let c = &b.children[0];
        assert_eq!(c.function, "c");
        assert_eq!(c.total_count, 1);
        assert_eq!(c.self_count, 1);
    }

    #[test]
    fn children_sorted_hottest_first() {
        let mut s = RequestSamples::default();
        // "cold" appears once, "hot" three times as direct children of root.
        s.push(vec![frame("hot")]);
        s.push(vec![frame("hot")]);
        s.push(vec![frame("hot")]);
        s.push(vec![frame("cold")]);
        let root = s.build_tree();
        assert_eq!(root.children[0].function, "hot");
        assert_eq!(root.children[0].total_count, 3);
        assert_eq!(root.children[1].function, "cold");
    }

    #[test]
    fn register_and_finish_publishes_result() {
        let hub = ProfilerHub::new(0, 0, 500);
        let h = hub.register("/posts".to_string());
        assert_eq!(hub.inflight_count(), 1);
        let mut s = RequestSamples::default();
        s.push(vec![frame("controller"), frame("render")]);
        hub.finish(h.id, &s);
        let recent = hub.recent_profiles();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].route, "/posts");
        assert_eq!(recent[0].sample_count, 1);
    }

    #[test]
    fn tick_arms_after_threshold() {
        // threshold 0 so it's immediately eligible; interval 0 so every tick arms.
        let hub = ProfilerHub::new(0, 0, 2);
        let h = hub.register("/slow".to_string());
        assert!(!h.want_sample.load(Ordering::Relaxed));
        hub.tick();
        assert!(h.want_sample.load(Ordering::Relaxed));
        // VM would clear it; simulate that and confirm the cap stops arming.
        h.want_sample.store(false, Ordering::Relaxed);
        hub.tick(); // armed_count now 2 == cap
        assert!(h.want_sample.load(Ordering::Relaxed));
        h.want_sample.store(false, Ordering::Relaxed);
        hub.tick(); // cap reached — must not arm again
        assert!(!h.want_sample.load(Ordering::Relaxed));
    }

    #[test]
    fn recent_ring_is_bounded() {
        let mut hub = ProfilerHub::new(0, 0, 500);
        hub.recent_cap = 3;
        for i in 0..5 {
            let h = hub.register(format!("/r{i}"));
            let mut s = RequestSamples::default();
            s.push(vec![frame("f")]);
            hub.finish(h.id, &s);
        }
        assert_eq!(hub.recent_profiles().len(), 3);
    }
}
