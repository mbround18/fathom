//! `keel-executor` (spec 055 T085, contracts/runner.md, research R2): one connection on the
//! executor socket is one request, served as the `automation` user.
//!
//! - `ReadLedger` → `Ledger` (head and per-job bases, for the network half's `Hello`).
//! - `Assign` → one run. The executor trusts nothing the network half says. In order it checks
//!   its local policy, its ledger (`ledger_corrupt`), that the run id is new (`replay`), that
//!   Keel's base matches its own (`base_mismatch`), signed mode (`unsigned`), and the
//!   ansible-core floor (`version_too_old`). It then collects the pack and secrets, verifies the
//!   pack's size and SHA-256, unpacks it into its own Git store, reads `.keel/jobs.yaml` from the
//!   target commit, checks the job's profiles against `allowed_profiles`
//!   (`profile_not_allowed`), checks the commit out into a private run directory and runs
//!   `ansible-playbook` against this host only: `--limit <inventory_name>` and
//!   `ansible_connection=local` are forced, whatever the job says.
//!
//! A run that cannot be set up (a bad pack, a job that does not exist, a credential the host does
//! not accept, ...) ends with a `Log` saying why and `Finished` with exit code
//! [`SETUP_FAILED`], without `Started`. The run directory, and every secret in it, is removed
//! before `Finished` is sent.

mod process;
pub mod secrets;
pub mod store;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use keel_core::manifest::{self, InventorySource, MANIFEST_PATH};
use keel_exec::frames::{Approval, Base, Frame, LogStream, Refusal, SecretItem};
use keel_exec::versions::{MIN_ANSIBLE_CORE, meets_min_ansible_core};
use keel_trust::ledger::Ledger;
use keel_trust::object::CommitId;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::policy::Policy;
use crate::runner::session::{spawn_reader, spawn_writer};

pub use process::EVENT_FD;

/// `Finished.exit_code` for a run that never started.
pub const SETUP_FAILED: i32 = 125;
/// `Finished.exit_code` for a run cancelled before it started (SIGINT's convention).
pub const CANCELLED: i32 = 130;
/// The ledger file under the state directory.
pub const LEDGER_FILE: &str = "ledger.jsonl";
/// How long the first frame may take.
const FIRST_FRAME: Duration = Duration::from_secs(30);

/// Where things are. Defaults match contracts/host-install.md.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// `/var/lib/keel-executor`: the ledger, the Git store and run directories.
    pub state_dir: PathBuf,
    /// `/etc/keel-runner/trust`.
    pub trust_dir: PathBuf,
    /// `/opt/keel-runner/venv`.
    pub venv: PathBuf,
    /// `ansible-playbook`; `<venv>/bin/ansible-playbook` in production.
    pub ansible_playbook: PathBuf,
    /// Where the `keel_events` callback plugin lives (`ANSIBLE_CALLBACK_PLUGINS`).
    pub callback_dir: PathBuf,
    /// Between SIGINT and SIGKILL after `Cancel` or the deadline.
    pub cancel_grace: Duration,
    /// The largest pack accepted.
    pub max_pack_bytes: u64,
}

impl ExecutorConfig {
    /// The production layout under a venv.
    pub fn with_defaults(state_dir: PathBuf, trust_dir: PathBuf, venv: PathBuf) -> Self {
        Self {
            ansible_playbook: venv.join("bin/ansible-playbook"),
            callback_dir: venv.join("share/keel/callback"),
            state_dir,
            trust_dir,
            venv,
            cancel_grace: Duration::from_secs(30),
            max_pack_bytes: 1024 * 1024 * 1024,
        }
    }

    fn ledger_path(&self) -> PathBuf {
        self.state_dir.join(LEDGER_FILE)
    }
}

/// Serves one connection: `ReadLedger` or one run.
pub async fn serve<S>(stream: S, cfg: &ExecutorConfig)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (r, w) = tokio::io::split(stream);
    let (mut incoming, reader) = spawn_reader(r);
    let (out, writer) = spawn_writer(w);
    match tokio::time::timeout(FIRST_FRAME, incoming.recv()).await {
        Ok(Some(Frame::ReadLedger)) => {
            let _ = out.send(ledger_answer(cfg));
        }
        Ok(Some(assign @ Frame::Assign { .. })) => run(assign, cfg, &out, &mut incoming).await,
        Ok(Some(other)) => eprintln!(
            "keel-executor: the first frame was {}; closing",
            other.name()
        ),
        Ok(None) | Err(_) => {}
    }
    // Let the writer send what is queued, then close.
    drop(out);
    let _ = writer.await;
    reader.abort();
}

/// Reads the ledger. A missing file is an empty ledger; anything unreadable is corrupt.
fn read_ledger(cfg: &ExecutorConfig) -> Result<Ledger, String> {
    match std::fs::read_to_string(cfg.ledger_path()) {
        Ok(text) => Ledger::parse(&text).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::empty()),
        Err(e) => Err(e.kind().to_string()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `Ledger { ledger_head, bases }`. An empty ledger has no head. A corrupt one reports no head
/// and no bases, so Keel sees a host with nothing applied, and every run is then refused
/// `ledger_corrupt`.
fn ledger_answer(cfg: &ExecutorConfig) -> Frame {
    match read_ledger(cfg) {
        Ok(ledger) => Frame::Ledger {
            ledger_head: (!ledger.lines().is_empty()).then(|| hex(&ledger.head())),
            bases: ledger
                .bases()
                .into_iter()
                .map(|(job, commit)| Base {
                    job,
                    commit: commit.as_str().to_owned(),
                })
                .collect(),
        },
        Err(e) => {
            eprintln!("keel-executor: the ledger is unreadable: {e}");
            Frame::Ledger {
                ledger_head: None,
                bases: vec![],
            }
        }
    }
}

fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Why a run did not start.
enum Stop {
    Refuse(Refusal, String),
    Fail(String),
    Cancelled,
    /// The socket closed: nobody to tell.
    Gone,
}

fn fail(msg: impl Into<String>) -> Stop {
    Stop::Fail(msg.into())
}

/// The run's private directory, removed (with every secret in it) on drop.
struct RunDir(PathBuf);

impl RunDir {
    fn create(state_dir: &Path, run_id: &str) -> Result<Self, String> {
        let runs = state_dir.join("runs");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&runs)
            .map_err(|e| format!("runs directory: {e}"))?;
        let dir = runs.join(run_id);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("run directory: {e}"))?;
        Ok(Self(dir))
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("keel-executor: could not remove {}: {e}", self.0.display());
        }
    }
}

/// What the network half sent after `Assign`.
struct Delivery {
    pack: Vec<u8>,
    secrets: Vec<SecretItem>,
    /// Kept for the signed-mode override checks (T098).
    _approvals: Vec<Approval>,
}

struct Assignment {
    run_id: String,
    job: String,
    base: Option<String>,
    target: String,
    deadline: u64,
    pack_size: u64,
}

async fn run(
    assign: Frame,
    cfg: &ExecutorConfig,
    out: &mpsc::UnboundedSender<Frame>,
    incoming: &mut mpsc::Receiver<Frame>,
) {
    let Frame::Assign {
        run_id,
        job,
        base,
        target,
        deadline,
        pack_size,
    } = assign
    else {
        return;
    };
    if !valid_run_id(&run_id) {
        eprintln!("keel-executor: refused a run id with unexpected characters; closing");
        return;
    }
    let a = Assignment {
        run_id,
        job,
        base,
        target,
        deadline,
        pack_size,
    };
    let run_id = a.run_id.clone();
    match prepare_and_run(&a, cfg, out, incoming).await {
        Ok(()) => {}
        Err(Stop::Gone) => {}
        Err(Stop::Refuse(reason, detail)) => {
            let _ = out.send(Frame::Refused {
                run_id,
                reason,
                detail,
            });
        }
        Err(Stop::Fail(msg)) => finish_unstarted(out, &run_id, SETUP_FAILED, &msg),
        Err(Stop::Cancelled) => finish_unstarted(out, &run_id, CANCELLED, "cancelled before start"),
    }
}

fn finish_unstarted(out: &mpsc::UnboundedSender<Frame>, run_id: &str, code: i32, msg: &str) {
    eprintln!("keel-executor: run {run_id}: {msg}");
    let _ = out.send(Frame::Log {
        run_id: run_id.to_owned(),
        seq: 0,
        stream: LogStream::Stderr,
        bytes: format!("keel-executor: {msg}\n").into_bytes(),
    });
    let _ = out.send(Frame::Finished {
        run_id: run_id.to_owned(),
        exit_code: code,
        stats: BTreeMap::new(),
        ledger_line: String::new(),
    });
}

/// The checks that need nothing but the assignment and local state.
fn check_assignment(a: &Assignment, cfg: &ExecutorConfig) -> Result<Policy, Stop> {
    let policy = Policy::load(&cfg.trust_dir).map_err(|e| fail(e.to_string()))?;
    let ledger = read_ledger(cfg)
        .map_err(|e| Stop::Refuse(Refusal::LedgerCorrupt, format!("ledger: {e}")))?;
    if ledger.has_start(&a.run_id) {
        return Err(Stop::Refuse(
            Refusal::Replay,
            "this run already started on this host".to_owned(),
        ));
    }
    let local = ledger.base(&a.job).map(|c| c.as_str().to_owned());
    if local != a.base {
        return Err(Stop::Refuse(
            Refusal::BaseMismatch,
            format!(
                "Keel's base is {}, this host's is {}",
                a.base.as_deref().unwrap_or("none"),
                local.as_deref().unwrap_or("none")
            ),
        ));
    }
    if policy.require_signed {
        // Signed-mode verification (contracts/signed-mode.md) is T098. Until it exists a host
        // that requires signed work runs nothing.
        return Err(Stop::Refuse(
            Refusal::Unsigned,
            "this host requires signed work, and signed-mode verification is not built yet (T098)"
                .to_owned(),
        ));
    }
    let installed = crate::venv::ansible_core_version(&cfg.venv);
    if !meets_min_ansible_core(&installed) {
        return Err(Stop::Refuse(
            Refusal::VersionTooOld,
            format!("ansible-core {installed} is below {MIN_ANSIBLE_CORE}"),
        ));
    }
    CommitId::parse(&a.target).map_err(|_| fail("the target is not a commit id"))?;
    Ok(policy)
}

/// Collects `Pack` until the last chunk and `Secrets`, and verifies the pack.
async fn collect(
    a: &Assignment,
    cfg: &ExecutorConfig,
    incoming: &mut mpsc::Receiver<Frame>,
    deadline: Instant,
) -> Result<Delivery, Stop> {
    if a.pack_size > cfg.max_pack_bytes {
        return Err(fail(format!(
            "the pack is {} bytes, more than the {} allowed",
            a.pack_size, cfg.max_pack_bytes
        )));
    }
    let mut pack = Vec::new();
    let mut pack_sha = None;
    let mut secrets = None;
    let mut approvals = None;
    while pack_sha.is_none() || secrets.is_none() {
        let frame = tokio::select! {
            f = incoming.recv() => f.ok_or(Stop::Gone)?,
            () = tokio::time::sleep_until(deadline) => return Err(fail("the deadline passed before the pack arrived")),
        };
        match frame {
            Frame::Pack {
                run_id,
                chunk,
                last,
                sha256,
            } if run_id == a.run_id && pack_sha.is_none() => {
                if pack.len() as u64 + chunk.len() as u64 > a.pack_size {
                    return Err(fail("the pack is larger than announced"));
                }
                pack.extend_from_slice(&chunk);
                if last {
                    pack_sha =
                        Some(sha256.ok_or_else(|| fail("the last pack chunk has no SHA-256"))?);
                } else if sha256.is_some() {
                    return Err(fail("a pack chunk before the last carries a SHA-256"));
                }
            }
            Frame::Secrets { run_id, items } if run_id == a.run_id && secrets.is_none() => {
                secrets = Some(items);
            }
            Frame::Approvals { run_id, items } if run_id == a.run_id && approvals.is_none() => {
                approvals = Some(items);
            }
            Frame::Cancel { run_id } if run_id == a.run_id => return Err(Stop::Cancelled),
            other => {
                return Err(fail(format!(
                    "unexpected {} while receiving the run",
                    other.name()
                )));
            }
        }
    }
    if pack.len() as u64 != a.pack_size {
        return Err(fail("the pack is incomplete (pack_incomplete)"));
    }
    let want = pack_sha.unwrap_or_default();
    if hex(&Sha256::digest(&pack)) != want.to_ascii_lowercase() {
        return Err(fail("the pack's SHA-256 does not match (pack_incomplete)"));
    }
    Ok(Delivery {
        pack,
        secrets: secrets.unwrap_or_default(),
        _approvals: approvals.unwrap_or_default(),
    })
}

/// The ledger line for a finished run. Appending signed start/end lines to the ledger is
/// signed mode's job (T098, research R19); until then nothing is appended, so `ledger_line` is
/// empty and the replay check above only sees lines written by a later version.
fn record_in_ledger_t098(_a: &Assignment, _exit_code: i32) -> String {
    String::new()
}

async fn prepare_and_run(
    a: &Assignment,
    cfg: &ExecutorConfig,
    out: &mpsc::UnboundedSender<Frame>,
    incoming: &mut mpsc::Receiver<Frame>,
) -> Result<(), Stop> {
    let policy = check_assignment(a, cfg)?;
    let now = crate::runner::unix_now();
    if a.deadline <= now {
        return Err(fail("the run's deadline has already passed"));
    }
    let mut deadline = Instant::now() + Duration::from_secs(a.deadline - now);
    let delivery = collect(a, cfg, incoming, deadline).await?;

    let store = store::Store::open(&cfg.state_dir.join("git")).map_err(|e| fail(e.to_string()))?;
    store
        .unpack(&delivery.pack)
        .map_err(|e| fail(e.to_string()))?;
    let (def, inventory_path) = read_job(a, &store, &policy, &delivery.secrets)?;
    let def = &def;
    drop(delivery.pack);
    if let Some(secs) = def.timeout_secs {
        deadline = deadline.min(Instant::now() + Duration::from_secs(secs));
    }

    let run_dir = RunDir::create(&cfg.state_dir, &a.run_id).map_err(fail)?;
    let tree_dir = run_dir.0.join("tree");
    store
        .checkout(&a.target, &tree_dir)
        .map_err(|e| fail(e.to_string()))?;
    let tmp = run_dir.0.join("tmp");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&tmp)
        .map_err(|e| fail(format!("tmp: {e}")))?;
    let secrets =
        secrets::materialize(&run_dir.0, &delivery.secrets).map_err(|e| fail(e.to_string()))?;
    let jobvars = run_dir.0.join("jobvars.json");
    let vars = serde_json::to_vec(&def.vars).map_err(|e| fail(e.to_string()))?;
    crate::write_private_new(&jobvars, &vars).map_err(|e| fail(format!("jobvars: {e}")))?;
    let forced = run_dir.0.join("forced.json");
    crate::write_private_new(&forced, br#"{"ansible_connection":"local"}"#)
        .map_err(|e| fail(format!("forced vars: {e}")))?;

    let at = |p: &Path| {
        let mut s = OsString::from("@");
        s.push(p.as_os_str());
        s
    };
    let mut args: Vec<OsString> = vec![
        "-i".into(),
        tree_dir.join(&inventory_path).into(),
        "--limit".into(),
        policy.inventory_name.clone().into(),
        "-e".into(),
        at(&jobvars),
        "-e".into(),
        at(&run_dir.0.join(secrets::SECRET_VARS_FILE)),
        // Last, so nothing above can override it.
        "-e".into(),
        at(&forced),
    ];
    args.extend(secrets.vault_args.iter().cloned());
    args.push(def.playbook.clone().into());

    let mut path = OsString::from(cfg.venv.join("bin"));
    path.push(":/usr/local/bin:/usr/bin:/bin");
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), path),
        ("HOME".into(), run_dir.0.clone().into()),
        ("LANG".into(), "C.UTF-8".into()),
        ("ANSIBLE_CALLBACKS_ENABLED".into(), "keel_events".into()),
        (
            "ANSIBLE_CALLBACK_PLUGINS".into(),
            cfg.callback_dir.clone().into(),
        ),
        ("ANSIBLE_LOCAL_TEMP".into(), tmp.into()),
        ("ANSIBLE_NOCOLOR".into(), "1".into()),
        ("ANSIBLE_RETRY_FILES_ENABLED".into(), "0".into()),
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("KEEL_EVENT_FD".into(), EVENT_FD.to_string().into()),
    ];
    env.extend(
        secrets
            .env
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v))),
    );

    out.send(Frame::Started {
        run_id: a.run_id.clone(),
    })
    .map_err(|_| Stop::Gone)?;
    let inv = process::Invocation {
        program: cfg.ansible_playbook.clone(),
        args,
        cwd: tree_dir,
        env,
    };
    let (exit_code, stats) =
        match process::run(inv, &a.run_id, out, incoming, deadline, cfg.cancel_grace).await {
            Ok(o) => (o.exit_code, o.stats),
            Err(e) => {
                let msg = format!("keel-executor: cannot run ansible-playbook: {e}\n");
                eprint!("{msg}");
                let _ = out.send(Frame::Log {
                    run_id: a.run_id.clone(),
                    seq: 0,
                    stream: LogStream::Stderr,
                    bytes: msg.into_bytes(),
                });
                (SETUP_FAILED, BTreeMap::new())
            }
        };
    drop(run_dir);
    let ledger_line = record_in_ledger_t098(a, exit_code);
    let _ = out.send(Frame::Finished {
        run_id: a.run_id.clone(),
        exit_code,
        stats,
        ledger_line,
    });
    Ok(())
}

/// Reads `.keel/jobs.yaml` at the target, validates it, and checks the job against this host:
/// its profiles against `allowed_profiles`, its credentials against what was sent, and its
/// inventory (repo inventories only). Synchronous: a `gix::Repository` must not live across an
/// await.
fn read_job(
    a: &Assignment,
    store: &store::Store,
    policy: &Policy,
    secrets: &[SecretItem],
) -> Result<(manifest::JobDefinition, String), Stop> {
    let tree = store.tree(&a.target).map_err(|e| fail(e.to_string()))?;
    let manifest_bytes = tree
        .read_file(MANIFEST_PATH)
        .ok_or_else(|| fail(format!("{MANIFEST_PATH} is missing at the target commit")))?;
    let ctx = manifest::Context {
        signed_mode: policy.require_signed,
        granted_credentials: secrets.iter().map(|s| s.name.clone()).collect(),
    };
    let manifest = manifest::validate(&manifest_bytes, &tree, &ctx).map_err(|issues| {
        let first = issues
            .first()
            .map(|i| format!("{}: {}", i.at, i.message))
            .unwrap_or_default();
        fail(format!("{MANIFEST_PATH} is invalid ({first})"))
    })?;
    let job = manifest
        .jobs
        .get(&a.job)
        .ok_or_else(|| fail(format!("job {:?} is not in {MANIFEST_PATH}", a.job)))?;
    let def = &job.definition;
    if !policy.allows_any(&def.profiles) {
        return Err(Stop::Refuse(
            Refusal::ProfileNotAllowed,
            format!(
                "the job's profiles {:?} are not in this host's allowed_profiles",
                def.profiles
            ),
        ));
    }
    if !job.missing_credentials.is_empty() {
        return Err(fail(format!(
            "credentials not sent: {}",
            job.missing_credentials.join(", ")
        )));
    }
    if def.inventory_source != InventorySource::Repo {
        return Err(fail(
            "managed inventories are not available on a host runner",
        ));
    }
    let inventory = manifest
        .inventories
        .get(&def.inventory)
        .ok_or_else(|| fail(format!("inventory {:?} is not declared", def.inventory)))?;
    Ok((def.clone(), inventory.path.clone()))
}
