#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Rust target triple the test toolchain is built for.
    pub triple: &'static str,
}

// Desktop bundles (Tauri) need native hosts and are built by
// .github/workflows/release.yml, not by this pipeline.
pub const LINUX_X86_64: Target = Target {
    triple: "x86_64-unknown-linux-gnu",
};
