//! Adoption: the gate between reading the host's daemon and changing it.
//!
//! This daemon is shared. Fathom can see every project on it, and that is fine — looking costs
//! nobody anything. Acting is different: a careless operation against a project Fathom was never
//! asked to manage takes work that belongs to someone else. So the whole scene is read-only until
//! the operator adopts a project by name, and adoption is per-project rather than per-daemon
//! because "I manage this one compose stack" is the actual intent.
//!
//! The one exception is computed, not stored: a container that backs a cluster the app provisioned
//! is already the app's to manage, so it is actionable without a separate grant — and cannot be
//! released, because there is no grant to take away.
//!
//! Where the grants live, and which of the app's commands need which grant, are the app's to say:
//! it opens the store at its own path ([`AdoptionStore::open`]) and maps its commands to a
//! [`Requirement`] (Fathom: `backend::container_runtime::adoption::requirement`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::wire::{AdoptionState, Container, OperationRefusal, OperationScope, ProjectGroup};

/// What a command needs before it may run.
///
/// Not a boolean. A boolean would have to be computed by the caller, which means a caller that
/// forgets to compute it passes `true` and the gate becomes decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requirement {
    /// Reads, previews and stream teardown. Nothing on the daemon changes.
    Free,
    /// Needs the named project adopted.
    Project(String),
    /// Needs whatever owns this container to be actionable — its project, or the cluster it backs.
    Container(String),
    /// The command names a preview, not a target. The scope comes from the stored preview and the
    /// gate is applied to that instead.
    FromPreview,
    /// Not a container-runtime command; this gate has nothing to say about it.
    OutOfScope,
}

/// The scope a preview carries, reduced to what the gate needs.
pub fn requirement_for_scope(scope: &OperationScope) -> Requirement {
    match scope {
        OperationScope::Container { id } => Requirement::Container(id.clone()),
        OperationScope::Project { project } => Requirement::Project(project.clone()),
    }
}

/// Resolves a container id to what owns it. The inventory implements this; tests fake it.
pub trait Ownership {
    /// The container's compose project, if it declares one.
    fn project_of(&self, container_id: &str) -> Option<String>;
    /// The Fathom-managed cluster the container backs, if any.
    fn managed_cluster_of(&self, container_id: &str) -> Option<String>;
}

/// A load result: the grants, and whether anything had to be discarded to get them.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub store: AdoptionStore,
    /// True when a record existed and could not be honoured. Surfaced as
    /// `ContainerAdoptionUpdated { fell_back }` so the operator is told, never as "nothing was
    /// ever adopted".
    pub fell_back: bool,
}

/// Why a release did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseRefusal {
    /// No grant to revoke — including the implicit, cluster-derived kind.
    NotAdopted(String),
    /// The grant was dropped in memory and the file could not be written. Carries the reason
    /// verbatim; the caller must not report success.
    NotWritten(String),
}

/// The operator's grants, loaded from disk.
#[derive(Debug, Clone, Default)]
pub struct AdoptionStore {
    /// The file the grants are read from and written to. `None` only for a store that could not
    /// be given one (the app had no config dir): it refuses every write rather than guessing.
    path: Option<PathBuf>,
    projects: BTreeSet<String>,
    /// Clusters Fathom provisioned. Not a grant and never persisted here: recomputed from the
    /// cluster list, because a cluster Fathom no longer manages must stop being actionable on its
    /// own, without the operator having to remember to revoke anything.
    managed_clusters: BTreeSet<String>,
}

impl AdoptionStore {
    /// Loads the grants, falling back to an empty set without touching the file.
    ///
    /// Principle VI: a file we cannot parse is the operator's data, and a rewrite would destroy a
    /// hand-edit we merely failed to understand. An empty set is also the safe failure — it
    /// refuses mutations rather than permitting them.
    ///
    /// The flag comes back with the store rather than being logged and dropped, because the UI has
    /// to say it out loud: an operator whose grants silently vanished would read the refusals that
    /// follow as a bug in the app.
    pub fn open(path: impl Into<PathBuf>) -> Loaded {
        let path = path.into();
        let empty = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        let Ok(value) = load_json_file(&path) else {
            tracing::warn!(
                path = %path.display(),
                "container adoption file could not be read; treating every project as unadopted \
                 and leaving the file exactly as it is"
            );
            return Loaded {
                store: empty,
                fell_back: true,
            };
        };
        // A file that is not there is not a fallback: it is a new install with nothing adopted.
        let missing =
            !value.is_object() || value.as_object().is_some_and(serde_json::Map::is_empty);
        let Some(items) = value.get("projects").and_then(|p| p.as_array()) else {
            return Loaded {
                store: empty,
                // Present but shaped wrong is a fallback; absent is not.
                fell_back: !missing,
            };
        };
        // A non-string entry is the same class of problem as unparseable JSON: part of the record
        // was not understood, so the whole record is refused rather than half-honoured.
        let understood = items.iter().all(|v| v.is_string());
        Loaded {
            store: Self {
                path: Some(path),
                projects: if understood {
                    items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_owned)
                        .collect()
                } else {
                    BTreeSet::new()
                },
                managed_clusters: BTreeSet::new(),
            },
            fell_back: !understood,
        }
    }

    fn persist(&self) -> Result<()> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| anyhow!("the adoption store has no file to write"))?;
        let value = serde_json::json!({
            "projects": self.projects.iter().collect::<Vec<_>>(),
        });
        save_json_file(path, &value)
    }

    /// Tells the store which clusters Fathom manages, for implicit adoption.
    pub fn set_managed_clusters(&mut self, clusters: impl IntoIterator<Item = String>) {
        self.managed_clusters = clusters.into_iter().collect();
    }

    /// Grants a project. Idempotent: adopting what is already adopted is not an error.
    pub fn adopt(&mut self, project: &str) -> Result<()> {
        self.projects.insert(project.to_owned());
        self.persist()
    }

    /// Revokes a grant.
    ///
    /// Refuses when the project holds no grant, which is also how implicit adoption is protected:
    /// a container that backs a Fathom-managed cluster is actionable because of the cluster, not
    /// because of a grant, so there is nothing here to take away. Succeeding silently would tell
    /// the operator they had revoked something they had not.
    pub fn release(&mut self, project: &str) -> Result<(), ReleaseRefusal> {
        if !self.projects.remove(project) {
            return Err(ReleaseRefusal::NotAdopted(project.to_owned()));
        }
        self.persist()
            .map_err(|e| ReleaseRefusal::NotWritten(e.to_string()))
    }

    pub fn adopted(&self) -> Vec<String> {
        self.projects.iter().cloned().collect()
    }

    pub fn is_adopted(&self, project: &str) -> bool {
        self.projects.contains(project)
    }

    /// Adoption state of a project group, for the inventory.
    ///
    /// Group-level state is grants only. A cluster's node containers carry no compose project, so
    /// they land in [`ProjectGroup::Unowned`] alongside unrelated strays — marking that whole
    /// bucket implicitly adopted because one cluster node is in it would hand the operator
    /// mutation rights over containers Fathom knows nothing about. Implicit adoption is therefore
    /// decided per container, in [`Self::state_for_container`].
    pub fn state_for(&self, group: &ProjectGroup) -> AdoptionState {
        match group {
            ProjectGroup::Project(name) if self.is_adopted(name) => AdoptionState::Adopted,
            ProjectGroup::Project(_) | ProjectGroup::Unowned => AdoptionState::NotAdopted,
        }
    }

    /// Adoption state of one container.
    pub fn state_for_container(&self, container: &Container) -> AdoptionState {
        if let Some(cluster) = &container.cluster
            && self.managed_clusters.contains(cluster)
        {
            return AdoptionState::ImplicitViaManagedCluster(cluster.clone());
        }
        match &container.project {
            Some(p) if self.is_adopted(p) => AdoptionState::Adopted,
            _ => AdoptionState::NotAdopted,
        }
    }

    /// The gate. `Ok(())` means the command may proceed.
    pub fn check(
        &self,
        requirement: &Requirement,
        ownership: &dyn Ownership,
    ) -> Result<(), OperationRefusal> {
        match requirement {
            Requirement::Free => Ok(()),
            Requirement::OutOfScope => Ok(()),
            Requirement::Project(project) => {
                if self.is_adopted(project) {
                    Ok(())
                } else {
                    Err(OperationRefusal::NotAdopted(project.clone()))
                }
            }
            Requirement::Container(id) => {
                if let Some(cluster) = ownership.managed_cluster_of(id)
                    && self.managed_clusters.contains(&cluster)
                {
                    return Ok(());
                }
                match ownership.project_of(id) {
                    Some(project) if self.is_adopted(&project) => Ok(()),
                    Some(project) => Err(OperationRefusal::NotAdopted(project)),
                    // A container with no project cannot be adopted, because adoption names a
                    // project. Refusing by container id at least says which container it was,
                    // rather than claiming some project was not adopted.
                    None => Err(OperationRefusal::NotAdopted(id.clone())),
                }
            }
            // The caller resolves the preview's scope and re-enters with that requirement.
            // Returning `Ok` here would let an unresolved preview through.
            Requirement::FromPreview => Err(OperationRefusal::PreviewStale),
        }
    }
}

/// Lets the grouping pass ask the store directly, so no caller has to build a wrapper whose only
/// job is to forward one method — and forget, on the way, which store it was forwarding to.
impl crate::grouping::AdoptionLookup for AdoptionStore {
    fn state_for(&self, group: &ProjectGroup) -> AdoptionState {
        AdoptionStore::state_for(self, group)
    }

    fn state_for_container(&self, container: &Container) -> AdoptionState {
        AdoptionStore::state_for_container(self, container)
    }
}

/// The record as JSON: an empty object when there is no file yet, an error when there is one that
/// does not parse.
fn load_json_file(path: &Path) -> Result<serde_json::Value> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(e.into()),
    }
}

fn save_json_file(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::RuntimeContainerState;

    struct Owned {
        project: Option<&'static str>,
        cluster: Option<&'static str>,
    }

    impl Ownership for Owned {
        fn project_of(&self, _: &str) -> Option<String> {
            self.project.map(str::to_owned)
        }
        fn managed_cluster_of(&self, _: &str) -> Option<String> {
            self.cluster.map(str::to_owned)
        }
    }

    const NOTHING: Owned = Owned {
        project: None,
        cluster: None,
    };

    fn store_with(projects: &[&str]) -> AdoptionStore {
        AdoptionStore {
            path: None,
            projects: projects.iter().map(|p| (*p).to_owned()).collect(),
            managed_clusters: BTreeSet::new(),
        }
    }

    fn container(project: Option<&str>, cluster: Option<&str>) -> Container {
        Container {
            id: "cid".into(),
            name: "c".into(),
            image: "busybox:latest".into(),
            state: RuntimeContainerState::Running,
            health: None,
            status: "Up 1 hour".into(),
            created_at: "2026-10-02 10:00:00 -0700 PDT".into(),
            command: "sleep 3600".into(),
            ports: vec![],
            project: project.map(str::to_owned),
            service: None,
            cluster: cluster.map(str::to_owned),
            adoption: AdoptionState::NotAdopted,
        }
    }

    #[test]
    fn adopting_the_project_opens_its_containers() {
        let store = store_with(&["alpha"]);
        let owner = Owned {
            project: Some("alpha"),
            cluster: None,
        };
        assert!(
            store
                .check(&Requirement::Container("cid".into()), &owner)
                .is_ok()
        );
        assert!(
            store
                .check(&Requirement::Project("alpha".into()), &NOTHING)
                .is_ok()
        );
    }

    #[test]
    fn a_refusal_names_the_project_the_operator_would_have_to_adopt() {
        let store = store_with(&["alpha"]);
        let owner = Owned {
            project: Some("slam-usa"),
            cluster: None,
        };
        assert_eq!(
            store.check(&Requirement::Container("cid".into()), &owner),
            Err(OperationRefusal::NotAdopted("slam-usa".into()))
        );
    }

    /// A preview-named requirement cannot be gated on its own. Defaulting it to permitted would
    /// make the gate worthless for the one command that actually changes something.
    #[test]
    fn issuing_cannot_be_gated_without_resolving_its_preview() {
        let req = Requirement::FromPreview;
        let store = store_with(&["alpha"]);
        assert_eq!(
            store.check(&req, &NOTHING),
            Err(OperationRefusal::PreviewStale),
            "an unresolved preview is refused, not waved through"
        );
        // Resolved against the preview's own scope, it passes.
        let resolved = requirement_for_scope(&OperationScope::Project {
            project: "alpha".into(),
        });
        assert!(store.check(&resolved, &NOTHING).is_ok());
    }

    /// A cluster the app provisioned is already the app's to manage.
    #[test]
    fn a_managed_clusters_containers_are_actionable_without_a_grant() {
        let mut store = AdoptionStore::default();
        store.set_managed_clusters(["fathom-dev".to_owned()]);
        let owner = Owned {
            project: None,
            cluster: Some("fathom-dev"),
        };
        assert!(
            store
                .check(&Requirement::Container("cid".into()), &owner)
                .is_ok()
        );
        assert_eq!(
            store.state_for_container(&container(None, Some("fathom-dev"))),
            AdoptionState::ImplicitViaManagedCluster("fathom-dev".into())
        );
    }

    /// A kind cluster somebody else created on this daemon is not the app's.
    #[test]
    fn an_unmanaged_clusters_containers_stay_read_only() {
        let mut store = AdoptionStore::default();
        store.set_managed_clusters(["fathom-dev".to_owned()]);
        let owner = Owned {
            project: None,
            cluster: Some("someone-elses"),
        };
        assert!(
            store
                .check(&Requirement::Container("cid".into()), &owner)
                .is_err()
        );
        assert_eq!(
            store.state_for_container(&container(None, Some("someone-elses"))),
            AdoptionState::NotAdopted
        );
    }

    /// The `Unowned` bucket is not implicitly adopted just because a cluster node landed in it.
    #[test]
    fn the_unowned_bucket_is_never_implicitly_adopted() {
        let mut store = AdoptionStore::default();
        store.set_managed_clusters(["fathom-dev".to_owned()]);
        assert_eq!(
            store.state_for(&ProjectGroup::Unowned),
            AdoptionState::NotAdopted
        );
    }

    #[test]
    fn group_state_follows_the_grant() {
        let store = store_with(&["alpha"]);
        assert_eq!(
            store.state_for(&ProjectGroup::Project("alpha".into())),
            AdoptionState::Adopted
        );
        assert_eq!(
            store.state_for(&ProjectGroup::Project("beta".into())),
            AdoptionState::NotAdopted
        );
    }

    /// A container with no project cannot be adopted — adoption names a project — so the refusal
    /// names the container instead of inventing a project that does not exist.
    #[test]
    fn an_unowned_container_refuses_by_its_own_id() {
        let store = store_with(&["alpha"]);
        assert_eq!(
            store.check(&Requirement::Container("cid".into()), &NOTHING),
            Err(OperationRefusal::NotAdopted("cid".into()))
        );
    }

    #[test]
    fn adopting_twice_is_not_an_error() {
        let mut store = store_with(&[]);
        store.projects.insert("alpha".into());
        store.projects.insert("alpha".into());
        assert_eq!(store.adopted(), vec!["alpha".to_owned()]);
    }

    // ---------------------------------------------------------------------
    // Persistence. Each test opens the store in its own temp dir, so these never touch a real
    // config dir.
    // ---------------------------------------------------------------------

    /// Runs `f` with a fresh temp dir (which `f` may create the file in) and the store's path in it.
    fn with_scoped_root<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let dir = tempfile::tempdir().expect("temp dir");
        // A subdirectory that does not exist yet, as an app's config dir on first run.
        f(&dir.path().join("config"))
    }

    fn file(root: &std::path::Path) -> PathBuf {
        root.join("container_adoption.json")
    }

    #[test]
    fn a_grant_persists_and_reloads() {
        with_scoped_root(|root| {
            let mut store = AdoptionStore::open(file(root)).store;
            store.adopt("thunderforge-app").expect("write");
            store.adopt("slam-usa").expect("write");

            let reloaded = AdoptionStore::open(file(root));
            assert!(!reloaded.fell_back);
            assert_eq!(
                reloaded.store.adopted(),
                vec!["slam-usa".to_owned(), "thunderforge-app".to_owned()]
            );
            assert!(reloaded.store.is_adopted("slam-usa"));
        });
    }

    /// A fresh install has no file. That is not a fallback, and reporting it as one would make
    /// every first run look like data loss.
    #[test]
    fn no_file_at_all_is_an_empty_set_and_not_a_fallback() {
        with_scoped_root(|root| {
            let loaded = AdoptionStore::open(file(root));
            assert!(loaded.store.adopted().is_empty());
            assert!(!loaded.fell_back);
        });
    }

    /// Principle VI. The operator's file is left byte-for-byte as it was: a record we failed to
    /// understand may still be a hand-edit worth keeping, and rewriting it destroys the evidence.
    #[test]
    fn a_malformed_record_yields_the_empty_set_and_leaves_the_file_alone() {
        with_scoped_root(|root| {
            let path = root.join("container_adoption.json");
            let original = "{ this is not json at all";
            std::fs::create_dir_all(root).expect("root");
            std::fs::write(&path, original).expect("write");

            let loaded = AdoptionStore::open(file(root));
            assert!(
                loaded.store.adopted().is_empty(),
                "the safe failure is nothing adopted, never everything"
            );
            assert!(
                loaded.fell_back,
                "the operator has to be told, or the refusals that follow read as a bug"
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("still there"),
                original,
                "the file was rewritten"
            );
        });
    }

    /// Valid JSON of the wrong shape is the same class of problem, and must not be read as
    /// "no projects are adopted, all is well".
    #[test]
    fn a_record_of_the_wrong_shape_reports_the_fallback() {
        with_scoped_root(|root| {
            std::fs::create_dir_all(root).expect("root");
            std::fs::write(
                root.join("container_adoption.json"),
                r#"{"projects": "thunderforge-app"}"#,
            )
            .expect("write");
            let loaded = AdoptionStore::open(file(root));
            assert!(loaded.store.adopted().is_empty());
            assert!(loaded.fell_back);
        });
    }

    #[test]
    fn one_unreadable_entry_refuses_the_whole_record_rather_than_half_of_it() {
        with_scoped_root(|root| {
            std::fs::create_dir_all(root).expect("root");
            std::fs::write(
                root.join("container_adoption.json"),
                r#"{"projects": ["alpha", 7]}"#,
            )
            .expect("write");
            let loaded = AdoptionStore::open(file(root));
            assert!(
                loaded.store.adopted().is_empty(),
                "honouring half a record the app did not understand is how a grant appears that \
                 the operator never gave"
            );
            assert!(loaded.fell_back);
        });
    }

    /// Granting `alpha` must not make anything in `beta` actionable.
    #[test]
    fn a_grant_does_not_leak_to_another_project() {
        with_scoped_root(|root| {
            let mut store = AdoptionStore::open(file(root)).store;
            store.adopt("alpha").expect("write");
            let beta = Owned {
                project: Some("beta"),
                cluster: None,
            };
            assert_eq!(
                store.check(&Requirement::Container("cid".into()), &beta),
                Err(OperationRefusal::NotAdopted("beta".into()))
            );
            assert!(!store.is_adopted("beta"));
        });
    }

    #[test]
    fn releasing_a_grant_persists_the_revocation() {
        with_scoped_root(|root| {
            let mut store = AdoptionStore::open(file(root)).store;
            store.adopt("alpha").expect("write");
            store.release("alpha").expect("release");
            assert!(AdoptionStore::open(file(root)).store.adopted().is_empty());
        });
    }

    /// T027: implicit adoption cannot be released, because there is no grant behind it. Saying
    /// "released" would tell the operator they had revoked access they still have.
    #[test]
    fn releasing_an_implicit_grant_is_refused() {
        with_scoped_root(|root| {
            let mut store = AdoptionStore::open(file(root)).store;
            store.set_managed_clusters(["fathom-dev".to_owned()]);
            assert_eq!(
                store.release("fathom-dev"),
                Err(ReleaseRefusal::NotAdopted("fathom-dev".to_owned()))
            );
            // And the cluster's containers are still actionable, as they should be.
            let node = Owned {
                project: None,
                cluster: Some("fathom-dev"),
            };
            assert!(
                store
                    .check(&Requirement::Container("cid".into()), &node)
                    .is_ok()
            );
        });
    }

    /// A store with no file refuses to claim a write it could not make.
    #[test]
    fn a_store_without_a_file_refuses_writes() {
        let mut store = AdoptionStore::default();
        assert!(store.adopt("alpha").is_err());
    }

    #[test]
    fn releasing_something_never_adopted_is_refused_not_silently_accepted() {
        with_scoped_root(|root| {
            let mut store = AdoptionStore::open(file(root)).store;
            assert_eq!(
                store.release("never-adopted"),
                Err(ReleaseRefusal::NotAdopted("never-adopted".to_owned()))
            );
        });
    }
}
