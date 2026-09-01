#!/usr/bin/env python3
"""Concurrent load A/B — wall-clock latency distribution and throughput.

The companion to `xengine_ab.py`, and deliberately its opposite. That rig sends
requests SERIALLY and measures CPU time, because per-frame and per-op levers are
what it was built to see and competing load cannot steal CPU from a measurement.

Both of those choices make an entire class of defect invisible:

  * A sleeping thread burns ZERO CPU. Work that parks — a poll loop, a sleep, a
    timed retry — reports as free on a CPU-time rig however long it takes.
  * An uncontended lock is free. Anything whose cost only appears when two
    requests want the same resource cannot happen at concurrency 1.

GH #401 is the worked example: a contended `<cflock>` polled in a 10ms sleep
loop from 2026-03-02 and survived every perf campaign, because in the rig we had
`try_write()` always succeeded on the first attempt. It took an external user
profiling a real app under real traffic to find it.

So this rig measures WALL CLOCK, under CONCURRENCY, and reports the tail:

  * throughput (req/s) across a concurrency sweep — flat or falling throughput as
    workers rise is serialisation, whatever the mean latency says;
  * p50/p95/p99 latency — a poll interval shows up as quantised tail latency long
    before it shows up in a mean.

Usage
-----
    # A/B two builds of our own engine on the same workload
    scripts/perf/concurrency_ab.py before:8611 after:8612

    # Cross-engine
    scripts/perf/concurrency_ab.py rustcfml:8611 lucee:8585 --path /index.cfm

    # Sweep further, and hold the lock longer
    scripts/perf/concurrency_ab.py a:8611 b:8612 --levels 1,4,16,32 \
        --path '/index.cfm?iterations=500'

Serve the bundled workload with:

    rustcfml --serve --port 8611 --root scripts/perf/workloads/lock_contention

Fairness discipline, carried over from `xengine_ab.py`
-----------------------------------------------------
  * Arms are INTERLEAVED per round and the order alternates, so a box that gets
    busier partway through penalises both arms equally rather than whichever ran
    second.
  * Every request carries a cache-buster. `--production` will happily serve a
    stale page and report a beautiful, meaningless number.
  * Never time on a bif-census or otherwise instrumented build.
  * Warm-up runs at the measured concurrency, not serially — first-hit compiles
    and pool growth are not what is being measured.
"""
import argparse
import statistics as st
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor


def parse_arms(specs):
    arms = []
    for spec in specs:
        if ":" not in spec:
            raise SystemExit(f"bad arm {spec!r} — expected label:port")
        label, port = spec.rsplit(":", 1)
        arms.append((label, int(port)))
    return arms


def one_request(url):
    """Return (latency_seconds, ok). Errors are counted, never raised — a rig
    that dies on the first 500 tells you nothing about the other 999."""
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            r.read()
            ok = r.status == 200
    except (urllib.error.URLError, urllib.error.HTTPError, OSError):
        ok = False
    return time.perf_counter() - t0, ok


def run_level(port, path, n, workers, counter=[0]):
    """Fire n requests with `workers` in flight. Returns (latencies, ok, wall)."""
    sep = "&" if "?" in path else "?"
    urls = []
    for _ in range(n):
        counter[0] += 1
        urls.append(f"http://127.0.0.1:{port}{path}{sep}cb={time.time_ns()}_{counter[0]}")

    t0 = time.perf_counter()
    with ThreadPoolExecutor(max_workers=workers) as pool:
        results = list(pool.map(one_request, urls))
    wall = time.perf_counter() - t0

    lat = [r[0] for r in results]
    ok = sum(1 for r in results if r[1])
    return lat, ok, wall


def pct(values, p):
    """Nearest-rank percentile — no interpolation, so a reported p99 is a real
    observed request and not an average of two."""
    if not values:
        return float("nan")
    s = sorted(values)
    k = max(0, min(len(s) - 1, int(round(p / 100.0 * len(s) + 0.5)) - 1))
    return s[k]


def main():
    ap = argparse.ArgumentParser(
        description="Concurrent load A/B: wall-clock tail latency and throughput.")
    ap.add_argument("arms", nargs="+", metavar="label:port")
    ap.add_argument("--path", default="/index.cfm", help="request path (default %(default)s)")
    ap.add_argument("--levels", default="1,2,4,8,16",
                    help="concurrency sweep (default %(default)s)")
    ap.add_argument("--requests", type=int, default=200,
                    help="requests per arm per level per round (default %(default)s)")
    ap.add_argument("--rounds", type=int, default=3,
                    help="rounds per level, arms interleaved (default %(default)s)")
    ap.add_argument("--warmup", type=int, default=100,
                    help="warm-up requests per arm (default %(default)s)")
    args = ap.parse_args()

    arms = parse_arms(args.arms)
    levels = [int(x) for x in args.levels.split(",") if x.strip()]

    print(f"== workload {args.path} ==")
    print(f"   arms      {', '.join(f'{l} :{p}' for l, p in arms)}")
    print(f"   levels    {levels}")
    print(f"   {args.requests} req/arm/level x {args.rounds} rounds, interleaved\n")

    print(f"== warm-up ({args.warmup} req/arm at concurrency 4) ==")
    for label, port in arms:
        lat, ok, wall = run_level(port, args.path, args.warmup, 4)
        if ok == 0:
            raise SystemExit(
                f"  {label}: 0/{args.warmup} ok — is it serving {args.path} on :{port}?")
        print(f"  {label:<12} {ok}/{args.warmup} ok, {wall:.1f}s")

    # level -> label -> merged latencies / throughputs
    lat_by = {n: {l: [] for l, _ in arms} for n in levels}
    rps_by = {n: {l: [] for l, _ in arms} for n in levels}
    err_by = {n: {l: 0 for l, _ in arms} for n in levels}

    for n in levels:
        print(f"\n== concurrency {n} ==")
        for r in range(args.rounds):
            order = arms if r % 2 == 0 else list(reversed(arms))
            line = []
            for label, port in order:
                lat, ok, wall = run_level(port, args.path, args.requests, n)
                lat_by[n][label].extend(lat)
                rps_by[n][label].append(args.requests / wall if wall > 0 else 0.0)
                err_by[n][label] += args.requests - ok
                line.append(f"{label} {args.requests/wall:7.1f} rps "
                            f"p50 {pct(lat,50)*1000:6.1f}ms")
            print(f"  r{r+1:<2} " + " | ".join(line))

    print("\n== RESULT ==")
    hdr = f"  {'level':>5}  {'arm':<12} {'rps':>8}  {'p50':>8} {'p95':>8} {'p99':>8}  {'err':>4}"
    print(hdr)
    print("  " + "-" * (len(hdr) - 2))
    for n in levels:
        for label, _ in arms:
            lat = lat_by[n][label]
            rps = st.median(rps_by[n][label])
            print(f"  {n:>5}  {label:<12} {rps:8.1f}  "
                  f"{pct(lat,50)*1000:7.1f}m {pct(lat,95)*1000:7.1f}m "
                  f"{pct(lat,99)*1000:7.1f}m  {err_by[n][label]:4d}")

    # Scaling: how throughput moved from the lowest level to the highest. Flat or
    # falling is serialisation — the thing this rig exists to expose.
    lo, hi = levels[0], levels[-1]
    if lo != hi:
        print(f"\n== scaling {lo} -> {hi} workers ==")
        for label, _ in arms:
            a, b = st.median(rps_by[lo][label]), st.median(rps_by[hi][label])
            ideal = hi / lo
            print(f"  {label:<12} {a:7.1f} -> {b:7.1f} rps = {b/a:5.2f}x "
                  f"(ideal {ideal:.0f}x, {b/a/ideal*100:5.1f}% of linear)")

    if len(arms) == 2:
        print("\n== A/B ==")
        (la, _), (lb, _) = arms
        for n in levels:
            ra, rb = st.median(rps_by[n][la]), st.median(rps_by[n][lb])
            pa, pb = pct(lat_by[n][la], 99), pct(lat_by[n][lb], 99)
            print(f"  c{n:<3} throughput {lb}/{la} = {rb/ra:5.2f}x   "
                  f"p99 {pa*1000:7.1f}ms -> {pb*1000:7.1f}ms")


if __name__ == "__main__":
    sys.exit(main())
