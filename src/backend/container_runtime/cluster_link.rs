//! Cluster identity, in both directions.
//!
//! A kind node is an ordinary container on this daemon, so the containers list and the clusters
//! list are the same objects seen from two sides. An operator who does not know that is one
//! mis-click from killing the cluster they are working in — which is why this is a safety feature
//! and not a convenience, and why it lives in one module rather than being inferred twice.
//!
//! Identity comes from the `io.x-k8s.kind.cluster` label and from nothing else. A container called
//! `kind-control-plane` that carries no label is not claimed for any cluster: guessing from a name
//! would attach a break warning to an unrelated container, and a warning that is sometimes about
//! nothing is a warning the operator learns to click past.

use std::collections::BTreeMap;

use crate::protocol::v1::{
    ClusterBreak, ClusterLink, Container, LifecycleAction, ObjectRef, OperationRequest,
    ProjectAction,
};

use super::inventory::ContainerRow;

/// The kubeconfig context name kind gives a cluster it created.
///
/// Derived rather than looked up, because the question is only ever "is the context the operator
/// is on the one this cluster answers to", and kind's naming of its own contexts is fixed.
fn kind_context(cluster: &str) -> String {
    format!("kind-{cluster}")
}

/// Which clusters these containers back, and which containers back each one.
///
/// `managed_by_fathom` is left `false`: this module can see that a container carries a cluster
/// label, but not whether Fathom provisioned that cluster. [`mark_managed`] sets it once the
/// caller that knows the managed list is in scope.
pub fn cluster_links(rows: &[ContainerRow]) -> Vec<ClusterLink> {
    let mut by_cluster: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        if let Some(cluster) = &row.container.cluster {
            by_cluster
                .entry(cluster.clone())
                .or_default()
                .push(row.container.id.clone());
        }
    }
    by_cluster
        .into_iter()
        .map(|(cluster, container_ids)| ClusterLink {
            cluster,
            container_ids,
            managed_by_fathom: false,
        })
        .collect()
}

/// Flags the links whose cluster Fathom provisioned.
pub fn mark_managed(links: &mut [ClusterLink], managed: &[String]) {
    for link in links {
        link.managed_by_fathom = managed.contains(&link.cluster);
    }
}

/// Whether a request stops the containers it names, rather than starting them.
///
/// `start` and `unpause` bring a node back. Warning that they break the cluster would teach the
/// operator that the warning means nothing.
fn halts(request: &OperationRequest) -> bool {
    match request {
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
    }
}

/// Which clusters an operation takes down with it.
///
/// `active_context` is the kubeconfig context the operator is on, so the warning can say whether
/// the cluster about to break is the one they are connected to right now.
pub fn cluster_breaks(
    request: &OperationRequest,
    targets: &[ObjectRef],
    containers: &[Container],
    active_context: Option<&str>,
) -> Vec<ClusterBreak> {
    if !halts(request) {
        return vec![];
    }

    let mut clusters: Vec<String> = targets
        .iter()
        .filter_map(|t| {
            containers
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
            connected: active_context.is_some_and(|c| c == kind_context(&cluster)),
            cluster,
        })
        .collect()
}

/// Fathom's answer to the shared preview's cluster question: kind clusters, with the operator's
/// current kubeconfig context deciding which one they are connected to.
pub struct KindClusters<'a> {
    pub active_context: Option<&'a str>,
}

impl ::container_runtime::preview::ClusterImpact for KindClusters<'_> {
    fn breaks(
        &self,
        request: &OperationRequest,
        targets: &[ObjectRef],
        containers: &[Container],
    ) -> Vec<ClusterBreak> {
        cluster_breaks(request, targets, containers, self.active_context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::container_runtime::inventory::parse_containers;
    use crate::protocol::v1::{ObjectClass, OperationScope};

    const SEP: char = '\u{1f}';

    /// Mock `docker ps` output: the fields the real template asks for, in its order. No daemon is
    /// contacted by any test in this module, and no cluster is created for them.
    fn ps(lines: &[[&str; 12]]) -> Vec<ContainerRow> {
        let joined: Vec<String> = lines.iter().map(|f| f.join(&SEP.to_string())).collect();
        parse_containers(&joined.join("\n"))
    }

    fn node(id: &str, name: &str, cluster: &str) -> [&'static str; 12] {
        // Leaked so the array can hold `&'static str` alongside the literals; test-only, and the
        // alternative is threading lifetimes through every fixture for no gain.
        [
            Box::leak(id.to_owned().into_boxed_str()),
            Box::leak(name.to_owned().into_boxed_str()),
            "kindest/node:v1.31.0",
            "running",
            "Up 2 hours",
            "2026-10-01 09:00:00 +0000 UTC",
            "/usr/local/bin/entrypoint",
            "127.0.0.1:6443->6443/tcp",
            "",
            "",
            "",
            Box::leak(cluster.to_owned().into_boxed_str()),
        ]
    }

    const PLAIN: [&str; 12] = [
        "cafe01",
        // Named exactly like a node, labelled as nothing. The look-alike case.
        "kind-control-plane",
        "busybox:latest",
        "running",
        "Up 1 minute",
        "2026-10-02 12:00:00 +0000 UTC",
        "sleep 3600",
        "",
        "",
        "",
        "",
        "",
    ];

    fn target(id: &str) -> ObjectRef {
        ObjectRef {
            class: ObjectClass::Container,
            id: id.to_owned(),
            name: id.to_owned(),
        }
    }

    fn stop(id: &str) -> OperationRequest {
        OperationRequest::Lifecycle {
            action: LifecycleAction::Stop,
            scope: OperationScope::Container { id: id.to_owned() },
        }
    }

    /// Scenario 1 and 2 share one resolution: the link is the same fact read from either side.
    #[test]
    fn a_labelled_node_names_its_cluster_and_the_cluster_names_its_nodes() {
        let rows = ps(&[
            node("aaa111", "fathom-dev-control-plane", "fathom-dev"),
            node("bbb222", "fathom-dev-worker", "fathom-dev"),
        ]);
        assert_eq!(rows[0].container.cluster.as_deref(), Some("fathom-dev"));

        let links = cluster_links(&rows);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].cluster, "fathom-dev");
        assert_eq!(links[0].container_ids, vec!["aaa111", "bbb222"]);
    }

    /// Scenario 5. The label is the only source, so a container that merely looks like a node is
    /// claimed for nothing — a break warning on an unrelated container is worse than none.
    #[test]
    fn a_look_alike_with_no_label_is_claimed_for_no_cluster() {
        let rows = ps(&[PLAIN]);
        assert_eq!(rows[0].container.cluster, None);
        assert!(cluster_links(&rows).is_empty());

        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();
        assert!(
            cluster_breaks(
                &stop("cafe01"),
                &[target("cafe01")],
                &containers,
                Some("kind-fathom-dev"),
            )
            .is_empty(),
            "a name is not an identity"
        );
    }

    /// Scenario 3, both halves. The cluster is named, and whether the operator is standing in it
    /// is a separate sentence.
    #[test]
    fn stopping_a_node_names_the_cluster_and_says_whether_the_operator_is_in_it() {
        let rows = ps(&[node("aaa111", "fathom-dev-control-plane", "fathom-dev")]);
        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();

        let connected = cluster_breaks(
            &stop("aaa111"),
            &[target("aaa111")],
            &containers,
            Some("kind-fathom-dev"),
        );
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].cluster, "fathom-dev");
        assert!(connected[0].connected);

        // Same breakage, different sentence: the cluster still goes, the operator is elsewhere.
        let elsewhere = cluster_breaks(
            &stop("aaa111"),
            &[target("aaa111")],
            &containers,
            Some("prod-eu"),
        );
        assert_eq!(
            elsewhere.len(),
            1,
            "it breaks whether or not they are in it"
        );
        assert!(!elsewhere[0].connected);
    }

    /// A removal is a break too, and it is the one that cannot be undone by starting the container
    /// again.
    #[test]
    fn removing_a_node_breaks_its_cluster() {
        let rows = ps(&[node("aaa111", "fathom-dev-control-plane", "fathom-dev")]);
        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();
        let breaks = cluster_breaks(
            &OperationRequest::Remove {
                objects: vec![target("aaa111")],
            },
            &[target("aaa111")],
            &containers,
            None,
        );
        assert_eq!(breaks.len(), 1);
        assert!(!breaks[0].connected, "no active context, so no claim");
    }

    /// The verbs that bring a node back do not warn. A warning that fires on `start` is a warning
    /// the operator stops reading.
    #[test]
    fn starting_a_node_warns_about_nothing() {
        let rows = ps(&[node("aaa111", "fathom-dev-control-plane", "fathom-dev")]);
        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();
        for action in [LifecycleAction::Start, LifecycleAction::Unpause] {
            let request = OperationRequest::Lifecycle {
                action,
                scope: OperationScope::Container {
                    id: "aaa111".to_owned(),
                },
            };
            assert!(
                cluster_breaks(&request, &[target("aaa111")], &containers, None).is_empty(),
                "{action:?} brings the node back"
            );
        }
    }

    /// End to end through the shared preview: with Fathom's `KindClusters` as its cluster answer,
    /// stopping a node names the cluster it breaks and that the operator is in it, and starting it
    /// back up names nothing. (Moved from the runtime crate's mutate tests in 050.)
    #[test]
    fn a_stopping_verb_on_a_cluster_node_names_the_cluster_it_breaks() {
        use ::container_runtime::preview::{self, PreviewContext};

        let rows = ps(&[node(
            "eeee000000005",
            "fathom-dev-control-plane",
            "fathom-dev",
        )]);
        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();
        let kind = KindClusters {
            active_context: Some("kind-fathom-dev"),
        };
        let context = PreviewContext {
            app_name: "Fathom",
            cli: ::container_runtime::Cli::docker(),
            containers: &containers,
            images: &[],
            volumes: &[],
            networks: &[],
            adopted: &[],
            definition_file: None,
            cluster: Some(&kind),
            as_of: "2026-10-02T12:00:00Z",
        };

        let stored = preview::build("prev-9".to_owned(), &stop("eeee000000005"), &context).unwrap();
        assert_eq!(stored.preview.breaks_clusters.len(), 1);
        assert_eq!(stored.preview.breaks_clusters[0].cluster, "fathom-dev");
        assert!(stored.preview.breaks_clusters[0].connected);

        let start = OperationRequest::Lifecycle {
            action: LifecycleAction::Start,
            scope: OperationScope::Container {
                id: "eeee000000005".to_owned(),
            },
        };
        let stored = preview::build("prev-10".to_owned(), &start, &context).unwrap();
        assert!(stored.preview.breaks_clusters.is_empty());
    }

    /// Two nodes of one cluster are one warning. "This breaks fathom-dev" twice is noise.
    #[test]
    fn two_nodes_of_one_cluster_are_one_warning() {
        let rows = ps(&[
            node("aaa111", "fathom-dev-control-plane", "fathom-dev"),
            node("bbb222", "fathom-dev-worker", "fathom-dev"),
        ]);
        let containers: Vec<Container> = rows.iter().map(|r| r.container.clone()).collect();
        let breaks = cluster_breaks(
            &OperationRequest::Remove {
                objects: vec![target("aaa111"), target("bbb222")],
            },
            &[target("aaa111"), target("bbb222")],
            &containers,
            None,
        );
        assert_eq!(breaks.len(), 1, "{breaks:?}");
    }

    /// Two clusters on one daemon are two links, not one merged list of nodes.
    #[test]
    fn nodes_of_different_clusters_do_not_pool() {
        let rows = ps(&[
            node("aaa111", "fathom-dev-control-plane", "fathom-dev"),
            node("bbb222", "fathom-dev-worker", "fathom-dev"),
            node("ccc333", "other-control-plane", "other"),
        ]);
        let links = cluster_links(&rows);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].container_ids.len(), 2);
        assert_eq!(links[1].container_ids, vec!["ccc333"]);
        assert!(
            !links[0].managed_by_fathom,
            "this module cannot know that; `mark_managed` does"
        );
    }

    #[test]
    fn a_daemon_with_no_clusters_produces_no_links() {
        assert!(cluster_links(&ps(&[PLAIN])).is_empty());
    }

    #[test]
    fn only_the_clusters_fathom_provisioned_are_marked_managed() {
        let rows = ps(&[
            node("aaa111", "fathom-dev-control-plane", "fathom-dev"),
            node("ccc333", "someone-elses-control-plane", "someone-else"),
        ]);
        let mut links = cluster_links(&rows);
        mark_managed(&mut links, &["fathom-dev".to_owned()]);
        let managed: Vec<(&str, bool)> = links
            .iter()
            .map(|l| (l.cluster.as_str(), l.managed_by_fathom))
            .collect();
        assert_eq!(
            managed,
            vec![("fathom-dev", true), ("someone-else", false)],
            "a cluster on this daemon is not Fathom's just because it is here"
        );
    }
}
