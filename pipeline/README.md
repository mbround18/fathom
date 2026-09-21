# pipeline

Fathom's build/test/release pipeline, powered by the [Dagger Rust SDK](https://docs.rs/dagger-sdk).
Everything runs in containers, so the only host requirement is Docker (or another
Dagger-compatible runner) - no local Rust cross toolchains, GTK/WebKitGTK dev
packages, Node, or pnpm needed.

## Usage

Requires Docker running locally (Dagger provisions its engine automatically).

```bash
cargo run -p pipeline -- build     # quick debug build for both targets -> dist/debug/
cargo run -p pipeline -- test      # the Principle III gates (see below)
cargo run -p pipeline -- release   # release build + packaged archives for both targets -> dist/
cargo run -p pipeline -- ci        # test, then release
```

## `test`

Runs in the native x86_64 image plus the Tauri WebKitGTK 4.1 dev packages
(`libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev`),
Node LTS (copied from `node:24-bookworm`), and the pnpm version pinned by
`packageManager` in `ui/package.json`. In order:

1. `cargo fmt --check`
2. `pnpm -C ui install --frozen-lockfile`, then `lint`, `typecheck`, `test`, `build`,
   `playwright install --with-deps chromium webkit`, and `e2e`
3. `cargo clippy --all-targets -- -D warnings` and `cargo clippy -p fathom-desktop -- -D warnings`
4. `cargo check --no-default-features --lib`
5. `cargo test --all`
6. Bindings drift: `cargo test -p fathom --test export_bindings`, failing if
   `ui/src/protocol/generated` changed (diffed against a pre-run snapshot, since `.git`
   isn't mounted into the container)

## Targets

| Target        | Triple                        | Archive                        |
| ------------- | ------------------------------ | ------------------------------- |
| Linux x86_64  | `x86_64-unknown-linux-gnu`     | `fathom-linux-x86_64.tar.gz`    |
| Linux arm64   | `aarch64-unknown-linux-gnu`    | `fathom-linux-arm64.tar.gz`     |

Both build inside a `rust:1.97.1-bookworm` container:

- **linux-x86_64** builds natively with the GTK/X11 dev packages eframe needs.
- **linux-arm64** cross-compiles using `gcc-aarch64-linux-gnu` plus arm64 copies of
  the same dev packages, installed via Debian multiarch (`dpkg --add-architecture arm64`).
There is no Windows target: the `x86_64-pc-windows-gnu` cross build was dropped
because Tauri's MSI/NSIS bundles need a Windows host. Windows releases are built
natively (`x86_64-pc-windows-msvc`) by `.github/workflows/release.yml`.

## Layout

- `src/targets.rs` - target triple/label/binary/archive definitions.
- `src/pipeline.rs` - container graph: source mounting, toolchain setup per target,
  build, packaging, and export.
- `src/main.rs` - clap CLI wiring.
