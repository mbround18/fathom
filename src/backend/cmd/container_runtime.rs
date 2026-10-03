//! Local container-runtime commands (spec 048).
//!
//! Every arm here passes through the adoption gate before it does anything, and it does so in one
//! place: [`adoption::requirement`] classifies the command, [`AdoptionStore::check`] answers it,
//! and a refusal is emitted to the operator rather than logged. Doing it per-arm would mean a new
//! mutating command could be added with the gate simply forgotten, which is the one failure mode
//! this feature cannot afford on a daemon shared with unrelated work.
//!
//! Hosted mode is not handled here: `protocol::v1::mutation::hosted_policy` already denies every
//! command in this family with `HostedDenial::ContainerRuntime`, and `server/src/filter.rs`
//! enforces that before a command ever reaches a backend.

use crossbeam_channel::Sender;
use std::sync::atomic::AtomicU64;

use crate::backend::container_runtime::adoption::{
    self, AdoptionStore, Ownership, ReleaseRefusal, Requirement,
};
use crate::backend::container_runtime::{Cli, grouping, inventory, probe};
use crate::backend::state::BackendState;
use crate::backend::{events, kind, persistence};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Container, ContainerInventory, Correlation,
    DefinitionFileState, OperationRefusal, Route, RuntimePresenceState, StreamEvent, section_keys,
};

/// Handles container-runtime commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> Option<BackendCmd> {
    // Classify first, refuse first. `requirement` returns `OutOfScope` for every command this
    // family does not own, so a foreign command passes through untouched.
    let requirement = adoption::requirement(&cmd);
    if !matches!(requirement, Requirement::OutOfScope) {
        let ownership = InventoryOwnership::from(&state.containers);
        let store = state
            .containers
            .adoption(state.cluster.seq.as_ref(), sender);
        if let Err(refusal) = store.check(&requirement, &ownership) {
            refuse(state, sender, &cmd, refusal);
            return None;
        }
    }

    match cmd {
        BackendCmd::RefreshContainerRuntimePresence => refresh_presence(state, sender).await,
        BackendCmd::RefreshContainerInventory { classes: _ } => {
            // `classes` narrows which panes are being looked at; the skeleton reads all four and
            // T035 makes the narrowing count. Reading more than was asked is wasteful, not wrong.
            refresh_inventory(state, sender).await;
        }
        BackendCmd::LoadProjectDefinition { project: _ } => {
            // T037 reads and parses the project's definition file through `compose.rs`. Until
            // then the inventory's `definition_state` is the only answer, and it is already sent.
        }
        BackendCmd::AdoptContainerProject { project } => adopt(state, sender, project),
        BackendCmd::ReleaseContainerProject { project } => release(state, sender, project),
        BackendCmd::PreviewContainerOperation { .. } => {
            // T040-T042: derive the argv and the removal set. Deliberately silent until then —
            // emitting an empty preview would let the UI show an operation with no blast radius.
        }
        BackendCmd::IssueContainerOperation { .. } => {
            // T043-T045: re-derive from the stored preview and run it. The gate above has already
            // refused this with `PreviewStale`, which is the correct answer while no preview can
            // be stored: nothing runs.
        }
        BackendCmd::StartContainerLogStream { .. } | BackendCmd::StopContainerLogStream { .. } => {
            // T056-T058: the log follow, over `container_runtime::stream`.
        }
        BackendCmd::StartContainerSession { .. }
        | BackendCmd::StopContainerSession { .. }
        | BackendCmd::SendContainerSessionInput { .. }
        | BackendCmd::ResizeContainerSession { .. } => {
            // T059-T062: the interactive session, over the PTY machinery 002 built.
        }
        BackendCmd::StartContainerUsageSampling { .. } | BackendCmd::StopContainerUsageSampling => {
            // T065-T068: the bounded usage sampler.
        }
        other => return Some(other),
    }
    None
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// What the backend remembers about the local runtime between commands.
///
/// Nothing in here is read at startup. Principle V: a scene nobody has opened must not cost a
/// probe, a disk read or a daemon round trip, so the presence probe, the adoption file and the
/// inventory are all pulled on the first command that needs them.
#[derive(Default)]
pub(crate) struct RuntimeState {
    /// `None` until the first presence probe. Distinct from `Absent`, which is a probe result.
    presence: Option<RuntimePresenceState>,
    /// Which binary answered. `None` whenever the runtime is not `Ready`.
    cli: Option<Cli>,
    /// `None` until the first command that needs a grant, so an operator who never opens the
    /// scene never has their adoption file read.
    store: Option<AdoptionStore>,
    /// The containers of the last inventory, kept for one reason: a gate decision needs to know
    /// which project a container id belongs to, and asking the daemon again mid-gate would let
    /// the answer change between the check and the act.
    containers: Vec<Container>,
}

impl RuntimeState {
    /// The grants, loading them on first use and telling the operator when a record was discarded.
    fn adoption(
        &mut self,
        seq: &AtomicU64,
        sender: &Sender<BackendEventEnvelope>,
    ) -> &mut AdoptionStore {
        if self.store.is_none() {
            let loaded = AdoptionStore::load();
            // Emitted on load, not only on change: `fell_back` is the operator's only signal that
            // the refusals they are about to see come from a record Fathom could not read.
            emit(
                seq,
                sender,
                StreamEvent::ContainerAdoptionUpdated {
                    adopted: loaded.store.adopted(),
                    fell_back: loaded.fell_back,
                },
            );
            self.store = Some(loaded.store);
        }
        self.store.as_mut().expect("just populated")
    }
}

/// Answers the gate's ownership questions from the last inventory.
struct InventoryOwnership {
    containers: Vec<Container>,
}

impl From<&RuntimeState> for InventoryOwnership {
    fn from(state: &RuntimeState) -> Self {
        Self {
            containers: state.containers.clone(),
        }
    }
}

impl Ownership for InventoryOwnership {
    fn project_of(&self, container_id: &str) -> Option<String> {
        self.containers
            .iter()
            .find(|c| c.id == container_id)
            .and_then(|c| c.project.clone())
    }

    fn managed_cluster_of(&self, container_id: &str) -> Option<String> {
        self.containers
            .iter()
            .find(|c| c.id == container_id)
            .and_then(|c| c.cluster.clone())
    }
}

// ---------------------------------------------------------------------------
// Presence and inventory
// ---------------------------------------------------------------------------

async fn refresh_presence(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    let presence = probe::detect_presence().await;
    state.containers.presence = Some(presence.state);
    state.containers.cli = match (presence.state, presence.runtime) {
        (RuntimePresenceState::Ready, Some(kind)) => Some(Cli::from_runtime(kind)),
        _ => None,
    };
    emit(
        state.cluster.seq.as_ref(),
        sender,
        StreamEvent::ContainerRuntimePresenceUpdated { presence },
    );
}

async fn refresh_inventory(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    // A refresh with no probe behind it is not an empty inventory: it is an unknown one.
    let Some(cli) = state.containers.cli else {
        emit(
            state.cluster.seq.as_ref(),
            sender,
            StreamEvent::ContainerInventoryUnavailable {
                reason: match state.containers.presence {
                    Some(RuntimePresenceState::Stopped) => {
                        "The container runtime is installed but not running.".to_owned()
                    }
                    Some(RuntimePresenceState::Absent) | None => {
                        "No container runtime was found on this machine.".to_owned()
                    }
                    _ => "The container runtime did not answer.".to_owned(),
                },
            },
        );
        return;
    };

    let read = tokio::task::spawn_blocking(move || read_inventory(cli)).await;
    let (containers, mut images, mut volumes, mut networks) = match read {
        Ok(Ok(parts)) => parts,
        Ok(Err(e)) => {
            emit(
                state.cluster.seq.as_ref(),
                sender,
                StreamEvent::ContainerInventoryUnavailable {
                    reason: runtime_error_text(&e),
                },
            );
            return;
        }
        Err(e) => {
            emit(
                state.cluster.seq.as_ref(),
                sender,
                StreamEvent::ContainerInventoryUnavailable {
                    reason: e.to_string(),
                },
            );
            return;
        }
    };

    inventory::attach_references(&mut images, &containers);
    inventory::attach_references(&mut volumes, &containers);
    inventory::attach_references(&mut networks, &containers);

    // Which clusters Fathom provisioned, so a cluster node is actionable without a project grant
    // and stops being actionable the moment Fathom no longer owns its cluster (FR-016).
    let managed: Vec<String> = persistence::load_kind_provenance()
        .clusters
        .into_iter()
        .map(|r| r.cluster)
        .collect();
    let mut links = grouping::cluster_links(&containers);
    for link in &mut links {
        link.managed_by_fathom = managed.contains(&link.cluster);
    }

    let store = state
        .containers
        .adoption(state.cluster.seq.as_ref(), sender);
    store.set_managed_clusters(managed);
    let projects = grouping::group_containers(&containers, &definition_state, store);

    state.containers.containers = containers.iter().map(|r| r.container.clone()).collect();

    emit(
        state.cluster.seq.as_ref(),
        sender,
        StreamEvent::ContainerInventoryUpdated(Box::new(ContainerInventory {
            projects,
            images,
            volumes,
            networks,
            as_of: chrono::Utc::now().to_rfc3339(),
        })),
    );
    emit(
        state.cluster.seq.as_ref(),
        sender,
        StreamEvent::ContainerClusterLinksUpdated { links },
    );
}

type InventoryParts = (
    Vec<inventory::ContainerRow>,
    Vec<crate::protocol::RemovableObject>,
    Vec<crate::protocol::RemovableObject>,
    Vec<crate::protocol::RemovableObject>,
);

/// The four listings, on a blocking thread. One failure fails the refresh: a partial inventory
/// would show an image as unreferenced because the container listing never arrived.
fn read_inventory(cli: Cli) -> Result<InventoryParts, kind::RunError> {
    let runner = kind::SystemRunner;
    Ok((
        inventory::list_containers_with(cli, &runner)?,
        inventory::list_images_with(cli, &runner)?,
        inventory::list_volumes_with(cli, &runner)?,
        inventory::list_networks_with(cli, &runner)?,
    ))
}

fn runtime_error_text(e: &kind::RunError) -> String {
    match e {
        kind::RunError::NotFound => "The container runtime binary is no longer on PATH.".to_owned(),
        kind::RunError::Other(detail) => detail.clone(),
    }
}

/// Whether a declared definition file is still on disk.
///
/// Existence only: reading and parsing the file is T037's job in `compose.rs`, and
/// `DefinitionFileState::Unreadable` must carry the parser's own words rather than a guess.
fn definition_state(path: &str) -> DefinitionFileState {
    if std::path::Path::new(path).is_file() {
        DefinitionFileState::Present
    } else {
        DefinitionFileState::Missing
    }
}

// ---------------------------------------------------------------------------
// Adoption
// ---------------------------------------------------------------------------

fn adopt(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, project: String) {
    let seq = state.cluster.seq.clone();
    let store = state.containers.adoption(seq.as_ref(), sender);
    match store.adopt(&project) {
        Ok(()) => {
            let adopted = store.adopted();
            emit(
                seq.as_ref(),
                sender,
                StreamEvent::ContainerAdoptionUpdated {
                    adopted,
                    fell_back: false,
                },
            );
        }
        Err(e) => {
            // The grant is in memory but not on disk. Saying nothing would leave the operator
            // with rights that vanish on restart and no idea why.
            events::notify_error(
                seq.as_ref(),
                sender,
                format!("Could not record adoption of '{project}': {e}"),
            );
        }
    }
}

fn release(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, project: String) {
    let seq = state.cluster.seq.clone();
    let store = state.containers.adoption(seq.as_ref(), sender);
    match store.release(&project) {
        Ok(()) => {
            let adopted = store.adopted();
            emit(
                seq.as_ref(),
                sender,
                StreamEvent::ContainerAdoptionUpdated {
                    adopted,
                    fell_back: false,
                },
            );
        }
        Err(ReleaseRefusal::NotAdopted(p)) => emit(
            seq.as_ref(),
            sender,
            StreamEvent::ContainerOperationRefused {
                request_id: format!("release:{p}"),
                reason: OperationRefusal::NotAdopted(p),
            },
        ),
        Err(ReleaseRefusal::NotWritten(detail)) => events::notify_error(
            seq.as_ref(),
            sender,
            format!("Could not record release of '{project}': {detail}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Refusal
// ---------------------------------------------------------------------------

/// Tells the operator the command was not run, on the channel the command came in on.
///
/// A refused session closes the session rather than emitting an operation refusal: the UI has a
/// pane open and waiting, and the refusal has to arrive where the pane is looking.
fn refuse(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    cmd: &BackendCmd,
    reason: OperationRefusal,
) {
    let seq = state.cluster.seq.as_ref();
    match cmd {
        BackendCmd::StartContainerSession { stream_id, .. } => emit(
            seq,
            sender,
            StreamEvent::ContainerSessionClosed {
                stream_id: *stream_id,
                error: Some(refusal_text(&reason)),
            },
        ),
        BackendCmd::StartContainerLogStream { stream_id, .. } => emit(
            seq,
            sender,
            StreamEvent::ContainerSessionClosed {
                stream_id: *stream_id,
                error: Some(refusal_text(&reason)),
            },
        ),
        BackendCmd::PreviewContainerOperation { request_id, .. }
        | BackendCmd::IssueContainerOperation { request_id, .. } => emit(
            seq,
            sender,
            StreamEvent::ContainerOperationRefused {
                request_id: request_id.clone(),
                reason,
            },
        ),
        // Every remaining gated command is keyed by neither a request nor a stream. Reporting it
        // as a notification is worse than nothing only if it is silent, and it is not.
        _ => events::notify_error(seq, sender, refusal_text(&reason)),
    }
}

fn refusal_text(reason: &OperationRefusal) -> String {
    match reason {
        OperationRefusal::NotAdopted(project) => {
            format!("'{project}' has not been adopted, so Fathom will not change it.")
        }
        OperationRefusal::PreviewStale => {
            "The inventory moved since the preview was taken; nothing was run.".to_owned()
        }
        OperationRefusal::ConfirmationMismatch => {
            "The typed confirmation did not match; nothing was run.".to_owned()
        }
        OperationRefusal::PresenceLost => {
            "The container runtime stopped answering; nothing was run.".to_owned()
        }
    }
}

fn emit(seq: &AtomicU64, sender: &Sender<BackendEventEnvelope>, ev: StreamEvent) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::Stream(ev),
    );
}
