# Memory management

RustCFML has no JVM, so there is no heap to size, no `-Xmx`, and no
stop-the-world garbage collector. Memory is managed in three layers:

1. **Reference counting** frees almost everything the moment it is no longer
   used.
2. A **cycle collector** frees the one kind of garbage reference counting can't:
   objects that refer to each other, such as every component instance.
3. The **mimalloc** allocator hands memory to the engine and decides when to
   give it back to the operating system.

For most deployments the only setting you need is a memory limit
(`--max-memory`, below). The rest of this page explains what happens underneath,
what you can measure, and the knobs that exist for diagnosing a footprint that
looks wrong.

## Setting a memory limit

Give the server a ceiling sized to its container, as you would give a JVM
`-Xmx`:

```bash
rustcfml --serve ./webroot --production --max-memory 1.5G   # also 1536M, 1610612736
rustcfml --serve ./webroot --production --max-memory auto   # 75% of the cgroup limit
RUSTCFML_MAX_MEMORY=1.5G rustcfml --serve ./webroot --production
```

- Sizes take `K`, `M`, `G` or `T` (binary: `1G` = 1024 MiB), any case, with an
  optional `B`/`iB`, and decimals are fine. A bare number is bytes. `off` or `0`
  means no limit.
- `auto` is 75% of the container's cgroup limit (`memory.max` on cgroup v2,
  `memory.limit_in_bytes` on v1). With no cgroup limit, `auto` means no limit.
- The flag wins over `RUSTCFML_MAX_MEMORY`. An unparseable value stops the
  server at startup. The limit applies to `--serve` only.

The startup banner says what is being measured and its current value:

```
Memory limit: 1.50G (new requests refused with 503 above 1.27G, largest in-flight request aborted above 1.42G; cgroup memory.current now 412M)
```

### What is measured

The limit is checked against the process's real physical footprint, not an
internal count:

| Platform | Measurement |
|---|---|
| Linux in a container with a memory limit | cgroup `memory.current` (v2) or `memory.usage_in_bytes` (v1), the number the OOM killer uses |
| Linux otherwise | resident set size (`/proc/self/statm`) |
| macOS | physical footprint (`proc_pid_rusage`, the figure Activity Monitor shows) |
| anything else | mimalloc's own process statistics |

### The two tiers

**At 85% of the limit: back-pressure.** New requests are refused with **503**
and `Retry-After: 2`, which load balancers and orchestrators read as
back-pressure rather than failure. Requests already running carry on, and the
server sheds memory (a cross-request collector sweep, then a return of the
allocator's retained pages to the OS, at most once every 2 seconds). It starts
admitting again once the footprint falls below 95% of that threshold.

```
[memory] footprint 1.29G over the soft limit 1.27G (max 1.50G) — refusing new requests with 503 and shedding
[memory] footprint 1.19G back under the soft limit 1.27G — admitting requests again (14 refused)
```

**At 95%: abort the runaway.** Back-pressure can't help when one request is
allocating without bound: nothing new is arriving, and the request already
inside would run until the OOM killer took the whole process. A watchdog checks
the footprint every 250 ms, and above this line it aborts **the in-flight
request that has allocated the most**, with an error its own `try/catch` can't
swallow. The client gets a plain **500** (not a 503, because a runaway request
shouldn't be retried elsewhere), every other request keeps running, and the log
names the request:

```
[max-memory] footprint 684M is over the hard limit 665M of 700M — aborting
request #2 [/app/report.cfm], the largest allocator (499915 tracked containers).
Other requests continue.
```

A request is only eligible once it has itself allocated more than **100,000**
containers (structs, arrays, queries, closure scopes, component instances). When
the memory is held by the application scope, the caches or the allocator, no
request is responsible and killing one would free nothing, so nothing is
aborted and the server sheds instead:

```
[max-memory] footprint 765M is over the hard limit 475M of 500M, but none of the
1 in-flight request(s) has allocated enough to be responsible — the memory is
held by the application, the caches or the allocator. Not aborting anything;
shedding instead.
```

The abort takes effect at the next user-function call, or inside `sleep`,
`queryExecute` or `cfhttp`. A tight loop that calls none of those is the
exception, the same gap `requestTimeout` has.

### Sizing

Size the limit at roughly **twice the steady-state footprint**:

- For a short while after a framework reload (`?fwreinit=true` and the like) two
  generations of the application are legitimately resident.
- Once the old generation is freed, the allocator keeps a plateau of about
  **1.6× the live data** (see [Allocator retention](#allocator-retention)).

A Preside site idling at ~450 MB sits at 750–850 MB after a few reloads: `auto`
in a 2 GB container (1.5 GB) suits it, a 700 MB limit doesn't.

## How memory is freed

### Reference counting

Every value that can be shared (strings, arrays, structs, queries, functions,
component instances) is reference-counted. When the last variable, scope or
container holding a value lets go of it, it is freed immediately, on the same
thread, as part of the operation that dropped it. A request's local data goes
back to the allocator as its function frames return, and whatever is left in its
`variables` and `request` scopes goes at request end.

There is no pause, nothing to tune, and this covers the great majority of
everything a request allocates.

### The cycle collector

Reference counting can't free a **cycle**: a group of values that refer to each
other, so that each keeps the others' counts above zero after the program has
let go of all of them. CFML creates cycles constantly:

- every component instance (`this` → `variables` → `this`)
- a closure stored in the scope it captured
- framework object graphs: WireBox's injector and binder, a ColdBox controller
  and its services, a Preside object and its relationships

Left alone, a long-running server would leak a little on every request. The
cycle collector finds and frees them.

**How it decides what is garbage.** The collector keeps a log of the containers
a request allocates: structs, arrays, queries, closure scopes and component
instances. It holds them weakly, so the log never keeps anything alive. To
sweep, it counts, for each container still alive, how many references it gets
from the other containers in the log. Anything with more references than that
is held from outside (a scope, the application, a variable still in use) and is
live, along with everything it reaches. What remains is referenced only from
inside the group: an unreachable cycle. The collector clears those containers,
their reference counts drop to zero, and reference counting frees them.

This is trial deletion (Bacon and Rajan) bounded to what a request created. The
collector never walks the whole heap and never stops other requests: each
request's log is its own, and a container it can't lock without waiting is
skipped and treated as live. A sweep can under-collect, leaving garbage for a
later sweep, but it can't free something still in use.

**When it runs.**

| When | What is swept |
|---|---|
| **During a request, at component construction** | Once a request has allocated 100,000 containers since its last sweep (`RUSTCFML_GC_INCREMENTAL`), the next `new` / `createObject` runs a **minor** sweep over the newest allocations. Survivors are promoted to an **old** generation, which a **major** sweep covers once it has doubled in size. A request that never constructs a component isn't swept until it ends. |
| **At the end of every request** | Everything the request logged. Survivors (anything it stored in the application or session scope, say) are carried into a cross-request set. |
| **At the end of every `cfthread` body** | The same, for the thread's own allocations. |
| **Across requests** | The carried survivors, once that set has doubled since its last sweep (starting at 50,000, `RUSTCFML_GC_PERSISTENT`). This frees anything that became garbage *after* the request that made it, such as an application-scope graph replaced by a later request. |
| **When a large graph is replaced** | Overwriting or deleting a key in the application scope, a component's `static` scope or a class's shared structures re-enters the displaced graph into the log. If that's at least 1,000 containers (`RUSTCFML_GC_DISPLACE_SWEEP_MIN`), a cross-request sweep runs at that request's end, and retries at later request ends or thread exits (at least 5 s apart, up to 6 times) until it reclaims at least half of the displaced graph. This is what frees the old generation after a framework reload. |
| **Under memory pressure** | `--max-memory` shedding runs a cross-request sweep. |

**Threads.** A request, or a `cfthread` body, isn't swept while a thread it
started is still running, because the two share scopes. Its log is set aside
instead, and swept at a later request end, or by a timer every 2 seconds, once
those threads have finished.

**Bounds.** A request's log holds up to 4,000,000 entries (about 64 MB,
`RUSTCFML_GC_LOG_CAP`). At the cap, dead and duplicate entries are dropped. If
three quarters of the cap are still live after that, the request stops logging
for its remainder: it isn't swept further, and anything it leaves behind is
treated as live.

**Where it runs.** The collector is active in `--serve`. A one-shot CLI run
(`rustcfml file.cfm`) frees everything when the process exits, so the collector
stays off there and in the WebAssembly and Workers builds. While it's off, each
allocation pays one atomic read.

### What reclaiming looks like from outside

Freed memory goes back to the allocator, not necessarily to the operating
system. So a footprint graph doesn't fall when an application generation is
freed; it plateaus, at around 1.6× the live data. A **leak** is a footprint that
keeps rising, reload after reload, with no plateau.

## Where memory goes

### Compiled code and engine caches

| Cache | Bound |
|---|---|
| Compiled bytecode | one entry per compiled file, for the life of the process (rechecked against the file's modification time unless `--production`) |
| Component, custom tag, `Application.cfc` and canonical-path lookups | one entry per path, populated in `--production` only |
| Component class tables and `static` scopes | one per component file, replaced when the file is recompiled |
| File-exists lookups | 100,000 entries, cleared when full |
| Directory case-folding | 10,000 entries |
| Regular expressions (`reFind`, `reReplace`, …, and the `java.util.regex` shim) | 256 patterns, least recently used evicted; each pattern's lazy DFA capped at 256 KiB |
| Glob patterns (`directoryList` filters) and URL-rewrite rules | 1,024 patterns each, cleared when full |
| `evaluate()` expressions | 2,048 expressions, cleared when full |
| Interned identifier names | grows with the distinct names the application uses, never pruned |
| MySQL prepared statements | 256 per connection, least recently used evicted |

All of these are sized by the application's code, not its traffic, except the
`evaluate()` and regex caches, which are bounded for that reason.

### Application data

- **The application, server and session scopes** hold whatever the application
  puts in them, for as long as it keeps it there.
- **`cachePut` / `cacheGet`** (the object cache) has no entry-count limit. An
  expired entry is removed when it is next read; nothing sweeps the cache in the
  background, so entries that are never read again stay until the server
  restarts or the application removes them.
- **In-memory sessions** have no count limit. Expired sessions are removed when
  read and by a background reaper (every 60 s by default). A serializing
  session store (datasource, Memcached, cluster) keeps them out of process. See
  [Sessions](sessions.md).
- **Ehcache-backed caches** (through the Java shim) honour their
  `maxObjects` / `maxSizeInMb` settings.

### Threads and scheduled tasks

- Every request and every `cfthread` runs on a VM of its own, which is dropped
  when the work finishes.
- Request and thread stacks reserve 64 MB of virtual address space each. That is
  address space, not memory: only the pages a deep call chain actually touches
  are committed.
- A periodic scheduled task (`scheduleAtFixedRate` and the like) keeps what it
  needs to run (its task, a copy of the scheduling page's `variables`, the
  application scope) for as long as the schedule lives. Shutting the executor
  down, or dropping it, cancels its schedules.

## Monitoring

### Prometheus

With `observability.metrics` enabled (see
[Debugging & observability](debugging.md#prometheus-metrics-only)), turning on
`jvmCompatibility` adds the engine's memory and collector figures under the
names a Lucee container's JMX exporter uses, so existing JVM dashboards keep
working:

```json
{
  "observability": {
    "enabled": true,
    "metrics": { "enabled": true, "jvmCompatibility": true }
  }
}
```

| Metric | Value |
|---|---|
| `java_lang_Memory_HeapMemoryUsage_used` | the process footprint in bytes, measured exactly as `--max-memory` measures it (the table above), whether or not a limit is set |
| `java_lang_GarbageCollector_CollectionTime` | cumulative time spent in cycle-collector sweeps, in milliseconds; `rate(java_lang_GarbageCollector_CollectionTime[3m]) / 180` is the share of time spent collecting, as for a JVM |
| `java_lang_OperatingSystem_ProcessCpuLoad` | process CPU since the previous scrape, 0–1 across the available cores |

The footprint gauge is the one to alert on: it is the number the limit and the
OOM killer act on. Graph it against request rate. A plateau after reloads is the
allocator; a line that keeps climbing is worth investigating with the
diagnostics below.

### Logs

With a limit set, the server logs when it starts and stops refusing requests,
and every abort, with the messages shown in [The two tiers](#the-two-tiers).
Nothing is logged in normal operation.

### What isn't exposed

The engine keeps internal counts of nodes collected, survivors carried across
requests, requests refused and requests aborted, but none of them is published
as a metric yet.

From CFML, `createObject("java", "java.lang.Runtime")` exists for code that
expects it, but `freeMemory()`, `totalMemory()` and `maxMemory()` return fixed
values and `gc()` does nothing, so don't build monitoring on them.

## Diagnosing a footprint

Work through these in order on a test instance; the diagnostics cost real time
and are not meant for production.

1. **Is it a leak?** Graph the footprint across several reloads. A plateau is
   allocator retention. A climb with no plateau is a leak.
2. **Is the engine still tracking more each time?** Run with
   `RUSTCFML_GC_DEBUG=1` and compare the `live=` counts after each reload. If
   they return to the same number while the footprint climbs, the memory is in
   the allocator, not the application. If they climb, something is holding the
   old generation.
3. **What is holding it?** `RUSTCFML_CACHE_CENSUS=1` prints the size of every
   engine cache and of each application scope, broken down by what it holds.
   `RUSTCFML_GC_ROOTS=20` names the references that keep the largest survivors
   alive, such as which scope and key.
4. **Which code allocated it?** A `--memprofile` build samples allocations by
   call stack.

### Diagnostic variables

All are read once at startup and print to stderr. Those marked *presence* turn
on with any value, `0` included.

| Variable | Prints |
|---|---|
| `RUSTCFML_GC_DEBUG` (presence) | A line per sweep, request end, displacement, deferred sweep and memory-limit shed. Also records where every background task was created, which is slow. |
| `RUSTCFML_GC_ROOTS=N` | On every sweep, the N most common reasons survivors are pinned and the N largest holders, with the scope or key that holds them. |
| `RUSTCFML_CACHE_CENSUS` (presence) | At request end: every engine cache's size, live component class tables, interned names and each application scope's approximate size by shape. |
| `RUSTCFML_GC_SAMPLE=N` | With `RUSTCFML_CACHE_CENSUS`, the 25 code sites that allocate the most structs and arrays, sampling 1 in N allocations. |
| `RUSTCFML_GC_UNREACHABLE_REPORT` (presence) | During cross-request sweeps, the survivors no known scope reaches. Reports only. |
| `RUSTCFML_RELOG_DEBUG` (presence) | How many containers each application-scope overwrite re-entered into the log. |
| `RUSTCFML_COUNTERS=1` | When the server shuts down (or a CLI run ends): counts of structs allocated tracked and untracked, frames, component instances created and component lookups. |

A few lines of `RUSTCFML_GC_DEBUG` output (the numbers are illustrative):

```
[cycle_gc] request end: live_threads=0 running=0 log_len=Some(48211) (structs=40112 arrays=6903 queries=12 scopes=1184) deferred_pending=0
[cycle_gc] survivors=48211 live=1520 collected=46691
[cycle_gc] incremental minor sweep over 100000 reclaimed 91822 node(s); 8178 promoted, old=8178 next major at 100000 (14 ms)
[cycle_gc] generation displaced (20000 nodes re-entered): sweep pending
[cycle_gc] displacement sweep #1 reclaimed 111204 node(s); done
[cycle_gc] cross-request sweep reclaimed 2311 node(s); 53002 still tracked, next sweep at 106004 (38 ms)
```

### Heap profiling

A build with the `memprofile` feature (Unix) samples allocations by call stack:

```bash
cargo build --release -p rustcfml-cli --features memprofile
./target/release/rustcfml --serve ./webroot --memprofile
kill -USR2 <pid>      # write a snapshot without stopping; Ctrl+C writes a final one
```

Each snapshot writes `rustcfml-memprofile-N-inuse` (what is live now) and
`-alloc` (everything allocated), each as pprof `.pb` and as `.folded` stacks for
a flame graph. Sampling averages one sample per 256 KiB allocated
(`RUSTCFML_MEMPROFILE_RATE`, in bytes). A memprofile build runs on the system
allocator rather than mimalloc, so compare live sizes from it, not footprints.

`--profile` is a CPU profiler, not a memory one; see
[Debugging & observability](debugging.md).

## Tuning

The defaults were chosen by measurement on real applications (Preside, Wheels,
TestBox suites) and should not normally be changed. All are environment
variables read once at startup, and only take effect in `--serve`.

| Variable | Default | What it controls |
|---|---|---|
| `RUSTCFML_GC_INCREMENTAL` | `100000` | Containers a request may allocate before the next component construction runs a minor sweep. Lower holds less garbage mid-request at the cost of more sweeps. `0` turns mid-request sweeping off. |
| `RUSTCFML_GC_PERSISTENT` | `50000` | The smallest cross-request set that triggers a cross-request sweep; afterwards a sweep runs whenever the set doubles. `0` stops carrying survivors across requests, so anything that becomes garbage after its request is never freed. Not recommended. |
| `RUSTCFML_GC_DISPLACE_SWEEP_MIN` | `1000` | The size of a replaced application-scope graph that triggers a sweep straight away. `0` turns the trigger off, leaving the doubling rule. |
| `RUSTCFML_RELOG_BUDGET` | `20000` | The most containers one overwrite or delete re-enters into the log. Bounds the cost of replacing a large graph. |
| `RUSTCFML_GC_LOG_CAP` | `4000000` | Entries in a request's log before it is compacted (about 16 bytes each). |
| `RUSTCFML_MAX_MEMORY` | unset | Same as `--max-memory`. |

### Switches for isolating a problem

These turn behaviour off to test whether it is responsible. Don't run with them.

| Variable | Effect |
|---|---|
| `RUSTCFML_NO_CYCLE_GC=1` | Never start the collector: only reference counting frees memory, so every cycle leaks. Also stops the end-of-request shed under `--max-memory` (the admission check and watchdog still shed). |
| `RUSTCFML_GC_PERSISTENT_ALWAYS` (presence) | Run a cross-request sweep at every request end. |
| `RUSTCFML_GC_TRACK_ALL` (presence) | Log every struct, including the short-lived ones the engine can prove never escape a function. Slow. |
| `RUSTCFML_NO_UNTRACKED_ARGS` (presence) | Log every function's `arguments` scope, including those that can't escape. |
| `RUSTCFML_NO_PERIODIC` (presence) | Run every periodic scheduled task once instead of on its schedule, to test whether long-lived schedules are holding memory. |
| `RUSTCFML_NO_COMPONENT_CACHE=1` | Turn off the `--production` component-path cache. |

### Allocator retention

The release binary uses **mimalloc** as its allocator, which is worth roughly
15% on a warm request. mimalloc keeps memory it has been given rather than
returning it to the OS promptly, so a server's footprint settles at a plateau
above its live data: a rounding error on a large application, but on a small
one it can roughly double the idle footprint. The engine asks mimalloc to
return retained memory only when shedding under `--max-memory`; doing it on
every request cost 31% throughput ([#354](https://github.com/RustCFML/RustCFML/issues/354)).

Two of mimalloc's own options recover most of the plateau. mimalloc reads them
itself, so they work on the binary as shipped:

```bash
MIMALLOC_ARENA_EAGER_COMMIT=0   # don't commit arena memory up front
MIMALLOC_PURGE_DELAY=0          # return freed memory immediately, not after 10 ms
```

**They are not the default, because the cost is real.** `PURGE_DELAY=0` hands
every freed block back to the OS immediately, which is exactly what an
allocation-heavy request does constantly. Measured on macOS arm64,
`--production`, `ab -c4`, three interleaved rounds:

| Workload | | req/s | Server CPU | RSS |
|---|---|---|---|---|
| 4k-row query → structs → JSON → 3k-line report | defaults | **178** | 6.58 s | 190 MiB |
| | both options | **122** | 9.59 s | 191 MiB |
| trivial page | defaults | **6541** | 0.10 s | 69 MiB |
| | both options | **5846** | 0.17 s | 69 MiB |

That is −31% throughput and +46% CPU on the allocation-heavy workload with no
RSS saving at all, and −11% even on a trivial page. In the other direction, the
same options took the engine's own test suite in serve mode from 501 MiB to
453 MiB, and [#354](https://github.com/RustCFML/RustCFML/issues/354) reports
156 → 101 MiB on a routing-heavy application on Linux with throughput
unchanged.

Use them when idle footprint is the constraint and the application is routing-
or IO-bound, and measure your allocation-heavy endpoints first.

## Builds without the collector

The WebAssembly and Cloudflare Workers builds use reference counting only: no
cycle collector, no mimalloc, no `--max-memory` and no metrics endpoint.
Cycles created there are reclaimed only when the instance itself goes away. See
[WebAssembly](wasm.md).
