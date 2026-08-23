#!/usr/bin/env python3
import sys, gzip, re
from collections import defaultdict

def read_varint(b, i):
    shift = 0; result = 0
    while True:
        byte = b[i]; i += 1
        result |= (byte & 0x7f) << shift
        if not (byte & 0x80): break
        shift += 7
    return result, i

def fields(b):
    """Yield (field_num, wire_type, payload) for a message's bytes."""
    i = 0; n = len(b)
    while i < n:
        tag, i = read_varint(b, i)
        fn = tag >> 3; wt = tag & 7
        if wt == 0:
            val, i = read_varint(b, i); yield fn, wt, val
        elif wt == 1:
            yield fn, wt, b[i:i+8]; i += 8
        elif wt == 2:
            ln, i = read_varint(b, i); yield fn, wt, b[i:i+ln]; i += ln
        elif wt == 5:
            yield fn, wt, b[i:i+4]; i += 4
        else:
            raise ValueError(f"bad wire type {wt}")

def read_packed_varints(b):
    out = []; i = 0; n = len(b)
    while i < n:
        v, i = read_varint(b, i); out.append(v)
    return out

def main(path):
    raw = open(path, 'rb').read()
    try:
        raw = gzip.decompress(raw)
    except Exception:
        pass

    string_table = []
    functions = {}   # func_id -> name_idx
    locations = {}   # loc_id -> [func_id,...] (leaf line first)
    samples = []     # (list_of_loc_ids_leaf_first, value)
    sample_types = []

    for fn, wt, payload in fields(raw):
        if fn == 1 and wt == 2:  # sample_type ValueType
            t = u = 0
            for f2, w2, p2 in fields(payload):
                if f2 == 1: t = p2
                elif f2 == 2: u = p2
            sample_types.append((t, u))
        elif fn == 2 and wt == 2:  # Sample
            locs = []; vals = []
            for f2, w2, p2 in fields(payload):
                if f2 == 1:
                    if w2 == 2: locs = read_packed_varints(p2)
                    else: locs.append(p2)
                elif f2 == 2:
                    if w2 == 2: vals = read_packed_varints(p2)
                    else: vals.append(p2)
            samples.append((locs, vals))
        elif fn == 4 and wt == 2:  # Location
            lid = 0; funcs = []
            for f2, w2, p2 in fields(payload):
                if f2 == 1: lid = p2
                elif f2 == 4 and w2 == 2:  # Line
                    for f3, w3, p3 in fields(p2):
                        if f3 == 1: funcs.append(p3)  # function_id
            locations[lid] = funcs
        elif fn == 5 and wt == 2:  # Function
            fid = 0; nm = 0
            for f2, w2, p2 in fields(payload):
                if f2 == 1: fid = p2
                elif f2 == 2: nm = p2
            functions[fid] = nm
        elif fn == 6 and wt == 2:  # string_table
            string_table.append(payload.decode('utf-8', 'replace'))

    def fname(fid):
        nm = functions.get(fid, 0)
        return string_table[nm] if nm < len(string_table) else f"<fid {fid}>"

    def clean(name):
        name = re.sub(r'::h[0-9a-f]{8,}$', '', name)
        return name

    flat = defaultdict(int)  # self samples per function
    cum = defaultdict(int)   # cumulative
    total = 0

    for locs, vals in samples:
        v = vals[0] if vals else 1
        total += v
        # leaf function = first line of first location
        if locs:
            leaf_loc = locs[0]
            lf = locations.get(leaf_loc, [])
            if lf:
                flat[clean(fname(lf[0]))] += v
        # cumulative: dedup functions across the stack
        seen = set()
        for loc in locs:
            for fid in locations.get(loc, []):
                nm = clean(fname(fid))
                if nm not in seen:
                    seen.add(nm); cum[nm] += v

    print(f"# sample_types: {[(string_table[t], string_table[u]) for t,u in sample_types]}")
    print(f"# total samples (value[0] sum): {total}   n_samples={len(samples)}  n_functions={len(functions)}  n_locations={len(locations)}")
    print()
    print("="*100)
    print("TOP 45 BY SELF (flat) — where CPU actually burns")
    print("="*100)
    for name, c in sorted(flat.items(), key=lambda x: -x[1])[:45]:
        print(f"{c:>9} {100.0*c/total:6.2f}%  {name}")
    print()
    print("="*100)
    print("TOP 55 BY CUMULATIVE (inclusive) — dominant call buckets")
    print("="*100)
    for name, c in sorted(cum.items(), key=lambda x: -x[1])[:55]:
        print(f"{c:>9} {100.0*c/total:6.2f}%  {name}")

if __name__ == '__main__':
    main(sys.argv[1])
