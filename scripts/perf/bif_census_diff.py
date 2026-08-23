#!/usr/bin/env python3
"""Isolate ONE request's builtin mix (and op mix) from a `--features
bif-census,op-census` server log.

Both censuses accumulate process-wide, and Preside's boot executes ~150x the
work of a warm render, so a raw report is a report about BOOT. The warm mix only
appears when two consecutive per-request dumps are subtracted -- and the two
mixes genuinely differ (compareNoCase is ~19% of the cumulative table and absent
from the warm top 12), so the subtraction is the measurement, not a tidy-up.

Usage:
    bif_census_diff.py <server.log> [--from N] [--to M] [--top K] [--ops]

`--from/--to` index the per-request dumps (0 = the boot request). Defaults to
the last two, i.e. one warm render.
"""
import argparse
import re
import sys

KIND_NAMES = ["null", "bool", "int", "double", "string", "array", "struct",
              "query", "fn", "binary", "other"]
POSITIONS = 4
# per-arg fields in a BIFRAW line: seen, k0..k10, size_sum, size_n, size_max
ARG_FIELDS = 1 + len(KIND_NAMES) + 3


def parse_raw_dumps(path):
    """Every `--- BIFRAW BEGIN ---` block, as {name: stat-dict}."""
    dumps, cur = [], None
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        if line.startswith("--- BIFRAW BEGIN"):
            cur = {}
        elif line.startswith("--- BIFRAW END"):
            if cur is not None:
                dumps.append(cur)
            cur = None
        elif cur is not None and line.startswith("BIFRAW\t"):
            f = line.split("\t")
            name, calls = f[1], int(f[2])
            arity = [int(x) for x in f[3].split(",")]
            args = []
            for i in range(POSITIONS):
                vals = [int(x) for x in f[4 + i].split(",")]
                assert len(vals) == ARG_FIELDS, (name, i, len(vals))
                args.append(vals)
            cur[name] = {"calls": calls, "arity": arity, "args": args}
    return dumps


def parse_op_dumps(path):
    """Every `--- dynamic op census` block, as {op_name: count}."""
    row = re.compile(r"^\s*(\d+)\s+[\d.]+%\s+[\d.]+%cum\s+(\S+)\s*$")
    dumps, cur = [], None
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        if line.startswith("--- dynamic op census"):
            if cur is not None:
                dumps.append(cur)
            cur = {}
            continue
        if cur is None:
            continue
        m = row.match(line)
        if m:
            cur[m.group(2)] = int(m.group(1))
        elif line.startswith("---") or line.startswith("==="):
            dumps.append(cur)
            cur = None
    if cur:
        dumps.append(cur)
    return dumps


def sub(b, a):
    """b - a for one builtin's accumulators (a may be absent)."""
    if a is None:
        return b
    out = {
        "calls": b["calls"] - a["calls"],
        "arity": [x - y for x, y in zip(b["arity"], a["arity"])],
        "args": [[x - y for x, y in zip(bb, aa)]
                 for bb, aa in zip(b["args"], a["args"])],
    }
    # size_max is a MAX, not a sum -- subtracting two of them is nonsense
    # (it yields ~0 and hides the outlier the field exists to expose). Carry
    # the later dump's cumulative max through instead: an upper bound over
    # boot+warm, labelled as such rather than silently wrong.
    for i, arg in enumerate(out["args"]):
        arg[-1] = b["args"][i][-1]
    return out


def fmt_arg(vals):
    seen = vals[0]
    if seen == 0:
        return None
    kinds = vals[1:1 + len(KIND_NAMES)]
    size_sum, size_n, size_max = vals[-3:]
    parts = [f"{KIND_NAMES[i]}={c / seen * 100:.0f}%"
             for i, c in enumerate(kinds) if c > 0]
    size = ""
    if size_n > 0:
        size = f"  size mean {size_sum / size_n:.1f} max<={size_max}"
    return " ".join(parts) + size


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("log")
    ap.add_argument("--from", dest="a", type=int, default=-2)
    ap.add_argument("--to", dest="b", type=int, default=-1)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--ops", action="store_true", help="also diff the op census")
    args = ap.parse_args()

    dumps = parse_raw_dumps(args.log)
    if len(dumps) < 2:
        sys.exit(f"need >=2 BIFRAW dumps, found {len(dumps)} "
                 f"(build with --features bif-census and set RUSTCFML_COUNTERS=1)")
    a, b = dumps[args.a], dumps[args.b]
    print(f"# {len(dumps)} dumps; diffing dump {args.a} -> {args.b}", file=sys.stderr)

    rows = []
    for name, stat in b.items():
        d = sub(stat, a.get(name))
        if d["calls"] > 0:
            rows.append((d["calls"], name, d))
    rows.sort(reverse=True, key=lambda t: t[0])
    total = sum(r[0] for r in rows)
    print(f"=== ONE-REQUEST BIF MIX: {total} calls, {len(rows)} distinct builtins "
          f"(top {min(args.top, len(rows))}) ===")
    cum = 0
    for calls, name, d in rows[:args.top]:
        cum += calls
        arity = " ".join(f"{i}:{c}" for i, c in enumerate(d["arity"]) if c > 0)
        print(f"{calls:>7} {calls / total * 100:>6.2f}% {cum / total * 100:>6.2f}%cum  "
              f"{name:<22} arity[{arity}]")
        for i, vals in enumerate(d["args"]):
            f = fmt_arg(vals)
            if f:
                print(f"{'':>24}arg{i + 1}: {f}")
    tail = total - cum
    if tail:
        print(f"{tail:>7} {tail / total * 100:>6.2f}%          "
              f"(remaining {len(rows) - args.top} builtins)")

    if args.ops:
        od = parse_op_dumps(args.log)
        if len(od) >= 2:
            oa, ob = od[args.a], od[args.b]
            orows = sorted(((ob[k] - oa.get(k, 0), k) for k in ob),
                           reverse=True)
            orows = [r for r in orows if r[0] > 0]
            otot = sum(r[0] for r in orows)
            print(f"\n=== ONE-REQUEST OP MIX: {otot} ops executed, "
                  f"{len(orows)} distinct opcodes (top {args.top}) ===")
            ocum = 0
            for n, k in orows[:args.top]:
                ocum += n
                print(f"{n:>8} {n / otot * 100:>6.2f}% {ocum / otot * 100:>6.2f}%cum  {k}")
            otail = otot - ocum
            if otail:
                print(f"{otail:>8} {otail / otot * 100:>6.2f}%          "
                      f"(remaining {len(orows) - args.top} opcodes)")
        else:
            print(f"\n(no op census in log: found {len(od)} dumps)", file=sys.stderr)


if __name__ == "__main__":
    main()
