#!/usr/bin/env bash
# Cross-build the Windows NSIS installer from Linux, in a container, through paws
# (https://github.com/mbround18/paws). No Windows SDK, NSIS or LLVM on the host: cargo-xwin
# fetches the MSVC CRT and SDK into a cache volume and lld-link does the linking.
#
#   scripts/build-windows.sh            -> dist/windows/nsis/Fathom_<version>_x64-setup.exe
#
# Needs: paws with `paws run --export` (paws main after 2026-10-09; the 0.1.0 binary from the
# feat/fathom-local-ci branch lacks --export) and Docker. First run is ~10 min of cargo plus the
# one-time image layers (apt, NodeSource, cargo-xwin) and the SDK download; later runs reuse the
# fathom-win-* cache volumes. Validated 2026-10-14; the installer has NOT been run on Windows.
#
# Knobs: CARGO_BUILD_JOBS (default 8), PAWS (path to the paws binary, default: on PATH).
set -euo pipefail
cd "$(dirname "$0")/.."

: "${CARGO_BUILD_JOBS:=8}"
: "${PAWS:=paws}"
export CARGO_BUILD_JOBS DAGGER_NO_NAG=1

# GTK/WebKit dev packages are still needed: the Tauri crate's build script and the host-side
# build dependencies compile natively even when the app targets Windows. ninja-build is for
# protobuf-src's cmake, which defaults to Ninja under cargo-xwin.
APT=curl,ca-certificates,nsis,lld,llvm,clang,cmake,ninja-build,pkg-config,libssl-dev
APT+=,libgtk-3-dev,libwebkit2gtk-4.1-dev,libxdo-dev,libayatana-appindicator3-dev,librsvg2-dev,libdbus-1-dev

# --workdir ui so corepack resolves pnpm from ui/package.json's packageManager (from the repo
# root it picks the latest pnpm and the version check fails).
exec "$PAWS" run --toolchain rust --workdir ui \
  --apt "$APT" \
  --setup "curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && apt-get install -y --no-install-recommends nodejs && corepack enable" \
  --setup "cargo install --locked cargo-xwin" \
  --env CI=true --env CARGO_TERM_COLOR=always --env CARGO_BUILD_JOBS --env COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
  --cache fathom-ci-cargo-registry=/usr/local/cargo/registry \
  --cache fathom-ci-cargo-git=/usr/local/cargo/git \
  --cache fathom-win-target=/src/target \
  --cache fathom-win-xwin=/root/.cache/cargo-xwin \
  --cache fathom-ci-pnpm-store=/pnpm-store \
  --cache fathom-ci-corepack=/root/.cache/node/corepack \
  --export /src/target/x86_64-pc-windows-msvc/release/bundle=dist/windows \
  --step "rustup target add x86_64-pc-windows-msvc" \
  --step "pnpm install --frozen-lockfile --store-dir /pnpm-store" \
  --step "pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis"
