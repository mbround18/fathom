#!/usr/bin/env bash
# Fathom CI, run through paws (https://github.com/mbround18/paws) so the same command is the CI
# job locally and on GitHub (.github/workflows/ci.yml calls this script, nothing else).
#
#   scripts/ci.sh [changed|shared|fathom|fathom-full|depths|keel|all]      default: changed
#
# One slice per app, so a change pays only for the app it touches:
#   shared       fmt, cargo-deny, the shared crates' own tests and dependency guards, and the
#                shared pnpm packages
#   fathom       Fathom's crates and UI; e2e on chromium at phone, laptop and wide
#   fathom-full  the same with the whole e2e matrix (7 widths x chromium and webkit, ~8k tests)
#   depths       Depths' crates, bindings and web app
#   keel         Keel's crates, bindings and web app
#   changed      the slices that the diff against CI_BASE touches (committed and uncommitted)
#   all          shared, fathom-full, depths and keel: the release gate
#
# Each job is one `paws run`: a container (rust:1-bookworm, or the Playwright image for the UI)
# built from the working tree filtered by .gitignore on the host, so a local run sees what a fresh
# checkout sees: no target/, no node_modules/, no ui/dist. Uncommitted edits ARE included.
#
# Knobs (environment):
#   CARGO_BUILD_JOBS    cargo parallelism inside the container      (default 4)
#   PLAYWRIGHT_WORKERS  Playwright workers inside the container     (default 2)
#   CI_NICE             nice level for paws and every step          (default 10)
#   CI_MIN_FREE_GB      GB of available memory the ui job needs     (default 8; depths needs half)
#   VITEST_WORKERS      forks in vitest's pool                      (default 6)
#   CI_BASE             what `changed` diffs against              (default origin/main)
#   CI_LOCK_WAIT        0 = exit 75 if another run holds the machine-wide lock   (default 1: wait)
#   CI_RUN_ID           part of every step's cache key         (default: new per invocation)
#                       Dagger caches a step whose inputs are unchanged and replays its old
#                       output, so without this a second run on the same tree re-runs nothing.
#                       Set it to a fixed value to deliberately reuse a previous run's result.
#
# Needs: paws (with `paws run`) and Docker; paws installs its own dagger engine.
set -euo pipefail

cd "$(dirname "$0")/.."

: "${CARGO_BUILD_JOBS:=4}"
: "${PLAYWRIGHT_WORKERS:=2}"
# One fork per core loses the startup handshake once the step is niced; see ui/vitest.config.ts.
: "${VITEST_WORKERS:=6}"
: "${CI_NICE:=10}"
: "${CI_MIN_FREE_GB:=8}"
: "${CI_RUN_ID:=$(date +%s%N)}"
export CARGO_BUILD_JOBS PLAYWRIGHT_WORKERS VITEST_WORKERS CI_RUN_ID DAGGER_NO_NAG=1

# System libraries for the Tauri crate (webkit2gtk-4.1 et al.), cmake for protobuf-src's protoc
# build, and libdbus for tao (the os-keyring feature talks to Secret Service over pure-Rust zbus).
RUST_APT=libgtk-3-dev,libssl-dev,pkg-config,libxkbcommon-dev,libwebkit2gtk-4.1-dev,libxdo-dev
RUST_APT+=,libayatana-appindicator3-dev,librsvg2-dev,libdbus-1-dev,cmake
# Keep in step with @playwright/test in pnpm-lock.yaml: the image ships matching browsers
# and their system deps, so there is no `playwright install --with-deps` step.
PLAYWRIGHT_IMAGE=mcr.microsoft.com/playwright:v1.64.1-noble

# Every step runs niced inside the container too: paws's own niceness does not reach the engine.
N="nice -n ${CI_NICE}"

# ui/dist is embedded by the Tauri crate at compile time; the Rust jobs only need it to exist.
STUB_UI_DIST="mkdir -p ui/dist && { [ -f ui/dist/index.html ] || echo '<!doctype html><title>Fathom</title>' > ui/dist/index.html; }"

# timed_steps --step <cmd> ...: wraps each step so it prints "ci-time: <secs>s <label>" from inside
# the container. run_job collects those lines into the slice's timing report and history.
timed_steps() {
  TIMED=()
  local label
  while [ $# -gt 0 ]; do
    if [ "$1" = --step ] && [ $# -gt 1 ]; then
      label=$(printf '%s' "${2//"$N "/}" | tr -cd 'A-Za-z0-9 _./:=@-' | cut -c1-70)
      TIMED+=(--step "t0=\$(date +%s); ( $2 ); rc=\$?; echo \"ci-time: \$((\$(date +%s) - t0))s $label\"; exit \$rc")
      shift 2
    else
      TIMED+=("$1")
      shift
    fi
  done
}

rust_run() {
  timed_steps --step "$STUB_UI_DIST" "$@"
  nice -n "$CI_NICE" paws run \
    --toolchain rust \
    --apt "$RUST_APT" \
    --setup "rustup component add clippy rustfmt" \
    --env CI=true --env CARGO_TERM_COLOR=always --env "RUSTFLAGS=-D warnings" \
    --env CARGO_INCREMENTAL=0 --env CARGO_BUILD_JOBS --env CI_RUN_ID \
    --cache fathom-ci-cargo-registry=/usr/local/cargo/registry \
    --cache fathom-ci-cargo-git=/usr/local/cargo/git \
    --cache fathom-ci-target=/src/target \
    --cache fathom-ci-cargo-tools=/opt/cargo-tools \
    "${TIMED[@]}"
}

# 050: the shared container runtime must stay app-agnostic. Fails if its dependency tree reaches
# kube, tauri or any fathom crate (contracts/shared-crates.md).
CONTAINER_RUNTIME_DEPS="if $N cargo tree -p container-runtime --prefix none \
| grep -E '^(kube|tauri|fathom)([- ]|\$)'; then \
echo 'error: crates/container-runtime depends on kube, tauri or fathom (above).' >&2; exit 1; fi"

# Spec 050 (T020): web-auth and audit-log are shared with Depths and must not pull in kube or
# any fathom crate.
SHARED_AUTH_DEPS_CHECK="$N cargo tree -p web-auth -p audit-log --prefix none --format '{p}' \
> /tmp/shared-auth-deps && if grep -E '^(kube|fathom)([- ]|\$)' /tmp/shared-auth-deps; then \
echo 'error: web-auth/audit-log depend on kube or fathom (above).' >&2; exit 1; fi"

# Spec 050 (T096): advisories, licences and sources per deny.toml. cargo-deny is installed once
# into a cached volume; the rust image's own /usr/local/cargo/bin holds the toolchain proxies.
CARGO_DENY="{ [ -x /opt/cargo-tools/bin/cargo-deny ] \
|| $N cargo install --locked --root /opt/cargo-tools cargo-deny; } \
&& $N /opt/cargo-tools/bin/cargo-deny check"

# Crates shared by more than one app. A change to one runs the shared slice and every app that
# depends on it (slices_for below).
SHARED_CRATES="-p container-runtime -p audit-log -p web-auth -p web-passkeys"

job_shared() {
  need_mem $((CI_MIN_FREE_GB / 2)) shared || return 1
  rust_run \
    --step "$CONTAINER_RUNTIME_DEPS" \
    --step "$SHARED_AUTH_DEPS_CHECK" \
    --step "$N cargo fmt --check" \
    --step "$N cargo clippy $SHARED_CRATES --all-targets -- -D warnings" \
    --step "$N cargo test $SHARED_CRATES" \
    --step "$CARGO_DENY" || return 1
  node_run . \
    --step "$N pnpm -w run lint:packages" \
    --step "$N pnpm -w run typecheck:packages" \
    --step "$N pnpm audit --prod --audit-level high"
}

# Fathom: the root crate, its daemon, server, desktop app and tools, and the shared crates they
# build on.
FATHOM_CRATES="-p fathom -p fathom-daemon -p fathom-server -p fathom-devbridge -p pipeline \
-p container-runtime -p audit-log -p web-auth"
# Per change: one engine at three widths. fathom-full runs every project in ui/playwright.config.ts.
FATHOM_E2E_QUICK="--project=chromium-phone --project=chromium-laptop --project=chromium-wide"

fathom_slice() {
  local e2e_projects=$1
  # Constitution Principle III: generated bindings must not drift. Snapshot, regenerate, compare
  # (diff -r also reports newly generated files, like `git add -N` + `git diff` did).
  local drift="rm -rf /tmp/generated-before && cp -r ui/src/protocol/generated /tmp/generated-before \
&& $N cargo test -p fathom --test export_bindings \
&& if ! diff -r /tmp/generated-before ui/src/protocol/generated; then \
echo 'error: ui/src/protocol/generated is stale. Run cargo test -p fathom --test export_bindings and commit the result.' >&2; exit 1; fi"
  need_mem "$CI_MIN_FREE_GB" fathom || return 1
  rust_run \
    --step "$N cargo check -p fathom --no-default-features --lib" \
    --step "$N cargo clippy $FATHOM_CRATES --all-targets -- -D warnings" \
    --step "$N cargo clippy -p fathom-desktop -- -D warnings" \
    --step "$N scripts/check-no-ldap.sh" \
    --step "$N cargo test $FATHOM_CRATES" \
    --step "$drift" || return 1
  # Installing from ui/ installs the whole workspace against the root pnpm-lock.yaml.
  node_run ui \
    --step "$N pnpm lint" \
    --step "$N pnpm typecheck" \
    --step "$N pnpm test" \
    --step "$N pnpm build" \
    --step "$N pnpm e2e $e2e_projects --workers=\"\$PLAYWRIGHT_WORKERS\""
}

job_fathom() { fathom_slice "$FATHOM_E2E_QUICK"; }
job_fathom-full() { fathom_slice ""; }

# need_mem <GB> <job>: refuse to start a Playwright job the machine has no room for.
need_mem() {
  local avail
  avail=$(free -g | awk '/^Mem:/ {print $7}')
  if [ "${avail:-0}" -lt "$1" ]; then
    echo "ci: only ${avail}GB memory available, the $2 job wants ${1}GB (CI_MIN_FREE_GB)." >&2
    echo "ci: wait for other Playwright/cargo runs to finish, or lower PLAYWRIGHT_WORKERS." >&2
    return 1
  fi
}

# node_run <workdir> --step ...: the Playwright image (node, pnpm via corepack, browsers) with the
# pnpm workspace installed. pnpm comes from packageManager via corepack.
node_run() {
  local workdir=$1
  shift
  timed_steps --step "corepack enable" \
    --step "$N pnpm install --frozen-lockfile --store-dir /pnpm-store" "$@"
  nice -n "$CI_NICE" paws run \
    --image "$PLAYWRIGHT_IMAGE" \
    --workdir "$workdir" \
    --env CI=true --env PLAYWRIGHT_WORKERS --env VITEST_WORKERS --env CI_RUN_ID --env COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    --cache fathom-ci-pnpm-store=/pnpm-store \
    --cache fathom-ci-corepack=/root/.cache/node/corepack \
    "${TIMED[@]}"
}

# Spec 050: Depths' crates (and the shared ones it builds on), then its web app against the fake
# backend.
DEPTHS_WEB=@companion/depths-web
DEPTHS_CRATES="-p depths-core -p depths-protocol -p depths-runtime-cli -p depths-stacks-fs \
-p depths-server -p depths-volumes -p depths-volume-helper -p container-runtime -p web-auth \
-p audit-log"

job_depths() {
  need_mem $((CI_MIN_FREE_GB / 2)) depths || return 1
  # Spec 050 (T034): Depths' protocol bindings must not drift, as for Fathom's in fathom_slice.
  local depths_drift="rm -rf /tmp/depths-generated-before \
&& cp -r apps/depths/web/src/protocol/generated /tmp/depths-generated-before \
&& $N cargo test -p depths-protocol --test export_bindings \
&& if ! diff -r /tmp/depths-generated-before apps/depths/web/src/protocol/generated; then \
echo 'error: apps/depths/web/src/protocol/generated is stale. Run cargo test -p depths-protocol --test export_bindings and commit the result.' >&2; exit 1; fi"
  rust_run \
    --step "$N cargo clippy $DEPTHS_CRATES --all-targets -- -D warnings" \
    --step "$N cargo test $DEPTHS_CRATES" \
    --step "$depths_drift" || return 1
  node_run apps/depths/web \
    --step "$N pnpm --filter $DEPTHS_WEB lint" \
    --step "$N pnpm --filter $DEPTHS_WEB typecheck" \
    --step "$N pnpm --filter $DEPTHS_WEB test" \
    --step "$N pnpm --filter $DEPTHS_WEB build" \
    --step "$N pnpm --filter $DEPTHS_WEB e2e --workers=\"\$PLAYWRIGHT_WORKERS\""
}

# Spec 055: Keel's crates (and the shared ones it builds on), its bindings, then its web app
# against the fake backend. keel-server's database tests skip without KEEL_TEST_DATABASE_URL.
KEEL_WEB=@companion/keel-web
KEEL_CRATES="-p keel-core -p keel-exec -p keel-git -p keel-protocol -p keel-trust \
-p keel-server -p keel-runner -p web-auth -p web-passkeys"

job_keel() {
  need_mem $((CI_MIN_FREE_GB / 2)) keel || return 1
  local keel_drift="rm -rf /tmp/keel-generated-before \
&& cp -r apps/keel/web/src/protocol/generated /tmp/keel-generated-before \
&& $N cargo test -p keel-protocol --test export_bindings \
&& if ! diff -r /tmp/keel-generated-before apps/keel/web/src/protocol/generated; then \
echo 'error: apps/keel/web/src/protocol/generated is stale. Run cargo test -p keel-protocol --test export_bindings and commit the result.' >&2; exit 1; fi"
  rust_run \
    --step "$N cargo clippy $KEEL_CRATES --all-targets -- -D warnings" \
    --step "$N cargo test $KEEL_CRATES" \
    --step "$keel_drift" || return 1
  node_run apps/keel/web \
    --step "$N pnpm --filter $KEEL_WEB lint" \
    --step "$N pnpm --filter $KEEL_WEB typecheck" \
    --step "$N pnpm --filter $KEEL_WEB test" \
    --step "$N pnpm --filter $KEEL_WEB build" \
    --step "$N pnpm --filter $KEEL_WEB e2e --workers=\"\$PLAYWRIGHT_WORKERS\""
}

# slices_for <path>...: the slices a set of changed paths needs, one per line. Docs and specs
# need none; anything not claimed by an app or the shared code is Fathom's.
slices_for() {
  local f
  for f in "$@"; do
    case "$f" in
      specs/* | docs/* | *.md | .specify/* | .claude/* | LICENSE*) ;;
      apps/depths/* | crates/depths-*) echo depths ;;
      apps/keel/* | crates/keel-*) echo keel ;;
      crates/container-runtime/* | crates/audit-log/*) printf '%s\n' shared fathom depths ;;
      crates/web-auth/*) printf '%s\n' shared fathom depths keel ;;
      crates/web-passkeys/*) printf '%s\n' shared keel ;;
      packages/*) printf '%s\n' shared fathom depths keel ;;
      Cargo.toml | Cargo.lock | deny.toml | rust-toolchain* | package.json | pnpm-lock.yaml \
        | pnpm-workspace.yaml | scripts/ci.sh | .github/*) printf '%s\n' shared fathom depths keel ;;
      *) echo fathom ;;
    esac
  done
}

job_changed() {
  local base=${CI_BASE:-origin/main} slices job failed=()
  mapfile -t slices < <(
    { git diff --name-only "$(git merge-base "$base" HEAD)"; git ls-files --others --exclude-standard; } \
      | sort -u | { mapfile -t files; slices_for "${files[@]}"; } | sort -u
  )
  if [ "${#slices[@]}" -eq 0 ]; then
    echo "ci: nothing to run: no changes against $base outside docs and specs"
    return 0
  fi
  echo "ci: changes against $base touch: ${slices[*]}"
  # shared first: it is the cheapest and every other slice builds on it.
  for job in shared fathom depths keel; do
    [[ " ${slices[*]} " == *" $job "* ]] || continue
    run_job "$job" || failed+=("$job")
  done
  [ "${#failed[@]}" -eq 0 ] || { echo "ci: FAILED (${failed[*]})" >&2; return 1; }
}

# Every slice's timings go here, one line per run: when, commit, slice, pass|fail, total seconds,
# then its steps slowest first. `column -t -s$'\t' <file> | tail` shows the trend.
CI_TIMES=${CI_TIMES:-${XDG_STATE_HOME:-$HOME/.local/state}/fathom/ci-times.tsv}

# report_times <slice> <pass|fail> <total secs> <log>: the slowest steps, what the engine spent
# outside them (image pulls, cache mounts, exports), and the change against the last pass.
report_times() {
  local name=$1 result=$2 total=$3 log=$4 steps sum prev
  steps=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -oE 'ci-time: [0-9]+s .*' | sed 's/^ci-time: //' \
    | awk '!seen[$0]++' | sort -rn)
  sum=$(awk '{ s += $1 } END { print s + 0 }' <<<"$steps")
  prev=$(awk -F'\t' -v n="$name" '$3 == n && $4 == "pass" { t = $5 } END { print t }' "$CI_TIMES" 2>/dev/null)
  echo "ci: $name steps, slowest first:"
  sed 's/^/ci:   /' <<<"$steps"
  echo "ci:   $((total - sum))s outside steps (engine, images, caches)"
  [ -n "$prev" ] && echo "ci: $name last passed in ${prev}s; this run $((total - prev))s against it"
  mkdir -p "$(dirname "$CI_TIMES")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(date -Is)" "$(git rev-parse --short HEAD)" "$name" "$result" \
    "$total" "$(paste -sd ';' <<<"$steps")" >>"$CI_TIMES"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    { echo "### $name: $result in ${total}s"; echo; sed 's/^\([0-9]*s\) \(.*\)/- \1 `\2`/' <<<"$steps"; } \
      >>"$GITHUB_STEP_SUMMARY"
  fi
}

run_job() {
  local name=$1 start log rc total
  start=$(date +%s)
  log=$(mktemp)
  echo "ci: == $name =="
  "job_$name" 2>&1 | tee "$log"
  rc=${PIPESTATUS[0]}
  total=$(($(date +%s) - start))
  if [ "$rc" -eq 0 ]; then
    report_times "$name" pass "$total" "$log"
    echo "ci: $name passed in ${total}s"
  else
    report_times "$name" fail "$total" "$log"
    echo "ci: $name FAILED after ${total}s" >&2
  fi
  rm -f "$log"
  return "$rc"
}

command -v paws >/dev/null || { echo "ci: paws is not on PATH (cargo install --git https://github.com/mbround18/paws paws-cli)" >&2; exit 1; }
paws run --help >/dev/null 2>&1 || { echo "ci: this paws has no \`paws run\`; install a newer one" >&2; exit 1; }

# One run per machine, across every checkout and worktree: parallel runs once drove load past 200
# on 32 cores and restarted the Dagger engine mid-run. A second run waits for the first; with
# CI_LOCK_WAIT=0 it exits 75 instead. GitHub runners are single-tenant and skip the lock.
if [ -z "${GITHUB_ACTIONS:-}" ] && command -v flock >/dev/null; then
  lock="${XDG_RUNTIME_DIR:-/tmp}/fathom-ci.lock"
  exec 9>>"$lock"
  if ! flock -n 9; then
    echo "ci: another run holds $lock: $(cat "$lock.owner" 2>/dev/null || echo unknown)" >&2
    [ "${CI_LOCK_WAIT:-1}" = 0 ] && exit 75
    echo "ci: waiting for it to finish (CI_LOCK_WAIT=0 to exit instead)" >&2
    flock 9
  fi
  echo "pid $$ in $PWD since $(date '+%H:%M:%S'): $0 ${1:-changed}" > "$lock.owner"
fi

case "${1:-changed}" in
  shared | fathom | fathom-full | depths | keel) run_job "$1" ;;
  changed) job_changed ;;
  all)
    # Like the workflow's matrix (fail-fast: false): every slice runs, then one verdict. One at a
    # time (two Playwright suites at once do not fit), the quick slices first so they report early.
    overall=$(date +%s)
    failed=()
    for job in shared depths keel fathom-full; do
      run_job "$job" || failed+=("$job")
    done
    if [ "${#failed[@]}" -gt 0 ]; then
      echo "ci: FAILED (${failed[*]}) in $(($(date +%s) - overall))s" >&2
      exit 1
    fi
    echo "ci: all passed in $(($(date +%s) - overall))s"
    ;;
  *)
    echo "usage: $0 [changed|shared|fathom|fathom-full|depths|keel|all]" >&2
    exit 2
    ;;
esac
