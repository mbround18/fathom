#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Rust target triple passed to `cargo build --target`.
    pub triple: &'static str,
    /// Short label used for archive/dist file names.
    pub label: &'static str,
    /// Name of the produced binary inside `target/<triple>/release/`.
    pub binary_name: &'static str,
    /// Archive format to package the binary into.
    pub archive_ext: &'static str,
}

pub const LINUX_X86_64: Target = Target {
    triple: "x86_64-unknown-linux-gnu",
    label: "linux-x86_64",
    binary_name: "fathom",
    archive_ext: "tar.gz",
};

pub const LINUX_ARM64: Target = Target {
    triple: "aarch64-unknown-linux-gnu",
    label: "linux-arm64",
    binary_name: "fathom",
    archive_ext: "tar.gz",
};

// No Windows target: Tauri's MSI/NSIS bundles need a Windows host, so Windows
// builds live in .github/workflows/release.yml (x86_64-pc-windows-msvc), not here.
pub const ALL: &[Target] = &[LINUX_X86_64, LINUX_ARM64];
