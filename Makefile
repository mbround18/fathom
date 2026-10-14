# Fathom release packaging.
#
# Every target drops its artifacts in ./dist (override with DIST=...) next to a
# regenerated SHA256SUMS, so "what do I ship" has one answer and not three
# directories under target/.
#
#   make build-linux      deb + rpm + AppImage + the bare binary   (verified)
#   make build-windows    NSIS installer (+ MSI when run on Windows)
#   make build-windows-paws  the same NSIS installer, cross-built in a container via paws
#   make build-macos      .app + .dmg                              (macOS host only)
#
# `make` on its own prints the target list. The Tauri config bundles `targets:
# "all"`; these targets pass --bundles explicitly so a Linux run never tries to
# produce an MSI and fail late.
#
# Packages are UNSIGNED. Signing, notarization and the macOS/Windows Tier-1 runs
# are post-MVP (docs/MVP.md); nothing here pretends otherwise.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

DIST    ?= dist
UI      := ui
CONF    := $(UI)/src-tauri/tauri.conf.json
VERSION := $(shell sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' $(CONF) | head -1)
BIN     := fathom-desktop

# Cargo does not size itself the way playwright.config.ts does, and this box is
# shared with other agents (CLAUDE.md). Override with CARGO_BUILD_JOBS=N.
CARGO_BUILD_JOBS ?= 4
export CARGO_BUILD_JOBS

WIN_TRIPLE := x86_64-pc-windows-msvc
HOST_OS    := $(shell uname -s)

.PHONY: help build-linux build-windows build-macos windows-deps dist-clean version

help: ## Print this target list
	@echo "Fathom $(VERSION) — artifacts go to ./$(DIST)"
	@echo
	@grep -hE '^[a-z][a-z-]*:.*## ' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "  DIST=<dir>              stage somewhere other than ./dist"
	@echo "  CARGO_BUILD_JOBS=<n>    cargo parallelism (default $(CARGO_BUILD_JOBS))"

.DEFAULT_GOAL := help

version: ## Print the version the bundles will carry
	@echo "$(VERSION)"

build-linux: ## deb + rpm + AppImage + the bare binary, into ./dist
	@[ "$(HOST_OS)" = Linux ] || { echo "build-linux needs a Linux host (this is $(HOST_OS))." >&2; exit 1; }
	cd $(UI) && pnpm tauri build --bundles deb,rpm,appimage
	@$(MAKE) --no-print-directory stage RELEASE=target/release

build-windows: ## NSIS installer; cross-compiles from Linux via cargo-xwin
ifeq ($(HOST_OS),Linux)
	@command -v cargo-xwin >/dev/null 2>&1 || { \
		echo "cargo-xwin is not installed — it is what lets a Linux host link MSVC objects."; \
		echo "Run: make windows-deps"; exit 1; }
	@rustup target list --installed | grep -qx '$(WIN_TRIPLE)' || { \
		echo "the $(WIN_TRIPLE) target is not installed."; \
		echo "Run: make windows-deps"; exit 1; }
	@# WiX/MSI has no cross-compile path; NSIS does. On Windows both are built.
	cd $(UI) && pnpm tauri build --runner cargo-xwin --target $(WIN_TRIPLE) --bundles nsis
	@$(MAKE) --no-print-directory stage RELEASE=target/$(WIN_TRIPLE)/release
else
	cd $(UI) && pnpm tauri build --bundles nsis,msi
	@$(MAKE) --no-print-directory stage RELEASE=target/release
endif

build-windows-paws: ## NSIS installer cross-built in a container via paws (no host toolchain)
	scripts/build-windows.sh

build-macos: ## .app + .dmg (macOS host only — there is no cross path)
	@[ "$(HOST_OS)" = Darwin ] || { \
		echo "build-macos needs a macOS host (this is $(HOST_OS))."; \
		echo "Apple's toolchain and codesign have no Linux cross path; use a macOS runner."; \
		exit 1; }
	cd $(UI) && pnpm tauri build --bundles app,dmg
	@$(MAKE) --no-print-directory stage RELEASE=target/release

windows-deps: ## Install the Windows cross toolchain (rustup target + cargo-xwin)
	rustup target add $(WIN_TRIPLE)
	cargo install --locked cargo-xwin

dist-clean: ## Empty ./dist
	rm -rf $(DIST)
	@echo "removed $(DIST)/"

# Collect whatever the bundler actually produced. Globs that match nothing are
# skipped rather than failing: a Linux run has no .msi, a Windows run no .deb,
# and the caller already knows which one they asked for.
.PHONY: stage
stage:
	@test -n "$(RELEASE)" || { echo "stage: RELEASE= is required" >&2; exit 1; }
	@mkdir -p $(DIST)
	@shopt -s nullglob; \
	found=0; \
	for f in $(RELEASE)/bundle/deb/*.deb \
	         $(RELEASE)/bundle/rpm/*.rpm \
	         $(RELEASE)/bundle/appimage/*.AppImage \
	         $(RELEASE)/bundle/nsis/*.exe \
	         $(RELEASE)/bundle/msi/*.msi \
	         $(RELEASE)/bundle/dmg/*.dmg \
	         $(RELEASE)/$(BIN) $(RELEASE)/$(BIN).exe; do \
	  [ -e "$$f" ] || continue; \
	  cp -f "$$f" $(DIST)/; found=1; \
	done; \
	for d in $(RELEASE)/bundle/macos/*.app; do cp -rf "$$d" $(DIST)/; found=1; done; \
	if [ "$$found" = 0 ]; then echo "stage: nothing to copy from $(RELEASE)" >&2; exit 1; fi
	@cd $(DIST) && find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%P\n' \
	  | sort | xargs -r sha256sum > SHA256SUMS
	@echo
	@ls -lh $(DIST)
