//! Reading what the runtime has: containers, images, volumes and networks.
//!
//! One invocation per object class, each with an explicit field template rather than
//! `--format '{{json .}}'`. That is not a style choice. In list output the runtime renders all
//! labels as one flat comma-joined `k=v` string, and compose writes labels whose *values* contain
//! both delimiters — `com.docker.compose.depends_on=app:service_started:false,db:service_healthy:
//! false` is a single label. Splitting that string is ambiguous and misattributes containers to
//! the wrong project, which is exactly the mistake the whole feature exists to prevent. Targeted
//! `{{.Label "<key>"}}` projections ask the runtime to do the extraction, so the ambiguity never
//! reaches us.

use std::collections::BTreeMap;

use crate::wire::{
    AdoptionState, Container, ContainerHealth, ObjectClass, ObjectReference, PublishedPort,
    ReferenceKind, RemovableObject, RuntimeContainerState,
};

use crate::cli::{Cli, parse_size};
use crate::runner::{RunError, Runner};

/// Field separator for the list templates. Chosen because no runtime field contains it: ids,
/// names, images and label values are all constrained to characters that exclude it, and the
/// status line and command are rendered last-but-known-width positions in a fixed template.
const SEP: char = '\u{1f}';

const COMPOSE_PROJECT: &str = "com.docker.compose.project";
const COMPOSE_SERVICE: &str = "com.docker.compose.service";
const COMPOSE_CONFIG_FILES: &str = "com.docker.compose.project.config_files";

/// The fixed fields every row has, before any extra label projections.
const FIXED_FIELDS: usize = 11;

fn container_template(labels: &[&str]) -> String {
    let mut fields: Vec<String> = [
        "{{.ID}}",
        "{{.Names}}",
        "{{.Image}}",
        "{{.State}}",
        "{{.Status}}",
        "{{.CreatedAt}}",
        "{{.Command}}",
        "{{.Ports}}",
        &format!("{{{{.Label \"{COMPOSE_PROJECT}\"}}}}"),
        &format!("{{{{.Label \"{COMPOSE_SERVICE}\"}}}}"),
        &format!("{{{{.Label \"{COMPOSE_CONFIG_FILES}\"}}}}"),
    ]
    .iter()
    .map(|f| (*f).to_owned())
    .collect();
    fields.extend(labels.iter().map(|l| format!("{{{{.Label \"{l}\"}}}}")));
    fields.join(&SEP.to_string())
}

/// One container row plus the config-file path its project declares, which `compose` resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRow {
    pub container: Container,
    /// Value of the compose config-files label, verbatim. Read-only: nothing here writes it.
    pub config_files: Option<String>,
    /// The extra labels the caller asked [`list_containers_labelled`] for, by key, present only
    /// when the container carries them. This is how an app reads a label the crate has no opinion
    /// on (Fathom: the kind cluster a node belongs to) without the crate learning what it means.
    pub labels: BTreeMap<String, String>,
}

/// Lists every container, running or not. `-a` is deliberate: a stopped container still holds
/// references to images and volumes, and hiding it makes a prune look safer than it is.
pub fn list_containers_with(cli: Cli, runner: &dyn Runner) -> Result<Vec<ContainerRow>, RunError> {
    list_containers_labelled(cli, runner, &[])
}

/// [`list_containers_with`], also projecting each of `labels` into [`ContainerRow::labels`].
/// Each label is its own targeted `{{.Label}}` projection, for the reason the module doc gives.
pub fn list_containers_labelled(
    cli: Cli,
    runner: &dyn Runner,
    labels: &[&str],
) -> Result<Vec<ContainerRow>, RunError> {
    let template = container_template(labels);
    let out = cli.run(runner, &["ps", "-a", "--no-trunc", "--format", &template])?;
    if !out.success {
        return Err(RunError::Other(nonempty(&out.stderr)));
    }
    Ok(parse_containers_labelled(&out.stdout, labels))
}

pub fn parse_containers(stdout: &str) -> Vec<ContainerRow> {
    parse_containers_labelled(stdout, &[])
}

/// Parses rows rendered by the template for `labels`, in the same order.
pub fn parse_containers_labelled(stdout: &str, labels: &[&str]) -> Vec<ContainerRow> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| parse_container_line(line, labels))
        .collect()
}

fn parse_container_line(line: &str, labels: &[&str]) -> Option<ContainerRow> {
    let f: Vec<&str> = line.split(SEP).collect();
    if f.len() < FIXED_FIELDS + labels.len() {
        return None;
    }
    let status = f[4].to_owned();
    Some(ContainerRow {
        container: Container {
            id: f[0].to_owned(),
            name: f[1].to_owned(),
            image: f[2].to_owned(),
            state: RuntimeContainerState::parse(f[3]),
            health: parse_health(&status),
            status,
            created_at: f[5].to_owned(),
            command: f[6].trim_matches('"').to_owned(),
            ports: parse_ports(f[7]),
            project: opt(f[8]),
            service: opt(f[9]),
            // Which cluster a node belongs to is the app's question, not the runtime's: an app
            // that manages clusters reads its own label through `labels` and sets this.
            cluster: None,
            // The parse has no adoption store to ask. `group_containers` fills this in, which is
            // also the only place a container reaches the UI from.
            adoption: AdoptionState::NotAdopted,
        },
        config_files: opt(f[10]),
        labels: labels
            .iter()
            .zip(&f[FIXED_FIELDS..])
            .filter_map(|(k, v)| opt(v).map(|v| ((*k).to_owned(), v)))
            .collect(),
    })
}

/// Health comes from the status line, which is where the runtime renders it: "Up 5 hours
/// (healthy)". A container with no healthcheck has nothing in parentheses, and that is `None` —
/// never `Healthy`, because "nobody is checking" is not "the check passed".
fn parse_health(status: &str) -> Option<ContainerHealth> {
    let s = status.to_ascii_lowercase();
    if s.contains("(health: starting)") || s.contains("(starting)") {
        Some(ContainerHealth::Starting)
    } else if s.contains("(unhealthy)") {
        Some(ContainerHealth::Unhealthy)
    } else if s.contains("(healthy)") {
        Some(ContainerHealth::Healthy)
    } else {
        None
    }
}

/// Parses the runtime's port rendering: `0.0.0.0:42080->80/tcp, [::]:42080->80/tcp, 30000/tcp`.
///
/// An exposed-but-unpublished port keeps `host_port: None` rather than being dropped: it is part
/// of what the container declares, and silently omitting it would make the row a lie.
fn parse_ports(raw: &str) -> Vec<PublishedPort> {
    let mut ports = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (host, container) = match part.split_once("->") {
            Some((h, c)) => (Some(h), c),
            None => (None, part),
        };
        let (port_str, protocol) = container
            .split_once('/')
            .map_or((container, "tcp"), |(p, pr)| (p, pr));
        let Ok(container_port) = port_str.trim().parse::<u16>() else {
            continue;
        };
        let (host_ip, host_port) = match host {
            // `[::]:42080` and `0.0.0.0:42080`: split at the last colon, so an IPv6 literal
            // keeps its own colons.
            Some(h) => match h.rsplit_once(':') {
                Some((ip, p)) => (Some(ip.to_owned()), p.trim().parse::<u16>().ok()),
                None => (None, None),
            },
            None => (None, None),
        };
        let candidate = PublishedPort {
            container_port,
            host_port,
            host_ip,
            protocol: protocol.trim().to_owned(),
        };
        if !ports.contains(&candidate) {
            ports.push(candidate);
        }
    }
    ports
}

// ---------------------------------------------------------------------------
// Removable objects
// ---------------------------------------------------------------------------

/// Lists images, including untagged ones, which is where reclaimable space usually hides.
pub fn list_images_with(cli: Cli, runner: &dyn Runner) -> Result<Vec<RemovableObject>, RunError> {
    let template = [
        "{{.ID}}",
        "{{.Repository}}",
        "{{.Tag}}",
        "{{.Size}}",
        "{{.CreatedAt}}",
    ]
    .join(&SEP.to_string());
    let out = cli.run(
        runner,
        &["images", "-a", "--no-trunc", "--format", &template],
    )?;
    if !out.success {
        return Err(RunError::Other(nonempty(&out.stderr)));
    }
    Ok(parse_images(&out.stdout))
}

pub fn parse_images(stdout: &str) -> Vec<RemovableObject> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let f: Vec<&str> = line.split(SEP).collect();
            if f.len() < 4 {
                return None;
            }
            let repo = f[1];
            let tag = f[2];
            // `<none>` in either position means untagged. A dangling image is the safe thing to
            // prune and a tagged one usually is not, so the distinction is carried explicitly
            // rather than inferred from the name at render time.
            let tagged = repo != "<none>" && tag != "<none>";
            let name = if tagged {
                format!("{repo}:{tag}")
            } else if repo != "<none>" {
                repo.to_owned()
            } else {
                f[0].to_owned()
            };
            Some(RemovableObject {
                class: ObjectClass::Image,
                id: f[0].to_owned(),
                name,
                size_bytes: parse_size(f[3]),
                tagged: Some(tagged),
                created_at: f.get(4).map(|s| (*s).to_owned()),
                project: None,
                referenced_by: vec![],
            })
        })
        .collect()
}

pub fn list_volumes_with(cli: Cli, runner: &dyn Runner) -> Result<Vec<RemovableObject>, RunError> {
    let template = [
        "{{.Name}}",
        "{{.Driver}}",
        &format!("{{{{.Label \"{COMPOSE_PROJECT}\"}}}}"),
    ]
    .join(&SEP.to_string());
    let out = cli.run(runner, &["volume", "ls", "--format", &template])?;
    if !out.success {
        return Err(RunError::Other(nonempty(&out.stderr)));
    }
    Ok(parse_volumes(&out.stdout))
}

pub fn parse_volumes(stdout: &str) -> Vec<RemovableObject> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let f: Vec<&str> = line.split(SEP).collect();
            if f.is_empty() || f[0].trim().is_empty() {
                return None;
            }
            Some(RemovableObject {
                class: ObjectClass::Volume,
                id: f[0].to_owned(),
                name: f[0].to_owned(),
                // `volume ls` does not report size. `None`, not `0`: a prune of volumes whose
                // sizes are unknown must not read as reclaiming nothing.
                size_bytes: None,
                tagged: None,
                created_at: None,
                project: f.get(2).and_then(|p| opt(p)),
                referenced_by: vec![],
            })
        })
        .collect()
}

pub fn list_networks_with(cli: Cli, runner: &dyn Runner) -> Result<Vec<RemovableObject>, RunError> {
    let template = [
        "{{.ID}}",
        "{{.Name}}",
        "{{.Driver}}",
        &format!("{{{{.Label \"{COMPOSE_PROJECT}\"}}}}"),
    ]
    .join(&SEP.to_string());
    let out = cli.run(
        runner,
        &["network", "ls", "--no-trunc", "--format", &template],
    )?;
    if !out.success {
        return Err(RunError::Other(nonempty(&out.stderr)));
    }
    Ok(parse_networks(&out.stdout))
}

pub fn parse_networks(stdout: &str) -> Vec<RemovableObject> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let f: Vec<&str> = line.split(SEP).collect();
            if f.len() < 2 {
                return None;
            }
            Some(RemovableObject {
                class: ObjectClass::Network,
                id: f[0].to_owned(),
                name: f[1].to_owned(),
                size_bytes: None,
                tagged: None,
                created_at: None,
                project: f.get(3).and_then(|p| opt(p)),
                referenced_by: vec![],
            })
        })
        .collect()
}

/// Fills in `referenced_by` for every removable object from the container inventory.
///
/// This is the "what is safe to delete" answer, and it is computed here rather than asked of the
/// runtime because the runtime has no way to answer it: `image prune` has no dry-run, so the only
/// honest preview is one Fathom derives itself.
pub fn attach_references(objects: &mut [RemovableObject], containers: &[ContainerRow]) {
    for obj in objects.iter_mut() {
        obj.referenced_by = containers
            .iter()
            .filter(|row| references(obj, &row.container))
            .map(|row| ObjectReference {
                kind: if row.container.state == RuntimeContainerState::Running {
                    ReferenceKind::RunningContainer
                } else {
                    ReferenceKind::StoppedContainer
                },
                id: row.container.id.clone(),
                name: row.container.name.clone(),
                project: row.container.project.clone(),
            })
            .collect();
        // A cluster's node containers make its images and networks load-bearing in a way a
        // container reference alone does not convey.
        for row in containers {
            if let Some(cluster) = &row.container.cluster
                && references(obj, &row.container)
                && !obj
                    .referenced_by
                    .iter()
                    .any(|r| r.kind == ReferenceKind::Cluster && &r.name == cluster)
            {
                obj.referenced_by.push(ObjectReference {
                    kind: ReferenceKind::Cluster,
                    id: cluster.clone(),
                    name: cluster.clone(),
                    project: row.container.project.clone(),
                });
            }
        }
    }
}

fn references(obj: &RemovableObject, c: &Container) -> bool {
    match obj.class {
        // The image field may be a tag or an id, depending on how the container was started.
        ObjectClass::Image => c.image == obj.name || c.image == obj.id,
        // Volume and network attachment is not in the list output, so the project label is the
        // only evidence available from one invocation. A match is a reference; a non-match is
        // "no evidence", which the preview states rather than treating as "unreferenced".
        ObjectClass::Volume | ObjectClass::Network => {
            obj.project.is_some() && obj.project == c.project
        }
        ObjectClass::Container => false,
    }
}

fn opt(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t == "<none>" {
        None
    } else {
        Some(t.to_owned())
    }
}

fn nonempty(stderr: &str) -> String {
    let t = stderr.trim();
    if t.is_empty() {
        "the container runtime failed without a message".to_owned()
    } else {
        t.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fields: &[&str]) -> String {
        fields.join(&SEP.to_string())
    }

    /// Captured from a live Docker 29.8.1 daemon shared with two unrelated projects.
    fn sample() -> String {
        [
            row(&[
                "a513ec3d51e64c42d5c24ddab5ac18fde2ee30ac84ec6af6522d7ec918c749aa",
                "thunderforge-app-web",
                "thunderforge-web:local",
                "running",
                "Up 5 hours (healthy)",
                "2026-10-02 13:06:39 -0700 PDT",
                "\"/docker-entrypoint.sh\"",
                "0.0.0.0:42080->80/tcp, [::]:42080->80/tcp",
                "thunderforge-app",
                "web",
                "/home/mbruno/development/thunderforge/compose.app.yml",
                "",
            ]),
            row(&[
                "c5bbc87dbfa3448665677fbb244407662599457e3c3c1d1cdafe3d54d3885d4f",
                "slam-usa-gateway-1",
                "slam-usa-gateway",
                "restarting",
                "Restarting (1) 19 seconds ago",
                "2026-09-29 13:04:25 -0700 PDT",
                "\"/app/gateway\"",
                "",
                "slam-usa",
                "gateway",
                "/home/mbruno/development/slam-usa/deploy/compose.yaml",
                "",
            ]),
            row(&[
                "f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0",
                "fathom-dev-control-plane",
                "kindest/node:v1.31.0",
                "running",
                "Up 2 days",
                "2026-09-30 09:00:00 -0700 PDT",
                "\"/usr/local/bin/entrypoint\"",
                "127.0.0.1:41337->6443/tcp",
                "",
                "",
                "",
                "fathom-dev",
            ]),
            row(&[
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                "loner",
                "busybox:latest",
                "exited",
                "Exited (0) 3 minutes ago",
                "2026-10-02 16:00:00 -0700 PDT",
                "\"sleep 3600\"",
                "30000/tcp",
                "",
                "",
                "",
                "",
            ]),
        ]
        .join("\n")
    }

    #[test]
    fn parses_a_live_daemons_rows() {
        let rows = parse_containers(&sample());
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0].container.project.as_deref(),
            Some("thunderforge-app")
        );
        assert_eq!(rows[0].container.service.as_deref(), Some("web"));
        assert_eq!(rows[0].container.health, Some(ContainerHealth::Healthy));
        assert_eq!(rows[0].container.command, "/docker-entrypoint.sh");
        assert_eq!(
            rows[0].container.id.len(),
            64,
            "full id, not the short form"
        );
    }

    /// Restarting is its own state. A restart loop rendered as "running" is the single most
    /// misleading thing this list could do.
    #[test]
    fn a_restarting_container_is_not_running() {
        let rows = parse_containers(&sample());
        assert_eq!(rows[1].container.state, RuntimeContainerState::Restarting);
    }

    const KIND: &str = "io.x-k8s.kind.cluster";

    /// What an app that manages clusters does with the label it asked for (Fathom's inventory).
    fn with_clusters(stdout: &str) -> Vec<ContainerRow> {
        parse_containers_labelled(stdout, &[KIND])
            .into_iter()
            .map(|mut r| {
                r.container.cluster = r.labels.get(KIND).cloned();
                r
            })
            .collect()
    }

    /// The template asks for an extra label by name, after the fixed fields, and the parse hands
    /// back only the labels a container actually carries.
    #[test]
    fn an_extra_label_is_projected_by_name_and_kept_only_when_present() {
        let template = container_template(&[KIND]);
        assert!(
            template.ends_with("{{.Label \"io.x-k8s.kind.cluster\"}}"),
            "{template}"
        );
        assert!(!container_template(&[]).contains(KIND));

        let rows = parse_containers_labelled(&sample(), &[KIND]);
        assert_eq!(
            rows[2].labels.get(KIND).map(String::as_str),
            Some("fathom-dev")
        );
        assert!(rows[0].labels.is_empty());
        assert_eq!(
            rows[2].container.cluster, None,
            "the crate does not decide what it means"
        );
    }

    #[test]
    fn a_cluster_node_carries_its_cluster_and_no_project() {
        let rows = with_clusters(&sample());
        assert_eq!(rows[2].container.cluster.as_deref(), Some("fathom-dev"));
        assert_eq!(rows[2].container.project, None);
    }

    /// No healthcheck is `None`. Defaulting it to `Healthy` would invent a passing check.
    #[test]
    fn absent_health_is_none_not_healthy() {
        let rows = parse_containers(&sample());
        assert_eq!(rows[3].container.health, None);
        assert_eq!(rows[2].container.health, None);
    }

    /// research.md D2, the reason this module uses targeted label projections.
    ///
    /// Compose writes `com.docker.compose.depends_on=app:service_started:false,db:service_healthy:
    /// false` — one label whose value contains both the separator and the assignment character of
    /// the flat `Labels` rendering. A parser that split that string would read `db:service_healthy`
    /// as a second label and, worse, could pick up a `project=` fragment from inside it. Asking
    /// the runtime for the project label by name means the value never has to be split at all.
    #[test]
    fn a_label_value_containing_commas_and_equals_cannot_misattribute_a_project() {
        let line = row(&[
            "abc123",
            "svc",
            "img:1",
            "running",
            "Up 1 minute",
            "2026-10-02 16:00:00 -0700 PDT",
            "\"/entry\"",
            "",
            // What the targeted projection returns for this container: the real project.
            "real-project",
            "svc",
            // The config-files value, which also contains a comma when a project has two files.
            "/a/compose.yaml,/a/compose.override.yaml",
        ]);
        let rows = parse_containers(&line);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].container.project.as_deref(), Some("real-project"));
        assert_eq!(
            rows[0].config_files.as_deref(),
            Some("/a/compose.yaml,/a/compose.override.yaml"),
            "a multi-file project keeps both paths intact"
        );
    }

    #[test]
    fn published_and_exposed_ports_are_both_kept() {
        let rows = parse_containers(&sample());
        let web = &rows[0].container.ports;
        assert_eq!(web.len(), 2, "{web:?}");
        assert_eq!(web[0].container_port, 80);
        assert_eq!(web[0].host_port, Some(42080));
        assert_eq!(web[0].host_ip.as_deref(), Some("0.0.0.0"));
        assert_eq!(
            web[1].host_ip.as_deref(),
            Some("[::]"),
            "IPv6 keeps its colons"
        );

        let loner = &rows[3].container.ports;
        assert_eq!(loner.len(), 1);
        assert_eq!(loner[0].container_port, 30000);
        assert_eq!(
            loner[0].host_port, None,
            "exposed but unpublished stays visible with no host port"
        );
    }

    #[test]
    fn a_malformed_row_is_skipped_not_guessed_at() {
        let rows = parse_containers("abc\u{1f}only\u{1f}three");
        assert!(rows.is_empty());
    }

    #[test]
    fn dangling_and_tagged_images_are_distinguished() {
        let stdout = [
            row(&[
                "sha256:aaa",
                "thunderforge-web",
                "local",
                "130MB",
                "2026-10-02 13:06:37",
            ]),
            row(&[
                "sha256:bbb",
                "<none>",
                "<none>",
                "522MB",
                "2026-10-02 13:03:18",
            ]),
        ]
        .join("\n");
        let images = parse_images(&stdout);
        assert_eq!(images[0].tagged, Some(true));
        assert_eq!(images[0].name, "thunderforge-web:local");
        assert_eq!(images[0].size_bytes, Some(130_000_000));
        assert_eq!(images[1].tagged, Some(false));
        assert_eq!(images[1].name, "sha256:bbb");
    }

    #[test]
    fn volume_size_is_unavailable_rather_than_zero() {
        let volumes = parse_volumes(&row(&["6aa4a18a7a71", "local", "gh-audit"]));
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].size_bytes, None);
        assert_eq!(volumes[0].project.as_deref(), Some("gh-audit"));
    }

    #[test]
    fn networks_keep_their_project_and_drop_the_unlabelled_ones() {
        let stdout = [
            row(&["8c2d94845ace", "bridge", "bridge", ""]),
            row(&["ba809dbf4767", "gh-audit_default", "bridge", "gh-audit"]),
        ]
        .join("\n");
        let nets = parse_networks(&stdout);
        assert_eq!(nets[0].project, None);
        assert_eq!(nets[1].project.as_deref(), Some("gh-audit"));
    }

    #[test]
    fn references_name_the_containers_that_hold_an_image() {
        let containers = parse_containers(&sample());
        let mut images = parse_images(&row(&[
            "sha256:aaa",
            "thunderforge-web",
            "local",
            "130MB",
            "2026-10-02 13:06:37",
        ]));
        attach_references(&mut images, &containers);
        assert_eq!(images[0].referenced_by.len(), 1);
        assert_eq!(
            images[0].referenced_by[0].kind,
            ReferenceKind::RunningContainer
        );
        assert_eq!(
            images[0].referenced_by[0].project.as_deref(),
            Some("thunderforge-app"),
            "the reference has to name the project, or the blast radius cannot"
        );
        assert!(!images[0].is_unreferenced());
    }

    /// A stopped container still holds its image. Treating only running containers as references
    /// is how a prune takes something that was about to be restarted.
    #[test]
    fn a_stopped_container_still_counts_as_a_reference() {
        let containers = parse_containers(&sample());
        let mut images = parse_images(&row(&[
            "sha256:busy",
            "busybox",
            "latest",
            "4.26MB",
            "2026-10-01 10:00:00",
        ]));
        attach_references(&mut images, &containers);
        assert_eq!(images[0].referenced_by.len(), 1);
        assert_eq!(
            images[0].referenced_by[0].kind,
            ReferenceKind::StoppedContainer
        );
    }

    /// An image a cluster's node runs is referenced by that cluster as well as by the container,
    /// so a preview can say "this breaks fathom-dev" rather than naming an opaque container id.
    #[test]
    fn a_cluster_node_image_is_referenced_by_its_cluster() {
        let containers = with_clusters(&sample());
        let mut images = parse_images(&row(&[
            "sha256:node",
            "kindest/node",
            "v1.31.0",
            "1.02GB",
            "2026-09-30 08:00:00",
        ]));
        attach_references(&mut images, &containers);
        let kinds: Vec<_> = images[0].referenced_by.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&ReferenceKind::Cluster), "{kinds:?}");
        assert_eq!(
            images[0]
                .referenced_by
                .iter()
                .find(|r| r.kind == ReferenceKind::Cluster)
                .map(|r| r.name.as_str()),
            Some("fathom-dev")
        );
    }
}
