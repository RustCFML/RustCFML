# Profile-guided optimisation (PGO)

Shipped binaries are built with PGO. `cargo build --release` is **not** — it stays
thin-LTO and fast for everyday use. The shipped configuration is
`--profile release-pgo` (fat LTO, see `Cargo.toml`) plus
`-Cprofile-use=pgo/rustcfml.profdata`.

## What it buys

Measured on a live Preside site, `--serve --production`, interleaved CPU-time A/B,
5 of 5 rounds in the same direction:

| build | warm CPU/req | vs shipped | boot |
|---|---|---|---|
| thin LTO, no PGO | 22.25 ms | — | 8.50 s |
| fat LTO, no PGO | 22.10 ms | −1.23% | — |
| **fat LTO + PGO** | **19.10 ms** | **−14.16%** | **7.21 s (−15.25%)** |

Spread was 0.15 ms on both arms. **PGO does essentially all of it**; fat LTO alone
is ~1%, and is kept only because the profile was trained under it.

## The committed profile

`pgo/rustcfml.profdata` — ~16 MB, `llvm-profdata merge --sparse`, ~59k functions.
Trained on two workloads:

1. `tests/runner.cfm` via the CLI — broad language coverage
2. a live Preside site: boot + 150 warm renders

Training-set choice is measured, not assumed: suite-only training gives −11.88%
(84% of the win with no site, no database, no fixtures), and adding the live-site
renders is worth the remaining ~2.3 points.

⚠️ A third variant trained additionally on **Preside's own TestBox suite** was
built and rejected *for now*: that suite contributes ~59% of the total profile
weight, more than the other two workloads combined, and PGO resolves conflicting
hot paths by compromise — so it risks optimising test-suite code at the expense of
the render path. It is unmeasured on request latency.

⚠️ **Authenticated admin pages were added and rejected on measurement**
(2026-08-17). The recipe used to end with "one `/admin/` hit" — which returns
**401** when not logged in, so every profile ever trained learned an
access-denied page rather than the admin. Fixing that properly (real form login,
150 renders across 10 admin pages) added only **+28 functions**: the admin runs
the *same* functions as the front end, just ~4x as often per render, so there was
never a coverage hole, only a re-weighting. It measured **parity on admin pages
themselves** (−0.54% mean, −0.49% adjacent-pair median, against a 4.84% A-to-A
spread) and parity on the front end. Available behind `--admin-user/--admin-pass`
for anyone who wants to re-test; off by default. **Check the function-count delta
before spending an A/B on a new training workload** — a workload that adds no
functions can only re-weight, and re-weighting has never cleared noise here.

## Regenerating the profile

Needed when: the pinned toolchain moves, the dispatch loop / codegen / hot stdlib
changes materially, or the coverage gate in `release.yml` starts complaining.

```bash
scripts/pgo-train.sh --site /path/to/site            # writes pgo/rustcfml.profdata
scripts/pgo-train.sh --site ... --out /tmp/cand.profdata   # candidate, don't overwrite
```

The script is mostly assertions, because **every PGO failure mode here is
silent** — they all yield a profile that loads, passes the release gate, and is
simply worse. It fails loudly if the suite produces no SUMMARY, the site never
returns 200, the server ignores SIGINT, the serve phase adds no profile counts,
or the merged profile carries fewer than 5000 functions (the same threshold
`release.yml` enforces, so a bad profile fails locally rather than in CI).

Then build and **A/B it** — a new profile is not automatically better; every
retrain so far has measured within noise of its predecessor:

```bash
RUSTFLAGS="-Cprofile-use=$PWD/pgo/rustcfml.profdata" \
  cargo build --profile release-pgo -p rustcfml-cli
```

### ⚠️ Traps, each of which cost a full build cycle

1. **The training server MUST exit via SIGINT.** `.profraw` is flushed from an
   atexit hook and the server only handles `tokio::signal::ctrl_c()`. A plain
   `pkill` (SIGTERM) kills it without flushing, silently training PGO on the CLI
   path alone.
2. **Do not judge a workload's contribution by profraw file size.**
   `-Cprofile-generate` writes `default_%m.profraw` and *online-merges* later runs
   of the same binary into it, so a workload that touches the same functions adds
   counts without growing the file. One training pass looked like "+0 KB" and was
   in fact 15.16 bn counts. To attribute a workload, capture it in isolation with
   `LLVM_PROFILE_FILE=<dir>/%m.profraw`, then `llvm-profdata show`.
3. **Use rustup's `llvm-profdata`.** Xcode's is a different LLVM version and cannot
   merge rustc's `.profraw`.
4. **Do not use missing-function warning counts as a staleness metric** on a
   `--sparse` profile: sparse deliberately drops never-executed functions, so they
   must report missing. Confirmed empirically: a profile trained on the *exact*
   source being built reports the **same 16,163 warnings, with identical
   per-function counts**, as one trained four releases earlier. The number says
   nothing about staleness.
5. **`-Cprofile-use` never mis-optimises, it only decays.** LLVM matches a
   profile to a function by name + CFG hash, so a changed function simply gets no
   data and compiles un-profiled. That is why the hazard is silence, not
   breakage — and why the training script asserts rather than trusts.

### Known limitation — value-profiling counters are exhausted

Instrumented builds emit `LLVM Profile Warning: Unable to track new values:
Running out of static counters` many times. Value profiling (indirect-call target
tracking) is therefore **truncated in every profile trained so far**. The profile
is still valid and this is not fatal, but it is a plausible reason the measured
win is smaller than it could be. Untested fix: raise the budget with
`-Cllvm-args=-vp-counters-per-site=<n>` on the *instrumented* build. Worth its own
A/B; do not fold it into an unrelated retrain.

## ⚠️ Do not put `shell: bash` on the Windows build step

v0.595.0's release failed on all four targets because of this. `shell: bash` makes
the Windows runner use Git-Bash, which puts **MSYS Perl**
(`/usr/share/perl5/core_perl`) ahead of Strawberry Perl on `PATH` — and MSYS Perl
cannot configure vendored OpenSSL for `VC-WIN64A`, so `openssl-sys`'s build script
dies with *"'perl' reported failure with exit code: 2"*. Nothing to do with PGO.

Pass `RUSTFLAGS` through `env:` instead of a shell line-continuation, which also
avoids quoting a Windows path full of backslashes. (The profile-verification step
*may* keep `shell: bash` — it only runs `find`/`grep` and does not invoke Perl.)

The matrix also sets `fail-fast: false`: the Windows-only fault cancelled the other
three targets mid-build, so a single broken platform looked like a total failure and
the healthy targets produced no evidence at all.

## Staleness is safe, but silent

LLVM matches a profile to a function by name + CFG hash, so a changed function
simply gets **no data** and compiles un-profiled. A stale profile decays toward the
un-profiled baseline; it never mis-optimises. The hazard is that nothing tells you,
which is why `release.yml` verifies the profile loads and carries a plausible
function count, and why the toolchain is **pinned** rather than `stable`.

## Not yet measured

- **Cross-architecture.** The profile is trained on macOS/arm64. `.profdata` keys on
  IR function name + CFG hash, so the portable core (VM dispatch loop, `CfmlValue`
  ops, stdlib) transfers, but `cfg`-gated platform code gets nothing. Expected
  degraded-not-harmful on Linux/Windows — an expectation, not a measurement.
- **Whether PGO helps test-suite wall-clock** as much as it helps request latency.
  The numbers above are warm request CPU.
