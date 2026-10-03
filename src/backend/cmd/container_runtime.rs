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
use crate::backend::container_runtime::preview::{PreviewContext, PreviewStore};
use crate::backend::container_runtime::{
    self as container_runtime, Cli, cluster_link, compose, grouping, inventory, logs, mutate,
    probe, session, stats,
};
use crate::backend::log_streams::{CloseGuard, StreamSink};
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
        BackendCmd::StartContainerLogStream {
            stream_id,
            container_id,
            follow,
            tail_lines,
        } => start_log_stream(state, sender, stream_id, &container_id, follow, tail_lines),
        BackendCmd::StopContainerLogStream { stream_id } => state.log_stream.stop(stream_id),
        BackendCmd::StartContainerSession {
            stream_id,
            container_id,
            shell,
        } => start_session(state, sender, stream_id, &container_id, shell.as_deref()),
        BackendCmd::StopContainerSession { stream_id } => stop_session(state, stream_id),
        BackendCmd::SendContainerSessionInput { stream_id, data } => {
            send_session_input(state, sender, stream_id, &data);
        }
        BackendCmd::ResizeContainerSession {
            stream_id,
            cols,
            rows,
        } => {
            if let Some(session) = session_matching(state, stream_id) {
                session.resize(cols, rows);
            }
        }
        BackendCmd::StartContainerUsageSampling { container_ids } => {
            start_usage_sampling(state, sender, container_ids);
        }
        // Dropping the sampler is what stops it, and `Sampler::stop` waits for the loop to be
        // gone rather than only asking it to finish (FR-045).
        BackendCmd::StopContainerUsageSampling => {
            if let Some(sampler) = state.containers.usage.take() {
                sampler.stop();
            }
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
    /// The images and volumes of that same reading, kept for the same reason: a removal set is
    /// resolved against the inventory the operator is looking at, never against a fresh listing
    /// that may have grown an object since they read it (`blast_radius.rs`).
    images: Vec<crate::protocol::RemovableObject>,
    volumes: Vec<crate::protocol::RemovableObject>,
    /// Definition file per project, kept only when it was readable. A project whose file is
    /// missing or unreadable is absent from here, which is how FR-033 is enforced: the preview
    /// cannot be built, so the operation is never offered.
    definitions: std::collections::BTreeMap<String, String>,
    /// When the inventory the previews are derived from was read. A preview whose snapshot has
    /// been replaced is stale, whatever it still says.
    inventory_as_of: Option<String>,
    /// Previews taken this session. Bounded; see `preview::PreviewStore`.
    previews: PreviewStore,
    /// The one live usage sampler, if any.
    ///
    /// One, like the session: the rows being looked at change as the operator scrolls, and a
    /// sampler per change would leave a pile of loops ticking at the same daemon for views that
    /// are no longer on screen. A start replaces whatever was running, and replacing it stops it.
    usage: Option<super::super::container_runtime::stats::Sampler>,
    /// The one live interactive session, if any.
    ///
    /// One, deliberately, exactly as pod exec is one: a second start replaces the first and
    /// replacing it ends it. Keeping several would mean a shell the operator has navigated away
    /// from still holding a process, which is what FR-036 forbids.
    session: Option<super::super::container_runtime::session::Session>,
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
    let mut links = cluster_link::cluster_links(&containers);
    cluster_link::mark_managed(&mut links, &managed);

    let store = state
        .containers
        .adoption(state.cluster.seq.as_ref(), sender);
    store.set_managed_clusters(managed);
    let projects = grouping::group_containers(&containers, &compose::Disk, store);

    state.containers.containers = containers.iter().map(|r| r.container.clone()).collect();
    state.containers.networks = networks.clone();
    state.containers.images = images.clone();
    state.containers.volumes = volumes.clone();
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

    // Taken before the context borrows the state: the grants decide what a removal set may
    // touch, and reading them here is also what emits `fell_back` on first use.
    let adopted = state
        .containers
        .adoption(seq.as_ref(), sender)
        .adopted()
        .to_vec();

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
        images: &state.containers.images,
        volumes: &state.containers.volumes,
        adopted: &adopted,
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
        // A refusal, not a loose notification: it is the answer to *this* request, so it carries
        // the request id and lands on the control that asked. A sentence in the status bar left the
        // scene sitting in "previewing" with nothing to dismiss and no way to tell which control
        // it belonged to.
        Err(e) => emit(
            seq.as_ref(),
            sender,
            StreamEvent::ContainerOperationRefused {
                request_id,
                reason: OperationRefusal::NotPreviewable(e.detail()),
            },
        ),
    }
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
    if target_set_moved(&stored, &state.containers) {
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

    // The preview is needed again after the run, to say what the fresh reading should show.
    let stored_for_verify = stored.clone();
    let run =
        tokio::task::spawn_blocking(move || mutate::issue(&kind::SystemRunner, &stored)).await;
    match run {
        Ok(Ok(mut outcome)) => {
            // FR-020/FR-023: the runtime is asked again *before* the outcome is emitted, because
            // what it now shows is part of the outcome. An operation that reported success and
            // left the object exactly as it was is a discrepancy the operator is told about, and
            // one that reported failure and did the thing anyway is one too.
            refresh_inventory(state, sender).await;
            let objects: Vec<crate::protocol::RemovableObject> = state
                .containers
                .images
                .iter()
                .chain(state.containers.volumes.iter())
                .chain(state.containers.networks.iter())
                .cloned()
                .collect();
            mutate::verify(
                &mut outcome,
                &stored_for_verify,
                &mutate::AfterReading {
                    containers: &state.containers.containers,
                    objects: &objects,
                },
            );
            emit(
                seq.as_ref(),
                sender,
                StreamEvent::ContainerOperationFinished {
                    request_id,
                    outcome,
                },
            );
        }
        Ok(Err(e)) => events::notify_error(seq.as_ref(), sender, runtime_error_text(&e)),
        Err(e) => events::notify_error(seq.as_ref(), sender, e.to_string()),
    }
}

/// Whether the objects the preview named are still the objects the inventory has.
///
/// By identity, not by count: a project that lost one container and gained another has the same
/// number of members and a different argv.
///
/// Each target is looked for in the pool its own class lives in. Checking an image id against the
/// container list would make every image removal look stale, which is a refusal the operator
/// could never get past.
fn target_set_moved(
    stored: &crate::backend::container_runtime::preview::StoredPreview,
    inventory: &RuntimeState,
) -> bool {
    // A creation names no existing object, so there is no set to have moved.
    if stored.targets.is_empty() {
        return false;
    }
    let containers = &inventory.containers;
    if stored.targets.iter().any(|t| {
        let present = match t.class {
            crate::protocol::ObjectClass::Container => containers.iter().any(|c| c.id == t.id),
            crate::protocol::ObjectClass::Image => inventory.images.iter().any(|o| o.id == t.id),
            crate::protocol::ObjectClass::Volume => inventory.volumes.iter().any(|o| o.id == t.id),
            crate::protocol::ObjectClass::Network => {
                inventory.networks.iter().any(|o| o.id == t.id)
            }
        };
        !present
    }) {
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
// Logs and the interactive session
// ---------------------------------------------------------------------------

/// Follows one container's output (FR-035).
///
/// Ungated on purpose: reading is readable whether the project was adopted or not, and the gate
/// says so (`adoption::requirement` classifies this `Free`). It runs on the application's one
/// stream registry rather than a second one of its own, so a context switch or a shutdown that
/// calls `stop_all` ends container follows too, and so the "exactly one close per stream" rule
/// stays in the one place that already enforces it.
/// Starts the usage sampler over exactly the containers the operator can see.
///
/// Scoped to those ids and to no others: an unscoped `docker stats` samples every container on the
/// daemon, and this one is shared with unrelated projects whose figures nobody asked for. A start
/// with no ids is therefore a stop — the view has nothing visible left to sample.
fn start_usage_sampling(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    container_ids: Vec<String>,
) {
    // Whatever was ticking is for a set of rows that are no longer the visible ones.
    if let Some(previous) = state.containers.usage.take() {
        previous.stop();
    }
    if container_ids.is_empty() {
        return;
    }
    // No runtime, no figures. Silently, because the scene that would show them already says why
    // the runtime is unavailable; a second sentence about it here would be the same fact twice.
    let Some(cli) = state.containers.cli else {
        return;
    };
    let seq = state.cluster.seq.clone();
    let sender = sender.clone();
    state.containers.usage = Some(stats::Sampler::start(
        cli,
        std::sync::Arc::new(kind::SystemRunner),
        container_ids,
        stats::MIN_INTERVAL,
        Box::new(move |samples, at_cap| {
            emit(
                &seq,
                &sender,
                StreamEvent::ContainerUsageSampled { samples, at_cap },
            );
        }),
    ));
}

fn start_log_stream(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    stream_id: crate::protocol::StreamId,
    container_id: &str,
    follow: bool,
    tail_lines: Option<u32>,
) {
    let guard = CloseGuard::new(StreamSink {
        stream_id,
        route: Route::global(section_keys::system()),
        seq: state.cluster.seq.clone(),
        sender: sender.clone(),
    });
    // A client id below the reserved floor would let a client collide with a backend-owned stream
    // and silently take over its pane; the pods path refuses the same way.
    let refused = (stream_id.0 < crate::protocol::CLIENT_STREAM_ID_MIN)
        .then(|| format!("Log stream id {} is reserved for the backend.", stream_id.0));
    let bin = state.containers.cli.map(|cli| cli.bin().to_owned());
    let args = logs::argv(container_id, follow, tail_lines);

    state
        .log_stream
        .spawn(guard, false, move |mut guard| async move {
            if let Some(reason) = refused {
                guard.failed(reason);
                return;
            }
            // No probe behind us means an unknown runtime, not an empty log. Saying which is the
            // difference between "nothing was written" and "we never looked".
            let Some(bin) = bin else {
                guard.failed(
                    "Fathom has not found a container runtime on this machine yet.".to_owned(),
                );
                return;
            };
            logs::follow(
                guard,
                std::sync::Arc::new(container_runtime::stream::SystemStreamRunner),
                bin,
                args,
            )
            .await;
        });
}

/// Opens an interactive session inside one container (FR-036).
///
/// Gated: the adoption check has already run by the time this is reached, so arriving here means
/// the operator holds the grant (or the container backs a cluster Fathom manages).
fn start_session(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    stream_id: crate::protocol::StreamId,
    container_id: &str,
    shell: Option<&str>,
) {
    let seq = state.cluster.seq.as_ref();
    // Replacing a session ends the one being replaced, rather than leaving its shell running
    // behind a pane nobody can see any more.
    if let Some(previous) = state.containers.session.take() {
        previous.close();
    }
    let Some(cli) = state.containers.cli else {
        emit(
            seq,
            sender,
            StreamEvent::ContainerSessionClosed {
                stream_id,
                error: Some(
                    "Fathom has not found a container runtime on this machine yet.".to_owned(),
                ),
            },
        );
        return;
    };
    let sink = StreamSink {
        stream_id,
        route: Route::global(section_keys::system()),
        seq: state.cluster.seq.clone(),
        sender: sender.clone(),
    };
    match session::Session::open(
        stream_id,
        cli.bin(),
        &session::argv(container_id, shell),
        sink,
    ) {
        Ok(session) => state.containers.session = Some(session),
        // The runtime's own words, verbatim (FR-004). A session that cannot start has to say why
        // in the pane that is waiting for it.
        Err(e) => emit(
            seq,
            sender,
            StreamEvent::ContainerSessionClosed {
                stream_id,
                error: Some(e.to_string()),
            },
        ),
    }
}

/// The live session, but only if it is the one the command names.
///
/// A command for a stream that has already closed must do nothing rather than reach the session
/// that replaced it: input typed into a closed pane cannot be allowed to land in a live shell.
fn session_matching(
    state: &mut BackendState,
    stream_id: crate::protocol::StreamId,
) -> Option<&mut session::Session> {
    state
        .containers
        .session
        .as_mut()
        .filter(|s| s.stream_id == stream_id)
}

fn stop_session(state: &mut BackendState, stream_id: crate::protocol::StreamId) {
    if session_matching(state, stream_id).is_some()
        && let Some(session) = state.containers.session.take()
    {
        // `close` emits nothing itself; the reader thread sees its EOF and sends the one
        // `ContainerSessionClosed` for this stream.
        session.close();
    }
}

fn send_session_input(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    stream_id: crate::protocol::StreamId,
    data: &str,
) {
    let failed = match session_matching(state, stream_id) {
        None => return,
        Some(session) => session.write(data).err(),
    };
    if let Some(e) = failed {
        // The shell is gone and the terminal will not take input. Ending here rather than
        // swallowing it means the pane is told once, instead of accepting keystrokes into nothing.
        if let Some(session) = state.containers.session.take() {
            session.close();
        }
        emit(
            state.cluster.seq.as_ref(),
            sender,
            StreamEvent::ContainerSessionClosed {
                stream_id,
                error: Some(e.to_string()),
            },
        );
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
        // Already a whole sentence by the time it gets here — see `NotPreviewable::detail`.
        OperationRefusal::NotPreviewable(detail) => detail.clone(),
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
