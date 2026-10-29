//! Update changes and batches waiting for their `Confirm` (`PreviewUpdateChange`,
//! `PreviewUpdateBatch`).
//!
//! A preview belongs to the session that made it, is used once, and expires after
//! [`PREVIEW_TTL_SECS`]. Preview ids are unique across kinds, so `Confirm` finds the change here
//! before it tries 050's operation previews.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use depths_protocol::{
    MaintenanceWindow, PreviewId, PreviewVoidReason, UpdatePolicy, UpdateSettings, WatchtowerAnswer,
};

use super::Updates;
use super::run::StackJob;
use super::store::{self, StoredPolicy};

pub const PREVIEW_TTL_SECS: i64 = 300;

/// What a preview holds until its `Confirm`.
#[derive(Debug, Clone)]
pub enum Held {
    Settings {
        before: UpdateSettings,
        after: UpdateSettings,
    },
    Policy {
        stack: String,
        before: Option<StoredPolicy>,
        policy: UpdatePolicy,
        window: Option<MaintenanceWindow>,
    },
    Watchtower {
        container_id: String,
        image_id: String,
        name: String,
        answer: WatchtowerAnswer,
    },
    Retry {
        stack: String,
        service: String,
        digest: String,
    },
    /// An administrator overrides a root-escalation hold: a one-stack run with `apply_held`.
    ApplyHeld { digest: String, job: StackJob },
    /// A batch: the stacks in order, each bound to its content hash and digests.
    Batch {
        jobs: Vec<StackJob>,
        /// Some stack has an escape-level compose finding (054's step-up rule for `Update`).
        escape: bool,
    },
    /// FR-026: a user's `PreviewOp { Update }` with checks on, a one-stack run.
    Manual(ManualUpdate),
}

/// FR-026: what a user's image update binds, and what its `Confirm` gates on (050's `Update`).
#[derive(Debug, Clone)]
pub struct ManualUpdate {
    pub job: StackJob,
    /// An escape-level compose finding (054's step-up rule for `Update`).
    pub escape: bool,
    /// 051: only an admin who acknowledges the findings may confirm.
    pub ack_required: bool,
}

impl Held {
    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch { .. })
    }
}

/// A change shown to an admin and not yet confirmed.
#[derive(Debug, Clone)]
pub struct PendingChange {
    pub session: String,
    pub held: Held,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct Pending(HashMap<PreviewId, PendingChange>);

/// Why a settings change is refused before it is previewed (data-model.md's rules).
pub fn check(before: &UpdateSettings, after: &UpdateSettings) -> Result<(), String> {
    if after == before {
        return Err("nothing to change".into());
    }
    if after.interval_secs < store::MIN_INTERVAL_SECS {
        return Err(format!(
            "interval_secs is {}; the minimum is {} (one hour)",
            after.interval_secs,
            store::MIN_INTERVAL_SECS
        ));
    }
    if !store::HEALTH_TIMEOUT_SECS.contains(&after.health_timeout_secs) {
        return Err(format!(
            "health_timeout_secs is {}; it must be 30 to 1800",
            after.health_timeout_secs
        ));
    }
    store::validate_window("window", &after.window)
}

/// The before/after summary a preview shows, one line per changed field.
pub fn summary(before: &UpdateSettings, after: &UpdateSettings) -> String {
    let on = |b: bool| if b { "on" } else { "off" };
    let mut lines = Vec::new();
    if before.checks_enabled != after.checks_enabled {
        lines.push(format!(
            "update checks: {} -> {}",
            on(before.checks_enabled),
            on(after.checks_enabled)
        ));
    }
    if before.interval_secs != after.interval_secs {
        lines.push(format!(
            "check interval: {} -> {}",
            interval(before.interval_secs),
            interval(after.interval_secs)
        ));
    }
    if before.window != after.window {
        lines.push(format!(
            "maintenance window: {} -> {}",
            window(&before.window),
            window(&after.window)
        ));
    }
    if before.health_timeout_secs != after.health_timeout_secs {
        lines.push(format!(
            "health timeout: {} s -> {} s",
            before.health_timeout_secs, after.health_timeout_secs
        ));
    }
    if before.cleanup != after.cleanup {
        lines.push(format!(
            "remove replaced images: {} -> {}",
            on(before.cleanup),
            on(after.cleanup)
        ));
    }
    if before.label_only != after.label_only {
        lines.push(format!(
            "only services with the Watchtower enable label: {} -> {}",
            on(before.label_only),
            on(after.label_only)
        ));
    }
    if before.scope != after.scope {
        let scope = |s: &Option<depths_protocol::UpdateScope>| {
            s.as_ref()
                .map_or_else(|| "none set".to_owned(), |s| format!("`{}`", s.as_str()))
        };
        lines.push(format!(
            "Watchtower scope: {} -> {}",
            scope(&before.scope),
            scope(&after.scope)
        ));
    }
    lines.join("\n")
}

/// `mon,tue 03:00-05:00`.
pub fn window(w: &MaintenanceWindow) -> String {
    let days: Vec<String> = w
        .days
        .iter()
        .map(|d| format!("{d:?}").to_lowercase())
        .collect();
    format!("{} {}-{}", days.join(","), w.start.as_str(), w.end.as_str())
}

fn interval(secs: u32) -> String {
    if secs.is_multiple_of(86_400) {
        format!("every {} h ({} d)", secs / 3600, secs / 86_400)
    } else if secs.is_multiple_of(3600) {
        format!("every {} h", secs / 3600)
    } else {
        format!("every {secs} s")
    }
}

impl Updates {
    /// Keeps `change` until it is confirmed or expires; returns its id.
    pub fn hold_change(&self, change: PendingChange) -> PreviewId {
        let id = PreviewId(uuid::Uuid::new_v4());
        let now = self.clock.now();
        let mut p = self.pending_lock();
        p.0.retain(|_, c| c.expires > now);
        p.0.insert(id, change);
        id
    }

    /// Whether `id` is a change this session made (and so whether `Confirm` is ours).
    pub fn has_change(&self, session: &str, id: PreviewId) -> bool {
        self.pending_lock()
            .0
            .get(&id)
            .is_some_and(|c| c.session == session)
    }

    /// Whether `id` is this session's batch (so `Confirm` gates it as `Update`), and whether a
    /// stack in it has an escape finding.
    pub fn batch_escape(&self, session: &str, id: PreviewId) -> Option<bool> {
        match self.pending_lock().0.get(&id) {
            Some(PendingChange {
                session: s,
                held: Held::Batch { escape, .. },
                ..
            }) if s == session => Some(*escape),
            _ => None,
        }
    }

    /// This session's FR-026 manual update `id`, without taking it: what its `Confirm` gates on.
    pub fn manual_update(&self, session: &str, id: PreviewId) -> Option<ManualUpdate> {
        match self.pending_lock().0.get(&id) {
            Some(PendingChange {
                session: s,
                held: Held::Manual(m),
                ..
            }) if s == session => Some(m.clone()),
            _ => None,
        }
    }

    /// Takes the change for its `Confirm`: used once.
    pub fn take_change(
        &self,
        session: &str,
        id: PreviewId,
    ) -> Result<PendingChange, PreviewVoidReason> {
        let mut p = self.pending_lock();
        match p.0.get(&id) {
            Some(c) if c.session == session => {}
            _ => return Err(PreviewVoidReason::Unknown),
        }
        let c = p.0.remove(&id).ok_or(PreviewVoidReason::Unknown)?;
        if c.expires <= self.clock.now() {
            return Err(PreviewVoidReason::Expired);
        }
        Ok(c)
    }

    fn pending_lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::store::StoredSettings;
    use depths_protocol::{TimeOfDay, UpdateScope, Weekday};

    fn base() -> UpdateSettings {
        StoredSettings::default().settings()
    }

    #[test]
    fn every_setting_may_change_within_its_bounds() {
        let b = base();
        let mut a = b.clone();
        a.checks_enabled = !b.checks_enabled;
        a.interval_secs = 7200;
        a.cleanup = true;
        a.label_only = true;
        a.health_timeout_secs = 300;
        a.scope = Some(UpdateScope::new("media").unwrap());
        assert!(check(&b, &a).is_ok());
        assert_eq!(check(&b, &b).unwrap_err(), "nothing to change");
        let mut short = b.clone();
        short.interval_secs = 60;
        assert!(check(&b, &short).unwrap_err().contains("minimum"));
        for t in [29, 1801] {
            let mut h = b.clone();
            h.health_timeout_secs = t;
            assert!(check(&b, &h).unwrap_err().contains("30 to 1800"), "{t}");
        }
        let mut w = b.clone();
        w.window.end = w.window.start.clone();
        assert!(check(&b, &w).unwrap_err().contains("end equals start"));
        let mut d = b.clone();
        d.window.days = vec![Weekday::Mon, Weekday::Mon];
        assert!(check(&b, &d).unwrap_err().contains("twice"));
    }

    #[test]
    fn summary_names_each_change() {
        let b = base();
        let mut a = b.clone();
        a.checks_enabled = !b.checks_enabled;
        a.interval_secs = 7200;
        a.cleanup = true;
        a.health_timeout_secs = 300;
        a.window.days = vec![Weekday::Sat];
        a.window.start = TimeOfDay::new("01:00").unwrap();
        let s = summary(&b, &a);
        assert!(s.contains("update checks"));
        assert!(s.contains("every 2 h"));
        assert!(s.contains("remove replaced images: off -> on"));
        assert!(s.contains("120 s -> 300 s"));
        assert!(s.contains("-> sat 01:00-05:00"), "{s}");
    }
}
