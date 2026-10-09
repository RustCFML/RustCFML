//! The debug footer's memory panel: what this request allocated and still
//! holds, the containers it created by kind, and where the process's memory
//! is. Only for a request whose memory is metered (allocation accounting on,
//! i.e. `debugging.enabled`); see `cfml_common::mem_account`.

use super::*;
use crate::debug_footer::{MemoryPanel, MemoryPot};
use cfml_common::mem_account;

/// The process breakdown is a walk over the application scopes, the sessions,
/// the object cache and the compiled code: milliseconds on a large
/// application. It is computed at most this often and shared by every request.
const POTS_TTL: std::time::Duration = std::time::Duration::from_secs(30);

struct PotsSnapshot {
    at: std::time::Instant,
    pots: Vec<MemoryPot>,
}

static POTS: Mutex<Option<PotsSnapshot>> = Mutex::new(None);

impl CfmlVirtualMachine {
    /// Start metering this request's memory, when debugging is enabled for it
    /// (whether or not the footer will show). Switches allocation accounting on
    /// the first time, if the server did not already at startup.
    pub fn begin_memory_metering(&mut self) {
        if !self.debug_config.enabled {
            return;
        }
        mem_account::enable();
        self.mem_meter = mem_account::RequestMeter::start();
        self.tmpl_alloc_stack.clear();
        self.thread_mem_allocated = 0;
    }

    /// The memory panel for the footer, or `None` when the request isn't
    /// metered.
    pub(crate) fn build_memory_panel(&self) -> Option<MemoryPanel> {
        let meter = self.mem_meter.as_ref()?;
        let footprint = mem_account::footprint_bytes();
        let live_heap = mem_account::live_heap_bytes();
        let (pots, age) = self.memory_pots(live_heap);
        Some(MemoryPanel {
            request: meter.read(),
            threads_allocated: self.thread_mem_allocated,
            census: cfml_common::cycle_gc::is_armed()
                .then(cfml_common::cycle_gc::request_census),
            footprint,
            limit: mem_account::limit_bytes(),
            live_heap,
            pots,
            pots_age_secs: age,
        })
    }

    fn memory_pots(&self, live_heap: Option<u64>) -> (Vec<MemoryPot>, u64) {
        let Some(ss) = self.server_state.as_ref() else {
            return (Vec::new(), 0);
        };
        let mut guard = match POTS.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let fresh = guard.as_ref().map(|s| s.at.elapsed() < POTS_TTL).unwrap_or(false);
        if !fresh {
            *guard = Some(PotsSnapshot {
                at: std::time::Instant::now(),
                pots: estimate_pots(ss),
            });
        }
        let snap = guard.as_ref().expect("just filled");
        let mut pots = snap.pots.clone();
        let pools = cfml_common::container_size::db_pool_count();
        // "Other" is the rest of the live heap, recomputed against the current
        // total rather than the cached one.
        if let Some(live) = live_heap {
            let attributed: u64 = pots.iter().map(|p| p.bytes).sum();
            pots.push(MemoryPot {
                name: "Other".to_string(),
                bytes: live.saturating_sub(attributed),
                detail: format!(
                    "the REMAINDER, not a measurement: in-flight requests, the \
                     per-request allocation logs (thread-local, not summable here), \
                     the driver buffers behind {} database connection pool{} (not \
                     sizeable through the driver crates), and whatever the pots above \
                     under-count",
                    pools,
                    if pools == 1 { "" } else { "s" }
                ),
            });
        }
        (pots, snap.at.elapsed().as_secs())
    }
}

fn kv_bytes(k: &str, v: &str) -> u64 {
    // A hash-map entry: the two strings plus table and allocation overhead.
    64 + k.len() as u64 + v.len() as u64
}

/// Estimate where the live heap is. One `seen` set across the data pots, so a
/// value reachable from several (an object in both the application scope and a
/// session) is counted once, under the first.
fn estimate_pots(ss: &ServerState) -> Vec<MemoryPot> {
    let mut out = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();

    let (files, bc) = ss.bytecode_cache.approx_bytes();
    out.push(MemoryPot {
        name: "Compiled code".to_string(),
        bytes: bc,
        detail: format!("{} files", files),
    });

    // Engine-owned tables are walked BEFORE the data scopes. `seen` credits a
    // shared backing store to whichever pot reaches it first, and a class's
    // method tables and blueprint metadata are the ENGINE's, merely referenced
    // by the instances an application scope holds. Walking the application
    // scope first charged them to it and left "Component classes" reading a
    // fraction of what the class cache actually holds.
    // Component classes, from the cross-request class cache: each class's
    // method tables, its shared metadata (and cached getMetadata() result),
    // plus its static scope.
    let mut class_bytes = 0u64;
    let class_n;
    {
        let classes = ss.class_caches.read();
        class_n = classes.len();
        let map_bytes = |m: &cfml_common::dynamic::ValueMap, seen: &mut HashSet<usize>| -> u64 {
            m.iter()
                .map(|(k, v)| 40 + k.as_str().len() as u64 + v.approx_heap_bytes(seen) as u64)
                .sum::<u64>()
        };
        for e in classes.values() {
            if let Some((a, b)) = &e.method_tables {
                for t in [a, b] {
                    if seen.insert(Arc::as_ptr(t) as usize) {
                        class_bytes += map_bytes(t, &mut seen);
                    }
                }
            }
            if let Some(t) = &e.own_table {
                if seen.insert(Arc::as_ptr(t) as usize) {
                    class_bytes += map_bytes(t, &mut seen);
                }
            }
            #[cfg(feature = "component-instance")]
            for (_, bp) in &e.blueprints {
                if seen.insert(Arc::as_ptr(bp) as usize) {
                    class_bytes += bp.metadata.approx_heap_bytes(&mut seen) as u64;
                    if let Some(m) = bp.metadata_cache.read().as_ref() {
                        class_bytes += m.approx_heap_bytes(&mut seen) as u64;
                    }
                }
            }
        }
    }
    let statics = ss.static_scopes.read();
    for e in statics.values() {
        class_bytes += e.scope.with_read(|m| {
            m.iter()
                .map(|(k, v)| 40 + k.as_str().len() as u64 + v.approx_heap_bytes(&mut seen) as u64)
                .sum::<u64>()
        });
    }
    out.push(MemoryPot {
        name: "Component classes".to_string(),
        bytes: class_bytes,
        detail: format!("{} classes, {} static scopes", class_n, statics.len()),
    });
    drop(statics);

    let apps = ss.applications.probe_states();
    let mut app_bytes = 0u64;
    let mut app_keys = 0usize;
    for (_, vars) in &apps {
        vars.with_read(|m| {
            app_keys += m.len();
            for (k, v) in m.iter() {
                app_bytes += 40 + k.as_str().len() as u64 + v.approx_heap_bytes(&mut seen) as u64;
            }
        });
    }
    out.push(MemoryPot {
        name: "Application scopes".to_string(),
        bytes: app_bytes,
        detail: format!("{} application{}, {} keys", apps.len(), if apps.len() == 1 { "" } else { "s" }, app_keys),
    });

    let server_bytes = ss.server_scope.with_read(|m| {
        m.iter()
            .map(|(k, v)| 40 + k.as_str().len() as u64 + v.approx_heap_bytes(&mut seen) as u64)
            .sum::<u64>()
    });
    out.push(MemoryPot {
        name: "Server scope".to_string(),
        bytes: server_bytes,
        detail: String::new(),
    });

    let sessions = ss.sessions.probe_variables();
    let session_bytes: u64 = sessions
        .iter()
        .map(|(_, vars)| {
            64 + vars
                .iter()
                .map(|(k, v)| 40 + k.as_str().len() as u64 + v.approx_heap_bytes(&mut seen) as u64)
                .sum::<u64>()
        })
        .sum();
    let session_detail = if ss.sessions.persists_by_serialization() {
        "stored outside the process".to_string()
    } else {
        format!("{} session{}", sessions.len(), if sessions.len() == 1 { "" } else { "s" })
    };
    out.push(MemoryPot {
        name: "Sessions".to_string(),
        bytes: session_bytes,
        detail: session_detail,
    });

    let (cache_n, cache_bytes) = {
        let c = ss.object_cache.read();
        let b: u64 = c
            .iter()
            .map(|(k, (v, _))| 64 + k.len() as u64 + v.approx_heap_bytes(&mut seen) as u64)
            .sum();
        (c.len(), b)
    };
    out.push(MemoryPot {
        name: "Object cache (cachePut)".to_string(),
        bytes: cache_bytes,
        detail: format!("{} entries", cache_n),
    });


    // The engine's own lookup caches (paths it has resolved). Small per entry;
    // their sizes are the key and value strings plus table overhead.
    let mut lookups = 0u64;
    let mut lookup_n = 0usize;
    {
        let c = ss.canonicalize_cache.read();
        lookup_n += c.len();
        lookups += c.iter().map(|(k, v)| kv_bytes(k, v.as_deref().unwrap_or(""))).sum::<u64>();
    }
    {
        let c = ss.exists_cache.read();
        lookup_n += c.len();
        lookups += c.keys().map(|k| kv_bytes(k, "") + 16).sum::<u64>();
    }
    {
        let c = ss.custom_tag_path_cache.read();
        lookup_n += c.len();
        lookups += c.iter().map(|((a, b), v)| kv_bytes(a, v) + b.len() as u64).sum::<u64>();
    }
    {
        let c = ss.component_path_cache.read();
        lookup_n += c.len();
        lookups += c.len() as u64 * 192;
    }
    {
        let c = ss.dir_fold_cache.read();
        lookup_n += c.len();
        lookups += c.keys().map(|k| kv_bytes(k, "") + 128).sum::<u64>();
    }
    out.push(MemoryPot {
        name: "Engine lookup caches".to_string(),
        bytes: lookups,
        detail: format!("{} entries", lookup_n),
    });

    // Interned identifier names. Counted here rather than folded into the
    // lookup caches: the interner is never pruned, so it grows with the
    // distinct identifiers the application's code uses and is worth seeing on
    // its own.
    out.push(MemoryPot {
        name: "Interned names".to_string(),
        bytes: cfml_common::name::Name::interned_bytes(),
        detail: format!("{} names", cfml_common::name::Name::interned_count()),
    });

    // The collector's CROSS-REQUEST survivor table: weak handles to every
    // container that outlived the request that created it. Its own memory, not
    // the containers'. Per-request logs are thread-local and cannot be summed
    // from here, so they stay in the remainder below.
    let (gc_n, gc_bytes) = cfml_common::cycle_gc::persistent_set_bytes();
    out.push(MemoryPot {
        name: "Collector survivor table".to_string(),
        bytes: gc_bytes,
        detail: format!("{} tracked containers", gc_n),
    });

    // Compiled regular expressions. The pattern strings are exact; the
    // compiled automaton behind each entry is not sizeable, so this is a
    // floor — see `cfml_stdlib::builtins` for the cache itself, which
    // registers its census through `container_size`.
    let (re_n, re_bytes) = cfml_common::container_size::regex_cache_census();
    out.push(MemoryPot {
        name: "Regex cache".to_string(),
        bytes: re_bytes,
        detail: format!("{} patterns; compiled automata not included", re_n),
    });

    out
}
