#!/bin/bash
# Round-trips + latency per render, per engine, straight from MySQL's own counters.
Q() { mysql -h127.0.0.1 -uroot -pfreeze -N -e "SHOW GLOBAL STATUS LIKE '$1'" 2>/dev/null | awk '{print $2}'; }
label=$1; port=$2; n=$3
# quiet the site first, then measure
for i in $(seq 1 20); do curl -s -o /dev/null "http://127.0.0.1:$port/?cb=warm$RANDOM$i"; done
q0=$(Q Questions); t0=$(date +%s.%N)
for i in $(seq 1 $n); do curl -s -o /dev/null "http://127.0.0.1:$port/?cb=$RANDOM$i"; done
t1=$(date +%s.%N); q1=$(Q Questions)
awk -v l="$label" -v q="$((q1-q0))" -v n="$n" -v t0="$t0" -v t1="$t1" \
  'BEGIN{printf "%-10s %6.1f queries/render   %6.2f ms wall/render\n", l, q/n, (t1-t0)*1000/n}'
