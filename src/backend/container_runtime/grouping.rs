//! Turning a flat container list into compose projects.
//!
//! Grouping is not presentation. It is what makes blast radius legible: "this prune touches
//! `thunderforge-app` and `slam-usa`" is an answer an operator can act on, and "this prune touches
//! 23 containers" is not. The project label is therefore the primary key of this whole scene.

use std::collections::BTreeMap;

use crate::protocol::v1::{
    AdoptionState, ClusterLink, DefinitionFileState, Project, ProjectGroup, RuntimeContainerState,
};

use super::inventory::ContainerRow;

/// Decides whether a project's declared definition file is usable.
///
/// Passed in rather than performed here so the grouping itself stays pure and testable without
/// touching a filesystem. The real caller stats the path; tests hand it a closure.
pub trait DefinitionProbe {
    fn inspect(&self, path: &str) -> DefinitionFileState;
}

impl<F: Fn(&str) -> DefinitionFileState> DefinitionProbe for F {
    fn inspect(&self, path: &str) -> DefinitionFileState {
        self(path)
    }
}

/// Answers "has the operator adopted this project?" for one group.
pub trait AdoptionLookup {
    fn state_for(&self, group: &ProjectGroup) -> AdoptionState;
}

/// Groups containers by their compose project, newest-stable-order first by project name.
///
/// Containers with no project label go to [`ProjectGroup::Unowned`] — a tagged variant, so a real
/// compose project literally named "unowned" cannot be mistaken for the bucket.
pub fn group_containers(
    rows: &[ContainerRow],
    definitions: &dyn DefinitionProbe,
    adoption: &dyn AdoptionLookup,
) -> Vec<Project> {
    // BTreeMap: project order is the operator's, not the daemon's. An inventory that reshuffles
    // between refreshes makes the list unreadable and the diff between two refreshes meaningless.
    let mut by_group: BTreeMap<ProjectGroup, Vec<&ContainerRow>> = BTreeMap::new();
    for row in rows {
        let group = match &row.container.project {
            Some(p) => ProjectGroup::Project(p.clone()),
            None => ProjectGroup::Unowned,
        };
        by_group.entry(group).or_default().push(row);
    }

    by_group
        .into_iter()
        .map(|(group, members)| {
            let definition_file = declared_definition(&members);
            let definition_state = match &definition_file {
                Some(path) => definitions.inspect(path),
                None => DefinitionFileState::Undeclared,
            };
            let running = members
                .iter()
                .filter(|r| r.container.state == RuntimeContainerState::Running)
                .count();
            Project {
                adoption: adoption.state_for(&group),
                group,
                #[allow(clippy::cast_possible_truncation)]
                members: members.len() as u32,
                #[allow(clippy::cast_possible_truncation)]
                running: running as u32,
                definition_file,
                definition_state,
                containers: members.iter().map(|r| r.container.clone()).collect(),
            }
        })
        .collect()
}

/// The definition path a project declares, as the first path of the compose config-files label.
///
/// Compose joins multiple files with a comma (`compose.yaml,compose.override.yaml`); the first is
/// the base file and the one worth showing. Members of a project should agree, and if they do not —
/// a project started twice from different directories — the first in sorted container order wins
/// deterministically rather than whichever the daemon happened to list first.
fn declared_definition(members: &[&ContainerRow]) -> Option<String> {
    let mut declared: Vec<&str> = members
        .iter()
        .filter_map(|r| r.config_files.as_deref())
        .collect();
    declared.sort_unstable();
    declared
        .first()
        .and_then(|v| v.split(',').next())
        .map(str::to_owned)
}

/// Which clusters the containers back, for the cluster cross-link in both directions.
///
/// `managed_by_fathom` is left `false` here: this module can see that a container carries a kind
/// cluster label, but not whether Fathom provisioned that cluster. The caller that knows the
/// managed-cluster list sets it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::container_runtime::inventory::parse_containers;

    const SEP: char = '\u{1f}';

    struct AllAdopted;
    impl AdoptionLookup for AllAdopted {
        fn state_for(&self, _: &ProjectGroup) -> AdoptionState {
            AdoptionState::Adopted
        }
    }

    struct NoneAdopted;
    impl AdoptionLookup for NoneAdopted {
        fn state_for(&self, _: &ProjectGroup) -> AdoptionState {
            AdoptionState::NotAdopted
        }
    }

    fn present(_: &str) -> DefinitionFileState {
        DefinitionFileState::Present
    }

    fn row(fields: &[&str]) -> String {
        fields.join(&SEP.to_string())
    }

    fn container(name: &str, state: &str, project: &str, cluster: &str, files: &str) -> String {
        row(&[
            &format!("id-{name}"),
            name,
            "busybox:latest",
            state,
            if state == "running" {
                "Up 1 hour"
            } else {
                "Exited (0) 1 hour ago"
            },
            "2026-10-02 10:00:00 -0700 PDT",
            "\"sleep 3600\"",
            "",
            project,
            name,
            cluster,
            files,
        ])
    }

    fn rows(lines: &[String]) -> Vec<ContainerRow> {
        parse_containers(&lines.join("\n"))
    }

    #[test]
    fn containers_group_by_project_with_counts() {
        let rows = rows(&[
            container("web", "running", "alpha", "", "/a/compose.yaml"),
            container("db", "exited", "alpha", "", "/a/compose.yaml"),
            container("job", "running", "beta", "", "/b/compose.yaml"),
        ]);
        let projects = group_containers(&rows, &present, &AllAdopted);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].group, ProjectGroup::Project("alpha".into()));
        assert_eq!(projects[0].members, 2);
        assert_eq!(projects[0].running, 1);
        assert_eq!(projects[1].group, ProjectGroup::Project("beta".into()));
        assert_eq!(projects[1].running, 1);
    }

    /// The bucket is a variant, not a name, so a project called "unowned" stays its own group.
    #[test]
    fn a_project_named_unowned_does_not_fall_into_the_unowned_bucket() {
        let rows = rows(&[
            container("stray", "running", "", "", ""),
            container("real", "running", "unowned", "", "/u/compose.yaml"),
        ]);
        let projects = group_containers(&rows, &present, &AllAdopted);
        assert_eq!(projects.len(), 2);
        let groups: Vec<&ProjectGroup> = projects.iter().map(|p| &p.group).collect();
        assert!(groups.contains(&&ProjectGroup::Project("unowned".into())));
        assert!(groups.contains(&&ProjectGroup::Unowned));
        let bucket = projects
            .iter()
            .find(|p| p.group == ProjectGroup::Unowned)
            .expect("the bucket exists");
        assert_eq!(bucket.members, 1);
        assert_eq!(bucket.containers[0].name, "stray");
    }

    #[test]
    fn the_unowned_bucket_declares_no_definition_file() {
        let rows = rows(&[container("stray", "running", "", "", "")]);
        let projects = group_containers(&rows, &present, &AllAdopted);
        assert_eq!(projects[0].definition_file, None);
        assert_eq!(
            projects[0].definition_state,
            DefinitionFileState::Undeclared
        );
    }

    /// Principle VI: an unreadable definition file is reported as unreadable and left alone. The
    /// project is still listed — losing the containers because their yaml moved would be worse.
    #[test]
    fn a_missing_definition_file_does_not_hide_the_project() {
        let rows = rows(&[container(
            "web",
            "running",
            "alpha",
            "",
            "/gone/compose.yaml",
        )]);
        let projects =
            group_containers(&rows, &|_: &str| DefinitionFileState::Missing, &AllAdopted);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].members, 1);
        assert_eq!(projects[0].definition_state, DefinitionFileState::Missing);
        assert_eq!(
            projects[0].definition_file.as_deref(),
            Some("/gone/compose.yaml"),
            "the path stays visible so the operator can see what it was looking for"
        );
    }

    #[test]
    fn a_multi_file_project_reports_its_base_file() {
        let rows = rows(&[container(
            "web",
            "running",
            "alpha",
            "",
            "/a/compose.yaml,/a/compose.override.yaml",
        )]);
        let projects = group_containers(&rows, &present, &AllAdopted);
        assert_eq!(
            projects[0].definition_file.as_deref(),
            Some("/a/compose.yaml")
        );
    }

    #[test]
    fn adoption_is_asked_per_group_not_assumed() {
        let rows = rows(&[container("web", "running", "alpha", "", "/a/compose.yaml")]);
        let adopted = group_containers(&rows, &present, &AllAdopted);
        let not = group_containers(&rows, &present, &NoneAdopted);
        assert_eq!(adopted[0].adoption, AdoptionState::Adopted);
        assert_eq!(not[0].adoption, AdoptionState::NotAdopted);
        assert!(!not[0].adoption.is_actionable());
    }

    /// Ordering is the operator's, not the daemon's: two refreshes of the same daemon must produce
    /// the same list, or nothing in the scene can be compared against what was there a moment ago.
    #[test]
    fn group_order_is_stable_regardless_of_daemon_order() {
        let a = rows(&[
            container("z", "running", "zeta", "", "/z/compose.yaml"),
            container("a", "running", "alpha", "", "/a/compose.yaml"),
        ]);
        let b = rows(&[
            container("a", "running", "alpha", "", "/a/compose.yaml"),
            container("z", "running", "zeta", "", "/z/compose.yaml"),
        ]);
        let names = |ps: Vec<Project>| -> Vec<String> {
            ps.iter()
                .map(|p| match &p.group {
                    ProjectGroup::Project(n) => n.clone(),
                    ProjectGroup::Unowned => "<unowned>".to_owned(),
                })
                .collect()
        };
        assert_eq!(
            names(group_containers(&a, &present, &AllAdopted)),
            names(group_containers(&b, &present, &AllAdopted))
        );
    }

    #[test]
    fn cluster_nodes_link_back_to_their_cluster() {
        let rows = rows(&[
            container("fathom-dev-control-plane", "running", "", "fathom-dev", ""),
            container("fathom-dev-worker", "running", "", "fathom-dev", ""),
            container("other-control-plane", "running", "", "other", ""),
        ]);
        let links = cluster_links(&rows);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].cluster, "fathom-dev");
        assert_eq!(links[0].container_ids.len(), 2);
        assert!(
            !links[0].managed_by_fathom,
            "this module cannot know that; the caller that has the managed list sets it"
        );
    }

    #[test]
    fn a_daemon_with_no_clusters_produces_no_links() {
        let rows = rows(&[container("web", "running", "alpha", "", "/a/compose.yaml")]);
        assert!(cluster_links(&rows).is_empty());
    }
}
