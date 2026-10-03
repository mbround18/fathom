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
    ClusterBreak, Container, ContainerSpec, ImageBuildSpec, LifecycleAction,
    NetworkSpec, ObjectClass, ObjectRef, OperationPreview, OperationRequest, OperationScope,
    ProjectAction, RemovableObject, RemovalLine, VolumeSpec,
};

use super::Cli;
use super::blast_radius::{Scene, resolve_objects, resolve_prune};

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
        }
    }
}

/// A stored preview: what the operator saw, plus what issuing it needs that they did not.
#[derive(Debug, Clone)]
pub struct StoredPreview {
    pub preview: OperationPreview,
    /// Every object the argv names, in argv order, so a per-object outcome can be attributed.
    pub targets: Vec<ObjectRef>,
    /// What the adoption gate must be satisfied for. `None` for an operation that creates
    /// something new: there is no existing project whose work it could disturb, and demanding a
    /// grant over a project that does not exist yet would make creation unreachable.
    pub scope: Option<OperationScope>,
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
        } => {
            let mut lines: Vec<RemovalLine> = targets
                .iter()
                .map(|object| RemovalLine {
                    object: object.clone(),
                    size_bytes: None,
                    project: Some(project.clone()),
                })
                .collect();
            lines.extend(
                ctx.networks
                    .iter()
                    .filter(|n| n.project.as_deref() == Some(project.as_str()))
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
            (
                lines,
                vec![project.clone()],
                vec![],
                // The project's own name, not a constant: a confirmation that is the same word
                // every time is a reflex, and a reflex confirms the wrong project as readily as
                // the right one.
                Some(project.clone()),
            )
        }
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
            breaks_clusters: cluster_breaks(ctx, request, &targets),
            required_confirmation,
            as_of: ctx.as_of.to_owned(),
        },
        targets,
        scope,
    })
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

/// Which clusters this operation takes down with it.
///
/// Only the verbs that actually stop a node count. `start` and `unpause` bring a node back, and
/// warning that they break the cluster would teach the operator to ignore the warning.
fn cluster_breaks(
    ctx: &PreviewContext<'_>,
    request: &OperationRequest,
    targets: &[ObjectRef],
) -> Vec<ClusterBreak> {
    let halting = match request {
        OperationRequest::Lifecycle { action, .. } => matches!(
            action,
            LifecycleAction::Stop
                | LifecycleAction::Kill
                | LifecycleAction::Pause
                | LifecycleAction::Restart
        ),
        OperationRequest::ProjectLifecycle { action, .. } => {
            matches!(action, ProjectAction::Down | ProjectAction::Restart)
        }
        OperationRequest::Remove { .. } => true,
        _ => false,
    };
    if !halting {
        return vec![];
    }

    let mut clusters: Vec<String> = targets
        .iter()
        .filter_map(|t| {
            ctx.containers
                .iter()
                .find(|c| c.id == t.id)
                .and_then(|c| c.cluster.clone())
        })
        .collect();
    clusters.sort();
    clusters.dedup();

    clusters
        .into_iter()
        .map(|cluster| ClusterBreak {
            // Stated separately from the breakage itself: "this breaks a cluster" and "this breaks
            // the cluster you are connected to" are different sentences, and collapsing them would
            // make the second one unsayable.
            connected: ctx
                .active_context
                .is_some_and(|c| c == format!("kind-{cluster}")),
            cluster,
        })
        .collect()
}
