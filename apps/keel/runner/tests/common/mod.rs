//! A small executor harness for the spec 068 tests: `serve` over a socket pair, a fake
//! `ansible-playbook` that records what it was given, and a Git repository built from a
//! manifest and a few files. `executor.rs` keeps its own copy for the older tests.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, SecretItem};
use keel_exec::pack_builder;
use keel_runner::executor::{ExecutorConfig, serve};
use keel_trust::CommitId;
use keel_trust::ledger::{Entry, Outcome};
use tokio::net::UnixStream;

/// A host whose ceiling is `debian` only.
pub const POLICY: &str = r#"
origin = "https://keel.example"
rp_id = "keel.example"
allowed_profiles = ["debian"]
inventory_name = "web1"
"#;

/// The fake `ansible-playbook`. It records argv, environment, its pid, session and process
/// group, and where stdin points, then writes one stats event to fd 3. With `prompt.flag` in
/// the checkout it reads a line from `/dev/tty`, as a prompting play would, and exits 2 when
/// there is no terminal to read from.
fn fake_playbook(record: &Path) -> String {
    format!(
        r#"#!/bin/sh
out='{out}'
{{
  for a in "$@"; do printf 'arg:%s\n' "$a"; done
  env | sort | sed 's/^/env:/'
  printf 'pid:%s\n' "$$"
  printf 'pgrp:%s\n' "$(cut -d' ' -f5 /proc/$$/stat)"
  printf 'sid:%s\n' "$(cut -d' ' -f6 /proc/$$/stat)"
  printf 'stdin:%s\n' "$(readlink /proc/$$/fd/0)"
}} > "$out/record"
printf '%s\n' '{{"v":1,"event":"stats","stats":{{"web1":{{"ok":1,"changed":0,"failures":0,"unreachable":0,"skipped":0,"rescued":0,"ignored":0}}}}}}' >&3
echo "playbook ran"
if [ -f prompt.flag ]; then
  if read answer < /dev/tty; then echo "answered"; exit 0; fi
  echo "no terminal to prompt on"
  exit 2
fi
exit 0
"#,
        out = record.display()
    )
}

pub struct Host {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub cfg: ExecutorConfig,
}

impl Host {
    pub fn new(policy: &str) -> Self {
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

    /// What the fake playbook recorded; empty when it never ran.
    pub fn record(&self) -> String {
        std::fs::read_to_string(self.root.join("record/record")).unwrap_or_default()
    }

    pub fn ledger(&self) -> String {
        std::fs::read_to_string(self.cfg.state_dir.join("ledger.jsonl")).unwrap_or_default()
    }
}

/// The `arg:` lines of a record.
pub fn args(record: &str) -> Vec<&str> {
    record
        .lines()
        .filter_map(|l| l.strip_prefix("arg:"))
        .collect()
}

/// The value of one `key:` line of a record.
pub fn field<'a>(record: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    record.lines().find_map(|l| l.strip_prefix(prefix.as_str()))
}

/// A Git repository holding `manifest` at `.keel/jobs.yaml`, `playbooks/web.yaml`,
/// `inventories/prod/hosts.yaml` and `files` (path, contents). With `key`, two commits (oldest
/// first) SSH-signed by the private key at `key`; otherwise one. Returns the commits and a pack
/// of HEAD.
pub fn repo(
    root: &Path,
    manifest: &str,
    files: &[(&str, &str)],
    key: Option<&Path>,
) -> (Vec<String>, Vec<u8>) {
    let repo = root.join("repo");
    let mut all: Vec<(&str, &str)> = vec![
        (".keel/jobs.yaml", manifest),
        ("playbooks/web.yaml", "- hosts: all\n  tasks: []\n"),
        (
            "inventories/prod/hosts.yaml",
            "all:\n  hosts:\n    web1: {}\n",
        ),
    ];
    all.extend_from_slice(files);
    for (path, contents) in all {
        let p = repo.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }
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

/// An SSH key for signing commits, and its `allowed_signers` line.
pub fn signing_key(root: &Path) -> (PathBuf, String) {
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

/// A ledger in which `job` already ran `base` to success, so a signed run from `base` needs no
/// baseline approval.
pub fn seeded_ledger(job: &str, base: &str) -> String {
    let mut seed = keel_trust::Ledger::empty();
    let mut text = seed
        .append(Entry::Start {
            at: "2026-01-01T00:00:00Z".to_owned(),
            run: "seed-run".to_owned(),
            job: job.to_owned(),
            base: None,
            target: CommitId::parse(base).unwrap(),
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
                job: job.to_owned(),
                outcome: Outcome::Succeeded,
                exit: Some(0),
            })
            .unwrap(),
    );
    text
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// An `Assign` for `job` at `target`.
pub fn assign(run_id: &str, job: &str, base: Option<&str>, target: &str, pack: &[u8]) -> Frame {
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
        check: false,
    }
}

/// Starts `serve` on one end of a socket pair and returns the other end.
pub fn start(cfg: &ExecutorConfig) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let cfg = cfg.clone();
    let task = tokio::spawn(async move { serve(theirs, &cfg).await });
    (ours, task)
}

pub async fn send(s: &mut UnixStream, frames: impl IntoIterator<Item = Frame>) {
    for f in frames {
        write_frame(s, &f).await.unwrap();
    }
}

/// Reads frames until the connection closes.
pub async fn drain(s: &mut UnixStream) -> Vec<Frame> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), read_frame(s)).await {
            Ok(Ok(Some(f))) => out.push(f),
            Ok(Ok(None)) | Ok(Err(_)) => return out,
            Err(_) => panic!("the executor did not finish; got {out:?}"),
        }
    }
}

/// Sends `assign`, the pack, `secrets` and (in signed mode) empty approvals, and returns every
/// frame the executor answered with.
pub async fn run(
    host: &Host,
    assign: Frame,
    pack: &[u8],
    secrets: Vec<SecretItem>,
    signed: bool,
) -> Vec<Frame> {
    let Frame::Assign { run_id, .. } = &assign else {
        panic!("not an Assign");
    };
    let run_id = run_id.clone();
    let (mut s, task) = start(&host.cfg);
    send(&mut s, [assign]).await;
    send(&mut s, pack_builder::frames(&run_id, pack)).await;
    send(
        &mut s,
        [Frame::Secrets {
            run_id: run_id.clone(),
            items: secrets,
        }],
    )
    .await;
    if signed {
        send(
            &mut s,
            [Frame::Approvals {
                run_id,
                items: vec![],
            }],
        )
        .await;
    }
    let frames = drain(&mut s).await;
    task.await.unwrap();
    frames
}

/// Everything the run logged, as text.
pub fn logs(frames: &[Frame]) -> String {
    frames
        .iter()
        .filter_map(|f| match f {
            Frame::Log { bytes, .. } => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect()
}
