# pipeline

Fathom's build/test/release pipeline, powered by the [Dagger Rust SDK](https://docs.rs/dagger-sdk).
Everything runs in containers, so the only host requirement is Docker (or another
Dagger-compatible runner) - no local Rust cross toolchains, mingw, or GTK dev
packages needed.

## Usage

Requires Docker running locally (Dagger provisions its engine automatically).

```bash
cargo run -p pipeline -- build     # quick debug build for all 3 targets -> dist/debug/
cargo run -p pipeline -- test      # cargo fmt --check, clippy -D warnings, cargo test --all
cargo run -p pipeline -- release   # release build + packaged archives for all 3 targets -> dist/
cargo run -p pipeline -- ci        # test, then release
```

## Targets

| Target        | Triple                        | Archive                        |
| ------------- | ------------------------------ | ------------------------------- |
| Linux x86_64  | `x86_64-unknown-linux-gnu`     | `fathom-linux-x86_64.tar.gz`    |
| Linux arm64   | `aarch64-unknown-linux-gnu`    | `fathom-linux-arm64.tar.gz`     |
| Windows x86_64| `x86_64-pc-windows-gnu`        | `fathom-windows-x86_64.zip`     |

All three build inside a `rust:1.85-bookworm` container:

- **linux-x86_64** builds natively with the GTK/X11 dev packages eframe needs.
- **linux-arm64** cross-compiles using `gcc-aarch64-linux-gnu` plus arm64 copies of
  the same dev packages, installed via Debian multiarch (`dpkg --add-architecture arm64`).
- **windows-x86_64** cross-compiles with `gcc-mingw-w64-x86-64` (the `-gnu` target,
  not `-msvc`, since MSVC's toolchain isn't available outside Windows).

**Known limitation:** `build.rs`'s `#[cfg(windows)]` icon-embedding step is
evaluated for the *host* running cargo, not the target - since all builds happen
inside a Linux container, the Windows archive ships without the embedded `.ico`.
Building on a native Windows host (as `.github/workflows/release.yml` already does)
is unaffected.

## Layout

- `src/targets.rs` - target triple/label/binary/archive definitions.
- `src/pipeline.rs` - container graph: source mounting, toolchain setup per target,
  build, packaging, and export.
- `src/main.rs` - clap CLI wiring.
