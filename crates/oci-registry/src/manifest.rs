//! Index, manifest and config parsing, and platform selection (contract rule 6).

/// An image platform, as an index entry or config names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

impl Platform {
    /// From `std::env::consts`: `x86_64` → `amd64`, `aarch64` → `arm64` (variant `v8`).
    pub fn current() -> Platform {
        todo!()
    }
}
