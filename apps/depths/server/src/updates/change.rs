//! Settings changes waiting for their `Confirm` (`PreviewUpdateChange(Settings)`).
//!
//! US1 takes only `checks_enabled` and `interval_secs`; the other settings arrive with the user
//! stories that use them. A preview belongs to the session that made it, is used once, and
//! expires after [`PREVIEW_TTL_SECS`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use depths_protocol::{PreviewId, PreviewVoidReason, UpdateSettings};

use super::Updates;

pub const PREVIEW_TTL_SECS: i64 = 300;

/// A settings change shown to an admin and not yet confirmed.
#[derive(Debug, Clone)]
pub struct PendingChange {
    pub session: String,
    pub before: UpdateSettings,
    pub after: UpdateSettings,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct Pending(HashMap<PreviewId, PendingChange>);

/// Why a settings change is refused before it is previewed.
pub fn check(before: &UpdateSettings, after: &UpdateSettings) -> Result<(), String> {
    let mut only_allowed = after.clone();
    only_allowed.checks_enabled = before.checks_enabled;
    only_allowed.interval_secs = before.interval_secs;
    if &only_allowed != before {
        return Err(
            "only checks_enabled and interval_secs can be changed for now; the other settings come with automatic updates".into(),
        );
    }
    if after == before {
        return Err("nothing to change".into());
    }
    if after.interval_secs < super::store::MIN_INTERVAL_SECS {
        return Err(format!(
            "interval_secs is {}; the minimum is {} (one hour)",
            after.interval_secs,
            super::store::MIN_INTERVAL_SECS
        ));
    }
    Ok(())
}

/// The before/after summary a preview shows, one line per changed field.
pub fn summary(before: &UpdateSettings, after: &UpdateSettings) -> String {
    let mut lines = Vec::new();
    if before.checks_enabled != after.checks_enabled {
        let word = |b: bool| if b { "on" } else { "off" };
        lines.push(format!(
            "update checks: {} -> {}",
            word(before.checks_enabled),
            word(after.checks_enabled)
        ));
    }
    if before.interval_secs != after.interval_secs {
        lines.push(format!(
            "check interval: {} -> {}",
            interval(before.interval_secs),
            interval(after.interval_secs)
        ));
    }
    lines.join("\n")
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

    /// Whether `id` is a settings change this session made (and so whether `Confirm` is ours).
    pub fn has_change(&self, session: &str, id: PreviewId) -> bool {
        self.pending_lock()
            .0
            .get(&id)
            .is_some_and(|c| c.session == session)
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

    fn base() -> UpdateSettings {
        StoredSettings::default().settings()
    }

    #[test]
    fn only_checks_and_interval_may_change() {
        let b = base();
        let mut a = b.clone();
        a.checks_enabled = !b.checks_enabled;
        a.interval_secs = 7200;
        assert!(check(&b, &a).is_ok());
        let mut other = b.clone();
        other.cleanup = !b.cleanup;
        assert!(
            check(&b, &other)
                .unwrap_err()
                .contains("only checks_enabled")
        );
        assert_eq!(check(&b, &b).unwrap_err(), "nothing to change");
        let mut short = b.clone();
        short.interval_secs = 60;
        assert!(check(&b, &short).unwrap_err().contains("minimum"));
    }

    #[test]
    fn summary_names_each_change() {
        let b = base();
        let mut a = b.clone();
        a.checks_enabled = !b.checks_enabled;
        a.interval_secs = 7200;
        let s = summary(&b, &a);
        assert!(s.contains("update checks"));
        assert!(s.contains("every 2 h"));
    }
}
