#!/usr/bin/env python3
"""Run bench/bifbench.cfm on both engines, interleaved, and print the ratio
table that decides H1 (PERFORMANCE_ROADMAP Part 1 Step 1).

H1: "individual BIF bodies are slower than Lucee's" -- ~1.6 ms of the 11.4 ms
gap. Decision rule from the roadmap: if >=5 BIFs are >2x AND together account
for >1 ms/render, open a body-optimisation track; otherwise H1 is dead.

Protocol:
  * Interleaved ABBA legs, adjacent-pair medians -- a delta smaller than the
    A-to-A spread is not a result.
  * Wall-clock in-engine timing only. Process CPU is INVALID cross-engine:
    Lucee's process CPU exceeds its own wall latency (JVM GC/JIT threads).
  * The RustCFML arm must be a CLEAN build. An instrumented one (bif-census
    takes a Mutex per builtin call) inflated every case by ~2.4us and made
    every BIF look identical.
  * Both arms must report the same SINK, or they did not compute the same work.
"""
import argparse
import statistics
import sys
import urllib.error
import urllib.request


def fetch(port, iters, passes, warmups, timeout=1800):
    url = (f"http://127.0.0.1:{port}/bench/bifbench.cfm"
           f"?iters={iters}&passes={passes}&warmups={warmups}")
    try:
        body = urllib.request.urlopen(url, timeout=timeout).read().decode()
    except urllib.error.HTTPError as e:
        body = e.read().decode()
    plain = body.split("<")[0]
    rows, meta = {}, {}
    for line in plain.splitlines():
        if "\t" in line:
            f = line.split("\t")
            if f[0] == "NAME":
                continue
            rows[f[0]] = (float(f[1]), int(f[2]))
        elif "=" in line:
            k, v = line.split("=", 1)
            meta[k] = v
    if not rows:
        sys.exit(f"port {port}: no rows parsed. First 400 chars:\n{plain[:400]}")
    return rows, meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rust-port", type=int, default=8612)
    ap.add_argument("--lucee-port", type=int, default=8585)
    ap.add_argument("--iters", type=int, default=200000)
    ap.add_argument("--passes", type=int, default=5)
    ap.add_argument("--warmups", type=int, default=3)
    ap.add_argument("--legs", type=int, default=3)
    args = ap.parse_args()

    legs = {"rust": [], "lucee": []}
    sinks, metas = {}, {}
    # ABBA so a drift in machine load cannot land entirely on one engine.
    order = []
    for i in range(args.legs):
        order += ["rust", "lucee"] if i % 2 == 0 else ["lucee", "rust"]
    for arm in order:
        port = args.rust_port if arm == "rust" else args.lucee_port
        rows, meta = fetch(port, args.iters, args.passes, args.warmups)
        legs[arm].append(rows)
        sinks.setdefault(arm, set()).add(meta.get("SINK"))
        metas[arm] = meta
        print(f"  leg {arm:<5} baseline {meta.get('BASELINE_NS'):>10} ns  "
              f"sink {meta.get('SINK')}", file=sys.stderr)

    if sinks["rust"] != sinks["lucee"]:
        print(f"\n!! SINKS DIFFER -- the arms did not compute the same work: "
              f"rust={sinks['rust']} lucee={sinks['lucee']}", file=sys.stderr)

    names = list(legs["rust"][0].keys())
    print(f"\n=== PER-BIF CROSS-ENGINE COST (H1) ===")
    print(f"RustCFML port {args.rust_port} vs Lucee port {args.lucee_port}; "
          f"{args.legs} legs x {args.passes} passes x {args.iters} iters, "
          f"{args.warmups} warmups; median of legs, net of an empty loop")
    print(f"{'BIF':<18}{'rust ns':>9}{'lucee ns':>10}{'ratio':>8}"
          f"{'calls/req':>11}{'rust ms':>9}{'lucee ms':>10}{'gap ms':>9}")
    total = {"rust": 0.0, "lucee": 0.0}
    over2 = []
    out_rows = []
    for n in names:
        r = statistics.median(l[n][0] for l in legs["rust"])
        u = statistics.median(l[n][0] for l in legs["lucee"])
        calls = legs["rust"][0][n][1]
        rms, ums = r * calls / 1e6, u * calls / 1e6
        total["rust"] += rms
        total["lucee"] += ums
        ratio = (r / u) if u > 0.5 else float("inf")
        out_rows.append((rms - ums, n, r, u, ratio, calls, rms, ums))
        if ratio > 2.0:
            over2.append((n, ratio, rms - ums))
    out_rows.sort(reverse=True)
    for gap, n, r, u, ratio, calls, rms, ums in out_rows:
        rs = f"{ratio:>7.2f}x" if ratio != float("inf") else "     inf"
        print(f"{n:<18}{r:>9.1f}{u:>10.1f}{rs}{calls:>11}"
              f"{rms:>9.4f}{ums:>10.4f}{gap:>9.4f}")
    print(f"{'TOTAL':<18}{'':>9}{'':>10}{'':>8}{'':>11}"
          f"{total['rust']:>9.4f}{total['lucee']:>10.4f}"
          f"{total['rust'] - total['lucee']:>9.4f}")

    print(f"\n--- H1 decision rule: >=5 BIFs over 2x AND together >1 ms/render ---")
    big = [x for x in over2 if x[2] > 0]
    print(f"BIFs over 2x: {len(over2)}  "
          f"({', '.join(f'{n} {r:.1f}x' for n, r, _ in sorted(over2, key=lambda t: -t[1])[:8])})")
    print(f"their combined gap: {sum(x[2] for x in big):.4f} ms/render")
    print(f"ALL-BIF gap vs Lucee: {total['rust'] - total['lucee']:.4f} ms/render "
          f"(of the 11.4 ms total request gap = "
          f"{(total['rust'] - total['lucee']) / 11.4 * 100:.1f}%)")

    # A-to-A spread: how much one engine's own legs disagree. A cross-engine
    # delta smaller than this is not a result.
    for arm in ("rust", "lucee"):
        if len(legs[arm]) > 1:
            tots = [sum(l[n][0] * l[n][1] / 1e6 for n in names) for l in legs[arm]]
            spread = (max(tots) - min(tots)) / statistics.median(tots) * 100
            print(f"{arm} A-to-A spread over {len(tots)} legs: {spread:.1f}% "
                  f"({min(tots):.3f}-{max(tots):.3f} ms)")


if __name__ == "__main__":
    main()
