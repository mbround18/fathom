#!/usr/bin/env bash
# Fathom CI, run through paws (https://github.com/mbround18/paws) so the same command is the CI
# job locally and on GitHub (.github/workflows/ci.yml calls this script, nothing else).
#
#   scripts/ci.sh [check|test|depths|ui|all]      default: all
#
# `depths` is Depths alone (its crates, bindings and web app), minutes rather than the ui job's
# hour and a half, so a Depths change can be gated without the Fathom Playwright suite.
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

rust_run() {
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
    --step "$STUB_UI_DIST" \
    "$@"
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

job_check() {
  rust_run \
    --step "$CONTAINER_RUNTIME_DEPS" \
    --step "$SHARED_AUTH_DEPS_CHECK" \
    --step "$N cargo check --all-targets" \
    --step "$N cargo check --no-default-features --lib" \
    --step "$N cargo clippy --workspace --all-targets -- -D warnings" \
    --step "$N cargo clippy -p fathom-desktop -- -D warnings" \
    --step "$N cargo fmt --check" \
    --step "$N scripts/check-no-ldap.sh" \
    --step "$CARGO_DENY"
}

job_test() {
  # Constitution Principle III: generated bindings must not drift. Snapshot, regenerate, compare
  # (diff -r also reports newly generated files, like `git add -N` + `git diff` did).
  local drift="rm -rf /tmp/generated-before && cp -r ui/src/protocol/generated /tmp/generated-before \
&& $N cargo test -p fathom --test export_bindings \
&& if ! diff -r /tmp/generated-before ui/src/protocol/generated; then \
echo 'error: ui/src/protocol/generated is stale. Run cargo test -p fathom --test export_bindings and commit the result.' >&2; exit 1; fi"
  rust_run \
    --step "$N cargo test --all" \
    --step "$drift"
}

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

# Spec 050: Depths' crates (and the shared ones it builds on), then its web app against the fake
# backend. The workspace-wide check and test jobs cover these crates too; this job is the fast
# path for a Depths-only change.
DEPTHS_WEB=@companion/depths-web
DEPTHS_CRATES="-p depths-core -p depths-protocol -p depths-runtime-cli -p depths-stacks-fs \
-p depths-server -p container-runtime -p web-auth -p audit-log"

job_depths() {
  need_mem $((CI_MIN_FREE_GB / 2)) depths || return 1
  # Spec 050 (T034): Depths' protocol bindings must not drift, as for Fathom's in job_test.
  local depths_drift="rm -rf /tmp/depths-generated-before \
&& cp -r apps/depths/web/src/protocol/generated /tmp/depths-generated-before \
&& $N cargo test -p depths-protocol --test export_bindings \
&& if ! diff -r /tmp/depths-generated-before apps/depths/web/src/protocol/generated; then \
echo 'error: apps/depths/web/src/protocol/generated is stale. Run cargo test -p depths-protocol --test export_bindings and commit the result.' >&2; exit 1; fi"
  rust_run \
    --step "$N cargo clippy $DEPTHS_CRATES --all-targets -- -D warnings" \
    --step "$N cargo test $DEPTHS_CRATES" \
    --step "$depths_drift" || return 1
  nice -n "$CI_NICE" paws run \
    --image "$PLAYWRIGHT_IMAGE" \
    --workdir apps/depths/web \
    --env CI=true --env PLAYWRIGHT_WORKERS --env VITEST_WORKERS --env CI_RUN_ID --env COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    --cache fathom-ci-pnpm-store=/pnpm-store \
    --cache fathom-ci-corepack=/root/.cache/node/corepack \
    --step "corepack enable" \
    --step "$N pnpm install --frozen-lockfile --store-dir /pnpm-store" \
    --step "$N pnpm --filter $DEPTHS_WEB lint" \
    --step "$N pnpm --filter $DEPTHS_WEB typecheck" \
    --step "$N pnpm --filter $DEPTHS_WEB test" \
    --step "$N pnpm --filter $DEPTHS_WEB build" \
    --step "$N pnpm --filter $DEPTHS_WEB e2e --workers=\"\$PLAYWRIGHT_WORKERS\""
}

job_ui() {
  need_mem "$CI_MIN_FREE_GB" ui || return 1
  # pnpm comes from packageManager via corepack. Installing from ui/ installs the whole workspace
  # against the root pnpm-lock.yaml.
  nice -n "$CI_NICE" paws run \
    --image "$PLAYWRIGHT_IMAGE" \
    --workdir ui \
    --env CI=true --env PLAYWRIGHT_WORKERS --env VITEST_WORKERS --env CI_RUN_ID --env COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    --cache fathom-ci-pnpm-store=/pnpm-store \
    --cache fathom-ci-corepack=/root/.cache/node/corepack \
    --step "corepack enable" \
    --step "$N pnpm install --frozen-lockfile --store-dir /pnpm-store" \
    --step "$N pnpm lint" \
    --step "$N pnpm typecheck" \
    --step "$N pnpm test" \
    --step "$N pnpm build" \
    --step "$N pnpm e2e --workers=\"\$PLAYWRIGHT_WORKERS\"" \
    --step "$N pnpm audit --prod --audit-level high"
}

run_job() {
  local name=$1 start
  start=$(date +%s)
  echo "ci: == $name =="
  if "job_$name"; then
    echo "ci: $name passed in $(($(date +%s) - start))s"
  else
    echo "ci: $name FAILED after $(($(date +%s) - start))s" >&2
    return 1
  fi
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
  echo "pid $$ in $PWD since $(date '+%H:%M:%S'): $0 ${1:-all}" > "$lock.owner"
fi

case "${1:-all}" in
  check | test | depths | ui) run_job "$1" ;;
  all)
    # Like the workflow's matrix (fail-fast: false): every job runs, then one verdict. One job at
    # a time (two Playwright suites at once do not fit), depths before ui so it reports early.
    overall=$(date +%s)
    failed=()
    for job in check test depths ui; do
      run_job "$job" || failed+=("$job")
    done
    if [ "${#failed[@]}" -gt 0 ]; then
      echo "ci: FAILED (${failed[*]}) in $(($(date +%s) - overall))s" >&2
      exit 1
    fi
    echo "ci: all passed in $(($(date +%s) - overall))s"
    ;;
  *)
    echo "usage: $0 [check|test|depths|ui|all]" >&2
    exit 2
    ;;
esac
