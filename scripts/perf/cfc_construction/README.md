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
Crossover ~48 methods. See GH #425 — the ~3.5 µs has NOT been decomposed yet,
and these are flat CFCs, so an inheritance shape must be measured before
drawing conclusions about real framework classes.

`structAppend` shows the same fixed-versus-marginal split: 95 ns/entry at 20
entries vs 42 at 300 (Lucee 48 and 35). Value kind is irrelevant — function
entries cost the same as strings — and appending into a component's `this` is
at parity with Lucee once construction is subtracted.
