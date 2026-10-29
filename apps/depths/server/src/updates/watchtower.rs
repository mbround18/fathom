//! Spec 062 US4: Watchtower containers running on the host (research R6,
//! contracts/watchtower-labels.md "Detecting a running Watchtower").
//!
//! Read from the runtime's container list and `docker inspect` (for labels and the image id); no
//! polling of its own: a check pass reads it. An administrator's answer matches the container id
//! plus image id, so a recreated or upgraded Watchtower raises the banner again. Auto stays paused
//! while any running Watchtower has no matching answer. Depths never stops, restarts or relabels
//! it (FR-021): nothing here mutates.

use depths_core::ports::ContainerRuntime;
use depths_core::updates::labels;
use depths_protocol::{AutoPause, RuntimeContainerState, WatchtowerAnswer, WatchtowerContainer};

use super::store::WatchtowerAck;

/// One running Watchtower container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub container_id: String,
    /// The image id it runs (`sha256:…`): part of what an answer matches.
    pub image_id: String,
    pub name: String,
    /// The image as written (`containrrr/watchtower:latest`).
    pub image: String,
}

/// The Watchtower containers running now. Empty when docker does not answer.
pub fn running(runtime: &dyn ContainerRuntime) -> Vec<Seen> {
    let Ok(snapshot) = runtime.containers_by_project() else {
        return Vec::new();
    };
    let containers: Vec<_> = snapshot
        .projects
        .into_values()
        .flat_map(|p| p.containers)
        .chain(snapshot.standalone)
        .filter(|c| c.state == RuntimeContainerState::Running)
        .collect();
    if containers.is_empty() {
        return Vec::new();
    }
    let ids: Vec<String> = containers.iter().map(|c| c.id.clone()).collect();
    let inspected = runtime.inspect_security(&ids).unwrap_or_default();
    let mut seen = Vec::new();
    for c in containers {
        let found = inspected
            .iter()
            .find(|i| !i.id.is_empty() && (i.id.starts_with(&c.id) || c.id.starts_with(&i.id)));
        let image = found
            .map(|i| i.config.image.clone())
            .filter(|i| !i.is_empty())
            .unwrap_or_else(|| c.image.clone());
        let no_labels = std::collections::BTreeMap::new();
        let labels_of = found.map_or(&no_labels, |i| &i.config.labels);
        if labels::is_watchtower(&image, labels_of, true) {
            seen.push(Seen {
                container_id: c.id,
                image_id: found.map(|i| i.image.clone()).unwrap_or_default(),
                name: c.name.trim_start_matches('/').to_owned(),
                image,
            });
        }
    }
    seen
}

/// The answer that still applies to `w`: same container and same image.
pub fn answer(w: &Seen, acks: &[WatchtowerAck]) -> Option<WatchtowerAnswer> {
    acks.iter()
        .find(|a| a.container_id == w.container_id && a.image_id == w.image_id)
        .map(|a| a.answer)
}

/// What Updates shows.
pub fn view(seen: &[Seen], acks: &[WatchtowerAck]) -> Vec<WatchtowerContainer> {
    seen.iter()
        .map(|w| WatchtowerContainer {
            id: w.container_id.clone(),
            name: w.name.clone(),
            image: w.image.clone(),
            acknowledged: answer(w, acks),
        })
        .collect()
}

/// Auto is paused while a running Watchtower has no answer.
pub fn pause(seen: &[Seen], acks: &[WatchtowerAck]) -> Option<AutoPause> {
    let containers: Vec<String> = seen
        .iter()
        .filter(|w| answer(w, acks).is_none())
        .map(|w| w.name.clone())
        .collect();
    (!containers.is_empty()).then_some(AutoPause::WatchtowerRunning { containers })
}

#[cfg(test)]
mod tests;
