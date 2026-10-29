//! Build identity, the update notice and self-update (spec 061, contracts/protocol.md).
//!
//! `UpdateStatus` is for instance owners only. Admins get `UpdateStatusLine`, which says whether
//! an update exists and nothing about the image (FR-015). Users get neither.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use keel_core::uuid::Uuid;

/// The running Keel build, shaped by role: `revision`, `built_at` and `digest` are `None` for
/// users (FR-002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KeelBuild {
    pub version: Option<String>,
    pub revision: Option<String>,
    /// RFC 3339 UTC.
    pub built_at: Option<String>,
    /// `sha256:…`; owners and admins only.
    pub digest: Option<String>,
    /// `0.2.0`, `untagged build abc1234` or `development build`.
    pub display: String,
}

/// Where the last update check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheckState {
    NotChecked,
    Checking,
    Checked,
}

/// What the last successful check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UpdateOutcome {
    UpToDate,
    UpdateAvailable,
    /// The watched tag points at an older build than the running one.
    OlderBuild,
    /// Same version, different digest.
    Rebuilt,
    /// Not enough metadata to compare; `UpdateStatus.cant_tell_reason` says why.
    CantTell,
}

/// The last failed check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UpdateError {
    /// A registry error code (`registry_unreachable`, `unauthorized`, …; contracts/registry.md).
    pub code: String,
    /// RFC 3339 UTC.
    pub at: String,
}

/// The image the watched tag points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AvailableImage {
    /// `sha256:…`.
    pub digest: String,
    pub version: Option<String>,
    pub revision: Option<String>,
    /// RFC 3339 UTC.
    pub built_at: Option<String>,
    pub release_notes: Option<String>,
    /// Why there is no release notes link (R7).
    pub release_notes_note: Option<String>,
}

/// The latest Keel update run that ended, so a failed update still shows after a reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LastUpdateRun {
    #[ts(type = "string")]
    pub run: Uuid,
    /// The run's end state: `succeeded`, `failed`, `refused`, `cancelled`, `denied` or `expired`.
    pub outcome: String,
    /// The digest the run updated to.
    pub digest: String,
    /// RFC 3339 UTC.
    pub ended_at: Option<String>,
}

/// How the update can be applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdatePath {
    /// The FR-016 prerequisites are met: `PreviewKeelUpdate` will queue an update run.
    Run,
    /// Something is missing, so the owner updates by hand (FR-021).
    Manual {
        /// `no_system_workspace`, `no_keel_host`, `profile_not_allowed` or `job_missing_param`.
        missing: String,
        /// `mbround18/keel@sha256:…`.
        digest_ref: String,
        /// Rendered by the server.
        steps: Vec<String>,
    },
}

/// The full update status. Instance owners only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UpdateStatus {
    pub enabled: bool,
    /// The watched reference, never with credentials.
    pub image: String,
    pub state: UpdateCheckState,
    pub outcome: Option<UpdateOutcome>,
    pub cant_tell_reason: Option<String>,
    pub checked_at: Option<String>,
    pub next_check_at: Option<String>,
    /// The earliest time `CheckForUpdates` is accepted again.
    pub manual_after: Option<String>,
    pub last_error: Option<UpdateError>,
    pub running: KeelBuild,
    pub available: Option<AvailableImage>,
    /// `false`: the running digest is unknown, so the builds were compared by revision (R3).
    pub digest_compare: bool,
    pub update_path: UpdatePath,
    /// A pending, queued or running update run.
    #[ts(type = "string | null")]
    pub update_run: Option<Uuid>,
    /// The latest update run that ended.
    pub last_update_run: Option<LastUpdateRun>,
}

/// What an admin who isn't an owner sees (FR-015, SC-006): nothing about the available image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UpdateStatusLine {
    /// `true` only for `UpdateOutcome::UpdateAvailable`.
    pub update_available: bool,
    pub checked_at: Option<String>,
}
