//! What a check pass reads from the host: the stacks, each service's compose `image:`, build
//! section and labels (`compose config --format json`, research R1 step 1), the image its
//! container runs (`docker inspect` then `docker image inspect`), and the daemon's platform.
//!
//! Behind [`Inputs`] so scheduler tests run without docker. Every method blocks; the scheduler
//! calls them on the blocking pool.

use std::collections::BTreeMap;
use std::sync::Arc;

use depths_core::StackName;
use depths_core::plan::{self, StackContext};
use depths_core::ports::{ContainerRuntime, HostPlatform, ImageInfo, StackStore};

/// One compose service, with what its container runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceInput {
    pub service: String,
    /// The compose `image:` as written (after interpolation).
    pub image: Option<String>,
    pub has_build: bool,
    pub labels: BTreeMap<String, String>,
    /// The container Depths runs in.
    pub is_depths: bool,
    /// Containers of the service.
    pub replicas: u32,
    /// The image the (first) container runs; `None` without a container.
    pub running: Option<ImageInfo>,
}

pub trait Inputs: Send + Sync {
    /// Every stack that can be acted on.
    fn stacks(&self) -> Vec<String>;
    /// The daemon's platform and image store; `None` when docker does not answer.
    fn host(&self) -> Option<HostPlatform>;
    /// A stack's services; empty when its compose file cannot be read.
    fn services(&self, stack: &str) -> Vec<ServiceInput>;
}

/// The production inputs: the stacks directory and the docker CLI.
pub struct RuntimeInputs {
    pub store: Arc<dyn StackStore>,
    pub runtime: Arc<dyn ContainerRuntime>,
    /// `$HOSTNAME`: Depths' own container id prefix, inside a container.
    pub hostname: Option<String>,
}

impl Inputs for RuntimeInputs {
    fn stacks(&self) -> Vec<String> {
        match self.store.list() {
            Ok(l) => l
                .stacks
                .into_iter()
                .filter(|e| e.actionable())
                .map(|e| e.name)
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "update check: cannot list stacks");
                Vec::new()
            }
        }
    }

    fn host(&self) -> Option<HostPlatform> {
        match self.runtime.host_platform() {
            Ok(h) => Some(h),
            Err(e) => {
                tracing::warn!(error = %e, "update check: docker info failed");
                None
            }
        }
    }

    fn services(&self, stack: &str) -> Vec<ServiceInput> {
        let Ok(name) = StackName::parse(stack) else {
            return Vec::new();
        };
        let Some(entry) = self.store.entry(&name).ok().filter(|e| e.actionable()) else {
            return Vec::new();
        };
        let ctx = StackContext {
            stacks_root: self.store.root(),
            stack: &name,
            global_env: self.store.global_env(),
            has_env: entry.has_env,
        };
        let config = match self.runtime.compose_config(&plan::compose_config(&ctx)) {
            Ok(out) => out,
            Err(e) => {
                tracing::warn!(stack, error = %e, "update check: compose config failed");
                return Vec::new();
            }
        };
        let mut services = parse_compose(&config);

        // What each service's container runs.
        let containers = self
            .runtime
            .containers_by_project()
            .ok()
            .and_then(|mut s| s.projects.remove(stack))
            .map(|p| p.containers)
            .unwrap_or_default();
        let ids: Vec<String> = containers.iter().map(|c| c.id.clone()).collect();
        let inspected = if ids.is_empty() {
            Vec::new()
        } else {
            self.runtime.inspect_security(&ids).unwrap_or_default()
        };
        let mut image_of: BTreeMap<String, String> = BTreeMap::new();
        for c in &containers {
            let Some(service) = &c.service else { continue };
            if let Some(svc) = services.iter_mut().find(|s| &s.service == service) {
                svc.replicas += 1;
                if self
                    .hostname
                    .as_deref()
                    .is_some_and(|h| !h.is_empty() && c.id.starts_with(h))
                {
                    svc.is_depths = true;
                }
            }
            if let Some(i) = inspected
                .iter()
                .find(|i| !i.id.is_empty() && (i.id.starts_with(&c.id) || c.id.starts_with(&i.id)))
                && !i.image.is_empty()
            {
                image_of.entry(service.clone()).or_insert(i.image.clone());
            }
        }
        let image_ids: Vec<String> = {
            let mut v: Vec<String> = image_of.values().cloned().collect();
            v.sort();
            v.dedup();
            v
        };
        let images = if image_ids.is_empty() {
            Vec::new()
        } else {
            self.runtime.image_inspect(&image_ids).unwrap_or_default()
        };
        for svc in &mut services {
            if let Some(id) = image_of.get(&svc.service) {
                svc.running = images.iter().find(|i| &i.id == id).cloned();
            }
        }
        services
    }
}

/// The services of `compose config --format json`.
pub fn parse_compose(json: &str) -> Vec<ServiceInput> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(services) = v.get("services").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    services
        .iter()
        .map(|(name, s)| ServiceInput {
            service: name.clone(),
            image: s.get("image").and_then(|i| i.as_str()).map(str::to_owned),
            has_build: s.get("build").is_some_and(|b| !b.is_null()),
            labels: labels(s.get("labels")),
            ..ServiceInput::default()
        })
        .collect()
}

/// Compose prints labels as a map; a list (`KEY=value`) is read too.
fn labels(v: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    match v {
        Some(serde_json::Value::Object(m)) => m
            .iter()
            .map(|(k, v)| {
                let v = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                (k.clone(), v)
            })
            .collect(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|e| e.as_str())
            .map(|e| match e.split_once('=') {
                Some((k, v)) => (k.to_owned(), v.to_owned()),
                None => (e.to_owned(), String::new()),
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_services_with_image_build_and_labels() {
        let got = parse_compose(
            r#"{"name":"web","services":{
                "app":{"image":"nginx:1.27","labels":{"com.centurylinklabs.watchtower.enable":"false"}},
                "built":{"build":{"context":"."},"image":"web-built"},
                "listed":{"image":"redis","labels":["a=b","c"]}
            }}"#,
        );
        assert_eq!(got.len(), 3);
        let app = got.iter().find(|s| s.service == "app").unwrap();
        assert_eq!(app.image.as_deref(), Some("nginx:1.27"));
        assert!(!app.has_build);
        assert_eq!(
            app.labels
                .get("com.centurylinklabs.watchtower.enable")
                .map(String::as_str),
            Some("false")
        );
        assert!(got.iter().find(|s| s.service == "built").unwrap().has_build);
        let listed = got.iter().find(|s| s.service == "listed").unwrap();
        assert_eq!(listed.labels.get("a").map(String::as_str), Some("b"));
        assert_eq!(listed.labels.get("c").map(String::as_str), Some(""));
        assert!(parse_compose("not json").is_empty());
    }
}
