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
# `build` typechecks tests/unit too, and the template test imports the vectors shared with keel_core.
COPY specs/060-keel-enroll-settings/contracts/template-vectors.json ./specs/060-keel-enroll-settings/contracts/
# The in-app help (spec 063 R6) bundles the Keel guides, and tests/unit checks their anchors.
COPY docs/guides/keel/ ./docs/guides/keel/
RUN --mount=type=cache,id=keel-pnpm-store,target=/pnpm-store \
    --mount=type=cache,id=keel-corepack,target=/root/.cache/node/corepack \
    pnpm install --offline --frozen-lockfile --filter @companion/keel-web... \
    && pnpm --filter @companion/keel-web build

# ---- Runtime: distroless cc (glibc, libgcc, libssl: webauthn-rs links OpenSSL), non-root as
# uid/gid 10001 (the keel_server Ansible role chowns the master key and TLS key to it).
# No shell and no curl, so there is no HEALTHCHECK here; compose orders start on the database.
FROM gcr.io/distroless/cc-debian13:nonroot AS server
# Build metadata (spec 061 R2). Empty means unknown: an untagged build has no VERSION, and a plain
# `docker build` (or compose's build) has no REVISION or CREATED. Keel refuses to start on a
# malformed value, so the defaults stay empty rather than placeholders. These live in this stage
# only, so changing them never invalidates the cargo-chef or build layers.
ARG VERSION=
ARG REVISION=
ARG CREATED=
ARG SOURCE=https://github.com/mbround18/fathom
LABEL org.opencontainers.image.title="keel" \
      org.opencontainers.image.description="Keel: the Ansible control plane bound to source control (server)" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=builder /src/target/release/keel /usr/local/bin/keel
COPY --from=builder --chown=10001:10001 /out/var/lib/keel /var/lib/keel
COPY --from=web /src/apps/keel/web/dist /usr/share/keel/web
ENV KEEL_LISTEN=0.0.0.0:7830 \
    KEEL_RUNNER_LISTEN=0.0.0.0:7831 \
    KEEL_STATIC_DIR=/usr/share/keel/web \
    KEEL_DATA_DIR=/var/lib/keel \
    KEEL_BUILD_VERSION=${VERSION} \
    KEEL_BUILD_REVISION=${REVISION} \
    KEEL_BUILD_DATE=${CREATED} \
    KEEL_BUILD_SOURCE=${SOURCE}
USER 10001:10001
EXPOSE 7830 7831
ENTRYPOINT ["/usr/local/bin/keel"]
