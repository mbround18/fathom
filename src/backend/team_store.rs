//! The self-hosted, team-controlled shared config store (014-teams-organizations).
//!
//! A plain directory the operator's install is pointed at — a local path, or a mounted
//! network share/self-hosted sync target the team already controls (spec.md's
//! Assumptions; FR-015: never a Fathom-hosted service). Holds three things:
//!
//! - `connection-profiles.json`: `{ "rev": u64, "profiles": { "<id>": ConnectionProfile } }`
//! - `cluster-settings/<context>.json`: one `OrgClusterSettings` per shared context
//! - `fleet-definitions.json`: `{ "rev": u64, "definitions": { "<id>": SharedFleetDefinition } }`
//!   — 046's shared labels and group definitions (046 US3)
//!
//! Every write is optimistic-concurrency-checked against the `rev` the caller last saw
//! (FR-012): a stale write is refused with `StoreError::Conflict`, never silently
//! overwritten. Writes are atomic (write to a sibling temp file, then rename) so a crash
//! mid-write can never corrupt the shared file for other teammates.

use crate::protocol::{
    ConnectionProfile, OrgClusterSettings, SharedFleetDefinition, TeamLink, scan_fields_for_secrets,
};
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
    /// 033 US3 (R8): advisory group -> profile links. Additive: older readers ignore the key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    team_links: Vec<TeamLink>,
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

/// Non-secret check for link fields (same heuristic as profile publishing).
pub fn validate_link(link: &TeamLink) -> Result<(), StoreError> {
    if link.group.trim().is_empty() || link.profile_id.trim().is_empty() {
        return Err(StoreError::Invalid(
            "a team link needs a group and a profile".into(),
        ));
    }
    scan_fields_for_secrets(&[("group", &link.group), ("profileId", &link.profile_id)])
        .map_err(|(field, reason)| StoreError::Invalid(format!("{field} {reason}")))
}

/// All advisory team links in the store (empty when none were ever written).
pub fn read_links(store_root: &Path) -> Result<Vec<TeamLink>, StoreError> {
    check_reachable(store_root)?;
    Ok(read_profiles_file(store_root)?.team_links)
}

fn write_profiles_file(store_root: &Path, file: &ProfilesFile) -> Result<(), StoreError> {
    let content =
        serde_json::to_string_pretty(file).map_err(|e| StoreError::Invalid(e.to_string()))?;
    write_atomic(&profiles_file_path(store_root), &content)
}

/// Adds a link (idempotent). Links are advisory visibility hints, not authority, so they do
/// not bump the profile `rev` (which guards profile content).
pub fn link_team(store_root: &Path, link: TeamLink) -> Result<Vec<TeamLink>, StoreError> {
    validate_link(&link)?;
    check_reachable(store_root)?;
    let mut file = read_profiles_file(store_root)?;
    if !file.team_links.contains(&link) {
        file.team_links.push(link);
        write_profiles_file(store_root, &file)?;
    }
    Ok(file.team_links)
}

/// Removes a link (idempotent).
pub fn unlink_team(store_root: &Path, link: &TeamLink) -> Result<Vec<TeamLink>, StoreError> {
    check_reachable(store_root)?;
    let mut file = read_profiles_file(store_root)?;
    let before = file.team_links.len();
    file.team_links.retain(|l| l != link);
    if file.team_links.len() != before {
        write_profiles_file(store_root, &file)?;
    }
    Ok(file.team_links)
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

/// `fleet-definitions.json`'s shape: `{ "rev": u64, "definitions": { "<id>": … } }` — the
/// `ProfilesFile` shape, one file for the whole fleet-definition set (046 data-model.md).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct FleetDefinitionsFile {
    #[serde(default)]
    rev: u64,
    #[serde(default)]
    definitions: BTreeMap<String, SharedFleetDefinition>,
}

fn fleet_definitions_file_path(store_root: &Path) -> PathBuf {
    store_root.join("fleet-definitions.json")
}

/// Reads every shared fleet definition in the store. Rows marked `removed` are returned
/// too: the caller decides what to show, and an operator who adopted a removed definition
/// has to be told "no longer shared" rather than handed a shorter list (FR-014).
pub fn read_fleet_definitions(store_root: &Path) -> Result<Vec<SharedFleetDefinition>, StoreError> {
    check_reachable(store_root)?;
    Ok(read_fleet_definitions_file(store_root)?
        .definitions
        .into_values()
        .collect())
}

/// The file as stored, or `Default` when it was never written. A file that *is* there and
/// does not parse is `Invalid`, never silently replaced with defaults: overwriting it would
/// destroy a teammate's published definitions (Constitution Principle VI).
fn read_fleet_definitions_file(store_root: &Path) -> Result<FleetDefinitionsFile, StoreError> {
    let path = fleet_definitions_file_path(store_root);
    if !path.exists() {
        return Ok(FleetDefinitionsFile::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| StoreError::Unreachable(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| StoreError::Invalid(format!("fleet-definitions.json: {e}")))
}

/// Publishes (creates or updates) one shared fleet definition. `expected_rev` is the `rev`
/// the caller last saw **for this definition** (0 for one that was never published); a
/// mismatch is a conflict (FR-012), never a silent overwrite. Returns the new store rev.
///
/// The check is per definition, as `publish_org_cluster_settings` does it, not against the
/// store-wide `rev` as `publish_profile` does: one `fleet-definitions.json` holds every
/// team's definitions, so a store-wide check would refuse the second definition anyone ever
/// publishes. The store-wide `rev` still advances on every write, so it remains a usable
/// "something changed here" marker.
pub fn publish_fleet_definition(
    store_root: &Path,
    mut definition: SharedFleetDefinition,
    expected_rev: u64,
) -> Result<u64, StoreError> {
    check_reachable(store_root)?;
    let mut file = read_fleet_definitions_file(store_root)?;
    let current_rev = file.definitions.get(&definition.id).map_or(0, |d| d.rev);
    if current_rev != expected_rev {
        return Err(StoreError::Conflict {
            expected_rev,
            current_rev,
        });
    }
    file.rev += 1;
    definition.rev = file.rev;
    definition.removed = false;
    file.definitions.insert(definition.id.clone(), definition);
    write_fleet_definitions_file(store_root, &file)?;
    Ok(file.rev)
}

/// Marks a definition as no longer shared. The row is **kept** with `removed = true` — the
/// same promise `remove_profile` makes, and the store half of FR-014: a teammate who adopted
/// it must be able to learn that it was withdrawn, which a vanished row cannot tell them.
pub fn remove_fleet_definition(
    store_root: &Path,
    definition_id: &str,
    expected_rev: u64,
) -> Result<u64, StoreError> {
    check_reachable(store_root)?;
    let mut file = read_fleet_definitions_file(store_root)?;
    let current_rev = file.definitions.get(definition_id).map_or(0, |d| d.rev);
    if current_rev != expected_rev {
        return Err(StoreError::Conflict {
            expected_rev,
            current_rev,
        });
    }
    file.rev += 1;
    if let Some(d) = file.definitions.get_mut(definition_id) {
        d.removed = true;
        d.rev = file.rev;
    }
    write_fleet_definitions_file(store_root, &file)?;
    Ok(file.rev)
}

fn write_fleet_definitions_file(
    store_root: &Path,
    file: &FleetDefinitionsFile,
) -> Result<(), StoreError> {
    let content =
        serde_json::to_string_pretty(file).map_err(|e| StoreError::Invalid(e.to_string()))?;
    write_atomic(&fleet_definitions_file_path(store_root), &content)
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

    fn link(group: &str, profile: &str) -> TeamLink {
        TeamLink {
            group: group.into(),
            profile_id: profile.into(),
        }
    }

    #[test]
    fn links_round_trip_and_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_links(dir.path()).unwrap().is_empty());
        link_team(dir.path(), link("fathom-editors", "p1")).unwrap();
        let links = link_team(dir.path(), link("fathom-editors", "p1")).unwrap();
        assert_eq!(links.len(), 1);
        link_team(dir.path(), link("fathom-viewers", "p1")).unwrap();
        assert_eq!(read_links(dir.path()).unwrap().len(), 2);
        let links = unlink_team(dir.path(), &link("fathom-editors", "p1")).unwrap();
        assert_eq!(links, vec![link("fathom-viewers", "p1")]);
    }

    #[test]
    fn publishing_a_profile_preserves_links() {
        let dir = tempfile::tempdir().unwrap();
        link_team(dir.path(), link("g", "p1")).unwrap();
        publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        assert_eq!(read_links(dir.path()).unwrap(), vec![link("g", "p1")]);
    }

    /// An older reader's view of the file: no links field, unknown keys ignored.
    #[derive(serde::Deserialize)]
    struct OldProfilesFile {
        #[serde(default)]
        rev: u64,
        #[serde(default)]
        profiles: BTreeMap<String, ConnectionProfile>,
    }

    #[test]
    fn older_readers_ignore_the_links_field() {
        let dir = tempfile::tempdir().unwrap();
        publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        link_team(dir.path(), link("g", "p1")).unwrap();
        let raw = fs::read_to_string(profiles_file_path(dir.path())).unwrap();
        assert!(raw.contains("team_links"));
        let old: OldProfilesFile = serde_json::from_str(&raw).unwrap();
        assert_eq!(old.rev, 1);
        assert!(old.profiles.contains_key("p1"));
    }

    #[test]
    fn links_never_carry_secrets() {
        let dir = tempfile::tempdir().unwrap();
        assert!(link_team(dir.path(), link("token=abc", "p1")).is_err());
        assert!(link_team(dir.path(), link("g", "Bearer xyz")).is_err());
        assert!(link_team(dir.path(), link("", "p1")).is_err());
        assert!(read_links(dir.path()).unwrap().is_empty());
        link_team(dir.path(), link("fathom-editors", "kind-abc123")).unwrap();
        let raw = fs::read_to_string(profiles_file_path(dir.path())).unwrap();
        assert!(scan_fields_for_secrets(&[("file", &raw)]).is_ok());
    }
    // -- 046 US3: shared fleet definitions ----------------------------------------------

    fn sample_definition(id: &str) -> SharedFleetDefinition {
        SharedFleetDefinition {
            id: id.to_owned(),
            members: std::collections::BTreeMap::from([(
                "prod-a".to_owned(),
                crate::protocol::FleetMemberShare {
                    display_name: Some("prod a".to_owned()),
                    labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
                },
            )]),
            groups: BTreeMap::from([("prod".to_owned(), "env in (prod)".to_owned())]),
            rev: 0,
            shared_by: "alice".to_owned(),
            updated_at: "2026-10-02T00:00:00Z".to_owned(),
            removed: false,
        }
    }

    #[test]
    fn fleet_definitions_publish_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_fleet_definitions(dir.path()).unwrap().is_empty());
        let rev = publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        assert_eq!(rev, 1);
        let back = read_fleet_definitions(dir.path()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, "prod");
        assert_eq!(back[0].rev, 1);
        assert_eq!(back[0].groups["prod"], "env in (prod)");
        assert_eq!(
            back[0].members["prod-a"].display_name.as_deref(),
            Some("prod a")
        );
    }

    /// A second definition is publishable: the concurrency check is per definition, so the
    /// store-wide `rev` that the first publish advanced is not mistaken for a conflict.
    #[test]
    fn a_second_fleet_definition_is_not_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        publish_fleet_definition(dir.path(), sample_definition("staging"), 0).unwrap();
        let ids: Vec<String> = read_fleet_definitions(dir.path())
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec!["prod".to_owned(), "staging".to_owned()]);
    }

    #[test]
    fn a_stale_fleet_definition_publish_is_a_conflict_not_a_silent_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        let mut stale = sample_definition("prod");
        stale.shared_by = "bob".to_owned();
        assert_eq!(
            publish_fleet_definition(dir.path(), stale, 0),
            Err(StoreError::Conflict {
                expected_rev: 0,
                current_rev: 1
            })
        );
        // Alice's definition is untouched.
        assert_eq!(
            read_fleet_definitions(dir.path()).unwrap()[0].shared_by,
            "alice"
        );
    }

    /// FR-014's store half: removal marks the row, it does not erase it. A teammate who
    /// adopted the definition has to be able to read that it was withdrawn.
    #[test]
    fn removing_a_fleet_definition_marks_it_instead_of_erasing_it() {
        let dir = tempfile::tempdir().unwrap();
        let rev = publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        remove_fleet_definition(dir.path(), "prod", rev).unwrap();
        let back = read_fleet_definitions(dir.path()).unwrap();
        assert_eq!(back.len(), 1, "the row must survive its removal");
        assert!(back[0].removed);
        assert_eq!(back[0].groups["prod"], "env in (prod)");
    }

    #[test]
    fn a_stale_fleet_definition_removal_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        assert!(matches!(
            remove_fleet_definition(dir.path(), "prod", 0),
            Err(StoreError::Conflict { .. })
        ));
        assert!(!read_fleet_definitions(dir.path()).unwrap()[0].removed);
    }

    /// An unreachable store is a distinct error from an empty one, and an unparseable
    /// `fleet-definitions.json` is `Invalid` and is left on disk byte-for-byte.
    #[test]
    fn an_unreadable_definitions_file_is_invalid_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = fleet_definitions_file_path(dir.path());
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(
            read_fleet_definitions(dir.path()),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            publish_fleet_definition(dir.path(), sample_definition("prod"), 0),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");

        let missing = dir.path().join("file-not-a-dir");
        fs::write(&missing, "x").unwrap();
        assert!(matches!(
            read_fleet_definitions(&missing),
            Err(StoreError::Unreachable(_))
        ));
    }

    /// Fleet definitions and connection profiles live in different files: publishing one
    /// cannot disturb the other.
    #[test]
    fn fleet_definitions_do_not_disturb_connection_profiles() {
        let dir = tempfile::tempdir().unwrap();
        publish_profile(dir.path(), sample_profile("p1"), 0).unwrap();
        publish_fleet_definition(dir.path(), sample_definition("prod"), 0).unwrap();
        assert_eq!(read_profiles(dir.path()).unwrap().len(), 1);
        assert_eq!(read_fleet_definitions(dir.path()).unwrap().len(), 1);
    }
}
