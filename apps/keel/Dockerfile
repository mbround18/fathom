# Keel server image (spec 055 T126). Build from the repo root:
#   make keel image    (or: docker build -f apps/keel/Dockerfile -t mbround18/keel:latest .)
# Web assets are copied beside the binary and served from KEEL_STATIC_DIR, never embedded.
# The server has no ansible-core: Ansible runs only in the host runners (constitution).
ARG RUST_VERSION=1.99.0

# Cache mounts keep downloads (crates, the pnpm store) across builds, so a build whose layers are
# invalidated (a lockfile change) only fetches what is new. They are BuildKit build cache:
# `make gc` caps it with the rest.

# ---- Rust: cargo-chef planner -> cook -> build, so dependency layers survive code changes
FROM rust:${RUST_VERSION}-trixie AS chef
RUN --mount=type=cache,id=keel-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=keel-cargo-git,target=/usr/local/cargo/git \
    cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV CARGO_BUILD_JOBS=4
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,id=keel-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=keel-cargo-git,target=/usr/local/cargo/git \
    cargo chef cook --release -p keel-server --recipe-path recipe.json
COPY . .
# The runtime has no shell, so the data directory is made here and owned in its COPY below.
RUN --mount=type=cache,id=keel-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=keel-cargo-git,target=/usr/local/cargo/git \
    cargo build --release -p keel-server --bin keel \
    && mkdir -p /out/var/lib/keel

# ---- Web: one lockfile for the workspace; install only the Keel app and its packages
FROM node:24-trixie-slim AS web
ENV npm_config_store_dir=/pnpm-store COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable
WORKDIR /src
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json ./
RUN --mount=type=cache,id=keel-pnpm-store,target=/pnpm-store \
    --mount=type=cache,id=keel-corepack,target=/root/.cache/node/corepack \
    pnpm fetch
COPY packages/ ./packages/
COPY apps/keel/web/ ./apps/keel/web/
RUN --mount=type=cache,id=keel-pnpm-store,target=/pnpm-store \
    --mount=type=cache,id=keel-corepack,target=/root/.cache/node/corepack \
    pnpm install --offline --frozen-lockfile --filter @companion/keel-web... \
    && pnpm --filter @companion/keel-web build

# ---- Runtime: distroless cc (glibc, libgcc, libssl: webauthn-rs links OpenSSL), non-root.
# No shell and no curl, so there is no HEALTHCHECK here; compose orders start on the database.
FROM gcr.io/distroless/cc-debian13:nonroot AS server
ARG VERSION=0.0.0
ARG REVISION=unknown
ARG SOURCE=https://github.com/mbround18/fathom
LABEL org.opencontainers.image.title="keel" \
      org.opencontainers.image.description="Keel: the Ansible control plane bound to source control (server)" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=builder /src/target/release/keel /usr/local/bin/keel
COPY --from=builder --chown=65532:65532 /out/var/lib/keel /var/lib/keel
COPY --from=web /src/apps/keel/web/dist /usr/share/keel/web
ENV KEEL_LISTEN=0.0.0.0:7830 \
    KEEL_RUNNER_LISTEN=0.0.0.0:7831 \
    KEEL_STATIC_DIR=/usr/share/keel/web \
    KEEL_DATA_DIR=/var/lib/keel
USER 65532:65532
EXPOSE 7830 7831
ENTRYPOINT ["/usr/local/bin/keel"]
