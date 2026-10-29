//! What a batch preview shows and binds (contracts/protocol.md `PreviewUpdateBatch`): per stack,
//! the services with a newer image, why the others are left, the steps, and the compose lint.
//!
//! A stack's [`StackJob`] binds its content hash and the digests shown, so the run installs what
//! was previewed or skips the stack as stale. Blocking: it reads the stack store and runs
//! `compose config` through [`Inputs`].

use depths_core::StackName;
use depths_core::compose_lint::ComposePolicy;
use depths_core::plan::{self, StackContext};
use depths_core::ports::StackStore;
use depths_core::updates::hold::{self, LintDecision, Trigger};
use depths_core::updates::keep::{self, KeepKind};
use depths_core::updates::reference::ImageRef;
use depths_protocol::{ServiceUpdate, StackUpdatePlan, UpdateState};

use super::Updates;
use super::inputs::{Inputs, ServiceInput};
use super::run::{ServicePin, StackJob, image_pins};
use super::view;

/// One stack's preview and what its confirm runs. `job` is `None` when nothing would change.
#[derive(Debug, Clone)]
pub struct StackPreview {
    pub plan: StackUpdatePlan,
    pub job: Option<StackJob>,
    /// An escape-level compose finding (054's step-up rule for `Update`).
    pub escape: bool,
}

/// Which services of a stack an update recreates.
pub enum Only<'a> {
    /// Every service with an available update.
    Available,
    /// A scheduled run: the services whose effective policy is Auto (labels can lower it).
    Auto,
    /// `ApplyHeld`: this held service at this digest.
    Held { service: &'a str, digest: &'a str },
}

/// The preview of `stack`; `Err` when the stack can't be acted on.
pub fn preview_stack(
    u: &Updates,
    store: &dyn StackStore,
    inputs: &dyn Inputs,
    policy: ComposePolicy,
    stack: &str,
    only: Only<'_>,
) -> Result<StackPreview, String> {
    let name = StackName::parse(stack).map_err(|e| format!("{stack}: {e}"))?;
    let entry = store.entry(&name).map_err(|e| format!("{stack}: {e}"))?;
    if !entry.actionable() {
        return Err(format!("{stack}: the stack cannot be acted on"));
    }
    let hash = entry
        .content_hash
        .as_ref()
        .map(|h| h.as_str().to_owned())
        .ok_or_else(|| format!("{stack}: the compose file can't be read"))?;
    let compose = store.read(&name).map(|f| f.compose).unwrap_or_default();
    let rejected = u
        .store()
        .decisions()
        .map(|d| d.rejected)
        .unwrap_or_default();
    let live = inputs.services(stack);

    let mut shown = Vec::new();
    let mut pins = Vec::new();
    let mut skipped = Vec::new();
    for s in view::stack_services(u, stack) {
        let wanted = match &only {
            Only::Available | Only::Auto => true,
            Only::Held { service, .. } => s.service == *service,
        };
        if !wanted {
            continue;
        }
        match pin(
            &s,
            &only,
            &rejected,
            live.iter().find(|l| l.service == s.service),
        ) {
            Ok(p) => {
                pins.push(p);
                shown.push(s);
            }
            Err(why) => skipped.push((s.service.clone(), why)),
        }
    }

    let lint = hold::lint_check(
        &compose,
        policy,
        Trigger::Manual {
            acknowledged: false,
        },
    );
    let escape = !lint.escape_codes().is_empty();
    let ctx = StackContext {
        stacks_root: store.root(),
        stack: &name,
        global_env: store.global_env(),
        has_env: entry.has_env,
    };
    let (services, image) = image_pins(&pins);
    let phases = plan::image_update_phases(&ctx, &services, &image);
    let keeps = pins.iter().map(|p| {
        plan::image_tag(
            &ctx,
            &p.from_image,
            &keep::tag(stack, &p.service, KeepKind::Previous),
        )
    });
    let argv = if pins.is_empty() {
        Vec::new()
    } else {
        phases
            .pulls
            .into_iter()
            .chain(keeps)
            .chain(phases.tags)
            .chain(phases.up)
            .map(|s| s.argv)
            .collect()
    };
    let job = (!pins.is_empty()).then(|| StackJob {
        stack: stack.to_owned(),
        stack_hash: hash,
        pins,
        acknowledged: false,
        apply_held: matches!(only, Only::Held { .. }),
    });
    Ok(StackPreview {
        plan: StackUpdatePlan {
            stack: stack.to_owned(),
            services: shown,
            skipped,
            argv,
            findings: lint.findings.iter().map(crate::security::warning).collect(),
            ack_required: lint.decision == LintDecision::NeedsAck,
        },
        job,
        escape,
    })
}

/// The pin for one service, or why it is left as it is.
fn pin(
    s: &ServiceUpdate,
    only: &Only<'_>,
    rejected: &[super::store::Rejected],
    live: Option<&ServiceInput>,
) -> Result<ServicePin, String> {
    if s.effective.excluded.is_some() {
        return Err(s.reason.clone().unwrap_or_else(|| "excluded".into()));
    }
    let want = match only {
        Only::Available | Only::Auto => UpdateState::Available,
        Only::Held { .. } => UpdateState::Held,
    };
    if matches!(only, Only::Auto) && s.effective.policy != depths_protocol::UpdatePolicy::Auto {
        return Err(format!(
            "its policy is {} here, not auto",
            serde_json::to_value(s.effective.policy)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        ));
    }
    if s.state != want {
        return Err(match s.state {
            UpdateState::UpToDate => "up to date".into(),
            UpdateState::NotChecked => "not checked yet".into(),
            other => {
                let word = serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
                    .replace('_', " ");
                match &s.reason {
                    Some(r) => format!("{word}: {r}"),
                    None => word,
                }
            }
        });
    }
    let available = s.available.as_ref().ok_or("no newer image is known")?;
    if let Only::Held { digest, .. } = only
        && available.digest != *digest
    {
        return Err(format!(
            "the held image is now {}, not {digest}: preview again",
            available.digest
        ));
    }
    let running = s.running.as_ref().ok_or("no running container")?;
    if rejected
        .iter()
        .any(|r| r.stack == s.stack && r.service == s.service && r.digest == available.digest)
    {
        return Err("rejected: it failed its health check before (Retry to allow it)".into());
    }
    let reference =
        ImageRef::parse(&s.reference).map_err(|e| format!("bad image reference: {e}"))?;
    Ok(ServicePin {
        service: s.service.clone(),
        reference: s.reference.clone(),
        pull: format!("{}@{}", reference.name(), available.digest),
        digest: available.digest.clone(),
        platform: live.and_then(|l| l.platform.clone()),
        to_image: Some(available.config_digest.clone()).filter(|c| !c.is_empty()),
        from_image: running.image_id.clone(),
        from_digest: running.repo_digest.clone(),
        version: available.version.clone(),
        compose_user: live.and_then(|l| l.user.clone()),
        old_user: live
            .and_then(|l| l.running.as_ref())
            .and_then(|i| i.user.clone())
            .unwrap_or_default(),
        replicas: s.replicas.max(1),
    })
}

#[cfg(test)]
mod tests;
