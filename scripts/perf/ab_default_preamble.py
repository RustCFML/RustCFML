#!/usr/bin/env python3
"""Interleaved ABBA A/B of the defaulted-parameter preamble on live Preside.

A = RCFML_LEGACY_DEFAULT_PREAMBLE=1 (the six-op sequence with LoadLocal("arguments"))
B = default                          (the fused SeedArgumentKey op)

ONE binary serves both arms via the env switch, so build-to-build variance --
larger than the effect on this box -- cannot contaminate the result.

Protocol per the roadmap's Rule 8: interleaved ABBA, adjacent-pair medians, and
the A-to-A spread reported. A delta smaller than the A-to-A spread is not a
result. Keep-alive on one connection (Rule 6: a per-request client spawn adds
~10 ms and fakes an I/O stall).

Preside boot can need up to 3 attempts after a cross-engine session -- the
dbsync converges (see feedback_part1_campaign_measurement_traps).
"""
import argparse
import http.client
import os
import signal
import statistics
import subprocess
import sys
import time

BIN = "./target/release/rustcfml"


def boot(port, site, legacy, logdir):
    env = dict(os.environ)
    env.pop("RUSTCFML_COUNTERS", None)          # counters perturb what we measure
    if legacy:
        env["RCFML_LEGACY_DEFAULT_PREAMBLE"] = "1"
    else:
        env.pop("RCFML_LEGACY_DEFAULT_PREAMBLE", None)
    log = open(f"{logdir}/ab_{'A' if legacy else 'B'}_{port}.log", "w")
    p = subprocess.Popen(
        [BIN, "--serve", site, "--port", str(port), "--production"],
        stdout=log, stderr=subprocess.STDOUT, env=env)
    for attempt in range(8):
        for _ in range(90):
            try:
                c = http.client.HTTPConnection("127.0.0.1", port, timeout=300)
                c.request("GET", f"/?cb=boot{attempt}")
                r = c.getresponse()
                body = r.read()
                c.close()
                if r.status == 200:
                    return p, len(body)
                break
            except Exception:
                time.sleep(2)
        time.sleep(1)
    p.send_signal(signal.SIGINT)
    sys.exit(f"port {port}: Preside never booted")


def measure(port, n, warm):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=120)
    for i in range(warm):
        c.request("GET", f"/?cb=warm{i}")
        c.getresponse().read()
    ts = []
    sizes = set()
    for i in range(n):
        t = time.perf_counter()
        c.request("GET", f"/?cb=m{i}")
        r = c.getresponse()
        b = r.read()
        ts.append((time.perf_counter() - t) * 1000)
        sizes.add(len(b))
        if r.status != 200:
            sys.exit(f"port {port}: HTTP {r.status} mid-run")
    c.close()
    # A render that suddenly gets 3x faster is usually not a faster render --
    # it is a DIFFERENT response. Surface the size set so that is visible.
    return ts, sizes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--site", default="/Users/alexskinner/Projects/Websites/"
                                      "readyintelligencewebsite/website")
    ap.add_argument("--legs", type=int, default=4)
    ap.add_argument("--renders", type=int, default=120)
    ap.add_argument("--warm", type=int, default=20)
    ap.add_argument("--logdir", default=".")
    # Arm is otherwise confounded with port. Run once each way to de-confound.
    ap.add_argument("--swap-ports", action="store_true")
    args = ap.parse_args()

    ports = {"A": 8662, "B": 8661} if args.swap_ports else {"A": 8661, "B": 8662}
    legs = {"A": [], "B": []}
    order = []
    for i in range(args.legs):
        order += [("A", "B")] if i % 2 == 0 else [("B", "A")]
    for first, second in order:
        # FRESH SERVERS PER LEG. Held across legs, Preside eventually serves the
        # homepage from a cache: the response size changes (5221/5222 ->
        # 5223/5224) and the time drops 21 ms -> 5.8 ms. That is a different
        # response, not a faster render, and it silently swamped the first
        # attempt at this A/B. Every earlier baseline in this repo was taken on
        # a fresh server inside the render regime, so match that.
        procs = {}
        try:
            for arm in (first, second):
                port = ports[arm]
                procs[arm], nb = boot(port, args.site, arm == "A", args.logdir)
            for arm in (first, second):
                port = ports[arm]
                ts, sizes = measure(port, args.renders, args.warm)
                legs[arm].append(statistics.median(ts))
                print(f"  leg {arm}: median {statistics.median(ts):.2f} ms "
                      f"(p10 {sorted(ts)[len(ts)//10]:.2f} / "
                      f"p90 {sorted(ts)[len(ts)*9//10]:.2f})  sizes={sorted(sizes)}",
                      file=sys.stderr)
        finally:
            for p in procs.values():
                p.send_signal(signal.SIGINT)  # never SIGTERM: holds the startup lock
            time.sleep(3)

    a, b = statistics.median(legs["A"]), statistics.median(legs["B"])
    spread_a = (max(legs["A"]) - min(legs["A"])) / a * 100
    spread_b = (max(legs["B"]) - min(legs["B"])) / b * 100
    print(f"\n=== defaulted-parameter preamble, live Preside warm homepage ===")
    print(f"A  legacy 6-op preamble : {a:8.2f} ms   (A-to-A spread {spread_a:.1f}%)")
    print(f"B  SeedArgumentKey      : {b:8.2f} ms   (B-to-B spread {spread_b:.1f}%)")
    print(f"delta                   : {b - a:+8.2f} ms  ({(b - a) / a * 100:+.2f}%)")
    floor = max(spread_a, spread_b)
    verdict = ("REAL" if abs((b - a) / a * 100) > floor
               else "NOT A RESULT (smaller than the noise floor)")
    print(f"noise floor (worst self-spread): {floor:.1f}%  ->  {verdict}")


if __name__ == "__main__":
    main()
