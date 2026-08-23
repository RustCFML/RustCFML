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

def read_packed_varints(b):
    out = []; i = 0; n = len(b)
    while i < n:
        v, i = read_varint(b, i); out.append(v)
    return out

raw = open(sys.argv[1], 'rb').read()
try: raw = gzip.decompress(raw)
except Exception: pass

string_table=[]; functions={}; locations={}; samples=[]
for fn, wt, payload in fields(raw):
    if fn == 2 and wt == 2:
        locs=[]; vals=[]
        for f2,w2,p2 in fields(payload):
            if f2==1: locs = read_packed_varints(p2) if w2==2 else locs+[p2]
            elif f2==2: vals = read_packed_varints(p2) if w2==2 else vals+[p2]
        samples.append((locs,vals))
    elif fn == 4 and wt == 2:
        lid=0; funcs=[]
        for f2,w2,p2 in fields(payload):
            if f2==1: lid=p2
            elif f2==4 and w2==2:
                for f3,w3,p3 in fields(p2):
                    if f3==1: funcs.append(p3)
        locations[lid]=funcs
    elif fn == 5 and wt == 2:
        fid=0; nm=0
        for f2,w2,p2 in fields(payload):
            if f2==1: fid=p2
            elif f2==2: nm=p2
        functions[fid]=nm
    elif fn == 6 and wt == 2:
        string_table.append(payload.decode('utf-8','replace'))

def fname(fid):
    nm=functions.get(fid,0)
    return string_table[nm] if nm<len(string_table) else f"<{fid}>"
def clean(n): return re.sub(r'::h[0-9a-f]{8,}$','',n)

# Build per-sample flat stack of function names (leaf first)
stacks=[]  # (names_leaf_first, value)
total=0
for locs,vals in samples:
    v=vals[0] if vals else 1; total+=v
    names=[]
    for loc in locs:
        for fid in locations.get(loc,[]):
            names.append(clean(fname(fid)))
    stacks.append((names,v))

def contains(names, needle): return any(needle in n for n in names)

def caller_breakdown(leaf_needle, label):
    callers=defaultdict(int); tot=0
    for names,v in stacks:
        if names and leaf_needle in names[0]:
            tot+=v
            caller = names[1] if len(names)>1 else "<root>"
            callers[caller]+=v
    print(f"\n### callers of leaf `{label}`  (self total {tot} = {100.0*tot/total:.1f}%)")
    for c,n in sorted(callers.items(), key=lambda x:-x[1])[:8]:
        print(f"   {n:>6} {100.0*n/total:5.1f}%  <- {c}")

caller_breakdown("vfs::RealFs as cfml_common::vfs::Vfs>::exists", "RealFs::exists")
caller_breakdown("CfmlStruct::new", "CfmlStruct::new")
caller_breakdown("Vfs>::canonicalize", "canonicalize")

# Compilation vs pure-resolution split of component resolution cost
comp_res_total=0; comp_res_compiling=0; comp_res_warm=0
for names,v in stacks:
    if contains(names,"resolve_component_template"):
        comp_res_total+=v
        if contains(names,"compile_file_cached") or contains(names,"read_to_string::inner"):
            comp_res_compiling+=v
        else:
            comp_res_warm+=v
print(f"\n### resolve_component_template samples: {comp_res_total} ({100.0*comp_res_total/total:.1f}%)")
print(f"     of which compiling (read/compile in stack): {comp_res_compiling} ({100.0*comp_res_compiling/total:.1f}%)")
print(f"     of which pure resolution (no compile):      {comp_res_warm} ({100.0*comp_res_warm/total:.1f}%)")

# How much total CPU touches the filesystem exists/canonicalize?
fs=0
for names,v in stacks:
    if names and ("Vfs>::exists" in names[0] or "Vfs>::canonicalize" in names[0]):
        fs+=v
print(f"\n### total self CPU in FS exists()+canonicalize(): {fs} ({100.0*fs/total:.1f}%)")

# Samples that go through a per-REQUEST lifecycle vs application boot/reload
# (heuristic: onApplicationStart / reload / applicationStart appear in CFML frames, but those aren't in Rust syms;
#  instead split by presence of compile (cold) anywhere)
cold=0
for names,v in stacks:
    if contains(names,"compile_file_cached") or contains(names,"read_to_string::inner"):
        cold+=v
print(f"### samples doing any file read/compile (cold/first-hit indicator): {cold} ({100.0*cold/total:.1f}%)")
