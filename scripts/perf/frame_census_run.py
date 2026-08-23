#!/usr/bin/env python3
"""Per-frame EXCLUSIVE cost of a REAL Preside render, boot subtracted.

Attribution (roadmap Part 2.6) puts a Preside frame at 1,276 ns against a
synthetic frame's 780 ns -- a +496 ns surcharge worth 28% of a warm admin
render that no phase table can see, because the call-phases body phase
recursively contains every nested frame.

`--features frame-census` gives each frame its own tally with children
subtracted. The counters are CUMULATIVE and the report only prints at
shutdown, so a single run is dominated by boot. This boots twice -- once with
`base` renders, once with `base + n` -- and diffs, leaving n warm renders.

  BIN=/path/to/bin_framecensus python3 scripts/perf/frame_census_run.py

Env: SITE, PORT, URL (default "/"), N (warm renders to isolate), BASE.
"""
import os, re, signal, socket, subprocess, sys, time, urllib.request

BIN  = os.environ.get("BIN")
SITE = os.environ.get("SITE", "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website")
PORT = int(os.environ.get("PORT", "8733"))
URL  = os.environ.get("URL", "/")
N    = int(os.environ.get("N", "40"))
BASE = int(os.environ.get("BASE", "4"))
TOP  = os.environ.get("TOP", "400")


def wait_up(port, path, timeout=300):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            with socket.create_connection(("127.0.0.1", port), 2):
                pass
            urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=120).read()
            return True
        except Exception:
            time.sleep(1)
    return False


def run(renders):
    """Boot, render `renders` times, SIGINT, return the parsed census."""
    env = dict(os.environ, RUSTCFML_COUNTERS="1", RCFML_FRAME_CENSUS_TOP=TOP)
    # stderr to a FILE, never a pipe: the server logs enough during a Preside
    # boot to fill the 64 KB pipe buffer, and a blocked write deadlocks the
    # process mid-render -- which looks exactly like an engine hang.
    errpath = os.path.join(os.environ.get("SCRATCH", "/tmp"), f"framecensus_{renders}.err")
    errf = open(errpath, "wb")
    p = subprocess.Popen([BIN, "--serve", SITE, "--port", str(PORT), "--production"],
                         stdout=subprocess.DEVNULL, stderr=errf, env=env)
    if not wait_up(PORT, f"{URL}?cb=boot"):
        p.kill(); sys.exit("server did not start")
    sizes = []
    for i in range(renders):
        sizes.append(len(urllib.request.urlopen(
            f"http://127.0.0.1:{PORT}{URL}?cb=r{i}", timeout=600).read()))
    p.send_signal(signal.SIGINT)
    p.wait()
    errf.close()
    err = open(errpath, "rb").read().decode("utf-8", "replace")
    return parse(err), sizes


ROW = re.compile(r"^\s*(\d+)\s+(\d+)\s+([\d.]+)\s+(\d+)\s+([\d.]+)\s+([\d.]+)\s+(\S.*)$")


def parse(err):
    rows, inside = {}, False
    for line in err.splitlines():
        if line.startswith("--- frame census:"):
            inside = True
            continue
        if inside and line.startswith("---"):
            inside = False
        if not inside:
            continue
        m = ROW.match(line)
        if m:
            calls, allocs, _, ns, _, _, name = m.groups()
            # self_ops is printed per frame; recover the total
            rows[name] = {"calls": int(calls), "allocs": int(allocs), "ns": int(ns)}
    return rows


def main():
    if not BIN:
        sys.exit("set BIN=<frame-census binary>")
    print(f"boot+{BASE} renders ...", flush=True)
    a, sa = run(BASE)
    print(f"boot+{BASE + N} renders ...", flush=True)
    b, sb = run(BASE + N)
    if not sa or not sb or min(sa) < 500 or min(sb) < 500:
        sys.exit(f"SANITY: body too small (min {min(sa or [0])}/{min(sb or [0])}) "
                 "-- a 302 to login looks exactly like a cheap workload")
    diff = []
    for name, r in b.items():
        base = a.get(name, {"calls": 0, "allocs": 0, "ns": 0})
        c = r["calls"] - base["calls"]
        if c <= 0:
            continue
        diff.append((name, c, r["allocs"] - base["allocs"], r["ns"] - base["ns"]))
    tc = sum(d[1] for d in diff)
    ta = sum(d[2] for d in diff)
    tn = sum(d[3] for d in diff)
    print(f"\n=== {N} warm renders of {URL} (boot subtracted), mean body {sum(sb)//len(sb)} bytes ===")
    print(f"frames/render   {tc / N:>12.0f}")
    print(f"self allocs/fr  {ta / max(tc, 1):>12.2f}")
    print(f"self ns/frame   {tn / max(tc, 1):>12.0f}   (instrumented build -- ratios, not absolutes)")
    print(f"allocs/render   {ta / N:>12.0f}")
    diff.sort(key=lambda d: -d[2])
    print(f"\n{'calls/r':>9} {'allocs':>10} {'al/frame':>9} {'ns/frame':>9}  function")
    for name, c, al, ns in diff[:40]:
        print(f"{c / N:>9.1f} {al:>10} {al / c:>9.2f} {ns / c:>9.0f}  {name}")


main()
