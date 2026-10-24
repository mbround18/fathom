//! T047/T048: what `ListStacks` and `GetStack` send, built from the store's listing and the
//! runtime's containers, and filtered to what the person may see.
//!
//! - A project is the stack of the same name when its `working_dir` label is that stack's
//!   directory, or is missing.
//! - A project named like a stack but run from the same-named directory under another root is
//!   still that stack, with a warning: the stacks directory is not mounted at the same path inside
//!   Depths as on the host, so relative bind mounts resolve differently (research R1, "Path
//!   identity").
//! - Anything else is foreign: no stack directory, or a `working_dir` outside the root. A foreign
//!   project run from a stack's directory under another name (`-p`) carries `elsewhere_of`.
//! - A `user` sees their assigned stacks only, with `foreign`, `standalone` and `warnings` empty.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use depths_core::discovery::ComposeFile;
use depths_core::ports::{
    DefinitionState, RuntimeSnapshot, Stack as CoreStack, StackEntry, StackFiles, StackListing,
};
use depths_core::role::{Access, Action, permits};
use depths_core::status::StackStatus as CoreStatus;
use depths_protocol::{
    ComposeFileName, Container, EnvText, ForeignProject, ServiceSummary, Stack, StackDetail,
    StackList, StackStatus,
};

fn rfc3339(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn file_name(f: ComposeFile) -> ComposeFileName {
    match f {
        ComposeFile::ComposeYaml => ComposeFileName::ComposeYaml,
        ComposeFile::ComposeYml => ComposeFileName::ComposeYml,
        ComposeFile::DockerComposeYml => ComposeFileName::DockerComposeYml,
        ComposeFile::DockerComposeYaml => ComposeFileName::DockerComposeYaml,
    }
}

fn status(s: CoreStatus) -> StackStatus {
    match s {
        CoreStatus::NotDeployed => StackStatus::NotDeployed,
        CoreStatus::Running => StackStatus::Running,
        CoreStatus::Partial => StackStatus::Partial,
        CoreStatus::Stopped => StackStatus::Stopped,
        CoreStatus::Invalid => StackStatus::Invalid,
    }
}

/// Why an entry is `invalid`, when it is.
pub fn invalid_reason(e: &StackEntry) -> Option<String> {
    if !e.valid_name {
        return Some(format!(
            "{:?} is not a valid compose project name (lowercase letters, digits, '_' and '-', \
             starting with a letter or digit)",
            e.name
        ));
    }
    match &e.definition {
        DefinitionState::Unparseable(why) => Some(why.clone()),
        _ => None,
    }
}

/// The wire stack for a store entry joined with its project's containers. A stack whose
/// directory could not be read has no compose file and no hash; it goes out as `compose.yaml`
/// and `""`, with `read_error` saying why.
pub fn stack_of(e: &StackEntry, containers: &[Container]) -> Stack {
    to_wire(
        CoreStack::assemble(e.clone(), containers),
        e.valid_name,
        invalid_reason(e),
    )
}

fn to_wire(s: CoreStack, valid_name: bool, invalid_reason: Option<String>) -> Stack {
    Stack {
        valid_name,
        invalid_reason,
        compose_file: s
            .compose_file
            .map_or(ComposeFileName::ComposeYaml, file_name),
        shadowed_files: s.shadowed_files.into_iter().map(file_name).collect(),
        has_env: s.has_env,
        content_hash: s
            .content_hash
            .map(|h| h.as_str().to_owned())
            .unwrap_or_default(),
        status: status(s.status),
        unhealthy: s.unhealthy,
        services: s
            .services
            .into_iter()
            .map(|v| ServiceSummary {
                name: v.name,
                image: v.image,
                containers: v.containers.into_iter().map(Into::into).collect(),
            })
            .collect(),
        running_services: s.running_services,
        total_services: s.total_services,
        modified_at: s.modified_at.map(rfc3339),
        read_error: s.read_error,
        read_only_reason: s.read_only_reason,
        too_large: s.too_large,
        security: None,
        name: s.name,
    }
}

/// Where a project belongs.
enum Home {
    /// The stack of this name.
    Stack(String),
    /// The stack of this name, run from the same-named directory under another root.
    StackElsewhere(String, String),
    Foreign {
        elsewhere_of: Option<String>,
    },
}

fn home_of(name: &str, working_dir: Option<&str>, root: &Path, stacks: &[&str]) -> Home {
    let is_stack = |n: &str| stacks.contains(&n);
    let Some(wd) = working_dir else {
        return if is_stack(name) {
            Home::Stack(name.to_owned())
        } else {
            Home::Foreign { elsewhere_of: None }
        };
    };
    let wd_path = Path::new(wd);
    let dir_name = wd_path.file_name().and_then(|n| n.to_str());
    let under_root = wd_path.parent() == Some(root);
    match (under_root, dir_name) {
        (true, Some(d)) if d == name && is_stack(name) => Home::Stack(name.to_owned()),
        (true, Some(d)) if is_stack(d) => Home::Foreign {
            elsewhere_of: Some(d.to_owned()),
        },
        (false, Some(d)) if d == name && is_stack(name) => {
            Home::StackElsewhere(name.to_owned(), wd.to_owned())
        }
        _ => Home::Foreign { elsewhere_of: None },
    }
}

/// The full picture, before filtering for the person.
pub fn build(
    listing: &StackListing,
    snapshot: &RuntimeSnapshot,
    stale: bool,
    as_of: String,
) -> StackList {
    let names: Vec<&str> = listing.stacks.iter().map(|e| e.name.as_str()).collect();
    let mut claimed: BTreeMap<&str, Vec<Container>> = BTreeMap::new();
    let mut foreign = Vec::new();
    let mut warnings = Vec::new();
    for (project, pc) in &snapshot.projects {
        match home_of(project, pc.working_dir.as_deref(), &listing.root, &names) {
            Home::Stack(n) => {
                let k = names.iter().find(|x| **x == n).copied().unwrap_or_default();
                claimed.entry(k).or_default().extend(pc.containers.clone());
            }
            Home::StackElsewhere(n, wd) => {
                warnings.push(format!(
                    "stack {n} runs from {wd}, not {}: mount the stacks directory at the same \
                     path inside Depths as on the host, or relative bind mounts resolve \
                     differently",
                    listing.root.join(&n).display()
                ));
                let k = names.iter().find(|x| **x == n).copied().unwrap_or_default();
                claimed.entry(k).or_default().extend(pc.containers.clone());
            }
            Home::Foreign { elsewhere_of } => foreign.push(ForeignProject {
                name: project.clone(),
                working_dir: pc.working_dir.clone(),
                containers: pc.containers.clone(),
                elsewhere_of,
            }),
        }
    }
    for e in &listing.stacks {
        if !e.shadowed_files.is_empty()
            && let Some(f) = e.compose_file
        {
            let others: Vec<&str> = e.shadowed_files.iter().map(|s| s.file_name()).collect();
            warnings.push(format!(
                "stack {} has {} and {}; Compose uses {}",
                e.name,
                f.file_name(),
                others.join(", "),
                f.file_name()
            ));
        }
    }
    let stacks = listing
        .stacks
        .iter()
        .map(|e| {
            let containers = claimed.get(e.name.as_str()).cloned().unwrap_or_default();
            stack_of(e, &containers)
        })
        .collect();
    StackList {
        root: listing.root.display().to_string(),
        stacks,
        foreign,
        standalone: snapshot.standalone.clone(),
        warnings,
        stale,
        as_of,
    }
}

/// Whether `access` may see `name` at all.
pub fn visible(access: &Access, name: &str) -> bool {
    depths_core::StackName::parse(name).is_ok_and(|n| permits(access, Some(&n), Action::View))
}

/// `list` as `access` may see it.
pub fn filter(list: &StackList, access: &Access) -> StackList {
    let foreign_ok = permits(access, None, Action::ViewForeign);
    StackList {
        root: list.root.clone(),
        stacks: list
            .stacks
            .iter()
            .filter(|s| foreign_ok || visible(access, &s.name))
            .cloned()
            .collect(),
        foreign: if foreign_ok {
            list.foreign.clone()
        } else {
            Vec::new()
        },
        standalone: if foreign_ok {
            list.standalone.clone()
        } else {
            Vec::new()
        },
        warnings: if foreign_ok {
            list.warnings.clone()
        } else {
            Vec::new()
        },
        stale: list.stale,
        as_of: list.as_of.clone(),
    }
}

/// `GetStack`: the files, `.env` masked by key.
pub fn detail(stack: Stack, files: Option<StackFiles>) -> StackDetail {
    match files {
        Some(f) => StackDetail {
            content_hash: f.content_hash.as_str().to_owned(),
            env: f.env.map(|t| match depths_core::secrets::mask_env(&t) {
                Some(m) => EnvText::Masked(m),
                None => EnvText::Plain(t),
            }),
            compose_warnings: crate::security::warnings(&f.compose),
            compose_text: Some(f.compose),
            stack,
        },
        None => StackDetail {
            content_hash: stack.content_hash.clone(),
            compose_text: None,
            env: None,
            stack,
            compose_warnings: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use depths_core::ports::ProjectContainers;
    use depths_core::role::Role;
    use depths_protocol::{AdoptionState, RuntimeContainerState};
    use std::path::PathBuf;

    fn entry(name: &str) -> StackEntry {
        StackEntry {
            name: name.into(),
            valid_name: depths_core::valid_name(name),
            compose_file: Some(ComposeFile::ComposeYaml),
            shadowed_files: vec![],
            has_env: false,
            content_hash: None,
            modified_at: None,
            definition: DefinitionState::Parsed(vec![]),
            read_error: None,
            read_only_reason: None,
            too_large: false,
        }
    }

    fn container(project: &str) -> Container {
        Container {
            id: format!("id-{project}"),
            name: format!("{project}-web-1"),
            image: "nginx".into(),
            state: RuntimeContainerState::Running,
            health: None,
            status: "Up".into(),
            created_at: String::new(),
            command: String::new(),
            ports: vec![],
            project: Some(project.into()),
            service: Some("web".into()),
            cluster: None,
            adoption: AdoptionState::NotAdopted,
        }
    }

    fn snap(projects: &[(&str, Option<&str>)]) -> RuntimeSnapshot {
        RuntimeSnapshot {
            projects: projects
                .iter()
                .map(|(p, wd)| {
                    (
                        (*p).to_owned(),
                        ProjectContainers {
                            working_dir: wd.map(str::to_owned),
                            containers: vec![container(p)],
                        },
                    )
                })
                .collect(),
            standalone: vec![container("loose")],
        }
    }

    fn listing(names: &[&str]) -> StackListing {
        StackListing {
            root: PathBuf::from("/opt/stacks"),
            global_env: false,
            stacks: names.iter().map(|n| entry(n)).collect(),
        }
    }

    fn user(stacks: &[&str]) -> Access {
        Access {
            role: Role::User,
            stacks: stacks
                .iter()
                .map(|s| depths_core::StackName::parse(s).unwrap())
                .collect(),
        }
    }

    #[test]
    fn projects_join_their_stacks_and_the_rest_are_foreign() {
        let l = build(
            &listing(&["web", "db", "media"]),
            &snap(&[
                ("web", Some("/opt/stacks/web")),
                ("db", None),
                ("stray", Some("/home/me/stray")),
                ("outside", Some("/srv/other/outside")),
                ("handrun", Some("/opt/stacks/media")),
            ]),
            false,
            "t".into(),
        );
        let s = |n: &str| l.stacks.iter().find(|s| s.name == n).unwrap();
        assert_eq!(s("web").running_services, 1);
        assert_eq!(s("db").services.len(), 1);
        // `media`'s directory runs under another project name: not claimed by the stack.
        assert!(s("media").services.is_empty());
        let f = |n: &str| l.foreign.iter().find(|f| f.name == n).unwrap();
        assert_eq!(f("stray").elsewhere_of, None);
        assert_eq!(f("outside").elsewhere_of, None);
        assert_eq!(f("handrun").elsewhere_of.as_deref(), Some("media"));
        assert_eq!(l.foreign.len(), 3);
        assert_eq!(l.standalone.len(), 1);
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn a_stack_run_from_another_root_is_claimed_with_a_warning() {
        let l = build(
            &listing(&["web"]),
            &snap(&[("web", Some("/srv/stacks/web"))]),
            false,
            "t".into(),
        );
        assert_eq!(l.stacks[0].running_services, 1);
        assert!(l.foreign.is_empty());
        assert_eq!(l.warnings.len(), 1);
        assert!(
            l.warnings[0].contains("/srv/stacks/web"),
            "{:?}",
            l.warnings
        );
    }

    #[test]
    fn a_user_never_receives_an_unassigned_stack_or_host_detail() {
        let l = build(
            &listing(&["web", "db"]),
            &snap(&[("web", None), ("stray", None), ("db", Some("/x/db"))]),
            false,
            "t".into(),
        );
        assert!(!l.warnings.is_empty());
        let mine = filter(&l, &user(&["web", "ghost"]));
        assert_eq!(
            mine.stacks
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["web"]
        );
        assert!(mine.foreign.is_empty() && mine.standalone.is_empty() && mine.warnings.is_empty());
        let admin = Access {
            role: Role::Admin,
            ..Default::default()
        };
        assert_eq!(filter(&l, &admin), l);
        assert!(filter(&l, &Access::default()).stacks.is_empty());
    }

    #[test]
    fn an_invalid_stack_says_why() {
        let mut bad = entry("web");
        bad.definition = DefinitionState::Unparseable("line 3: mapping expected".into());
        let s = stack_of(&bad, &[]);
        assert_eq!(s.status, StackStatus::Invalid);
        assert_eq!(
            s.invalid_reason.as_deref(),
            Some("line 3: mapping expected")
        );
        let s = stack_of(&entry("My App"), &[]);
        assert!(!s.valid_name && s.invalid_reason.unwrap().contains("My App"));
        assert_eq!(stack_of(&entry("ok"), &[]).invalid_reason, None);
    }

    #[test]
    fn shadowed_compose_files_warn() {
        let mut ls = listing(&["web"]);
        ls.stacks[0].shadowed_files = vec![ComposeFile::DockerComposeYml];
        let l = build(&ls, &RuntimeSnapshot::default(), false, "t".into());
        assert!(l.warnings[0].contains("docker-compose.yml"));
    }

    #[test]
    fn detail_masks_secret_env_values() {
        let stack = stack_of(&entry("web"), &[]);
        let files = StackFiles {
            compose_file: ComposeFile::ComposeYaml,
            compose: "services: {}\n".into(),
            env: Some("DB_PASSWORD=hunter2\n".into()),
            content_hash: depths_core::ContentHash::parse(&"a".repeat(64)).unwrap(),
        };
        let d = detail(stack, Some(files));
        assert_eq!(
            d.env,
            Some(EnvText::Masked("DB_PASSWORD=********\n".into()))
        );
        assert!(!serde_json::to_string(&d).unwrap().contains("hunter2"));
    }
}
