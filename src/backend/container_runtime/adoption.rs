//! Adoption: the gate between reading the host's daemon and changing it.
//!
//! This daemon is shared. Fathom can see every project on it, and that is fine — looking costs
//! nobody anything. Acting is different: a careless operation against a project Fathom was never
//! asked to manage takes work that belongs to someone else. So the whole scene is read-only until
//! the operator adopts a project by name, and adoption is per-project rather than per-daemon
//! because "I manage this one compose stack" is the actual intent.
//!
//! The one exception is computed, not stored: a container that backs a cluster Fathom provisioned
//! is already Fathom's to manage, so it is actionable without a separate grant — and cannot be
//! released, because there is no grant to take away.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::protocol::v1::{
    AdoptionState, BackendCmdV1, Container, OperationRefusal, OperationScope, ProjectGroup,
};

use super::super::persistence::{fathom_config_dir, load_json_file, save_json_file};

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

/// What a command needs, by command.
///
/// Every container-runtime command is listed explicitly. The compiler cannot force that — a
/// `BackendCmdV1` match here would have to enumerate the whole protocol — so
/// [`tests::every_container_runtime_command_is_classified`] does, against the same command list
/// the hosted-policy test uses. A new command added without a line here fails that test.
#[allow(clippy::match_same_arms)]
pub fn requirement(cmd: &BackendCmdV1) -> Requirement {
    use BackendCmdV1 as C;
    match cmd {
        // Reading. Available whether or not anything is adopted — that is the point.
        C::RefreshContainerRuntimePresence
        | C::RefreshContainerInventory { .. }
        | C::LoadProjectDefinition { .. }
        | C::StartContainerUsageSampling { .. }
        | C::StopContainerUsageSampling => Requirement::Free,

        // Previewing derives an argv and a removal set and runs nothing, so it is readable for
        // any project. A preview of an unadopted project is how an operator decides whether to
        // adopt it; refusing here would make adoption a blind choice.
        C::PreviewContainerOperation { .. } => Requirement::Free,

        // Granting and revoking the grant itself. Gating these on the grant would be circular.
        C::AdoptContainerProject { .. } | C::ReleaseContainerProject { .. } => Requirement::Free,

        // Logs are a read. Stopping a stream Fathom started is bookkeeping, not a mutation.
        C::StartContainerLogStream { container_id, .. } => {
            Requirement::Container(container_id.clone())
        }
        C::StopContainerLogStream { .. } | C::StopContainerSession { .. } => Requirement::Free,

        // An interactive session inside a container is a mutation path by any other name
        // (FR-013): whatever the operator types in it runs with the container's privileges.
        C::StartContainerSession { container_id, .. } => {
            Requirement::Container(container_id.clone())
        }
        C::SendContainerSessionInput { .. } | C::ResizeContainerSession { .. } => {
            // The gate already ran when the session was opened. Re-deriving it per keystroke
            // would mean a release mid-session kills the shell under the operator's hands with
            // no explanation; closing the session is the honest way to revoke it.
            Requirement::Free
        }

        C::IssueContainerOperation { .. } => Requirement::FromPreview,

        _ => Requirement::OutOfScope,
    }
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

/// The operator's grants, loaded from disk.
#[derive(Debug, Clone, Default)]
pub struct AdoptionStore {
    projects: BTreeSet<String>,
    /// Clusters Fathom provisioned. Not a grant and never persisted here: recomputed from the
    /// cluster list, because a cluster Fathom no longer manages must stop being actionable on its
    /// own, without the operator having to remember to revoke anything.
    managed_clusters: BTreeSet<String>,
}

impl AdoptionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the grants, falling back to an empty set without touching the file.
    ///
    /// Principle VI: a file we cannot parse is the operator's data, and a rewrite would destroy a
    /// hand-edit we merely failed to understand. An empty set is also the safe failure — it
    /// refuses mutations rather than permitting them.
    pub fn load() -> Self {
        let Ok(path) = Self::path() else {
            return Self::default();
        };
        let Ok(value) = load_json_file(&path) else {
            tracing::warn!(
                path = %path.display(),
                "container adoption file could not be read; treating every project as unadopted \
                 and leaving the file exactly as it is"
            );
            return Self::default();
        };
        let projects = value
            .get("projects")
            .and_then(|p| p.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            projects,
            managed_clusters: BTreeSet::new(),
        }
    }

    fn path() -> Result<PathBuf> {
        Ok(fathom_config_dir()?.join("container_adoption.json"))
    }

    fn persist(&self) -> Result<()> {
        let path = Self::path()?;
        let value = serde_json::json!({
            "projects": self.projects.iter().collect::<Vec<_>>(),
        });
        save_json_file(&path, &value)
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

    /// Revokes a grant. Idempotent in the same way.
    pub fn release(&mut self, project: &str) -> Result<()> {
        self.projects.remove(project);
        self.persist()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::{LifecycleAction, OperationRequest, RuntimeContainerState, StreamId};

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
        }
    }

    /// Every command this feature adds, in the same order and with the same shapes as the
    /// hosted-policy test. The gate must have an opinion about each one.
    fn every_container_command() -> Vec<BackendCmdV1> {
        use BackendCmdV1 as C;
        let sid = StreamId(1 << 32);
        vec![
            C::RefreshContainerRuntimePresence,
            C::RefreshContainerInventory { classes: vec![] },
            C::LoadProjectDefinition {
                project: "alpha".into(),
            },
            C::AdoptContainerProject {
                project: "alpha".into(),
            },
            C::ReleaseContainerProject {
                project: "alpha".into(),
            },
            C::PreviewContainerOperation {
                request_id: "r1".into(),
                request: OperationRequest::Lifecycle {
                    action: LifecycleAction::Stop,
                    scope: OperationScope::Container { id: "cid".into() },
                },
            },
            C::IssueContainerOperation {
                request_id: "r1".into(),
                preview_id: "p1".into(),
                typed_confirmation: None,
            },
            C::StartContainerLogStream {
                stream_id: sid,
                container_id: "cid".into(),
                follow: true,
                tail_lines: Some(200),
            },
            C::StopContainerLogStream { stream_id: sid },
            C::StartContainerSession {
                stream_id: sid,
                container_id: "cid".into(),
                shell: None,
            },
            C::StopContainerSession { stream_id: sid },
            C::SendContainerSessionInput {
                stream_id: sid,
                data: "ls\n".into(),
            },
            C::ResizeContainerSession {
                stream_id: sid,
                cols: 120,
                rows: 40,
            },
            C::StartContainerUsageSampling {
                container_ids: vec!["cid".into()],
            },
            C::StopContainerUsageSampling,
        ]
    }

    #[test]
    fn every_container_runtime_command_is_classified() {
        for cmd in every_container_command() {
            assert_ne!(
                requirement(&cmd),
                Requirement::OutOfScope,
                "{cmd:?} has no line in requirement(); a new command must be classified"
            );
        }
    }

    /// The whole premise: nothing is actionable until the operator says so.
    #[test]
    fn an_empty_store_refuses_every_gated_command() {
        let store = AdoptionStore::new();
        let owner = Owned {
            project: Some("alpha"),
            cluster: None,
        };
        let gated: Vec<Requirement> = every_container_command()
            .iter()
            .map(requirement)
            .filter(|r| *r != Requirement::Free)
            .collect();
        assert!(!gated.is_empty(), "something has to be gated");
        for req in gated {
            assert!(
                store.check(&req, &owner).is_err(),
                "{req:?} was permitted with nothing adopted"
            );
        }
    }

    /// Reading is not gated. An operator who cannot see a project cannot decide to adopt it.
    #[test]
    fn reads_and_previews_work_with_nothing_adopted() {
        let store = AdoptionStore::new();
        for cmd in every_container_command() {
            let req = requirement(&cmd);
            if matches!(
                cmd,
                BackendCmdV1::RefreshContainerRuntimePresence
                    | BackendCmdV1::RefreshContainerInventory { .. }
                    | BackendCmdV1::LoadProjectDefinition { .. }
                    | BackendCmdV1::PreviewContainerOperation { .. }
            ) {
                assert_eq!(req, Requirement::Free, "{cmd:?}");
                assert!(store.check(&req, &NOTHING).is_ok());
            }
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

    /// An exec session is a mutation path: whatever is typed into it runs in the container.
    #[test]
    fn an_exec_session_needs_adoption_but_logs_of_the_same_container_do_too() {
        let session = requirement(&BackendCmdV1::StartContainerSession {
            stream_id: StreamId(1 << 32),
            container_id: "cid".into(),
            shell: None,
        });
        assert_eq!(session, Requirement::Container("cid".into()));
        let store = AdoptionStore::new();
        let owner = Owned {
            project: Some("slam-usa"),
            cluster: None,
        };
        assert!(store.check(&session, &owner).is_err());
    }

    /// Issuing takes a preview id, so it cannot be gated on its own. Defaulting it to permitted
    /// would make the gate worthless for the one command that actually changes something.
    #[test]
    fn issuing_cannot_be_gated_without_resolving_its_preview() {
        let req = requirement(&BackendCmdV1::IssueContainerOperation {
            request_id: "r1".into(),
            preview_id: "p1".into(),
            typed_confirmation: None,
        });
        assert_eq!(req, Requirement::FromPreview);
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

    /// A cluster Fathom provisioned is already Fathom's to manage.
    #[test]
    fn a_managed_clusters_containers_are_actionable_without_a_grant() {
        let mut store = AdoptionStore::new();
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

    /// A kind cluster somebody else created on this daemon is not Fathom's.
    #[test]
    fn an_unmanaged_clusters_containers_stay_read_only() {
        let mut store = AdoptionStore::new();
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
        let mut store = AdoptionStore::new();
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
    fn adopt_and_release_are_idempotent() {
        let mut store = store_with(&[]);
        store.projects.insert("alpha".into());
        store.projects.insert("alpha".into());
        assert_eq!(store.adopted(), vec!["alpha".to_owned()]);
        store.projects.remove("alpha");
        store.projects.remove("alpha");
        assert!(store.adopted().is_empty());
    }
}
