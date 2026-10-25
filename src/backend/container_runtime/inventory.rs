//! Fathom's reading of the container inventory: the shared crate's, plus which kind cluster a
//! node container belongs to.
//!
//! The crate reads containers without knowing what a cluster is. Fathom asks it for kind's node
//! label as one more targeted projection and copies that onto `Container::cluster`, which is what
//! grouping, references and the cluster-break preview key on.

pub use ::container_runtime::inventory::*;

use ::container_runtime::Cli;
use ::container_runtime::runner::{RunError, Runner};

/// The label kind puts on each node container, valued with the cluster's name.
pub const KIND_CLUSTER: &str = "io.x-k8s.kind.cluster";

/// Every container, running or not, with its kind cluster filled in.
pub fn list_containers_with(cli: Cli, runner: &dyn Runner) -> Result<Vec<ContainerRow>, RunError> {
    Ok(with_clusters(list_containers_labelled(
        cli,
        runner,
        &[KIND_CLUSTER],
    )?))
}

/// Parses rows rendered by [`list_containers_with`]'s template (the test fixtures' entry point).
#[cfg(test)]
pub fn parse_containers(stdout: &str) -> Vec<ContainerRow> {
    with_clusters(parse_containers_labelled(stdout, &[KIND_CLUSTER]))
}

fn with_clusters(rows: Vec<ContainerRow>) -> Vec<ContainerRow> {
    rows.into_iter()
        .map(|mut row| {
            row.container.cluster = row.labels.get(KIND_CLUSTER).cloned();
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::container_runtime::test_util::RecordingRunner;

    const SEP: char = '\u{1f}';

    fn row(cluster: &str) -> String {
        [
            "f0f0",
            "fathom-dev-control-plane",
            "kindest/node:v1.31.0",
            "running",
            "Up 2 days",
            "2026-09-30 09:00:00 -0700 PDT",
            "\"/usr/local/bin/entrypoint\"",
            "",
            "",
            "",
            "",
            cluster,
        ]
        .join(&SEP.to_string())
    }

    #[test]
    fn a_kind_node_carries_its_cluster_and_a_plain_container_none() {
        let rows = parse_containers(&format!("{}\n{}", row("fathom-dev"), row("")));
        assert_eq!(rows[0].container.cluster.as_deref(), Some("fathom-dev"));
        assert_eq!(rows[1].container.cluster, None);
    }

    /// The live listing asks the runtime for the kind label by name.
    #[test]
    fn the_listing_projects_the_kind_label() {
        let runner = RecordingRunner::ok(&row("fathom-dev"));
        let rows = list_containers_with(Cli::docker(), &runner).unwrap();
        assert_eq!(rows[0].container.cluster.as_deref(), Some("fathom-dev"));
        let args = runner.argv().join(" ");
        assert!(
            args.contains("{{.Label \"io.x-k8s.kind.cluster\"}}"),
            "{args}"
        );
    }
}
