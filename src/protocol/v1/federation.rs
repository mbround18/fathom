//! 034-simultaneous-federation: an opt-in set of contexts held live at once. See
//! `specs/034-simultaneous-federation/data-model.md` for the contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Connection state of one federated context's live tier. Serialized as a bare string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FederationConnectionState {
    /// Client is being built / first sync not finished.
    Pending,
    /// Watches are running and counters are current.
    Live,
    /// A previously live (or failing) connection is being re-established. Any numbers carried
    /// are last-known and must be presented as stale.
    Reconnecting,
    /// The API server could not be reached (network error, timeout, context missing).
    Unreachable,
    /// Credentials were rejected (401/403 on connect).
    AuthError,
}

/// Live counters for one federated context. Numeric fields are `None` when `state` is
/// `Pending`, `Unreachable` or `AuthError`, or when the metric is listed in `restricted`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct FederationContextRollup {
    pub context: String,
    pub state: FederationConnectionState,
    /// Monotonic per federate; consumers drop events older than the newest seen.
    pub generation: u64,
    /// Verbatim; only for `Unreachable`/`AuthError`.
    pub error: Option<String>,
    pub nodes_ready: Option<u32>,
    pub nodes_total: Option<u32>,
    pub pods_running: Option<u32>,
    pub pods_pending: Option<u32>,
    pub pods_failed: Option<u32>,
    pub pods_unhealthy: Option<u32>,
    pub deployments_unavailable: Option<u32>,
    pub warning_events_1h: Option<u32>,
    pub cluster_version: Option<String>,
    /// Metrics the credentials cannot read (`"nodes"`, `"pods"`, `"deployments"`, `"events"`).
    pub restricted: Vec<String>,
    /// ISO8601 timestamp.
    pub last_updated: String,
}

/// Sums over `Live` contexts only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FederationCombinedRollup {
    pub contributing_contexts: u32,
    pub total_contexts: u32,
    pub nodes_ready: u32,
    pub nodes_total: u32,
    pub pods_running: u32,
    pub pods_pending: u32,
    pub pods_failed: u32,
    pub pods_unhealthy: u32,
    pub deployments_unavailable: u32,
    pub warning_events_1h: u32,
}

/// Soft cap on federated contexts; exceeding it warns but never disconnects.
pub const FEDERATION_SOFT_CAP: u32 = 8;

impl FederationCombinedRollup {
    /// Combine per-context rollups; only `Live` contexts contribute.
    pub fn combine<'a>(
        total_contexts: u32,
        rollups: impl IntoIterator<Item = &'a FederationContextRollup>,
    ) -> Self {
        let mut c = Self {
            total_contexts,
            ..Self::default()
        };
        for r in rollups {
            if r.state != FederationConnectionState::Live {
                continue;
            }
            c.contributing_contexts += 1;
            c.nodes_ready += r.nodes_ready.unwrap_or(0);
            c.nodes_total += r.nodes_total.unwrap_or(0);
            c.pods_running += r.pods_running.unwrap_or(0);
            c.pods_pending += r.pods_pending.unwrap_or(0);
            c.pods_failed += r.pods_failed.unwrap_or(0);
            c.pods_unhealthy += r.pods_unhealthy.unwrap_or(0);
            c.deployments_unavailable += r.deployments_unavailable.unwrap_or(0);
            c.warning_events_1h += r.warning_events_1h.unwrap_or(0);
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(nodes: u32, unhealthy: u32) -> FederationContextRollup {
        FederationContextRollup {
            context: "a".into(),
            state: FederationConnectionState::Live,
            generation: 1,
            error: None,
            nodes_ready: Some(nodes),
            nodes_total: Some(nodes),
            pods_running: Some(10),
            pods_pending: Some(1),
            pods_failed: Some(0),
            pods_unhealthy: Some(unhealthy),
            deployments_unavailable: Some(0),
            warning_events_1h: Some(2),
            cluster_version: None,
            restricted: vec![],
            last_updated: String::new(),
        }
    }

    #[test]
    fn combine_only_counts_live() {
        let mut down = live(9, 9);
        down.state = FederationConnectionState::Unreachable;
        let all = [live(3, 1), live(2, 2), down];
        let c = FederationCombinedRollup::combine(3, all.iter());
        assert_eq!(c.contributing_contexts, 2);
        assert_eq!(c.total_contexts, 3);
        assert_eq!(c.nodes_total, 5);
        assert_eq!(c.pods_unhealthy, 3);
        assert_eq!(c.warning_events_1h, 4);
    }
}
