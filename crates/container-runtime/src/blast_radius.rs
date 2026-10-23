//! What a removal would actually take, named object by object.
//!
//! This module exists because the runtime offers no dry-run (research D3): `docker prune` tells
//! you what it removed, never what it would remove. So the only honest preview of a destructive
//! operation is one Fathom computes itself, against the inventory it has already read — and the
//! consequence of that is the rule in [`resolve_prune`]: a prune is never handed to the runtime as
//! a prune. It is resolved here into a list of identities, and what runs is a removal of exactly
//! those. A broad `prune` issued on the operator's behalf would act on whatever the daemon
//! happened to hold a second later, which on this machine means another project's work.
//!
//! Two rules do the real work here, and both are about a shared daemon:
//!
//! - An object belonging to a project the operator has not adopted is **excluded** (FR-025) — not
//!   flagged, not listed with a warning beside it. A warning the operator can confirm past is
//!   exactly how an unrelated project's volume gets deleted.
//! - An object is judged by every project that can reach it, not only the one it declares. A
//!   dangling image nobody owns is still off limits while an unadopted project's container
//!   references it, because removing it is removing something that project depends on.
//!
//! The blast radius is the set of projects the removals **touch**. Projects that appear only
//! through an exclusion are not in it: nothing of theirs is going to happen, and the excluded list
//! already names them with the reason.

use std::collections::BTreeSet;

use crate::wire::{
    Container, ExcludedObject, ExclusionReason, ObjectClass, ObjectRef, PruneFilter,
    RemovableObject, RemovalLine, RuntimeContainerState,
};

use super::preview::NotPreviewable;

/// One reading of the runtime, which is what every removal set is resolved against.
pub struct Scene<'a> {
    pub containers: &'a [Container],
    pub images: &'a [RemovableObject],
    pub volumes: &'a [RemovableObject],
    pub networks: &'a [RemovableObject],
    /// The projects the operator has adopted. Everything else is read-only (FR-012), and that
    /// applies to removal above all.
    pub adopted: &'a [String],
}

impl Scene<'_> {
    fn pool(&self, class: ObjectClass) -> &[RemovableObject] {
        match class {
            ObjectClass::Image => self.images,
            ObjectClass::Volume => self.volumes,
            ObjectClass::Network => self.networks,
            ObjectClass::Container => &[],
        }
    }

    fn adopted(&self, project: &str) -> bool {
        self.adopted.iter().any(|a| a == project)
    }
}

/// A resolved removal set: what goes, what stays, and whose work it reaches.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Radius {
    /// Every object that disappears, by identity. Never a count (FR-023).
    pub removals: Vec<RemovalLine>,
    /// Every project the removals touch, sorted, each named once.
    pub projects: Vec<String>,
    /// Objects inside the selection that will not be touched, and why.
    pub excluded: Vec<ExcludedObject>,
}

/// The class a filter can possibly select. A filter and a class that disagree are not a narrower
/// selection, they are a mistake, and resolving one of them silently would remove the wrong kind
/// of object.
fn class_of(filter: PruneFilter) -> ObjectClass {
    match filter {
        PruneFilter::DanglingImages | PruneFilter::UnusedImages => ObjectClass::Image,
        PruneFilter::UnusedVolumes => ObjectClass::Volume,
        PruneFilter::UnusedNetworks => ObjectClass::Network,
        PruneFilter::StoppedContainers => ObjectClass::Container,
    }
}

/// Resolves an explicit list of objects the operator picked.
///
/// An id this inventory does not contain is refused rather than passed through: the operator is
/// looking at a list, and an object that is not in it is a sign the list is older than they think.
pub fn resolve_objects(scene: &Scene<'_>, objects: &[ObjectRef]) -> Result<Radius, NotPreviewable> {
    if objects.is_empty() {
        return Err(NotPreviewable::NotEnumerable(
            "removing nothing at all".to_owned(),
        ));
    }
    let mut found = Vec::with_capacity(objects.len());
    for want in objects {
        found.push(locate(scene, want)?);
    }
    Ok(judge(scene, found))
}

/// Resolves a prune filter into the identities it covers, right now.
///
/// The result is a targeted removal, which is the whole point: `docker image prune` run a second
/// later is a different operation from the one previewed, and this one cannot be.
pub fn resolve_prune(
    scene: &Scene<'_>,
    class: ObjectClass,
    filter: PruneFilter,
) -> Result<Radius, NotPreviewable> {
    if class_of(filter) != class {
        return Err(NotPreviewable::NotEnumerable(
            "a prune whose filter and object class disagree".to_owned(),
        ));
    }
    let candidates: Vec<Candidate> = match filter {
        PruneFilter::StoppedContainers => scene
            .containers
            .iter()
            .filter(|c| {
                matches!(
                    c.state,
                    RuntimeContainerState::Exited | RuntimeContainerState::Dead
                )
            })
            .map(Candidate::from_container)
            .collect(),
        PruneFilter::DanglingImages => scene
            .images
            .iter()
            // `tagged: None` means the runtime did not say. An image Fathom cannot classify is
            // not swept up by a prune of the ones it can.
            .filter(|i| i.tagged == Some(false))
            .map(Candidate::from_object)
            .collect(),
        PruneFilter::UnusedImages | PruneFilter::UnusedVolumes | PruneFilter::UnusedNetworks => {
            scene
                .pool(class)
                .iter()
                .filter(|o| o.is_unreferenced())
                .map(Candidate::from_object)
                .collect()
        }
    };
    if candidates.is_empty() {
        return Err(NotPreviewable::NotEnumerable(
            "a prune with nothing in it".to_owned(),
        ));
    }
    Ok(judge(scene, candidates))
}

/// An object resolved out of the inventory, with everything the adoption rule needs.
struct Candidate {
    object: ObjectRef,
    size_bytes: Option<u64>,
    /// The project the object declares, if any.
    project: Option<String>,
    /// Every project that can reach it, including the one it declares.
    reachable_from: Vec<String>,
}

impl Candidate {
    fn from_object(o: &RemovableObject) -> Self {
        let mut reachable_from: Vec<String> = o.project.iter().cloned().collect();
        reachable_from.extend(o.referenced_by.iter().filter_map(|r| r.project.clone()));
        Self {
            object: ObjectRef {
                class: o.class,
                id: o.id.clone(),
                name: o.name.clone(),
            },
            size_bytes: o.size_bytes,
            project: o.project.clone(),
            reachable_from,
        }
    }

    fn from_container(c: &Container) -> Self {
        Self {
            object: ObjectRef {
                class: ObjectClass::Container,
                id: c.id.clone(),
                name: c.name.clone(),
            },
            // The runtime reports a container's writable-layer size only when asked separately,
            // and an unasked-for number is not a number. `None`, never `0`.
            size_bytes: None,
            project: c.project.clone(),
            reachable_from: c.project.iter().cloned().collect(),
        }
    }
}

/// Finds one object the operator named, in whichever pool its class says it lives.
fn locate(scene: &Scene<'_>, want: &ObjectRef) -> Result<Candidate, NotPreviewable> {
    if want.class == ObjectClass::Container {
        return scene
            .containers
            .iter()
            .find(|c| c.id == want.id)
            .map(Candidate::from_container)
            .ok_or_else(|| NotPreviewable::UnknownTarget(want.id.clone()));
    }
    scene
        .pool(want.class)
        .iter()
        .find(|o| o.id == want.id)
        .map(Candidate::from_object)
        .ok_or_else(|| NotPreviewable::UnknownTarget(want.id.clone()))
}

/// Applies the adoption rule to a resolved selection.
///
/// One pass, so an object cannot end up in both lists: the first unadopted project that can reach
/// it decides, and the reason names that project rather than saying "a project" — the operator's
/// next move is to adopt it, and they need to know which it is.
fn judge(scene: &Scene<'_>, candidates: Vec<Candidate>) -> Radius {
    let mut removals = Vec::new();
    let mut excluded = Vec::new();
    let mut projects = BTreeSet::new();

    for candidate in candidates {
        let blocker = candidate
            .reachable_from
            .iter()
            .find(|p| !scene.adopted(p))
            .cloned();
        match blocker {
            Some(project) => excluded.push(ExcludedObject {
                object: candidate.object,
                reason: ExclusionReason::UnadoptedProject(project),
            }),
            None => {
                projects.extend(candidate.reachable_from.iter().cloned());
                removals.push(RemovalLine {
                    object: candidate.object,
                    size_bytes: candidate.size_bytes,
                    project: candidate.project,
                });
            }
        }
    }

    Radius {
        removals,
        projects: projects.into_iter().collect(),
        excluded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{ContainerHealth, ObjectReference, ReferenceKind};

    fn object(class: ObjectClass, name: &str, project: Option<&str>) -> RemovableObject {
        RemovableObject {
            class,
            id: format!("{name}-id"),
            name: name.to_owned(),
            size_bytes: Some(1024),
            tagged: (class == ObjectClass::Image).then_some(true),
            created_at: None,
            project: project.map(str::to_owned),
            referenced_by: vec![],
        }
    }

    fn referenced(mut o: RemovableObject, by: &str, project: Option<&str>) -> RemovableObject {
        o.referenced_by.push(ObjectReference {
            kind: ReferenceKind::RunningContainer,
            id: format!("{by}-id"),
            name: by.to_owned(),
            project: project.map(str::to_owned),
        });
        o
    }

    fn container(name: &str, state: RuntimeContainerState, project: Option<&str>) -> Container {
        Container {
            id: format!("{name}-id"),
            name: name.to_owned(),
            image: "busybox".to_owned(),
            state,
            health: None::<ContainerHealth>,
            status: "Exited (0) 1 minute ago".to_owned(),
            created_at: "2026-10-02T11:00:00Z".to_owned(),
            command: "sh".to_owned(),
            ports: vec![],
            project: project.map(str::to_owned),
            service: None,
            cluster: None,
            adoption: crate::wire::AdoptionState::NotAdopted,
        }
    }

    struct Pools {
        containers: Vec<Container>,
        images: Vec<RemovableObject>,
        volumes: Vec<RemovableObject>,
        networks: Vec<RemovableObject>,
        adopted: Vec<String>,
    }

    impl Pools {
        fn new() -> Self {
            Self {
                containers: vec![],
                images: vec![],
                volumes: vec![],
                networks: vec![],
                adopted: vec![],
            }
        }
        fn scene(&self) -> Scene<'_> {
            Scene {
                containers: &self.containers,
                images: &self.images,
                volumes: &self.volumes,
                networks: &self.networks,
                adopted: &self.adopted,
            }
        }
    }

    fn refs(removals: &[RemovalLine]) -> Vec<&str> {
        removals.iter().map(|r| r.object.name.as_str()).collect()
    }

    /// FR-024: the operator selected one project's volume, and the set reaches another's. Saying so
    /// is the whole reason the radius exists.
    #[test]
    fn the_radius_names_every_project_the_removals_reach_not_only_the_selected_one() {
        let mut pools = Pools::new();
        pools.volumes = vec![
            object(ObjectClass::Volume, "shop-data", Some("shop")),
            referenced(
                object(ObjectClass::Volume, "shared-cache", None),
                "blog-web",
                Some("blog"),
            ),
        ];
        pools.adopted = vec!["shop".to_owned(), "blog".to_owned()];
        let scene = pools.scene();
        let picked: Vec<ObjectRef> = pools
            .volumes
            .iter()
            .map(|v| ObjectRef {
                class: v.class,
                id: v.id.clone(),
                name: v.name.clone(),
            })
            .collect();

        let radius = resolve_objects(&scene, &picked).expect("both are in the inventory");
        assert_eq!(radius.projects, vec!["blog".to_owned(), "shop".to_owned()]);
        assert_eq!(refs(&radius.removals), vec!["shop-data", "shared-cache"]);
    }

    /// FR-025. The distinction this test exists for: excluded means it is not in the removal list
    /// at all, so there is nothing for a confirmation to approve past.
    #[test]
    fn an_object_of_an_unadopted_project_is_excluded_not_merely_warned_about() {
        let mut pools = Pools::new();
        pools.volumes = vec![
            object(ObjectClass::Volume, "shop-data", Some("shop")),
            object(ObjectClass::Volume, "blog-data", Some("blog")),
        ];
        pools.adopted = vec!["shop".to_owned()];
        let scene = pools.scene();
        let picked: Vec<ObjectRef> = pools
            .volumes
            .iter()
            .map(|v| ObjectRef {
                class: v.class,
                id: v.id.clone(),
                name: v.name.clone(),
            })
            .collect();

        let radius = resolve_objects(&scene, &picked).expect("both are in the inventory");
        assert_eq!(refs(&radius.removals), vec!["shop-data"]);
        assert_eq!(radius.projects, vec!["shop".to_owned()]);
        assert_eq!(
            radius.excluded,
            vec![ExcludedObject {
                object: ObjectRef {
                    class: ObjectClass::Volume,
                    id: "blog-data-id".to_owned(),
                    name: "blog-data".to_owned(),
                },
                // Named, not "a project": adopting it is the operator's next move.
                reason: ExclusionReason::UnadoptedProject("blog".to_owned()),
            }]
        );
    }

    /// Reachability, not ownership. The image is the adopted project's by declaration and an
    /// unadopted project's by use, and removing it would break the one that did not consent.
    #[test]
    fn an_object_two_projects_reach_is_excluded_when_either_is_unadopted() {
        let mut pools = Pools::new();
        pools.images = vec![referenced(
            object(ObjectClass::Image, "busybox:latest", Some("shop")),
            "blog-worker",
            Some("blog"),
        )];
        pools.adopted = vec!["shop".to_owned()];
        let scene = pools.scene();
        let picked = vec![ObjectRef {
            class: ObjectClass::Image,
            id: "busybox:latest-id".to_owned(),
            name: "busybox:latest".to_owned(),
        }];

        let radius = resolve_objects(&scene, &picked).expect("it is in the inventory");
        assert!(radius.removals.is_empty(), "{:?}", radius.removals);
        assert_eq!(
            radius.excluded.first().map(|e| &e.reason),
            Some(&ExclusionReason::UnadoptedProject("blog".to_owned()))
        );
    }

    /// FR-026: what runs is a removal of these identities, so the daemon cannot add to the set
    /// between the preview and the act.
    #[test]
    fn a_prune_resolves_to_the_identities_it_covers_right_now() {
        let mut pools = Pools::new();
        pools.volumes = vec![
            referenced(
                object(ObjectClass::Volume, "in-use", Some("shop")),
                "shop-db",
                Some("shop"),
            ),
            object(ObjectClass::Volume, "orphan", None),
        ];
        pools.adopted = vec!["shop".to_owned()];

        let radius = resolve_prune(
            &pools.scene(),
            ObjectClass::Volume,
            PruneFilter::UnusedVolumes,
        )
        .expect("one volume is unused");
        assert_eq!(refs(&radius.removals), vec!["orphan"]);
        // Nothing holds it and nobody owns it, so no project is reached at all.
        assert!(radius.projects.is_empty());
    }

    /// An image the runtime did not classify is not a dangling image. `None` is "it did not say",
    /// and a prune that treated it as untagged would delete a tagged image on a guess.
    #[test]
    fn an_image_the_runtime_did_not_classify_is_not_swept_up_as_dangling() {
        let mut pools = Pools::new();
        let mut unknown = object(ObjectClass::Image, "mystery", None);
        unknown.tagged = None;
        let mut dangling = object(ObjectClass::Image, "none", None);
        dangling.tagged = Some(false);
        pools.images = vec![unknown, dangling];

        let radius = resolve_prune(
            &pools.scene(),
            ObjectClass::Image,
            PruneFilter::DanglingImages,
        )
        .expect("one image is dangling");
        assert_eq!(refs(&radius.removals), vec!["none"]);
    }

    #[test]
    fn stopped_containers_are_the_exited_and_the_dead_and_nothing_running() {
        let mut pools = Pools::new();
        pools.containers = vec![
            container("gone", RuntimeContainerState::Exited, Some("shop")),
            container("crashed", RuntimeContainerState::Dead, Some("shop")),
            container("alive", RuntimeContainerState::Running, Some("shop")),
            container("paused", RuntimeContainerState::Paused, Some("shop")),
        ];
        pools.adopted = vec!["shop".to_owned()];

        let radius = resolve_prune(
            &pools.scene(),
            ObjectClass::Container,
            PruneFilter::StoppedContainers,
        )
        .expect("two containers have stopped");
        assert_eq!(refs(&radius.removals), vec!["gone", "crashed"]);
        // A container's writable-layer size is not in a plain listing, and an unasked-for number
        // is not a number.
        assert!(radius.removals.iter().all(|r| r.size_bytes.is_none()));
    }

    /// FR-027: a prune with nothing in it is not an operation, and offering it would show the
    /// operator a destructive command whose blast radius really is nothing.
    #[test]
    fn a_prune_that_covers_nothing_is_not_offered() {
        let pools = Pools::new();
        assert_eq!(
            resolve_prune(
                &pools.scene(),
                ObjectClass::Volume,
                PruneFilter::UnusedVolumes
            ),
            Err(NotPreviewable::NotEnumerable(
                "a prune with nothing in it".to_owned()
            ))
        );
    }

    #[test]
    fn a_filter_that_cannot_select_the_named_class_is_refused() {
        let mut pools = Pools::new();
        pools.images = vec![object(ObjectClass::Image, "busybox", None)];
        let err = resolve_prune(
            &pools.scene(),
            ObjectClass::Image,
            PruneFilter::UnusedVolumes,
        )
        .expect_err("a volume filter cannot select images");
        assert!(matches!(err, NotPreviewable::NotEnumerable(_)), "{err:?}");
    }

    /// The operator is looking at a list. An id that is not in it means the list is older than
    /// they think, and the honest answer is to say so rather than pass the id to the daemon.
    #[test]
    fn an_object_this_inventory_does_not_contain_is_refused_by_id() {
        let pools = Pools::new();
        assert_eq!(
            resolve_objects(
                &pools.scene(),
                &[ObjectRef {
                    class: ObjectClass::Volume,
                    id: "sha256:ghost".to_owned(),
                    name: "ghost".to_owned(),
                }]
            ),
            Err(NotPreviewable::UnknownTarget("sha256:ghost".to_owned()))
        );
    }

    #[test]
    fn a_size_the_runtime_did_not_report_stays_unknown_rather_than_zero() {
        let mut pools = Pools::new();
        let mut unsized_volume = object(ObjectClass::Volume, "unsized", None);
        unsized_volume.size_bytes = None;
        pools.volumes = vec![unsized_volume];

        let radius = resolve_prune(
            &pools.scene(),
            ObjectClass::Volume,
            PruneFilter::UnusedVolumes,
        )
        .expect("it is unused");
        assert_eq!(radius.removals.first().and_then(|r| r.size_bytes), None);
    }
}
