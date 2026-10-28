//! T093 (SC-009): a Keel server with the right host CA turns hostile. Over a real session (the
//! runner's client side, relaying to a real executor on its socket) it sends five assignments a
//! signed-mode host must refuse, each for its own reason:
//!
//! - an unsigned commit with approvals that carry no passkey assertion → `unsigned`
//! - a base that is not the host's → `base_mismatch` (the detail names the host's base)
//! - a target older than the host's base → `downgrade`
//! - approvals that do not name this host → `bad_override`
//! - a run id the host's ledger already holds → `replay`
//!
//! Nothing runs, no run directory is left behind, and the ledger is unchanged byte for byte.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, HostCertRequest, RunnerCertRequest, RunnerCertVerifier, RunnerIdentity, SshCa,
    TrustedCa,
};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Approval, Frame, Refusal};
use keel_exec::krl::Krl;
use keel_exec::pack_builder;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_runner::executor::{ExecutorConfig, LEDGER_FILE, serve as serve_executor};
use keel_runner::runner::{CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig};
use keel_trust::CommitId;
use keel_trust::ledger::{Auth, Entry, Ledger, Outcome};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::Msg;
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::mpsc;

const SERVER_NAME: &str = "localhost";
const INVENTORY_NAME: &str = "web-1";
const JOB: &str = "web-deploy";
const SEED_RUN: &str = "seed-run";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(60);

const MANIFEST: &str = r#"
version: 1
profiles:
  base: { description: "Every host" }
inventories:
  prod:
    path: inventories/prod/hosts.yaml
jobs:
  web-deploy:
    playbook: playbooks/site.yaml
    profiles: [base]
    inventory: prod
"#;

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Pki {
    host_ca: SshCa,
    runner_ca: SshCa,
}

/// A signed-mode host: the runner's trust and state, and the executor's state and venv.
struct Host {
    _dir: tempfile::TempDir,
    trust: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    exec: ExecutorConfig,
    marker: PathBuf,
}

impl Host {
    fn new(pki: &Pki) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let trust = root.join("trust");
        let state = root.join("state");
        let exec_state = root.join("exec-state");
        let venv = root.join("venv");
        for d in [&trust, &state, &exec_state] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            trust.join("policy.toml"),
            format!(
                "require_signed = true\norigin = \"https://keel.test\"\nrp_id = \"keel.test\"\n\
                 inventory_name = \"{INVENTORY_NAME}\"\nallowed_profiles = [\"base\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            trust.join("host_ca.pub"),
            format!("{}\n", pki.host_ca.cert_authority_line(SERVER_NAME)),
        )
        .unwrap();
        // Signed mode with nobody trusted: no commit can be signed well enough, and no passkey
        // is pinned.
        std::fs::write(trust.join("signers"), "").unwrap();
        std::fs::write(trust.join("approvers.json"), "[]").unwrap();
        std::fs::create_dir_all(
            venv.join("lib/python3.12/site-packages/ansible_core-2.21.5.dist-info"),
        )
        .unwrap();
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        let marker = root.join("playbook-ran");
        let playbook = venv.join("bin/ansible-playbook");
        std::fs::write(
            &playbook,
            format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&playbook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exec = ExecutorConfig::with_defaults(exec_state, trust.clone(), venv);
        let socket = root.join("executor.sock");
        Host {
            _dir: dir,
            trust,
            state,
            socket,
            exec,
            marker,
        }
    }

    fn runner(&self, port: u16) -> Runner {
        Runner::load(RunnerConfig {
            server: format!("{SERVER_NAME}:{port}"),
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.socket.clone(),
            venv: self.state.join("no-venv"),
        })
        .unwrap()
    }

    fn enroll(&self, pki: &Pki, key: &KeyData) {
        let profiles = vec!["base".to_owned()];
        let cert = pki
            .runner_ca
            .issue_runner_cert(&RunnerCertRequest {
                runner_id: "r-1",
                inventory_name: INVENTORY_NAME,
                assigned_profiles: &profiles,
                allowed_profiles: &profiles,
                public_key: key,
                ttl: DAY,
                now: SystemTime::now(),
            })
            .unwrap();
        std::fs::write(
            self.state.join(CERT_FILE),
            format!("{}\n", cert.to_openssh().unwrap()),
        )
        .unwrap();
        std::fs::write(self.state.join(HOST_ID_FILE), "h-1\n").unwrap();
    }

    fn ledger(&self) -> String {
        std::fs::read_to_string(self.exec.state_dir.join(LEDGER_FILE)).unwrap()
    }

    fn run_dirs(&self) -> usize {
        std::fs::read_dir(self.exec.state_dir.join("runs"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

/// The real executor on the host's socket, one `serve` per connection.
fn executor(host: &Host) {
    let listener = UnixListener::bind(&host.socket).unwrap();
    let cfg = host.exec.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let cfg = cfg.clone();
            tokio::spawn(async move { serve_executor(stream, &cfg).await });
        }
    });
}

/// Three unsigned commits A ← B ← C; returns their ids (oldest first) and a pack of all three.
fn repo(root: &Path) -> (Vec<String>, Vec<u8>) {
    let repo = root.join("repo");
    for d in ["playbooks", "inventories/prod", ".keel"] {
        std::fs::create_dir_all(repo.join(d)).unwrap();
    }
    std::fs::write(repo.join(".keel/jobs.yaml"), MANIFEST).unwrap();
    std::fs::write(
        repo.join("inventories/prod/hosts.yaml"),
        format!("all:\n  hosts:\n    {INVENTORY_NAME}: {{}}\n"),
    )
    .unwrap();
    let git = |args: &[&str], stdin: Option<&[u8]>| -> Vec<u8> {
        let mut child = Command::new("git")
            .current_dir(&repo)
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
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
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
    let mut commits = vec![];
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        std::fs::write(
            repo.join("playbooks/site.yaml"),
            format!("# {i}\n- hosts: all\n  tasks: []\n"),
        )
        .unwrap();
        git(&["add", "-A"], None);
        git(&["commit", "-q", "-m", name], None);
        let id = String::from_utf8(git(&["rev-parse", "HEAD"], None)).unwrap();
        commits.push(id.trim().to_owned());
    }
    let pack = git(
        &["pack-objects", "--stdout", "--revs", "-q"],
        Some(b"HEAD\n"),
    );
    (commits, pack)
}

/// The host already ran `base` for the job, as `SEED_RUN`.
fn seed_ledger(host: &Host, base: &str) {
    let mut ledger = Ledger::empty();
    let mut text = ledger
        .append(Entry::Start {
            at: "2026-01-01T00:00:00Z".to_owned(),
            run: SEED_RUN.to_owned(),
            job: JOB.to_owned(),
            base: None,
            target: CommitId::parse(base).unwrap(),
            auth: Auth::Signed {
                signers: vec!["SHA256:seed".to_owned()],
            },
        })
        .unwrap();
    text.push_str(
        &ledger
            .append(Entry::End {
                at: "2026-01-01T00:01:00Z".to_owned(),
                run: SEED_RUN.to_owned(),
                job: JOB.to_owned(),
                outcome: Outcome::Succeeded,
                exit: Some(0),
            })
            .unwrap(),
    );
    std::fs::write(host.exec.state_dir.join(LEDGER_FILE), text).unwrap();
}

struct Authority {
    verifier: Arc<RunnerCertVerifier>,
    enrolled: Vec<KeyData>,
}

impl RunnerAuthority for Authority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, key: &KeyData) -> bool {
        self.enrolled.contains(key)
    }

    async fn bare_key(&self, _key: &KeyData, _now: SystemTime) -> BareKey {
        BareKey::Unknown
    }
}

/// The hostile Keel: after `Hello` it sends every frame in `script`, records what comes back,
/// and ends the session once `runs` runs have ended (`Refused` or `Finished`).
struct Hostile {
    seen: mpsc::UnboundedSender<Frame>,
    script: Vec<Frame>,
    runs: usize,
}

impl SubsystemService for Hostile {
    async fn serve(&self, access: Access, mut stream: ChannelStream<Msg>) {
        let mut ended = 0;
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            if !access.permits(&frame) {
                return;
            }
            let _ = self.seen.send(frame.clone());
            match frame {
                Frame::Hello { .. } => {
                    for f in &self.script {
                        if write_frame(&mut stream, f).await.is_err() {
                            return;
                        }
                    }
                }
                Frame::Refused { .. } | Frame::Finished { .. } => {
                    ended += 1;
                    if ended == self.runs {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

/// A server whose host certificate comes from the CA the host trusts.
async fn hostile_server(
    pki: &Pki,
    enrolled: KeyData,
    script: Vec<Frame>,
    runs: usize,
) -> (u16, mpsc::UnboundedReceiver<Frame>) {
    let host_key = key();
    let cert: Certificate = pki
        .host_ca
        .issue_host_cert(&HostCertRequest {
            host_names: &[SERVER_NAME.to_owned()],
            public_key: host_key.public_key().key_data(),
            ttl: DAY,
            now: SystemTime::now(),
        })
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = server_config(host_key, Some(cert));
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let authority = Arc::new(Authority {
        verifier: Arc::new(RunnerCertVerifier::new(
            vec![TrustedCa::active(pki.runner_ca.key_data().clone())],
            Krl::default(),
        )),
        enrolled: vec![enrolled],
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let service = Arc::new(Hostile {
        seen: tx,
        script,
        runs,
    });
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = KeelSshHandler::new(Arc::clone(&authority), Arc::clone(&service));
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (port, rx)
}

/// One run's frames: `Assign`, the pack, empty secrets and the given approvals.
fn run(
    run_id: &str,
    base: Option<&str>,
    target: &str,
    pack: &[u8],
    approvals: Vec<Approval>,
) -> Vec<Frame> {
    let mut frames = vec![Frame::Assign {
        run_id: run_id.to_owned(),
        workspace: "ws-1".to_owned(),
        binding: "main".to_owned(),
        job: JOB.to_owned(),
        base: base.map(str::to_owned),
        target: target.to_owned(),
        deadline: now() + 600,
        pack_size: pack.len() as u64,
        quorum: None,
    }];
    frames.extend(pack_builder::frames(run_id, pack));
    frames.push(Frame::Secrets {
        run_id: run_id.to_owned(),
        items: vec![],
    });
    frames.push(Frame::Approvals {
        run_id: run_id.to_owned(),
        items: approvals,
    });
    frames
}

/// An approval for `host` with the given assertion fields (base64url).
fn approval(
    host: &str,
    authenticator_data: &str,
    client_data_json: &str,
    signature: &str,
) -> Approval {
    Approval {
        kind: "override".to_owned(),
        approver: "mallory".to_owned(),
        credential_id: "Y3JlZGVudGlhbA".to_owned(),
        authenticator_data: authenticator_data.to_owned(),
        client_data_json: client_data_json.to_owned(),
        signature: signature.to_owned(),
        expires_at: now() + 3600,
        comment: "trust me".to_owned(),
        hosts: vec![host.to_owned()],
    }
}

/// Run id, Keel's base, target, approvals, and the refusal the host must answer with.
type Case<'a> = (&'a str, Option<&'a str>, &'a str, Vec<Approval>, Refusal);

#[tokio::test]
async fn a_hostile_server_with_the_right_ca_gets_every_run_refused() {
    let pki = Pki {
        host_ca: SshCa::generate(CaKind::Host).unwrap(),
        runner_ca: SshCa::generate(CaKind::Runner).unwrap(),
    };
    let host = Host::new(&pki);
    let (commits, pack) = repo(&host._dir.path().join("src"));
    let (a, b, c) = (&commits[0], &commits[1], &commits[2]);
    seed_ledger(&host, b);
    let ledger_before = host.ledger();

    // An approval with a plausible assertion, given for another host.
    let other_host = approval(
        "web-2",
        "eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eA",
        "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiQUFBQSIsIm9yaWdpbiI6Imh0dHBzOi8va2VlbC50ZXN0In0",
        "c3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzcw",
    );
    let cases: [Case; 5] = [
        (
            "run-unsigned",
            Some(b),
            c,
            vec![
                approval(INVENTORY_NAME, "", "", ""),
                approval(INVENTORY_NAME, "", "", ""),
            ],
            Refusal::Unsigned,
        ),
        ("run-wrong-base", Some(c), c, vec![], Refusal::BaseMismatch),
        ("run-downgrade", Some(b), a, vec![], Refusal::Downgrade),
        (
            "run-other-host",
            Some(b),
            c,
            vec![other_host],
            Refusal::BadOverride,
        ),
        (SEED_RUN, Some(b), c, vec![], Refusal::Replay),
    ];
    let mut script = vec![];
    for (run_id, base, target, approvals, _) in &cases {
        script.extend(run(run_id, *base, target, &pack, approvals.clone()));
    }

    let key_data = host.runner(1).public_key().clone();
    host.enroll(&pki, &key_data);
    let (port, mut seen) = hostile_server(&pki, key_data, script, cases.len()).await;
    executor(&host);
    tokio::time::timeout(WAIT, host.runner(port).run_once())
        .await
        .expect("hung")
        .unwrap();

    let mut frames = vec![];
    while let Ok(f) = seen.try_recv() {
        frames.push(f);
    }
    // The host told Keel its own base in Hello.
    match frames.first() {
        Some(Frame::Hello { bases, .. }) => {
            assert!(
                bases.iter().any(|x| x.job == JOB && x.commit == *b),
                "{bases:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    for (run_id, _, _, _, want) in &cases {
        let got: Vec<&Frame> = frames
            .iter()
            .filter(|f| match f {
                Frame::Started { run_id: r }
                | Frame::Refused { run_id: r, .. }
                | Frame::Event { run_id: r, .. }
                | Frame::Log { run_id: r, .. }
                | Frame::Finished { run_id: r, .. } => r == run_id,
                _ => false,
            })
            .collect();
        match got.as_slice() {
            [Frame::Refused { reason, detail, .. }] => {
                assert_eq!(reason, want, "{run_id}: {detail}");
                if *want == Refusal::BaseMismatch {
                    assert!(detail.contains(b.as_str()), "{run_id}: {detail}");
                }
                if *want == Refusal::BadOverride {
                    assert!(detail.contains("HostNotCovered"), "{run_id}: {detail}");
                }
            }
            other => panic!("{run_id}: {other:?}"),
        }
    }

    assert!(!host.marker.exists(), "a playbook ran");
    assert_eq!(host.run_dirs(), 0);
    assert_eq!(host.ledger(), ledger_before, "the ledger changed");
}
