#!/usr/bin/env bash
# The gate while iterating: what scripts/ci.sh's changed slices would check for this diff, run on
# the host, without the Playwright suites, the production web builds or the containers. It is
# enough to keep building on a branch; scripts/ci.sh still runs once before a push or release.
#
#   scripts/gate.sh            (or: make gate)
#
# Steps run one at a time and stop at nothing: every step runs, then one verdict. Each step's
# output goes to $GATE_LOGS/<step>.log; the terminal gets one line per step with its time.
#
# Knobs (environment):
#   GATE_BASE         what the diff is taken against        (default origin/main)
#   GATE_SLICES       slices to run instead of the diff's, e.g. "shared keel"
#   GATE_LOGS         where step logs go                    (default target/gate)
#   CARGO_BUILD_JOBS  cargo parallelism                     (default 4: the box is shared)
set -uo pipefail

cd "$(dirname "$0")/.."
. scripts/lib/slices.sh

: "${CARGO_BUILD_JOBS:=4}"
export CARGO_BUILD_JOBS
logs=${GATE_LOGS:-target/gate}
mkdir -p "$logs"

if [ -n "${GATE_SLICES:-}" ]; then
  read -ra slices <<<"$GATE_SLICES"
else
  mapfile -t slices < <(changed_slices "${GATE_BASE:-origin/main}")
fi
if [ "${#slices[@]}" -eq 0 ]; then
  echo "gate: nothing to check: no changes against ${GATE_BASE:-origin/main} outside docs and specs"
  exit 0
fi
echo "gate: slices: ${slices[*]} (logs in $logs/)"
has() { [[ " ${slices[*]} " == *" $1 "* ]]; }

failed=()
overall=$(date +%s)
# step <name> <command>: runs it, logs to $logs/<name>.log, prints ok|FAIL and its time.
step() {
  local t0 rc
  t0=$(date +%s)
  printf '== %-10s ' "$1"
  bash -c "$2" >"$logs/$1.log" 2>&1
  rc=$?
  if [ "$rc" -eq 0 ]; then echo "ok   $(($(date +%s) - t0))s"; else
    echo "FAIL $(($(date +%s) - t0))s  ($logs/$1.log)"
    failed+=("$1")
  fi
}

# Cheapest first, so a formatting slip shows in seconds.
step fmt "cargo fmt --all --check"
if has shared; then
  step shared "cargo clippy $SHARED_CRATES --all-targets -- -D warnings && cargo test $SHARED_CRATES"
  step audit "if command -v cargo-deny >/dev/null; then cargo deny check; \
else echo 'cargo-deny not installed (cargo install cargo-deny --locked): skipped'; fi \
&& pnpm audit --prod --audit-level high"
fi
for app in depths keel; do
  has "$app" && step "$app" "make --no-print-directory -C apps/$app lint test bindings"
done
has fathom && step fathom "make --no-print-directory -C ui lint test bindings"

echo
if [ "${#failed[@]}" -gt 0 ]; then
  echo "gate: FAIL (${failed[*]}) in $(($(date +%s) - overall))s" >&2
  exit 1
fi
echo "gate: pass in $(($(date +%s) - overall))s. Not run: Playwright suites, web builds, containers (scripts/ci.sh)"
