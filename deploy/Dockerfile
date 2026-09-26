# Hosted image (spec 031). Build from repo root: docker build -f deploy/Dockerfile -t fathom-server .
FROM node:22-alpine AS ui
RUN corepack enable
WORKDIR /src/ui
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
ENV VITE_TRANSPORT=hosted
RUN pnpm exec vite build

FROM rust:1-alpine AS server
# g++/cmake: the root build.rs builds libprotobuf via protobuf-src (no system protoc needed)
RUN apk add --no-cache musl-dev g++ cmake perl make linux-headers
WORKDIR /src
COPY . .
ENV CARGO_BUILD_JOBS=2 RUSTFLAGS="-C target-feature=+crt-static"
# explicit --target keeps crt-static off the host (proc-macro/build-script) artifacts
# BuildKit cache mounts keep dependency artifacts across builds (local iteration); the binary
# is copied out in the same step because the mounted target/ is not part of the layer
RUN --mount=type=cache,target=/src/target --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p fathom-server --target x86_64-unknown-linux-musl \
    && cp target/x86_64-unknown-linux-musl/release/fathom-server /fathom-server

# ---- spec 036: in-cluster daemon. Build with: docker build -f deploy/Dockerfile --target fathom-daemon -t fathom-daemon .
# Placed before the 031 final stage so a plain `docker build` still yields the hosted server image.
FROM rust:1-alpine AS daemon-build
RUN apk add --no-cache musl-dev g++ cmake perl make linux-headers
WORKDIR /src
COPY . .
ENV CARGO_BUILD_JOBS=2 RUSTFLAGS="-C target-feature=+crt-static"
# BuildKit cache mounts keep dependency artifacts across builds (local iteration); the binary
# is copied out in the same step because the mounted target/ is not part of the layer
RUN --mount=type=cache,target=/src/target --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p fathom-daemon --target x86_64-unknown-linux-musl \
    && cp target/x86_64-unknown-linux-musl/release/fathom-daemon /fathom-daemon

FROM gcr.io/distroless/static-debian12:nonroot AS fathom-daemon
COPY --from=daemon-build /fathom-daemon /fathom-daemon
# API (TLS, behind the APIService) and plain-HTTP health; the pod runs with a read-only root filesystem
ENV FATHOM_DAEMON_LISTEN=0.0.0.0:8443 FATHOM_DAEMON_HEALTH_LISTEN=0.0.0.0:8081
USER 65532:65532
EXPOSE 8443 8081
ENTRYPOINT ["/fathom-daemon"]

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=server /fathom-server /fathom-server
COPY --from=ui /src/ui/dist /ui
ENV FATHOM_UI_DIR=/ui FATHOM_LISTEN=0.0.0.0:8080
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/fathom-server"]
