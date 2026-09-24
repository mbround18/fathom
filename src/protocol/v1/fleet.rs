//! 027-fleet-rollup-dashboard: a view-only, all-contexts-at-once health rollup. See
//! `specs/027-fleet-rollup-dashboard/data-model.md` for the full contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::FluxOverview;

/// Health of one context's most recent fleet poll attempt. Fieldless enum, serialized as a
/// bare string (matches `ContextConnectionStatus`/`NotificationLevel`'s convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FleetHealthState {
    /// Reachable, 0 unhealthy pods, node/version fetched successfully.
    Healthy,
    /// Reachable, but at least one unhealthy pod (or a node NotReady).
    Degraded,
    /// Poll timed out or the connection failed (network/API-server error).
    Unreachable,
    /// Poll failed specifically due to credential/auth rejection.
    AuthError,
    /// No successful poll yet (first cycle since the poller started for this context).
    Pending,
}

/// GitOps rollup status for one context, derived by reducing its `FluxOverview`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FluxRollupStatus {
    /// Flux installed; every Kustomization/HelmRelease `Ready`.
    Synced,
    /// Flux installed; at least one Kustomization/HelmRelease not `Ready`.
    OutOfSync,
    /// No Flux CRDs detected on this cluster — "not applicable," never an error.
    NotInstalled,
}

/// One context's fleet-dashboard summary card. Validation rules (enforced by construction in
/// `src/backend/fleet.rs`):
/// - `error` MUST be `None` unless `health` is `Unreachable` or `AuthError`.
/// - `node_ready_count`/`node_total_count`/`unhealthy_pod_count`/`cluster_version` MUST all be
///   `None` together when `health` is `Pending`, `Unreachable`, or `AuthError` — a failed poll
///   never reports stale numbers.
/// - For `Healthy`/`Degraded`, a count is `None` only when its group is listed in `restricted`.
/// - `last_updated` is set on every poll attempt, success or failure.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct FleetContextSummary {
    pub context: String,
    pub health: FleetHealthState,
    pub error: Option<String>,
    pub node_ready_count: Option<u32>,
    pub node_total_count: Option<u32>,
    pub unhealthy_pod_count: Option<u32>,
    pub cluster_version: Option<String>,
    pub flux: Option<FluxRollupStatus>,
    /// Resource groups the credentials could not read (`"nodes"`, `"pods"`); the matching
    /// counts are `None` and the card shows "no access". Empty when nothing was denied, and
    /// always empty for `Pending`/`Unreachable`/`AuthError`.
    pub restricted: Vec<String>,
    /// ISO8601 timestamp.
    pub last_updated: String,
}

/// Reduce a `FluxOverview` into the fleet card's single-value rollup status.
pub fn reduce_flux_overview(overview: &FluxOverview) -> FluxRollupStatus {
    if !overview.installed {
        return FluxRollupStatus::NotInstalled;
    }
    let all_ready = overview
        .sources
        .iter()
        .chain(overview.kustomizations.iter())
        .chain(overview.helmreleases.iter())
        .all(|r| r.ready == super::FluxReadyState::Ready);
    if all_ready {
        FluxRollupStatus::Synced
    } else {
        FluxRollupStatus::OutOfSync
    }
}

#[cfg(test)]
mod tests {
    use super::super::FluxResourceSummary;
    use super::*;

    fn resource(ready: super::super::FluxReadyState) -> FluxResourceSummary {
        FluxResourceSummary {
            kind: "Kustomization".to_owned(),
            name: "apps".to_owned(),
            namespace: "flux-system".to_owned(),
            ready,
            message: String::new(),
            revision: String::new(),
            source_ref: None,
            suspended: false,
            interval: "5m".to_owned(),
            last_transition: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn not_installed_when_flux_absent() {
        let overview = FluxOverview::default();
        assert_eq!(
            reduce_flux_overview(&overview),
            FluxRollupStatus::NotInstalled
        );
    }

    #[test]
    fn synced_when_everything_ready() {
        let overview = FluxOverview {
            installed: true,
            sources: vec![resource(super::super::FluxReadyState::Ready)],
            kustomizations: vec![resource(super::super::FluxReadyState::Ready)],
            helmreleases: vec![],
        };
        assert_eq!(reduce_flux_overview(&overview), FluxRollupStatus::Synced);
    }

    #[test]
    fn out_of_sync_when_anything_not_ready() {
        let overview = FluxOverview {
            installed: true,
            sources: vec![resource(super::super::FluxReadyState::Ready)],
            kustomizations: vec![resource(super::super::FluxReadyState::Failed)],
            helmreleases: vec![],
        };
        assert_eq!(reduce_flux_overview(&overview), FluxRollupStatus::OutOfSync);
    }
}
