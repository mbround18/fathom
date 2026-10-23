# Depths image (spec 050, research R11). Build from the repo root:
#   make depths-image    (or: docker build -f apps/depths/Dockerfile -t depths:dev .)
# Web assets are copied beside the binary and served from STATIC_DIR, never embedded.
ARG RUST_VERSION=1.99.0

# ---- Rust: cargo-chef planner -> cook -> build, so dependency layers survive code changes
FROM rust:${RUST_VERSION}-trixie AS chef
RUN cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV CARGO_BUILD_JOBS=4
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release -p depths-server --recipe-path recipe.json
COPY . .
RUN cargo build --release -p depths-server --bin depths

# ---- Web: one lockfile for the workspace; install only the Depths app and its packages
FROM node:24-trixie-slim AS web
RUN corepack enable
WORKDIR /src
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json ./
RUN pnpm fetch
COPY packages/ ./packages/
COPY apps/depths/web/ ./apps/depths/web/
RUN pnpm install --offline --frozen-lockfile --filter @companion/depths-web... \
    && pnpm --filter @companion/depths-web build

# ---- Docker CLI and the compose plugin, from the official image
FROM docker:29-cli AS cli

# ---- Runtime
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --home-dir /var/lib/depths --create-home depths \
    && chmod 0700 /var/lib/depths
COPY --from=cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=cli /usr/local/libexec/docker/cli-plugins/docker-compose /usr/local/libexec/docker/cli-plugins/docker-compose
COPY --from=builder /src/target/release/depths /usr/local/bin/depths
COPY --from=web /src/apps/depths/web/dist /usr/share/depths/web
ENV DEPTHS_LISTEN=0.0.0.0:8080 \
    STATIC_DIR=/usr/share/depths/web \
    DEPTHS_DATA_DIR=/var/lib/depths
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/depths"]
