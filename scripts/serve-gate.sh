#!/usr/bin/env bash
#
# Serve `tests/runner.cfm` over HTTP and assert a clean run on TWO CONSECUTIVE
# requests to the SAME server process.
#
# Usage: scripts/serve-gate.sh <binary> <port> [--production]
#
# Why two requests, and why the same process: the bytecode cache, the template
# freshness cache, the component-path cache and the cross-request cycle-GC
# survivor set all behave differently once a process is warm, and differently
# again under `--production`. A bug that only bites on request 2+ is invisible
# to any single-request check, and we have shipped several — GH #284 stayed red
# in production for 23 releases because the runner was only ever served in dev.
#
# Harness rules below are not stylistic; each one is a failure we have had
# (see GH #415):
#
#   1. Readiness is probed on a STATIC asset, never on the endpoint under test.
#      Probing the runner with a curl timeout does not stop the server executing
#      it — curl walks away, the suite keeps running, and every retry launches
#      another overlapping run.
#   2. The server is killed by the PID captured at launch. Never by name: a
#      pattern matches every other engine process on the machine.
#   3. Failure is asserted on the HTTP status AND on the summary text. The
#      harness answers 500 on failure, but a crash mid-render can still return
#      200 with a truncated body, which the text check catches.
set -uo pipefail

BIN=${1:?usage: serve-gate.sh <binary> <port> [--production]}
PORT=${2:?usage: serve-gate.sh <binary> <port> [--production]}
MODE=${3:-}
LABEL="dev"; [ -n "$MODE" ] && LABEL="production"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

"$BIN" --serve . --port "$PORT" $MODE > "$WORK/serve.log" 2>&1 &
SERVER_PID=$!
# Kill only the PID we started, whatever happens from here.
trap 'kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; rm -rf "$WORK"' EXIT

# Readiness: a static asset that cannot do any work (rule 1).
ready=0
for _ in $(seq 1 60); do
  if [ "$(curl -s -m 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/crab.svg")" = "200" ]; then
    ready=1; break
  fi
  sleep 1
done
if [ "$ready" != "1" ]; then
  echo "::error::[$LABEL] server on port $PORT never became ready"
  sed -n 1,40p "$WORK/serve.log"
  exit 1
fi

rc=0
for pass in cold warm; do
  code=$(curl -s -m 1800 -o "$WORK/$pass.txt" -w '%{http_code}' "http://127.0.0.1:$PORT/tests/runner.cfm")
  summary=$(grep -E '^SUMMARY:' "$WORK/$pass.txt" | tail -1)

  if [ "$code" != "200" ]; then
    echo "::error::[$LABEL/$pass] runner returned HTTP $code (the harness answers 500 when the suite fails)"
    rc=1
  fi
  if [ -z "$summary" ]; then
    echo "::error::[$LABEL/$pass] no SUMMARY line — the run did not reach the end"
    rc=1
  fi
  # "SUMMARY: <passed>/<total> passed across <n> suites"
  if [ -n "$summary" ]; then
    passed=${summary#SUMMARY: }; passed=${passed%%/*}
    total=${summary#*/}; total=${total%% *}
    [ "$passed" != "$total" ] && { echo "::error::[$LABEL/$pass] $passed/$total assertions passed"; rc=1; }
  fi
  for marker in '^FAILED:' '^ERRORED:'; do
    if grep -qE "$marker" "$WORK/$pass.txt"; then
      echo "::error::[$LABEL/$pass] $(grep -E "$marker" "$WORK/$pass.txt" | tail -1)"
      rc=1
    fi
  done

  echo "[$LABEL/$pass] HTTP $code — ${summary:-<no summary>}"
  if [ "$rc" != "0" ]; then
    echo "--- failures ---"
    grep -E '^(FAIL \||  - )' "$WORK/$pass.txt" | head -40
    break
  fi
done

exit $rc
