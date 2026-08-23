#!/usr/bin/env python3
"""Cross-engine CPU-time A/B on one live Preside site.

Both servers run concurrently on different ports, but requests go in ALTERNATING
BURSTS so each burst has the box essentially to itself. Per-arm CPU time is read
from the OS (`ps utime+stime`), which competing wall-clock load cannot steal.
Preside's background heartbeats burn CPU on both engines, so an idle rate is
measured per arm and subtracted from every burst.
"""
import subprocess, sys, time, statistics as st, random

def cpu_seconds(pid):
    out = subprocess.run(["ps","-o","utime=,stime=","-p",str(pid)],
                         capture_output=True, text=True).stdout.split()
    if len(out) < 2:
        raise SystemExit(f"pid {pid} gone")
    total = 0.0
    for f in out[:2]:
        parts = f.split(":")
        secs = float(parts[-1])
        if len(parts) >= 2: secs += float(parts[-2]) * 60
        if len(parts) >= 3: secs += float(parts[-3]) * 3600
        total += secs
    return total

def render(port, n):
    """Fire n renders; return (ok_count, wall_seconds)."""
    ok = 0
    t0 = time.time()
    for i in range(n):
        cb = f"{random.randint(0,10**9)}{i}"
        r = subprocess.run(["curl","-s","-o","/dev/null","-w","%{http_code}",
                            f"http://127.0.0.1:{port}/?cb={cb}"],
                           capture_output=True, text=True)
        if r.stdout.strip() == "200": ok += 1
    return ok, time.time() - t0

def idle_rate(pid, seconds):
    c0 = cpu_seconds(pid); time.sleep(seconds); c1 = cpu_seconds(pid)
    return (c1 - c0) / seconds

arms = []            # (label, port, pid)
for spec in sys.argv[1:]:
    label, port, pid = spec.split(":")
    arms.append((label, int(port), int(pid)))

WARM, BURST, ROUNDS = 400, 50, 10

print("== idle CPU baseline (15s each, Preside heartbeats) ==")
idle = {}
for label, port, pid in arms:
    idle[label] = idle_rate(pid, 15)
    print(f"  {label:<10} {idle[label]*1000:.2f} ms CPU/s idle")

print(f"\n== warm-up ({WARM} renders each; JVM JIT + caches) ==")
for label, port, pid in arms:
    ok, w = render(port, WARM)
    print(f"  {label:<10} {ok}/{WARM} ok, {w:.1f}s wall ({w/WARM*1000:.1f} ms/render)")

print(f"\n== {ROUNDS} rounds x {BURST} renders, alternating order ==")
res = {label: [] for label,_,_ in arms}
wall = {label: [] for label,_,_ in arms}
for r in range(ROUNDS):
    order = arms if r % 2 == 0 else list(reversed(arms))
    line = []
    for label, port, pid in order:
        c0 = cpu_seconds(pid)
        ok, w = render(port, BURST)
        c1 = cpu_seconds(pid)
        cpu = (c1 - c0) - idle[label] * w      # subtract heartbeat CPU
        per = cpu / BURST * 1000
        res[label].append(per); wall[label].append(w / BURST * 1000)
        line.append(f"{label} {per:6.2f} ms cpu / {w/BURST*1000:6.2f} ms wall ({ok}/{BURST})")
    print(f"  r{r+1:<2} " + " | ".join(line))

print("\n== RESULT (per render) ==")
summary = {}
for label,_,_ in arms:
    v = sorted(res[label]); w = sorted(wall[label])
    summary[label] = st.median(v)
    spread = (max(v)-min(v))/st.median(v)*100
    print(f"  {label:<10} CPU median {st.median(v):6.2f} ms  mean {st.mean(v):6.2f}  "
          f"spread {spread:5.1f}%   |  wall median {st.median(w):6.2f} ms")
if len(summary) == 2:
    (la, va), (lb, vb) = summary.items()
    hi, lo = (la, lb) if va > vb else (lb, la)
    print(f"\n  {hi} / {lo} = {max(va,vb)/min(va,vb):.2f}x CPU per render "
          f"({max(va,vb)-min(va,vb):+.2f} ms gap)")
