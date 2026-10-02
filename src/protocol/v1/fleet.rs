//! 027-fleet-rollup-dashboard: a view-only, all-contexts-at-once health rollup. See
//! `specs/027-fleet-rollup-dashboard/data-model.md` for the full contract.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

use super::{FluxOverview, SettingOrigin};

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
    /// 034: true when this summary comes from the context's live federation tier rather than
    /// the 027 poller. Additive; absent in older payloads.
    #[serde(default)]
    pub live: bool,
    /// 035: typed exec-plugin failure behind `error` (distinct from an API 401/403).
    pub plugin_error: Option<super::PluginError>,
}

// ---------------------------------------------------------------------------------------
// 046-fleet-model: the fleet as a named, labelled, groupable set of kubeconfig contexts.
// See `specs/046-fleet-model/data-model.md`. Nothing here creates a poller, a client or a
// watch (FR-015); it is operator-authored metadata, never read from or written to any
// Kubernetes object.
// ---------------------------------------------------------------------------------------

/// Whether a fleet member's context is still in the current kubeconfig. An `Absent` member
/// keeps every byte of its metadata and is removed only by `ForgetFleetMember` (FR-009).
/// Fieldless enum, serialized as a bare string like `FleetHealthState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FleetMemberAvailability {
    /// The context appears in the kubeconfig Fathom just listed.
    Present,
    /// The context does not. Metadata is retained, the member is shown as unavailable.
    Absent,
}

/// One kubeconfig context as the fleet knows it (046 FR-001/FR-002).
///
/// `context` is the kubeconfig context name verbatim — never the API server URL, so two
/// contexts pointing at one API server are two members. `display_name` is shown *beside*
/// `context`, never instead of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct FleetMember {
    /// The key. Non-empty. The kubeconfig context name, verbatim (FR-001).
    pub context: String,
    /// Operator-set, trimmed, max 128 chars. Empty after trimming is `None`, never `""`.
    pub display_name: Option<String>,
    /// Operator-authored labels, validated at entry by `src/backend/fleet_model.rs`.
    /// Ordered so the persisted file and every rendering are stable.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Per-field provenance, keyed by field name (`display_name`) or `labels/<key>`.
    /// Mirrors `ClusterSettingsFieldOrigin`. A local value wins, is marked local, and the
    /// shared value it overrode stays visible (FR-013).
    #[serde(default)]
    pub origins: BTreeMap<String, SettingOrigin>,
    pub availability: FleetMemberAvailability,
}

/// A named label selector over members. Membership is computed on every model update,
/// never stored (FR-004). A group matching nothing is shown as matching nothing — it is
/// not hidden, rejected or deleted (FR-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct FleetGroup {
    /// Non-empty, trimmed, unique within the fleet, max 128 chars.
    pub name: String,
    /// The operator's expression, stored verbatim as typed. Parsed at entry; an invalid
    /// expression is rejected with the parse error verbatim and no group is created
    /// (FR-006). Round-trips to the string `ListParams::labels()` takes.
    pub selector: String,
    pub origin: SettingOrigin,
}

/// Why one member of a group is not covered by a rollup.
///
/// These four variants are deliberately distinct and MUST NOT collapse into one (FR-011).
/// A denied read is not an empty cluster, an unreachable cluster is not a missing one, and
/// a member that has simply not been polled yet is none of those. This is the type that
/// stops the `integrationVisible` mistake from being repeated: there is no way to express
/// "nothing found" without saying whether it was looked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FleetGapReason {
    /// The credentials were refused (a 403). `detail` is the API server's own reason,
    /// verbatim (Principle I).
    NotPermitted { detail: String },
    /// The connection failed. `detail` is the transport/API error, verbatim.
    Unreachable { detail: String },
    /// The member's context is not in the current kubeconfig.
    Absent,
    /// The member has no summary yet — nothing has polled it.
    Pending,
}

/// One member a rollup could not include, named rather than counted (FR-010).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FleetCoverageGap {
    /// The member not covered, by context name.
    pub context: String,
    pub reason: FleetGapReason,
}

/// A group's aggregated health, which always carries its own coverage.
///
/// Two invariants, asserted in test (FR-010, SC-004):
/// 1. `members_covered <= members_total`.
/// 2. `gaps.len() == members_total - members_covered` — one gap per uncovered member.
///
/// There is deliberately no constructor and no variant of this type that yields
/// `node_ready`/`node_total`/`unhealthy_pods` without `members_covered`, `members_total`
/// and `gaps` beside them: a sum can never be stated without stating what it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FleetGroupRollup {
    /// The group name this rolls up.
    pub group: String,
    /// Members the group's selector currently matches.
    pub members_total: u32,
    /// Members whose summary was read in full. Always `<= members_total`.
    pub members_covered: u32,
    /// One entry per uncovered member. Length is `members_total - members_covered`.
    pub gaps: Vec<FleetCoverageGap>,
    /// Sums over the covered members only — never rendered without the coverage fields.
    pub node_ready: u32,
    pub node_total: u32,
    pub unhealthy_pods: u32,
}

/// The publishable slice of one member: display name and labels only. No field here is
/// capable of carrying a credential (FR-020).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
pub struct FleetMemberShare {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

/// The publishable form of labels and groups, in the 014 shared config store. Attributes
/// mirror `ConnectionProfile` (`src/protocol/v1/team.rs`) because FR-012 says to reuse
/// 014's semantics, not invent new ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SharedFleetDefinition {
    /// Stable id within the store.
    pub id: String,
    /// Context name -> the shareable slice of that member.
    #[serde(default)]
    pub members: BTreeMap<String, FleetMemberShare>,
    /// Group name -> selector expression, verbatim as its author typed it.
    #[serde(default)]
    pub groups: BTreeMap<String, String>,
    /// Optimistic concurrency. A stale publish is refused with `StoreError::Conflict`,
    /// never silently overwritten.
    pub rev: u64,
    /// Attribution, as 014 records it.
    pub shared_by: String,
    /// RFC 3339.
    pub updated_at: String,
    /// `true` once removed upstream. The row is **kept**, never deleted, so an operator
    /// who adopted it is told "no longer shared" instead of losing it (FR-014).
    #[serde(default)]
    pub removed: bool,
}

/// Local record of a definition the operator adopted. Losing store access, or the upstream
/// definition going away, MUST NOT delete this row — it only stops following the shared
/// copy (FR-014).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AdoptedFleetDefinition {
    /// The copy as adopted.
    pub definition: SharedFleetDefinition,
    /// RFC 3339.
    pub adopted_at: String,
    /// `false` when the store is unreachable or the definition was removed upstream. The
    /// local copy is **never** deleted in either case (FR-014); this flag is the only
    /// thing that changes.
    #[serde(default = "default_true")]
    pub still_shared: bool,
}

fn default_true() -> bool {
    true
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

    // -----------------------------------------------------------------------------------
    // 046-fleet-model round-trips (FR-019). Every new type, and one case per
    // `FleetGapReason` variant so a collapsed enum fails the build (FR-011).
    // -----------------------------------------------------------------------------------

    fn rt<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value, "round-trip changed the value: {json}");
    }

    fn member() -> FleetMember {
        FleetMember {
            context: "arn:aws:eks:eu-west-1:1234:cluster/prod-a".to_owned(),
            display_name: Some("prod a".to_owned()),
            labels: BTreeMap::from([
                ("env".to_owned(), "prod".to_owned()),
                ("team.example.com/owner".to_owned(), String::new()),
            ]),
            origins: BTreeMap::from([
                ("display_name".to_owned(), SettingOrigin::Local),
                ("labels/env".to_owned(), SettingOrigin::OrgDefault),
            ]),
            availability: FleetMemberAvailability::Present,
        }
    }

    #[test]
    fn fleet_member_roundtrip() {
        rt(&member());
        rt(&FleetMember {
            display_name: None,
            labels: BTreeMap::new(),
            origins: BTreeMap::new(),
            availability: FleetMemberAvailability::Absent,
            ..member()
        });
        rt(&FleetMemberAvailability::Present);
        rt(&FleetMemberAvailability::Absent);
    }

    #[test]
    fn fleet_group_roundtrip() {
        rt(&FleetGroup {
            name: "prod".to_owned(),
            selector: "env in (prod,staging),!canary".to_owned(),
            origin: SettingOrigin::Local,
        });
        rt(&FleetGroup {
            name: "shared".to_owned(),
            selector: "env=prod".to_owned(),
            origin: SettingOrigin::OrgDefault,
        });
    }

    /// One case per variant. If `NotPermitted`, `Unreachable`, `Absent` and `Pending` are
    /// ever collapsed into one variant, this test stops compiling (FR-011).
    #[test]
    fn fleet_gap_reason_roundtrips_each_variant_distinctly() {
        let not_permitted = FleetGapReason::NotPermitted {
            detail: "nodes is forbidden: User \"alice\" cannot list resource \"nodes\"".to_owned(),
        };
        let unreachable = FleetGapReason::Unreachable {
            detail: "error trying to connect: tcp connect error: timed out".to_owned(),
        };
        let absent = FleetGapReason::Absent;
        let pending = FleetGapReason::Pending;

        rt(&not_permitted);
        rt(&unreachable);
        rt(&absent);
        rt(&pending);

        // Four distinct serializations: no two reasons may be indistinguishable on the wire.
        let wire: Vec<String> = [&not_permitted, &unreachable, &absent, &pending]
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        let unique: std::collections::BTreeSet<&String> = wire.iter().collect();
        assert_eq!(
            unique.len(),
            4,
            "gap reasons collapsed on the wire: {wire:?}"
        );
        assert_ne!(not_permitted, unreachable);
        assert_ne!(absent, pending);
    }

    #[test]
    fn fleet_coverage_gap_roundtrip() {
        for reason in [
            FleetGapReason::NotPermitted {
                detail: "forbidden".to_owned(),
            },
            FleetGapReason::Unreachable {
                detail: "no route to host".to_owned(),
            },
            FleetGapReason::Absent,
            FleetGapReason::Pending,
        ] {
            rt(&FleetCoverageGap {
                context: "prod-b".to_owned(),
                reason,
            });
        }
    }

    #[test]
    fn fleet_group_rollup_roundtrip_and_invariants() {
        let rollup = FleetGroupRollup {
            group: "prod".to_owned(),
            members_total: 3,
            members_covered: 1,
            gaps: vec![
                FleetCoverageGap {
                    context: "prod-b".to_owned(),
                    reason: FleetGapReason::NotPermitted {
                        detail: "pods is forbidden".to_owned(),
                    },
                },
                FleetCoverageGap {
                    context: "prod-c".to_owned(),
                    reason: FleetGapReason::Absent,
                },
            ],
            node_ready: 4,
            node_total: 5,
            unhealthy_pods: 2,
        };
        rt(&rollup);
        assert!(rollup.members_covered <= rollup.members_total);
        assert_eq!(
            rollup.gaps.len(),
            (rollup.members_total - rollup.members_covered) as usize
        );
        // Coverage is not optional: it is present in the serialized form, always.
        let json = serde_json::to_string(&rollup).unwrap();
        assert!(json.contains("\"members_covered\":1"), "{json}");
        assert!(json.contains("\"members_total\":3"), "{json}");
        assert!(json.contains("\"gaps\""), "{json}");
    }

    fn definition() -> SharedFleetDefinition {
        SharedFleetDefinition {
            id: "prod-definition".to_owned(),
            members: BTreeMap::from([(
                "prod-a".to_owned(),
                FleetMemberShare {
                    display_name: Some("prod a".to_owned()),
                    labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
                },
            )]),
            groups: BTreeMap::from([("prod".to_owned(), "env in (prod)".to_owned())]),
            rev: 7,
            shared_by: "alice".to_owned(),
            updated_at: "2026-10-02T12:00:00Z".to_owned(),
            removed: false,
        }
    }

    #[test]
    fn shared_fleet_definition_roundtrip() {
        rt(&FleetMemberShare::default());
        rt(&FleetMemberShare {
            display_name: Some("prod a".to_owned()),
            labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
        });
        rt(&definition());
        rt(&SharedFleetDefinition {
            removed: true,
            ..definition()
        });
    }

    #[test]
    fn adopted_fleet_definition_roundtrip_keeps_removed_rows() {
        rt(&AdoptedFleetDefinition {
            definition: definition(),
            adopted_at: "2026-10-02T12:05:00Z".to_owned(),
            still_shared: true,
        });
        // The row survives an upstream removal; only `still_shared` changes (FR-014).
        let orphaned = AdoptedFleetDefinition {
            definition: SharedFleetDefinition {
                removed: true,
                ..definition()
            },
            adopted_at: "2026-10-02T12:05:00Z".to_owned(),
            still_shared: false,
        };
        rt(&orphaned);
        assert_eq!(orphaned.definition.id, "prod-definition");

        // An older payload with no `still_shared` reads as still following.
        let legacy = serde_json::json!({
            "definition": definition(),
            "adopted_at": "2026-10-02T12:05:00Z",
        });
        let back: AdoptedFleetDefinition = serde_json::from_value(legacy).unwrap();
        assert!(back.still_shared);
    }
}
