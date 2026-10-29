//! Detection over the runtime snapshot, and how answers match (T069).

use std::collections::BTreeMap;
use std::sync::Mutex;

use depths_core::plan::RunStep;
use depths_core::ports::{
    CancelToken, OutputSink, ProjectContainers, RuntimeError, RuntimeSnapshot, StepOutcome,
    Validation,
};
use depths_core::security::{Inspect, InspectConfig};
use depths_protocol::{AdoptionState, Container, Provider};

use super::*;
use crate::updates::store::StoredActor;

/// `(id, image as written, image id, labels, state)`.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    Vec<(&'static str, &'static str)>,
    RuntimeContainerState,
);

#[derive(Default)]
struct Fake {
    rows: Vec<Row>,
    /// Every argv it was asked to run: detection runs none.
    ran: Mutex<Vec<String>>,
}

fn container(r: &Row, project: Option<&str>) -> Container {
    Container {
        id: r.0.into(),
        name: format!("/{}", r.0.trim_end_matches("-id")),
        image: r.1.into(),
        state: r.4.clone(),
        health: None,
        status: String::new(),
        created_at: String::new(),
        command: String::new(),
        ports: vec![],
        project: project.map(str::to_owned),
        service: project.map(|_| "svc".to_owned()),
        cluster: None,
        adoption: AdoptionState::NotAdopted,
    }
}

impl ContainerRuntime for Fake {
    fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
        let mut snap = RuntimeSnapshot::default();
        // The first row is in a stack, the rest are standalone: both count.
        if let Some(first) = self.rows.first() {
            snap.projects.insert(
                "ops".into(),
                ProjectContainers {
                    working_dir: None,
                    containers: vec![container(first, Some("ops"))],
                },
            );
        }
        snap.standalone = self
            .rows
            .iter()
            .skip(1)
            .map(|r| container(r, None))
            .collect();
        Ok(snap)
    }
    fn run_step(
        &self,
        step: &RunStep,
        _: Option<&[u8]>,
        _: &dyn OutputSink,
        _: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError> {
        self.ran.lock().unwrap().push(step.argv.join(" "));
        Err(RuntimeError::Unsupported)
    }
    fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
        Ok(Validation::Valid)
    }
    fn inspect_security(&self, ids: &[String]) -> Result<Vec<Inspect>, RuntimeError> {
        Ok(self
            .rows
            .iter()
            .filter(|r| ids.iter().any(|i| i == r.0))
            .map(|r| Inspect {
                id: r.0.into(),
                image: r.2.into(),
                config: InspectConfig {
                    image: r.1.into(),
                    labels: r
                        .3
                        .iter()
                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                        .collect::<BTreeMap<_, _>>(),
                    ..InspectConfig::default()
                },
                ..Inspect::default()
            })
            .collect())
    }
}

const RUN: RuntimeContainerState = RuntimeContainerState::Running;

fn names(seen: &[Seen]) -> Vec<&str> {
    seen.iter().map(|s| s.container_id.as_str()).collect()
}

#[test]
fn watchtower_is_found_by_image_or_by_its_marker_label_and_only_while_running() {
    let fake = Fake {
        rows: vec![
            (
                "a-id",
                "containrrr/watchtower:latest",
                "sha256:a",
                vec![],
                RUN,
            ),
            (
                "b-id",
                "ghcr.io/containrrr/watchtower@sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "sha256:b",
                vec![],
                RUN,
            ),
            (
                "c-id",
                "myorg/watchtower-dashboard",
                "sha256:c",
                vec![],
                RUN,
            ),
            (
                "d-id",
                "custom/updater",
                "sha256:d",
                vec![("com.centurylinklabs.watchtower", "true")],
                RUN,
            ),
            (
                "e-id",
                "containrrr/watchtower",
                "sha256:e",
                vec![],
                RuntimeContainerState::Exited,
            ),
            ("f-id", "nickfedor/watchtower:1", "sha256:f", vec![], RUN),
            (
                "g-id",
                "ghcr.io/nicholas-fedor/watchtower",
                "sha256:g",
                vec![],
                RUN,
            ),
            ("h-id", "beatkind/watchtower", "sha256:h", vec![], RUN),
            (
                "i-id",
                "nginx",
                "sha256:i",
                vec![("com.centurylinklabs.watchtower.enable", "false")],
                RUN,
            ),
        ],
        ..Fake::default()
    };
    let seen = running(&fake);
    assert_eq!(
        names(&seen),
        ["a-id", "b-id", "d-id", "f-id", "g-id", "h-id"]
    );
    assert_eq!(seen[0].image_id, "sha256:a");
    assert_eq!(seen[0].name, "a", "the leading slash is dropped");
    assert!(
        fake.ran.lock().unwrap().is_empty(),
        "Depths never touches it (FR-021)"
    );
}

fn ack(id: &str, image: &str, answer: WatchtowerAnswer) -> WatchtowerAck {
    WatchtowerAck {
        container_id: id.into(),
        image_id: image.into(),
        name: "watchtower".into(),
        answer,
        by: StoredActor {
            provider: Provider::Github,
            subject: "admin".into(),
            username: "admin".into(),
        },
        at: "2026-10-01T00:00:00Z".into(),
    }
}

fn seen(id: &str, image_id: &str) -> Seen {
    Seen {
        container_id: id.into(),
        image_id: image_id.into(),
        name: format!("wt-{id}"),
        image: "containrrr/watchtower".into(),
    }
}

#[test]
fn an_answer_matches_container_and_image_so_a_recreated_watchtower_asks_again() {
    let acks = [ack("a", "sha256:1", WatchtowerAnswer::MonitorOnly)];
    assert_eq!(
        answer(&seen("a", "sha256:1"), &acks),
        Some(WatchtowerAnswer::MonitorOnly)
    );
    assert_eq!(
        answer(&seen("a", "sha256:2"), &acks),
        None,
        "upgraded image"
    );
    assert_eq!(
        answer(&seen("b", "sha256:1"), &acks),
        None,
        "recreated container"
    );
}

#[test]
fn auto_stays_paused_while_any_running_watchtower_has_no_answer() {
    let both = [seen("a", "sha256:1"), seen("b", "sha256:1")];
    let one = [ack("a", "sha256:1", WatchtowerAnswer::Stopped)];
    assert_eq!(
        pause(&both, &one),
        Some(AutoPause::WatchtowerRunning {
            containers: vec!["wt-b".into()]
        })
    );
    let all = [
        ack("a", "sha256:1", WatchtowerAnswer::Stopped),
        ack("b", "sha256:1", WatchtowerAnswer::MonitorOnly),
    ];
    assert_eq!(pause(&both, &all), None);
    assert_eq!(pause(&[], &[]), None);
    let shown = view(&both, &one);
    assert_eq!(shown[0].acknowledged, Some(WatchtowerAnswer::Stopped));
    assert_eq!(shown[1].acknowledged, None);
}
