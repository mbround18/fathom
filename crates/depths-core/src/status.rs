//! A stack's status, derived from its containers (R2, data-model.md "StackStatus").
//!
//! The input is the 048 `Container` rows labelled `com.docker.compose.project = <stack>` (from one
//! `docker ps -a`), never `compose ls`'s summary string: that string is what makes Dockge call a
//! stack "exited" when a one-shot init job finishes (dockge #177, #806).
//!
//! The rule that fixes it: a container that exited with code 0 and whose restart policy is `no`
//! has **completed**. It is not long-running, so it never makes a stack partial. A `restarting`
//! container counts as not running.
//!
//! Restart policies come from the compose file, because `docker ps` does not report them; a
//! service the file does not mention (an orphan, or a file edited since the deploy) gets
//! Compose's default, `no`.

use container_runtime::wire::{Container, ContainerHealth, RuntimeContainerState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackStatus {
    /// Files, no containers.
    NotDeployed,
    /// Every long-running service is up.
    Running,
    /// Some long-running services up, some not.
    Partial,
    /// No long-running service is up.
    Stopped,
    /// The name is not a valid project name or the compose file does not parse. Never acted on.
    Invalid,
}

/// A service's `restart` policy, as Compose reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    No,
    Always,
    OnFailure,
    UnlessStopped,
}

/// One service of a parsed compose file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub image: Option<String>,
    pub restart: RestartPolicy,
}

/// The services of a compose file's text, in file order.
///
/// Only what status and the service list need: names, `image`, and `restart` (or, when that is
/// unset, `deploy.restart_policy.condition`, which Compose also honours). A file that is not a
/// YAML mapping, or whose `services` is not one, is an error carrying the parser's message.
pub fn parse_services(compose_text: &str) -> Result<Vec<ServiceSpec>, String> {
    use serde_yaml::Value;

    let doc: Value = serde_yaml::from_str(compose_text).map_err(|e| e.to_string())?;
    let root = match &doc {
        Value::Mapping(m) => m,
        Value::Null => return Err("the compose file is empty".to_owned()),
        _ => return Err("the compose file is not a mapping".to_owned()),
    };
    let services = match root.get("services") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Mapping(m)) => m,
        Some(_) => return Err("`services` is not a mapping".to_owned()),
    };
    services
        .iter()
        .map(|(key, body)| {
            let name = key
                .as_str()
                .ok_or_else(|| "a service name is not a string".to_owned())?
                .to_owned();
            let body = match body {
                Value::Mapping(m) => Some(m),
                Value::Null => None,
                _ => return Err(format!("service `{name}` is not a mapping")),
            };
            let image = body
                .and_then(|b| b.get("image"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let restart = body.map_or(Ok(RestartPolicy::No), |b| restart_policy(&name, b))?;
            Ok(ServiceSpec {
                name,
                image,
                restart,
            })
        })
        .collect()
}

fn restart_policy(service: &str, body: &serde_yaml::Mapping) -> Result<RestartPolicy, String> {
    use serde_yaml::Value;

    match body.get("restart") {
        // YAML 1.1 reads a bare `no` as false; Compose accepts it either way.
        Some(Value::Bool(false)) | Some(Value::Null) => Ok(RestartPolicy::No),
        Some(Value::String(s)) => {
            let policy = s.split(':').next().unwrap_or_default();
            match policy {
                "no" | "" => Ok(RestartPolicy::No),
                "always" => Ok(RestartPolicy::Always),
                "on-failure" => Ok(RestartPolicy::OnFailure),
                "unless-stopped" => Ok(RestartPolicy::UnlessStopped),
                other => Err(format!(
                    "service `{service}` has an unknown restart policy `{other}`"
                )),
            }
        }
        Some(_) => Err(format!("service `{service}` has a restart policy that is not a string")),
        None => {
            let condition = body
                .get("deploy")
                .and_then(|d| d.get("restart_policy"))
                .and_then(|r| r.get("condition"))
                .and_then(Value::as_str);
            Ok(match condition {
                Some("any") => RestartPolicy::Always,
                Some("on-failure") => RestartPolicy::OnFailure,
                _ => RestartPolicy::No,
            })
        }
    }
}

/// What is known about a stack's definition when its status is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Definition<'a> {
    /// The compose file parsed.
    Services(&'a [ServiceSpec]),
    /// The file was not loaded (too large, say): derive from the containers alone.
    Unknown,
    /// The name is invalid or the file does not parse.
    Invalid,
}

/// A stack's derived status and counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSummary {
    pub status: StackStatus,
    /// Any container's health check reports `unhealthy`.
    pub unhealthy: bool,
    /// Long-running services that are up.
    pub running_services: u32,
    /// Long-running services: every service except those whose containers all completed.
    pub total_services: u32,
}

/// The exit code in a runtime status line such as `Exited (137) 2 minutes ago`.
pub fn exit_code(status_line: &str) -> Option<i32> {
    let rest = status_line.trim_start().strip_prefix("Exited (")?;
    let (code, _) = rest.split_once(')')?;
    code.parse().ok()
}

fn completed(c: &Container, policy: RestartPolicy) -> bool {
    c.state == RuntimeContainerState::Exited
        && policy == RestartPolicy::No
        && exit_code(&c.status) == Some(0)
}

/// Derives a stack's status from its definition and the containers of its project.
///
/// `containers` must already be the project's (grouped by the compose project label).
pub fn derive_status(definition: Definition<'_>, containers: &[Container]) -> StatusSummary {
    let unhealthy = containers
        .iter()
        .any(|c| c.health == Some(ContainerHealth::Unhealthy));
    let defined: &[ServiceSpec] = match definition {
        Definition::Services(s) => s,
        Definition::Unknown => &[],
        Definition::Invalid => {
            return StatusSummary {
                status: StackStatus::Invalid,
                unhealthy,
                running_services: 0,
                total_services: 0,
            };
        }
    };

    // Every service the file defines, then any the containers name that it does not.
    let mut services: Vec<(&str, RestartPolicy)> = defined
        .iter()
        .map(|s| (s.name.as_str(), s.restart))
        .collect();
    for c in containers {
        let name = c.service.as_deref().unwrap_or(c.name.as_str());
        if !services.iter().any(|(n, _)| *n == name) {
            services.push((name, RestartPolicy::No));
        }
    }

    let mut running = 0u32;
    let mut long_running = 0u32;
    for (name, policy) in &services {
        let mine: Vec<&Container> = containers
            .iter()
            .filter(|c| c.service.as_deref().unwrap_or(c.name.as_str()) == *name)
            .collect();
        let live: Vec<&&Container> = mine.iter().filter(|c| !completed(c, *policy)).collect();
        if !mine.is_empty() && live.is_empty() {
            continue; // Completed: a one-shot job that finished cleanly.
        }
        long_running += 1;
        if !live.is_empty()
            && live
                .iter()
                .all(|c| c.state == RuntimeContainerState::Running)
        {
            running += 1;
        }
    }

    let status = if containers.is_empty() {
        StackStatus::NotDeployed
    } else if running == 0 {
        // Including a stack whose every container completed: nothing is running.
        StackStatus::Stopped
    } else if running == long_running {
        StackStatus::Running
    } else {
        StackStatus::Partial
    };
    StatusSummary {
        status,
        unhealthy,
        running_services: running,
        total_services: long_running,
    }
}

#[cfg(test)]
mod tests {
    use container_runtime::wire::AdoptionState;

    use super::*;

    fn container(service: &str, state: RuntimeContainerState, status: &str) -> Container {
        Container {
            id: format!("{service}-id"),
            name: format!("depths-test-{service}-1"),
            image: "busybox".into(),
            state,
            health: None,
            status: status.into(),
            created_at: "2026-10-23 12:00:00 +0000 UTC".into(),
            command: "sleep 1d".into(),
            ports: vec![],
            project: Some("depths-test".into()),
            service: Some(service.into()),
            cluster: None,
            adoption: AdoptionState::NotAdopted,
        }
    }

    fn up(service: &str) -> Container {
        container(service, RuntimeContainerState::Running, "Up 3 hours")
    }

    fn exited(service: &str, code: i32) -> Container {
        container(
            service,
            RuntimeContainerState::Exited,
            &format!("Exited ({code}) 2 minutes ago"),
        )
    }

    fn spec(name: &str, restart: RestartPolicy) -> ServiceSpec {
        ServiceSpec {
            name: name.into(),
            image: Some("busybox".into()),
            restart,
        }
    }

    fn web_db() -> Vec<ServiceSpec> {
        vec![
            spec("web", RestartPolicy::UnlessStopped),
            spec("db", RestartPolicy::Always),
        ]
    }

    fn derive(services: &[ServiceSpec], containers: &[Container]) -> StatusSummary {
        derive_status(Definition::Services(services), containers)
    }

    #[test]
    fn no_containers_is_not_deployed() {
        let s = derive(&web_db(), &[]);
        assert_eq!(s.status, StackStatus::NotDeployed);
        assert_eq!((s.running_services, s.total_services), (0, 2));
    }

    #[test]
    fn every_service_up_is_running() {
        let s = derive(&web_db(), &[up("web"), up("db")]);
        assert_eq!(s.status, StackStatus::Running);
        assert_eq!((s.running_services, s.total_services), (2, 2));
    }

    #[test]
    fn some_up_is_partial() {
        let s = derive(&web_db(), &[up("web"), exited("db", 1)]);
        assert_eq!(s.status, StackStatus::Partial);
        assert_eq!((s.running_services, s.total_services), (1, 2));
    }

    #[test]
    fn a_defined_service_with_no_container_makes_a_stack_partial() {
        let s = derive(&web_db(), &[up("web")]);
        assert_eq!(s.status, StackStatus::Partial);
        assert_eq!((s.running_services, s.total_services), (1, 2));
    }

    #[test]
    fn none_up_is_stopped() {
        let s = derive(&web_db(), &[exited("web", 137), exited("db", 0)]);
        assert_eq!(s.status, StackStatus::Stopped);
        assert_eq!((s.running_services, s.total_services), (0, 2));
    }

    #[test]
    fn an_invalid_definition_is_invalid_whatever_the_containers() {
        let s = derive_status(Definition::Invalid, &[up("web")]);
        assert_eq!(s.status, StackStatus::Invalid);
    }

    #[test]
    fn a_one_shot_job_that_exited_zero_with_restart_no_is_completed_not_partial() {
        let services = vec![
            spec("web", RestartPolicy::UnlessStopped),
            spec("migrate", RestartPolicy::No),
        ];
        let s = derive(&services, &[up("web"), exited("migrate", 0)]);
        assert_eq!(s.status, StackStatus::Running);
        assert_eq!((s.running_services, s.total_services), (1, 1));
    }

    #[test]
    fn a_completed_job_needs_both_exit_zero_and_restart_no() {
        // Non-zero exit with restart `no`: a failed job, so the stack is partial.
        let services = vec![
            spec("web", RestartPolicy::Always),
            spec("migrate", RestartPolicy::No),
        ];
        let s = derive(&services, &[up("web"), exited("migrate", 1)]);
        assert_eq!(s.status, StackStatus::Partial);

        // Exit 0 under a restart policy: it is meant to be running.
        let services = vec![
            spec("web", RestartPolicy::Always),
            spec("worker", RestartPolicy::OnFailure),
        ];
        let s = derive(&services, &[up("web"), exited("worker", 0)]);
        assert_eq!(s.status, StackStatus::Partial);
    }

    #[test]
    fn a_stack_of_completed_jobs_only_is_stopped() {
        let services = vec![spec("backup", RestartPolicy::No)];
        let s = derive(&services, &[exited("backup", 0)]);
        assert_eq!(s.status, StackStatus::Stopped);
        assert_eq!((s.running_services, s.total_services), (0, 0));
    }

    #[test]
    fn restarting_counts_as_not_running() {
        let restarting = container(
            "db",
            RuntimeContainerState::Restarting,
            "Restarting (1) 5 seconds ago",
        );
        let s = derive(&web_db(), &[up("web"), restarting.clone()]);
        assert_eq!(s.status, StackStatus::Partial);
        let only = vec![spec("db", RestartPolicy::Always)];
        assert_eq!(derive(&only, &[restarting]).status, StackStatus::Stopped);
    }

    #[test]
    fn unhealthy_is_a_separate_flag() {
        let mut sick = up("db");
        sick.health = Some(ContainerHealth::Unhealthy);
        let s = derive(&web_db(), &[up("web"), sick]);
        assert_eq!(s.status, StackStatus::Running);
        assert!(s.unhealthy);

        let mut ok = up("db");
        ok.health = Some(ContainerHealth::Healthy);
        let mut starting = up("web");
        starting.health = Some(ContainerHealth::Starting);
        assert!(!derive(&web_db(), &[starting, ok]).unhealthy);
    }

    #[test]
    fn a_scaled_service_is_up_only_when_every_replica_is() {
        let services = vec![spec("web", RestartPolicy::Always)];
        let s = derive(&services, &[up("web"), exited("web", 1)]);
        assert_eq!(s.status, StackStatus::Stopped);
        assert_eq!((s.running_services, s.total_services), (0, 1));
    }

    #[test]
    fn an_orphan_container_counts_with_composes_default_policy() {
        let services = vec![spec("web", RestartPolicy::Always)];
        let s = derive(&services, &[up("web"), up("old")]);
        assert_eq!((s.running_services, s.total_services), (2, 2));
        let s = derive(&services, &[up("web"), exited("old", 0)]);
        assert_eq!(s.status, StackStatus::Running, "an exited-0 orphan completed");
    }

    #[test]
    fn an_unknown_definition_derives_from_containers_alone() {
        let s = derive_status(Definition::Unknown, &[up("web"), exited("db", 2)]);
        assert_eq!(s.status, StackStatus::Partial);
        assert_eq!((s.running_services, s.total_services), (1, 2));
    }

    #[test]
    fn exit_codes_are_read_from_the_status_line() {
        assert_eq!(exit_code("Exited (0) 2 minutes ago"), Some(0));
        assert_eq!(exit_code("Exited (137) 1 second ago"), Some(137));
        assert_eq!(exit_code("Up 3 hours"), None);
        assert_eq!(exit_code("Exited (x) now"), None);
        assert_eq!(exit_code(""), None);
    }

    #[test]
    fn services_parse_with_their_restart_policies() {
        let text = "\
# a comment
services:
  web:
    image: nginx
    restart: unless-stopped
  migrate:
    image: busybox
    restart: \"no\"
  bare:
    image: busybox
    restart: no
  retry:
    image: busybox
    restart: on-failure:3
  swarmish:
    image: busybox
    deploy:
      restart_policy:
        condition: any
  empty:
";
        let got = parse_services(text).unwrap();
        let summary: Vec<(&str, Option<&str>, RestartPolicy)> = got
            .iter()
            .map(|s| (s.name.as_str(), s.image.as_deref(), s.restart))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("web", Some("nginx"), RestartPolicy::UnlessStopped),
                ("migrate", Some("busybox"), RestartPolicy::No),
                ("bare", Some("busybox"), RestartPolicy::No),
                ("retry", Some("busybox"), RestartPolicy::OnFailure),
                ("swarmish", Some("busybox"), RestartPolicy::Always),
                ("empty", None, RestartPolicy::No),
            ]
        );
    }

    #[test]
    fn unparseable_compose_files_are_errors() {
        assert!(parse_services("services: [").is_err());
        assert!(parse_services("").is_err());
        assert!(parse_services("- a\n- b\n").is_err());
        assert!(parse_services("services: 3\n").is_err());
        assert!(parse_services("services:\n  web: 3\n").is_err());
        assert!(parse_services("services:\n  web:\n    restart: sometimes\n").is_err());
        assert_eq!(parse_services("name: x\n"), Ok(vec![]));
    }
}
