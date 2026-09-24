//! Velero backup integration (022-velero-backup-pane).
//!
//! Discovers whichever served API version of the Velero CRDs is installed
//! (`velero.io`: Backup, Schedule, Restore, BackupStorageLocation) and reads them as dynamic
//! objects — the same discovery pattern `flux.rs`/`longhorn.rs` use, so this works across Velero
//! versions without pinning a compiled API type. Read-only: this module has no mutating
//! functions, matching the pane's read-only scope (FR-010).

use crate::protocol::{
    VeleroBackupSummary, VeleroOverview, VeleroPhase, VeleroRestoreSummary, VeleroScheduleSummary,
    VeleroStorageLocationSummary,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, Error as KubeError, api::Api};
use serde_json::Value;

const BACKUP_CRD: &str = "backups.velero.io";
const SCHEDULE_CRD: &str = "schedules.velero.io";
const RESTORE_CRD: &str = "restores.velero.io";
const STORAGE_LOCATION_CRD: &str = "backupstoragelocations.velero.io";

/// Label Velero writes on every Backup it creates from a Schedule.
const SCHEDULE_NAME_LABEL: &str = "velero.io/schedule-name";

struct DiscoveredCrd {
    group: String,
    version: String,
    kind: String,
}

/// Find a served version for `crd_name` among already-listed CRDs. Mirrors `flux.rs::find_served`.
fn find_served(crds: &[CustomResourceDefinition], crd_name: &str) -> Option<DiscoveredCrd> {
    let crd = crds
        .iter()
        .find(|c| c.metadata.name.as_deref() == Some(crd_name))?;
    let group = crd.spec.group.clone();
    let version = crd
        .spec
        .versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())?;
    let kind = crd.spec.names.kind.clone();
    Some(DiscoveredCrd {
        group,
        version,
        kind,
    })
}

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
///
/// Unlike `flux.rs::list_instances`, this distinguishes a permission-denied (403) response from
/// success so the caller can surface `VeleroOverview::permission_denied` instead of a
/// misleadingly-empty list (specs/022-velero-backup-pane/research.md R5, FR-012).
async fn list_instances(client: &Client, crd: &DiscoveredCrd) -> Result<Vec<DynamicObject>, bool> {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    match api.list(&ListParams::default()).await {
        Ok(list) => Ok(list.items),
        Err(KubeError::Api(resp)) if resp.code == 403 => Err(true),
        Err(_) => Err(false),
    }
}

fn get_str(value: Option<&Value>, key: &str) -> String {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

fn get_bool(value: Option<&Value>, key: &str, default: bool) -> bool {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

fn get_i64(value: Option<&Value>, key: &str) -> i64 {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn summarize_backup(obj: &DynamicObject) -> VeleroBackupSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let status = obj.data.get("status");
    let schedule_name = obj
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get(SCHEDULE_NAME_LABEL))
        .cloned();

    VeleroBackupSummary {
        name,
        namespace,
        phase: VeleroPhase::from_raw(&get_str(status, "phase")),
        schedule_name,
        start_timestamp: get_str(status, "startTimestamp"),
        completion_timestamp: get_str(status, "completionTimestamp"),
        errors: get_i64(status, "errors"),
        warnings: get_i64(status, "warnings"),
    }
}

fn summarize_restore(obj: &DynamicObject) -> VeleroRestoreSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let spec = obj.data.get("spec");
    let status = obj.data.get("status");

    VeleroRestoreSummary {
        name,
        namespace,
        phase: VeleroPhase::from_raw(&get_str(status, "phase")),
        backup_name: get_str(spec, "backupName"),
        start_timestamp: get_str(status, "startTimestamp"),
        completion_timestamp: get_str(status, "completionTimestamp"),
    }
}

fn summarize_storage_location(obj: &DynamicObject) -> VeleroStorageLocationSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let spec = obj.data.get("spec");
    let status = obj.data.get("status");
    let phase = get_str(status, "phase");

    VeleroStorageLocationSummary {
        name,
        available: phase == "Available",
        phase,
        message: get_str(status, "message"),
        is_default: get_bool(spec, "default", false),
    }
}

/// Build a [`VeleroScheduleSummary`], resolving its last-backup-completion from whichever
/// already-listed Backup has a matching `velero.io/schedule-name` label and the latest
/// completion timestamp (specs/022-velero-backup-pane/research.md R3). `None` when no Backup
/// references this Schedule ("no backups yet", FR-005).
fn summarize_schedule(
    obj: &DynamicObject,
    backups: &[VeleroBackupSummary],
) -> VeleroScheduleSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let spec = obj.data.get("spec");

    let mut matching: Vec<&VeleroBackupSummary> = backups
        .iter()
        .filter(|b| b.namespace == namespace && b.schedule_name.as_deref() == Some(name.as_str()))
        .collect();
    matching.sort_by(|a, b| {
        let key = |s: &VeleroBackupSummary| {
            if s.completion_timestamp.is_empty() {
                s.start_timestamp.clone()
            } else {
                s.completion_timestamp.clone()
            }
        };
        key(a).cmp(&key(b))
    });
    let latest = matching.last();

    let last_backup_completion = latest.and_then(|b| {
        let ts = if b.completion_timestamp.is_empty() {
            &b.start_timestamp
        } else {
            &b.completion_timestamp
        };
        if ts.is_empty() {
            None
        } else {
            Some(ts.clone())
        }
    });
    let last_backup_phase = latest.map(|b| b.phase);

    VeleroScheduleSummary {
        name,
        namespace,
        cron_schedule: get_str(spec, "schedule"),
        paused: get_bool(spec, "paused", false),
        last_backup_completion,
        last_backup_phase,
    }
}

/// Discover the Velero CRDs installed in this cluster and build a full overview. Returns
/// `installed: false` (with empty lists) when none of the four `velero.io` CRDs are found, so
/// the UI can show an honest "not installed" empty state (FR-003). When the CRDs are found but
/// at least one kind's listing is denied by RBAC, `permission_denied` is set instead of
/// returning a misleading empty list (FR-012).
pub(super) async fn scan_velero(client: &Client) -> anyhow::Result<VeleroOverview> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let backup_crd = find_served(&crds, BACKUP_CRD);
    let schedule_crd = find_served(&crds, SCHEDULE_CRD);
    let restore_crd = find_served(&crds, RESTORE_CRD);
    let storage_location_crd = find_served(&crds, STORAGE_LOCATION_CRD);

    let installed = backup_crd.is_some()
        || schedule_crd.is_some()
        || restore_crd.is_some()
        || storage_location_crd.is_some();

    let mut permission_denied = false;

    let mut backups = Vec::new();
    if let Some(discovered) = &backup_crd {
        match list_instances(client, discovered).await {
            Ok(items) => backups.extend(items.iter().map(summarize_backup)),
            Err(denied) => permission_denied = permission_denied || denied,
        }
    }

    let mut restores = Vec::new();
    if let Some(discovered) = &restore_crd {
        match list_instances(client, discovered).await {
            Ok(items) => restores.extend(items.iter().map(summarize_restore)),
            Err(denied) => permission_denied = permission_denied || denied,
        }
    }

    let mut storage_locations = Vec::new();
    if let Some(discovered) = &storage_location_crd {
        match list_instances(client, discovered).await {
            Ok(items) => storage_locations.extend(items.iter().map(summarize_storage_location)),
            Err(denied) => permission_denied = permission_denied || denied,
        }
    }

    let mut schedules = Vec::new();
    if let Some(discovered) = &schedule_crd {
        match list_instances(client, discovered).await {
            Ok(items) => {
                schedules.extend(items.iter().map(|obj| summarize_schedule(obj, &backups)))
            }
            Err(denied) => permission_denied = permission_denied || denied,
        }
    }

    backups.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    schedules.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    restores.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    storage_locations.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(VeleroOverview {
        installed,
        permission_denied,
        backups,
        schedules,
        restores,
        storage_locations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::core::ObjectMeta;

    fn backup(
        namespace: &str,
        name: &str,
        schedule_name: Option<&str>,
        phase: &str,
        completion: &str,
    ) -> VeleroBackupSummary {
        VeleroBackupSummary {
            name: name.to_owned(),
            namespace: namespace.to_owned(),
            phase: VeleroPhase::from_raw(phase),
            schedule_name: schedule_name.map(|s| s.to_owned()),
            start_timestamp: String::new(),
            completion_timestamp: completion.to_owned(),
            errors: 0,
            warnings: 0,
        }
    }

    fn dynamic_schedule(namespace: &str, name: &str, cron: &str) -> DynamicObject {
        DynamicObject {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            types: None,
            data: serde_json::json!({ "spec": { "schedule": cron } }),
        }
    }

    #[test]
    fn schedule_with_matching_backup_reports_latest_completion() {
        let backups = vec![
            backup(
                "velero",
                "nightly-1",
                Some("nightly"),
                "Completed",
                "2026-01-01T00:00:00Z",
            ),
            backup(
                "velero",
                "nightly-2",
                Some("nightly"),
                "Completed",
                "2026-01-02T00:00:00Z",
            ),
        ];
        let schedule = dynamic_schedule("velero", "nightly", "0 0 * * *");
        let summary = summarize_schedule(&schedule, &backups);
        assert_eq!(
            summary.last_backup_completion.as_deref(),
            Some("2026-01-02T00:00:00Z")
        );
        assert_eq!(summary.last_backup_phase, Some(VeleroPhase::Completed));
    }

    #[test]
    fn schedule_with_no_matching_backup_reports_none() {
        let backups = vec![backup(
            "velero",
            "other-1",
            Some("other-schedule"),
            "Completed",
            "2026-01-01T00:00:00Z",
        )];
        let schedule = dynamic_schedule("velero", "nightly", "0 0 * * *");
        let summary = summarize_schedule(&schedule, &backups);
        assert_eq!(summary.last_backup_completion, None);
        assert_eq!(summary.last_backup_phase, None);
    }

    #[test]
    fn storage_location_health_from_phase() {
        let obj = DynamicObject {
            metadata: ObjectMeta {
                name: Some("default".to_owned()),
                ..Default::default()
            },
            types: None,
            data: serde_json::json!({ "status": { "phase": "Available" } }),
        };
        let summary = summarize_storage_location(&obj);
        assert!(summary.available);
        assert_eq!(summary.phase, "Available");

        let obj = DynamicObject {
            metadata: ObjectMeta {
                name: Some("secondary".to_owned()),
                ..Default::default()
            },
            types: None,
            data: serde_json::json!({ "status": { "phase": "Unavailable", "message": "bucket unreachable" } }),
        };
        let summary = summarize_storage_location(&obj);
        assert!(!summary.available);
        assert_eq!(summary.message, "bucket unreachable");
    }

    /// Not run in CI - requires a real cluster with Velero installed and a working kubeconfig.
    /// `cargo test -p fathom scan_velero_against_live_cluster -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn scan_velero_against_live_cluster() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_default().await.expect("kube client");
        let overview = scan_velero(&client).await.expect("scan_velero");

        println!("installed: {}", overview.installed);
        println!("permission_denied: {}", overview.permission_denied);
        for backup in &overview.backups {
            println!(
                "  backup {}/{} phase={:?}",
                backup.namespace, backup.name, backup.phase
            );
        }
        assert!(overview.installed);
    }
}
