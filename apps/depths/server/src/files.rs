//! The volume explorer's server side (spec 053): the gate every file command passes (R12), the
//! refusals it answers with, mount discovery (R10) and the helper pool's state (R2).
//!
//! Paths a person browses are audited (they are the point of the audit) but never logged or put
//! on a span.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use depths_core::StackName;
use depths_core::plan::{self, StackContext};
use depths_core::ports::{
    ChannelError, ContainerRuntime, HelperChannel, HelperSpec, OpMode, RuntimeError, VolumeHelper,
};
use depths_core::role::{self, CommandKind};
use depths_core::security::{self, Severity};
use depths_protocol::{
    AuditAction, AuditFile, AuditOutcome, Browsable, CreateKind, DenyReason, EntryKind, Event,
    FileContent, FileOp, FilePreview, FileRefusal, FileVersion, Fingerprint, Mount, MountId,
    MountKind, MountService, MountState, Name, NotBrowsable, NotBrowsableCode, PreviewId,
    PreviewVoidReason, RawName, RelPath, RuntimeContainerState, VersionId, VersionReason,
    WriteBlocked,
};
use depths_runtime_cli::helper::{HELPER_IDLE, ImageId, OneShot, PoolError, SystemClock};
use depths_runtime_cli::{CliRuntime, CliVolumeHelper, HelperPool};
use depths_volumes::frame::{Frame, FrameError, FrameKind};
use depths_volumes::mount::{BindRating, HelperSource, browsability};
use depths_volumes::op::{
    CreateKind as OpCreateKind, Hello, HelperError, Op, Reply, SizeCap, StatInfo,
};
use depths_volumes::text;

use crate::audit;
use crate::config::FilesMode;
use crate::http::AppState;
use crate::ops::Who;

/// A mount's owners as listed, by path.
type Owners = HashMap<RelPath, (u32, u32)>;

/// The explorer's run-time state: the configured mode, lowered to `off` (with a reason admins
/// see) when the helper cannot run here, and the helper pool once it is installed.
pub struct Files {
    state: RwLock<(FilesMode, Option<String>)>,
    pool: OnceLock<(Arc<dyn VolumeHelper>, HelperPool)>,
    /// Each stack's mounts as last discovered, so a command naming a mount id finds its source.
    known: Mutex<HashMap<StackName, Discovered>>,
    /// Owners seen in listings, per mount, so the next op on a path runs as its owner (R5).
    owners: Mutex<HashMap<(StackName, MountId), Owners>>,
    /// Where replaced content goes (R9).
    history: RwLock<Arc<dyn HistorySink>>,
    /// Unconfirmed file changes, by preview id.
    previews: Mutex<HashMap<PreviewId, Pending>>,
    ttl: RwLock<Duration>,
}

impl Files {
    pub fn new(mode: FilesMode) -> Self {
        Self {
            state: RwLock::new((mode, None)),
            pool: OnceLock::new(),
            known: Mutex::new(HashMap::new()),
            owners: Mutex::new(HashMap::new()),
            history: RwLock::new(Arc::new(NoHistory)),
            previews: Mutex::new(HashMap::new()),
            ttl: RwLock::new(PREVIEW_TTL),
        }
    }

    /// Where replaced content goes from now on.
    pub fn set_history(&self, sink: Arc<dyn HistorySink>) {
        *self.history.write().unwrap() = sink;
    }

    pub fn history(&self) -> Arc<dyn HistorySink> {
        self.history.read().unwrap().clone()
    }

    /// How long a file preview may be confirmed for ([`PREVIEW_TTL`] unless changed).
    pub fn set_preview_ttl(&self, ttl: Duration) {
        *self.ttl.write().unwrap() = ttl;
    }

    /// Whether `id` is a file preview of `session`'s, so `Confirm` comes here.
    pub fn has_preview(&self, session: &str, id: PreviewId) -> bool {
        self.previews
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|p| p.session == session)
    }

    /// The mode in force: the configured one, unless the helper forced it off.
    pub fn mode(&self) -> FilesMode {
        self.state.read().unwrap().0
    }

    /// Why the feature is off when the configuration has it on (`/api/me`, admins only).
    pub fn reason(&self) -> Option<String> {
        self.state.read().unwrap().1.clone()
    }

    /// Turns the feature off for this run because the helper cannot work here.
    pub fn force_off(&self, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::warn!(reason = %reason, "volume explorer turned off");
        *self.state.write().unwrap() = (FilesMode::Off, Some(reason));
    }

    /// Installs the helper and its pool, once.
    pub fn install(&self, helper: Arc<dyn VolumeHelper>, pool: HelperPool) {
        let _ = self.pool.set((helper, pool));
    }

    pub fn pool(&self) -> Option<&HelperPool> {
        self.pool.get().map(|(_, p)| p)
    }

    pub fn helper(&self) -> Option<&Arc<dyn VolumeHelper>> {
        self.pool.get().map(|(h, _)| h)
    }

    /// Checks a helper's first frame: without `openat2` no path can be resolved safely, so the
    /// feature goes off for this run.
    pub fn check_hello(&self, hello: &Hello) -> Result<(), FileRefusal> {
        if hello.openat2 {
            return Ok(());
        }
        self.force_off(NO_OPENAT2);
        Err(FileRefusal::HelperUnavailable {
            reason: NO_OPENAT2.into(),
        })
    }

    pub fn remember(&self, stack: &StackName, found: Discovered) {
        self.known.lock().unwrap().insert(stack.clone(), found);
    }

    /// The mount `id` of `stack` and its source, as last discovered.
    pub fn known(&self, stack: &StackName, id: &MountId) -> Option<(Mount, HelperSource)> {
        let known = self.known.lock().unwrap();
        let (m, src) = known.get(stack)?.get(id)?;
        Some((m.clone(), src.clone()))
    }

    /// The uid:gid `path` had when it was last listed or stat-ed.
    pub fn owner(&self, stack: &StackName, mount: &MountId, path: &RelPath) -> Option<(u32, u32)> {
        let owners = self.owners.lock().unwrap();
        owners
            .get(&(stack.clone(), mount.clone()))?
            .get(path)
            .copied()
    }

    /// Remembers owners seen in a listing or a stat. A mount's map is dropped once it holds
    /// [`OWNERS_MAX`] paths; later ops then stat their way down again.
    pub fn note_owners(
        &self,
        stack: &StackName,
        mount: &MountId,
        seen: impl IntoIterator<Item = (RelPath, (u32, u32))>,
    ) {
        let mut owners = self.owners.lock().unwrap();
        let map = owners.entry((stack.clone(), mount.clone())).or_default();
        for (path, owner) in seen {
            if map.len() >= OWNERS_MAX {
                map.clear();
            }
            map.insert(path, owner);
        }
    }

    /// Forgets `path`'s owner after the helper found it changed.
    pub fn forget_owner(&self, stack: &StackName, mount: &MountId, path: &RelPath) {
        if let Some(map) = self
            .owners
            .lock()
            .unwrap()
            .get_mut(&(stack.clone(), mount.clone()))
        {
            map.remove(path);
        }
    }
}

/// How many owners one mount's map keeps before it starts over.
const OWNERS_MAX: usize = 100_000;

// ---------------------------------------------------------------------------
// The helper image (R2)
// ---------------------------------------------------------------------------

/// Why the feature is off when no image can run the helper.
pub const NO_IMAGE: &str = "no helper image";
/// Why the feature is off when the helper cannot use `openat2`.
pub const NO_OPENAT2: &str = "kernel older than 5.6 or seccomp blocks openat2";

/// The image the helper runs, by id only: `DEPTHS_HELPER_IMAGE`, else the image of the
/// container this server runs in (`HOSTNAME` is its id). Blocking.
pub fn helper_image(
    configured: Option<&str>,
    hostname: Option<&str>,
    runtime: &dyn ContainerRuntime,
) -> Result<ImageId, String> {
    if let Some(raw) = configured {
        return ImageId::parse(raw)
            .map_err(|_| format!("{NO_IMAGE}: DEPTHS_HELPER_IMAGE is not an image id"));
    }
    let host = hostname.filter(|h| !h.is_empty()).ok_or(NO_IMAGE)?;
    let found = runtime
        .inspect_security(&[host.to_owned()])
        .map_err(|_| NO_IMAGE.to_owned())?;
    found
        .iter()
        .find(|i| !i.id.is_empty() && i.id.starts_with(host))
        .and_then(|i| ImageId::parse(&i.image).ok())
        .ok_or_else(|| NO_IMAGE.to_owned())
}

/// Sets the explorer up at start (blocking): finds the helper image, sweeps helpers a previous
/// run left behind and installs the pool. With no image, the feature is off for this run.
pub fn start(app: &AppState, cli: CliRuntime, hostname: Option<&str>) {
    if app.files.mode() == FilesMode::Off {
        return;
    }
    let image = match helper_image(app.cfg.files.helper_image.as_deref(), hostname, &cli) {
        Ok(image) => image,
        Err(why) => return app.files.force_off(why),
    };
    let helper: Arc<dyn VolumeHelper> = Arc::new(CliVolumeHelper::new(cli, image));
    match helper.sweep() {
        Ok(0) => {}
        Ok(n) => tracing::info!(removed = n, "removed volume helpers left by a previous run"),
        Err(e) => tracing::warn!(error = ?e, "cannot sweep old volume helpers"),
    }
    let pool = HelperPool::new(
        helper.clone(),
        Arc::new(SystemClock),
        app.cfg.files.helpers as usize,
        HELPER_IDLE,
    );
    app.files.install(helper, pool);
}

/// Ticks the pool so idle helpers close even when nobody browses.
pub fn spawn_reaper(app: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(HELPER_IDLE / 3);
        loop {
            tick.tick().await;
            if let Some(pool) = app.files.pool().cloned() {
                let _ = tokio::task::spawn_blocking(move || pool.reap()).await;
            }
        }
    })
}

// ---------------------------------------------------------------------------
// The gate (R12)
// ---------------------------------------------------------------------------

/// Where a command acts: the mount it names and the path in it.
#[derive(Debug, Clone, Copy)]
pub struct At<'a> {
    pub mount: &'a Mount,
    pub path: &'a RelPath,
}

/// Lets a file command through, or answers why not, in this order: the feature mode, the
/// person's role, the 052 network gate, then the mount's state, whether it may be browsed and,
/// for a write, whether any service mounts it read-write. The refusal is boxed: an `Event` is
/// large.
///
/// Every refusal is audited once: `files.refused` with the reason, except the network gate's,
/// which writes its own `network_refused` entry.
pub fn gate(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    kind: CommandKind,
    cmd: &str,
    at: Option<At<'_>>,
) -> Result<(), Box<Event>> {
    match (app.files.mode(), kind) {
        (FilesMode::Off, _) => {
            return Err(Box::new(deny(
                app,
                who,
                stack,
                cmd,
                at,
                DenyReason::FeatureDisabled,
            )));
        }
        (FilesMode::ReadOnly, CommandKind::FilesWrite) => {
            return Err(Box::new(refuse(
                app,
                who,
                stack,
                cmd,
                at,
                FileRefusal::FeatureReadOnly,
            )));
        }
        _ => {}
    }
    let action = kind.files_action().unwrap_or(role::Action::Edit);
    if !role::permits(&who.access, Some(stack), action) {
        return Err(Box::new(deny(
            app,
            who,
            stack,
            cmd,
            at,
            DenyReason::NotPermitted,
        )));
    }
    if app
        .trust
        .gate(&who.user, &who.actor, &who.session, who.ip, kind, cmd)
        .is_err()
    {
        return Err(Box::new(Event::Denied {
            reason: DenyReason::UntrustedNetwork,
            detail: None,
        }));
    }
    match at {
        Some(at) => gate_mount(app, who, stack, kind, cmd, at),
        None => Ok(()),
    }
}

/// The mount half of [`gate`], for a command whose mount is looked up after the person passed:
/// its state, whether it may be browsed and, for a write, whether any service mounts it
/// read-write.
pub fn gate_mount(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    kind: CommandKind,
    cmd: &str,
    at: At<'_>,
) -> Result<(), Box<Event>> {
    let m = at.mount;
    let refusal = match m.state {
        MountState::NotCreated => Some(FileRefusal::NotCreated),
        MountState::Missing => Some(FileRefusal::Missing),
        MountState::Unresolved => Some(FileRefusal::NotBrowsable {
            reason: NotBrowsable {
                code: NotBrowsableCode::Other,
                message: "its source could not be worked out from the compose file".into(),
            },
        }),
        MountState::Ready => match &m.browsable.reason {
            Some(reason) if !m.browsable.ok => Some(FileRefusal::NotBrowsable {
                reason: reason.clone(),
            }),
            _ if kind == CommandKind::FilesWrite && !m.writable => Some(FileRefusal::ReadOnlyMount),
            _ => None,
        },
    };
    match refusal {
        Some(r) => Err(Box::new(refuse(app, who, stack, cmd, Some(at), r))),
        None => Ok(()),
    }
}

/// The refusal for an error the helper answered.
pub fn refusal_from_helper(e: HelperError) -> FileRefusal {
    match e {
        HelperError::OutsideMount => FileRefusal::OutsideMount,
        HelperError::WriteBlocked { reason } => FileRefusal::WriteBlocked { reason },
        HelperError::ChangedOnDisk { current } => FileRefusal::ChangedOnDisk { current },
        HelperError::OwnerChanged => FileRefusal::OwnerChanged,
        HelperError::TooLarge => FileRefusal::TooLarge { bytes: None },
        HelperError::NotText => FileRefusal::NotText,
        HelperError::Exists => FileRefusal::Exists,
        HelperError::NotFound => FileRefusal::NotFound,
        HelperError::InvalidName => FileRefusal::InvalidName,
        HelperError::MixedOwners => FileRefusal::MixedOwners,
        HelperError::Partial {
            removed,
            failed_at,
            errno,
            message,
        } => FileRefusal::Partial {
            removed,
            failed_at,
            errno,
            message,
        },
        HelperError::OsError { errno, message } => FileRefusal::OsError { errno, message },
        HelperError::Unsupported => FileRefusal::HelperUnavailable {
            reason: "this volume helper does not support that operation".into(),
        },
        HelperError::Protocol { message } => FileRefusal::HelperUnavailable { reason: message },
    }
}

/// Audits `refusal` as `files.refused` and answers it.
pub fn refuse(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    cmd: &str,
    at: Option<At<'_>>,
    refusal: FileRefusal,
) -> Event {
    record(app, who, stack, cmd, at, refusal_code(&refusal));
    Event::FileRefused(refusal)
}

fn deny(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    cmd: &str,
    at: Option<At<'_>>,
    reason: DenyReason,
) -> Event {
    record(app, who, stack, cmd, at, wire_code(&reason));
    Event::Denied {
        reason,
        detail: None,
    }
}

fn record(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    cmd: &str,
    at: Option<At<'_>>,
    why: String,
) {
    let place = at.map(|at| place(at.mount, at.path));
    let _ = app.audit.record(&audit::file_refused(
        who.actor.clone(),
        who.ip,
        stack.as_str(),
        cmd,
        place,
        why,
    ));
}

/// What an audit entry names of where a file command acted.
pub fn place(m: &Mount, path: &RelPath) -> AuditFile {
    AuditFile {
        service: m.services.first().map(|s| s.service.clone()),
        mount: m.id.clone(),
        source: m.source.clone(),
        path: path.clone(),
        bytes: None,
        op: None,
    }
}

/// A refusal's wire `kind` (`changed_on_disk`, ...), as the audit records it.
pub fn refusal_code(r: &FileRefusal) -> String {
    serde_json::to_value(r)
        .ok()
        .and_then(|v| v.get("kind")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn wire_code(v: &impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Mount discovery (R10)
// ---------------------------------------------------------------------------

/// One service's view of its mounts: from its container's inspect when it has one.
#[derive(Debug, Clone)]
pub struct Seen {
    pub service: String,
    pub running: bool,
    pub mounts: Vec<security::Mount>,
}

/// What is at a mount's source on the host, as far as the server can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Dir,
    File,
    Missing,
    Unknown,
}

/// A stack's mounts as the UI lists them, and where each one is on the host. The sources stay
/// on the server: the client names a mount by its id only.
#[derive(Debug, Clone, Default)]
pub struct Discovered {
    pub mounts: Vec<Mount>,
    pub sources: HashMap<MountId, HelperSource>,
}

impl Discovered {
    pub fn get(&self, id: &MountId) -> Option<(&Mount, &HelperSource)> {
        let m = self.mounts.iter().find(|m| &m.id == id)?;
        Some((m, self.sources.get(id)?))
    }
}

/// One mount of one service, before mounts shared by services are merged.
struct Raw {
    kind: MountKind,
    source: String,
    service: MountService,
    inspected: bool,
    running: bool,
}

/// Raw mounts with the same kind and source, one entry per service.
struct Merged {
    kind: MountKind,
    source: String,
    services: Vec<MountService>,
    inspected: bool,
    running: Vec<String>,
}

/// Lists a stack's mounts (pure: `probe` is the only look at the host). Services with a
/// container are read from its inspect; the rest from `compose config`, so a stack that was
/// never deployed still lists its volumes, as `not_created`. Anonymous volumes have no name to
/// find again and are left out.
pub fn discover(
    stack: &str,
    stack_dir: &Path,
    seen: &[Seen],
    compose: Option<&serde_json::Value>,
    probe: &dyn Fn(&HelperSource) -> Found,
) -> Discovered {
    let mut raws: Vec<Raw> = Vec::new();
    for s in seen {
        for m in &s.mounts {
            let kind = MountKind::from_inspect(&m.r#type);
            let source = match kind {
                MountKind::Volume => m.name.clone().unwrap_or_else(|| m.source.clone()),
                MountKind::Tmpfs => m.destination.clone(),
                _ => m.source.clone(),
            };
            raws.push(Raw {
                kind,
                source,
                service: MountService {
                    service: s.service.clone(),
                    destination: m.destination.clone(),
                    read_only: !m.rw,
                },
                inspected: true,
                running: s.running,
            });
        }
    }
    if let Some(c) = compose {
        raws.extend(from_compose(stack, c, seen));
    }

    let mut out = Discovered::default();
    let mut merged: Vec<Merged> = Vec::new();
    for r in raws {
        let at = match merged
            .iter()
            .position(|m| m.kind == r.kind && m.source == r.source)
        {
            Some(i) => i,
            None => {
                merged.push(Merged {
                    kind: r.kind,
                    source: r.source.clone(),
                    services: Vec::new(),
                    inspected: false,
                    running: Vec::new(),
                });
                merged.len() - 1
            }
        };
        let m = &mut merged[at];
        m.inspected |= r.inspected;
        if r.running && !m.running.contains(&r.service.service) {
            m.running.push(r.service.service.clone());
        }
        if !m.services.iter().any(|s| s.service == r.service.service) {
            m.services.push(r.service);
        }
    }

    for Merged {
        kind,
        source,
        services,
        inspected,
        running,
    } in merged
    {
        let writable = depths_volumes::mount::any_writable(&services);
        let (state, helper) = match kind {
            MountKind::Volume => {
                let src = HelperSource::Volume(source.clone());
                let state = match (probe(&src), inspected) {
                    (Found::Missing, _) => {
                        if inspected {
                            MountState::Missing
                        } else {
                            MountState::NotCreated
                        }
                    }
                    (Found::Unknown, false) => MountState::NotCreated,
                    _ => MountState::Ready,
                };
                (state, Some(src))
            }
            MountKind::Bind if source.contains('$') || source.starts_with('~') => {
                (MountState::Unresolved, None)
            }
            MountKind::Bind => {
                let path: PathBuf = stack_dir
                    .join(&source)
                    .components()
                    .filter(|c| *c != Component::CurDir)
                    .collect();
                let src = HelperSource::Bind(path);
                let state = match probe(&src) {
                    Found::Missing => MountState::Missing,
                    _ => MountState::Ready,
                };
                (state, Some(src))
            }
            _ => (MountState::Ready, None),
        };
        let source = match &helper {
            Some(HelperSource::Bind(p)) => p.to_string_lossy().into_owned(),
            _ => source,
        };
        let rating = match kind {
            MountKind::Bind => match security::bind_reason(&source, writable) {
                None => BindRating::None,
                Some(r) if r.severity == Severity::Info => BindRating::Info,
                Some(r) => BindRating::Risky(r.message),
            },
            _ => BindRating::None,
        };
        let mut browsable = browsability(kind, &rating);
        if browsable.ok
            && kind == MountKind::Bind
            && helper.as_ref().is_some_and(|h| probe(h) == Found::File)
        {
            browsable = Browsable::no(
                NotBrowsableCode::SingleFile,
                "this mount is a single file; open it from the folder that holds it",
            );
        }
        let mount = Mount::new(kind, source, services, state, browsable, running);
        if let Some(h) = helper {
            out.sources.insert(mount.id.clone(), h);
        }
        out.mounts.push(mount);
    }
    out
}

/// The mounts of services that have no container, from `compose config --format json`.
fn from_compose(stack: &str, c: &serde_json::Value, seen: &[Seen]) -> Vec<Raw> {
    let project = c.get("name").and_then(|v| v.as_str()).unwrap_or(stack);
    let Some(services) = c.get("services").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (service, def) in services {
        if seen.iter().any(|s| &s.service == service) {
            continue;
        }
        let vols = def.get("volumes").and_then(|v| v.as_array());
        for v in vols.into_iter().flatten() {
            let field = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default();
            let (target, source) = (field("target"), field("source"));
            let (kind, source) = match field("type") {
                "volume" if source.is_empty() => continue,
                "volume" => {
                    let name = c
                        .pointer(&format!(
                            "/volumes/{}/name",
                            source.replace('~', "~0").replace('/', "~1")
                        ))
                        .and_then(|n| n.as_str())
                        .map_or_else(|| format!("{project}_{source}"), str::to_owned);
                    (MountKind::Volume, name)
                }
                "bind" => (MountKind::Bind, source.to_owned()),
                "tmpfs" => (MountKind::Tmpfs, target.to_owned()),
                _ => (MountKind::Other, source.to_owned()),
            };
            out.push(Raw {
                kind,
                source,
                service: MountService {
                    service: service.clone(),
                    destination: target.to_owned(),
                    read_only: v
                        .get("read_only")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false),
                },
                inspected: false,
                running: false,
            });
        }
    }
    out
}

/// Discovers `stack`'s mounts now (blocking: run it with `spawn_blocking`) and remembers them
/// for the commands that name a mount by id.
pub fn mounts(app: &AppState, stack: &StackName) -> Result<Discovered, RuntimeError> {
    let snap = app.runtime.containers_by_project()?;
    let containers = snap
        .projects
        .get(stack.as_str())
        .map(|p| p.containers.clone())
        .unwrap_or_default();
    let ids: Vec<String> = containers.iter().map(|c| c.id.clone()).collect();
    let inspected = if ids.is_empty() {
        Vec::new()
    } else {
        app.runtime.inspect_security(&ids)?
    };
    let seen: Vec<Seen> = containers
        .iter()
        .filter_map(|c| {
            let service = c.service.clone()?;
            let i = inspected.iter().find(|i| {
                !i.id.is_empty() && (i.id.starts_with(&c.id) || c.id.starts_with(&i.id))
            })?;
            Some(Seen {
                service,
                running: c.state == RuntimeContainerState::Running,
                mounts: i.mounts.clone(),
            })
        })
        .collect();

    let ctx = app
        .store
        .entry(stack)
        .ok()
        .filter(|e| e.actionable())
        .map(|e| e.has_env);
    let compose = ctx.and_then(|has_env| {
        let ctx = StackContext {
            stacks_root: app.store.root(),
            stack,
            global_env: app.store.global_env(),
            has_env,
        };
        let out = app
            .runtime
            .compose_config(&plan::compose_config(&ctx))
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&out).ok()
    });
    let stack_dir = app.store.root().join(stack.as_str());
    let helper = app.files.helper().cloned();
    let probe = |src: &HelperSource| probe(helper.as_deref(), src);
    let found = discover(stack.as_str(), &stack_dir, &seen, compose.as_ref(), &probe);
    app.files.remember(stack, found.clone());
    Ok(found)
}

/// Whether a source is still there: `docker volume inspect` for a volume, a stat for a bind.
pub fn probe(helper: Option<&dyn VolumeHelper>, src: &HelperSource) -> Found {
    match src {
        HelperSource::Volume(name) => match helper.map(|h| h.volume_exists(name)) {
            Some(Ok(true)) => Found::Dir,
            Some(Ok(false)) => Found::Missing,
            _ => Found::Unknown,
        },
        HelperSource::Bind(path) => match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => Found::Dir,
            Ok(_) => Found::File,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Found::Missing,
            Err(_) => Found::Unknown,
        },
    }
}

// ---------------------------------------------------------------------------
// Browsing (US1)
// ---------------------------------------------------------------------------

/// The uid:gid a mount root is stat-ed as (R5): a stat of `/volume` needs no permission.
pub const NOBODY: (u32, u32) = (65534, 65534);

/// A file command's answers, or the refusal it ended with (already audited).
pub type Answer<T> = Result<T, Box<Event>>;

/// The mount a command names, as discovered.
struct Target<'a> {
    stack: &'a StackName,
    mount: Mount,
    source: HelperSource,
}

impl Target<'_> {
    fn at<'p>(&'p self, path: &'p RelPath) -> At<'p> {
        At {
            mount: &self.mount,
            path,
        }
    }
}

/// `ListMounts`: gate, then discover. Blocking.
pub fn list_mounts(app: &AppState, who: &Who, stack: &StackName) -> Answer<Vec<Mount>> {
    gate(app, who, stack, CommandKind::FilesList, "ListMounts", None)?;
    match mounts(app, stack) {
        Ok(found) => Ok(found.mounts),
        Err(e) => {
            tracing::warn!(error = %e, "mount discovery failed");
            Err(Box::new(refuse(
                app,
                who,
                stack,
                "ListMounts",
                None,
                FileRefusal::HelperUnavailable {
                    reason: "the stack's containers could not be read".into(),
                },
            )))
        }
    }
}

/// One page of a listing, as the helper sent it.
pub struct Page {
    pub entries: Vec<depths_protocol::Entry>,
    pub page: u32,
    pub last: bool,
}

/// `ListDir`: every page of `path`, run as the folder's owner. Blocking.
pub fn list_dir(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: &MountId,
    path: &RelPath,
) -> Answer<Vec<Page>> {
    const CMD: &str = "ListDir";
    let t = target(app, who, stack, mount, path, CommandKind::FilesList, CMD)?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CMD, Some(t.at(path)), r));
    let owner = owner_of(app, &t, path).map_err(fail)?;
    let pages = on_helper(app, &t, owner, |chan| {
        if let Err(e) = recheck(chan, path, owner)? {
            return Ok(Err(e));
        }
        let mut pages = Vec::new();
        for n in 0.. {
            let op = Op::List {
                path: path.clone(),
                page: n,
            };
            match ask(chan, &op)? {
                Ok(Reply::Page {
                    entries,
                    page,
                    last,
                }) => {
                    pages.push(Page {
                        entries,
                        page,
                        last,
                    });
                    if last {
                        break;
                    }
                }
                Ok(_) => return Err(unexpected("List")),
                Err(e) => return Ok(Err(e)),
            }
        }
        Ok(Ok(pages))
    });
    let pages = settle(app, &t, path, pages).map_err(fail)?;
    app.files.note_owners(
        stack,
        mount,
        pages.iter().flat_map(|p| &p.entries).filter_map(|e| {
            let child = e.name_b64.is_none().then(|| path.join(&e.name).ok())??;
            Some((child, (e.uid, e.gid)))
        }),
    );
    Ok(pages)
}

/// `ReadFile`: the file decoded for the editor, run as its owner; audited `files.read`.
/// Blocking.
pub fn read_file(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: &MountId,
    path: &RelPath,
) -> Answer<FileContent> {
    const CMD: &str = "ReadFile";
    let t = target(app, who, stack, mount, path, CommandKind::FilesRead, CMD)?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CMD, Some(t.at(path)), r));
    let owner = owner_of(app, &t, path).map_err(fail)?;
    let max = app.cfg.files.editor_max;
    let read = on_helper(app, &t, owner, |chan| {
        let op = Op::Read {
            path: path.clone(),
            raw_name: None,
            max,
        };
        match ask(chan, &op)? {
            Ok(Reply::Content {
                fingerprint,
                owner,
                write_blocked,
                data,
            }) => {
                let bytes = if data { Some(drain(chan, max)?) } else { None };
                Ok(Ok((fingerprint, owner, write_blocked, bytes)))
            }
            Ok(_) => Err(unexpected("Read")),
            Err(e) => Ok(Err(e)),
        }
    });
    let (fingerprint, found, blocked, bytes) = settle(app, &t, path, read).map_err(fail)?;
    if (found.uid, found.gid) != owner {
        app.files.forget_owner(stack, mount, path);
        return Err(fail(FileRefusal::OwnerChanged));
    }
    let read_only = blocked.or((!t.mount.writable).then_some(WriteBlocked::ReadOnlyMount));
    let file = FileContent::build(fingerprint, found, read_only, bytes.as_deref(), max);
    let mut place = place(&t.mount, path);
    place.bytes = Some(file.size);
    let _ = app.audit.record(&audit::file(
        who.actor.clone(),
        who.ip,
        AuditAction::FilesRead,
        stack.as_str(),
        place,
        AuditOutcome::Succeeded,
    ));
    Ok(file)
}

/// A download on a helper of its own: the size and name, and the helper with the file's bytes
/// waiting as Data… End.
pub struct Download {
    pub chan: OneShot,
    pub size: u64,
    pub name: String,
}

/// Why a download did not start, for the HTTP status.
pub enum NoDownload {
    /// Refused and audited, as a WS command would be.
    Refused(Box<Event>),
    /// A folder: downloads of folders arrive with uploads (T050).
    Folder,
}

/// Opens `path` for download, as its owner, on a one-shot helper; audited `files.download` with
/// the size. Blocking.
pub fn open_download(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: &MountId,
    path: &RelPath,
) -> Result<Download, NoDownload> {
    const CMD: &str = "download";
    let t = target(app, who, stack, mount, path, CommandKind::FilesRead, CMD)
        .map_err(NoDownload::Refused)?;
    let fail = |r: FileRefusal| {
        NoDownload::Refused(Box::new(refuse(app, who, stack, CMD, Some(t.at(path)), r)))
    };
    let owner = owner_of(app, &t, path).map_err(fail)?;
    let Some(pool) = app.files.pool() else {
        return Err(fail(unavailable(app)));
    };
    let mut chan = pool
        .one_shot(&spec(&t, owner, OpMode::ReadOnly))
        .map_err(|e| fail(start_failed(app, &t, e)))?;
    app.files.check_hello(chan.hello()).map_err(fail)?;
    let max = app.cfg.files.download_max;
    let opened = (|| {
        let info = match recheck(&mut chan, path, owner)? {
            Ok(info) => info,
            Err(e) => return Ok(Err(e)),
        };
        if info.kind == EntryKind::Dir {
            return Ok(Ok(None));
        }
        let op = Op::Read {
            path: path.clone(),
            raw_name: None,
            max,
        };
        match ask(&mut chan, &op)? {
            Ok(Reply::Content {
                fingerprint, data, ..
            }) => Ok(Ok(Some((fingerprint.size(), data)))),
            Ok(_) => Err(unexpected("Read")),
            Err(e) => Ok(Err(e)),
        }
    })();
    let (size, data) = match opened {
        Ok(Ok(Some(opened))) => opened,
        Ok(Ok(None)) => {
            chan.close();
            return Err(NoDownload::Folder);
        }
        Ok(Err(e)) => {
            chan.close();
            return Err(fail(helper_refusal(app, &t, path, e)));
        }
        Err(e) => {
            chan.kill();
            tracing::warn!(error = %e, "volume helper failed");
            return Err(fail(FileRefusal::HelperUnavailable {
                reason: "the volume helper failed".into(),
            }));
        }
    };
    if !data {
        chan.close();
        return Err(fail(FileRefusal::TooLarge { bytes: Some(size) }));
    }
    let mut place = place(&t.mount, path);
    place.bytes = Some(size);
    let _ = app.audit.record(&audit::file(
        who.actor.clone(),
        who.ip,
        AuditAction::FilesDownload,
        stack.as_str(),
        place,
        AuditOutcome::Succeeded,
    ));
    Ok(Download {
        chan,
        size,
        name: path.file_name().unwrap_or("download").to_owned(),
    })
}

// ---------------------------------------------------------------------------
// History (R9)
// ---------------------------------------------------------------------------

/// The file a kept version belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileKey {
    pub stack: StackName,
    pub mount: MountId,
    pub path: RelPath,
}

/// Where content a change replaced goes (R9): what a save, an upload or a restore overwrote, and
/// a deleted file. Content over `DEPTHS_FILES_HISTORY_FILE_MAX` never reaches it.
pub trait HistorySink: Send + Sync {
    /// Keeps `bytes` as `file`'s newest version. Best effort: a failure is logged, never fatal.
    /// Blocking.
    fn keep(&self, file: &FileKey, author: &str, reason: VersionReason, bytes: &[u8]);
    /// `file`'s kept versions, newest first.
    fn versions(&self, _file: &FileKey) -> Vec<FileVersion> {
        Vec::new()
    }
    /// The content of version `id` of `file`.
    fn version(&self, _file: &FileKey, _id: &VersionId) -> Option<Vec<u8>> {
        None
    }
}

/// History that keeps nothing.
pub struct NoHistory;

impl HistorySink for NoHistory {
    fn keep(&self, _: &FileKey, _: &str, _: VersionReason, _: &[u8]) {}
}

/// The most replaced content a helper sends back after a write (its `PRIOR_MAX`).
const PRIOR_MAX: u64 = 10 * 1024 * 1024;

/// Hands content `t`'s `path` lost to history, unless there is none or it is too large to keep.
fn keep(
    app: &AppState,
    who: &Who,
    t: &Target<'_>,
    path: &RelPath,
    reason: VersionReason,
    bytes: Option<Vec<u8>>,
) {
    let Some(bytes) = bytes else { return };
    if bytes.len() as u64 > app.cfg.files.history_file_max {
        return;
    }
    let file = FileKey {
        stack: t.stack.clone(),
        mount: t.mount.id.clone(),
        path: path.clone(),
    };
    app.files.history().keep(&file, &who.user, reason, &bytes);
}

// ---------------------------------------------------------------------------
// Previews and confirms (US2 to US5)
// ---------------------------------------------------------------------------

/// How long a file preview may be confirmed for, as for stack operations.
pub const PREVIEW_TTL: Duration = crate::ops::PREVIEW_TTL;
/// Most unconfirmed file previews held at once.
const MAX_PREVIEWS: usize = 10_000;

const PREVIEW: &str = "PreviewFileOp";
const CONFIRM: &str = "Confirm";

/// A file preview waiting for `Confirm`.
struct Pending {
    session: String,
    stack: StackName,
    change: Change,
    expires: Instant,
    used: bool,
}

/// What a confirmed preview does.
#[derive(Clone)]
enum Change {
    Write(Write),
    /// An empty file or folder `name` in `parent`.
    Create {
        mount: MountId,
        parent: RelPath,
        name: Name,
        kind: CreateKind,
    },
    Rename {
        mount: MountId,
        from: RelPath,
        to: RelPath,
    },
    /// `path`, or `raw_name` in folder `path`; `folder` goes with everything in it. `size` is a
    /// file's size at the preview, for the audit.
    Delete {
        mount: MountId,
        path: RelPath,
        raw_name: Option<RawName>,
        folder: bool,
        size: Option<u64>,
    },
}

impl Change {
    /// An upload stays after its confirm: the POSTs that follow carry its id.
    fn is_upload(&self) -> bool {
        false
    }
}

/// `Save` and `Restore`: new content for `path`, over `base` (`None`: the file is gone, so it
/// is made again).
#[derive(Clone)]
struct Write {
    mount: MountId,
    path: RelPath,
    bytes: Vec<u8>,
    base: Option<Fingerprint>,
    reason: VersionReason,
}

impl Files {
    fn bind(
        &self,
        who: &Who,
        stack: &StackName,
        change: Change,
    ) -> Result<(PreviewId, String), FileRefusal> {
        let ttl = *self.ttl.read().unwrap();
        let now = Instant::now();
        let mut previews = self.previews.lock().unwrap();
        previews.retain(|_, p| p.expires > now && (!p.used || p.change.is_upload()));
        if previews.len() >= MAX_PREVIEWS {
            return Err(FileRefusal::HelperUnavailable {
                reason: "too many unconfirmed previews; try again in a few minutes".into(),
            });
        }
        let id = PreviewId(uuid::Uuid::new_v4());
        previews.insert(
            id,
            Pending {
                session: who.session.clone(),
                stack: stack.clone(),
                change,
                expires: now + ttl,
                used: false,
            },
        );
        let at = (chrono::Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        Ok((id, at))
    }

    /// `session`'s preview `id`, claimed when `claim`. The checks run in the order stack
    /// operations use: unknown (or another session's), used, expired.
    fn take(
        &self,
        session: &str,
        id: PreviewId,
        claim: bool,
    ) -> Result<(StackName, Change), PreviewVoidReason> {
        let mut previews = self.previews.lock().unwrap();
        let Some(p) = previews.get_mut(&id).filter(|p| p.session == session) else {
            return Err(PreviewVoidReason::Unknown);
        };
        if p.used {
            return Err(PreviewVoidReason::Used);
        }
        if p.expires <= Instant::now() {
            previews.remove(&id);
            return Err(PreviewVoidReason::Expired);
        }
        p.used |= claim;
        Ok((p.stack.clone(), p.change.clone()))
    }
}

/// `PreviewFileOp`: what the change would do, checked as far as it can be without writing.
/// Nothing is written and no read-write helper starts. Blocking.
pub fn preview(app: &AppState, who: &Who, stack: &StackName, op: FileOp) -> Answer<FilePreview> {
    match op {
        FileOp::Save {
            mount,
            path,
            text,
            base,
            bom,
            eol,
        } => {
            let bytes = text::encode(&text, bom, eol);
            preview_save(app, who, stack, &mount, &path, bytes, base)
        }
        FileOp::Create {
            mount,
            parent,
            name,
            kind,
        } => preview_create(app, who, stack, mount, parent, name, kind),
        FileOp::Rename { mount, from, to } => preview_rename(app, who, stack, mount, from, to),
        FileOp::Delete {
            mount,
            path,
            raw_name,
        } => preview_delete(app, who, stack, mount, path, raw_name),
        _ => {
            gate(app, who, stack, CommandKind::FilesWrite, PREVIEW, None)?;
            Err(Box::new(refuse(
                app,
                who,
                stack,
                PREVIEW,
                None,
                FileRefusal::HelperUnavailable {
                    reason: "this file change is not available yet".into(),
                },
            )))
        }
    }
}

fn preview_save(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: &MountId,
    path: &RelPath,
    bytes: Vec<u8>,
    base: Fingerprint,
) -> Answer<FilePreview> {
    let t = target(
        app,
        who,
        stack,
        mount,
        path,
        CommandKind::FilesWrite,
        PREVIEW,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, PREVIEW, Some(t.at(path)), r));
    let max = app.cfg.files.editor_max;
    if bytes.len() as u64 > max {
        return Err(fail(FileRefusal::TooLarge {
            bytes: Some(bytes.len() as u64),
        }));
    }
    let owner = owner_of(app, &t, path).map_err(fail)?;
    let (current, old) = read_bytes(app, &t, path, owner, max).map_err(fail)?;
    if current != base {
        return Err(fail(FileRefusal::ChangedOnDisk { current }));
    }
    let Some(old) = old.filter(|old| text::decode(old, max).is_ok()) else {
        return Err(fail(FileRefusal::NotText));
    };
    let diff = crate::diff::unified(
        path.as_str(),
        Some(&String::from_utf8_lossy(&old)),
        Some(&String::from_utf8_lossy(&bytes)),
    );
    let summary = format!("Save {path} ({} bytes)", bytes.len());
    let change = Change::Write(Write {
        mount: mount.clone(),
        path: path.clone(),
        bytes,
        base: Some(current),
        reason: VersionReason::Save,
    });
    let mut p = bound(app, who, &t, change, summary).map_err(fail)?;
    p.diff = Some(diff);
    Ok(p)
}

/// Holds `change` for `Confirm` and describes it.
fn bound(
    app: &AppState,
    who: &Who,
    t: &Target<'_>,
    change: Change,
    summary: String,
) -> Result<FilePreview, FileRefusal> {
    let (preview_id, expires_at) = app.files.bind(who, t.stack, change)?;
    Ok(FilePreview {
        preview_id,
        stack: t.stack.as_str().to_owned(),
        mount: t.mount.id.clone(),
        summary,
        diff: None,
        entries: None,
        entries_capped: false,
        uploads: Vec::new(),
        running: t.mount.running.clone(),
        expires_at,
    })
}

/// `path`'s fingerprint and, when it is at most `max` bytes, its content.
fn read_bytes(
    app: &AppState,
    t: &Target<'_>,
    path: &RelPath,
    owner: (u32, u32),
    max: u64,
) -> Result<(Fingerprint, Option<Vec<u8>>), FileRefusal> {
    let read = on_helper(app, t, owner, |chan| {
        let op = Op::Read {
            path: path.clone(),
            raw_name: None,
            max,
        };
        match ask(chan, &op)? {
            Ok(Reply::Content {
                fingerprint, data, ..
            }) => {
                let bytes = if data { Some(drain(chan, max)?) } else { None };
                Ok(Ok((fingerprint, bytes)))
            }
            Ok(_) => Err(unexpected("Read")),
            Err(e) => Ok(Err(e)),
        }
    });
    settle(app, t, path, read)
}

/// `Confirm` of a file preview: the person passes the gate again (the network may have changed),
/// then the change is applied once. Blocking.
pub fn confirm(app: &AppState, who: &Who, id: PreviewId) -> Answer<Event> {
    let void = |reason| Box::new(Event::PreviewVoid { reason });
    let (stack, _) = app.files.take(&who.session, id, false).map_err(void)?;
    gate(app, who, &stack, CommandKind::FilesWrite, CONFIRM, None)?;
    let (stack, change) = app.files.take(&who.session, id, true).map_err(void)?;
    match change {
        Change::Write(w) => write(app, who, &stack, id, w),
        Change::Create {
            mount,
            parent,
            name,
            kind,
        } => create(app, who, &stack, id, &mount, &parent, name, kind),
        Change::Rename { mount, from, to } => rename(app, who, &stack, id, &mount, &from, &to),
        Change::Delete {
            mount,
            path,
            raw_name,
            folder,
            size,
        } => delete(app, who, &stack, id, &mount, &path, raw_name, folder, size),
    }
}

/// Writes a save's or a restore's content as the file's owner (the folder's, for a file that is
/// gone), on a read-write helper of its own; audited `files.save` or `files.restore`.
fn write(app: &AppState, who: &Who, stack: &StackName, id: PreviewId, w: Write) -> Answer<Event> {
    let path = &w.path;
    let t = target(
        app,
        who,
        stack,
        &w.mount,
        path,
        CommandKind::FilesWrite,
        CONFIRM,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CONFIRM, Some(t.at(path)), r));
    let (op, owner) = match &w.base {
        Some(base) => (
            Op::Write {
                path: path.clone(),
                base: *base,
                mode: None,
            },
            owner_of(app, &t, path).map_err(fail)?,
        ),
        None => {
            let (dir, name) = split(path).map_err(fail)?;
            let owner = owner_of(app, &t, &dir).map_err(fail)?;
            (
                Op::Put {
                    dir,
                    name,
                    replace: None,
                },
                owner,
            )
        }
    };
    let written = once(app, &t, owner, |chan| send_data(chan, &op, &w.bytes));
    let (fingerprint, prior) = settle(app, &t, path, written).map_err(fail)?;
    let action = match w.reason {
        VersionReason::Restore => AuditAction::FilesRestore,
        _ => AuditAction::FilesSave,
    };
    audit_done(app, who, &t, path, action, Some(w.bytes.len() as u64));
    keep(app, who, &t, path, w.reason, prior);
    Ok(done(id, &t, w.path.clone(), Some(fingerprint)))
}

fn done(id: PreviewId, t: &Target<'_>, path: RelPath, fingerprint: Option<Fingerprint>) -> Event {
    Event::FileOpDone {
        preview_id: id,
        mount: t.mount.id.clone(),
        path,
        fingerprint,
        running: t.mount.running.clone(),
    }
}

/// A change that went through, audited with its size when it has one.
fn audit_done(
    app: &AppState,
    who: &Who,
    t: &Target<'_>,
    path: &RelPath,
    action: AuditAction,
    bytes: Option<u64>,
) {
    let mut place = place(&t.mount, path);
    place.bytes = bytes;
    let _ = app.audit.record(&audit::file(
        who.actor.clone(),
        who.ip,
        action,
        t.stack.as_str(),
        place,
        AuditOutcome::Succeeded,
    ));
}

/// The most entries a folder delete's preview counts before it says "10,000+".
const DELETE_COUNT_CAP: u64 = 10_000;

fn preview_create(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: MountId,
    parent: RelPath,
    name: Name,
    kind: CreateKind,
) -> Answer<FilePreview> {
    let t = target(
        app,
        who,
        stack,
        &mount,
        &parent,
        CommandKind::FilesWrite,
        PREVIEW,
    )?;
    let path = joined(app, who, &t, &parent, &name, PREVIEW)?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, PREVIEW, Some(t.at(&path)), r));
    let owner = owner_of(app, &t, &parent).map_err(fail)?;
    if look(app, &t, &path, None, owner).map_err(fail)?.is_some() {
        return Err(fail(FileRefusal::Exists));
    }
    let what = match kind {
        CreateKind::File => "file",
        CreateKind::Dir => "folder",
    };
    let summary = format!("Create {what} {path}");
    let change = Change::Create {
        mount,
        parent,
        name,
        kind,
    };
    bound(app, who, &t, change, summary).map_err(fail)
}

fn preview_rename(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: MountId,
    from: RelPath,
    to: RelPath,
) -> Answer<FilePreview> {
    let t = target(
        app,
        who,
        stack,
        &mount,
        &from,
        CommandKind::FilesWrite,
        PREVIEW,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, PREVIEW, Some(t.at(&from)), r));
    let owner = rename_owner(app, &t, &from, &to).map_err(fail)?;
    if look(app, &t, &from, None, owner).map_err(fail)?.is_none() {
        return Err(fail(FileRefusal::NotFound));
    }
    if look(app, &t, &to, None, owner).map_err(fail)?.is_some() {
        return Err(fail(FileRefusal::Exists));
    }
    let summary = format!("Rename {from} → {to}");
    let change = Change::Rename {
        mount,
        from: from.clone(),
        to,
    };
    bound(app, who, &t, change, summary).map_err(fail)
}

fn preview_delete(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    mount: MountId,
    path: RelPath,
    raw_name: Option<RawName>,
) -> Answer<FilePreview> {
    let t = target(
        app,
        who,
        stack,
        &mount,
        &path,
        CommandKind::FilesWrite,
        PREVIEW,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, PREVIEW, Some(t.at(&path)), r));
    let holder = holder(&path, raw_name.as_ref()).map_err(fail)?;
    let owner = owner_of(app, &t, &holder).map_err(fail)?;
    let Some(info) = look(app, &t, &path, raw_name.as_ref(), owner).map_err(fail)? else {
        return Err(fail(FileRefusal::NotFound));
    };
    let shown = shown(&path, raw_name.as_ref());
    let folder = info.kind == EntryKind::Dir;
    let (summary, entries, capped) = if folder {
        let walker = (info.owner.uid, info.owner.gid);
        let (n, capped) = count(app, &t, &path, raw_name.as_ref(), walker).map_err(fail)?;
        let n = n.min(DELETE_COUNT_CAP);
        let plus = if capped { "+" } else { "" };
        let summary = format!(
            "Delete folder {shown} and the {}{plus} entries in it",
            grouped(n)
        );
        (summary, Some(n), capped)
    } else {
        (format!("Delete {shown} ({} bytes)", info.size), None, false)
    };
    let change = Change::Delete {
        mount,
        path: path.clone(),
        raw_name,
        folder,
        size: (!folder).then_some(info.size),
    };
    let mut p = bound(app, who, &t, change, summary).map_err(fail)?;
    p.entries = entries;
    p.entries_capped = capped;
    Ok(p)
}

/// `name` in `parent`, or `invalid_name` (audited) when they do not make a path.
fn joined(
    app: &AppState,
    who: &Who,
    t: &Target<'_>,
    parent: &RelPath,
    name: &Name,
    cmd: &str,
) -> Answer<RelPath> {
    parent.join(name.as_str()).map_err(|_| {
        Box::new(refuse(
            app,
            who,
            t.stack,
            cmd,
            Some(t.at(parent)),
            FileRefusal::InvalidName,
        ))
    })
}

/// The folder an entry to delete sits in: `path` itself when the entry is `raw` in it.
fn holder(path: &RelPath, raw: Option<&RawName>) -> Result<RelPath, FileRefusal> {
    match raw {
        Some(_) => Ok(path.clone()),
        None => path.parent().ok_or(FileRefusal::InvalidName),
    }
}

/// How a path reads in a preview, with a raw last name shown lossily.
fn shown(path: &RelPath, raw: Option<&RawName>) -> String {
    match raw {
        Some(raw) if path.is_root() => raw.lossy(),
        Some(raw) => format!("{path}/{}", raw.lossy()),
        None => path.to_string(),
    }
}

/// `n` with thousands separators: 10,000.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

/// The uid:gid a rename runs as: both folders' owner, or `mixed_owners` when they differ (no
/// single uid could move it without privileges).
fn rename_owner(
    app: &AppState,
    t: &Target<'_>,
    from: &RelPath,
    to: &RelPath,
) -> Result<(u32, u32), FileRefusal> {
    let (Some(from_dir), Some(to_dir)) = (from.parent(), to.parent()) else {
        return Err(FileRefusal::InvalidName);
    };
    let owner = owner_of(app, t, &from_dir)?;
    if owner_of(app, t, &to_dir)? != owner {
        return Err(FileRefusal::MixedOwners);
    }
    Ok(owner)
}

/// `path` (or `raw` in it) as `owner` sees it; `None` when it is not there.
fn look(
    app: &AppState,
    t: &Target<'_>,
    path: &RelPath,
    raw: Option<&RawName>,
    owner: (u32, u32),
) -> Result<Option<StatInfo>, FileRefusal> {
    let asked = on_helper(app, t, owner, |chan| {
        let op = Op::Stat {
            path: path.clone(),
            raw_name: raw.cloned(),
            hash: false,
        };
        match ask(chan, &op)? {
            Ok(Reply::Stat(info)) => Ok(Ok(Some(info))),
            Ok(_) => Err(unexpected("Stat")),
            Err(HelperError::NotFound) => Ok(Ok(None)),
            Err(e) => Ok(Err(e)),
        }
    });
    settle(app, t, path, asked)
}

/// The entries beneath a folder, walked as `owner` up to [`DELETE_COUNT_CAP`], and whether the
/// walk stopped there.
fn count(
    app: &AppState,
    t: &Target<'_>,
    path: &RelPath,
    raw: Option<&RawName>,
    owner: (u32, u32),
) -> Result<(u64, bool), FileRefusal> {
    let asked = on_helper(app, t, owner, |chan| {
        let op = Op::Size {
            path: path.clone(),
            raw_name: raw.cloned(),
            cap: SizeCap {
                bytes: u64::MAX,
                entries: DELETE_COUNT_CAP,
            },
        };
        match ask(chan, &op)? {
            Ok(Reply::Size {
                entries, capped, ..
            }) => Ok(Ok((entries, capped))),
            Ok(_) => Err(unexpected("Size")),
            Err(e) => Ok(Err(e)),
        }
    });
    settle(app, t, path, asked)
}

/// Sends `op` and expects the bare reply `ok` accepts.
fn plain(
    chan: &mut HelperChannel,
    op: &Op,
    ok: fn(&Reply) -> bool,
) -> Result<Result<(), HelperError>, ChannelError> {
    match ask(chan, op)? {
        Ok(r) if ok(&r) => Ok(Ok(())),
        Ok(_) => Err(unexpected("change")),
        Err(e) => Ok(Err(e)),
    }
}

/// Makes the empty file or folder as its folder's owner; audited `files.create`.
#[allow(clippy::too_many_arguments)]
fn create(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    id: PreviewId,
    mount: &MountId,
    parent: &RelPath,
    name: Name,
    kind: CreateKind,
) -> Answer<Event> {
    let t = target(
        app,
        who,
        stack,
        mount,
        parent,
        CommandKind::FilesWrite,
        CONFIRM,
    )?;
    let path = joined(app, who, &t, parent, &name, CONFIRM)?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CONFIRM, Some(t.at(&path)), r));
    let owner = owner_of(app, &t, parent).map_err(fail)?;
    let op = Op::Create {
        parent: parent.clone(),
        name,
        kind: match kind {
            CreateKind::File => OpCreateKind::File,
            CreateKind::Dir => OpCreateKind::Dir,
        },
    };
    let made = once(app, &t, owner, |chan| {
        plain(chan, &op, |r| matches!(r, Reply::Created))
    });
    settle(app, &t, &path, made).map_err(fail)?;
    app.files.note_owners(stack, mount, [(path.clone(), owner)]);
    audit_done(app, who, &t, &path, AuditAction::FilesCreate, None);
    Ok(done(id, &t, path, None))
}

/// Moves `from` to `to` as both folders' owner; audited `files.rename`.
fn rename(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    id: PreviewId,
    mount: &MountId,
    from: &RelPath,
    to: &RelPath,
) -> Answer<Event> {
    let t = target(
        app,
        who,
        stack,
        mount,
        from,
        CommandKind::FilesWrite,
        CONFIRM,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CONFIRM, Some(t.at(from)), r));
    let owner = rename_owner(app, &t, from, to).map_err(fail)?;
    let op = Op::Rename {
        from: from.clone(),
        to: to.clone(),
    };
    let moved = once(app, &t, owner, |chan| {
        plain(chan, &op, |r| matches!(r, Reply::Renamed))
    });
    settle(app, &t, from, moved).map_err(fail)?;
    app.files.forget_owner(stack, mount, from);
    audit_done(app, who, &t, from, AuditAction::FilesRename, None);
    Ok(done(id, &t, to.clone(), None))
}

/// Deletes the entry as its folder's owner; audited `files.delete`, a deleted file's content
/// kept in history. A folder delete that stops part way is audited as failed with what it
/// removed, and answered `partial` verbatim.
#[allow(clippy::too_many_arguments)]
fn delete(
    app: &AppState,
    who: &Who,
    stack: &StackName,
    id: PreviewId,
    mount: &MountId,
    path: &RelPath,
    raw_name: Option<RawName>,
    folder: bool,
    size: Option<u64>,
) -> Answer<Event> {
    let t = target(
        app,
        who,
        stack,
        mount,
        path,
        CommandKind::FilesWrite,
        CONFIRM,
    )?;
    let fail = |r: FileRefusal| Box::new(refuse(app, who, stack, CONFIRM, Some(t.at(path)), r));
    let holder = holder(path, raw_name.as_ref()).map_err(fail)?;
    let owner = owner_of(app, &t, &holder).map_err(fail)?;
    let raw = raw_name.is_some();
    let op = Op::Delete {
        path: path.clone(),
        raw_name,
        recursive: folder,
    };
    let gone = once(app, &t, owner, |chan| match ask(chan, &op)? {
        Ok(Reply::Deleted { prior, .. }) => Ok(Ok(if prior {
            Some(drain(chan, PRIOR_MAX)?)
        } else {
            None
        })),
        Ok(_) => Err(unexpected("Delete")),
        Err(e) => Ok(Err(e)),
    });
    let prior = match gone {
        Ok(Err(HelperError::Partial {
            removed,
            failed_at,
            errno,
            message,
        })) => {
            app.files.forget_owner(stack, mount, path);
            let mut place = place(&t.mount, path);
            place.bytes = size;
            let _ = app.audit.record(&audit::file(
                who.actor.clone(),
                who.ip,
                AuditAction::FilesDelete,
                stack.as_str(),
                place,
                AuditOutcome::Failed {
                    exit_code: None,
                    stderr_tail: format!(
                        "removed {removed} entries, then {failed_at}: {errno}: {message}"
                    ),
                },
            ));
            return Err(Box::new(Event::FileRefused(FileRefusal::Partial {
                removed,
                failed_at,
                errno,
                message,
            })));
        }
        gone => settle(app, &t, path, gone).map_err(fail)?,
    };
    if !raw {
        app.files.forget_owner(stack, mount, path);
    }
    audit_done(app, who, &t, path, AuditAction::FilesDelete, size);
    if !raw {
        keep(app, who, &t, path, VersionReason::Delete, prior);
    }
    Ok(done(id, &t, path.clone(), None))
}

/// A path as its folder and its name; the mount root has no name.
fn split(path: &RelPath) -> Result<(RelPath, Name), FileRefusal> {
    let dir = path.parent().ok_or(FileRefusal::InvalidName)?;
    let name = path
        .file_name()
        .and_then(|n| Name::new(n).ok())
        .ok_or(FileRefusal::InvalidName)?;
    Ok((dir, name))
}

/// Runs `op` on a read-write one-shot helper for `t` as `owner`, after checking its `Hello`.
/// Every write runs this way, so no read-write helper outlives its change.
fn once<T>(
    app: &AppState,
    t: &Target<'_>,
    owner: (u32, u32),
    op: impl FnOnce(&mut HelperChannel) -> Result<Result<T, HelperError>, ChannelError>,
) -> Asked<T> {
    let Some(pool) = app.files.pool() else {
        return Err(unavailable(app));
    };
    let mut chan = pool
        .one_shot(&spec(t, owner, OpMode::ReadWrite))
        .map_err(|e| start_failed(app, t, e))?;
    app.files.check_hello(chan.hello())?;
    match op(&mut chan) {
        Ok(answer) => {
            chan.close();
            Ok(answer)
        }
        Err(e) => {
            chan.kill();
            tracing::warn!(error = %e, "volume helper failed");
            Err(FileRefusal::HelperUnavailable {
                reason: "the volume helper failed".into(),
            })
        }
    }
}

/// A `Written` answer: the new fingerprint and the replaced content, if the helper sent it.
type Wrote = Result<Result<(Fingerprint, Option<Vec<u8>>), HelperError>, ChannelError>;

/// The most bytes in one Data frame the server sends.
const DATA_CHUNK: usize = 256 * 1024;

/// Sends `op` and `bytes` as Data… End, then reads what [`written`] reads.
fn send_data(chan: &mut HelperChannel, op: &Op, bytes: &[u8]) -> Wrote {
    chan.send(&Frame::json(FrameKind::Op, op)?)?;
    for chunk in bytes.chunks(DATA_CHUNK) {
        chan.send(&Frame::data(chunk.to_vec()))?;
    }
    chan.send(&Frame::end())?;
    written(chan)
}

/// A `Written` reply, and the content it replaced when the helper sent it.
fn written(chan: &mut HelperChannel) -> Wrote {
    let frame = chan.recv()?;
    if frame.kind == FrameKind::Error {
        return Ok(Err(frame.parse(FrameKind::Error)?));
    }
    match frame.parse(FrameKind::Reply)? {
        Reply::Written { fingerprint, prior } => {
            let prior = if prior {
                Some(drain(chan, PRIOR_MAX)?)
            } else {
                None
            };
            Ok(Ok((fingerprint, prior)))
        }
        _ => Err(unexpected("Write")),
    }
}

/// Passes the person, finds the mount (discovering the stack's mounts when it is not known
/// yet), then passes the mount.
fn target<'a>(
    app: &AppState,
    who: &Who,
    stack: &'a StackName,
    mount: &MountId,
    path: &RelPath,
    kind: CommandKind,
    cmd: &str,
) -> Answer<Target<'a>> {
    gate(app, who, stack, kind, cmd, None)?;
    let found = app.files.known(stack, mount).or_else(|| {
        mounts(app, stack).ok()?;
        app.files.known(stack, mount)
    });
    let Some((m, source)) = found else {
        return Err(Box::new(refuse(
            app,
            who,
            stack,
            cmd,
            None,
            FileRefusal::Missing,
        )));
    };
    let t = Target {
        stack,
        mount: m,
        source,
    };
    gate_mount(app, who, stack, kind, cmd, t.at(path))?;
    Ok(t)
}

/// The uid:gid to act on `path` as: from the listing that led here, else a stat as its parent's
/// owner, and the mount root stat-ed as [`NOBODY`].
fn owner_of(app: &AppState, t: &Target<'_>, path: &RelPath) -> Result<(u32, u32), FileRefusal> {
    if let Some(owner) = app.files.owner(t.stack, &t.mount.id, path) {
        return Ok(owner);
    }
    let asker = match path.parent() {
        Some(parent) => owner_of(app, t, &parent)?,
        None => NOBODY,
    };
    let stat = on_helper(app, t, asker, |chan| stat(chan, path));
    let info = settle(app, t, path, stat)?;
    let owner = (info.owner.uid, info.owner.gid);
    app.files
        .note_owners(t.stack, &t.mount.id, [(path.clone(), owner)]);
    Ok(owner)
}

/// What the helper answered, or why it could not answer.
type Asked<T> = Result<Result<T, HelperError>, FileRefusal>;

fn spec(t: &Target<'_>, (uid, gid): (u32, u32), mode: OpMode) -> HelperSpec {
    HelperSpec {
        stack: t.stack.clone(),
        source: t.source.clone(),
        uid,
        gid,
        mode,
    }
}

/// Runs `op` on the pooled helper for `t` as `owner`, after checking its `Hello`.
fn on_helper<T>(
    app: &AppState,
    t: &Target<'_>,
    owner: (u32, u32),
    op: impl FnOnce(&mut HelperChannel) -> Result<Result<T, HelperError>, ChannelError>,
) -> Asked<T> {
    let Some(pool) = app.files.pool() else {
        return Err(unavailable(app));
    };
    let mut hello = Ok(());
    let ran = pool.run(&spec(t, owner, OpMode::ReadOnly), |chan| {
        hello = app.files.check_hello(chan.hello());
        match hello {
            Ok(()) => op(chan).map(Some),
            Err(_) => Ok(None),
        }
    });
    match ran {
        Ok(Some(answer)) => Ok(answer),
        Ok(None) => Err(hello.err().unwrap_or_else(|| unavailable(app))),
        Err(PoolError::Start(e)) => Err(start_failed(app, t, PoolError::Start(e))),
        Err(PoolError::Channel(e)) => {
            tracing::warn!(error = %e, "volume helper failed");
            Err(FileRefusal::HelperUnavailable {
                reason: "the volume helper failed".into(),
            })
        }
    }
}

/// A helper error as the refusal to answer with; an owner that moved is forgotten.
fn settle<T>(
    app: &AppState,
    t: &Target<'_>,
    path: &RelPath,
    asked: Asked<T>,
) -> Result<T, FileRefusal> {
    match asked? {
        Ok(v) => Ok(v),
        Err(e) => Err(helper_refusal(app, t, path, e)),
    }
}

fn helper_refusal(app: &AppState, t: &Target<'_>, path: &RelPath, e: HelperError) -> FileRefusal {
    if e == HelperError::OwnerChanged {
        app.files.forget_owner(t.stack, &t.mount.id, path);
    }
    refusal_from_helper(e)
}

fn unavailable(app: &AppState) -> FileRefusal {
    FileRefusal::HelperUnavailable {
        reason: app.files.reason().unwrap_or_else(|| NO_IMAGE.into()),
    }
}

/// A helper that did not start: its source is gone, or the helper cannot run.
fn start_failed(app: &AppState, t: &Target<'_>, e: PoolError) -> FileRefusal {
    if probe(app.files.helper().map(|h| h.as_ref()), &t.source) == Found::Missing {
        return FileRefusal::Missing;
    }
    tracing::warn!(error = %e, "volume helper did not start");
    FileRefusal::HelperUnavailable {
        reason: "the volume helper did not start".into(),
    }
}

/// Sends `op` and reads its answer: a Reply, or the helper's Error.
fn ask(chan: &mut HelperChannel, op: &Op) -> Result<Result<Reply, HelperError>, ChannelError> {
    chan.send(&Frame::json(FrameKind::Op, op)?)?;
    let frame = chan.recv()?;
    if frame.kind == FrameKind::Error {
        return Ok(Err(frame.parse(FrameKind::Error)?));
    }
    Ok(Ok(frame.parse(FrameKind::Reply)?))
}

fn stat(
    chan: &mut HelperChannel,
    path: &RelPath,
) -> Result<Result<StatInfo, HelperError>, ChannelError> {
    let op = Op::Stat {
        path: path.clone(),
        raw_name: None,
        hash: false,
    };
    match ask(chan, &op)? {
        Ok(Reply::Stat(info)) => Ok(Ok(info)),
        Ok(_) => Err(unexpected("Stat")),
        Err(e) => Ok(Err(e)),
    }
}

/// Re-checks with a stat that `path` is still owned by `owner` (R5).
fn recheck(
    chan: &mut HelperChannel,
    path: &RelPath,
    owner: (u32, u32),
) -> Result<Result<StatInfo, HelperError>, ChannelError> {
    Ok(stat(chan, path)?.and_then(|info| {
        if (info.owner.uid, info.owner.gid) == owner {
            Ok(info)
        } else {
            Err(HelperError::OwnerChanged)
        }
    }))
}

/// The Data… End after a `Content` reply, at most `max` bytes.
pub fn drain(chan: &mut HelperChannel, max: u64) -> Result<Vec<u8>, ChannelError> {
    let mut bytes = Vec::new();
    loop {
        let frame = chan.recv()?;
        match frame.kind {
            FrameKind::Data if (bytes.len() + frame.body.len()) as u64 <= max => {
                bytes.extend_from_slice(&frame.body);
            }
            FrameKind::End => return Ok(bytes),
            _ => return Err(unexpected("Data")),
        }
    }
}

fn unexpected(what: &str) -> ChannelError {
    ChannelError::Protocol(FrameError::Io(std::io::Error::other(format!(
        "unexpected answer to {what}"
    ))))
}
