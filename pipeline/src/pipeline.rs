use dagger_sdk::{Container, ContainerWithExecOpts, HostDirectoryOpts, Query, ReturnType};

use crate::targets::{LINUX_X86_64, Target};

const RUST_IMAGE: &str = "rust:1.97.1-bookworm";
/// Node LTS, copied into the Rust image for the `ui/` checks (research R9).
const NODE_IMAGE: &str = "node:24-bookworm";
/// Native (non-cross) build dependencies for the Tauri (GTK) build.
const LINUX_NATIVE_PACKAGES: &[&str] = &[
    "libgtk-3-dev",
    "libxcb-render0-dev",
    "libxcb-shape0-dev",
    "libxcb-xfixes0-dev",
    "libxkbcommon-dev",
    "libssl-dev",
    "pkg-config",
];

/// Tauri v2 (webkit2gtk-4.1) build dependencies for the `fathom-desktop` crate.
const TAURI_LINUX_PACKAGES: &[&str] = &[
    "libwebkit2gtk-4.1-dev",
    "libxdo-dev",
    "libayatana-appindicator3-dev",
    "librsvg2-dev",
];

/// Committed ts-rs output that `tests/export_bindings.rs` regenerates.
const GENERATED_BINDINGS: &str = "ui/src/protocol/generated";

fn source(client: &Query) -> dagger_sdk::Directory {
    client.host().directory_opts(
        ".",
        HostDirectoryOpts {
            exclude: Some(vec![
                "target",
                "dist",
                "graphify-out",
                ".git",
                "ui/node_modules",
                "ui/dist",
                "ui/test-results",
                "ui/playwright-report",
                "ui/src-tauri/target",
            ]),
            gitignore: None,
            include: None,
            no_cache: None,
        },
    )
}

fn apt_install(container: Container, packages: &[&str]) -> Container {
    let mut args: Vec<&str> = vec!["apt-get", "install", "-y", "--no-install-recommends"];
    args.extend_from_slice(packages);
    container
        .with_exec(vec!["apt-get", "update"])
        .with_exec(args)
}

/// Base container: Rust toolchain image, cached cargo registry/target dirs, source mounted.
///
/// The source directory must be mounted at /app *before* the target cache is
/// mounted at /app/target - mounting a parent path after a nested one would
/// otherwise shadow the nested mount.
fn cargo_container(client: &Query) -> Container {
    client
        .container()
        .from(RUST_IMAGE)
        .with_mounted_cache(
            "/usr/local/cargo/registry",
            client.cache_volume("fathom-cargo-registry"),
        )
        .with_workdir("/app")
        .with_mounted_directory("/app", source(client))
        .with_mounted_cache("/app/target", client.cache_volume("fathom-cargo-target"))
}

/// Runs a command and, on nonzero exit, surfaces stdout/stderr in the error
/// instead of letting it disappear into an opaque GraphQL error.
async fn exec_checked(
    container: &Container,
    label: &str,
    args: Vec<String>,
) -> eyre::Result<Container> {
    let cmd = args.join(" ");
    let attempt = container.with_exec_opts(
        args,
        ContainerWithExecOpts {
            expect: Some(ReturnType::Any),
            expand: None,
            experimental_privileged_nesting: None,
            insecure_root_capabilities: None,
            no_init: None,
            redirect_stderr: None,
            redirect_stdin: None,
            redirect_stdout: None,
            stdin: None,
            use_entrypoint: None,
        },
    );

    let exit_code = attempt.exit_code().await?;
    if exit_code != 0 {
        let stdout = attempt.stdout().await.unwrap_or_default();
        let stderr = attempt.stderr().await.unwrap_or_default();
        eyre::bail!(
            "[{label}] `{cmd}` exited with {exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        );
    }

    Ok(attempt)
}

/// A cargo_container() plus whatever's needed to build for `target`.
///
/// Only the `fathom` package is built here (the workspace root's default
/// member); the Tauri desktop bundles need native hosts and are built by
/// .github/workflows/release.yml.
fn toolchain(client: &Query, target: &Target) -> Container {
    let container = cargo_container(client);

    match target.triple {
        "x86_64-unknown-linux-gnu" => apt_install(container, LINUX_NATIVE_PACKAGES),

        other => unreachable!("no toolchain wired up for target {other}"),
    }
}

/// Reads the pnpm version pinned by `packageManager` in `ui/package.json`
/// (e.g. `"pnpm@12.4.2"`), so the pipeline and CI use the same pnpm.
fn pnpm_version() -> eyre::Result<String> {
    let manifest = std::fs::read_to_string("ui/package.json")?;
    let spec = manifest
        .lines()
        .find_map(|line| {
            let (key, value) = line.trim().split_once(':')?;
            (key.trim().trim_matches('"') == "packageManager").then(|| {
                value
                    .trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .to_string()
            })
        })
        .ok_or_else(|| eyre::eyre!("ui/package.json has no packageManager field"))?;
    let version = spec
        .strip_prefix("pnpm@")
        .ok_or_else(|| eyre::eyre!("packageManager is `{spec}`, expected pnpm@<version>"))?;
    // Drop a corepack integrity suffix (`pnpm@x.y.z+sha512...`) if present.
    Ok(version.split('+').next().unwrap_or(version).to_string())
}

/// The native x86_64 toolchain plus everything `test` needs on top of it:
/// the Tauri (webkit2gtk-4.1) dev packages, Node LTS, and the pinned pnpm.
///
/// Node is copied out of the official image rather than apt-installed, since
/// bookworm's packaged Node is far behind LTS. The pnpm store and Playwright
/// browsers live in cache volumes so reruns skip the downloads.
fn test_toolchain(client: &Query) -> eyre::Result<Container> {
    let pnpm = format!("pnpm@{}", pnpm_version()?);
    let node = client.container().from(NODE_IMAGE);

    let container = apt_install(toolchain(client, &LINUX_X86_64), TAURI_LINUX_PACKAGES)
        .with_file("/usr/local/bin/node", node.file("/usr/local/bin/node"))
        .with_directory(
            "/usr/local/lib/node_modules",
            node.directory("/usr/local/lib/node_modules"),
        )
        .with_exec(vec![
            "ln",
            "-sf",
            "/usr/local/lib/node_modules/npm/bin/npm-cli.js",
            "/usr/local/bin/npm",
        ])
        .with_exec(vec!["npm", "install", "--global", pnpm.as_str()])
        .with_env_variable("CI", "true")
        .with_env_variable("npm_config_store_dir", "/pnpm-store")
        .with_mounted_cache("/pnpm-store", client.cache_volume("fathom-pnpm-store"))
        .with_env_variable("PLAYWRIGHT_BROWSERS_PATH", "/ms-playwright")
        .with_mounted_cache("/ms-playwright", client.cache_volume("fathom-playwright"));

    Ok(container)
}

/// `pnpm -C ui <args...>` as an exec argument list.
fn pnpm_ui(args: &[&str]) -> Vec<String> {
    ["pnpm", "-C", "ui"]
        .iter()
        .chain(args)
        .map(|arg| arg.to_string())
        .collect()
}

/// The Principle III gates, mirroring the `check`, `test`, and `ui` jobs in
/// .github/workflows/ci.yml:
///
/// 1. `cargo fmt --check`
/// 2. `ui/`: frozen install, lint, typecheck, unit tests, build, then the
///    Playwright e2e suite in Chromium and WebKit
/// 3. `cargo clippy -D warnings` for `fathom` and `fathom-desktop` (the latter
///    embeds `ui/dist`, so it runs after the UI build)
/// 4. `cargo check --no-default-features --lib` (the backend library)
/// 5. `cargo test --all`
/// 6. Bindings drift: regenerate `ui/src/protocol/generated` and fail if it
///    differs from the committed copy. `.git` isn't mounted, so this diffs
///    against a snapshot taken before regeneration instead of `git diff`.
pub async fn test(client: &Query) -> eyre::Result<()> {
    let mut container = test_toolchain(client)?;

    let steps: Vec<(&str, Vec<String>)> = vec![
        ("fmt", vec!["cargo".into(), "fmt".into(), "--check".into()]),
        ("ui-install", pnpm_ui(&["install", "--frozen-lockfile"])),
        ("ui-lint", pnpm_ui(&["lint"])),
        ("ui-typecheck", pnpm_ui(&["typecheck"])),
        ("ui-test", pnpm_ui(&["test"])),
        ("ui-build", pnpm_ui(&["build"])),
        (
            "ui-playwright-install",
            pnpm_ui(&[
                "exec",
                "playwright",
                "install",
                "--with-deps",
                "chromium",
                "webkit",
            ]),
        ),
        ("ui-e2e", pnpm_ui(&["e2e"])),
        (
            "clippy",
            vec![
                "cargo".into(),
                "clippy".into(),
                "--all-targets".into(),
                "--".into(),
                "-D".into(),
                "warnings".into(),
            ],
        ),
        (
            "clippy-desktop",
            vec![
                "cargo".into(),
                "clippy".into(),
                "-p".into(),
                "fathom-desktop".into(),
                "--".into(),
                "-D".into(),
                "warnings".into(),
            ],
        ),
        (
            "check-no-default-features",
            vec![
                "cargo".into(),
                "check".into(),
                "--no-default-features".into(),
                "--lib".into(),
            ],
        ),
        ("test", vec!["cargo".into(), "test".into(), "--all".into()]),
        (
            "bindings-drift",
            vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "rm -rf /tmp/bindings-committed \
                     && cp -r {GENERATED_BINDINGS} /tmp/bindings-committed \
                     && cargo test -p fathom --test export_bindings \
                     && diff -ru /tmp/bindings-committed {GENERATED_BINDINGS} \
                     || {{ echo 'Generated bindings drifted: run `cargo test -p fathom --test export_bindings` and commit {GENERATED_BINDINGS}'; exit 1; }}"
                ),
            ],
        ),
    ];

    for (label, args) in steps {
        container = exec_checked(&container, label, args).await?;
    }

    println!("test: fmt, ui checks + e2e, clippy, tests, and bindings drift passed");
    Ok(())
}
