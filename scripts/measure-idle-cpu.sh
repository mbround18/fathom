#!/usr/bin/env bash
# SC-005 idle CPU check (T120 / quickstart V13). Linux only.
#
# Launches the built desktop binary (or attaches to a running one), waits for it to settle, then
# samples utime+stime from /proc/<pid>/stat for the app process AND its descendants
# (WebKitWebProcess / WebKitNetworkProcess do the rendering work) over a window, and reports the
# average as a percentage of ONE core. Exits non-zero at or above the budget (default 2%).
#
# Usage: scripts/measure-idle-cpu.sh [--binary PATH] [--settle SECS] [--window SECS]
#                                    [--budget PCT] [--attach PID]
#
# NOTE: this measures whatever view the app is showing. SC-005 is defined with the app idle on
# Pods; getting there needs a UI driver (tauri-driver) or a human. For the manual procedure,
# launch the app yourself, open Pods, and pass --attach <pid>.
set -euo pipefail

BIN="${BIN:-target/release/fathom-desktop}"
SETTLE=10
WINDOW=60
BUDGET=2
ATTACH=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BIN="$2"; shift 2 ;;
    --settle) SETTLE="$2"; shift 2 ;;
    --window) WINDOW="$2"; shift 2 ;;
    --budget) BUDGET="$2"; shift 2 ;;
    --attach) ATTACH="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

HZ="$(getconf CLK_TCK)"
LAUNCHED=""
cleanup() { if [[ -n "$LAUNCHED" ]]; then kill "$LAUNCHED" 2>/dev/null || true; fi; }
trap cleanup EXIT

if [[ -n "$ATTACH" ]]; then
  PID="$ATTACH"
else
  [[ -x "$BIN" ]] || { echo "binary not found or not executable: $BIN" >&2; exit 2; }
  "$BIN" >/dev/null 2>&1 &
  PID=$!
  LAUNCHED=$PID
fi

descendants() { # pid plus all descendants
  local p="$1" c
  echo "$p"
  for c in $(pgrep -P "$p" 2>/dev/null || true); do descendants "$c"; done
}

ticks() { # total utime+stime ticks across the process tree
  local total=0 p t
  for p in $(descendants "$PID"); do
    # Strip "pid (comm) " first (comm may contain spaces); then fields 12,13 are utime,stime.
    t=$(sed -E 's/^[0-9]+ \(.*\) //' "/proc/$p/stat" 2>/dev/null | awk '{print $12+$13}' || true)
    total=$((total + ${t:-0}))
  done
  echo "$total"
}

kill -0 "$PID" 2>/dev/null || { echo "process $PID is not running" >&2; exit 2; }
echo "pid=$PID settle=${SETTLE}s window=${WINDOW}s budget=${BUDGET}%"
sleep "$SETTLE"
kill -0 "$PID" 2>/dev/null || { echo "process exited during settle" >&2; exit 2; }

T0=$(ticks); S0=$(date +%s.%N)
sleep "$WINDOW"
T1=$(ticks); S1=$(date +%s.%N)

PCT=$(awk -v t0="$T0" -v t1="$T1" -v s0="$S0" -v s1="$S1" -v hz="$HZ" \
  'BEGIN { printf "%.2f", ((t1 - t0) / hz) / (s1 - s0) * 100 }')
echo "idle_cpu_percent_of_one_core=$PCT (tree ticks $T0 -> $T1 over ${WINDOW}s)"
if awk -v p="$PCT" -v b="$BUDGET" 'BEGIN { exit !(p < b) }'; then
  echo PASS
else
  echo "FAIL: >= ${BUDGET}%"
  exit 1
fi
