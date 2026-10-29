//! The update commands (spec 061 contracts/protocol.md): `GetKeelBuild`, `GetUpdateStatus`,
//! `CheckForUpdates`, `DismissUpdateDot` and `PreviewKeelUpdate`, plus the role-shaped push the
//! socket sends on every status change (R5).
//!
//! Every command is for sessions only (`dispatch::authorize`). Replies are shaped by role:
//! owners see everything, admins the running build and a one-line status, users the version.

use keel_core::InstanceRole;
use keel_protocol::{
    AvailableImage, Cmd, Event, KeelBuild, UpdateError, UpdateOutcome, UpdateStatus,
    UpdateStatusLine,
};
use serde_json::{Value, json};
use time::Duration;

use crate::audit::{self, Decision, Entry};
use crate::build_info::{self, BuildInfo};
use crate::dispatch::Request;
use crate::governance::{deny, internal, origin, pending, preview, ts};
use crate::identity::{self, Caller};
use crate::preview::Draft;
use crate::runs::{self, UpdateAudit};
use crate::serve::AppState;
use crate::system::{self, UpdateTarget};

/// The preview's action and the confirm's audit action.
const ACTION: &str = "keel_update";
/// The command name, for `Denied`.
const CMD: &str = "preview_keel_update";

/// Whether `cmd` is one of this module's commands.
#[must_use]
pub fn is_update_cmd(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::GetKeelBuild
            | Cmd::GetUpdateStatus
            | Cmd::CheckForUpdates
            | Cmd::DismissUpdateDot { .. }
            | Cmd::PreviewKeelUpdate { .. }
    )
}

/// Routes one update command.
pub(crate) async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::GetKeelBuild => vec![Event::KeelBuild(keel_build(
            &app.updates.build,
            req.caller.user.instance_role,
        ))],
        Cmd::GetUpdateStatus => get_status(app, req).await,
        Cmd::CheckForUpdates => check_now(app, req).await,
        Cmd::DismissUpdateDot { digest } => dismiss(app, req, digest).await,
        Cmd::PreviewKeelUpdate { digest } => preview_update(app, req, digest).await,
        _ => vec![internal()],
    }
}

fn is_owner(c: &Caller) -> bool {
    c.token.is_none() && c.user.instance_role == InstanceRole::Owner
}

fn is_admin(c: &Caller) -> bool {
    c.token.is_none()
        && matches!(
            c.user.instance_role,
            InstanceRole::Owner | InstanceRole::Admin
        )
}

// ---- the running build (US1) -----------------------------------------------------------------

/// The running build for `role`: revision, build date and digest for owners and admins only
/// (FR-002).
#[must_use]
pub fn keel_build(build: &BuildInfo, role: InstanceRole) -> KeelBuild {
    let full = matches!(role, InstanceRole::Owner | InstanceRole::Admin);
    KeelBuild {
        version: build.version.clone(),
        revision: build.revision.clone().filter(|_| full),
        built_at: build.built_at.filter(|_| full).map(ts),
        digest: build.digest.clone().filter(|_| full),
        display: build.display(),
    }
}

// ---- the status (US2, US3) -------------------------------------------------------------------

/// The full status, for owners.
async fn full_status(app: &AppState) -> Result<UpdateStatus, sqlx::Error> {
    let s = app.updates.status();
    let available = s.available.as_ref().map(|a| AvailableImage {
        digest: a.digest.to_string(),
        version: a.version.clone(),
        revision: a.revision.clone(),
        built_at: a.built_at.map(ts),
        release_notes: a.release_notes.as_ref().map(ToString::to_string),
        release_notes_note: a.release_notes_note.clone(),
    });
    let digest = available
        .as_ref()
        .map(|a| a.digest.clone())
        .or_else(|| app.updates.build.digest.clone());
    let mut conn = app.db.acquire().await?;
    let update_path = system::update_path(&mut conn, &app.updates.config.image, digest.as_deref()).await?;
    let update_run = system::update_run(&mut conn).await?;
    Ok(UpdateStatus {
        enabled: s.enabled,
        image: s.image,
        state: s.state,
        outcome: s.outcome,
        cant_tell_reason: s.cant_tell_reason,
        checked_at: s.checked_at.map(ts),
        next_check_at: s.next_check_at.map(ts),
        manual_after: s.manual_after.map(ts),
        last_error: s.last_error.map(|e| UpdateError {
            code: e.code.to_owned(),
            at: ts(e.at),
        }),
        running: keel_build(&app.updates.build, InstanceRole::Owner),
        available,
        digest_compare: s.digest_compare,
        update_path,
        update_run,
    })
}

/// The one-line status, for admins (FR-015, SC-006): nothing about the available image.
fn status_line(app: &AppState) -> UpdateStatusLine {
    let s = app.updates.status();
    UpdateStatusLine {
        update_available: s.outcome == Some(UpdateOutcome::UpdateAvailable),
        checked_at: s.checked_at.map(ts),
    }
}

async fn full_reply(app: &AppState) -> Vec<Event> {
    match full_status(app).await {
        Ok(s) => vec![Event::UpdateStatus(s)],
        Err(e) => {
            tracing::error!(error = %e, "reading the update status failed");
            vec![internal()]
        }
    }
}

/// What a status change pushes to `caller` (R5): `UpdateStatus` to owners, `UpdateStatusLine`
/// to admins, nothing to users.
pub(crate) async fn pushed(app: &AppState, caller: &Caller) -> Option<Event> {
    if is_owner(caller) {
        match full_status(app).await {
            Ok(s) => Some(Event::UpdateStatus(s)),
            Err(e) => {
                tracing::error!(error = %e, "reading the update status failed");
                None
            }
        }
    } else if is_admin(caller) {
        Some(Event::UpdateStatusLine(status_line(app)))
    } else {
        None
    }
}

async fn get_status(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    if is_owner(req.caller) {
        full_reply(app).await
    } else if is_admin(req.caller) {
        vec![Event::UpdateStatusLine(status_line(app))]
    } else {
        let o = origin(req);
        deny(app, req.caller, "get_update_status", None, "admin_only", o.context).await
    }
}

/// `CheckForUpdates`: replies `checking` at once and runs the check in the background; its
/// result reaches the sockets through the status channel.
async fn check_now(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    if !is_owner(req.caller) {
        let o = origin(req);
        return deny(app, req.caller, "check_for_updates", None, "owner_only", o.context).await;
    }
    if !app.updates.config.check {
        return vec![Event::op_error(
            "update_check_off",
            "Update checks are off (KEEL_UPDATE_CHECK=false).",
        )];
    }
    if let Some(t) = app.updates.manual_retry_at() {
        return vec![Event::op_error("update_check_too_soon", ts(t))];
    }
    let updates = app.updates.clone();
    tokio::spawn(async move {
        updates.check_now(true).await;
    });
    // The spawned check may not have marked itself yet: the reply says `checking` either way,
    // and the result follows on the socket.
    match full_status(app).await {
        Ok(mut s) => {
            s.state = keel_protocol::UpdateCheckState::Checking;
            vec![Event::UpdateStatus(s)]
        }
        Err(e) => {
            tracing::error!(error = %e, "reading the update status failed");
            vec![internal()]
        }
    }
}

/// `DismissUpdateDot`: remembers the digest the owner dismissed the dot for (R6).
async fn dismiss(app: &AppState, req: &Request<'_>, digest: String) -> Vec<Event> {
    if !is_owner(req.caller) {
        let o = origin(req);
        return deny(app, req.caller, "dismiss_update_dot", None, "owner_only", o.context).await;
    }
    if !build_info::is_digest(&digest) {
        return vec![Event::op_error(
            "invalid_digest",
            "A digest is sha256: and 64 lowercase hex characters.",
        )];
    }
    let r = sqlx::query("update users set update_dot_dismissed = $2 where id = $1")
        .bind(req.caller.user.id)
        .bind(&digest)
        .execute(&app.db)
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, "storing the dismissed update failed");
        return vec![internal()];
    }
    let window =
        Duration::try_from(app.cfg.step_up_window).unwrap_or(Duration::minutes(5));
    match identity::me(&app.db, req.caller, window, app.clock.now()).await {
        Ok(mut me) => {
            me.keel_version = Some(app.updates.build.display());
            vec![Event::Me(me)]
        }
        Err(e) => {
            tracing::error!(error = %e, "GetMe failed");
            vec![internal()]
        }
    }
}

// ---- the update run (US4) --------------------------------------------------------------------

/// Audits a refusal of `PreviewKeelUpdate` or its confirm as `keel_update_refused`.
async fn audit_refusal(
    app: &AppState,
    caller: &Caller,
    context: &Value,
    reason: &str,
    new_digest: &str,
) {
    let mut context = context.clone();
    context["old_digest"] = json!(app.updates.build.digest);
    context["new_digest"] = json!(new_digest);
    let actor = caller.actor();
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &actor,
                action: "keel_update_refused",
                workspace: None,
                target: json!({}),
                decision: Decision::Denied,
                reason: Some(reason),
                context,
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
}

/// Dispatch refused `PreviewKeelUpdate` from an API token: audit it as an update refusal too
/// (FR-020).
pub(crate) async fn refused_by_dispatch(app: &AppState, req: &Request<'_>, digest: &str) {
    let o = origin(req);
    audit_refusal(app, req.caller, &o.context, "token_caller", digest).await;
}

/// The reply for a refusal `code`.
fn refusal(code: &str) -> Vec<Event> {
    let message = match code {
        "owner_only" => {
            return vec![Event::Denied {
                action: CMD.to_owned(),
                reason: code.to_owned(),
            }];
        }
        "no_system_workspace" => "There is no System workspace, so Keel can't update itself.",
        "no_keel_host" => "No enrolled host in the System workspace has the keel-server profile.",
        "profile_not_allowed" => "The System workspace doesn't allow the keel-server profile.",
        "job_missing_param" => {
            "The System workspace's keel-server job doesn't declare \
             params.keel_server_image_digest, or can't run."
        }
        "digest_stale" => "The update status changed. Reload it and try again.",
        _ => "Keel could not complete that. Try again.",
    };
    vec![Event::op_error(code, message)]
}

/// Whether `digest` is still the update on offer.
fn stale(app: &AppState, digest: &str) -> bool {
    let s = app.updates.status();
    let movable = matches!(
        s.outcome,
        Some(UpdateOutcome::UpdateAvailable | UpdateOutcome::Rebuilt | UpdateOutcome::OlderBuild)
    );
    !movable || s.available.as_ref().is_none_or(|a| a.digest.as_str() != digest)
}

/// The target, or the first refusal (audited).
async fn checked_target(
    app: &AppState,
    caller: &Caller,
    context: &Value,
    digest: &str,
) -> Result<UpdateTarget, Vec<Event>> {
    if !is_owner(caller) {
        audit_refusal(app, caller, context, "owner_only", digest).await;
        return Err(refusal("owner_only"));
    }
    let target = async {
        let mut conn = app.db.acquire().await?;
        system::update_target(&mut conn).await
    }
    .await;
    let target = match target {
        Ok(Ok(t)) => t,
        Ok(Err(code)) => {
            audit_refusal(app, caller, context, code, digest).await;
            return Err(refusal(code));
        }
        Err(e) => {
            tracing::error!(error = %e, "reading the update prerequisites failed");
            return Err(vec![internal()]);
        }
    };
    if stale(app, digest) {
        audit_refusal(app, caller, context, "digest_stale", digest).await;
        return Err(refusal("digest_stale"));
    }
    Ok(target)
}

fn or_unknown(v: Option<&str>) -> &str {
    v.unwrap_or("unknown")
}

async fn preview_update(app: &AppState, req: &Request<'_>, digest: String) -> Vec<Event> {
    let o = origin(req);
    let target = match checked_target(app, req.caller, &o.context, &digest).await {
        Ok(t) => t,
        Err(events) => return events,
    };
    let planned = async {
        let mut conn = app.db.acquire().await?;
        let plan =
            runs::plan_summary(&mut conn, target.workspace, &target.job_name, &target.def).await?;
        let existing = system::update_run(&mut conn).await?;
        let slug: Option<String> = sqlx::query_scalar("select slug from workspaces where id = $1")
            .bind(target.workspace)
            .fetch_optional(&mut *conn)
            .await?;
        Ok::<_, sqlx::Error>((plan, existing, slug))
    }
    .await;
    let (plan, existing, slug) = match planned {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "planning the update run failed");
            return vec![internal()];
        }
    };
    let Some((hosts, policy)) = plan else {
        // The job's inventory, limit or profiles reach no enrolled host.
        audit_refusal(app, req.caller, &o.context, "no_keel_host", &digest).await;
        return refusal("no_keel_host");
    };
    let status = app.updates.status();
    let build = &app.updates.build;
    let available = status.available.clone();
    let mut lines = Vec::new();
    match status.outcome {
        Some(UpdateOutcome::OlderBuild) => lines.push(
            "Warning: the watched tag points at an older build than the one running.".to_owned(),
        ),
        Some(UpdateOutcome::Rebuilt) => lines.push(
            "Warning: this is the same version, rebuilt; the image differs from the running one."
                .to_owned(),
        ),
        _ => {}
    }
    let (new_version, new_revision) = available
        .as_ref()
        .map_or((None, None), |a| (a.version.clone(), a.revision.clone()));
    lines.push(format!(
        "Update Keel from {} (revision {}, {}) to {} (revision {}, {}).",
        or_unknown(build.version.as_deref()),
        or_unknown(build.revision.as_deref()),
        or_unknown(build.digest.as_deref()),
        or_unknown(new_version.as_deref()),
        or_unknown(new_revision.as_deref()),
        digest,
    ));
    lines.push(format!(
        "Runs job {} in workspace {} on {}. Keel restarts when the job replaces it.",
        target.job_name,
        slug.as_deref().unwrap_or(system::SLUG),
        hosts.join(", "),
    ));
    lines.push(format!("Approval: {policy}."));
    if let Some(run) = existing {
        lines.push(format!(
            "An update run is already in progress ({run}); confirming opens it instead."
        ));
    }
    // One paragraph: the confirm dialog doesn't render newlines, and every line is a sentence.
    let summary = lines.join(" ");
    let ctx = o.context.clone();
    let op = pending(move |app, caller| async move {
        let target = match checked_target(&app, &caller, &ctx, &digest).await {
            Ok(t) => t,
            Err(events) => return events,
        };
        let audit = UpdateAudit {
            old_digest: app.updates.build.digest.clone(),
            new_digest: digest,
            old_revision: app.updates.build.revision.clone(),
            new_revision,
            context: ctx,
        };
        let run = match runs::create_update_run(&app, &caller, &target, &audit).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "creating the update run failed");
                return vec![internal()];
            }
        };
        match runs::get_run(&app, run).await {
            Ok(Some(e)) => vec![e],
            Ok(None) => vec![internal()],
            Err(e) => {
                tracing::error!(error = %e, "reading the update run failed");
                vec![internal()]
            }
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: hosts,
            step_up: true,
        },
        op,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oci_registry::Platform;

    fn build() -> BuildInfo {
        BuildInfo {
            version: Some("0.2.0".into()),
            revision: Some("abcdef1".into()),
            built_at: Some(time::OffsetDateTime::UNIX_EPOCH),
            source: None,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            platform: Platform::current(),
        }
    }

    #[test]
    fn users_get_the_version_only() {
        let b = keel_build(&build(), InstanceRole::User);
        assert_eq!(b.version.as_deref(), Some("0.2.0"));
        assert_eq!(b.display, "0.2.0");
        assert_eq!((b.revision, b.built_at, b.digest), (None, None, None));
        for role in [InstanceRole::Owner, InstanceRole::Admin] {
            let b = keel_build(&build(), role);
            assert!(b.revision.is_some() && b.built_at.is_some() && b.digest.is_some());
        }
    }
}
