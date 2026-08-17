#!/usr/bin/env python3
"""Interleaved ABBA A/B of two rustcfml binaries on a live site's warm renders.

Measures SERVER CPU SECONDS, not wall clock — on a loaded machine wall clock has
swamped real effects repeatedly. Boots each binary fresh per leg, discards boot,
then times a fixed number of warm renders with a cache-busting query string.

Reports mean, best case, and the A-to-A spread. **A delta smaller than the
A-to-A spread is not a result.** Prefer the adjacent-pair median when legs drift.

  BIN_A=/path/to/a BIN_B=/path/to/b SITE=/path/to/site \
    LEGS=ABBAABBAABBA RENDERS=200 python3 scripts/perf/ab_warm.py

Shutdown is SIGINT, never SIGTERM: the engine only handles ctrl_c(), and a
SIGTERM'd Preside leaves a startup lock held so the NEXT leg boots forever —
one bad leg silently poisons every leg after it.
"""
import subprocess, time, sys, os, statistics, urllib.request, signal

SITE = os.environ.get("SITE", "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website")
SCRATCH = os.path.dirname(os.path.abspath(__file__))
RENDERS = int(os.environ.get("RENDERS", "150"))
LEGS = os.environ.get("LEGS", "ABBAAB")


def cpu_seconds(pid):
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)],
                         capture_output=True, text=True).stdout.strip()
    if not out:
        return None
    parts = out.split(":")
    parts = [float(p) for p in parts]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


def fetch(port, q, timeout=300):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/?cb={q}", timeout=timeout) as r:
        return len(r.read())


def leg(binary, port, tag):
    log = open(f"{SCRATCH}/ab_{tag}.log", "w")
    p = subprocess.Popen([binary, "--serve", SITE, "--port", str(port), "--production"],
                         stdout=log, stderr=subprocess.STDOUT)
    try:
        for _ in range(600):                       # wait for boot + first render
            time.sleep(1)
            try:
                fetch(port, "boot")
                break
            except Exception:
                if p.poll() is not None:
                    raise RuntimeError(f"{tag}: server DIED during boot")
                continue
        else:
            raise RuntimeError(f"{tag}: server never answered")
        fetch(port, "warmup")
        t0 = cpu_seconds(p.pid)
        w0 = time.time()
        for i in range(RENDERS):
            fetch(port, f"{tag}{i}")
        wall = time.time() - w0
        t1 = cpu_seconds(p.pid)
        return (t1 - t0) / RENDERS * 1000.0, wall / RENDERS * 1000.0
    finally:
        p.send_signal(signal.SIGINT)
        try:
            p.wait(timeout=120)
        except subprocess.TimeoutExpired:
            p.kill()
        log.close()


BIN = {"A": os.environ.get("BIN_A", f"{SCRATCH}/rustcfml_base"),
       "B": os.environ.get("BIN_B", f"{SCRATCH}/rustcfml_key")}
NAME = {"A": os.environ.get("NAME_A", "A"), "B": os.environ.get("NAME_B", "B")}
res = {"A": [], "B": []}
port = int(os.environ.get("PORT0", "8850"))
for i, side in enumerate(LEGS):
    port += 1
    cpu, wall = leg(BIN[side], port, f"{side}{i}")
    res[side].append(cpu)
    print(f"leg {i+1} [{side}] {NAME[side]:>18}: {cpu:7.3f} ms CPU/render  "
          f"({wall:7.3f} ms wall)", flush=True)

print()
for s in "AB":
    v = res[s]
    print(f"{NAME[s]:>18}: mean {statistics.mean(v):7.3f}  best {min(v):7.3f}  "
          f"legs {[round(x,3) for x in v]}")
ma, mb = statistics.mean(res["A"]), statistics.mean(res["B"])
spread_a = max(res["A"]) - min(res["A"])
print(f"\ndelta (mean): {(mb-ma)/ma*100:+.2f}%   "
      f"best-case: {(min(res['B'])-min(res['A']))/min(res['A'])*100:+.2f}%")
print(f"A-to-A spread: {spread_a:.3f} ms ({spread_a/ma*100:.2f}%) — "
      f"a delta below this is not a result")
