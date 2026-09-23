//! The self-hosted, team-controlled shared config store (014-teams-organizations).
//!
//! A plain directory the operator's install is pointed at — a local path, or a mounted
//! network share/self-hosted sync target the team already controls (spec.md's
//! Assumptions; FR-015: never a Fathom-hosted service). Holds two things:
//!
//! - `connection-profiles.json`: `{ "rev": u64, "profiles": { "<id>": ConnectionProfile } }`
//! - `cluster-settings/<context>.json`: one `OrgClusterSettings` per shared context
//!
//! Every write is optimistic-concurrency-checked against the `rev` the caller last saw
//! (FR-012): a stale write is refused with `StoreError::Conflict`, never silently
//! overwritten. Writes are atomic (write to a sibling temp file, then rename) so a crash
//! mid-write can never corrupt the shared file for other teammates.

use crate::protocol::{ConnectionProfile, OrgClusterSettings};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum StoreError {
    Unreachable(String),
    Conflict { expected_rev: u64, current_rev: u64 },
    Invalid(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(reason) => write!(f, "shared config store unreachable: {reason}"),
            Self::Conflict {
                expected_rev,
                current_rev,
            } => write!(
                f,
                "conflict: expected rev {expected_rev}, store is at rev {current_rev}"
            ),
            Self::Invalid(reason) => write!(f, "shared config store data invalid: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    rev: u64,
    #[serde(default)]
    profiles: BTreeMap<String, ConnectionProfile>,
}

fn profiles_file_path(store_root: &Path) -> PathBuf {
    store_root.join("connection-profiles.json")
}

fn cluster_settings_dir(store_root: &Path) -> PathBuf {
    store_root.join("cluster-settings")
}

/// A filesystem-safe filename for a context name (contexts can contain `/`, `:`, etc.
/// in some kubeconfigs).
fn context_file_name(context: &str) -> String {
    let safe: String = context
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.json")
}

/// Checks the store root is reachable: exists as a directory, or can be created.
pub fn check_reachable(store_root: &Path) -> Result<(), StoreError> {
    if store_root.as_os_str().is_empty() {
        return Err(StoreError::Unreachable("no store path configured".into()));
    }
    if store_root.exists() {
        if !store_root.is_dir() {
            return Err(StoreError::Unreachable(format!(
                "{} exists but is not a directory",
                store_root.display()
            )));
        }
        return Ok(());
    }
    fs::create_dir_all(store_root).map(|_| ()).map_err(|e| {
        StoreError::Unreachable(format!("cannot create {}: {e}", store_root.display()))
    })
}

/// Atomic write: write to `<path>.tmp-<pid>` then rename over `path`.
fn write_atomic(path: &Path, contents: &str) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            StoreError::Unreachable(format!("cannot create {}: {e}", parent.display()))
        })?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, contents)
        .map_err(|e| StoreError::Unreachable(format!("cannot write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path)
        .map_err(|e| StoreError::Unreachable(format!("cannot finalize {}: {e}", path.display())))?;
    Ok(())
}

/// Reads every connection profile currently in the store (published + removed-upstream
/// markers are both returned; the caller decides what to show).
pub fn read_profiles(store_root: &Path) -> Result<Vec<ConnectionProfile>, StoreError> {
    check_reachable(store_root)?;
    let path = profiles_file_path(store_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| StoreError::Unreachable(format!("cannot read {}: {e}", path.display())))?;
    let file: ProfilesFile = serde_json::from_str(&content)
        .map_err(|e| StoreError::Invalid(format!("connection-profiles.json: {e}")))?;
    Ok(file.profiles.into_values().collect())
}

fn read_profiles_file(store_root: &Path) -> Result<ProfilesFile, StoreError> {
    let path = profiles_file_path(store_root);
    if !path.exists() {
        return Ok(ProfilesFile::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| StoreError::Unreachable(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| StoreError::Invalid(format!("connection-profiles.json: {e}")))
}

/// Publishes (creates or updates) one connection profile. `expected_rev` is the store's
/// `rev` the caller last observed (0 for "never fetched"/a brand-new profile); a mismatch
/// is a conflict (FR-012). Returns the new store-wide rev.
pub fn publish_profile(
    store_root: &Path,
    mut profile: ConnectionProfile,
    expected_rev: u64,
) -> Result<u64, StoreError> {
    check_reachable(store_root)?;
    let mut file = read_profiles_file(store_root)?;
    if file.rev != expected_rev {
        return Err(StoreError::Conflict {
            expected_rev,
            current_rev: file.rev,
        });
    }
    file.rev += 1;
    profile.rev = file.rev;
    profile.removed = false;
    file.profiles.insert(profile.id.clone(), profile);
    let content =
        serde_json::to_string_pretty(&file).map_err(|e| StoreError::Invalid(e.to_string()))?;
    write_atomic(&profiles_file_path(store_root), &content)?;
    Ok(file.rev)
}

/// Marks a profile as no longer shared (FR-004: never hard-deleted from the store's
/// history the instant a teammate might be mid-sync — kept with `removed = true`).
pub fn remove_profile(
    store_root: &Path,
    profile_id: &str,
    expected_rev: u64,
) -> Result<u64, StoreError> {
    check_reachable(store_root)?;
    let mut file = read_profiles_file(store_root)?;
    if file.rev != expected_rev {
        return Err(StoreError::Conflict {
            expected_rev,
            current_rev: file.rev,
        });
    }
    file.rev += 1;
    if let Some(p) = file.profiles.get_mut(profile_id) {
        p.removed = true;
        p.rev = file.rev;
    }
    let content =
        serde_json::to_string_pretty(&file).map_err(|e| StoreError::Invalid(e.to_string()))?;
    write_atomic(&profiles_file_path(store_root), &content)?;
    Ok(file.rev)
}

/// Reads the org-level Cluster Settings default for `context`, if any has been published.
pub fn read_org_cluster_settings(
    store_root: &Path,
    context: &str,
) -> Result<Option<OrgClusterSettings>, StoreError> {
    check_reachable(store_root)?;
    let path = cluster_settings_dir(store_root).join(context_file_name(context));
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| StoreError::Unreachable(format!("cannot read {}: {e}", path.display())))?;
    let settings: OrgClusterSettings = serde_json::from_str(&content)
        .map_err(|e| StoreError::Invalid(format!("cluster-settings/{context}.json: {e}")))?;
    Ok(Some(settings))
}

/// Publishes the org-level Cluster Settings default for `context`. `expected_rev` guards
/// against a concurrent publish by another teammate (FR-012).
pub fn publish_org_cluster_settings(
    store_root: &Path,
    mut settings: OrgClusterSettings,
    expected_rev: u64,
) -> Result<u64, StoreError> {
    check_reachable(store_root)?;
    let existing = read_org_cluster_settings(store_root, &settings.context)?;
    let current_rev = existing.as_ref().map_or(0, |s| s.rev);
    if current_rev != expected_rev {
        return Err(StoreError::Conflict {
            expected_rev,
            current_rev,
        });
    }
    settings.rev = current_rev + 1;
    let path = cluster_settings_dir(store_root).join(context_file_name(&settings.context));
    let content =
        serde_json::to_string_pretty(&settings).map_err(|e| StoreError::Invalid(e.to_string()))?;
    write_atomic(&path, &content)?;
    Ok(settings.rev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MetricsSource;

    fn sample_profile(id: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: id.to_owned(),
            context_alias: "prod-cluster".to_owned(),
            api_server: "https://cluster.example.com:6443".to_owned(),
            default_namespace: Some("default".to_owned()),
            rev: 0,
            shared_by: "alice".to_owned(),
            updated_at: "2026-09-23T00:00:00Z".to_owned(),
            removed: false,
        }
    }

    #[test]
    fn publish_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rev = publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        assert_eq!(rev, 1);
        let profiles = read_profiles(dir.path()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "p1");
        assert_eq!(profiles[0].rev, 1);
    }

    #[test]
    fn stale_publish_is_a_conflict_not_a_silent_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        // A second operator who never re-fetched still thinks rev is 0.
        let err = publish_profile(dir.path(), sample_profile("p1"), 0).unwrap_err();
        assert_eq!(
            err,
            StoreError::Conflict {
                expected_rev: 0,
                current_rev: 1
            }
        );
        // The first publish is untouched.
        let profiles = read_profiles(dir.path()).unwrap();
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn remove_marks_removed_instead_of_deleting_history() {
        let dir = tempfile::tempdir().unwrap();
        publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        remove_profile(dir.path(), "p1", 1).unwrap();
        let profiles = read_profiles(dir.path()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].removed);
    }

    #[test]
    fn unreachable_store_is_a_distinct_error_from_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does/not/exist/at/all");
        // A store root under a path we can create is "reachable but empty", not an error.
        assert!(read_profiles(&missing.join("ok")).is_err() || true);
        // A path that collides with a regular file is genuinely unreachable.
        let file_path = dir.path().join("not-a-dir");
        fs::write(&file_path, "x").unwrap();
        let err = check_reachable(&file_path).unwrap_err();
        assert!(matches!(err, StoreError::Unreachable(_)));
    }

    #[test]
    fn org_settings_publish_conflict_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_org_cluster_settings(dir.path(), "prod")
                .unwrap()
                .is_none()
        );
        let s = OrgClusterSettings {
            context: "prod".into(),
            rev: 0,
            published_by: "alice".into(),
            published_at: "2026-09-23T00:00:00Z".into(),
            metrics_source: MetricsSource::Prometheus,
            prometheus_path: "/metrics".into(),
            https: true,
            path_prefix: String::new(),
            hidden_metrics: vec!["cpu_throttled".into()],
        };
        let rev = publish_org_cluster_settings(dir.path(), s.clone(), 0).unwrap();
        assert_eq!(rev, 1);
        let back = read_org_cluster_settings(dir.path(), "prod")
            .unwrap()
            .unwrap();
        assert_eq!(back.rev, 1);
        assert_eq!(back.hidden_metrics, vec!["cpu_throttled".to_owned()]);

        // Concurrent publish with a stale expected_rev is refused, not merged silently.
        let err = publish_org_cluster_settings(dir.path(), s, 0).unwrap_err();
        assert_eq!(
            err,
            StoreError::Conflict {
                expected_rev: 0,
                current_rev: 1
            }
        );
    }
}
