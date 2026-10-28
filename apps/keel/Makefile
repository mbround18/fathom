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

# ---- Server image and the dev compose stack (055 T126, T127, T134) ----------------------------
# The image is the server only; runners are host binaries. Version tags come from git tags later.
IMAGE_REPO   ?= mbround18/keel
IMAGE_TAG    ?= latest
IMAGE        := $(IMAGE_REPO):$(IMAGE_TAG)
KEEL_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(ROOT)/apps/keel/server/Cargo.toml | head -1)
RUST_VERSION := $(shell sed -n 's/^channel = "\(.*\)"/\1/p' $(ROOT)/rust-toolchain.toml)
# The same PostgreSQL image as compose.yaml; compose-secrets borrows its shell for chown.
PG_IMAGE     := postgres:18-trixie
# The uid the image runs as (Dockerfile USER, and the keel_server Ansible role).
KEEL_UID     := 10001
COMPOSE      := docker compose -f $(ROOT)/apps/keel/compose.yaml

.PHONY: image image-push image-push-check compose-secrets compose-config compose-up compose-down

image: ## Build the server image (IMAGE_REPO:IMAGE_TAG, default mbround18/keel:latest); runs gc first
	@$(MAKE) --no-print-directory -C $(ROOT) gc
	@rev=$$(git -C $(ROOT) rev-parse HEAD); \
	if [ -n "$$(git -C $(ROOT) status --porcelain)" ]; then rev="$$rev-dirty"; fi; \
	set -x; docker build -f $(ROOT)/apps/keel/Dockerfile --target server -t $(IMAGE) \
	  --build-arg RUST_VERSION=$(RUST_VERSION) \
	  --build-arg VERSION=$(KEEL_VERSION) \
	  --build-arg REVISION="$$rev" \
	  $(ROOT)

image-push-check:
	@if [ -n "$$(git -C $(ROOT) status --porcelain)" ]; then \
	  echo "keel: the git tree is dirty; commit first, so the image's revision label is true" >&2; exit 1; fi
	@if ! docker info 2>/dev/null | grep -q '^ *Username:' \
	   && ! grep -qs '"https://index.docker.io/v1/"' "$${DOCKER_CONFIG:-$$HOME/.docker}/config.json"; then \
	  echo "keel: no Docker Hub login (docker info shows none); run docker login first" >&2; exit 1; fi

image-push: image-push-check image ## Push the image; refuses on a dirty tree or without a registry login
	docker push $(IMAGE)

compose-secrets: ## TLS cert + key and the master key into KEEL_SECRETS_DIR (from .env); never overwrites
	@[ -f .env ] || { echo "keel: copy .env.example to .env first" >&2; exit 1; }
	@set -a; . ./.env; set +a; \
	dir="$${KEEL_SECRETS_DIR:?set KEEL_SECRETS_DIR in .env}"; name="$${KEEL_TLS_NAME:-keel.boop.ninja}"; \
	mkdir -p "$$dir"; chmod 0711 "$$dir"; dir=$$(cd "$$dir" && pwd); \
	if [ -e "$$dir/tls.key" ]; then echo "keel: $$dir/tls.key exists; left as is"; \
	elif command -v mkcert >/dev/null; then \
	  mkcert -cert-file "$$dir/tls.crt" -key-file "$$dir/tls.key" "$$name"; \
	else \
	  (umask 077; openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 365 \
	    -subj "/CN=$$name" -addext "subjectAltName=DNS:$$name" \
	    -keyout "$$dir/tls.key" -out "$$dir/tls.crt" 2>/dev/null); \
	  echo "keel: wrote a self-signed certificate for $$name. Browsers refuse passkeys behind an"; \
	  echo "      untrusted certificate: trust tls.crt, or install mkcert, delete both and rerun."; \
	fi; \
	chmod 0644 "$$dir/tls.crt"; \
	if [ -e "$$dir/master.key" ]; then echo "keel: $$dir/master.key exists; left as is"; \
	else docker run --rm --network none --user 0:0 -v "$$dir:/secrets" "$${KEEL_IMAGE:-$(IMAGE)}" \
	  keygen --master-key-file /secrets/master.key; fi; \
	need=""; for f in master.key tls.key; do \
	  [ "$$(stat -c %u "$$dir/$$f")" = $(KEEL_UID) ] || need="$$need /secrets/$$f"; done; \
	if [ -n "$$need" ]; then \
	  docker run --rm --network none --user 0:0 -v "$$dir:/secrets" --entrypoint sh $(PG_IMAGE) \
	    -c "chown $(KEEL_UID):$(KEEL_UID) $$need && chmod 0400 $$need"; \
	  echo "keel: master.key and tls.key are $(KEEL_UID)-owned, 0400 (you can still delete them)"; fi

compose-config: ## Check that compose.yaml resolves with .env
	$(COMPOSE) config --quiet

compose-up: ## Start keel and keel-db, detached
	$(COMPOSE) up -d

compose-down: ## Stop and remove the stack's containers; volumes are kept
	$(COMPOSE) down
