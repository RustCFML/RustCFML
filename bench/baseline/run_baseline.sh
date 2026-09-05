#!/usr/bin/env bash
# KERNEL BASELINE — run all four workloads, 3 trials each, report min
# wall-clock. Use as a reference point for engine perf claims.
#
# Was a JIT A/B (interpreter vs default) until v0.653.0 removed the JIT; the
# kernels themselves remain useful as breadth benchmarks, so the harness now
# just times them.
#
# Usage: from repo root,
#   cargo build --release
#   bench/baseline/run_baseline.sh
set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/release/rustcfml"
[[ -x "$BIN" ]] || { echo "release binary missing — cargo build --release first"; exit 1; }

bench_one() {
    local label="$1" file="$2"
    local best=99999 t
    for i in 1 2 3; do
        t=$(/usr/bin/time -p "$BIN" "$file" 2>&1 >/dev/null | awk '/^real/{print $2}')
        awk -v a="$t" -v b="$best" 'BEGIN{ exit !(a<b) }' && best="$t"
    done
    printf "%-26s  %6ss (min of 3)\n" "$label" "$best"
}

echo "KERNEL BASELINE  $(date '+%Y-%m-%d %H:%M:%S')  $(cd "$REPO_ROOT" && git describe --tags --dirty 2>/dev/null || echo 'unknown')"
echo "==================================================================="
bench_one "numeric_kernel"        "$REPO_ROOT/bench/baseline/numeric_kernel.cfm"
bench_one "udf_call_graph"        "$REPO_ROOT/bench/baseline/udf_call_graph.cfm"
bench_one "string_kernel"         "$REPO_ROOT/bench/baseline/string_kernel.cfm"
bench_one "struct_member_kernel"  "$REPO_ROOT/bench/baseline/struct_member_kernel.cfm"
echo "==================================================================="
echo "tests/runner.cfm full-suite as a representative breadth workload:"
bench_one "tests/runner.cfm"      "$REPO_ROOT/tests/runner.cfm"
