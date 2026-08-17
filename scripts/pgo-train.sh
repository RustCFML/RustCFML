#!/usr/bin/env bash
# pgo-train.sh — regenerate pgo/rustcfml.profdata.
#
# Shipped binaries are built with `--profile release-pgo -Cprofile-use=...`;
# `cargo build --release` deliberately stays thin-LTO and fast. See docs/pgo.md
# for what PGO buys and when the profile needs regenerating.
#
# This script exists because the recipe used to live only as prose. Every
# retrain was re-derived by hand, and the failure modes below are all SILENT —
# they produce a profile that loads, passes the release gate, and is simply
# worse. Most of this file is assertions.
#
#   Usage:
#     scripts/pgo-train.sh --site /path/to/preside/site
#     scripts/pgo-train.sh --site ... --port 8641 --renders 150
#     scripts/pgo-train.sh --site ... --out /tmp/candidate.profdata   # don't overwrite the committed one
#     scripts/pgo-train.sh --site ... --admin-user sysadmin --admin-pass password
#
# Training set (docs/pgo.md): the CFML suite via the CLI, plus a live site boot
# and warm renders. Suite-only training already captures ~84% of the win; the
# live renders are worth the remaining points.
#
# NOT trained on by default, both measured and rejected:
#   * Preside's own TestBox suite — contributes ~59% of profile weight, and PGO
#     resolves conflicting hot paths by compromise, so it risks optimising test
#     code over the render path.
#   * Authenticated admin pages (--admin-user) — measured 2026-08-17: adds only
#     +28 functions (the admin runs the SAME functions as the front end, just
#     ~4x as often) and measured PARITY on admin pages themselves (-0.54% mean
#     against a 4.84% noise floor). Kept as an opt-in flag, with a real login
#     and a status assertion, because the old prose recipe hit /admin/ WITHOUT
#     logging in and silently trained on a 401 error page for months.
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root
REPO=$(pwd)

SITE=""; PORT=8641; RENDERS=150; OUT="$REPO/pgo/rustcfml.profdata"
ADMIN_USER=""; ADMIN_PASS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --site)       SITE=${2:?};       shift 2 ;;
    --port)       PORT=${2:?};       shift 2 ;;
    --renders)    RENDERS=${2:?};    shift 2 ;;
    --out)        OUT=${2:?};        shift 2 ;;
    --admin-user) ADMIN_USER=${2:?}; shift 2 ;;
    --admin-pass) ADMIN_PASS=${2:?}; shift 2 ;;
    -h|--help)    sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$SITE" ] || { echo "ERROR: --site is required (path to a live CFML site to train on)" >&2; exit 2; }
[ -d "$SITE" ] || { echo "ERROR: --site '$SITE' is not a directory" >&2; exit 2; }

WORK=${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/rustcfml-pgo.XXXXXX")}
mkdir -p "$WORK"
PROFRAW="$WORK/profraw"; rm -rf "$PROFRAW"; mkdir -p "$PROFRAW"
# rustup's llvm-profdata, NOT Xcode's — different LLVM, cannot read rustc profraw.
PROFDATA_TOOL=$(find "$(rustc --print sysroot)" -name 'llvm-profdata*' | head -1)
[ -n "$PROFDATA_TOOL" ] || { echo "ERROR: llvm-profdata not in sysroot — 'rustup component add llvm-tools'" >&2; exit 1; }
TGT="$WORK/target"
BIN="$TGT/release/rustcfml"
SRV_PID=""
# Browser-ish headers are REQUIRED: without an Accept header Preside answers 401
# with a full access-denied render instead of redirecting to the login form.
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36"
JAR="$WORK/cookies.txt"
CURL=(curl -sS -A "$UA" -H "Accept: text/html,application/xhtml+xml" -b "$JAR" -c "$JAR" --max-time 600)

cleanup() {
  if [ -n "$SRV_PID" ] && kill -0 "$SRV_PID" 2>/dev/null; then
    kill -9 "$SRV_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
# Counts, NOT bytes. `-Cprofile-generate` writes default_%m.profraw and
# ONLINE-MERGES later runs of the same binary into the existing file, so both
# the file count and the byte size stay flat by design once the first workload
# has touched a function — a serve phase worth billions of counts routinely
# shows "+0 KB". Total count is the only honest measure of contribution.
total_count() {
  "$PROFDATA_TOOL" merge -o "$WORK/probe.profdata" "$PROFRAW" 2>/dev/null || { echo 0; return; }
  "$PROFDATA_TOOL" show "$WORK/probe.profdata" 2>/dev/null \
    | grep -i 'Total count' | grep -oE '[0-9]+' | head -1 || echo 0
}

step "1/5  instrumented build  (work dir: $WORK)"
# Default features only. Never build the training binary with a diagnostic
# feature (call-phases, probe-sites, …): it trains on code that will not ship,
# and its edits sit inside the hottest functions, whose CFG hash then changes.
CARGO_TARGET_DIR="$TGT" RUSTFLAGS="-Cprofile-generate=$PROFRAW" \
  cargo build --release -p rustcfml-cli -j "${JOBS:-4}"
[ -x "$BIN" ] || { echo "ERROR: instrumented binary not produced" >&2; exit 1; }
"$BIN" --version

step "2/5  train (a): CFML suite via the CLI"
"$BIN" tests/runner.cfm > "$WORK/train_cli.log" 2>&1 || true
SUMMARY=$(grep -E "^SUMMARY" "$WORK/train_cli.log" | tail -1 || true)
[ -n "$SUMMARY" ] || { echo "ERROR: suite produced no SUMMARY — training would be worthless" >&2
                       tail -20 "$WORK/train_cli.log" >&2; exit 1; }
echo "    $SUMMARY"
if grep -qE "^(FAIL \||ERROR)" "$WORK/train_cli.log"; then
  echo "    WARNING: suite reported failures; profile still usable but investigate" >&2
fi
CLI_COUNT=$(total_count); echo "    profile count after CLI: $CLI_COUNT"

step "3/5  train (b): live site boot + $RENDERS warm renders"
"$BIN" --serve "$SITE" --port "$PORT" --production > "$WORK/train_serve.log" 2>&1 &
SRV_PID=$!
code=""
for _ in $(seq 1 600); do
  sleep 1
  code=$("${CURL[@]}" -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/?cb=boot" || true)
  [ "$code" = "200" ] && break
  kill -0 "$SRV_PID" 2>/dev/null || { echo "ERROR: training server died during boot" >&2
                                      tail -20 "$WORK/train_serve.log" >&2; exit 1; }
done
[ "$code" = "200" ] || { echo "ERROR: site never returned 200 (last=$code)" >&2; exit 1; }
echo "    booted"
for i in $(seq 1 "$RENDERS"); do
  "${CURL[@]}" -o /dev/null "http://127.0.0.1:$PORT/?cb=w$i" || true
done
echo "    $RENDERS warm renders done"

if [ -n "$ADMIN_USER" ]; then
  step "3b/5 train (c): authenticated admin pages  [opt-in; measured parity]"
  "${CURL[@]}" -L -o "$WORK/login.html" "http://127.0.0.1:$PORT/admin/" || true
  ACTION=$(grep -oE '<form[^>]*action="[^"]*login/login/[^"]*"' "$WORK/login.html" \
           | head -1 | sed -E 's/.*action="([^"]*)".*/\1/' || true)
  [ -n "$ACTION" ] || { echo "ERROR: no admin login form found (a 401 body means the Accept header was dropped)" >&2; exit 1; }
  "${CURL[@]}" -L --post301 --post302 -o "$WORK/admin.html" \
    --data-urlencode "loginId=$ADMIN_USER" --data-urlencode "password=$ADMIN_PASS" "$ACTION" || true
  if grep -q "You do not have access" "$WORK/admin.html"; then
    echo "ERROR: admin login rejected — refusing to train on an access-denied page" >&2; exit 1
  fi
  for i in $(seq 1 "$RENDERS"); do
    for pg in /admin/ /admin/sitetree/ /admin/datamanager/ /admin/dashboard/; do
      "${CURL[@]}" -o /dev/null "http://127.0.0.1:$PORT$pg?cb=a$i" || true
    done
  done
  echo "    admin renders done"
fi

step "4/5  flush the profile (SIGINT only)"
# .profraw is written from an atexit hook and the server handles ONLY
# tokio::signal::ctrl_c(). SIGTERM/SIGKILL discards every serve-side count and
# leaves a CLI-only profile that looks perfectly healthy.
kill -INT "$SRV_PID"
for _ in $(seq 1 180); do kill -0 "$SRV_PID" 2>/dev/null || break; sleep 1; done
if kill -0 "$SRV_PID" 2>/dev/null; then
  echo "ERROR: server ignored SIGINT; the serve-side profile would be lost" >&2; exit 1
fi
SRV_PID=""
SRV_COUNT=$(total_count)
echo "    profile count ${CLI_COUNT} -> ${SRV_COUNT}"
# The serve phase MUST have added counts. If it did not, the atexit flush was
# lost (usually a SIGTERM) and the profile is CLI-only — which loads fine,
# passes the release gate, and is simply worse. This is the failure this whole
# script exists to catch, so it is fatal.
[ "${SRV_COUNT:-0}" -gt "${CLI_COUNT:-0}" ] || {
  echo "ERROR: serve phase added no profile counts — the flush was lost (SIGTERM instead of SIGINT?)" >&2; exit 1; }

step "5/5  merge"
"$PROFDATA_TOOL" merge -o "$WORK/merged.profdata" "$PROFRAW"
mkdir -p "$(dirname "$OUT")"
"$PROFDATA_TOOL" merge --sparse -o "$OUT" "$WORK/merged.profdata"

FUNCS=$("$PROFDATA_TOOL" show "$OUT" | grep -i 'Total functions' | grep -oE '[0-9]+' | head -1)
COUNT=$("$PROFDATA_TOOL" show "$OUT" | grep -i 'Total count'     | grep -oE '[0-9]+' | head -1)
echo "    $OUT"
ls -la "$OUT"
echo "    functions=$FUNCS  total count=$COUNT  (rustc $(rustc --version | awk '{print $2}'))"
# Same threshold release.yml enforces, so a bad profile fails here, not in CI.
[ "${FUNCS:-0}" -ge 5000 ] || { echo "ERROR: only $FUNCS functions — expected >=5000. Training did not run properly." >&2; exit 1; }

cat <<EOF

Done. Build with it:
  RUSTFLAGS="-Cprofile-use=$OUT" cargo build --profile release-pgo -p rustcfml-cli

A new profile is NOT automatically better. A/B it against the committed one on a
real workload (interleaved legs, server CPU seconds, not wall clock) before
committing — every retrain so far has measured within noise of its predecessor.
Work dir kept for inspection: $WORK
EOF
