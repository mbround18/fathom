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
use crate::backend::container_runtime::preview::{NotPreviewable, PreviewContext, PreviewStore};
use crate::backend::container_runtime::{Cli, compose, grouping, inventory, mutate, probe};
use crate::backend::state::BackendState;
use crate::backend::{events, kind, persistence};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Container, ContainerInventory, Correlation,
    DefinitionFileState, OperationRefusal, OperationRequest, OperationScope, Route,
    RuntimePresenceState, StreamEvent, section_keys,
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
    // `FromPreview` is not a requirement, it is an instruction to go and find one. The preview is
    // what knows the scope; the command carries only an id, deliberately, so a client cannot name
    // a target the preview never showed. A preview that is no longer stored cannot be issued at
    // all, and `PreviewStale` is both the honest answer and the safe one.
    let requirement = if matches!(requirement, Requirement::FromPreview) {
        let BackendCmd::IssueContainerOperation { preview_id, .. } = &cmd else {
            // Only `IssueContainerOperation` is classified `FromPreview`; anything else arriving
            // here is a classification bug, and refusing is the safe way to find out about it.
            refuse(state, sender, &cmd, OperationRefusal::PreviewStale);
            return None;
        };
        match state
            .containers
            .previews
            .get(preview_id)
            .map(|p| p.scope.clone())
        {
            None => {
                refuse(state, sender, &cmd, OperationRefusal::PreviewStale);
                return None;
            }
            // A creation owns nothing yet: there is no existing project whose work it could
            // disturb, and demanding a grant over a project that does not exist would make
            // creation unreachable rather than careful.
            Some(None) => Requirement::Free,
            Some(Some(scope)) => adoption::requirement_for_scope(&scope),
        }
    } else {
        requirement
    };
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
            // A definition's state is part of the inventory, not a reply of its own: `compose.rs`
            // is consulted for every project each time the inventory is read, so re-reading is
            // what loading one amounts to. Doing it this way means the answer arrives with an
            // honest `as_of` for the whole picture rather than one corrected field dated now and
            // the rest dated whenever it was last read.
            refresh_inventory(state, sender).await;
        }
        BackendCmd::AdoptContainerProject { project } => adopt(state, sender, project),
        BackendCmd::ReleaseContainerProject { project } => release(state, sender, project),
        BackendCmd::PreviewContainerOperation {
            request_id,
            request,
        } => preview_operation(state, sender, request_id, &request),
        BackendCmd::IssueContainerOperation {
            request_id,
            preview_id,
            typed_confirmation,
        } => {
            issue_operation(
                state,
                sender,
                request_id,
                &preview_id,
                typed_confirmation.as_deref(),
            )
            .await;
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
    /// The networks of the last inventory, kept because a `compose down` preview has to name
    /// every one it takes with it and cannot ask the daemon mid-preview without the answer
    /// changing between the showing and the acting.
    networks: Vec<crate::protocol::RemovableObject>,
    /// Definition file per project, kept only when it was readable. A project whose file is
    /// missing or unreadable is absent from here, which is how FR-033 is enforced: the preview
    /// cannot be built, so the operation is never offered.
    definitions: std::collections::BTreeMap<String, String>,
    /// When the inventory the previews are derived from was read. A preview whose snapshot has
    /// been replaced is stale, whatever it still says.
    inventory_as_of: Option<String>,
    /// Previews taken this session. Bounded; see `preview::PreviewStore`.
    previews: PreviewStore,
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
    let projects = grouping::group_containers(&containers, &compose::Disk, store);

    state.containers.containers = containers.iter().map(|r| r.container.clone()).collect();
    state.containers.networks = networks.clone();
    // Only a file that was readable at this reading. A project whose file is missing or
    // unreadable drops out, which is how a project-level operation stops being offered for it
    // (FR-033) — and nothing here reads or writes the file to find out.
    state.containers.definitions = projects
        .iter()
        .filter(|p| matches!(p.definition_state, DefinitionFileState::Present))
        .filter_map(|p| match (&p.group, &p.definition_file) {
            (crate::protocol::ProjectGroup::Project(name), Some(file)) => {
                Some((name.clone(), file.clone()))
            }
            _ => None,
        })
        .collect();

    let as_of = chrono::Utc::now().to_rfc3339();
    state.containers.inventory_as_of = Some(as_of.clone());

    emit(
        state.cluster.seq.as_ref(),
        sender,
        StreamEvent::ContainerInventoryUpdated(Box::new(ContainerInventory {
            projects,
            images,
            volumes,
            networks,
            as_of,
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

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Derives the argv and hands it back. Runs nothing.
///
/// Available for an unadopted project on purpose (the gate says `Free`): a preview is how an
/// operator decides whether to adopt, and refusing to show them what an operation would do makes
/// adoption a blind choice.
fn preview_operation(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    request: &OperationRequest,
) {
    let seq = state.cluster.seq.clone();
    // No runtime, no argv. Deriving one against a daemon that is not answering would produce a
    // command the operator could confirm and that could then only fail.
    let Some(cli) = state.containers.cli else {
        emit(
            seq.as_ref(),
            sender,
            StreamEvent::ContainerOperationRefused {
                request_id,
                reason: OperationRefusal::PresenceLost,
            },
        );
        return;
    };

    let project = project_of(request);
    let definition_file = project
        .as_deref()
        .and_then(|p| state.containers.definitions.get(p))
        .map(String::as_str);
    let as_of = state
        .containers
        .inventory_as_of
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let ctx = PreviewContext {
        cli,
        containers: &state.containers.containers,
        networks: &state.containers.networks,
        definition_file,
        active_context: state.cluster.current_context.as_deref(),
        as_of: &as_of,
    };

    match state.containers.previews.take(request, &ctx) {
        Ok(preview) => emit(
            seq.as_ref(),
            sender,
            StreamEvent::ContainerOperationPreviewed {
                request_id,
                preview: Box::new(preview),
            },
        ),
        // There is no `OperationRefusal` variant for "this cannot be described in advance", and
        // inventing one would be a protocol change to carry a sentence. The operator is told
        // instead of the request being dropped, which is the part that matters: a control that
        // does nothing and says nothing is the failure FR-017 names.
        Err(e) => notify_not_previewable(seq.as_ref(), sender, &e),
    }
}

fn notify_not_previewable(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    reason: &NotPreviewable,
) {
    events::notify_error(seq, sender, reason.detail());
}

/// The project an operation is scoped to, if any.
fn project_of(request: &OperationRequest) -> Option<String> {
    match request {
        OperationRequest::ProjectLifecycle { project, .. } => Some(project.clone()),
        OperationRequest::Lifecycle {
            scope: OperationScope::Project { project },
            ..
        } => Some(project.clone()),
        _ => None,
    }
}

/// Runs a preview the operator confirmed.
///
/// Nothing is rebuilt here. The argv comes out of the store, which is what makes the preview a
/// guarantee rather than a rendering — see `container_runtime::mutate`.
async fn issue_operation(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    preview_id: &str,
    typed_confirmation: Option<&str>,
) {
    let seq = state.cluster.seq.clone();

    // The argv already carries its own program, so the runtime is not consulted for one here.
    // It is still checked: issuing a command at a daemon that stopped answering would report a
    // failure that is really an absence, and the operator would read it as the operation's fault.
    if state.containers.cli.is_none() {
        refuse_operation(
            seq.as_ref(),
            sender,
            request_id,
            OperationRefusal::PresenceLost,
        );
        return;
    }

    let Some(stored) = state.containers.previews.get(preview_id).cloned() else {
        refuse_operation(
            seq.as_ref(),
            sender,
            request_id,
            OperationRefusal::PreviewStale,
        );
        return;
    };

    // The inventory may have been re-read since the preview was taken; that alone is harmless.
    // What is not harmless is the target set having changed under it, because then the argv names
    // objects the operator did not see, or no longer names ones they did.
    if target_set_moved(&stored, &state.containers.containers) {
        refuse_operation(
            seq.as_ref(),
            sender,
            request_id,
            OperationRefusal::PreviewStale,
        );
        return;
    }

    if let Err(reason) = mutate::confirmation_ok(&stored, typed_confirmation) {
        refuse_operation(seq.as_ref(), sender, request_id, reason);
        return;
    }

    // Consumed before it runs, not after: a preview that has been acted on must not be issuable a
    // second time even if the run itself fails half way.
    state.containers.previews.consume(preview_id);

    let run =
        tokio::task::spawn_blocking(move || mutate::issue(&kind::SystemRunner, &stored)).await;
    match run {
        Ok(Ok(outcome)) => {
            emit(
                seq.as_ref(),
                sender,
                StreamEvent::ContainerOperationFinished {
                    request_id,
                    outcome,
                },
            );
            // FR-020: the UI reflects nothing until the runtime has been asked again. The outcome
            // above says what the runtime reported; this says what the daemon now looks like.
            refresh_inventory(state, sender).await;
        }
        Ok(Err(e)) => events::notify_error(seq.as_ref(), sender, runtime_error_text(&e)),
        Err(e) => events::notify_error(seq.as_ref(), sender, e.to_string()),
    }
}

/// Whether the objects the preview named are still the objects the inventory has.
///
/// By identity, not by count: a project that lost one container and gained another has the same
/// number of members and a different argv.
fn target_set_moved(
    stored: &crate::backend::container_runtime::preview::StoredPreview,
    containers: &[Container],
) -> bool {
    // A creation names no existing object, so there is no set to have moved.
    if stored.targets.is_empty() {
        return false;
    }
    if stored
        .targets
        .iter()
        .any(|t| !containers.iter().any(|c| c.id == t.id))
    {
        return true;
    }
    match &stored.scope {
        Some(OperationScope::Project { project }) => {
            let now = containers
                .iter()
                .filter(|c| c.project.as_deref() == Some(project.as_str()))
                .count();
            now != stored.targets.len()
        }
        _ => false,
    }
}

fn refuse_operation(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    reason: OperationRefusal,
) {
    emit(
        seq,
        sender,
        StreamEvent::ContainerOperationRefused { request_id, reason },
    );
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
