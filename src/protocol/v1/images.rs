//! Container image inspection (010-container-image-inspection): on-demand OCI
//! manifest/config metadata for a container's image, fetched by the backend only,
//! never any layer blob content. See specs/010-container-image-inspection/spec.md.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The registry host, repository, and tag/digest a container's image resolves to.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    pub tag: Option<String>,
    /// The digest the manifest fetch actually returned (always present once resolved).
    pub digest: String,
}

/// One layer entry from the resolved image manifest, joined with its build-history command
/// from the image config's `history` field (best-effort positional match).
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ImageLayer {
    pub digest: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub command: Option<String>,
    pub empty_layer: bool,
}

/// Best-effort base-image identification (User Story 3, FR-010). Never a fabricated guess
/// presented as fact — its absence is represented as `None` on `ImageInspection`.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct BaseImageGuess {
    pub label: String,
    /// Always "best-effort" today; kept as a string so future confidence levels don't need a
    /// protocol version bump.
    pub confidence: String,
}

/// The full result of inspecting one container's image: manifest + config metadata only,
/// never layer blob bodies (FR-002, FR-013).
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ImageInspection {
    pub image_ref: ImageRef,
    pub total_size_bytes: u64,
    pub layers: Vec<ImageLayer>,
    pub created: Option<String>,
    pub base_image: Option<BaseImageGuess>,
}

/// Every failure mode the "Image" tab must render as an explicit, distinguishable state
/// (FR-009) rather than a blank or indefinitely-loading panel.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(tag = "kind", content = "detail")]
pub enum ImageInspectError {
    RegistryUnreachable(String),
    ImageNotFound(String),
    AuthenticationFailed(String),
    AuthMethodUnsupported(String),
    Other(String),
}

impl ImageInspectError {
    /// Backend errors MUST reach the operator verbatim (constitution I) — this is the message
    /// shown alongside the error kind's label in the UI.
    pub fn detail(&self) -> &str {
        match self {
            Self::RegistryUnreachable(d)
            | Self::ImageNotFound(d)
            | Self::AuthenticationFailed(d)
            | Self::AuthMethodUnsupported(d)
            | Self::Other(d) => d,
        }
    }
}
