#!/usr/bin/env bash
#
# Two-node cluster smoke test for application messaging (the cbjgroups shim).
#
# Usage: scripts/cluster-smoke.sh <binary> [basePort]
#
# Starts two `--serve` processes on localhost, joined by a top-level `cluster`
# block with static seeds, and checks what a single process cannot:
#   1. both nodes see the same two-member view, in the same order;
#   2. exactly one coordinator — the node that started first;
#   3. a node's messages reach the other, in send order, and not itself
#      (discardOwnMessages = true);
#   4. when the coordinator stops, the survivor becomes coordinator.
#
# The CFML suite covers the single-node behaviour (tests/stdlib/test_cbjgroups_shim.cfm).
# Servers are stopped by the PIDs captured at launch — never by name.
set -uo pipefail

BIN=${1:?usage: cluster-smoke.sh <binary> [basePort]}
BASE=${2:-8751}
HTTP_A=$BASE; HTTP_B=$((BASE + 1))
GOSSIP_A=$((BASE + 10)); GOSSIP_B=$((BASE + 11))

WORK=$(mktemp -d)
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; wait "$p" 2>/dev/null; done
  rm -rf "$WORK"
}
trap cleanup EXIT

for p in $HTTP_A $HTTP_B $GOSSIP_A $GOSSIP_B; do
  if lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port $p is in use — pass a different basePort" >&2; exit 2
  fi
done

node_dir() {
  local dir=$1 name=$2 listen=$3 seed=$4
  mkdir -p "$dir"
  cat > "$dir/.cfconfig.json" <<EOF
{ "cluster": { "listenAddr": "127.0.0.1:$listen", "nodeName": "$name",
  "discovery": { "method": "static", "seeds": [ "127.0.0.1:$seed" ] } } }
EOF
  echo 'component { this.name = "clustersmoke"; }' > "$dir/Application.cfc"
  cat > "$dir/Listener.cfc" <<'EOF'
component {
	function init() { variables.ctx = getPageContext().getApplicationContext(); return this; }
	public void function receive( required any msg ) {
		getPageContext().setApplicationContext( variables.ctx );
		var m = deserializeJson( toString( msg.getBuffer() ) );
		lock name="clustersmoke" type="exclusive" timeout=5 { arrayAppend( application.received, m.from & m.n ); }
	}
	public void function viewAccepted( required any view ) {}
}
EOF
  cat > "$dir/join.cfm" <<'EOF'
<cfscript>
if ( !structKeyExists( application, "w" ) ) {
	application.received = [];
	application.w = createObject( "java", "org.pixl8.cbjgroups.CbJGroupsClusterWrapper" ).init( "", true, new Listener(), {}, "/" );
	application.w.connect( "smoke" );
}
writeOutput( "ok" );
</cfscript>
EOF
  cat > "$dir/send.cfm" <<'EOF'
<cfscript>for ( i = 1; i <= 5; i++ ) application.w.sendMessage( serializeJson( { from = url.me, n = i } ) ); writeOutput( "sent" );</cfscript>
EOF
  cat > "$dir/status.cfm" <<'EOF'
<cfscript>st = application.w.getStats(); writeOutput( "coord=" & application.w.isCoordinator() & ";members=" & st.members.toList() & ";received=" & application.received.toList() );</cfscript>
EOF
}

node_dir "$WORK/a" nodeA $GOSSIP_A $GOSSIP_B
node_dir "$WORK/b" nodeB $GOSSIP_B $GOSSIP_A

wait_up() { for _ in $(seq 1 60); do curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$1/Application.cfc" && return 0; sleep 1; done; return 1; }

(cd "$WORK/a" && exec "$BIN" --serve . --port $HTTP_A > "$WORK/a.log" 2>&1) & PIDS+=($!)
wait_up $HTTP_A || { echo "node A did not start"; cat "$WORK/a.log"; exit 1; }
curl -s -o /dev/null "http://127.0.0.1:$HTTP_A/join.cfm"; sleep 1
(cd "$WORK/b" && exec "$BIN" --serve . --port $HTTP_B > "$WORK/b.log" 2>&1) & PIDS+=($!)
wait_up $HTTP_B || { echo "node B did not start"; cat "$WORK/b.log"; exit 1; }
curl -s -o /dev/null "http://127.0.0.1:$HTTP_B/join.cfm"

FAIL=0
check() { if [[ "$2" == *"$3"* ]]; then echo "ok   $1"; else echo "FAIL $1: expected [$3] in [$2]"; FAIL=1; fi; }
status() { curl -s "http://127.0.0.1:$1/status.cfm"; }

# Views converge within a few gossip rounds.
T0=$(date +%s)
for _ in $(seq 1 30); do
  [[ "$(status $HTTP_A)" == *"members=nodeA,nodeB"* && "$(status $HTTP_B)" == *"members=nodeA,nodeB"* ]] && break
  sleep 1
done
echo "     (views converged in $(( $(date +%s) - T0 ))s)"
A=$(status $HTTP_A); B=$(status $HTTP_B)
check "A sees both members, oldest first" "$A" "members=nodeA,nodeB"
check "B sees the same view"             "$B" "members=nodeA,nodeB"
check "A (started first) is coordinator" "$A" "coord=true"
check "B is not coordinator"             "$B" "coord=false"

curl -s -o /dev/null "http://127.0.0.1:$HTTP_A/send.cfm?me=A"
for _ in $(seq 1 20); do [[ "$(status $HTTP_B)" == *"A5"* ]] && break; sleep 0.5; done
check "B received A's messages in order" "$(status $HTTP_B)" "received=A1,A2,A3,A4,A5"
[[ "$(status $HTTP_A)" == *"received=" ]] && echo "ok   A did not hear itself" || { echo "FAIL A heard itself: $(status $HTTP_A)"; FAIL=1; }

kill "${PIDS[0]}"; wait "${PIDS[0]}" 2>/dev/null
for _ in $(seq 1 40); do [[ "$(status $HTTP_B)" == *"coord=true"* ]] && break; sleep 1; done
B=$(status $HTTP_B)
check "B takes over as coordinator"      "$B" "coord=true"
check "B's view is itself alone"         "$B" "members=nodeB;"

if [ $FAIL -ne 0 ]; then echo "cluster smoke: FAILED"; echo "--- a.log"; tail -20 "$WORK/a.log"; echo "--- b.log"; tail -20 "$WORK/b.log"; exit 1; fi
echo "cluster smoke: all checks passed"
