//! The volume explorer's server side (spec 053): the gate every file command passes (R12), the
//! refusals it answers with, mount discovery (R10) and the helper pool's state (R2).
//!
//! Paths a person browses are audited (they are the point of the audit) but never logged or put
//! on a span.

use std::sync::{Arc, OnceLock, RwLock};

use depths_core::StackName;
use depths_core::ports::VolumeHelper;
use depths_core::role::{self, CommandKind};
use depths_protocol::{
    AuditFile, DenyReason, Event, FileRefusal, Mount, MountState, NotBrowsable, NotBrowsableCode,
    RelPath,
};
use depths_runtime_cli::HelperPool;
use depths_volumes::op::HelperError;

use crate::audit;
use crate::config::FilesMode;
use crate::http::AppState;
use crate::ops::Who;

/// The explorer's run-time state: the configured mode, lowered to `off` (with a reason admins
/// see) when the helper cannot run here, and the helper pool once it is installed.
pub struct Files {
    state: RwLock<(FilesMode, Option<String>)>,
    pool: OnceLock<(Arc<dyn VolumeHelper>, HelperPool)>,
}

impl Files {
    pub fn new(mode: FilesMode) -> Self {
        Self {
            state: RwLock::new((mode, None)),
            pool: OnceLock::new(),
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
