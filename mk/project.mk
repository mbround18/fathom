# Targets every project Makefile shares: the fast, targeted gate for one app instead of the full
# scripts/ci.sh pipeline. A project Makefile sets these, then includes this file:
#
#   PROJECT        name shown in help
#   CRATES         cargo package flags (-p a -p b ...)
#   CARGO_FEATURES extra cargo flags for test/clippy (e.g. --features keel-exec/fake), optional
#   TEST_PREP      shell run before cargo test in the same shell (e.g. start a database), optional
#   WEB_PKG        pnpm package name of the web app, optional
#   BINDINGS_CRATE crate whose `export_bindings` test writes the TS protocol bindings, optional
#   BINDINGS_DIR   where those bindings land, relative to the repository root
#
# Run from the root as `make <project> <target>`, or here with `make <target>`.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)

# The box is shared with other agents (CLAUDE.md); cargo does not size itself.
CARGO_BUILD_JOBS ?= 4
export CARGO_BUILD_JOBS

.DEFAULT_GOAL := help
.PHONY: help build test test-rust test-web lint fmt bindings check e2e

help: ## Print this target list
	@echo "$(PROJECT) targets (from the root: make $(PROJECT) <target>):"
	@grep -hE '^[a-zA-Z0-9_-]+:.*## ' $(MAKEFILE_LIST) | sort -u \
	  | awk 'BEGIN {FS = ":.*## "}; {printf "  %-12s %s\n", $$1, $$2}'

build: ## Build the Rust crates (and the web app, if any)
	cargo build $(CRATES)
ifdef WEB_PKG
	pnpm --filter $(WEB_PKG) build
endif

test: test-rust test-web ## Unit and integration tests: cargo test, then vitest

test-rust: ## cargo test for this project's crates
	$(TEST_PREP) cargo test $(CRATES) $(CARGO_FEATURES)

test-web: ## vitest for the web app
ifdef WEB_PKG
	pnpm --filter $(WEB_PKG) test
else
	@echo "$(PROJECT): no web app"
endif

lint: ## clippy -D warnings, rustfmt --check, then eslint and typecheck
	cargo clippy $(CRATES) $(CARGO_FEATURES) --all-targets -- -D warnings
	cargo fmt --check $(CRATES)
ifdef WEB_PKG
	pnpm --filter $(WEB_PKG) lint
	pnpm --filter $(WEB_PKG) typecheck
endif

fmt: ## rustfmt this project's crates
	cargo fmt $(CRATES)

bindings: ## Regenerate the TS protocol bindings and fail if they differ from Git
ifdef BINDINGS_CRATE
	cargo test -p $(BINDINGS_CRATE) --test export_bindings
	@cd $(ROOT) && if [ -n "$$(git status --porcelain -- $(BINDINGS_DIR))" ]; then \
	  git status --short -- $(BINDINGS_DIR); \
	  echo "$(PROJECT): $(BINDINGS_DIR) is stale; commit the regenerated files." >&2; exit 1; \
	fi
else
	@echo "$(PROJECT): no generated bindings"
endif

check: lint test bindings ## The project's whole fast gate: lint, test, bindings

e2e: ## Playwright suite for this project only (sizes its own workers)
ifdef WEB_PKG
	pnpm --filter $(WEB_PKG) e2e
else
	@echo "$(PROJECT): no e2e suite"
endif
