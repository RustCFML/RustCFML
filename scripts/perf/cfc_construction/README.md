# CFC construction and `StructAppend` cost, vs Lucee

The harness behind the GH #402 verdict and the GH #425 finding. It measures
what a CFC costs to construct as the declared method count grows, and what the
Wheels `Plugins.$initializeMixins` body costs on top of that.

## Running it

Serve the repo root and hit the pages; the CFCs resolve as
`scripts.perf.cfc_construction.*`, so nothing needs installing.

```bash
cargo build --release
./target/release/rustcfml --serve . --port 8690 &
curl -s "http://127.0.0.1:8690/scripts/perf/cfc_construction/bench6.cfm?iter=5000&warm=5000"
```

For the Lucee arm, `box server start` (pinned to lucee@7) and hit the same path
on its port.

| page | measures |
|---|---|
| `bench3.cfm` | the four levels of the `$initializeMixins` body at three CFC widths |
| `bench4.cfm` | `structAppend` per entry, with FUNCTION vs STRING vs STRUCT values |
| `bench5.cfm` | appending into a plain struct vs a large struct vs a component's `this` |
| `bench6.cfm` | construction only, vs declared method count — the #425 table |
| `bench7.cfm` | the subtractive LADDER: bare UDF call -> empty CFC -> +init -> 20 methods |
| `bench8.cfm` | an empty-CFC loop with nothing else in it, for `--profile` / DHAT |
| `bench9.cfm` | empty-CFC construction, cross-engine (page-scope safe for Lucee) |
| `bench10.cfm` | ONE arm, width and warm-up both parameters — for convergence checks |

## Warm-up is not optional — and the NARROWER the CFC, the longer it takes

**Lucee needs ~200,000 warm-up iterations to converge on an EMPTY CFC.** Measured
with `bench9.cfm` on Lucee 7.1.0.204, the same construction reads:

| warm-up iterations | 2k | 5k | 20k | 50k | 200k | 500k |
|---|---|---|---|---|---|---|
| Lucee, empty CFC | 3.95 us | 1.06 us | 0.59 us | 0.58 us | 0.41 us | 0.40 us |

A tenfold error at the warm-up this file used to recommend. The reason is that
warm-up is counted in ITERATIONS but Lucee's JIT responds to WORK: the less each
iteration does, the more iterations it takes to trigger compilation. The wide
arms are fine at 5,000 (the 20-method arm converges by ~50k to 1.80-1.98 us
against the 1.73 us originally published), so the #425 table stands as written —
but anything narrow needs far more. Use `bench10.cfm` to walk the ladder and SEE
the convergence rather than assuming a number is warm.

RustCFML has no JIT and is flat across the whole ladder (2.75-2.83 us), so every
under-warmed comparison flatters us.

## Warm-up is not optional

At 150 iterations Lucee reads **26.4 µs** for a construction that is really
**5.7 µs**. Under-warming always flatters us, and it flattered us by 3× here
before the numbers were re-taken at 5,000. Use `warm=5000&iter=5000` for
anything you intend to quote, and take the median of three runs — a single
RustCFML run in one sitting came back 30% high on two of six rows.

## What it found (v0.683.0 vs Lucee 7.1.0.204)

Construction is FIXED-cost here and PER-METHOD on Lucee:

| declared methods | RustCFML | Lucee |
|---|---|---|
| 20 | 3.67 µs | 1.73 µs |
| 86 | 3.94 µs | 5.68 µs |
| 300 | 4.44 µs | 20.6 µs |

RustCFML ≈ 3.5 µs fixed + ~3 ns/method; Lucee ≈ 0.4 µs fixed + ~68 ns/method.
Crossover ~48 methods. These are flat CFCs, so an inheritance shape must still be
measured before drawing conclusions about real framework classes.

## The gap is a constant, not a ratio (v0.685.0, both engines at CONVERGED warm-up)

Measured with `bench9`/`bench10` over HTTP, interleaved, medians:

| declared methods | Lucee | RustCFML | ratio | **absolute gap** |
|---|---|---|---|---|
| 0 (empty `component { }`) | 0.407 µs | 2.682 µs | 6.6x | **2.28 µs** |
| 20 | 1.830 µs | 3.892 µs | 2.13x | **2.06 µs** |

The RATIO collapses from 6.6x to 2.1x while the ABSOLUTE gap barely moves: it is
the same ~2.1-2.3 µs of fixed overhead either way, and the ratio only improves
because Lucee's own per-method cost (20 x 68 ns) dilutes it. Quoting "2.1x at 20
methods" understates the defect — at zero methods, where nothing masks it, we are
nearly 7x.

Decomposed at v0.684.1 (DHAT, marginal over 30k constructions): an empty CFC cost
**61 allocations / 3,816 bytes**, against ~0 for an empty UDF call. The native
profile put ~45% of request-thread CPU in mimalloc's fresh-page path. v0.685.0
took that to 47 allocations; the rest has no comparably cheap fix — see GH #425.

`structAppend` shows the same fixed-versus-marginal split: 95 ns/entry at 20
entries vs 42 at 300 (Lucee 48 and 35). Value kind is irrelevant — function
entries cost the same as strings — and appending into a component's `this` is
at parity with Lucee once construction is subtracted.
