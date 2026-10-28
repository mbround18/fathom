//! The executor's isolation guarantees (spec 055 T077, contracts/runner.md), without systemd:
//! `serve` runs over one end of a socket pair and a fake `ansible-playbook` records what it was
//! given.
//!
//! - `--limit <inventory_name>` and `ansible_connection=local` are forced, whatever the job says.
//! - A job whose profiles are outside `allowed_profiles` is refused `profile_not_allowed`.
//! - Secrets are `0600` in the run directory while the playbook runs, and gone after `Finished`.
//! - `secret_vars` reach Ansible tagged `!unsafe`, so a template in a value stays literal text.
//! - `ReadLedger` is answered, a base mismatch and a short pack are refused, and `Cancel`
//!   interrupts a running playbook.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, Refusal, SecretItem, SecretString};
use keel_exec::pack_builder;
use keel_runner::executor::{CANCELLED, ExecutorConfig, SETUP_FAILED, serve};
use tokio::net::UnixStream;

const POLICY: &str = r#"
origin = "https://keel.example"
rp_id = "keel.example"
allowed_profiles = ["debian"]
inventory_name = "web1"
"#;

const MANIFEST: &str = r#"
version: 1
profiles:
  debian: { description: "Every Debian host" }
  gameserver: { description: "Game servers" }
inventories:
  prod:
    path: inventories/prod/hosts.yaml
jobs:
  web-deploy:
    playbook: playbooks/web.yaml
    profiles: [debian]
    inventory: prod
    limit: "all"
    vars: { release_channel: stable, ansible_connection: ssh }
    credentials: [app-secrets, prod-vault]
  game:
    playbook: playbooks/web.yaml
    profiles: [gameserver]
    inventory: prod
"#;

const LOOKUP: &str = "{{ lookup('pipe','id') }}";

/// The fake `ansible-playbook`: records argv, environment, the run directory's secret files
/// (mode and name) and a copy of `secret_vars.yaml`, writes one stats event to fd 3, and then
/// either exits or (with `SLEEP` in the job's env credential) waits to be interrupted.
fn fake_playbook(record: &Path) -> String {
    format!(
        r#"#!/bin/sh
out='{out}'
{{
  for a in "$@"; do printf 'arg:%s\n' "$a"; done
  env | sort | sed 's/^/env:/'
  for f in "$HOME"/*; do
    [ -f "$f" ] && printf 'file:%s %s\n' "$(stat -c %a "$f")" "$(basename "$f")"
  done
  printf 'pwd:%s\n' "$PWD"
  printf 'home:%s\n' "$HOME"
}} > "$out/record"
cp "$HOME/secret_vars.yaml" "$out/secret_vars.yaml"
printf '%s\n' '{{"v":1,"event":"stats","stats":{{"web1":{{"ok":2,"changed":1,"failures":0,"unreachable":0,"skipped":0,"rescued":0,"ignored":0}}}}}}' >&3
echo "playbook ran"
if [ -n "$SLEEP" ]; then
  trap 'echo interrupted; exit 99' INT
  echo ready > "$out/ready"
  while :; do sleep 0.1; done
fi
exit 0
"#,
        out = record.display()
    )
}

struct Host {
    _dir: tempfile::TempDir,
    root: PathBuf,
    cfg: ExecutorConfig,
}

impl Host {
    fn new(policy: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let trust = root.join("trust");
        let state = root.join("state");
        let venv = root.join("venv");
        let record = root.join("record");
        for d in [&trust, &state, &record] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(trust.join("policy.toml"), policy).unwrap();
        std::fs::create_dir_all(
            venv.join("lib/python3.12/site-packages/ansible_core-2.21.5.dist-info"),
        )
        .unwrap();
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        let playbook = venv.join("bin/ansible-playbook");
        std::fs::write(&playbook, fake_playbook(&record)).unwrap();
        std::fs::set_permissions(&playbook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = ExecutorConfig::with_defaults(state, trust, venv);
        cfg.cancel_grace = Duration::from_secs(5);
        Self {
            _dir: dir,
            root,
            cfg,
        }
    }

    fn record(&self) -> String {
        std::fs::read_to_string(self.root.join("record/record")).unwrap_or_default()
    }

    fn runs_dir_entries(&self) -> usize {
        std::fs::read_dir(self.cfg.state_dir.join("runs"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

/// A Git repository with the manifest, committed; returns (commit id, pack of HEAD).
fn repo(root: &Path) -> (String, Vec<u8>) {
    let repo = root.join("repo");
    for d in ["playbooks", "inventories/prod", ".keel"] {
        std::fs::create_dir_all(repo.join(d)).unwrap();
    }
    std::fs::write(repo.join(".keel/jobs.yaml"), MANIFEST).unwrap();
    std::fs::write(
        repo.join("playbooks/web.yaml"),
        "- hosts: all\n  tasks: []\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("inventories/prod/hosts.yaml"),
        "all:\n  hosts:\n    web1: {}\n    web2: {}\n",
    )
    .unwrap();
    let git = |args: &[&str], stdin: Option<&[u8]>| -> Vec<u8> {
        let mut cmd = Command::new("git");
        cmd.current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=Keel Test",
                "-c",
                "user.email=keel@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        {
            use std::io::Write as _;
            let mut input = child.stdin.take().unwrap();
            if let Some(bytes) = stdin {
                input.write_all(bytes).unwrap();
            }
        }
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "git {args:?} failed");
        out.stdout
    };
    git(&["init", "-q"], None);
    git(&["add", "-A"], None);
    git(&["commit", "-q", "-m", "init"], None);
    let commit = String::from_utf8(git(&["rev-parse", "HEAD"], None))
        .unwrap()
        .trim()
        .to_owned();
    let pack = git(
        &["pack-objects", "--stdout", "--revs", "-q"],
        Some(b"HEAD\n"),
    );
    (commit, pack)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn secrets(run_id: &str, sleep: bool) -> Frame {
    let mut env = "API_TOKEN=s3cret-env\n".to_owned();
    if sleep {
        env.push_str("SLEEP=1\n");
    }
    Frame::Secrets {
        run_id: run_id.to_owned(),
        items: vec![
            SecretItem {
                name: "app-secrets".to_owned(),
                kind: "secret_vars".to_owned(),
                value: SecretString::new(
                    serde_json::json!({ "db_password": LOOKUP, "port": 5432 }).to_string(),
                ),
            },
            SecretItem {
                name: "prod-vault".to_owned(),
                kind: "vault_password".to_owned(),
                value: SecretString::new("vault-pass"),
            },
            SecretItem {
                name: "env".to_owned(),
                kind: "env".to_owned(),
                value: SecretString::new(env),
            },
        ],
    }
}

fn assign(run_id: &str, job: &str, base: Option<&str>, target: &str, pack: &[u8]) -> Frame {
    Frame::Assign {
        run_id: run_id.to_owned(),
        job: job.to_owned(),
        base: base.map(str::to_owned),
        target: target.to_owned(),
        deadline: now() + 600,
        pack_size: pack.len() as u64,
    }
}

/// Starts `serve` on one end of a socket pair and returns the other end.
fn start(cfg: &ExecutorConfig) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let cfg = cfg.clone();
    let task = tokio::spawn(async move { serve(theirs, &cfg).await });
    (ours, task)
}

async fn send(s: &mut UnixStream, frames: impl IntoIterator<Item = Frame>) {
    for f in frames {
        write_frame(s, &f).await.unwrap();
    }
}

/// Reads frames until the connection closes.
async fn drain(s: &mut UnixStream) -> Vec<Frame> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), read_frame(s)).await {
            Ok(Ok(Some(f))) => out.push(f),
            Ok(Ok(None)) | Ok(Err(_)) => return out,
            Err(_) => panic!("the executor did not finish; got {out:?}"),
        }
    }
}

async fn full_run(host: &Host, run_id: &str, job: &str) -> Vec<Frame> {
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(&mut s, [assign(run_id, job, None, &commit, &pack)]).await;
    send(&mut s, pack_builder::frames(run_id, &pack)).await;
    send(&mut s, [secrets(run_id, false)]).await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    frames
}

fn args(record: &str) -> Vec<&str> {
    record
        .lines()
        .filter_map(|l| l.strip_prefix("arg:"))
        .collect()
}

#[tokio::test]
async fn a_run_forces_this_host_and_a_local_connection() {
    let host = Host::new(POLICY);
    let frames = full_run(&host, "run-1", "web-deploy").await;

    assert!(
        matches!(&frames[0], Frame::Started { run_id } if run_id == "run-1"),
        "{frames:?}"
    );
    let Some(Frame::Finished {
        exit_code, stats, ..
    }) = frames.last()
    else {
        panic!("no Finished: {frames:?}");
    };
    assert_eq!(*exit_code, 0);
    assert_eq!(stats["web1"].ok, 2);
    assert!(
        frames
            .iter()
            .any(|f| matches!(f, Frame::Event { json, .. } if json.contains("\"stats\"")))
    );
    assert!(frames.iter().any(
        |f| matches!(f, Frame::Log { bytes, .. } if String::from_utf8_lossy(bytes).contains("playbook ran"))
    ));

    let record = host.record();
    let argv = args(&record);
    // The job says `limit: all` and `ansible_connection: ssh`; neither wins.
    let limit = argv.iter().position(|a| *a == "--limit").unwrap();
    assert_eq!(argv[limit + 1], "web1");
    assert_eq!(argv.iter().filter(|a| **a == "--limit").count(), 1);
    assert!(!argv.contains(&"all"));
    let extra: Vec<&str> = argv
        .windows(2)
        .filter(|w| w[0] == "-e")
        .map(|w| w[1])
        .collect();
    let forced = extra.last().unwrap().strip_prefix('@').unwrap();
    assert!(
        forced.ends_with("/forced.json"),
        "forced vars must be the last -e: {extra:?}"
    );
    assert_eq!(argv.last(), Some(&"playbooks/web.yaml"));
    let inventory = argv[argv.iter().position(|a| *a == "-i").unwrap() + 1];
    assert!(inventory.ends_with("/tree/inventories/prod/hosts.yaml"));
    assert!(
        argv.iter()
            .any(|a| a.starts_with("prod-vault@") && a.ends_with("/vault-1"))
    );

    // The environment is built, not inherited.
    assert!(record.contains("env:KEEL_EVENT_FD=3\n"));
    assert!(record.contains("env:ANSIBLE_CALLBACKS_ENABLED=keel_events\n"));
    assert!(record.contains("env:API_TOKEN=s3cret-env\n"));
    assert!(
        !record.contains("env:CARGO"),
        "the test's environment leaked in"
    );
    assert!(
        record
            .lines()
            .any(|l| l.starts_with("pwd:") && l.ends_with("/tree"))
    );
}

#[tokio::test]
async fn secrets_are_private_while_running_and_gone_after_finished() {
    let host = Host::new(POLICY);
    let frames = full_run(&host, "run-2", "web-deploy").await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );

    let record = host.record();
    let files: Vec<&str> = record
        .lines()
        .filter_map(|l| l.strip_prefix("file:"))
        .collect();
    for name in ["secret_vars.yaml", "vault-1", "jobvars.json", "forced.json"] {
        assert!(
            files.contains(&format!("600 {name}").as_str()),
            "{name}: {files:?}"
        );
    }
    assert!(files.iter().all(|f| f.starts_with("600 ")), "{files:?}");

    // `Finished` came after the run directory was removed.
    let home = record
        .lines()
        .find_map(|l| l.strip_prefix("home:"))
        .unwrap();
    assert!(!Path::new(home).exists());
    assert_eq!(host.runs_dir_entries(), 0);

    // No secret value travels back over the socket.
    let wire = format!("{frames:?}");
    for value in ["s3cret-env", "vault-pass", "lookup('pipe'"] {
        assert!(!wire.contains(value), "{value} was sent back");
    }
}

#[tokio::test]
async fn secret_vars_are_tagged_unsafe_so_templates_stay_literal() {
    let host = Host::new(POLICY);
    full_run(&host, "run-3", "web-deploy").await;
    let yaml = std::fs::read_to_string(host.root.join("record/secret_vars.yaml")).unwrap();
    assert!(
        yaml.contains(&format!("!unsafe \"db_password\": !unsafe \"{LOOKUP}\"")),
        "{yaml}"
    );
    assert!(yaml.contains("!unsafe \"port\": 5432"), "{yaml}");
    // No string scalar escapes the tag.
    let tagged = yaml.matches("!unsafe \"").count();
    assert_eq!(yaml.matches('"').count(), tagged * 2, "{yaml}");
}

#[tokio::test]
async fn a_job_outside_allowed_profiles_is_refused() {
    let host = Host::new(POLICY);
    let frames = full_run(&host, "run-4", "game").await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused { run_id, reason: Refusal::ProfileNotAllowed, .. }] if run_id == "run-4"
        ),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
    assert_eq!(host.runs_dir_entries(), 0);
}

#[tokio::test]
async fn an_empty_allowed_profiles_refuses_everything() {
    let host = Host::new(&POLICY.replace(r#"allowed_profiles = ["debian"]"#, ""));
    let frames = full_run(&host, "run-5", "web-deploy").await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::ProfileNotAllowed,
                ..
            }]
        ),
        "{frames:?}"
    );
}

#[tokio::test]
async fn read_ledger_is_answered_and_a_base_mismatch_refused() {
    let host = Host::new(POLICY);
    let (mut s, task) = start(&host.cfg);
    send(&mut s, [Frame::ReadLedger]).await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(frames.as_slice(), [Frame::Ledger { ledger_head: None, bases }] if bases.is_empty()),
        "{frames:?}"
    );

    // Keel believes a base this host never applied.
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-6", "web-deploy", Some(&commit), &commit, &pack)],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::BaseMismatch,
                ..
            }]
        ),
        "{frames:?}"
    );
}

#[tokio::test]
async fn a_corrupt_ledger_refuses_runs() {
    let host = Host::new(POLICY);
    std::fs::write(host.cfg.state_dir.join("ledger.jsonl"), "not json\n").unwrap();
    let frames = full_run(&host, "run-7", "web-deploy").await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::LedgerCorrupt,
                ..
            }]
        ),
        "{frames:?}"
    );
}

#[tokio::test]
async fn an_old_ansible_core_is_refused() {
    let host = Host::new(POLICY);
    let site = host.cfg.venv.join("lib/python3.12/site-packages");
    std::fs::rename(
        site.join("ansible_core-2.21.5.dist-info"),
        site.join("ansible_core-2.17.0.dist-info"),
    )
    .unwrap();
    let frames = full_run(&host, "run-8", "web-deploy").await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::VersionTooOld,
                ..
            }]
        ),
        "{frames:?}"
    );
}

#[tokio::test]
async fn require_signed_refuses_until_signed_mode_exists() {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    let frames = full_run(&host, "run-9", "web-deploy").await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::Unsigned,
                ..
            }]
        ),
        "{frames:?}"
    );
}

#[tokio::test]
async fn a_tampered_pack_fails_before_anything_runs() {
    let host = Host::new(POLICY);
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-10", "web-deploy", None, &commit, &pack)],
    )
    .await;
    let mut frames = pack_builder::frames("run-10", &pack);
    if let Some(Frame::Pack { chunk, .. }) = frames.first_mut() {
        chunk[20] ^= 0xff;
    }
    send(&mut s, frames).await;
    send(&mut s, [secrets("run-10", false)]).await;
    let got = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(got.as_slice(), [Frame::Log { .. }, Frame::Finished { exit_code, .. }] if *exit_code == SETUP_FAILED),
        "{got:?}"
    );
    assert!(host.record().is_empty());
    assert_eq!(host.runs_dir_entries(), 0);
}

#[tokio::test]
async fn cancel_interrupts_the_playbook_and_cleans_up() {
    let host = Host::new(POLICY);
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-11", "web-deploy", None, &commit, &pack)],
    )
    .await;
    send(&mut s, pack_builder::frames("run-11", &pack)).await;
    send(&mut s, [secrets("run-11", true)]).await;
    let ready = host.root.join("record/ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the playbook never started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    send(
        &mut s,
        [Frame::Cancel {
            run_id: "run-11".to_owned(),
        }],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 99, .. })),
        "{frames:?}"
    );
    assert_eq!(host.runs_dir_entries(), 0);
}

#[tokio::test]
async fn cancel_before_start_finishes_without_running() {
    let host = Host::new(POLICY);
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-12", "web-deploy", None, &commit, &pack)],
    )
    .await;
    send(
        &mut s,
        [Frame::Cancel {
            run_id: "run-12".to_owned(),
        }],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code, .. }) if *exit_code == CANCELLED),
        "{frames:?}"
    );
    assert!(!frames.iter().any(|f| matches!(f, Frame::Started { .. })));
}

/// The binary as systemd starts it: the connection on fd 0, and fds 1 and 2 inherited from it
/// (`StandardOutput=inherit`). Its log lines must not reach the wire.
#[tokio::test]
async fn the_binary_serves_fd_0_and_keeps_logs_off_the_wire() {
    let host = Host::new(POLICY);
    std::fs::write(host.cfg.state_dir.join("ledger.jsonl"), "garbage\n").unwrap();
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_keel-executor"))
        .arg("--state-dir")
        .arg(&host.cfg.state_dir)
        .arg("--trust-dir")
        .arg(&host.cfg.trust_dir)
        .arg("--venv")
        .arg(&host.cfg.venv)
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(
            theirs.try_clone().unwrap(),
        )))
        .stdout(std::process::Stdio::from(std::os::fd::OwnedFd::from(
            theirs.try_clone().unwrap(),
        )))
        .stderr(std::process::Stdio::from(std::os::fd::OwnedFd::from(
            theirs,
        )))
        .spawn()
        .unwrap();
    ours.set_nonblocking(true).unwrap();
    let mut s = UnixStream::from_std(ours).unwrap();
    // A corrupt ledger makes the executor log a line on stderr before answering.
    send(&mut s, [Frame::ReadLedger]).await;
    let frames = drain(&mut s).await;
    assert!(child.wait().await.unwrap().success());
    assert!(
        matches!(frames.as_slice(), [Frame::Ledger { ledger_head: None, bases }] if bases.is_empty()),
        "{frames:?}"
    );
}
