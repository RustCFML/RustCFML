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

`pgo/rustcfml.profdata` — 5.3 MB (1.04 MB gzipped), `llvm-profdata merge --sparse`.
Trained on two workloads:

1. `tests/runner.cfm` via the CLI — broad language coverage
2. a live Preside site: boot + 150 warm renders + `/admin/`

Training-set choice is measured, not assumed: suite-only training gives −11.88%
(84% of the win with no site, no database, no fixtures), and adding the live-site
renders is worth the remaining ~2.3 points.

⚠️ A third variant trained additionally on **Preside's own TestBox suite** was
built and rejected *for now*: that suite contributes ~59% of the total profile
weight, more than the other two workloads combined, and PGO resolves conflicting
hot paths by compromise — so it risks optimising test-suite code at the expense of
the render path. It is unmeasured on request latency. See
`/Users/alexskinner/rustcfml-pgo/README.md` (local, untracked) for the artifacts.

## Regenerating the profile

Needed when: the pinned toolchain moves, the dispatch loop / codegen / hot stdlib
changes materially, or the coverage gate in `release.yml` starts complaining.

```bash
DIR=/tmp/pgo-profraw
rm -rf $DIR && mkdir -p $DIR

# 1. instrument
RUSTFLAGS="-Cprofile-generate=$DIR" cargo build --release

# 2. train — BOTH workloads
target/release/rustcfml tests/runner.cfm
cd /path/to/preside/site && .../target/release/rustcfml --serve . --port 8641 --production &
#    ...boot, ~150 warm renders, one /admin/ hit, then:
kill -INT <pid>            # SIGINT ONLY — see the traps below

# 3. merge (rustup's llvm-profdata, NOT Xcode's)
PROFDATA=$(find "$(rustc --print sysroot)" -name 'llvm-profdata*' | head -1)
$PROFDATA merge -o merged.profdata $DIR
$PROFDATA merge --sparse -o pgo/rustcfml.profdata merged.profdata
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
   must report missing.

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
