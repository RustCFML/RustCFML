---
name: profile-cfml-app
description: Find where a CFML application running on RustCFML actually spends its time, and verify a change made it faster. Use when someone asks why their CFML app, site, or request is slow, wants to profile or benchmark it, wants a flamegraph, or wants to check whether an optimisation actually helped. Covers the debug footer, the sampling profiler, the native --profile flamegraph, request counters, and how to A/B a change without fooling yourself.
---

# Profiling a CFML application on RustCFML

Work top-down: find the slow *request*, then the slow *function*, then the slow
*line*. Do not start by optimising something that looks expensive — on this
engine that has been wrong far more often than it has been right.

## 0. Rules that decide whether the answer is real

These are not style preferences. Every one of them has produced a wrong
conclusion in this codebase.

1. **Measure CPU seconds, not wall clock**, unless latency itself is the
   question. On a busy machine wall clock has swamped real effects repeatedly.
2. **Interleave A/B legs** (`ABBAAB…`), never "before" then "after". Machines
   drift. Report the A-to-A spread; *a delta smaller than that spread is not a
   result.*
3. **Separate boot from warm.** First-request costs (component resolution,
   metadata, migrations) can be 100x the warm cost and will dominate a naive
   average. A cost that looks like a per-request problem is often a boot problem.
4. **A timing instrument has a resolution floor** (~17 ns per clock read here).
   A phase measuring under ~35 ns is measuring the instrument. Report it as "not
   measurable", not as a small number.
5. **Prefer counters to timers.** "How many times does this run?" is exact and
   free; "how long does it take?" is neither. Most real levers here were found by
   counting, then confirmed by timing.
6. **Chase allocations, not instruction counts.** Pure instruction-count savings
   have measured ~zero repeatedly; allocation churn has paid.

## 1. Which request is slow — the debug footer

Cheapest first look. Per-request totals, template/BIF counts, query list and
timings, rendered at the bottom of the page. Enable a `debugging` block in
config; see `docs/debugging.md` for the four activation gates and the URL
trigger.

Read in this order: **total ms**, then **query ms vs application ms** (a query
problem and a CFML problem need completely different fixes), then **templates
executed**. Template count is the best single proxy for CFML work — on a real
Preside site the public front end executes ~1,600 templates per render while an
admin page executes ~11,000, and the ~4x difference in time follows it almost
exactly.

Compare a **cold** and a **warm** hit of the same URL before concluding anything
(rule 3).

## 2. Which function is slow — the sampling profiler

Threshold-gated, FusionReactor-style: when a request exceeds a threshold a
watchdog asks that request's VM to snapshot its CFML call stack on an interval,
folded into a call tree with self/total percentages. Off by default and free when
off. Enable under `observability.profiler` (`docs/debugging.md`).

- `profileNow()` — force-profile the current request
- `getRequestProfile()` — the folded tree as a struct
- `GET /__rustcfml/profiler` — recent slow requests as JSON, in serve mode

Use it when one request type is slow and you do not know which function is
responsible. Every frame is interpreted, so all of them are attributed.

## 3. Where the native time goes — `--profile`

A CPU/wall-clock sampling profiler over the *engine*, emitting a flamegraph on
graceful shutdown (`docs/debugging.md`). Use when the CFML-level view says the
time is not in any one CFML function — i.e. it is spread across engine
machinery. Drive representative load, then **Ctrl+C**; the flamegraph is written
on graceful shutdown only.

## 4. Continuous / production — OpenTelemetry

OTLP traces plus native Prometheus metrics for RED-style monitoring, for finding
slow routes in production rather than reproducing locally. See
`docs/debugging.md` and `docs/observability-ops.md`.

## 5. Engine-level counters — `RUSTCFML_COUNTERS=1`

Cumulative engine counters printed after every request in serve mode. Because
they are cumulative, **diff two consecutive reports to isolate one request**, and
diff an end-of-boot snapshot against the end of a run to separate boot from warm
(rule 3). This is the tool for "how many times does the engine do X per render",
and it is how most levers here have been sized.

## 6. Proving a change helped

Do not trust a single before/after run.

- Interleave legs and report the A-to-A spread (rule 2).
- Prefer **adjacent-pair medians** over means; they cancel monotonic drift, and
  a couple of slow legs can otherwise fabricate a several-percent "win".
- Use a **representative** workload — a real page on a real site, not a
  microbenchmark. A microbenchmark once reported a dispatch path at 44% that the
  real workload measured at 6.8%, because the bench only called trivial builtins.
- Quiesce the machine, or accept that small effects are unprovable on it. Check
  load before believing a sub-2% result.
- If two independent methods agree (e.g. a counter-derived estimate and an
  end-to-end A/B), that is worth more than either alone.

## 7. Making the engine itself faster for one workload (PGO)

Shipped binaries are PGO-built; `cargo build --release` is not. A user building
from source can train a profile on **their own** application, which is the case
PGO suits best — a profile trained on the workload that actually matters:

```bash
scripts/pgo-train.sh --site /path/to/your/site --port 8641
RUSTFLAGS="-Cprofile-use=$PWD/pgo/rustcfml.profdata" \
  cargo build --profile release-pgo -p rustcfml-cli
```

Then A/B it against a non-PGO build per section 6 — a new profile is not
automatically better. See `docs/pgo.md`. Note the training script's assertions
are the point: every PGO failure mode is silent (a profile that loads and is
simply worse), so treat a skipped assertion as a failed retrain.

## Applying this

Report what was measured, how, and what the noise floor was. If a result does not
clear its noise floor, say so plainly rather than quoting the favourable
estimator — an unproven number that gets repeated becomes a fact that steers
later work. When a hypothesis is disproved, record it as dead so nobody pays for
it twice.
