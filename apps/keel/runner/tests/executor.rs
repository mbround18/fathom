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
use keel_trust::CommitId;
use keel_trust::ledger::{Entry, Outcome};
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
  keel-server:
    playbook: playbooks/web.yaml
    profiles: [debian]
    inventory: prod
    params:
      keel_server_image_digest: { kind: oci_digest }
"#;

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
if [ -f "$HOME/params.json" ]; then cp "$HOME/params.json" "$out/params.json"; fi
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
    let (mut commits, pack) = repo_signed(root, None);
    (commits.pop().unwrap(), pack)
}

/// As [`repo`]; with `key`, two commits (oldest first) SSH-signed by the private key at `key`.
fn repo_signed(root: &Path, key: Option<&Path>) -> (Vec<String>, Vec<u8>) {
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
            .args(match key {
                Some(k) => vec![
                    "-c".to_owned(),
                    "gpg.format=ssh".to_owned(),
                    "-c".to_owned(),
                    format!("user.signingkey={}", k.display()),
                ],
                None => vec![],
            })
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
    let head = || {
        String::from_utf8(git(&["rev-parse", "HEAD"], None))
            .unwrap()
            .trim()
            .to_owned()
    };
    let mut commits = vec![];
    if key.is_some() {
        git(&["commit", "-q", "-S", "-m", "init"], None);
        commits.push(head());
        git(&["commit", "-q", "-S", "--allow-empty", "-m", "next"], None);
    } else {
        git(&["commit", "-q", "-m", "init"], None);
    }
    commits.push(head());
    let pack = git(
        &["pack-objects", "--stdout", "--revs", "-q"],
        Some(b"HEAD\n"),
    );
    (commits, pack)
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
        workspace: "ws-1".to_owned(),
        binding: "main".to_owned(),
        job: job.to_owned(),
        base: base.map(str::to_owned),
        target: target.to_owned(),
        deadline: now() + 600,
        pack_size: pack.len() as u64,
        quorum: None,
        params: Default::default(),
    }
}

/// As [`assign`], with job params (spec 061).
fn assign_params(
    run_id: &str,
    job: &str,
    target: &str,
    pack: &[u8],
    params: &[(&str, &str)],
) -> Frame {
    let mut frame = assign(run_id, job, None, target, pack);
    if let Frame::Assign { params: p, .. } = &mut frame {
        *p = params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
    }
    frame
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
async fn require_signed_refuses_an_unsigned_first_run() {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    std::fs::write(host.cfg.trust_dir.join("signers"), "").unwrap();
    std::fs::write(host.cfg.trust_dir.join("approvers.json"), "[]").unwrap();
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-9", "web-deploy", None, &commit, &pack)],
    )
    .await;
    send(&mut s, pack_builder::frames("run-9", &pack)).await;
    send(
        &mut s,
        [
            secrets("run-9", false),
            Frame::Approvals {
                run_id: "run-9".to_owned(),
                items: vec![],
            },
        ],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
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
    assert!(host.record().is_empty());
    assert!(!host.cfg.state_dir.join("ledger.jsonl").exists());
}

/// An SSH key for signing commits, and its `allowed_signers` line.
fn signing_key(root: &Path) -> (PathBuf, String) {
    let key = root.join("signing_key");
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "keel-test", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    (key, format!("keel {}", public.trim()))
}

#[tokio::test]
async fn a_signed_run_is_recorded_start_and_end() {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    let (key, signer) = signing_key(&host.root);
    std::fs::write(host.cfg.trust_dir.join("signers"), format!("{signer}\n")).unwrap();
    std::fs::write(host.cfg.trust_dir.join("approvers.json"), "[]").unwrap();
    let (commits, pack) = repo_signed(&host.root, Some(&key));
    let (base, commit) = (commits[0].clone(), commits[1].clone());
    // The host already ran `base` (a first run needs a baseline approval).
    let mut seed = keel_trust::Ledger::empty();
    let mut text = seed
        .append(Entry::Start {
            at: "2026-01-01T00:00:00Z".to_owned(),
            run: "seed-run".to_owned(),
            job: "web-deploy".to_owned(),
            base: None,
            target: CommitId::parse(&base).unwrap(),
            auth: keel_trust::Auth::Signed {
                signers: vec!["SHA256:seed".to_owned()],
            },
        })
        .unwrap();
    text.push_str(
        &seed
            .append(Entry::End {
                at: "2026-01-01T00:01:00Z".to_owned(),
                run: "seed-run".to_owned(),
                job: "web-deploy".to_owned(),
                outcome: Outcome::Succeeded,
                exit: Some(0),
            })
            .unwrap(),
    );
    std::fs::write(host.cfg.state_dir.join("ledger.jsonl"), &text).unwrap();

    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-14", "web-deploy", Some(&base), &commit, &pack)],
    )
    .await;
    send(&mut s, pack_builder::frames("run-14", &pack)).await;
    send(
        &mut s,
        [
            secrets("run-14", false),
            Frame::Approvals {
                run_id: "run-14".to_owned(),
                items: vec![],
            },
        ],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    let Some(Frame::Finished {
        exit_code: 0,
        ledger_line,
        ..
    }) = frames.last()
    else {
        panic!("{frames:?}");
    };
    assert!(!host.record().is_empty(), "the playbook did not run");
    let ledger = std::fs::read_to_string(host.cfg.state_dir.join("ledger.jsonl")).unwrap();
    let lines: Vec<&str> = ledger.lines().collect();
    assert_eq!(lines.len(), 4, "{ledger}");
    assert!(ledger.starts_with(&text), "the ledger was rewritten");
    assert!(
        lines[2].contains("\"start\"") && lines[2].contains(&commit),
        "{ledger}"
    );
    assert_eq!(lines[3], ledger_line);
    assert!(ledger_line.contains("\"end\""), "{ledger_line}");

    // The same run id again is a replay, and the ledger does not change.
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign(
            "run-14",
            "web-deploy",
            Some(&commit),
            &commit,
            &pack,
        )],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::Replay,
                ..
            }]
        ),
        "{frames:?}"
    );
    assert_eq!(
        std::fs::read_to_string(host.cfg.state_dir.join("ledger.jsonl")).unwrap(),
        ledger
    );
}

#[tokio::test]
async fn signed_mode_without_trust_files_fails_closed() {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(
        &mut s,
        [assign("run-13", "web-deploy", None, &commit, &pack)],
    )
    .await;
    send(&mut s, pack_builder::frames("run-13", &pack)).await;
    send(
        &mut s,
        [
            secrets("run-13", false),
            Frame::Approvals {
                run_id: "run-13".to_owned(),
                items: vec![],
            },
        ],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code, .. }) if *exit_code == SETUP_FAILED),
        "{frames:?}"
    );
    assert!(host.record().is_empty());
}

#[test]
fn ledger_verify_reports_a_broken_chain_and_leaves_it_alone() {
    let host = Host::new(POLICY);
    let verify = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_keel-executor"))
            .arg("--state-dir")
            .arg(&host.cfg.state_dir)
            .args(["ledger", "verify"])
            .output()
            .unwrap()
    };
    let out = verify();
    assert!(out.status.success(), "{out:?}");
    let ledger = host.cfg.state_dir.join("ledger.jsonl");
    std::fs::write(&ledger, "garbage\n").unwrap();
    let out = verify();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(std::fs::read_to_string(&ledger).unwrap(), "garbage\n");
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

// ---- spec 061 T063: job params ----------------------------------------------------------------

/// The value of each `-e`, in order.
fn extra_vars(argv: &[&str]) -> Vec<String> {
    argv.windows(2)
        .filter(|w| w[0] == "-e")
        .map(|w| w[1].to_owned())
        .collect()
}

/// The file name of each `-e @file`, in order.
fn extra_var_files(record: &str) -> Vec<String> {
    extra_vars(&args(record))
        .iter()
        .map(|e| e.rsplit('/').next().unwrap().to_owned())
        .collect()
}

async fn params_run(host: &Host, run_id: &str, job: &str, params: &[(&str, &str)]) -> Vec<Frame> {
    let (commit, pack) = repo(&host.root);
    let (mut s, task) = start(&host.cfg);
    send(&mut s, [assign_params(run_id, job, &commit, &pack, params)]).await;
    send(&mut s, pack_builder::frames(run_id, &pack)).await;
    send(&mut s, [secrets(run_id, false)]).await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    frames
}

#[tokio::test]
async fn params_are_a_private_file_after_secret_vars_and_before_forced_vars() {
    let host = Host::new(POLICY);
    let frames = params_run(
        &host,
        "run-20",
        "keel-server",
        &[("keel_server_image_digest", DIGEST)],
    )
    .await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let record = host.record();
    assert!(
        extra_vars(&args(&record))
            .iter()
            .all(|e| e.starts_with('@')),
        "{record}"
    );
    assert_eq!(
        extra_var_files(&record),
        [
            "jobvars.json",
            "secret_vars.yaml",
            "params.json",
            "forced.json"
        ]
    );
    assert!(
        record.lines().any(|l| l == "file:600 params.json"),
        "{record}"
    );
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(host.root.join("record/params.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "keel_server_image_digest": DIGEST })
    );
    assert_eq!(host.runs_dir_entries(), 0);
}

#[tokio::test]
async fn a_declared_param_may_be_left_unset() {
    let host = Host::new(POLICY);
    let frames = params_run(&host, "run-21", "keel-server", &[]).await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let record = host.record();
    assert!(!record.contains("params.json"), "{record}");
    assert_eq!(
        extra_var_files(&record),
        ["jobvars.json", "secret_vars.yaml", "forced.json"]
    );
}

#[tokio::test]
async fn a_run_without_params_runs_as_before() {
    let host = Host::new(POLICY);
    let frames = full_run(&host, "run-22", "web-deploy").await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let record = host.record();
    assert!(!record.contains("params.json"), "{record}");
    assert_eq!(
        extra_var_files(&record),
        ["jobvars.json", "secret_vars.yaml", "forced.json"]
    );
}

#[tokio::test]
async fn params_that_do_not_match_the_job_are_refused() {
    let upper = format!("sha256:{}", "ABCDEF0123456789".repeat(4));
    let cases: [(&str, &str, &str); 5] = [
        // A name the job doesn't declare.
        ("keel-server", "keel_server_image", DIGEST),
        // Values that aren't of the declared kind.
        ("keel-server", "keel_server_image_digest", "latest"),
        ("keel-server", "keel_server_image_digest", &upper),
        ("keel-server", "keel_server_image_digest", LOOKUP),
        // A job that declares no params at all.
        ("web-deploy", "keel_server_image_digest", DIGEST),
    ];
    for (i, (job, name, value)) in cases.into_iter().enumerate() {
        let host = Host::new(POLICY);
        let run_id = format!("run-3{i}");
        let frames = params_run(&host, &run_id, job, &[(name, value)]).await;
        let [
            Frame::Refused {
                run_id: got,
                reason: Refusal::ParamsMismatch,
                detail,
            },
        ] = frames.as_slice()
        else {
            panic!("{job} {name}={value}: {frames:?}");
        };
        assert_eq!(*got, run_id);
        assert!(!detail.contains(value), "the value was echoed: {detail}");
        assert!(
            host.record().is_empty(),
            "the playbook ran: {job} {name}={value}"
        );
        assert_eq!(host.runs_dir_entries(), 0);
    }
}

// ---- spec 061 T063/T064: params in signed mode ------------------------------------------------

const WS: &str = "0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4";
const BINDING: &str = "1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f5";

fn b64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// A deterministic software passkey (EdDSA) for `approvers.json` and baseline approvals.
struct Passkey(russh::keys::ssh_key::private::Ed25519Keypair);

impl Passkey {
    fn new() -> Self {
        Self(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
            &[9; 32],
        ))
    }

    fn approvers_json(&self) -> String {
        // COSE_Key {1: 1 (OKP), 3: -8 (EdDSA), -1: 6 (Ed25519), -2: x}.
        let mut cose = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
        cose.extend_from_slice(&self.0.public.0);
        serde_json::json!([{
            "label": "alice",
            "credential_id": b64url(b"cred-1"),
            "public_key_cose": b64url(&cose),
            "alg": "EdDSA",
        }])
        .to_string()
    }

    /// A `baseline` approval of `run` at `target` for `web1`, signed over `params`.
    fn baseline(&self, run: &str, target: &str, params: &[(&str, &str)]) -> Frame {
        use russh::keys::signature::Signer as _;
        use sha2::{Digest as _, Sha256};
        let expires = now() + 3_600;
        let expires_at =
            keel_trust::challenge::format_utc(i64::try_from(expires).unwrap()).unwrap();
        let scope = keel_trust::OverrideScope {
            kind: keel_trust::ApprovalKind::Baseline,
            workspace: WS.to_owned(),
            binding: BINDING.to_owned(),
            run: run.to_owned(),
            job: "keel-server".to_owned(),
            base: None,
            target: CommitId::parse(target).unwrap(),
            hosts: vec!["web1".to_owned()],
            expires_at,
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        };
        let client_data = serde_json::json!({
            "type": "webauthn.get",
            "challenge": b64url(&scope.challenge().unwrap()),
            "origin": "https://keel.example",
        })
        .to_string();
        let mut auth_data = Sha256::digest(b"keel.example").to_vec();
        auth_data.extend_from_slice(&[0x05, 0, 0, 0, 1]); // UP | UV, counter 1
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(client_data.as_bytes()));
        let signature: russh::keys::ssh_key::Signature = self.0.try_sign(&signed).unwrap();
        Frame::Approvals {
            run_id: run.to_owned(),
            items: vec![keel_exec::frames::Approval {
                kind: "baseline".to_owned(),
                approver: "alice".to_owned(),
                credential_id: b64url(b"cred-1"),
                authenticator_data: b64url(&auth_data),
                client_data_json: b64url(client_data.as_bytes()),
                signature: b64url(signature.as_bytes()),
                expires_at: expires,
                comment: "update Keel".to_owned(),
                hosts: vec!["web1".to_owned()],
            }],
        }
    }
}

fn signed_host() -> Host {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    std::fs::write(host.cfg.trust_dir.join("signers"), "").unwrap();
    std::fs::write(
        host.cfg.trust_dir.join("approvers.json"),
        Passkey::new().approvers_json(),
    )
    .unwrap();
    host
}

/// A first (baseline) signed run of `keel-server` assigned a digest, approved over
/// `approved_with`.
async fn signed_params_run(host: &Host, run: &str, approved_with: &[(&str, &str)]) -> Vec<Frame> {
    let (commit, pack) = repo(&host.root);
    let mut assign = assign_params(
        run,
        "keel-server",
        &commit,
        &pack,
        &[("keel_server_image_digest", DIGEST)],
    );
    if let Frame::Assign {
        workspace, binding, ..
    } = &mut assign
    {
        *workspace = WS.to_owned();
        *binding = BINDING.to_owned();
    }
    let (mut s, task) = start(&host.cfg);
    send(&mut s, [assign]).await;
    send(&mut s, pack_builder::frames(run, &pack)).await;
    send(
        &mut s,
        [
            secrets(run, false),
            Passkey::new().baseline(run, &commit, approved_with),
        ],
    )
    .await;
    let frames = drain(&mut s).await;
    task.await.unwrap();
    frames
}

#[tokio::test]
async fn a_signed_run_with_params_needs_an_approval_bound_to_them() {
    let run = "3e4f5061-7b8c-4d9e-afb0-c1d2e3f40516";

    // Approved without the params the run was assigned: the challenges differ.
    let host = signed_host();
    let frames = signed_params_run(&host, run, &[]).await;
    assert!(
        matches!(
            frames.as_slice(),
            [Frame::Refused {
                reason: Refusal::BadOverride,
                ..
            }]
        ),
        "{frames:?}"
    );
    assert!(host.record().is_empty());

    // Approved with them: the host rebuilds the same challenge, and the run goes ahead.
    let host = signed_host();
    let frames = signed_params_run(&host, run, &[("keel_server_image_digest", DIGEST)]).await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let ledger = std::fs::read_to_string(host.cfg.state_dir.join("ledger.jsonl")).unwrap();
    assert!(
        ledger.contains("\"start\"") && ledger.contains(run),
        "{ledger}"
    );
    assert!(host.record().contains("params.json"));
}
