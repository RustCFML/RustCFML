#!/usr/bin/env python3
"""Reconcile sum(op_count x op_cost) against the measured warm render
(PERFORMANCE_ROADMAP Part 1 Step 2). The roadmap's rule: it must land within
~20% of the ~21 ms render, and "if it doesn't reconcile, the model is wrong and
the reconciliation failure is the finding."

Counts: Step 0 op census, one warm Preside homepage render (213,966 ops).
Costs:  Step 2 op bench, RustCFML --no-jit (Preside admits ~0% to the JIT, so
        the interpreted number is the honest one), net of an empty loop.
"""
# op -> (count per warm render, ns each, how the cost was derived)
CHEAP = 5.5   # empty-loop ops: 8 ops in 43.7 ns (op count verified by census)

OPS = [
    # (name, count, ns, source)
    ("LoadLocal",         23579, 17.2, "local_read net"),
    ("LineInfo",          23304, CHEAP, "assumed cheap-op floor"),
    ("LoadSlot",          15225, CHEAP, "slot read, in the empty-loop mix"),
    ("JumpIfFalse",       13366, CHEAP, "in the empty-loop mix"),
    ("LoadGlobal",        13170, 58.7, "global_read net"),
    ("GetProperty",       12441, 31.6, "struct_read net"),
    ("Return",             8193, 0.0,  "priced inside the frame cost below"),
    ("StoreSlot",          7383, CHEAP, "in the empty-loop mix"),
    ("String",             7106, 23.3, "string_literal net"),
    ("StoreLocal",         6710, 13.3, "local_write net"),
    ("DeclareSlot",        6368, CHEAP, "cheap-op floor"),
    ("JumpIfNotNull",      6063, CHEAP, "cheap-op floor"),
    ("GetIndex",           5511, 59.0, "array_read net"),
    ("JumpIfArgPresent",   4286, CHEAP, "cheap-op floor"),
    ("Pop",                3779, CHEAP, "cheap-op floor"),
    ("TryGetProperty",     3100, 31.6, "as GetProperty"),
    ("SetProperty",        2935, 59.9, "struct_write net"),
    ("Swap",               2874, CHEAP, "cheap-op floor"),
    ("False",              2712, CHEAP, "cheap-op floor"),
    ("Jump",               2671, CHEAP, "cheap-op floor"),
    ("Dup",                2295, CHEAP, "cheap-op floor"),
    ("Lte",                2225, CHEAP, "cheap-op floor"),
    ("ValidateParamType",  2063, CHEAP, "cheap-op floor"),
    ("True",               1888, CHEAP, "cheap-op floor"),
    ("Not",                1804, CHEAP, "cheap-op floor"),
    ("LoadLocalProperty",  1723, 31.6, "as GetProperty"),
    ("IncrementSlot",      1599, CHEAP, "in the empty-loop mix"),
    ("JumpIfTrue",         1230, CHEAP, "cheap-op floor"),
    ("TryLoadSlot",        1200, CHEAP, "cheap-op floor"),
    ("Concat",             1088, 88.6, "string_concat net"),
    ("DeclareLocal",       1060, CHEAP, "cheap-op floor"),
    ("Integer",             959, CHEAP, "cheap-op floor"),
    ("BuildStruct",         785, 40.0, "estimate: structNew-ish"),
    ("SetIndex",            746, 56.4, "array_write net"),
    ("DefineFunction",      563, CHEAP, "cheap-op floor"),
    ("Eq",                  482, CHEAP, "cheap-op floor"),
]
TOTAL_OPS = 213966
CALL_OPS = 12684 + 3669 + 763 + 486        # Call, CallMethod, CallMethodNamed, CallNamed
REST = TOTAL_OPS - sum(c for _, c, _, _ in OPS) - CALL_OPS

# Constructs measured end-to-end, priced OUTSIDE the per-op table.
BIF_CALLS, BIF_MS = 8343, 1.4965           # directly measured by the BIF bench
FRAMES = 8193                              # = Return count
METHOD_CALLS = 3669 + 763
UDF_CALLS = FRAMES - METHOD_CALLS
NS_METHOD, NS_UDF = 268.1, 870.3           # --no-jit, empty callee, same empty-loop baseline

RENDER_MS_PGO = 20.7                       # roadmap baseline, PGO, warm homepage
RENDER_CPU_MS = 18.9                       # 91% CPU-bound in-request

print("=== RECONCILIATION: sum(op_count x op_cost) vs the measured render ===\n")
print(f"{'op':<20}{'count':>8}{'ns':>8}{'ms':>9}  source")
sub = 0.0
for n, c, ns, src in sorted(OPS, key=lambda t: -t[1] * t[2]):
    ms = c * ns / 1e6
    sub += ms
    print(f"{n:<20}{c:>8}{ns:>8.1f}{ms:>9.4f}  {src}")
rest_ms = REST * CHEAP / 1e6
sub += rest_ms
print(f"{'(other ops)':<20}{REST:>8}{CHEAP:>8.1f}{rest_ms:>9.4f}  cheap-op floor")
print(f"{'ORDINARY OPS':<20}{'':>8}{'':>8}{sub:>9.4f}\n")

print(f"{'construct':<20}{'count':>8}{'ns':>8}{'ms':>9}  source")
frames_ms = (METHOD_CALLS * NS_METHOD + UDF_CALLS * NS_UDF) / 1e6
print(f"{'BIF calls':<20}{BIF_CALLS:>8}{BIF_MS*1e6/BIF_CALLS:>8.1f}{BIF_MS:>9.4f}  BIF bench, measured directly")
print(f"{'CFC method frames':<20}{METHOD_CALLS:>8}{NS_METHOD:>8.1f}{METHOD_CALLS*NS_METHOD/1e6:>9.4f}  sibling() call, --no-jit")
print(f"{'other UDF frames':<20}{UDF_CALLS:>8}{NS_UDF:>8.1f}{UDF_CALLS*NS_UDF/1e6:>9.4f}  page UDF call, --no-jit")
total = sub + BIF_MS + frames_ms
print(f"\n{'MODELLED TOTAL':<20}{'':>24}{total:>9.4f} ms")
print(f"{'measured render':<20}{'':>24}{RENDER_MS_PGO:>9.4f} ms wall (PGO baseline)")
print(f"{'measured CPU':<20}{'':>24}{RENDER_CPU_MS:>9.4f} ms")
print(f"\nmodel explains {total/RENDER_CPU_MS*100:.0f}% of render CPU; "
      f"UNACCOUNTED = {RENDER_CPU_MS-total:.2f} ms ({(RENDER_CPU_MS-total)/RENDER_CPU_MS*100:.0f}%)")
print(f"roadmap tolerance is +/-20% -> reconciliation "
      f"{'PASSES' if abs(total-RENDER_CPU_MS)/RENDER_CPU_MS <= 0.2 else 'FAILS'}")
print(f"\nimplied real cost per op if the render were all ops: "
      f"{RENDER_CPU_MS*1e6/TOTAL_OPS:.1f} ns/op")
print(f"microbenchmarked average over the same mix:           "
      f"{sub*1e6/(TOTAL_OPS-CALL_OPS):.1f} ns/op")
print(f"=> real-context ops cost {RENDER_CPU_MS*1e6/TOTAL_OPS / (sub*1e6/(TOTAL_OPS-CALL_OPS)):.1f}x "
      f"their tight-loop cost")
