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
RUN cargo build --release -p fathom-server --target x86_64-unknown-linux-musl \
    && cp target/x86_64-unknown-linux-musl/release/fathom-server /fathom-server

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=server /fathom-server /fathom-server
COPY --from=ui /src/ui/dist /ui
ENV FATHOM_UI_DIR=/ui FATHOM_LISTEN=0.0.0.0:8080
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/fathom-server"]
