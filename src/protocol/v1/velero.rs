use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Normalized status shared by Velero Backups and Restores, derived from `status.phase`.
/// `PartiallyFailed` and `FailedValidation` collapse into `Failed` (022-velero-backup-pane
/// FR-008 only requires distinguishing completed/failed/in-progress at a glance); an
/// unrecognized or missing phase maps to `Unknown` so a future Velero phase can never crash or
/// be silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum VeleroPhase {
    Completed,
    Failed,
    InProgress,
    Unknown,
}

impl VeleroPhase {
    /// Normalize a raw Velero `status.phase` string (Backup or Restore) into this closed enum.
    /// See specs/022-velero-backup-pane/research.md R2.
    pub fn from_raw(phase: &str) -> Self {
        match phase {
            "Completed" => VeleroPhase::Completed,
            "Failed" | "PartiallyFailed" | "FailedValidation" => VeleroPhase::Failed,
            "New" | "InProgress" | "" => VeleroPhase::InProgress,
            _ => VeleroPhase::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            VeleroPhase::Completed => "Completed",
            VeleroPhase::Failed => "Failed",
            VeleroPhase::InProgress => "In Progress",
            VeleroPhase::Unknown => "Unknown",
        }
    }
}

/// One Velero `Backup` resource.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VeleroBackupSummary {
    pub name: String,
    pub namespace: String,
    pub phase: VeleroPhase,
    /// `metadata.labels["velero.io/schedule-name"]`, when this Backup was produced by a Schedule.
    pub schedule_name: Option<String>,
    /// `status.startTimestamp` (RFC3339), empty if absent.
    pub start_timestamp: String,
    /// `status.completionTimestamp` (RFC3339), empty if absent/in-progress.
    pub completion_timestamp: String,
    pub errors: i64,
    pub warnings: i64,
}

/// One Velero `Schedule` resource, with its last-produced Backup's completion resolved
/// (specs/022-velero-backup-pane/research.md R3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VeleroScheduleSummary {
    pub name: String,
    pub namespace: String,
    /// `spec.schedule` (a cron expression).
    pub cron_schedule: String,
    pub paused: bool,
    /// `None` means "no backups yet" for this schedule.
    pub last_backup_completion: Option<String>,
    pub last_backup_phase: Option<VeleroPhase>,
}

/// One Velero `Restore` resource.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VeleroRestoreSummary {
    pub name: String,
    pub namespace: String,
    pub phase: VeleroPhase,
    /// `spec.backupName`: the Backup this Restore was created from.
    pub backup_name: String,
    pub start_timestamp: String,
    pub completion_timestamp: String,
}

/// One Velero `BackupStorageLocation` resource.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VeleroStorageLocationSummary {
    pub name: String,
    pub available: bool,
    /// Raw `status.phase` (`Available` / `Unavailable` / empty).
    pub phase: String,
    /// `status.message`, empty if absent.
    pub message: String,
    pub is_default: bool,
}

/// Aggregated Velero state for the backup pane: whether Velero is installed at all, whether the
/// current identity is denied read access to its resources, plus every Backup, Schedule,
/// Restore, and BackupStorageLocation found. Read-only; there is no mutation surface
/// (022-velero-backup-pane FR-010).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct VeleroOverview {
    pub installed: bool,
    /// `true` when Velero's CRDs are installed but at least one kind's listing was denied by
    /// RBAC — distinct from `installed: false` (FR-012).
    pub permission_denied: bool,
    pub backups: Vec<VeleroBackupSummary>,
    pub schedules: Vec<VeleroScheduleSummary>,
    pub restores: Vec<VeleroRestoreSummary>,
    pub storage_locations: Vec<VeleroStorageLocationSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_from_raw_maps_completed() {
        assert_eq!(VeleroPhase::from_raw("Completed"), VeleroPhase::Completed);
    }

    #[test]
    fn phase_from_raw_maps_failed_variants() {
        for raw in ["Failed", "PartiallyFailed", "FailedValidation"] {
            assert_eq!(VeleroPhase::from_raw(raw), VeleroPhase::Failed, "raw={raw}");
        }
    }

    #[test]
    fn phase_from_raw_maps_in_progress_variants() {
        for raw in ["New", "InProgress", ""] {
            assert_eq!(
                VeleroPhase::from_raw(raw),
                VeleroPhase::InProgress,
                "raw={raw}"
            );
        }
    }

    #[test]
    fn phase_from_raw_maps_unrecognized_to_unknown() {
        assert_eq!(
            VeleroPhase::from_raw("SomeFuturePhase"),
            VeleroPhase::Unknown
        );
    }
}
