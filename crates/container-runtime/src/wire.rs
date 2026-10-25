//! Wire types for local container-runtime management (spec 048), shared by every app's contract.
//!
//! Moved verbatim from Fathom's `protocol::v1::container_runtime` (spec 050 T009): each app
//! re-exports these, so the serde shape and the ts-rs bindings are defined exactly once. Doc
//! comments here are exported into the TypeScript bindings, so editing one is a protocol change.
//!
//! The runtime is reached through its command line, not its socket API, because FR-018 requires
//! every mutation to be previewed "in a form the operator could reproduce themselves". An argv is
//! reproducible; an HTTP call to a unix socket is not. That decision is why [`OperationPreview`]
//! carries a literal `Vec<String>` and not a prose rendering: see `specs/048-docker-management`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// Monotonic stream counter. Exported to TypeScript as `number` (safe below 2^53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct StreamId(#[ts(type = "number")] pub u64);

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stream-{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Container runtime detected on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KindRuntimeKind {
    Docker,
    Podman,
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

/// Whether a local container runtime is usable, in the only four states the feature recognises.
///
/// There is no fifth variant and no catch-all on purpose: every one of these reads differently to
/// an operator, and collapsing any pair of them produces the "no containers" lie FR-003 forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePresenceState {
    /// No runtime binary on `PATH`. The scene is not rendered at all.
    Absent,
    /// Binary present, daemon not running. Say exactly that.
    Stopped,
    /// Binary present, daemon answered with an error (typically a permission denial).
    Unreachable,
    /// Binary present, daemon answered.
    Ready,
}

/// Host container-runtime readiness snapshot (backend-computed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RuntimePresence {
    pub state: RuntimePresenceState,
    /// `Some` whenever `state` is not `Absent`.
    pub runtime: Option<KindRuntimeKind>,
    /// The socket or host the runtime reports talking to. `None` only when `Absent`.
    pub endpoint: Option<String>,
    /// The runtime's own message, verbatim and unwrapped. `Some` when `Unreachable`.
    pub detail: Option<String>,
}

impl RuntimePresence {
    /// The absent case, which is also the fallback when the probe times out.
    pub fn absent() -> Self {
        Self {
            state: RuntimePresenceState::Absent,
            runtime: None,
            endpoint: None,
            detail: None,
        }
    }

    /// Whether the runtime can answer queries at all.
    pub fn is_ready(&self) -> bool {
        self.state == RuntimePresenceState::Ready
    }
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// Lifecycle state of one container on the local runtime.
///
/// Named apart from `pods::ContainerState`, which is a Kubernetes pod's container: the two live in
/// one flat protocol namespace and mean different things.
///
/// `Restarting` is distinct because a restart loop is the single most useful thing to see, and
/// `Unknown` carries the runtime's own string rather than discarding a value we do not recognise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
    Unknown(String),
}

impl RuntimeContainerState {
    /// Maps a runtime's `State` field, preserving anything unrecognised.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "created" => Self::Created,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "removing" => Self::Removing,
            "exited" => Self::Exited,
            "dead" => Self::Dead,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

/// Health as reported by a configured healthcheck.
///
/// Absent health is `None` on the containing struct and is never defaulted to `Healthy`: "no
/// healthcheck configured" and "healthcheck passing" are different facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContainerHealth {
    Starting,
    Healthy,
    Unhealthy,
}

/// One published or exposed port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PublishedPort {
    pub container_port: u16,
    /// `None` when the port is exposed but not published to the host.
    pub host_port: Option<u16>,
    pub host_ip: Option<String>,
    /// `tcp`, `udp`, `sctp` — whatever the runtime said.
    pub protocol: String,
}

/// One container as the inventory sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Container {
    /// Full identity, not the twelve-character short form: the short form is ambiguous and this id
    /// is what gets written into an argv.
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: RuntimeContainerState,
    /// `None` when the container declares no healthcheck. Never defaulted to healthy.
    pub health: Option<ContainerHealth>,
    /// The runtime's own status line ("Up 3 hours", "Exited (137) 2 minutes ago").
    pub status: String,
    pub created_at: String,
    pub command: String,
    pub ports: Vec<PublishedPort>,
    /// Compose project this container declares, if any.
    pub project: Option<String>,
    /// Compose service name within that project, if any.
    pub service: Option<String>,
    /// Cluster this container backs, taken from the cluster label alone.
    pub cluster: Option<String>,
    /// Whether this one container may be mutated.
    ///
    /// Decided per container, not per group. A cluster's node containers carry no compose project,
    /// so they land in the `Unowned` bucket beside unrelated strays: group adoption can neither
    /// speak for them (FR-016 makes a managed cluster's nodes actionable without a grant) nor be
    /// widened to cover them (that would hand the operator mutation rights over every stray on a
    /// shared daemon).
    pub adoption: AdoptionState,
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

/// State of a project's definition file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionFileState {
    /// The project declares a config file and it is readable.
    Present,
    /// The project declares a config file that is no longer there.
    Missing,
    /// The file is there and could not be read or parsed. Carries the reason; the file is left
    /// exactly as it is on disk (Principle VI).
    Unreadable(String),
    /// The project declares no config file at all.
    Undeclared,
}

/// One complete reading of the runtime's objects, as of one moment.
///
/// A named payload rather than five fields on the event variant: inline, those collections made
/// `ContainerInventoryUpdated` several times the size of every other `StreamEvent`, and the enum
/// is sized by its largest variant whichever one is actually in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContainerInventory {
    pub projects: Vec<Project>,
    pub images: Vec<RemovableObject>,
    pub volumes: Vec<RemovableObject>,
    pub networks: Vec<RemovableObject>,
    /// When this reading was taken. Everything derived from it — previews above all — is only
    /// as true as this timestamp.
    pub as_of: String,
}

/// Which bucket a set of containers belongs to.
///
/// `Unowned` is a distinct variant rather than a reserved name so that no real project can collide
/// with the no-project bucket.
// `Ord` so a grouped inventory can be keyed by group and come back in the same order every
// refresh. A list that reshuffles between refreshes makes two snapshots incomparable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum ProjectGroup {
    Project(String),
    Unowned,
}

/// Adoption of a project, as the UI needs to render it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionState {
    /// The operator granted this project explicitly.
    Adopted,
    /// Readable, not actionable.
    NotAdopted,
    /// Backs a cluster Fathom manages. Computed, never stored, and cannot be released.
    ImplicitViaManagedCluster(String),
}

impl AdoptionState {
    /// Whether mutation is permitted for this project.
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Adopted | Self::ImplicitViaManagedCluster(_))
    }
}

/// One group of containers, rendered as a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Project {
    pub group: ProjectGroup,
    /// Total containers in the group.
    pub members: u32,
    /// Of those, how many are running. Never greater than `members`.
    pub running: u32,
    pub adoption: AdoptionState,
    /// Path the project declares for its definition file, if it declares one.
    pub definition_file: Option<String>,
    pub definition_state: DefinitionFileState,
    pub containers: Vec<Container>,
}

// ---------------------------------------------------------------------------
// Removable objects
// ---------------------------------------------------------------------------

/// Class of object the inventory can enumerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ObjectClass {
    Container,
    Image,
    Volume,
    Network,
}

/// What kind of thing holds a reference to an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    RunningContainer,
    StoppedContainer,
    Image,
    Cluster,
}

/// One thing that currently references a removable object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ObjectReference {
    pub kind: ReferenceKind,
    pub id: String,
    pub name: String,
    /// Project the referencing object belongs to, if any.
    pub project: Option<String>,
}

/// An image, volume or network, with the facts needed to answer "is this safe to delete".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RemovableObject {
    pub class: ObjectClass,
    pub id: String,
    pub name: String,
    /// `None` when the runtime does not report a size. Never `0` — "unavailable" and "empty" are
    /// different answers and only one of them makes a prune look free.
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    /// `Some` for images, `None` for every other class.
    pub tagged: Option<bool>,
    pub created_at: Option<String>,
    pub project: Option<String>,
    pub referenced_by: Vec<ObjectReference>,
}

impl RemovableObject {
    /// Nothing holds a reference, so removing it affects nothing else.
    pub fn is_unreferenced(&self) -> bool {
        self.referenced_by.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// A lifecycle verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Kill,
}

/// A project-level verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAction {
    Up,
    Down,
    Restart,
}

impl ProjectAction {
    /// `Down` removes containers and networks, so it takes the destructive guard.
    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Down)
    }
}

/// What a lifecycle operation applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationScope {
    Container { id: String },
    Project { project: String },
}

/// Which objects a prune selection covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PruneFilter {
    /// Images with no tag.
    DanglingImages,
    /// Images no container references.
    UnusedImages,
    /// Volumes no container references.
    UnusedVolumes,
    /// Networks no container is attached to.
    UnusedNetworks,
    /// Containers that have exited.
    StoppedContainers,
}

/// Fields a new container is created from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContainerSpec {
    pub image: String,
    pub name: Option<String>,
    /// `container_port:host_port` pairs, already validated as numbers.
    pub ports: Vec<PublishedPort>,
    /// `KEY=VALUE` pairs, split by the UI so neither side can contain a stray `=`.
    pub env: Vec<(String, String)>,
    /// `source:target` pairs.
    pub volumes: Vec<(String, String)>,
    pub networks: Vec<String>,
    /// Entrypoint arguments as a list, never a shell string.
    pub command: Vec<String>,
    pub restart_policy: Option<String>,
    pub detach: bool,
}

/// Fields an image build is issued from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImageBuildSpec {
    pub context_path: String,
    pub dockerfile: Option<String>,
    pub tag: String,
    pub build_args: Vec<(String, String)>,
    pub target: Option<String>,
    pub no_cache: bool,
}

/// Fields a new network is created from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NetworkSpec {
    pub name: String,
    pub driver: Option<String>,
    pub subnet: Option<String>,
    pub gateway: Option<String>,
    pub internal: bool,
    pub labels: Vec<(String, String)>,
}

/// Fields a new volume is created from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VolumeSpec {
    pub name: String,
    pub driver: Option<String>,
    pub labels: Vec<(String, String)>,
}

/// Every operation this feature can issue.
///
/// Exhaustive on purpose, and deliberately without any variant or field meaning "extra
/// arguments", "raw command" or equivalent. FR-019 says every operation must be describable in
/// advance; a free-text field makes that unkeepable by construction, because the system would have
/// to preview a string whose effect it cannot enumerate. `tests::no_free_text_argument_field`
/// guards the rule against a future edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationRequest {
    Lifecycle {
        action: LifecycleAction,
        scope: OperationScope,
    },
    Remove {
        objects: Vec<ObjectRef>,
    },
    PruneSelection {
        class: ObjectClass,
        filter: PruneFilter,
    },
    CreateContainer {
        spec: ContainerSpec,
    },
    BuildImage {
        spec: ImageBuildSpec,
    },
    CreateNetwork {
        spec: NetworkSpec,
    },
    CreateVolume {
        spec: VolumeSpec,
    },
    ProjectLifecycle {
        project: String,
        action: ProjectAction,
    },
}

impl OperationRequest {
    /// Whether this request destroys something, and so takes the full guard: enumerated preview,
    /// typed confirmation, post-action verification.
    pub fn is_destructive(&self) -> bool {
        match self {
            Self::Remove { .. } | Self::PruneSelection { .. } => true,
            Self::ProjectLifecycle { action, .. } => action.is_destructive(),
            Self::Lifecycle { .. }
            | Self::CreateContainer { .. }
            | Self::BuildImage { .. }
            | Self::CreateNetwork { .. }
            | Self::CreateVolume { .. } => false,
        }
    }
}

/// A pointer to one object by class and identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ObjectRef {
    pub class: ObjectClass,
    pub id: String,
    pub name: String,
}

/// Why an object inside a blast radius was left alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum ExclusionReason {
    /// Belongs to a project the operator has not adopted. FR-025: excluded, not merely flagged.
    UnadoptedProject(String),
}

/// One object an operation will remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RemovalLine {
    pub object: ObjectRef,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    pub project: Option<String>,
}

/// One object inside the blast radius that will not be touched, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ExcludedObject {
    pub object: ObjectRef,
    pub reason: ExclusionReason,
}

/// A cluster an operation will break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ClusterBreak {
    pub cluster: String,
    /// Whether the operator is connected to that cluster right now. Stated separately, because
    /// "this breaks a cluster" and "this breaks the cluster you are looking at" land differently.
    pub connected: bool,
}

/// What an operation will do, before it does it.
///
/// `command` is what runs. No code path may issue an argv that differs from the one this preview
/// displayed; `mutate` re-derives the argv from the stored preview rather than rebuilding it, and
/// the argv invariant test asserts the two are byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OperationPreview {
    pub preview_id: String,
    pub request: OperationRequest,
    /// The literal argv, program first. Not a prose rendering of it.
    pub command: Vec<String>,
    pub destructive: bool,
    /// Every object that disappears. Never summarised as a count.
    pub removals: Vec<RemovalLine>,
    #[ts(type = "number | null")]
    pub total_size_bytes: Option<u64>,
    /// Every project the removal set reaches, selected or not.
    pub blast_radius: Vec<String>,
    pub excluded: Vec<ExcludedObject>,
    pub breaks_clusters: Vec<ClusterBreak>,
    /// The string the operator must type for a destructive operation. `None` otherwise.
    pub required_confirmation: Option<String>,
    /// Inventory timestamp the preview was derived from, used to detect staleness.
    pub as_of: String,
}

/// Outcome for one object an operation touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Succeeded,
    Failed,
    /// The operation was not attempted for this object.
    Skipped,
}

/// Per-object result, because a project operation that reports one aggregate verdict hides which
/// member failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ObjectOutcome {
    pub object: ObjectRef,
    pub status: OutcomeStatus,
    /// The runtime's stderr, verbatim and unwrapped.
    pub error: Option<String>,
}

/// A difference between what the preview said and what the re-read inventory shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Discrepancy {
    pub object: ObjectRef,
    pub expected: String,
    pub observed: String,
}

/// What actually happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OperationOutcome {
    pub preview_id: String,
    /// The argv that ran. Identical to the preview's `command`.
    pub command: Vec<String>,
    /// `Succeeded` only when every per-object entry succeeded.
    pub overall: OutcomeStatus,
    pub objects: Vec<ObjectOutcome>,
    /// Differences found by re-reading the inventory afterwards. Reported, never swallowed.
    pub discrepancies: Vec<Discrepancy>,
}

impl OperationOutcome {
    /// Derives `overall` from the per-object entries, so the two cannot disagree.
    pub fn overall_from(objects: &[ObjectOutcome]) -> OutcomeStatus {
        if objects.iter().any(|o| o.status == OutcomeStatus::Failed) {
            OutcomeStatus::Failed
        } else if objects.iter().all(|o| o.status == OutcomeStatus::Skipped) && !objects.is_empty()
        {
            OutcomeStatus::Skipped
        } else {
            OutcomeStatus::Succeeded
        }
    }
}

/// Why an issue request was refused before anything ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum OperationRefusal {
    /// The project is readable but not actionable. Carries the project name so the UI can offer
    /// adoption rather than leaving the control silently absent.
    NotAdopted(String),
    /// The inventory moved in a way that changes the target set.
    PreviewStale,
    /// The typed confirmation did not match.
    ConfirmationMismatch,
    /// The runtime stopped answering between preview and issue.
    PresenceLost,
    /// The operation could not be described in advance, so it was never offered. Carries the
    /// sentence, because the reason is the whole content: "cannot be previewed" on its own tells
    /// the operator nothing they can act on.
    NotPreviewable(String),
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// One usage tick for one container.
///
/// Numbers, not the runtime's display strings: those are parsed once in the backend so the UI never
/// has to, and an unparseable figure stays `None` rather than becoming a misleading zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct UsageSample {
    pub container_id: String,
    pub at: String,
    pub cpu_percent: Option<f32>,
    #[ts(type = "number | null")]
    pub memory_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub memory_limit_bytes: Option<u64>,
}

/// Containers backing one cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ClusterLink {
    pub cluster: String,
    pub container_ids: Vec<String>,
    /// Whether this is a cluster Fathom provisioned and manages.
    pub managed_by_fathom: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_roundtrip() {
        let p = RuntimePresence {
            state: RuntimePresenceState::Unreachable,
            runtime: Some(KindRuntimeKind::Docker),
            endpoint: Some("unix:///var/run/docker.sock".into()),
            detail: Some("permission denied while trying to connect".into()),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<RuntimePresence>(&json).unwrap(), p);
        assert!(json.contains("\"unreachable\""));
    }

    #[test]
    fn unrecognised_state_is_preserved_not_collapsed() {
        let s = RuntimeContainerState::parse("hibernating");
        assert_eq!(s, RuntimeContainerState::Unknown("hibernating".into()));
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("hibernating"), "{json}");
        assert_eq!(
            serde_json::from_str::<RuntimeContainerState>(&json).unwrap(),
            s
        );
    }

    #[test]
    fn restarting_is_its_own_state() {
        assert_eq!(
            RuntimeContainerState::parse("Restarting"),
            RuntimeContainerState::Restarting
        );
        assert_ne!(
            RuntimeContainerState::parse("restarting"),
            RuntimeContainerState::Running
        );
    }

    #[test]
    fn unowned_group_cannot_collide_with_a_real_project() {
        let unowned = serde_json::to_string(&ProjectGroup::Unowned).unwrap();
        let named = serde_json::to_string(&ProjectGroup::Project("unowned".into())).unwrap();
        assert_ne!(unowned, named);
        assert_eq!(
            serde_json::from_str::<ProjectGroup>(&named).unwrap(),
            ProjectGroup::Project("unowned".into())
        );
    }

    #[test]
    fn implicit_adoption_is_actionable_and_not_adopted_is_not() {
        assert!(AdoptionState::Adopted.is_actionable());
        assert!(AdoptionState::ImplicitViaManagedCluster("fathom-dev".into()).is_actionable());
        assert!(!AdoptionState::NotAdopted.is_actionable());
    }

    #[test]
    fn unavailable_size_stays_none() {
        let o = RemovableObject {
            class: ObjectClass::Volume,
            id: "v1".into(),
            name: "data".into(),
            size_bytes: None,
            tagged: None,
            created_at: None,
            project: None,
            referenced_by: vec![],
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"size_bytes\":null"), "{json}");
        assert_eq!(serde_json::from_str::<RemovableObject>(&json).unwrap(), o);
    }

    #[test]
    fn destructive_classification() {
        assert!(OperationRequest::Remove { objects: vec![] }.is_destructive());
        assert!(
            OperationRequest::PruneSelection {
                class: ObjectClass::Image,
                filter: PruneFilter::DanglingImages,
            }
            .is_destructive()
        );
        assert!(
            OperationRequest::ProjectLifecycle {
                project: "p".into(),
                action: ProjectAction::Down,
            }
            .is_destructive()
        );
        assert!(
            !OperationRequest::ProjectLifecycle {
                project: "p".into(),
                action: ProjectAction::Up,
            }
            .is_destructive()
        );
        assert!(
            !OperationRequest::Lifecycle {
                action: LifecycleAction::Stop,
                scope: OperationScope::Container { id: "c".into() },
            }
            .is_destructive()
        );
    }

    #[test]
    fn overall_is_not_succeeded_when_one_object_failed() {
        let objects = vec![
            ObjectOutcome {
                object: ObjectRef {
                    class: ObjectClass::Container,
                    id: "a".into(),
                    name: "a".into(),
                },
                status: OutcomeStatus::Succeeded,
                error: None,
            },
            ObjectOutcome {
                object: ObjectRef {
                    class: ObjectClass::Container,
                    id: "b".into(),
                    name: "b".into(),
                },
                status: OutcomeStatus::Failed,
                error: Some("No such container: b".into()),
            },
        ];
        assert_eq!(
            OperationOutcome::overall_from(&objects),
            OutcomeStatus::Failed
        );
    }

    #[test]
    fn operation_request_roundtrips_every_variant() {
        let port = PublishedPort {
            container_port: 80,
            host_port: Some(8080),
            host_ip: None,
            protocol: "tcp".into(),
        };
        for r in [
            OperationRequest::Lifecycle {
                action: LifecycleAction::Kill,
                scope: OperationScope::Project {
                    project: "p".into(),
                },
            },
            OperationRequest::Remove {
                objects: vec![ObjectRef {
                    class: ObjectClass::Image,
                    id: "sha256:abc".into(),
                    name: "busybox:latest".into(),
                }],
            },
            OperationRequest::PruneSelection {
                class: ObjectClass::Volume,
                filter: PruneFilter::UnusedVolumes,
            },
            OperationRequest::CreateContainer {
                spec: ContainerSpec {
                    image: "busybox".into(),
                    name: Some("c".into()),
                    ports: vec![port.clone()],
                    env: vec![("A".into(), "1".into())],
                    volumes: vec![],
                    networks: vec![],
                    command: vec!["sleep".into(), "3600".into()],
                    restart_policy: None,
                    detach: true,
                },
            },
            OperationRequest::BuildImage {
                spec: ImageBuildSpec {
                    context_path: ".".into(),
                    dockerfile: None,
                    tag: "t:1".into(),
                    build_args: vec![],
                    target: None,
                    no_cache: false,
                },
            },
            OperationRequest::CreateNetwork {
                spec: NetworkSpec {
                    name: "n".into(),
                    driver: None,
                    subnet: None,
                    gateway: None,
                    internal: false,
                    labels: vec![],
                },
            },
            OperationRequest::CreateVolume {
                spec: VolumeSpec {
                    name: "v".into(),
                    driver: None,
                    labels: vec![],
                },
            },
            OperationRequest::ProjectLifecycle {
                project: "p".into(),
                action: ProjectAction::Down,
            },
        ] {
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(
                serde_json::from_str::<OperationRequest>(&json).unwrap(),
                r,
                "{json}"
            );
        }
    }

    /// FR-019 as a test rather than a promise.
    ///
    /// Every operation must be describable in advance, which is only true while no payload carries
    /// a free-text argument. A field named like one here is almost certainly someone reaching for
    /// the escape hatch the operator ruled out; if a new field legitimately needs one of these
    /// names, the describability requirement needs revisiting first.
    #[test]
    fn no_free_text_argument_field() {
        let banned = [
            "extra_args",
            "extra_arguments",
            "raw_command",
            "raw_args",
            "args_string",
            "shell",
            "cmdline",
            "command_line",
            "free_text",
            "passthrough",
        ];
        let source = include_str!("wire.rs");
        // Only the type definitions matter; the test's own list is allowed to name them.
        let defs = source
            .split("#[cfg(test)]")
            .next()
            .expect("source has a definitions section");
        for name in banned {
            assert!(
                !defs.contains(&format!("{name}:")),
                "`{name}` would make FR-019 unkeepable: an operation whose effect cannot be \
                 enumerated cannot be previewed. See specs/048-docker-management/spec.md FR-019."
            );
        }
    }
}
