//! What an operation runs: the argv table of R2, as data.
//!
//! The preview shows these steps and the confirm runs exactly them, so the plan is the one place
//! an operation becomes commands. Every command is `docker compose ...` with cwd = the stack
//! directory and no `-f`, the same project Dockge and a terminal see (R1). Every argv is a
//! `Vec<String>` handed to a runner; nothing here builds a shell line (T095), and every name in
//! an argv arrived as a [`StackName`] or [`ServiceName`], which cannot start with `-`.
//!
//! `[env-files]`: when `<stacks>/global.env` exists, every command gets
//! `--env-file ../global.env`, plus `--env-file ./.env` when `.env` exists, because any explicit
//! `--env-file` turns off Compose's automatic `.env` (R1). With no `global.env`, no flag at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::discovery::{ENV_FILE, GLOBAL_ENV_FILE};
use crate::name::{ServiceName, StackName};
use crate::role::Action;

/// The runtime binary every step runs.
pub const DOCKER: &str = "docker";

/// An operation as requested, with its options (data-model.md "Operation").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    Up,
    Stop,
    Restart,
    Down {
        remove_volumes: bool,
        remove_images: bool,
    },
    Pull,
    /// Pull, then up: one preview, two commands.
    Update,
    /// Write the draft to an existing stack, then `up` when `deploy`.
    Save {
        deploy: bool,
        /// Whether `.env` exists once the draft is written (see `StackDraft::env_exists_after`).
        env_after_write: bool,
    },
    /// Create the stack directory with the draft, then `up` when `deploy`.
    Create {
        deploy: bool,
        env_after_write: bool,
    },
    Delete {
        remove_directory: bool,
    },
    ServiceUp {
        service: ServiceName,
    },
    ServiceStop {
        service: ServiceName,
    },
    ServiceRestart {
        service: ServiceName,
    },
}

/// The operation kinds, as the protocol and the audit log name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Up,
    Stop,
    Restart,
    Down,
    Pull,
    Update,
    Save,
    Create,
    Delete,
    ServiceUp,
    ServiceStop,
    ServiceRestart,
}

impl Operation {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Up => OperationKind::Up,
            Self::Stop => OperationKind::Stop,
            Self::Restart => OperationKind::Restart,
            Self::Down { .. } => OperationKind::Down,
            Self::Pull => OperationKind::Pull,
            Self::Update => OperationKind::Update,
            Self::Save { .. } => OperationKind::Save,
            Self::Create { .. } => OperationKind::Create,
            Self::Delete { .. } => OperationKind::Delete,
            Self::ServiceUp { .. } => OperationKind::ServiceUp,
            Self::ServiceStop { .. } => OperationKind::ServiceStop,
            Self::ServiceRestart { .. } => OperationKind::ServiceRestart,
        }
    }

    /// The permission-table row this operation needs (`role::permits`).
    pub fn required_action(&self) -> Action {
        match self {
            Self::Down {
                remove_volumes,
                remove_images,
            } if *remove_volumes || *remove_images => Action::DownDestructive,
            Self::Up
            | Self::Stop
            | Self::Restart
            | Self::Down { .. }
            | Self::Pull
            | Self::Update
            | Self::ServiceUp { .. }
            | Self::ServiceStop { .. }
            | Self::ServiceRestart { .. } => Action::Lifecycle,
            Self::Save { .. } => Action::Edit,
            Self::Create { .. } => Action::CreateStack,
            Self::Delete { .. } => Action::DeleteStack,
        }
    }
}

/// What a step feeds the command on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStdin {
    None,
    /// The draft compose text (`-f -`).
    Draft,
}

/// One command: an argv (program first), the directory it runs in, and its stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub stdin: StepStdin,
}

/// One step of a plan. Commands go to the container runtime port; the file steps go to the
/// stack store port, so they too appear in the preview in the order they happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Run(RunStep),
    /// Write the draft: `StackStore::write_atomic` for a save, `StackStore::create` for a create.
    WriteDraft,
    /// `StackStore::delete_dir`.
    RemoveDir,
}

impl Step {
    pub fn as_run(&self) -> Option<&RunStep> {
        match self {
            Self::Run(r) => Some(r),
            _ => None,
        }
    }
}

/// The steps an operation runs, and whether it destroys data the operator cannot get back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub destructive: bool,
}

/// What the plan needs to know about the stack, all of it read by the caller.
#[derive(Debug, Clone, Copy)]
pub struct StackContext<'a> {
    /// The canonical stacks root.
    pub stacks_root: &'a Path,
    pub stack: &'a StackName,
    /// `<stacks>/global.env` exists.
    pub global_env: bool,
    /// `<stack>/.env` exists now.
    pub has_env: bool,
}

impl StackContext<'_> {
    /// `root/name`. Safe to join: a [`StackName`] has no separator, no `..` and no leading `/`.
    pub fn stack_dir(&self) -> PathBuf {
        self.stacks_root.join(self.stack.as_str())
    }
}

/// `--env-file` flags for a command whose cwd is the stack directory.
fn env_files(global_env: bool, has_env: bool) -> Vec<String> {
    if !global_env {
        return Vec::new();
    }
    let mut flags = vec!["--env-file".to_owned(), format!("../{GLOBAL_ENV_FILE}")];
    if has_env {
        flags.extend(["--env-file".to_owned(), format!("./{ENV_FILE}")]);
    }
    flags
}

fn compose(env: &[String], args: &[&str]) -> Vec<String> {
    let mut argv = vec![DOCKER.to_owned(), "compose".to_owned()];
    argv.extend(env.iter().cloned());
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    argv
}

/// Builds the plan for `op` on the stack in `ctx`.
pub fn plan(ctx: &StackContext<'_>, op: &Operation) -> Plan {
    let dir = ctx.stack_dir();
    let env = env_files(ctx.global_env, ctx.has_env);
    let run = |argv: Vec<String>| {
        Step::Run(RunStep {
            argv,
            cwd: dir.clone(),
            stdin: StepStdin::None,
        })
    };
    let up = |env: &[String]| run(compose(env, &["up", "-d", "--remove-orphans"]));
    let validate_in_dir = || {
        Step::Run(RunStep {
            argv: compose(&env, &["-f", "-", "config", "--quiet"]),
            cwd: dir.clone(),
            stdin: StepStdin::Draft,
        })
    };

    match op {
        Operation::Up => Plan::safe(vec![up(&env)]),
        Operation::Stop => Plan::safe(vec![run(compose(&env, &["stop"]))]),
        Operation::Restart => Plan::safe(vec![run(compose(&env, &["restart"]))]),
        Operation::Down {
            remove_volumes,
            remove_images,
        } => {
            let mut args = vec!["down"];
            if *remove_volumes {
                args.push("--volumes");
            }
            if *remove_images {
                args.extend(["--rmi", "all"]);
            }
            Plan {
                steps: vec![run(compose(&env, &args))],
                destructive: *remove_volumes || *remove_images,
            }
        }
        Operation::Pull => Plan::safe(vec![run(compose(&env, &["pull"]))]),
        Operation::Update => Plan::safe(vec![run(compose(&env, &["pull"])), up(&env)]),
        Operation::Save {
            deploy,
            env_after_write,
        } => {
            let mut steps = vec![validate_in_dir(), Step::WriteDraft];
            if *deploy {
                steps.push(up(&env_files(ctx.global_env, *env_after_write)));
            }
            Plan::safe(steps)
        }
        Operation::Create {
            deploy,
            env_after_write,
        } => {
            // The directory does not exist yet, so validation runs in the stacks root with the
            // project named explicitly; the draft's own `.env` is not on disk to pass.
            let mut argv = vec![DOCKER.to_owned(), "compose".to_owned()];
            if ctx.global_env {
                argv.extend(["--env-file".to_owned(), format!("./{GLOBAL_ENV_FILE}")]);
            }
            argv.extend(
                ["-p", ctx.stack.as_str(), "-f", "-", "config", "--quiet"]
                    .iter()
                    .map(|a| (*a).to_owned()),
            );
            let mut steps = vec![
                Step::Run(RunStep {
                    argv,
                    cwd: ctx.stacks_root.to_path_buf(),
                    stdin: StepStdin::Draft,
                }),
                Step::WriteDraft,
            ];
            if *deploy {
                steps.push(up(&env_files(ctx.global_env, *env_after_write)));
            }
            Plan::safe(steps)
        }
        Operation::Delete { remove_directory } => {
            let mut steps = vec![run(compose(&env, &["down", "--remove-orphans"]))];
            if *remove_directory {
                steps.push(Step::RemoveDir);
            }
            Plan {
                steps,
                destructive: *remove_directory,
            }
        }
        Operation::ServiceUp { service } => {
            Plan::safe(vec![run(compose(&env, &["up", "-d", service.as_str()]))])
        }
        Operation::ServiceStop { service } => {
            Plan::safe(vec![run(compose(&env, &["stop", service.as_str()]))])
        }
        Operation::ServiceRestart { service } => {
            Plan::safe(vec![run(compose(&env, &["restart", service.as_str()]))])
        }
    }
}

impl Plan {
    fn safe(steps: Vec<Step>) -> Self {
        Self {
            steps,
            destructive: false,
        }
    }

    /// The commands only, in order.
    pub fn commands(&self) -> impl Iterator<Item = &RunStep> {
        self.steps.iter().filter_map(Step::as_run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/opt/stacks";

    fn name() -> StackName {
        StackName::parse("web").unwrap()
    }

    fn svc(s: &str) -> ServiceName {
        ServiceName::parse(s).unwrap()
    }

    fn planned(global_env: bool, has_env: bool, op: Operation) -> Plan {
        let stack = name();
        plan(
            &StackContext {
                stacks_root: Path::new(ROOT),
                stack: &stack,
                global_env,
                has_env,
            },
            &op,
        )
    }

    /// Each command as one string, for readable assertions (the real thing stays a vector).
    fn argvs(p: &Plan) -> Vec<String> {
        p.commands().map(|r| r.argv.join(" ")).collect()
    }

    fn steps(p: &Plan) -> Vec<String> {
        p.steps
            .iter()
            .map(|s| match s {
                Step::Run(r) => r.argv.join(" "),
                Step::WriteDraft => "<write draft>".into(),
                Step::RemoveDir => "<remove dir>".into(),
            })
            .collect()
    }

    #[test]
    fn the_argv_table_without_global_env() {
        let cases = [
            (Operation::Up, "docker compose up -d --remove-orphans"),
            (Operation::Stop, "docker compose stop"),
            (Operation::Restart, "docker compose restart"),
            (
                Operation::Down {
                    remove_volumes: false,
                    remove_images: false,
                },
                "docker compose down",
            ),
            (Operation::Pull, "docker compose pull"),
            (
                Operation::ServiceUp { service: svc("db") },
                "docker compose up -d db",
            ),
            (
                Operation::ServiceStop { service: svc("db") },
                "docker compose stop db",
            ),
            (
                Operation::ServiceRestart { service: svc("db") },
                "docker compose restart db",
            ),
        ];
        for (op, want) in cases {
            // `.env` alone changes nothing: Compose reads it by itself.
            for has_env in [false, true] {
                let p = planned(false, has_env, op.clone());
                assert_eq!(argvs(&p), vec![want.to_owned()], "{op:?}");
                assert!(!p.destructive, "{op:?}");
            }
        }
    }

    #[test]
    fn every_command_runs_in_the_stack_directory_with_no_stdin() {
        let p = planned(true, true, Operation::Update);
        for r in p.commands() {
            assert_eq!(r.cwd, Path::new("/opt/stacks/web"));
            assert_eq!(r.stdin, StepStdin::None);
        }
    }

    #[test]
    fn global_env_adds_env_files_to_every_command() {
        let with_env = "--env-file ../global.env --env-file ./.env";
        let without = "--env-file ../global.env";
        let cases = [
            (Operation::Up, "up -d --remove-orphans"),
            (Operation::Stop, "stop"),
            (Operation::Restart, "restart"),
            (
                Operation::Down {
                    remove_volumes: false,
                    remove_images: false,
                },
                "down",
            ),
            (Operation::Pull, "pull"),
            (Operation::ServiceUp { service: svc("db") }, "up -d db"),
            (Operation::ServiceStop { service: svc("db") }, "stop db"),
            (
                Operation::ServiceRestart { service: svc("db") },
                "restart db",
            ),
        ];
        for (op, tail) in cases {
            assert_eq!(
                argvs(&planned(true, true, op.clone())),
                vec![format!("docker compose {with_env} {tail}")],
                "{op:?}"
            );
            assert_eq!(
                argvs(&planned(true, false, op.clone())),
                vec![format!("docker compose {without} {tail}")],
                "{op:?}: ./.env only when it exists"
            );
        }
    }

    #[test]
    fn env_file_flags_are_separate_arguments() {
        let p = planned(true, true, Operation::Stop);
        let argv = &p.commands().next().unwrap().argv;
        assert_eq!(
            argv,
            &[
                "docker",
                "compose",
                "--env-file",
                "../global.env",
                "--env-file",
                "./.env",
                "stop"
            ]
        );
    }

    #[test]
    fn down_with_volumes_or_images_is_destructive() {
        let down = |v, i| {
            planned(
                false,
                false,
                Operation::Down {
                    remove_volumes: v,
                    remove_images: i,
                },
            )
        };
        let p = down(true, false);
        assert_eq!(argvs(&p), vec!["docker compose down --volumes"]);
        assert!(p.destructive);
        let p = down(false, true);
        assert_eq!(argvs(&p), vec!["docker compose down --rmi all"]);
        assert!(p.destructive);
        let p = down(true, true);
        assert_eq!(argvs(&p), vec!["docker compose down --volumes --rmi all"]);
        assert!(p.destructive);
        assert!(!down(false, false).destructive);
    }

    #[test]
    fn update_is_pull_then_up() {
        let p = planned(false, false, Operation::Update);
        assert_eq!(
            argvs(&p),
            vec!["docker compose pull", "docker compose up -d --remove-orphans"]
        );
        assert!(!p.destructive);
    }

    #[test]
    fn delete_is_down_and_removes_the_directory_only_when_chosen() {
        let p = planned(
            false,
            false,
            Operation::Delete {
                remove_directory: false,
            },
        );
        assert_eq!(steps(&p), vec!["docker compose down --remove-orphans"]);
        assert!(!p.destructive);

        let p = planned(
            true,
            false,
            Operation::Delete {
                remove_directory: true,
            },
        );
        assert_eq!(
            steps(&p),
            vec![
                "docker compose --env-file ../global.env down --remove-orphans",
                "<remove dir>"
            ]
        );
        assert!(p.destructive);
    }

    #[test]
    fn save_validates_the_draft_then_writes_it() {
        let p = planned(
            false,
            false,
            Operation::Save {
                deploy: false,
                env_after_write: false,
            },
        );
        assert_eq!(
            steps(&p),
            vec!["docker compose -f - config --quiet", "<write draft>"]
        );
        let validate = p.commands().next().unwrap();
        assert_eq!(validate.stdin, StepStdin::Draft);
        assert_eq!(validate.cwd, Path::new("/opt/stacks/web"));
        assert!(!p.destructive);
    }

    #[test]
    fn save_with_deploy_is_validate_write_up() {
        let p = planned(
            false,
            false,
            Operation::Save {
                deploy: true,
                env_after_write: true,
            },
        );
        assert_eq!(
            steps(&p),
            vec![
                "docker compose -f - config --quiet",
                "<write draft>",
                "docker compose up -d --remove-orphans",
            ]
        );
    }

    #[test]
    fn save_with_global_env_validates_against_todays_env_and_deploys_with_the_new_one() {
        // No .env yet; the draft creates one.
        let p = planned(
            true,
            false,
            Operation::Save {
                deploy: true,
                env_after_write: true,
            },
        );
        assert_eq!(
            steps(&p),
            vec![
                "docker compose --env-file ../global.env -f - config --quiet",
                "<write draft>",
                "docker compose --env-file ../global.env --env-file ./.env up -d --remove-orphans",
            ]
        );
    }

    #[test]
    fn create_validates_in_the_root_with_the_project_named() {
        let p = planned(
            true,
            false,
            Operation::Create {
                deploy: true,
                env_after_write: false,
            },
        );
        assert_eq!(
            steps(&p),
            vec![
                "docker compose --env-file ./global.env -p web -f - config --quiet",
                "<write draft>",
                "docker compose --env-file ../global.env up -d --remove-orphans",
            ]
        );
        let validate = p.commands().next().unwrap();
        assert_eq!(validate.cwd, Path::new(ROOT));
        assert_eq!(validate.stdin, StepStdin::Draft);
        assert_eq!(p.commands().nth(1).unwrap().cwd, Path::new("/opt/stacks/web"));

        let p = planned(
            false,
            false,
            Operation::Create {
                deploy: false,
                env_after_write: false,
            },
        );
        assert_eq!(
            steps(&p),
            vec!["docker compose -p web -f - config --quiet", "<write draft>"]
        );
    }

    #[test]
    fn operations_need_their_table_row() {
        let down = |v, i| Operation::Down {
            remove_volumes: v,
            remove_images: i,
        };
        assert_eq!(Operation::Up.required_action(), Action::Lifecycle);
        assert_eq!(down(false, false).required_action(), Action::Lifecycle);
        assert_eq!(down(true, false).required_action(), Action::DownDestructive);
        assert_eq!(down(false, true).required_action(), Action::DownDestructive);
        assert_eq!(
            Operation::Save {
                deploy: true,
                env_after_write: false
            }
            .required_action(),
            Action::Edit
        );
        assert_eq!(
            Operation::Create {
                deploy: false,
                env_after_write: false
            }
            .required_action(),
            Action::CreateStack
        );
        assert_eq!(
            Operation::Delete {
                remove_directory: false
            }
            .required_action(),
            Action::DeleteStack
        );
        assert_eq!(
            Operation::ServiceRestart { service: svc("db") }.required_action(),
            Action::Lifecycle
        );
    }

    #[test]
    fn operations_deserialise_with_validated_service_names() {
        let op: Operation =
            serde_json::from_str(r#"{"kind":"service_stop","service":"db"}"#).unwrap();
        assert_eq!(op, Operation::ServiceStop { service: svc("db") });
        assert_eq!(op.kind(), OperationKind::ServiceStop);
        assert!(
            serde_json::from_str::<Operation>(r#"{"kind":"service_stop","service":"-v"}"#)
                .is_err()
        );
    }
}
