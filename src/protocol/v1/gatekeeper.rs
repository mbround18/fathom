//! Protocol types for 020-gatekeeper-policy-pane: a read-only view of OPA Gatekeeper
//! constraint compliance, discovered the same way `flux.rs` discovers Flux's CRDs (see
//! `src/backend/gatekeeper.rs`). Deliberately read-only: there is no create/update/delete
//! surface here (FR-009), and no constraint-authoring/template-creation type is defined
//! (FR-011, Kubescape-scope, deferred per `docs/FEATURE_BACKLOG.md`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::SectionKey;

/// The four honest states this pane can be in (FR-002). Kept as an enum rather than a
/// bare `installed: bool` (unlike `FluxOverview`) because this feature has an extra
/// state Flux's overview doesn't need: `PermissionDenied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum GatekeeperInstallState {
    /// Neither `constrainttemplates.templates.gatekeeper.sh` nor any CRD in the
    /// `constraints.gatekeeper.sh` API group was found.
    NotInstalled,
    /// Gatekeeper's CRDs are present but zero `Constraint` instances exist.
    InstalledNoConstraints,
    /// Gatekeeper's CRDs are present and at least one `Constraint` instance exists.
    Active,
    /// CRDs were found but listing `Constraint`/`ConstraintTemplate` instances failed
    /// with a permission error. `GatekeeperOverview.error` carries the verbatim backend
    /// error text (Principle I: errors reach the operator verbatim).
    PermissionDenied,
}

/// One entry from a `Constraint`'s `.status.violations[]`, enriched with a resolved
/// Inspector navigation target (`section`) so the frontend never has to guess a
/// kind-to-section mapping itself (mirrors `RelatedResourceRef.section`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperViolation {
    /// Where to navigate this object in the Inspector; `custom_resources()` is used as
    /// the fallback for kinds with no dedicated section.
    pub section: SectionKey,
    pub kind: String,
    /// Empty string for cluster-scoped resources (mirrors `FluxResourceSummary.namespace`'s
    /// convention).
    pub namespace: String,
    pub name: String,
    /// `enforcementAction` reported on this specific violation entry, falling back to the
    /// owning constraint's own `enforcementAction` when the entry itself doesn't carry one.
    pub enforcement_action: String,
    pub message: String,
}

/// One `Constraint` instance, of any constraint kind (each constraint kind is its own CRD
/// under the `constraints.gatekeeper.sh` API group).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperConstraintSummary {
    /// The constraint kind (== the CRD's Kind, e.g. "K8sRequiredLabels"), not the literal
    /// string "Constraint" (mirrors `FluxResourceSummary.kind` distinguishing concrete kinds).
    pub kind: String,
    pub name: String,
    /// `spec.enforcementAction` verbatim; empty string if unset. Gatekeeper defaults
    /// unset enforcementAction to "deny" at admission time, but this field shows what is
    /// actually on the object rather than an inferred default.
    pub enforcement_action: String,
    /// `.status.totalViolations` if present, else `.status.violations.len()` if `.status`
    /// is present but `totalViolations` isn't, else `None` when `.status` itself is
    /// entirely absent (the "awaiting status" case) — never coerced to `Some(0)`.
    pub total_violations: Option<u64>,
    /// Full detail for up to Gatekeeper's configured `constraintViolationsLimit` violations.
    pub violations: Vec<GatekeeperViolation>,
    /// True when `total_violations` is greater than `violations.len()` — drives the
    /// "showing N of M" truncation notice (FR-007). Computed live, never from a hardcoded
    /// limit constant, since `constraintViolationsLimit` is operator-configurable.
    pub violations_truncated: bool,
    /// True when this constraint's owning `ConstraintTemplate` CRD could not be found
    /// among the templates discovered in the same scan (orphaned-constraint edge case).
    pub template_missing: bool,
}

/// Top-level aggregate broadcast on every poll tick — the direct analog of `FluxOverview`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GatekeeperOverview {
    pub state: GatekeeperInstallState,
    /// Verbatim backend error text when `state == PermissionDenied`; empty otherwise.
    pub error: String,
    pub constraints: Vec<GatekeeperConstraintSummary>,
}

impl Default for GatekeeperOverview {
    fn default() -> Self {
        Self {
            state: GatekeeperInstallState::NotInstalled,
            error: String::new(),
            constraints: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back);
    }

    impl PartialEq for GatekeeperViolation {
        fn eq(&self, other: &Self) -> bool {
            self.section == other.section
                && self.kind == other.kind
                && self.namespace == other.namespace
                && self.name == other.name
                && self.enforcement_action == other.enforcement_action
                && self.message == other.message
        }
    }

    impl PartialEq for GatekeeperConstraintSummary {
        fn eq(&self, other: &Self) -> bool {
            self.kind == other.kind
                && self.name == other.name
                && self.enforcement_action == other.enforcement_action
                && self.total_violations == other.total_violations
                && self.violations == other.violations
                && self.violations_truncated == other.violations_truncated
                && self.template_missing == other.template_missing
        }
    }

    impl PartialEq for GatekeeperOverview {
        fn eq(&self, other: &Self) -> bool {
            self.state == other.state
                && self.error == other.error
                && self.constraints == other.constraints
        }
    }

    fn sample_violation() -> GatekeeperViolation {
        GatekeeperViolation {
            section: SectionKey::new("workloads/pods"),
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "nginx".to_owned(),
            enforcement_action: "deny".to_owned(),
            message: "missing required label \"team\"".to_owned(),
        }
    }

    fn sample_constraint() -> GatekeeperConstraintSummary {
        GatekeeperConstraintSummary {
            kind: "K8sRequiredLabels".to_owned(),
            name: "require-team-label".to_owned(),
            enforcement_action: "deny".to_owned(),
            total_violations: Some(1),
            violations: vec![sample_violation()],
            violations_truncated: false,
            template_missing: false,
        }
    }

    #[test]
    fn gatekeeper_install_state_roundtrip() {
        roundtrip(&GatekeeperInstallState::NotInstalled);
        roundtrip(&GatekeeperInstallState::InstalledNoConstraints);
        roundtrip(&GatekeeperInstallState::Active);
        roundtrip(&GatekeeperInstallState::PermissionDenied);
    }

    #[test]
    fn gatekeeper_violation_roundtrip() {
        roundtrip(&sample_violation());
    }

    #[test]
    fn gatekeeper_constraint_summary_roundtrip() {
        roundtrip(&sample_constraint());

        let awaiting_status = GatekeeperConstraintSummary {
            total_violations: None,
            ..sample_constraint()
        };
        roundtrip(&awaiting_status);

        let truncated = GatekeeperConstraintSummary {
            total_violations: Some(143),
            violations_truncated: true,
            template_missing: true,
            ..sample_constraint()
        };
        roundtrip(&truncated);
    }

    #[test]
    fn gatekeeper_overview_roundtrip() {
        roundtrip(&GatekeeperOverview {
            state: GatekeeperInstallState::Active,
            error: String::new(),
            constraints: vec![sample_constraint()],
        });

        roundtrip(&GatekeeperOverview {
            state: GatekeeperInstallState::NotInstalled,
            error: String::new(),
            constraints: Vec::new(),
        });

        roundtrip(&GatekeeperOverview {
            state: GatekeeperInstallState::PermissionDenied,
            error: "constraints.gatekeeper.sh is forbidden: User \"jane\" cannot list resource"
                .to_owned(),
            constraints: Vec::new(),
        });
    }
}
