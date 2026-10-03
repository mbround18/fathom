//! The first half of every mutation: say what will run, before anything runs.
//!
//! A preview is not a description of an operation — it *is* the operation, stored. The argv it
//! carries is the argv `mutate` later executes, byte for byte, because `mutate` reads it back out
//! of the store rather than rebuilding it from the request. That is the whole reason this module
//! exists separately: if issuing rebuilt the command, the preview would be a rendering of an
//! intention and the two could drift without anyone noticing.
//! `mutate::tests::issued_argv_is_byte_identical_to_the_preview` is what keeps that honest.
//!
//! The removal set and the blast radius are not computed here: `blast_radius.rs` resolves them
//! against the same inventory snapshot, and this module asks it. What is enforced here is the
//! consequence — an operation whose scope cannot be enumerated is not offered at all (FR-027), so
//! a removal that resolves to nothing, or to objects of more than one kind, is refused rather than
//! previewed with a blast radius that appears to be nothing.

use std::collections::VecDeque;

use crate::protocol::v1::{
    Container, ContainerSpec, ExcludedObject, ExclusionReason, ImageBuildSpec, LifecycleAction,
    NetworkSpec, ObjectClass, ObjectRef, OperationPreview, OperationRequest, OperationScope,
    ProjectAction, RemovableObject, RemovalLine, RuntimeContainerState, VolumeSpec,
};

use super::Cli;
use super::blast_radius::{Scene, resolve_objects, resolve_prune};
use super::cluster_link;

/// Everything a preview is derived from, as of one reading of the runtime.
pub struct PreviewContext<'a> {
    pub cli: Cli,
    /// The containers of the inventory the preview is derived from.
    pub containers: &'a [Container],
    /// The images and volumes of that same inventory. Needed only by the destructive operations,
    /// which are resolved to identities here rather than handed to the runtime as a filter.
    pub images: &'a [RemovableObject],
    pub volumes: &'a [RemovableObject],
    /// The networks of that same inventory, needed because bringing a project down takes its
    /// networks with it and FR-023 forbids summarising any of them as a count.
    pub networks: &'a [RemovableObject],
    /// The definition file a project declares, resolved by the caller, which already knows whether
    /// it is readable. `None` means project-level operations are unavailable for that project
    /// (FR-033) — and this module never reads or writes the file itself.
    pub definition_file: Option<&'a str>,
    /// The kubeconfig context the operator is connected to right now, used only to decide whether
    /// a cluster this operation breaks is *the* cluster they are looking at.
    pub active_context: Option<&'a str>,
    /// The projects the operator has adopted, which is what decides whether an object inside a
    /// removal set is touched or excluded (FR-025).
    pub adopted: &'a [String],
    /// The inventory timestamp. Everything derived here is only as true as this.
    pub as_of: &'a str,
}

/// Why no preview could be produced.
///
/// Each variant is something the operator is told, not something logged: a control that vanishes
/// with no reason given is the failure FR-017 exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotPreviewable {
    /// The scope cannot be enumerated from the inventory, so the operation is not offered.
    NotEnumerable(String),
    /// The operation names something this inventory does not contain.
    UnknownTarget(String),
    /// The project declares no readable definition file, so there is nothing to act on (FR-033).
    NoDefinitionFile(String),
    /// Every object in the selection was excluded, so there is nothing left to remove. Carries the
    /// projects that did the excluding, named, because that is what the operator can act on.
    AllExcluded(Vec<String>),
}

impl NotPreviewable {
    /// The operator-facing sentence. Plain, and never blaming them for it.
    pub fn detail(&self) -> String {
        match self {
            Self::NotEnumerable(what) => format!(
                "Fathom cannot list in advance everything {what} would affect, so it does not \
                 offer it."
            ),
            Self::UnknownTarget(id) => {
                format!("'{id}' is not in the inventory Fathom last read.")
            }
            Self::NoDefinitionFile(project) => format!(
                "'{project}' has no readable definition file, so project-level operations are \
                 unavailable for it. Fathom has not changed the file."
            ),
            Self::AllExcluded(projects) => format!(
                "Every object in that selection belongs to a project that has not been adopted: \
                 {}. Adopt one to act on its objects.",
                projects.join(", ")
            ),
        }
    }
}

/// A stored preview: what the operator saw, plus what issuing it needs that they did not.
#[derive(Debug, Clone)]
pub struct StoredPreview {
    pub preview: OperationPreview,
    /// What the runtime should look like afterwards, per object.
    ///
    /// Derived here rather than in `mutate`, because the request is what says what success means
    /// and this is the last place it is read. `mutate::verify` compares a fresh reading against
    /// this (FR-023): an operation that reported success and left the object exactly as it was is
    /// a discrepancy the operator must be told about, not a green tick.
    pub expected: Vec<(ObjectRef, Expected)>,
    /// Every object the argv names, in argv order, so a per-object outcome can be attributed.
    pub targets: Vec<ObjectRef>,
    /// What the adoption gate must be satisfied for. `None` for an operation that creates
    /// something new: there is no existing project whose work it could disturb, and demanding a
    /// grant over a project that does not exist yet would make creation unreachable.
    pub scope: Option<OperationScope>,
}

/// What one object should look like once the operation has run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// The object is gone — every removal, and a `compose down`.
    Absent,
    /// The container is in this state.
    ContainerState(RuntimeContainerState),
}

/// The state a lifecycle verb promises to leave a container in.
///
/// `Restart` and `Up` promise `Running`: a verb whose whole point is that the container comes back
/// is not satisfied by one that went away.
fn promised(action: LifecycleAction) -> RuntimeContainerState {
    match action {
        LifecycleAction::Start | LifecycleAction::Restart | LifecycleAction::Unpause => {
            RuntimeContainerState::Running
        }
        LifecycleAction::Stop | LifecycleAction::Kill => RuntimeContainerState::Exited,
        LifecycleAction::Pause => RuntimeContainerState::Paused,
    }
}

/// The previews taken this session, newest last.
///
/// Bounded because an operator who opens and closes a preview a hundred times must not grow the
/// backend's memory a hundred times, and because a preview old enough to have fallen out is a
/// preview whose inventory has certainly moved — the correct answer for which is already
/// `PreviewStale`.
pub struct PreviewStore {
    entries: VecDeque<StoredPreview>,
    next: u64,
}

/// How many previews are remembered at once. Generous for one operator working through a project,
/// far short of unbounded.
const CAPACITY: usize = 16;

impl Default for PreviewStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewStore {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next: 1,
        }
    }

    /// Mints the next preview id. Monotonic within a session, never reused, so a stale id from an
    /// earlier preview can never resolve to a later one's argv.
    fn mint(&mut self) -> String {
        let id = format!("prev-{}", self.next);
        self.next += 1;
        id
    }

    /// Builds a preview and remembers it. Returns what the operator is shown.
    pub fn take(
        &mut self,
        request: &OperationRequest,
        ctx: &PreviewContext<'_>,
    ) -> Result<OperationPreview, NotPreviewable> {
        let id = self.mint();
        let stored = build(id, request, ctx)?;
        let preview = stored.preview.clone();
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(stored);
        Ok(preview)
    }

    /// The stored preview for an id, or `None` when it was never taken or has aged out.
    pub fn get(&self, preview_id: &str) -> Option<&StoredPreview> {
        self.entries
            .iter()
            .find(|e| e.preview.preview_id == preview_id)
    }

    /// Forgets a preview once it has been issued, so the same preview cannot be issued twice.
    /// A second confirmation of the same button press would run the operation again against an
    /// inventory that has, by then, definitely changed.
    pub fn consume(&mut self, preview_id: &str) -> Option<StoredPreview> {
        let at = self
            .entries
            .iter()
            .position(|e| e.preview.preview_id == preview_id)?;
        self.entries.remove(at)
    }
}

/// Derives one preview. Separate from the store so it can be tested without one.
pub fn build(
    preview_id: String,
    request: &OperationRequest,
    ctx: &PreviewContext<'_>,
) -> Result<StoredPreview, NotPreviewable> {
    // Resolved first, because for a destructive request the removal set *is* the argv: the
    // identities decide what runs, and nothing downstream may widen them.
    let radius = match request {
        OperationRequest::Remove { objects } => Some(resolve_objects(&scene(ctx), objects)?),
        OperationRequest::PruneSelection { class, filter } => {
            Some(resolve_prune(&scene(ctx), *class, *filter)?)
        }
        _ => None,
    };
    // A selection every object of which an unadopted project can reach leaves nothing to do. It
    // is not offered as a preview that removes nothing: a typed confirmation over a no-op teaches
    // the operator that the guard is noise, and the reason they want is which project to adopt.
    if let Some(r) = &radius
        && r.removals.is_empty()
    {
        return Err(NotPreviewable::AllExcluded(blockers(&r.excluded)));
    }

    let (command, targets, scope) = match request {
        OperationRequest::Lifecycle { action, scope } => {
            let (command, targets) = lifecycle(ctx, *action, scope)?;
            (command, targets, Some(scope.clone()))
        }
        OperationRequest::ProjectLifecycle { project, action } => {
            let (command, targets) = project_lifecycle(ctx, project, *action)?;
            (
                command,
                targets,
                Some(OperationScope::Project {
                    project: project.clone(),
                }),
            )
        }
        OperationRequest::CreateContainer { spec } => {
            (create_container(ctx.cli, spec), vec![], None)
        }
        OperationRequest::BuildImage { spec } => (build_image(ctx.cli, spec), vec![], None),
        OperationRequest::CreateNetwork { spec } => (create_network(ctx.cli, spec), vec![], None),
        OperationRequest::CreateVolume { spec } => (create_volume(ctx.cli, spec), vec![], None),
        // Both resolve the same way: by identity, never by filter. A prune is not issued as a
        // prune (FR-026) — what runs is a removal of exactly the objects the preview listed, so
        // the daemon cannot add to the set between reading it and acting on it.
        OperationRequest::Remove { .. } | OperationRequest::PruneSelection { .. } => {
            let radius = radius.as_ref().expect("resolved for destructive requests");
            let command = removal_argv(ctx.cli, &radius.removals)?;
            let targets: Vec<ObjectRef> =
                radius.removals.iter().map(|r| r.object.clone()).collect();
            (command, targets, removal_scope(&radius.removals))
        }
    };

    let destructive = request.is_destructive();
    // What a `down` takes with it: the project's containers, and the networks compose created for
    // it. Enumerated by identity, every one named — a count would be exactly what FR-023 forbids.
    // Volumes are absent on purpose: `compose down` leaves them unless asked with `-v`, and
    // listing them would warn about a loss that is not going to happen.
    let (removals, blast_radius, excluded, required_confirmation) = match request {
        OperationRequest::ProjectLifecycle {
            project,
            action: ProjectAction::Down,
        } => (
            down_removals(ctx, project, &targets),
            vec![project.clone()],
            vec![],
            // The project's own name, not a constant: a confirmation that is the same word every
            // time is a reflex, and a reflex confirms the wrong project as readily as the right
            // one.
            Some(project.clone()),
        ),
        // One resolution fills all four, so the list, the radius and the exclusions cannot
        // disagree about the same selection.
        _ => match radius {
            Some(r) => {
                let confirmation = confirmation_for(&r.removals);
                (r.removals, r.projects, r.excluded, Some(confirmation))
            }
            None => (vec![], vec![], vec![], None),
        },
    };

    debug_assert!(
        !destructive || required_confirmation.is_some(),
        "a destructive preview without a typed confirmation would skip the guard entirely"
    );
    let expected = expectations(request, &targets, &removals);

    Ok(StoredPreview {
        preview: OperationPreview {
            preview_id,
            request: request.clone(),
            command,
            destructive,
            total_size_bytes: sum_sizes(&removals),
            removals,
            blast_radius,
            excluded,
            breaks_clusters: cluster_link::cluster_breaks(
                request,
                &targets,
                ctx.containers,
                ctx.active_context,
            ),
            required_confirmation,
            as_of: ctx.as_of.to_owned(),
        },
        targets,
        scope,
        expected,
    })
}

/// What a `compose down` takes with it: the project's containers, and the networks compose
/// created for it.
///
/// Enumerated by identity, every one named — a count would be exactly what FR-023 forbids.
/// Volumes are absent on purpose: `compose down` leaves them unless asked with `-v`, and listing
/// them would warn about a loss that is not going to happen.
fn down_removals(
    ctx: &PreviewContext<'_>,
    project: &str,
    targets: &[ObjectRef],
) -> Vec<RemovalLine> {
    let mut lines: Vec<RemovalLine> = targets
        .iter()
        .map(|object| RemovalLine {
            object: object.clone(),
            size_bytes: None,
            project: Some(project.to_owned()),
        })
        .collect();
    lines.extend(
        ctx.networks
            .iter()
            .filter(|n| n.project.as_deref() == Some(project))
            .map(|n| RemovalLine {
                object: ObjectRef {
                    class: n.class,
                    id: n.id.clone(),
                    name: n.name.clone(),
                },
                size_bytes: n.size_bytes,
                project: n.project.clone(),
            }),
    );
    lines
}

/// Every object the operation touches, paired with what it promised to leave behind.
///
/// A removal's promise is its removal line; a lifecycle verb's is a state. `mutate::verify` has
/// nothing to check an operation against but this, so an object missing here is an object whose
/// outcome nobody re-reads.
fn expectations(
    request: &OperationRequest,
    targets: &[ObjectRef],
    removals: &[RemovalLine],
) -> Vec<(ObjectRef, Expected)> {
    match request {
        OperationRequest::Lifecycle { action, .. } => targets
            .iter()
            .map(|t| (t.clone(), Expected::ContainerState(promised(*action))))
            .collect(),
        OperationRequest::ProjectLifecycle { action, .. } => match action {
            ProjectAction::Down => removals
                .iter()
                .map(|r| (r.object.clone(), Expected::Absent))
                .collect(),
            ProjectAction::Up | ProjectAction::Restart => targets
                .iter()
                .map(|t| {
                    (
                        t.clone(),
                        Expected::ContainerState(RuntimeContainerState::Running),
                    )
                })
                .collect(),
        },
        OperationRequest::Remove { .. } | OperationRequest::PruneSelection { .. } => removals
            .iter()
            .map(|r| (r.object.clone(), Expected::Absent))
            .collect(),
        // A creation has no object to verify until it exists, and the name it will take is not an
        // identity the inventory can be checked against yet.
        _ => vec![],
    }
}

/// The scene a destructive request is resolved against: this reading of the runtime, and the
/// grants in force over it.
fn scene<'a>(ctx: &'a PreviewContext<'a>) -> Scene<'a> {
    Scene {
        containers: ctx.containers,
        images: ctx.images,
        volumes: ctx.volumes,
        networks: ctx.networks,
        adopted: ctx.adopted,
    }
}

/// One argv for the whole removal, which is what forces a selection to be of one kind.
///
/// `docker rm`, `docker image rm`, `docker volume rm` and `docker network rm` are four different
/// commands. A mixed selection would therefore need several argvs, and a preview that showed one
/// of them while running all four would be the drift this whole two-phase path exists to prevent.
/// So it is refused, and the operator removes the images and then the volumes — two previews, two
/// confirmations, each of them true.
fn removal_argv(cli: Cli, removals: &[RemovalLine]) -> Result<Vec<String>, NotPreviewable> {
    let mut classes: Vec<ObjectClass> = removals.iter().map(|r| r.object.class).collect();
    classes.dedup();
    let [class] = classes[..] else {
        return Err(NotPreviewable::NotEnumerable(
            "removing objects of more than one kind in one command".to_owned(),
        ));
    };
    let mut command: Vec<String> = match class {
        // A container is removed by `rm` alone; the others carry their class first.
        ObjectClass::Container => vec![cli.bin().to_owned(), "rm".to_owned()],
        ObjectClass::Image => vec![cli.bin().to_owned(), "image".to_owned(), "rm".to_owned()],
        ObjectClass::Volume => vec![cli.bin().to_owned(), "volume".to_owned(), "rm".to_owned()],
        ObjectClass::Network => vec![cli.bin().to_owned(), "network".to_owned(), "rm".to_owned()],
    };
    command.extend(removals.iter().map(|r| r.object.id.clone()));
    Ok(command)
}

/// What the adoption gate is re-checked against when the operation is issued.
///
/// Every object in the set belongs to one adopted project, or to none at all — `blast_radius`
/// excluded anything else. When they share a project, that project is the scope and the gate
/// applies again at issue time. When they belong to none, there is no scope: the no-project bucket
/// is deliberately not adoptable (`adoption.rs`), so demanding a grant over it would make a
/// dangling image impossible to ever remove.
fn removal_scope(removals: &[RemovalLine]) -> Option<OperationScope> {
    let mut projects = removals.iter().map(|r| r.project.as_deref());
    let first = projects.next().flatten()?;
    projects
        .all(|p| p == Some(first))
        .then(|| OperationScope::Project {
            project: first.to_owned(),
        })
}

/// The projects that excluded every object in a selection, each named once, in order.
///
/// Named rather than counted, because "two objects were left alone" is not something the operator
/// can act on and "blog has not been adopted" is.
fn blockers(excluded: &[ExcludedObject]) -> Vec<String> {
    let mut names: Vec<String> = excluded
        .iter()
        .map(|e| match &e.reason {
            ExclusionReason::UnadoptedProject(project) => project.clone(),
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The string the operator types to confirm a removal.
///
/// It is the object's own name when there is one, and the count with the kind when there are
/// several. Either way it changes with what is being removed, which is the point: a confirmation
/// that reads the same every time is a reflex, and a reflex approves the wrong list as readily as
/// the right one. The list itself is above it, in full — this is the deliberate act, not the
/// summary.
fn confirmation_for(removals: &[RemovalLine]) -> String {
    match removals {
        [one] => one.object.name.clone(),
        many => format!("remove {} {}", many.len(), plural(many[0].object.class)),
    }
}

fn plural(class: ObjectClass) -> &'static str {
    match class {
        ObjectClass::Container => "containers",
        ObjectClass::Image => "images",
        ObjectClass::Volume => "volumes",
        ObjectClass::Network => "networks",
    }
}

/// Total of the sizes that are known.
///
/// `None` when nothing in the set reports one, rather than `0`: "we do not know how much this
/// frees" and "this frees nothing" are different sentences, and only one of them makes a removal
/// look pointless.
fn sum_sizes(removals: &[RemovalLine]) -> Option<u64> {
    let known: Vec<u64> = removals.iter().filter_map(|r| r.size_bytes).collect();
    (!known.is_empty()).then(|| known.iter().sum())
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

fn verb(action: LifecycleAction) -> &'static str {
    match action {
        LifecycleAction::Start => "start",
        LifecycleAction::Stop => "stop",
        LifecycleAction::Restart => "restart",
        LifecycleAction::Pause => "pause",
        LifecycleAction::Unpause => "unpause",
        LifecycleAction::Kill => "kill",
    }
}

/// One argv for the whole scope.
///
/// A project-scoped verb names every member in a single command rather than issuing one command
/// per container, which is what lets FR-029 hold: one preview, one confirmation. The per-member
/// outcome FR-022 demands comes from the runtime's own output, which names each id it handled.
fn lifecycle(
    ctx: &PreviewContext<'_>,
    action: LifecycleAction,
    scope: &OperationScope,
) -> Result<(Vec<String>, Vec<ObjectRef>), NotPreviewable> {
    let targets = match scope {
        OperationScope::Container { id } => {
            let found = ctx
                .containers
                .iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| NotPreviewable::UnknownTarget(id.clone()))?;
            vec![object_ref(found)]
        }
        OperationScope::Project { project } => {
            // Sorted by name, not by whatever order the daemon listed them in, so the preview the
            // operator reads twice reads the same twice.
            let mut members: Vec<&Container> = ctx
                .containers
                .iter()
                .filter(|c| c.project.as_deref() == Some(project.as_str()))
                .collect();
            members.sort_by(|a, b| a.name.cmp(&b.name));
            if members.is_empty() {
                return Err(NotPreviewable::UnknownTarget(project.clone()));
            }
            members.into_iter().map(object_ref).collect()
        }
    };

    let mut command = vec![ctx.cli.bin().to_owned(), verb(action).to_owned()];
    command.extend(targets.iter().map(|t| t.id.clone()));
    Ok((command, targets))
}

/// `compose` against the project's own file.
///
/// `-f` is passed explicitly rather than relying on the working directory, because the daemon's
/// idea of a project's file is the one the containers were created from, and Fathom's working
/// directory has nothing to do with it. The file is read by nobody here and written by nobody at
/// all (Principle VI).
fn project_lifecycle(
    ctx: &PreviewContext<'_>,
    project: &str,
    action: ProjectAction,
) -> Result<(Vec<String>, Vec<ObjectRef>), NotPreviewable> {
    let file = ctx
        .definition_file
        .ok_or_else(|| NotPreviewable::NoDefinitionFile(project.to_owned()))?;

    let verb = match action {
        ProjectAction::Up => "up",
        ProjectAction::Down => "down",
        ProjectAction::Restart => "restart",
    };

    let mut command = vec![
        ctx.cli.bin().to_owned(),
        "compose".to_owned(),
        "-p".to_owned(),
        project.to_owned(),
        "-f".to_owned(),
        file.to_owned(),
        verb.to_owned(),
    ];
    if matches!(action, ProjectAction::Up) {
        // Detached, because an `up` that holds the terminal open would hold a backend task open
        // with it and the operator has no terminal to read.
        command.push("-d".to_owned());
    }

    let mut members: Vec<&Container> = ctx
        .containers
        .iter()
        .filter(|c| c.project.as_deref() == Some(project))
        .collect();
    members.sort_by(|a, b| a.name.cmp(&b.name));
    // An `up` on a project with no containers yet is legitimate: the targets are what compose
    // will create, which Fathom cannot name in advance, and the preview says so by naming none
    // rather than by inventing service names out of a file it has not parsed.
    Ok((command, members.into_iter().map(object_ref).collect()))
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/// `run` or `create`, from the form's fields and nothing else.
///
/// Every value lands in its own argv element, so a name, an environment value or a command word
/// containing a space, a quote or a semicolon is one argument and cannot become two. There is no
/// shell in this path to interpret it.
fn create_container(cli: Cli, spec: &ContainerSpec) -> Vec<String> {
    let mut command = vec![cli.bin().to_owned()];
    command.push(if spec.detach { "run" } else { "create" }.to_owned());
    if spec.detach {
        command.push("-d".to_owned());
    }
    if let Some(name) = &spec.name {
        command.push("--name".to_owned());
        command.push(name.clone());
    }
    for port in &spec.ports {
        if let Some(host) = port.host_port {
            command.push("-p".to_owned());
            let mapping = match &port.host_ip {
                Some(ip) => format!("{ip}:{host}:{}", port.container_port),
                None => format!("{host}:{}", port.container_port),
            };
            command.push(format!("{mapping}/{}", port.protocol));
        } else {
            // Exposed but not published: a host port Fathom invented would bind something the
            // operator never asked to bind.
            command.push("--expose".to_owned());
            command.push(port.container_port.to_string());
        }
    }
    for (key, value) in &spec.env {
        command.push("-e".to_owned());
        command.push(format!("{key}={value}"));
    }
    for (source, target) in &spec.volumes {
        command.push("-v".to_owned());
        command.push(format!("{source}:{target}"));
    }
    for network in &spec.networks {
        command.push("--network".to_owned());
        command.push(network.clone());
    }
    if let Some(policy) = &spec.restart_policy {
        command.push("--restart".to_owned());
        command.push(policy.clone());
    }
    command.push(spec.image.clone());
    command.extend(spec.command.iter().cloned());
    command
}

fn build_image(cli: Cli, spec: &ImageBuildSpec) -> Vec<String> {
    let mut command = vec![cli.bin().to_owned(), "build".to_owned()];
    command.push("-t".to_owned());
    command.push(spec.tag.clone());
    if let Some(dockerfile) = &spec.dockerfile {
        command.push("-f".to_owned());
        command.push(dockerfile.clone());
    }
    if let Some(target) = &spec.target {
        command.push("--target".to_owned());
        command.push(target.clone());
    }
    if spec.no_cache {
        command.push("--no-cache".to_owned());
    }
    for (key, value) in &spec.build_args {
        command.push("--build-arg".to_owned());
        command.push(format!("{key}={value}"));
    }
    // Last, and positional: a context path that happens to start with a dash is still the context.
    command.push(spec.context_path.clone());
    command
}

fn create_network(cli: Cli, spec: &NetworkSpec) -> Vec<String> {
    let mut command = vec![
        cli.bin().to_owned(),
        "network".to_owned(),
        "create".to_owned(),
    ];
    if let Some(driver) = &spec.driver {
        command.push("--driver".to_owned());
        command.push(driver.clone());
    }
    if let Some(subnet) = &spec.subnet {
        command.push("--subnet".to_owned());
        command.push(subnet.clone());
    }
    if let Some(gateway) = &spec.gateway {
        command.push("--gateway".to_owned());
        command.push(gateway.clone());
    }
    if spec.internal {
        command.push("--internal".to_owned());
    }
    for (key, value) in &spec.labels {
        command.push("--label".to_owned());
        command.push(format!("{key}={value}"));
    }
    command.push(spec.name.clone());
    command
}

fn create_volume(cli: Cli, spec: &VolumeSpec) -> Vec<String> {
    let mut command = vec![
        cli.bin().to_owned(),
        "volume".to_owned(),
        "create".to_owned(),
    ];
    if let Some(driver) = &spec.driver {
        command.push("--driver".to_owned());
        command.push(driver.clone());
    }
    for (key, value) in &spec.labels {
        command.push("--label".to_owned());
        command.push(format!("{key}={value}"));
    }
    command.push(spec.name.clone());
    command
}

// ---------------------------------------------------------------------------
// Cluster breakage
// ---------------------------------------------------------------------------

fn object_ref(container: &Container) -> ObjectRef {
    ObjectRef {
        class: ObjectClass::Container,
        id: container.id.clone(),
        name: container.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::{AdoptionState, PublishedPort};

    const NO_OBJECTS: &[RemovableObject] = &[];

    fn container(id: &str, name: &str, project: Option<&str>) -> Container {
        Container {
            id: id.to_owned(),
            name: name.to_owned(),
            image: "busybox".to_owned(),
            state: RuntimeContainerState::Running,
            health: None,
            status: "Up 1 minute".to_owned(),
            created_at: "2026-10-03T00:00:00Z".to_owned(),
            command: "sh".to_owned(),
            ports: vec![],
            project: project.map(str::to_owned),
            service: None,
            cluster: None,
            adoption: AdoptionState::NotAdopted,
        }
    }

    fn ctx<'a>(
        containers: &'a [Container],
        definition_file: Option<&'a str>,
    ) -> PreviewContext<'a> {
        PreviewContext {
            cli: Cli::docker(),
            containers,
            images: NO_OBJECTS,
            volumes: NO_OBJECTS,
            networks: NO_OBJECTS,
            definition_file,
            active_context: None,
            adopted: &[],
            as_of: "2026-10-03T12:00:00Z",
        }
    }

    fn preview_of(request: &OperationRequest, ctx: &PreviewContext<'_>) -> OperationPreview {
        build("prev-1".to_owned(), request, ctx)
            .expect("this request is previewable")
            .preview
    }

    /// The whole of US7's promise in one assertion: every field the form collected is its own argv
    /// element. A value with a space, a quote or a semicolon in it is one argument and stays one,
    /// because nothing on this path concatenates and nothing on it reaches a shell.
    #[test]
    fn a_run_spells_out_every_field_the_form_collected_as_its_own_argument() {
        let request = OperationRequest::CreateContainer {
            spec: ContainerSpec {
                image: "busybox:latest".to_owned(),
                name: Some("my app".to_owned()),
                ports: vec![PublishedPort {
                    container_port: 80,
                    host_port: Some(8080),
                    host_ip: Some("127.0.0.1".to_owned()),
                    protocol: "tcp".to_owned(),
                }],
                env: vec![("GREETING".to_owned(), "hello; rm -rf /".to_owned())],
                volumes: vec![("/srv/data".to_owned(), "/data".to_owned())],
                networks: vec!["shop_default".to_owned()],
                command: vec!["sh".to_owned(), "-c".to_owned(), "sleep 3600".to_owned()],
                restart_policy: Some("unless-stopped".to_owned()),
                detach: true,
            },
        };
        let containers: Vec<Container> = vec![];
        let preview = preview_of(&request, &ctx(&containers, None));

        assert_eq!(
            preview.command,
            vec![
                "docker",
                "run",
                "-d",
                "--name",
                "my app",
                "-p",
                "127.0.0.1:8080:80/tcp",
                "-e",
                "GREETING=hello; rm -rf /",
                "-v",
                "/srv/data:/data",
                "--network",
                "shop_default",
                "--restart",
                "unless-stopped",
                "busybox:latest",
                "sh",
                "-c",
                "sleep 3600",
            ]
        );
        // Creating something is not destroying something, so it carries no typed confirmation: a
        // guard asked for where nothing is lost is the reflex that waves through the one that
        // matters.
        assert!(!preview.destructive);
        assert_eq!(preview.required_confirmation, None);
        assert!(preview.removals.is_empty());
    }

    /// `create` rather than `run`, and a port the operator did not publish stays unpublished: a
    /// host port Fathom chose would bind something nobody asked to bind.
    #[test]
    fn an_undetached_create_is_a_create_and_an_unpublished_port_is_only_exposed() {
        let request = OperationRequest::CreateContainer {
            spec: ContainerSpec {
                image: "busybox".to_owned(),
                name: None,
                ports: vec![PublishedPort {
                    container_port: 5432,
                    host_port: None,
                    host_ip: None,
                    protocol: "tcp".to_owned(),
                }],
                env: vec![],
                volumes: vec![],
                networks: vec![],
                command: vec![],
                restart_policy: None,
                detach: false,
            },
        };
        let containers: Vec<Container> = vec![];
        let preview = preview_of(&request, &ctx(&containers, None));

        assert_eq!(
            preview.command,
            vec!["docker", "create", "--expose", "5432", "busybox"]
        );
    }

    /// The context path is last and positional, so a context that begins with a dash is still the
    /// context and not a flag the builder half-reads.
    #[test]
    fn a_build_names_its_tag_first_and_its_context_last() {
        let request = OperationRequest::BuildImage {
            spec: ImageBuildSpec {
                context_path: "/srv/app".to_owned(),
                dockerfile: Some("/srv/app/docker/Dockerfile".to_owned()),
                tag: "shop-web:dev".to_owned(),
                build_args: vec![("VERSION".to_owned(), "1.2.3".to_owned())],
                target: Some("runtime".to_owned()),
                no_cache: true,
            },
        };
        let containers: Vec<Container> = vec![];
        let preview = preview_of(&request, &ctx(&containers, None));

        assert_eq!(
            preview.command,
            vec![
                "docker",
                "build",
                "-t",
                "shop-web:dev",
                "-f",
                "/srv/app/docker/Dockerfile",
                "--target",
                "runtime",
                "--no-cache",
                "--build-arg",
                "VERSION=1.2.3",
                "/srv/app",
            ]
        );
        assert!(!preview.destructive);
    }

    #[test]
    fn a_network_creation_spells_out_its_addressing_and_its_labels() {
        let request = OperationRequest::CreateNetwork {
            spec: NetworkSpec {
                name: "shop_back".to_owned(),
                driver: Some("bridge".to_owned()),
                subnet: Some("172.30.0.0/16".to_owned()),
                gateway: Some("172.30.0.1".to_owned()),
                internal: true,
                labels: vec![("owner".to_owned(), "shop".to_owned())],
            },
        };
        let containers: Vec<Container> = vec![];
        let preview = preview_of(&request, &ctx(&containers, None));

        assert_eq!(
            preview.command,
            vec![
                "docker",
                "network",
                "create",
                "--driver",
                "bridge",
                "--subnet",
                "172.30.0.0/16",
                "--gateway",
                "172.30.0.1",
                "--internal",
                "--label",
                "owner=shop",
                "shop_back",
            ]
        );
    }

    #[test]
    fn a_volume_creation_is_its_driver_its_labels_and_its_name() {
        let request = OperationRequest::CreateVolume {
            spec: VolumeSpec {
                name: "shop_db_data".to_owned(),
                driver: Some("local".to_owned()),
                labels: vec![("owner".to_owned(), "shop".to_owned())],
            },
        };
        let containers: Vec<Container> = vec![];
        let preview = preview_of(&request, &ctx(&containers, None));

        assert_eq!(
            preview.command,
            vec![
                "docker",
                "volume",
                "create",
                "--driver",
                "local",
                "--label",
                "owner=shop",
                "shop_db_data",
            ]
        );
    }

    /// A creation has no scope. The gate is re-checked at issue time against whatever the preview
    /// named, and demanding a grant over a project that does not exist yet would make the first
    /// container of a new project impossible to create.
    #[test]
    fn a_creation_is_scoped_to_no_existing_project() {
        let request = OperationRequest::CreateVolume {
            spec: VolumeSpec {
                name: "fresh".to_owned(),
                driver: None,
                labels: vec![],
            },
        };
        let containers: Vec<Container> = vec![];
        let stored = build("prev-1".to_owned(), &request, &ctx(&containers, None))
            .expect("a creation is previewable");
        assert_eq!(stored.scope, None);
        assert!(stored.expected.is_empty());
    }

    /// `up` is detached and acts on the project's own file, named explicitly: the daemon's idea of
    /// the project's file has nothing to do with Fathom's working directory.
    #[test]
    fn a_project_up_is_detached_and_names_the_file_the_project_declared() {
        let containers = vec![
            container("bbbb000000002", "shop-web", Some("shop")),
            container("aaaa000000001", "shop-db", Some("shop")),
        ];
        let request = OperationRequest::ProjectLifecycle {
            project: "shop".to_owned(),
            action: ProjectAction::Up,
        };
        let preview = preview_of(&request, &ctx(&containers, Some("/srv/shop/compose.yaml")));

        assert_eq!(
            preview.command,
            vec![
                "docker",
                "compose",
                "-p",
                "shop",
                "-f",
                "/srv/shop/compose.yaml",
                "up",
                "-d",
            ]
        );
        assert!(!preview.destructive);
        assert_eq!(preview.required_confirmation, None);
    }

    /// A `down` is a removal wearing a lifecycle verb's clothes, and is classified as one: it
    /// takes the project's containers with it, so it earns the full guard and the confirmation is
    /// the project's own name rather than a word that reads the same every time.
    #[test]
    fn a_project_down_is_destructive_and_asks_for_the_projects_own_name() {
        let containers = vec![
            container("bbbb000000002", "shop-web", Some("shop")),
            container("aaaa000000001", "shop-db", Some("shop")),
        ];
        let request = OperationRequest::ProjectLifecycle {
            project: "shop".to_owned(),
            action: ProjectAction::Down,
        };
        let preview = preview_of(&request, &ctx(&containers, Some("/srv/shop/compose.yaml")));

        assert!(preview.destructive);
        assert_eq!(preview.required_confirmation.as_deref(), Some("shop"));
        assert_eq!(preview.blast_radius, vec!["shop".to_owned()]);
        // Every container named, in the order the preview reads them, never counted (FR-023).
        let removed: Vec<&str> = preview
            .removals
            .iter()
            .map(|r| r.object.name.as_str())
            .collect();
        assert_eq!(removed, vec!["shop-db", "shop-web"]);
        assert!(
            preview
                .command
                .iter()
                .all(|arg| arg != "-v" && arg != "--volumes"),
            "a down must not take the project's volumes with it unless asked"
        );
    }

    /// FR-033: no readable file, no project operation — and the reason says plainly that Fathom
    /// has not touched the file (Principle VI).
    #[test]
    fn a_project_with_no_definition_file_is_refused_with_a_reason_and_no_rewrite() {
        let containers = vec![container("aaaa000000001", "shop-web", Some("shop"))];
        let request = OperationRequest::ProjectLifecycle {
            project: "shop".to_owned(),
            action: ProjectAction::Restart,
        };
        let err = build("prev-1".to_owned(), &request, &ctx(&containers, None))
            .expect_err("no file, no preview");
        assert_eq!(err, NotPreviewable::NoDefinitionFile("shop".to_owned()));
        assert!(err.detail().contains("has not changed the file"));
    }
}
