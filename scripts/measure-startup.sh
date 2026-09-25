#!/usr/bin/env bash
# SC-003 startup check (T121). Linux only.
#
# The egui baseline no longer exists (egui was removed, T125), so SC-003 is checked against its
# absolute budget only: launch to populated pods in under 3000 ms. The script launches a binary N
# times, reports milliseconds from spawn to the first log line matching a marker (median and all
# samples), and exits 1 if the median is at or above --budget-ms.
#
#   scripts/measure-startup.sh --binary target/release/fathom-desktop [--marker REGEX] [--budget-ms 3000]
#
# The default marker ("Sending .* contexts to UI") is the backend's context-list handoff, the
# latest milestone visible on stdout without a UI driver. It is a lower bound on, NOT the same as,
# "populated pods". For SC-003 proper pass a marker that only appears once pods are listed if the
# backend logs one, or use the tauri-driver smoke test (ui/tests/driver/smoke.test.ts).
#
# Usage: scripts/measure-startup.sh [--binary PATH] [--marker REGEX] [--runs N] [--timeout SECS]
#                                   [--budget-ms MS]
set -euo pipefail

BIN="target/release/fathom-desktop"
MARKER='Sending [0-9]+ contexts to UI'
RUNS=5
TIMEOUT=30
BUDGET_MS=3000
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BIN="$2"; shift 2 ;;
    --marker) MARKER="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --budget-ms) BUDGET_MS="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
[[ -x "$BIN" ]] || { echo "binary not found or not executable: $BIN" >&2; exit 2; }

samples=()
for ((i = 1; i <= RUNS; i++)); do
  start=$(date +%s%N)
  ms=""
  # Merge stderr: tracing writes there. Kill the app as soon as the marker shows up.
  while IFS= read -r line; do
    if [[ "$line" =~ $MARKER ]]; then
      ms=$((($(date +%s%N) - start) / 1000000))
      break
    fi
  done < <(timeout "$TIMEOUT" "$BIN" 2>&1)
  # Only this run's process tree (never other running instances of the same binary).
  pkill -P "$!" 2>/dev/null || true
  kill "$!" 2>/dev/null || true
  if [[ -z "$ms" ]]; then
    echo "run $i: marker not seen within ${TIMEOUT}s" >&2
  else
    echo "run $i: ${ms} ms"
    samples+=("$ms")
  fi
  sleep 2
done

if ((${#samples[@]} == 0)); then
  echo "no samples" >&2
  exit 1
fi
median=$(printf '%s\n' "${samples[@]}" | sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}')
echo "binary=$BIN marker=/$MARKER/ samples_ms=${samples[*]} median_ms=$median"
if ((median < BUDGET_MS)); then
  echo "PASS: median ${median} ms < ${BUDGET_MS} ms (SC-003)"
else
  echo "FAIL: median ${median} ms >= ${BUDGET_MS} ms (SC-003)" >&2
  exit 1
fi
