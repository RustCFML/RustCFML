#!/bin/bash
# Item #1: the true shipped-equivalent gap to Lucee.
#   arm A  RustCFML PGO, debug footer ON  (the user's current dev config)
#   arm B  RustCFML PGO, debug footer OFF (production-equivalent)
#   arm C  Lucee 7, debugging off
# Restores the site's .cfconfig.json on ANY exit.
set -uo pipefail
S=/private/tmp/claude-501/-Users-alexskinner-Repos-opensource-CFMLs-RustCFML/6376d428-1b98-4ad1-889b-028341115cab/scratchpad
SITE=/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website
CFG=$SITE/.cfconfig.json
PGO=/Users/alexskinner/Repos/opensource/CFMLs/RustCFML/target/release-pgo/rustcfml

cp -p "$CFG" "$S/cfconfig.backup.json"
restore() { cp -p "$S/cfconfig.backup.json" "$CFG"; echo "[restored .cfconfig.json]"; }
trap restore EXIT

start_rcfml() { # port
  cd "$SITE"; "$PGO" --serve --production ./ --port "$1" > "$S/parity_rcfml_$1.log" 2>&1 &
  echo $!
}
wait_up() { # port
  for i in $(seq 1 90); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$1/?cb=boot$RANDOM")" = "200" ] && return 0
    sleep 1
  done
  echo "FAILED to boot on $1" >&2; return 1
}

echo "### ARM A — RustCFML PGO, debug footer ON (current dev config)"
PA=$(start_rcfml 8507); wait_up 8507 || exit 1
python3 "$S/xengine_ab.py" "rcfml-dbg:8507:$PA" 2>&1 | tail -12
kill $PA 2>/dev/null; wait $PA 2>/dev/null

echo
echo "### toggling debugging.enabled -> false (both engines) and restarting Lucee"
python3 - <<PY
import json
p="$CFG"
d=json.load(open(p))
d.setdefault("debugging",{})["enabled"]=False
json.dump(d,open(p,"w"),indent=4)
print("  debugging.enabled =", d["debugging"]["enabled"])
PY
cd "$SITE"
box server restart name=website2 > /dev/null 2>&1
for i in $(seq 1 90); do
  [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:8585/?cb=boot$RANDOM")" = "200" ] && break
  sleep 1
done
LUCEE_PID=$(lsof -ti :8585 | head -1)
echo "  lucee pid $LUCEE_PID"
# confirm the footer is really gone from the RustCFML side
PB=$(start_rcfml 8508); wait_up 8508 || exit 1
SZ_R=$(curl -s "http://127.0.0.1:8508/?cb=x1" | wc -c)
SZ_L=$(curl -s "http://127.0.0.1:8585/?cb=x1" | wc -c)
echo "  page bytes: rustcfml $SZ_R  lucee $SZ_L  (must be comparable now)"

echo
echo "### ARMS B vs C — RustCFML PGO (debug off) vs Lucee 7, interleaved"
python3 "$S/xengine_ab.py" "rustcfml:8508:$PB" "lucee:8585:$LUCEE_PID"
kill $PB 2>/dev/null; wait $PB 2>/dev/null
