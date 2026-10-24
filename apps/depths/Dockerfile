# Depths image (spec 050, research R11). Build from the repo root:
#   make depths-image    (or: docker build -f apps/depths/Dockerfile -t depths:dev .)
# Web assets are copied beside the binary and served from STATIC_DIR, never embedded.
ARG RUST_VERSION=1.99.0

# Cache mounts keep downloads (crates, the pnpm store, apt packages) across builds, so even a build
# whose layers are invalidated (a lockfile change) only fetches what is new. They are BuildKit
# build cache: `make gc` caps it with the rest.

# ---- Rust: cargo-chef planner -> cook -> build, so dependency layers survive code changes
FROM rust:${RUST_VERSION}-trixie AS chef
RUN --mount=type=cache,id=depths-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=depths-cargo-git,target=/usr/local/cargo/git \
    cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV CARGO_BUILD_JOBS=4
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,id=depths-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=depths-cargo-git,target=/usr/local/cargo/git \
    cargo chef cook --release -p depths-server --recipe-path recipe.json
# The volume helper (053) is linked statically: glibc with crt-static, so this stage needs no
# musl toolchain. `--target` keeps RUSTFLAGS off build scripts and proc macros, and puts its
# output in target/<triple>/, apart from the server's.
RUN --mount=type=cache,id=depths-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=depths-cargo-git,target=/usr/local/cargo/git \
    RUSTFLAGS="-C target-feature=+crt-static" cargo chef cook --release -p depths-volume-helper \
        --target "$(rustc -vV | sed -n 's/^host: //p')" --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,id=depths-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=depths-cargo-git,target=/usr/local/cargo/git \
    cargo build --release -p depths-server --bin depths
RUN --mount=type=cache,id=depths-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=depths-cargo-git,target=/usr/local/cargo/git \
    host="$(rustc -vV | sed -n 's/^host: //p')" \
    && RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -p depths-volume-helper \
        --bin depths-volume-helper --target "$host" \
    && cp "target/$host/release/depths-volume-helper" /src/depths-volume-helper \
    && if readelf -d /src/depths-volume-helper | grep -q NEEDED; then \
        echo "depths-volume-helper is not static" >&2; exit 1; fi

# ---- Web: one lockfile for the workspace; install only the Depths app and its packages
FROM node:24-trixie-slim AS web
ENV npm_config_store_dir=/pnpm-store COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable
WORKDIR /src
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json ./
RUN --mount=type=cache,id=depths-pnpm-store,target=/pnpm-store \
    --mount=type=cache,id=depths-corepack,target=/root/.cache/node/corepack \
    pnpm fetch
COPY packages/ ./packages/
COPY apps/depths/web/ ./apps/depths/web/
RUN --mount=type=cache,id=depths-pnpm-store,target=/pnpm-store \
    --mount=type=cache,id=depths-corepack,target=/root/.cache/node/corepack \
    pnpm install --offline --frozen-lockfile --filter @companion/depths-web... \
    && pnpm --filter @companion/depths-web build

# ---- Docker CLI and the compose plugin, from the official image
FROM docker:29-cli AS cli

# ---- Runtime
FROM debian:trixie-slim
# The image's docker-clean hook deletes downloaded packages; drop it so the cache mount keeps them.
RUN --mount=type=cache,id=depths-apt-cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,id=depths-apt-lists,target=/var/lib/apt/lists,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tini \
    && useradd --system --uid 10001 --user-group --home-dir /var/lib/depths --create-home depths \
    && chmod 0700 /var/lib/depths
COPY --from=cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=cli /usr/local/libexec/docker/cli-plugins/docker-compose /usr/local/libexec/docker/cli-plugins/docker-compose
COPY --from=builder /src/target/release/depths /usr/local/bin/depths
# Run with `--entrypoint` from this image's id (053 contracts/helper.md).
COPY --from=builder /src/depths-volume-helper /usr/local/bin/depths-volume-helper
COPY --from=web /src/apps/depths/web/dist /usr/share/depths/web
ENV DEPTHS_LISTEN=0.0.0.0:8080 \
    STATIC_DIR=/usr/share/depths/web \
    DEPTHS_DATA_DIR=/var/lib/depths
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/depths"]
