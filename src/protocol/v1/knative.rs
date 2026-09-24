use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Coarse readiness for a Knative Serving object, derived from its `Ready` status condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum KnativeReadyState {
    Ready,
    NotReady,
    Unknown,
}

/// Why a Revision's actual replica count differs from its desired replica count, derived from
/// the Revision's `Active` status condition (and, when unavailable, its raw replica counts).
/// Always paired with the raw actual/desired numbers by the caller so the UI never has to guess
/// when this is `Unavailable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data")]
pub enum KnativeScaleReason {
    /// `actual_replicas == desired_replicas`; nothing to explain.
    AtDesiredScale,
    /// Scaled to zero due to inactivity. `idle_since` is the `Active` condition's
    /// `lastTransitionTime`, when present.
    ScaledToZero { idle_since: Option<String> },
    /// Scaling up (activating or handling load). `pending_requests` is populated only when the
    /// autoscaler status exposed a concrete pending-request count.
    ScalingUp { pending_requests: Option<u32> },
    /// The status fields needed to classify a discrepancy were missing or unrecognized.
    Unavailable,
}

/// One Knative Service.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnativeServiceSummary {
    pub name: String,
    pub namespace: String,
    pub ready: KnativeReadyState,
    pub latest_ready_revision: Option<String>,
    pub latest_created_revision: Option<String>,
}

/// One Knative Revision.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnativeRevisionSummary {
    pub name: String,
    pub namespace: String,
    /// Owning Knative Service, from the `serving.knative.dev/service` label (falling back to an
    /// owner reference of kind `Configuration`/`Service` when the label is absent).
    pub service_name: String,
    pub ready: KnativeReadyState,
    pub actual_replicas: u32,
    pub desired_replicas: u32,
    pub scale_reason: KnativeScaleReason,
    /// Sum of `KnativeRouteTrafficTarget.percent` across every Route target that resolves to
    /// this Revision. `0` when no Route currently sends it traffic.
    pub traffic_percent: u32,
}

/// One entry in a Route's resolved traffic split (`status.traffic[]`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnativeRouteTrafficTarget {
    /// Always a concrete Revision name — Knative's own Route status already resolves
    /// Configuration/"latest ready" targets down to a Revision name (research.md R2).
    pub revision_name: String,
    pub percent: u32,
    pub tag: Option<String>,
    /// `true` when `revision_name` does not match any Revision this scan found (a stale target).
    pub revision_missing: bool,
}

/// One Knative Route.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnativeRouteSummary {
    pub name: String,
    pub namespace: String,
    pub service_name: String,
    pub ready: KnativeReadyState,
    pub url: Option<String>,
    pub traffic: Vec<KnativeRouteTrafficTarget>,
}

/// Aggregated Knative Serving state: whether it's installed at all, plus every Service,
/// Revision, and Route found. `installed` is `true` iff at least one of the four
/// `serving.knative.dev` CRDs (Service, Revision, Route, Configuration) is served — detection is
/// per-kind, not all-or-nothing, so a partial/mid-upgrade install still yields the data it can.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct KnativeOverview {
    pub installed: bool,
    pub services: Vec<KnativeServiceSummary>,
    pub revisions: Vec<KnativeRevisionSummary>,
    pub routes: Vec<KnativeRouteSummary>,
}
