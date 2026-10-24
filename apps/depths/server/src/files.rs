//! The volume explorer's server side (spec 053): the gate every file command passes (R12), the
//! refusals it answers with, mount discovery (R10) and the helper pool's state (R2).
//!
//! Paths a person browses are audited (they are the point of the audit) but never logged or put
//! on a span.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use depths_core::StackName;
use depths_core::plan::{self, StackContext};
use depths_core::ports::{ContainerRuntime, RuntimeError, VolumeHelper};
use depths_core::role::{self, CommandKind};
use depths_core::security::{self, Severity};
use depths_protocol::{
    AuditFile, Browsable, DenyReason, Event, FileRefusal, Mount, MountId, MountKind, MountService,
    MountState, NotBrowsable, NotBrowsableCode, RelPath, RuntimeContainerState,
};
use depths_runtime_cli::helper::{HELPER_IDLE, ImageId, SystemClock};
use depths_runtime_cli::{CliRuntime, CliVolumeHelper, HelperPool};
use depths_volumes::mount::{BindRating, HelperSource, browsability};
use depths_volumes::op::{Hello, HelperError};

use crate::audit;
use crate::config::FilesMode;
use crate::http::AppState;
use crate::ops::Who;

/// The explorer's run-time state: the configured mode, lowered to `off` (with a reason admins
/// see) when the helper cannot run here, and the helper pool once it is installed.
pub struct Files {
    state: RwLock<(FilesMode, Option<String>)>,
    pool: OnceLock<(Arc<dyn VolumeHelper>, HelperPool)>,
    /// Each stack's mounts as last discovered, so a command naming a mount id finds its source.
    known: Mutex<HashMap<StackName, Discovered>>,
}

impl Files {
    pub fn new(mode: FilesMode) -> Self {
        Self {
            state: RwLock::new((mode, None)),
            pool: OnceLock::new(),
            known: Mutex::new(HashMap::new()),
        }
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
}

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
    let Some(at) = at else {
        return Ok(());
    };
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
