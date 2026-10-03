//! The second half of every mutation: run exactly what the preview showed.
//!
//! The one rule this module exists to keep: **the argv that runs is the argv the preview carried,
//! element for element.** It is not rebuilt from the request here, it is read out of the stored
//! preview, because a rebuild is a second implementation of the same derivation and two
//! implementations drift. `tests::issued_argv_is_byte_identical_to_the_preview` is the assertion
//! that makes the preview a guarantee instead of a decoration; without it everything the operator
//! reads before confirming is a claim nobody checks.
//!
//! The second rule: a multi-object operation reports one outcome per object (FR-022). A single
//! `stop a b c` that fails on `b` exits non-zero as a whole, and reporting that as one aggregate
//! failure would hide that `a` and `c` did stop. So outcomes are attributed from the runtime's own
//! output, and the error text is its stderr verbatim (FR-021) — unwrapped, unprefixed, not
//! rephrased into something friendlier that no longer matches what a terminal would have said.

use crate::protocol::v1::{
    Container, Discrepancy, ObjectOutcome, ObjectRef, OperationOutcome, OperationRefusal,
    OutcomeStatus, RemovableObject, RuntimeContainerState,
};

use super::super::kind::{CmdOutput, RunError, Runner};
use super::preview::{Expected, StoredPreview};

/// Runs a stored preview.
///
/// Takes the preview rather than a request, so there is no path through this function that could
/// run something a preview did not describe.
pub fn issue(runner: &dyn Runner, stored: &StoredPreview) -> Result<OperationOutcome, RunError> {
    let command = &stored.preview.command;
    // A preview with no program is not a thing `build` can produce; treating it as a runtime
    // error rather than panicking keeps a corrupt store from taking the backend down.
    let Some((bin, args)) = command.split_first() else {
        return Err(RunError::Other(
            "the stored preview carried no command".to_owned(),
        ));
    };
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner.run(bin, &borrowed)?;

    let objects = attribute(&stored.targets, &output);
    Ok(OperationOutcome {
        preview_id: stored.preview.preview_id.clone(),
        // The same vector that was displayed, not a reconstruction of it.
        command: command.clone(),
        overall: OperationOutcome::overall_from(&objects),
        objects,
        // Filled by `verify`, which re-reads the inventory. An operation that has not been
        // verified yet reports no discrepancies rather than reporting that there were none.
        discrepancies: vec![],
    })
}

/// Turns one command's output into one outcome per target.
///
/// The runtime names what it handled: `docker stop a b` prints the ids it stopped on stdout, one
/// per line, and writes a line to stderr for each one it could not. So presence on stdout is the
/// success signal, and a target named in neither stream was not reached at all — `Skipped`, which
/// is a different thing from having failed and must not be reported as one.
///
/// An operation with no enumerable targets (a creation) has exactly one outcome, taken from the
/// exit status: there is no object identity to attribute it to until it exists.
fn attribute(targets: &[ObjectRef], output: &CmdOutput) -> Vec<ObjectOutcome> {
    if targets.is_empty() {
        return vec![ObjectOutcome {
            object: ObjectRef {
                class: crate::protocol::v1::ObjectClass::Container,
                id: String::new(),
                name: String::new(),
            },
            status: if output.success {
                OutcomeStatus::Succeeded
            } else {
                OutcomeStatus::Failed
            },
            error: (!output.success).then(|| output.stderr.trim_end().to_owned()),
        }];
    }

    targets
        .iter()
        .map(|target| {
            if mentions(&output.stdout, target) {
                ObjectOutcome {
                    object: target.clone(),
                    status: OutcomeStatus::Succeeded,
                    error: None,
                }
            } else if let Some(line) = error_line(&output.stderr, target) {
                ObjectOutcome {
                    object: target.clone(),
                    status: OutcomeStatus::Failed,
                    // Verbatim, including the runtime's own wording for "no such container",
                    // which is the sentence an operator can search for.
                    error: Some(line),
                }
            } else if output.success {
                // The whole command succeeded and this target went unmentioned: some runtimes
                // print nothing for an object already in the requested state. Succeeded is the
                // honest reading, because nothing failed and the state asked for is the state.
                ObjectOutcome {
                    object: target.clone(),
                    status: OutcomeStatus::Succeeded,
                    error: None,
                }
            } else {
                ObjectOutcome {
                    object: target.clone(),
                    status: OutcomeStatus::Skipped,
                    error: None,
                }
            }
        })
        .collect()
}

/// Whether a stream names this object, by full id, short id or name.
///
/// All three because the runtime echoes back whichever form it was given, and the short id is a
/// prefix of the full one.
fn mentions(stream: &str, target: &ObjectRef) -> bool {
    stream.lines().map(str::trim).any(|line| {
        line == target.id
            || line == target.name
            || (!line.is_empty() && line.len() >= 12 && target.id.starts_with(line))
    })
}

/// The stderr line that is about this object, if any.
fn error_line(stderr: &str, target: &ObjectRef) -> Option<String> {
    stderr
        .lines()
        .find(|line| {
            !target.id.is_empty() && line.contains(&target.id)
                || !target.name.is_empty() && line.contains(&target.name)
        })
        .map(|line| line.trim().to_owned())
}

/// Whether the typed confirmation satisfies a destructive preview.
///
/// Compared against the string the preview itself demanded, not against a constant like "DELETE":
/// a confirmation that is the same words every time is a reflex, and a reflex confirms the wrong
/// object as readily as the right one.
pub fn confirmation_ok(
    stored: &StoredPreview,
    typed: Option<&str>,
) -> Result<(), OperationRefusal> {
    match &stored.preview.required_confirmation {
        None => Ok(()),
        Some(required) => {
            if typed.map(str::trim) == Some(required.as_str()) {
                Ok(())
            } else {
                Err(OperationRefusal::ConfirmationMismatch)
            }
        }
    }
}

/// A reading of the runtime taken *after* the operation ran.
///
/// Separate from the preview's reading on purpose: FR-023 is satisfied by asking the runtime
/// again, not by trusting what it said while it was working.
pub struct AfterReading<'a> {
    pub containers: &'a [Container],
    /// Images, volumes and networks together — a discrepancy is about an identity, and which pool
    /// it came from does not change the question.
    pub objects: &'a [RemovableObject],
}

/// Compares what the operation claimed with what the runtime now shows.
///
/// Both directions are reported. An object the outcome called succeeded that is still exactly as
/// it was is the dangerous case — the operator would otherwise read a green tick and move on —
/// but an object reported as failed that is nonetheless gone is also a discrepancy: the operation
/// happened, and a UI that says it did not is lying in the other direction.
///
/// Objects the command never reached (`Skipped`) are not checked. Nothing was claimed about them.
pub fn verify(outcome: &mut OperationOutcome, stored: &StoredPreview, after: &AfterReading<'_>) {
    for (object, want) in &stored.expected {
        let Some(reported) = outcome
            .objects
            .iter()
            .find(|o| o.object.id == object.id)
            .map(|o| o.status.clone())
        else {
            continue;
        };
        if reported == OutcomeStatus::Skipped {
            continue;
        }
        let observed = observe(object, after);
        let claimed = match (&reported, want) {
            (OutcomeStatus::Succeeded, Expected::Absent) => "gone".to_owned(),
            (_, Expected::Absent) => "still present".to_owned(),
            (OutcomeStatus::Succeeded, Expected::ContainerState(s)) => state_word(s),
            (_, Expected::ContainerState(s)) => format!("not {}", state_word(s)),
        };
        if claimed != observed {
            outcome.discrepancies.push(Discrepancy {
                object: object.clone(),
                expected: claimed,
                observed,
            });
        }
    }
}

/// What the runtime now shows for one object, in the same words `verify` compares against.
fn observe(object: &ObjectRef, after: &AfterReading<'_>) -> String {
    if let Some(c) = after.containers.iter().find(|c| c.id == object.id) {
        return state_word(&c.state);
    }
    if after.objects.iter().any(|o| o.id == object.id) {
        return "still present".to_owned();
    }
    "gone".to_owned()
}

fn state_word(state: &RuntimeContainerState) -> String {
    match state {
        RuntimeContainerState::Created => "created".to_owned(),
        RuntimeContainerState::Running => "running".to_owned(),
        RuntimeContainerState::Paused => "paused".to_owned(),
        RuntimeContainerState::Restarting => "restarting".to_owned(),
        RuntimeContainerState::Removing => "removing".to_owned(),
        RuntimeContainerState::Exited => "exited".to_owned(),
        RuntimeContainerState::Dead => "dead".to_owned(),
        // The runtime's own word, not "unknown": FR-021 means its vocabulary reaches the operator
        // even when Fathom has no name for it.
        RuntimeContainerState::Unknown(word) => word.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::container_runtime::Cli;
    use crate::backend::container_runtime::preview::{self, PreviewContext};
    use crate::protocol::v1::{
        Container, LifecycleAction, ObjectClass, OperationRequest, OperationScope,
        RuntimeContainerState,
    };
    use std::sync::Mutex;

    /// Records what it was asked to run and answers with a canned result.
    struct Recorder {
        reply: CmdOutput,
        seen: Mutex<Vec<Vec<String>>>,
    }

    impl Recorder {
        fn new(success: bool, stdout: &str, stderr: &str) -> Self {
            Self {
                reply: CmdOutput {
                    success,
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                },
                seen: Mutex::new(vec![]),
            }
        }

        fn argv(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .first()
                .cloned()
                .unwrap_or_default()
        }
    }

    impl Runner for Recorder {
        fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
            let mut argv = vec![bin.to_owned()];
            argv.extend(args.iter().map(|a| (*a).to_owned()));
            self.seen.lock().unwrap().push(argv);
            Ok(self.reply.clone())
        }
    }

    fn container(id: &str, name: &str, project: Option<&str>) -> Container {
        Container {
            id: id.to_owned(),
            name: name.to_owned(),
            image: "busybox".to_owned(),
            state: RuntimeContainerState::Running,
            health: None,
            status: "Up 1 minute".to_owned(),
            created_at: "2026-10-02T00:00:00Z".to_owned(),
            command: "sh".to_owned(),
            ports: vec![],
            project: project.map(str::to_owned),
            service: None,
            cluster: None,
            adoption: crate::protocol::v1::AdoptionState::NotAdopted,
        }
    }

    fn four_member_project() -> Vec<Container> {
        vec![
            container("aaaa000000001", "proj-a-1", Some("proj")),
            container("bbbb000000002", "proj-b-1", Some("proj")),
            container("cccc000000003", "proj-c-1", Some("proj")),
            container("dddd000000004", "proj-d-1", Some("proj")),
        ]
    }

    /// No networks unless a test says otherwise; `compose down` is the only path that reads them.
    const NO_NETWORKS: &[crate::protocol::v1::RemovableObject] = &[];

    /// The removal paths are previewed in `blast_radius`'s own tests; these fixtures exercise the
    /// lifecycle half, so there is nothing here for a removal to reach and no project is adopted.
    const NO_OBJECTS: &[crate::protocol::v1::RemovableObject] = &[];
    const NO_GRANTS: &[String] = &[];

    fn ctx<'a>(containers: &'a [Container]) -> PreviewContext<'a> {
        PreviewContext {
            cli: Cli::docker(),
            containers,
            images: NO_OBJECTS,
            volumes: NO_OBJECTS,
            networks: NO_NETWORKS,
            adopted: NO_GRANTS,
            definition_file: Some("/srv/proj/compose.yaml"),
            active_context: None,
            as_of: "2026-10-02T12:00:00Z",
        }
    }

    /// **The argv invariant.** Without this, the preview is a story told about an operation rather
    /// than the operation itself: the UI could display one command and the backend run another,
    /// and nothing would notice. Everything FR-018 promises rests here.
    #[test]
    fn issued_argv_is_byte_identical_to_the_preview() {
        let containers = four_member_project();
        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Project {
                project: "proj".to_owned(),
            },
        };
        let stored = preview::build("prev-1".to_owned(), &request, &ctx(&containers)).unwrap();
        let shown = stored.preview.command.clone();

        let runner = Recorder::new(
            true,
            "aaaa000000001\nbbbb000000002\ncccc000000003\ndddd000000004\n",
            "",
        );
        let outcome = issue(&runner, &stored).unwrap();

        assert_eq!(runner.argv(), shown, "what ran differs from what was shown");
        assert_eq!(
            outcome.command, shown,
            "the outcome reports a different command"
        );
        assert_eq!(
            shown,
            vec![
                "docker",
                "stop",
                "aaaa000000001",
                "bbbb000000002",
                "cccc000000003",
                "dddd000000004"
            ]
        );
    }

    #[test]
    fn a_creation_runs_the_argv_the_form_produced_and_no_shell() {
        let containers: Vec<Container> = vec![];
        let request = OperationRequest::CreateContainer {
            spec: crate::protocol::v1::ContainerSpec {
                image: "busybox".to_owned(),
                name: Some("probe".to_owned()),
                ports: vec![],
                // A value with a semicolon and a space in it: one argv element, never two, and
                // nothing in this path would interpret it even if it were.
                env: vec![("MSG".to_owned(), "hello; rm -rf /".to_owned())],
                volumes: vec![],
                networks: vec![],
                command: vec!["sleep".to_owned(), "1".to_owned()],
                restart_policy: None,
                detach: true,
            },
        };
        let stored = preview::build("prev-2".to_owned(), &request, &ctx(&containers)).unwrap();
        let runner = Recorder::new(true, "abc123\n", "");
        issue(&runner, &stored).unwrap();

        let argv = runner.argv();
        assert_eq!(argv, stored.preview.command);
        assert!(argv.contains(&"MSG=hello; rm -rf /".to_owned()), "{argv:?}");
        assert_eq!(argv.iter().filter(|a| a.contains("rm -rf")).count(), 1);
    }

    /// FR-022: four members, one failure, four outcomes — and an overall status that is not
    /// success. An aggregate verdict would tell the operator "failed" and leave them to work out
    /// which three of the four did in fact stop.
    #[test]
    fn one_failed_member_of_four_reports_four_outcomes_and_does_not_claim_success() {
        let containers = four_member_project();
        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Project {
                project: "proj".to_owned(),
            },
        };
        let stored = preview::build("prev-3".to_owned(), &request, &ctx(&containers)).unwrap();
        let runner = Recorder::new(
            false,
            "aaaa000000001\ncccc000000003\ndddd000000004\n",
            "Error response from daemon: cannot stop container bbbb000000002: permission denied\n",
        );
        let outcome = issue(&runner, &stored).unwrap();

        assert_eq!(outcome.objects.len(), 4);
        assert_ne!(outcome.overall, OutcomeStatus::Succeeded);
        assert_eq!(outcome.overall, OutcomeStatus::Failed);

        let failed: Vec<&ObjectOutcome> = outcome
            .objects
            .iter()
            .filter(|o| o.status == OutcomeStatus::Failed)
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].object.id, "bbbb000000002");
        assert_eq!(
            failed[0].error.as_deref(),
            Some(
                "Error response from daemon: cannot stop container bbbb000000002: permission denied"
            ),
            "the runtime's own sentence, unwrapped"
        );
        assert_eq!(
            outcome
                .objects
                .iter()
                .filter(|o| o.status == OutcomeStatus::Succeeded)
                .count(),
            3
        );
    }

    /// A container removed between preview and issue: the runtime's "No such container" is what
    /// the operator sees, not Fathom's paraphrase of it.
    #[test]
    fn a_target_that_disappeared_surfaces_the_runtimes_own_words() {
        let containers = vec![container("aaaa000000001", "gone", None)];
        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Container {
                id: "aaaa000000001".to_owned(),
            },
        };
        let stored = preview::build("prev-4".to_owned(), &request, &ctx(&containers)).unwrap();
        let runner = Recorder::new(
            false,
            "",
            "Error response from daemon: No such container: aaaa000000001\n",
        );
        let outcome = issue(&runner, &stored).unwrap();

        assert_eq!(outcome.objects.len(), 1);
        assert_eq!(outcome.objects[0].status, OutcomeStatus::Failed);
        assert_eq!(
            outcome.objects[0].error.as_deref(),
            Some("Error response from daemon: No such container: aaaa000000001")
        );
    }

    /// A member the runtime named in neither stream of a failed command was not reached. Calling
    /// that a failure would invent an error the runtime never reported.
    #[test]
    fn an_unmentioned_target_of_a_failed_command_is_skipped_not_failed() {
        let containers = four_member_project();
        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Project {
                project: "proj".to_owned(),
            },
        };
        let stored = preview::build("prev-5".to_owned(), &request, &ctx(&containers)).unwrap();
        let runner = Recorder::new(
            false,
            "aaaa000000001\n",
            "Error response from daemon: cannot stop container bbbb000000002: device busy\n",
        );
        let outcome = issue(&runner, &stored).unwrap();

        let by_id = |id: &str| {
            outcome
                .objects
                .iter()
                .find(|o| o.object.id == id)
                .unwrap()
                .status
                .clone()
        };
        assert_eq!(by_id("aaaa000000001"), OutcomeStatus::Succeeded);
        assert_eq!(by_id("bbbb000000002"), OutcomeStatus::Failed);
        assert_eq!(by_id("cccc000000003"), OutcomeStatus::Skipped);
        assert_eq!(by_id("dddd000000004"), OutcomeStatus::Skipped);
    }

    #[test]
    fn a_project_lifecycle_acts_on_the_projects_own_file() {
        let containers = four_member_project();
        let request = OperationRequest::ProjectLifecycle {
            project: "proj".to_owned(),
            action: crate::protocol::v1::ProjectAction::Up,
        };
        let stored = preview::build("prev-6".to_owned(), &request, &ctx(&containers)).unwrap();
        assert_eq!(
            stored.preview.command,
            vec![
                "docker",
                "compose",
                "-p",
                "proj",
                "-f",
                "/srv/proj/compose.yaml",
                "up",
                "-d"
            ]
        );
    }

    /// FR-023 on the one destructive path US2 can already enumerate: `compose down` names every
    /// container and every network it will take, and demands the project's own name typed back.
    /// A `down` with an empty removal list would be the exact "trust me" dialog the spec forbids.
    #[test]
    fn compose_down_names_its_containers_and_networks_and_asks_for_the_project_name() {
        use crate::protocol::v1::{ObjectClass, RemovableObject};

        let containers = four_member_project();
        let networks = vec![
            RemovableObject {
                class: ObjectClass::Network,
                id: "net0000000001".to_owned(),
                name: "proj_default".to_owned(),
                size_bytes: None,
                tagged: None,
                created_at: None,
                project: Some("proj".to_owned()),
                referenced_by: vec![],
            },
            // Another project's network on the same shared daemon: it must not appear.
            RemovableObject {
                class: ObjectClass::Network,
                id: "net0000000002".to_owned(),
                name: "other_default".to_owned(),
                size_bytes: None,
                tagged: None,
                created_at: None,
                project: Some("other".to_owned()),
                referenced_by: vec![],
            },
        ];
        let mut context = ctx(&containers);
        context.networks = &networks;

        let request = OperationRequest::ProjectLifecycle {
            project: "proj".to_owned(),
            action: crate::protocol::v1::ProjectAction::Down,
        };
        let stored = preview::build("prev-down".to_owned(), &request, &context).unwrap();
        let preview = &stored.preview;

        assert!(preview.destructive, "compose down removes things");
        assert_eq!(
            preview.required_confirmation.as_deref(),
            Some("proj"),
            "a destructive project operation must be typed back by name"
        );
        assert_eq!(preview.blast_radius, vec!["proj".to_owned()]);

        let named: Vec<&str> = preview
            .removals
            .iter()
            .map(|r| r.object.name.as_str())
            .collect();
        assert_eq!(
            named,
            vec![
                "proj-a-1",
                "proj-b-1",
                "proj-c-1",
                "proj-d-1",
                "proj_default"
            ],
            "every member and the project's own network, and nothing of another project's"
        );
        assert!(
            preview
                .removals
                .iter()
                .all(|r| r.project.as_deref() == Some("proj")),
            "a removal line that does not say whose it is cannot be read for blast radius"
        );
    }

    /// FR-033: no readable file, no project-level operation — and the reason is said out loud.
    #[test]
    fn a_project_with_no_definition_file_gets_no_preview_and_a_reason() {
        let containers = four_member_project();
        let mut context = ctx(&containers);
        context.definition_file = None;
        let request = OperationRequest::ProjectLifecycle {
            project: "proj".to_owned(),
            action: crate::protocol::v1::ProjectAction::Down,
        };
        let err = preview::build("prev-7".to_owned(), &request, &context).unwrap_err();
        assert_eq!(
            err,
            preview::NotPreviewable::NoDefinitionFile("proj".to_owned())
        );
        assert!(
            err.detail().contains("has not changed the file"),
            "{}",
            err.detail()
        );
    }

    /// FR-027. A removal has no preview until its blast radius can be enumerated, and an
    /// operation with no preview is not offered at all.
    ///
    /// This context carries no images, so the id names nothing the operator can be looking at.
    /// The refusal says which id, because "the list you are reading is older than you think" is
    /// an answer and "could not enumerate" is not.
    #[test]
    fn a_removal_is_refused_rather_than_previewed_with_an_empty_blast_radius() {
        let containers = four_member_project();
        let request = OperationRequest::Remove {
            objects: vec![ObjectRef {
                class: ObjectClass::Image,
                id: "sha256:abc".to_owned(),
                name: "busybox:latest".to_owned(),
            }],
        };
        assert_eq!(
            preview::build("prev-8".to_owned(), &request, &ctx(&containers)).err(),
            Some(preview::NotPreviewable::UnknownTarget(
                "sha256:abc".to_owned()
            ))
        );
    }

    /// FR-025 again, at the other end: exclusion is total, so a selection in which every object
    /// is excluded has nothing left in it.
    ///
    /// That selection is not previewed as a removal of nothing. A typed confirmation over a no-op
    /// is how an operator learns the guard is noise, and the sentence they can act on names the
    /// project to adopt. Before this, the empty set reached `removal_argv` and came back as
    /// "objects of more than one kind", which is not what happened.
    #[test]
    fn a_selection_every_object_of_which_is_excluded_is_not_previewed_as_removing_nothing() {
        let containers = four_member_project();
        let images = vec![RemovableObject {
            class: ObjectClass::Image,
            id: "sha256:owned".to_owned(),
            name: "blog/web:1".to_owned(),
            size_bytes: Some(4096),
            tagged: Some(true),
            created_at: None,
            project: Some("blog".to_owned()),
            referenced_by: vec![],
        }];
        let mut context = ctx(&containers);
        context.images = &images;
        let request = OperationRequest::Remove {
            objects: vec![ObjectRef {
                class: ObjectClass::Image,
                id: "sha256:owned".to_owned(),
                name: "blog/web:1".to_owned(),
            }],
        };
        let err = preview::build("prev-excluded".to_owned(), &request, &context).unwrap_err();
        assert_eq!(
            err,
            preview::NotPreviewable::AllExcluded(vec!["blog".to_owned()]),
            "the reason must name the project to adopt, not count what was skipped"
        );
    }

    /// A preview is good for one issue. Confirming the same button twice would run the operation
    /// twice against an inventory that has certainly moved in between.
    #[test]
    fn a_preview_can_only_be_issued_once() {
        let containers = four_member_project();
        let mut store = preview::PreviewStore::new();
        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Start,
            scope: OperationScope::Container {
                id: "aaaa000000001".to_owned(),
            },
        };
        let shown = store.take(&request, &ctx(&containers)).unwrap();
        assert!(store.get(&shown.preview_id).is_some());
        assert!(store.consume(&shown.preview_id).is_some());
        assert!(store.consume(&shown.preview_id).is_none());
    }

    #[test]
    fn a_stopping_verb_on_a_cluster_node_names_the_cluster_it_breaks() {
        let mut node = container("eeee000000005", "fathom-dev-control-plane", None);
        node.cluster = Some("fathom-dev".to_owned());
        let containers = vec![node];
        let mut context = ctx(&containers);
        context.active_context = Some("kind-fathom-dev");

        let request = OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Container {
                id: "eeee000000005".to_owned(),
            },
        };
        let stored = preview::build("prev-9".to_owned(), &request, &context).unwrap();
        assert_eq!(stored.preview.breaks_clusters.len(), 1);
        assert_eq!(stored.preview.breaks_clusters[0].cluster, "fathom-dev");
        assert!(stored.preview.breaks_clusters[0].connected);

        // Starting it back up breaks nothing, and must not say it does: a warning that fires on
        // the repair as well as the damage is a warning the operator learns to click through.
        let start = OperationRequest::Lifecycle {
            action: LifecycleAction::Start,
            scope: OperationScope::Container {
                id: "eeee000000005".to_owned(),
            },
        };
        let stored = preview::build("prev-10".to_owned(), &start, &context).unwrap();
        assert!(stored.preview.breaks_clusters.is_empty());
    }

    // ---------------------------------------------------------------------------
    // Post-verification (FR-023)

    fn removable(class: ObjectClass, name: &str) -> crate::protocol::v1::RemovableObject {
        crate::protocol::v1::RemovableObject {
            class,
            id: format!("{name}-id"),
            name: name.to_owned(),
            size_bytes: Some(1024),
            tagged: None,
            created_at: None,
            project: Some("shop".to_owned()),
            referenced_by: vec![],
        }
    }

    fn outcome_for(object: &ObjectRef, status: OutcomeStatus) -> OperationOutcome {
        let objects = vec![ObjectOutcome {
            object: object.clone(),
            status,
            error: None,
        }];
        OperationOutcome {
            preview_id: "prev-v".to_owned(),
            command: vec!["docker".to_owned(), "volume".to_owned(), "rm".to_owned()],
            overall: OperationOutcome::overall_from(&objects),
            objects,
            discrepancies: vec![],
        }
    }

    fn stored_expecting(object: &ObjectRef, want: preview::Expected) -> preview::StoredPreview {
        preview::StoredPreview {
            preview: crate::protocol::v1::OperationPreview {
                preview_id: "prev-v".to_owned(),
                request: OperationRequest::Remove {
                    objects: vec![object.clone()],
                },
                command: vec!["docker".to_owned(), "volume".to_owned(), "rm".to_owned()],
                destructive: true,
                total_size_bytes: None,
                removals: vec![],
                blast_radius: vec![],
                excluded: vec![],
                breaks_clusters: vec![],
                required_confirmation: Some(object.name.clone()),
                as_of: "2026-10-02T12:00:00Z".to_owned(),
            },
            targets: vec![object.clone()],
            scope: None,
            expected: vec![(object.clone(), want)],
        }
    }

    /// The dangerous case. Without this the operator reads a green tick over a volume that is
    /// still there, and the next thing they do is build on a deletion that never happened.
    #[test]
    fn a_removal_that_reported_success_and_changed_nothing_is_a_discrepancy() {
        let volume = removable(ObjectClass::Volume, "shop-data");
        let object = ObjectRef {
            class: volume.class,
            id: volume.id.clone(),
            name: volume.name.clone(),
        };
        let mut outcome = outcome_for(&object, OutcomeStatus::Succeeded);
        let stored = stored_expecting(&object, preview::Expected::Absent);
        let objects = [volume];

        verify(
            &mut outcome,
            &stored,
            &AfterReading {
                containers: &[],
                objects: &objects,
            },
        );
        assert_eq!(
            outcome.discrepancies,
            vec![Discrepancy {
                object,
                expected: "gone".to_owned(),
                observed: "still present".to_owned(),
            }]
        );
    }

    #[test]
    fn a_removal_the_runtime_actually_performed_reports_nothing() {
        let object = ObjectRef {
            class: ObjectClass::Volume,
            id: "shop-data-id".to_owned(),
            name: "shop-data".to_owned(),
        };
        let mut outcome = outcome_for(&object, OutcomeStatus::Succeeded);
        let stored = stored_expecting(&object, preview::Expected::Absent);

        verify(
            &mut outcome,
            &stored,
            &AfterReading {
                containers: &[],
                objects: &[],
            },
        );
        assert!(
            outcome.discrepancies.is_empty(),
            "{:?}",
            outcome.discrepancies
        );
    }

    /// The other direction, and just as much a lie: the runtime said it failed and the object is
    /// gone. Reporting only the first direction would leave this one silent.
    #[test]
    fn an_operation_that_reported_failure_and_happened_anyway_is_a_discrepancy() {
        let object = ObjectRef {
            class: ObjectClass::Volume,
            id: "shop-data-id".to_owned(),
            name: "shop-data".to_owned(),
        };
        let mut outcome = outcome_for(&object, OutcomeStatus::Failed);
        let stored = stored_expecting(&object, preview::Expected::Absent);

        verify(
            &mut outcome,
            &stored,
            &AfterReading {
                containers: &[],
                objects: &[],
            },
        );
        assert_eq!(
            outcome
                .discrepancies
                .first()
                .map(|d| (d.expected.clone(), d.observed.clone())),
            Some(("still present".to_owned(), "gone".to_owned()))
        );
    }

    /// A stop whose container is still running. The state is the runtime's own word, so the
    /// operator reads the same thing a terminal would have shown them.
    #[test]
    fn a_lifecycle_verb_whose_container_did_not_move_is_a_discrepancy() {
        let containers = vec![container("aaaa000000001", "proj-a-1", Some("proj"))];
        let object = ObjectRef {
            class: ObjectClass::Container,
            id: "aaaa000000001".to_owned(),
            name: "proj-a-1".to_owned(),
        };
        let mut outcome = outcome_for(&object, OutcomeStatus::Succeeded);
        let stored = stored_expecting(
            &object,
            preview::Expected::ContainerState(RuntimeContainerState::Exited),
        );

        verify(
            &mut outcome,
            &stored,
            &AfterReading {
                containers: &containers,
                objects: &[],
            },
        );
        assert_eq!(
            outcome
                .discrepancies
                .first()
                .map(|d| (d.expected.clone(), d.observed.clone())),
            Some(("exited".to_owned(), "running".to_owned()))
        );
    }

    /// Nothing was claimed about a target the command never reached, so there is nothing to
    /// contradict — and inventing a discrepancy here would bury the real ones.
    #[test]
    fn a_target_the_command_never_reached_is_not_verified() {
        let object = ObjectRef {
            class: ObjectClass::Volume,
            id: "shop-data-id".to_owned(),
            name: "shop-data".to_owned(),
        };
        let mut outcome = outcome_for(&object, OutcomeStatus::Skipped);
        let stored = stored_expecting(&object, preview::Expected::Absent);
        let objects = [removable(ObjectClass::Volume, "shop-data")];

        verify(
            &mut outcome,
            &stored,
            &AfterReading {
                containers: &[],
                objects: &objects,
            },
        );
        assert!(outcome.discrepancies.is_empty());
    }
}
