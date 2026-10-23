#!/usr/bin/env bash
# Fathom CI, run through paws (https://github.com/mbround18/paws) so the same command is the CI
# job locally and on GitHub (.github/workflows/ci.yml calls this script, nothing else).
#
#   scripts/ci.sh [check|test|ui|all]      default: all
#
# Each job is one `paws run`: a container (rust:1-bookworm, or the Playwright image for the UI)
# built from the working tree filtered by .gitignore on the host, so a local run sees what a fresh
# checkout sees: no target/, no node_modules/, no ui/dist. Uncommitted edits ARE included.
#
# Knobs (environment):
#   CARGO_BUILD_JOBS    cargo parallelism inside the container      (default 4)
#   PLAYWRIGHT_WORKERS  Playwright workers inside the container     (default 2)
#   CI_NICE             nice level for paws and every step          (default 10)
#   CI_MIN_FREE_GB      GB of available memory the ui job needs     (default 8)
#   VITEST_WORKERS      forks in vitest's pool                      (default 6)
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
# build, and libdbus for the os-keyring feature.
RUST_APT=libgtk-3-dev,libssl-dev,pkg-config,libxkbcommon-dev,libwebkit2gtk-4.1-dev,libxdo-dev
RUST_APT+=,libayatana-appindicator3-dev,librsvg2-dev,libdbus-1-dev,cmake
# Keep in step with @playwright/test in ui/pnpm-lock.yaml: the image ships matching browsers
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
    --step "$STUB_UI_DIST" \
    "$@"
}

job_check() {
  rust_run \
    --step "$N cargo check --all-targets" \
    --step "$N cargo check --no-default-features --lib" \
    --step "$N cargo clippy --workspace --all-targets -- -D warnings" \
    --step "$N cargo clippy -p fathom-desktop -- -D warnings" \
    --step "$N cargo fmt --check" \
    --step "$N scripts/check-no-ldap.sh"
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

job_ui() {
  local avail
  avail=$(free -g | awk '/^Mem:/ {print $7}')
  if [ "${avail:-0}" -lt "$CI_MIN_FREE_GB" ]; then
    echo "ci: only ${avail}GB memory available, the ui job wants ${CI_MIN_FREE_GB}GB (CI_MIN_FREE_GB)." >&2
    echo "ci: wait for other Playwright/cargo runs to finish, or lower PLAYWRIGHT_WORKERS." >&2
    return 1
  fi
  # pnpm comes from ui/package.json's packageManager via corepack.
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
    --step "$N pnpm e2e --workers=\"\$PLAYWRIGHT_WORKERS\""
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

case "${1:-all}" in
  check | test | ui) run_job "$1" ;;
  all)
    # Like the workflow's matrix (fail-fast: false): every job runs, then one verdict.
    overall=$(date +%s)
    failed=()
    for job in check test ui; do
      run_job "$job" || failed+=("$job")
    done
    if [ "${#failed[@]}" -gt 0 ]; then
      echo "ci: FAILED (${failed[*]}) in $(($(date +%s) - overall))s" >&2
      exit 1
    fi
    echo "ci: all passed in $(($(date +%s) - overall))s"
    ;;
  *)
    echo "usage: $0 [check|test|ui|all]" >&2
    exit 2
    ;;
esac
