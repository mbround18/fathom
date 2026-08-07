use dagger_sdk::{Container, ContainerWithExecOpts, HostDirectoryOpts, Query, ReturnType};

use crate::targets::{ALL, LINUX_X86_64, Target};

const RUST_IMAGE: &str = "rust:1.97.1-bookworm";
const DIST_DIR: &str = "dist";

/// Native (non-cross) build dependencies for egui/eframe's GTK/X11 backend.
const LINUX_NATIVE_PACKAGES: &[&str] = &[
    "libgtk-3-dev",
    "libxcb-render0-dev",
    "libxcb-shape0-dev",
    "libxcb-xfixes0-dev",
    "libxkbcommon-dev",
    "libssl-dev",
    "pkg-config",
];

/// arm64 counterparts installed alongside the native amd64 toolchain via
/// Debian multiarch, so pkg-config can resolve headers/libs for the cross target.
const LINUX_ARM64_PACKAGES: &[&str] = &[
    "gcc-aarch64-linux-gnu",
    "libgtk-3-dev:arm64",
    "libxcb-render0-dev:arm64",
    "libxcb-shape0-dev:arm64",
    "libxcb-xfixes0-dev:arm64",
    "libxkbcommon-dev:arm64",
    "libssl-dev:arm64",
    "pkg-config",
];

fn source(client: &Query) -> dagger_sdk::Directory {
    client.host().directory_opts(
        ".",
        HostDirectoryOpts {
            exclude: Some(vec!["target", "dist", "graphify-out", ".git"]),
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
/// Cross builds run inside a Linux (amd64) container, so `build.rs`'s
/// `#[cfg(windows)]` icon-embedding step only fires for native Linux builds -
/// the Windows archive ships without the embedded .ico, same as any other
/// Linux-hosted cross build would.
fn toolchain(client: &Query, target: &Target) -> Container {
    let container = cargo_container(client);

    match target.triple {
        "x86_64-unknown-linux-gnu" => apt_install(container, LINUX_NATIVE_PACKAGES),

        "aarch64-unknown-linux-gnu" => {
            let container = container.with_exec(vec!["dpkg", "--add-architecture", "arm64"]);
            let container = apt_install(container, LINUX_ARM64_PACKAGES);
            container
                .with_exec(vec!["rustup", "target", "add", "aarch64-unknown-linux-gnu"])
                .with_env_variable("PKG_CONFIG_ALLOW_CROSS", "1")
                .with_env_variable("PKG_CONFIG_PATH", "/usr/lib/aarch64-linux-gnu/pkgconfig")
                .with_env_variable(
                    "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER",
                    "aarch64-linux-gnu-gcc",
                )
        }

        "x86_64-pc-windows-gnu" => {
            // cmake + nasm are required by aws-lc-sys (pulled in transitively via rustls).
            let container = apt_install(container, &["gcc-mingw-w64-x86-64", "cmake", "nasm"]);
            container
                .with_exec(vec!["rustup", "target", "add", "x86_64-pc-windows-gnu"])
                .with_env_variable(
                    "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER",
                    "x86_64-w64-mingw32-gcc",
                )
        }

        other => unreachable!("no toolchain wired up for target {other}"),
    }
}

fn cargo_out_dir(target: &Target, release: bool) -> String {
    format!(
        "target/{}/{}",
        target.triple,
        if release { "release" } else { "debug" }
    )
}

/// Builds `target`, then copies the produced binary out of the cache-mounted
/// `target/` dir into `/output` - Dagger cannot export a file that still
/// lives inside a cache volume mount, so this step is required before any
/// `.file(..).export(..)` call downstream.
async fn build_target(client: &Query, target: &Target, release: bool) -> eyre::Result<Container> {
    let mut args = vec!["cargo".to_string(), "build".to_string()];
    if release {
        args.push("--release".to_string());
    }
    args.push("--target".to_string());
    args.push(target.triple.to_string());

    let container = exec_checked(&toolchain(client, target), target.label, args).await?;

    let out_dir = cargo_out_dir(target, release);
    let copy = vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "mkdir -p /output && cp {out_dir}/{} /output/{}",
            target.binary_name, target.binary_name
        ),
    ];
    exec_checked(&container, target.label, copy).await
}

/// Packages the staged `/output/<binary>` into a tar.gz/zip and exports the
/// archive to `dist/`.
async fn package_and_export(container: &Container, target: &Target) -> eyre::Result<()> {
    let archive_name = format!("fathom-{}.{}", target.label, target.archive_ext);
    let archive_path = format!("/output/{archive_name}");

    let packaged = if target.archive_ext == "zip" {
        let container = apt_install(container.clone(), &["zip"]);
        exec_checked(
            &container,
            target.label,
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("cd /output && zip {archive_name} {}", target.binary_name),
            ],
        )
        .await?
    } else {
        exec_checked(
            container,
            target.label,
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "cd /output && tar czf {archive_name} {}",
                    target.binary_name
                ),
            ],
        )
        .await?
    };

    packaged
        .file(archive_path)
        .export(format!("{DIST_DIR}/{archive_name}"))
        .await?;

    Ok(())
}

/// Quick multi-target debug build; exports raw binaries (no archives) to
/// `dist/debug/<target-label>/`.
///
/// Targets build sequentially rather than concurrently: they share a single
/// cargo registry cache volume, and concurrent `cargo build` processes race
/// unpacking the same crate into that shared cache (`.cargo-ok` file exists).
pub async fn build(client: &Query) -> eyre::Result<()> {
    for target in ALL {
        let container = build_target(client, target, false).await?;
        let export_path = format!("{DIST_DIR}/debug/{}/{}", target.label, target.binary_name);
        container
            .file(format!("/output/{}", target.binary_name))
            .export(export_path.clone())
            .await?;
        println!("build: {} -> {export_path}", target.label);
    }

    println!("build: debug binaries exported to {DIST_DIR}/debug/");
    Ok(())
}

/// `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test --all`,
/// mirroring the checks in .github/workflows/ci.yml.
pub async fn test(client: &Query) -> eyre::Result<()> {
    let container = toolchain(client, &LINUX_X86_64);
    let container = exec_checked(
        &container,
        "fmt",
        vec!["cargo".into(), "fmt".into(), "--check".into()],
    )
    .await?;
    let container = exec_checked(
        &container,
        "clippy",
        vec![
            "cargo".into(),
            "clippy".into(),
            "--all-targets".into(),
            "--".into(),
            "-D".into(),
            "warnings".into(),
        ],
    )
    .await?;
    exec_checked(
        &container,
        "test",
        vec!["cargo".into(), "test".into(), "--all".into()],
    )
    .await?;

    println!("test: fmt, clippy, and test suite passed");
    Ok(())
}

/// Release builds + packaged archives for every target in `targets::ALL`,
/// exported to `dist/`. Sequential for the same reason as `build()`.
pub async fn release(client: &Query) -> eyre::Result<()> {
    for target in ALL {
        let container = build_target(client, target, true).await?;
        package_and_export(&container, target).await?;
        println!("release: {} packaged", target.label);
    }

    println!("release: archives exported to {DIST_DIR}/");
    Ok(())
}
