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
    /// The compose `user:`, when set (a service with one is never held for root, research R4).
    pub user: Option<String>,
    /// The compose `platform:`, when set (the pull passes it).
    pub platform: Option<String>,
    /// The container Depths runs in.
    pub is_depths: bool,
    /// Containers of the service.
    pub replicas: u32,
    /// The image the (first) container runs; `None` without a container.
    pub running: Option<ImageInfo>,
    /// The container's image has no name left (a pull moved its tag to a newer image) and the
    /// daemon no longer describes it: `running` holds only its id.
    pub unnamed_image: bool,
}

pub trait Inputs: Send + Sync {
    /// Every stack that can be acted on.
    fn stacks(&self) -> Vec<String>;
    /// The daemon's platform and image store; `None` when docker does not answer.
    fn host(&self) -> Option<HostPlatform>;
    /// A stack's services; empty when its compose file cannot be read.
    fn services(&self, stack: &str) -> Vec<ServiceInput>;
    /// The Watchtower containers running on the host (US4).
    fn watchtower(&self) -> Vec<super::watchtower::Seen> {
        Vec::new()
    }
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
        // Each service's image id, and its container's user.
        let mut image_of: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut labels_from_container = std::collections::BTreeSet::new();
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
            let Some(i) = inspected
                .iter()
                .find(|i| !i.id.is_empty() && (i.id.starts_with(&c.id) || c.id.starts_with(&i.id)))
            else {
                continue;
            };
            // The running container's labels are what Watchtower reads; the compose file's
            // are the fallback when no container runs (T073).
            if !labels_from_container.contains(service)
                && let Some(svc) = services.iter_mut().find(|s| &s.service == service)
            {
                svc.labels = i.config.labels.clone();
                labels_from_container.insert(service.clone());
            }
            if !i.image.is_empty() {
                image_of
                    .entry(service.clone())
                    .or_insert((i.image.clone(), i.config.user.clone()));
            }
        }
        let image_ids: Vec<String> = {
            let mut v: Vec<String> = image_of.values().map(|(id, _)| id.clone()).collect();
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
            if let Some((id, user)) = image_of.get(&svc.service) {
                let (running, unnamed) = running_image(&images, id, user);
                svc.running = Some(running);
                svc.unnamed_image = unnamed;
            }
        }
        services
    }

    fn watchtower(&self) -> Vec<super::watchtower::Seen> {
        super::watchtower::running(&*self.runtime)
    }
}

/// The image a container runs, by the container's image id, never by name: after someone pulls a
/// newer image by hand, the tag names the new image and the container's own image may have no
/// name left. Under the containerd image store `docker image inspect` no longer finds such an
/// image, so the id alone stands for it (`true`): the check still compares it with the tag. Its
/// user is then the container's `Config.User`, so a root escalation is still held.
pub fn running_image(images: &[ImageInfo], id: &str, container_user: &str) -> (ImageInfo, bool) {
    let found = images.iter().find(|i| {
        i.id == id
            || i.repo_digests
                .iter()
                .any(|d| d.rsplit_once('@').is_some_and(|(_, d)| d == id))
    });
    match found {
        Some(i) => (i.clone(), false),
        None => (
            ImageInfo {
                id: id.to_owned(),
                user: Some(container_user.to_owned()).filter(|u| !u.trim().is_empty()),
                ..ImageInfo::default()
            },
            true,
        ),
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
            user: text(s.get("user")),
            platform: text(s.get("platform")),
            ..ServiceInput::default()
        })
        .collect()
}

/// A non-empty string field; a numeric `user:` is read too.
fn text(v: Option<&serde_json::Value>) -> Option<String> {
    let s = match v? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.trim().is_empty()).then_some(s)
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
                "listed":{"image":"redis","labels":["a=b","c"],"user":"1000:1000","platform":"linux/arm64"}
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
        assert_eq!(listed.user.as_deref(), Some("1000:1000"));
        assert_eq!(listed.platform.as_deref(), Some("linux/arm64"));
        assert_eq!(app.user, None);
        assert!(parse_compose("not json").is_empty());
    }

    #[test]
    fn the_running_image_is_found_by_the_containers_image_id() {
        let named = ImageInfo {
            id: "sha256:aaaa".into(),
            repo_tags: vec!["nginx:1.27".into()],
            repo_digests: vec!["nginx@sha256:bbbb".into()],
            user: Some("101".into()),
            ..ImageInfo::default()
        };
        let images = [named.clone()];
        assert_eq!(running_image(&images, "sha256:aaaa", "0"), (named.clone(), false));
        // Under the containerd store the id is the digest it was pulled by.
        assert_eq!(running_image(&images, "sha256:bbbb", "0"), (named, false));

        // A pull moved the tag away and the daemon no longer describes the image: the id stands
        // for it, with the container's user.
        let (gone, unnamed) = running_image(&images, "sha256:cccc", "1000");
        assert!(unnamed);
        assert_eq!(gone.id, "sha256:cccc");
        assert!(gone.repo_tags.is_empty() && gone.repo_digests.is_empty());
        assert_eq!(gone.user.as_deref(), Some("1000"));
        assert_eq!(running_image(&[], "sha256:cccc", "").0.user, None);
    }

    // --- label source (T070) -------------------------------------------------------------------

    use depths_core::plan::RunStep;
    use depths_core::ports::{
        CancelToken, OutputSink, ProjectContainers, RuntimeError, RuntimeSnapshot, StepOutcome,
        Validation,
    };
    use depths_core::security::{Inspect, InspectConfig};
    use depths_protocol::{AdoptionState, Container, RuntimeContainerState};

    const ENABLE: &str = "com.centurylinklabs.watchtower.enable";

    /// `app` has a running container whose labels say `false`; `db` has no container.
    struct Fake;

    impl ContainerRuntime for Fake {
        fn compose_config(&self, _: &RunStep) -> Result<String, RuntimeError> {
            Ok(format!(
                r#"{{"services":{{
                    "app":{{"image":"nginx","labels":{{"{ENABLE}":"true"}}}},
                    "db":{{"image":"postgres","labels":{{"{ENABLE}":"false"}}}}
                }}}}"#
            ))
        }
        fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
            let mut snap = RuntimeSnapshot::default();
            snap.projects.insert(
                "web".into(),
                ProjectContainers {
                    working_dir: None,
                    containers: vec![Container {
                        id: "c-app".into(),
                        name: "/web-app-1".into(),
                        image: "nginx".into(),
                        state: RuntimeContainerState::Running,
                        health: None,
                        status: String::new(),
                        created_at: String::new(),
                        command: String::new(),
                        ports: vec![],
                        project: Some("web".into()),
                        service: Some("app".into()),
                        cluster: None,
                        adoption: AdoptionState::NotAdopted,
                    }],
                },
            );
            Ok(snap)
        }
        fn inspect_security(&self, _: &[String]) -> Result<Vec<Inspect>, RuntimeError> {
            Ok(vec![Inspect {
                id: "c-app".into(),
                image: "sha256:run".into(),
                config: InspectConfig {
                    image: "nginx".into(),
                    labels: [(ENABLE.to_owned(), "false".to_owned())].into(),
                    ..InspectConfig::default()
                },
                ..Inspect::default()
            }])
        }
        fn run_step(
            &self,
            _: &RunStep,
            _: Option<&[u8]>,
            _: &dyn OutputSink,
            _: &CancelToken,
        ) -> Result<StepOutcome, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
            Ok(Validation::Valid)
        }
    }

    #[test]
    fn labels_come_from_the_running_container_and_from_compose_without_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("web")).unwrap();
        std::fs::write(
            dir.path().join("web/compose.yaml"),
            "services:\n  app:\n    image: nginx\n",
        )
        .unwrap();
        let inputs = RuntimeInputs {
            store: Arc::new(depths_stacks_fs::FsStackStore::open(dir.path()).unwrap()),
            runtime: Arc::new(Fake),
            hostname: None,
        };
        let got = inputs.services("web");
        let label = |svc: &str| {
            got.iter()
                .find(|s| s.service == svc)
                .and_then(|s| s.labels.get(ENABLE).cloned())
        };
        assert_eq!(
            label("app").as_deref(),
            Some("false"),
            "the container's labels win (what Watchtower reads)"
        );
        assert_eq!(
            label("db").as_deref(),
            Some("false"),
            "compose, no container"
        );
        assert_eq!(got.iter().find(|s| s.service == "app").unwrap().replicas, 1);
    }
}
