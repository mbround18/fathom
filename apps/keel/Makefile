# Keel (spec 055). From the root: make keel <target>. `make keel` lists the targets.

PROJECT        := keel
CRATES         := -p keel-core -p keel-protocol -p keel-git -p keel-exec -p keel-trust \
                  -p web-passkeys -p keel-server -p keel-runner
CARGO_FEATURES := --features keel-exec/fake
WEB_PKG        := @companion/keel-web
BINDINGS_CRATE := keel-protocol
BINDINGS_DIR   := apps/keel/web/src/protocol/generated

# keel-server's database tests skip without KEEL_TEST_DATABASE_URL, so start the throwaway
# PostgreSQL 18 (no Docker) first. A URL already in the environment wins.
TEST_PREP = if [ -z "$${KEEL_TEST_DATABASE_URL:-}" ]; then eval "$$($(ROOT)/scripts/keel-test-db.sh start)"; fi;

include ../../mk/project.mk

.PHONY: db-stop
db-stop: ## Stop the throwaway test database and delete its directory
	$(ROOT)/scripts/keel-test-db.sh stop
