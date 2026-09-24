# Feature Specification: Gatekeeper Policy Pane

**Feature Branch**: `020-gatekeeper-policy-pane`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Gatekeeper policy pane — detect OPA Gatekeeper via its ConstraintTemplate/Constraint CRDs (constraints.gatekeeper.sh, templates.gatekeeper.sh API groups), show live constraint list with violation counts (from each Constraint's .status.totalViolations / .status.violations), drill into a constraint to see its violating resources with a link-out to each one's Inspector. Read-only, no mutation surface, no new external dependency (pure kube-rs CRD watches, same mechanism as the Flux panel). Non-goals: no policy authoring/editing UI (Kubescape-scope, deferred), no constraint-template creation."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See live policy compliance status at a glance (Priority: P1)

An operator managing a cluster that enforces OPA Gatekeeper policies opens Fathom and wants to know, without leaving the app or running `kubectl`, whether any Gatekeeper constraints are currently being violated and by how much. They open the Gatekeeper pane and see every active constraint with its live violation count.

**Why this priority**: This is the entire value proposition — a read-only, always-current view of policy compliance that today requires `kubectl get constraints -o yaml` or a separate dashboard. It is the smallest slice that is independently useful and mirrors the Flux and Hubble panels already shipped.

**Independent Test**: Can be fully tested by pointing Fathom at a cluster with Gatekeeper installed and at least one Constraint with violations, opening the Gatekeeper pane, and confirming the constraint list renders with accurate, live-updating violation counts.

**Acceptance Scenarios**:

1. **Given** a cluster with OPA Gatekeeper installed and one or more `Constraint` resources present, **When** the operator opens the Gatekeeper pane, **Then** they see a list of constraints, each showing its name, kind (constraint template), enforcement action, and current violation count drawn from `.status.totalViolations`.
2. **Given** a constraint's violation count changes in the cluster (a new violation appears or an existing one is resolved), **When** the pane is open, **Then** the displayed count updates without the operator needing to manually refresh.
3. **Given** a constraint has zero violations, **When** the operator views the list, **Then** it is shown with a clear "0 violations" / compliant indicator rather than being omitted.

---

### User Story 2 - Drill into a constraint to find and fix violating resources (Priority: P1)

An operator sees a constraint with a nonzero violation count and wants to know exactly which resources are out of compliance so they can go fix them. They click into the constraint and see the list of violating resources, each with enough detail (namespace, kind, name, violation message) to understand the problem, and can jump straight to that resource's existing Inspector view.

**Why this priority**: Seeing a violation count alone doesn't help an operator act — the drill-down to specific offending resources with a direct link to the existing Inspector is what turns this into a usable troubleshooting tool. It ships in the same slice as Story 1 because a constraint list with no way to see *what* is violating is not independently useful.

**Independent Test**: Can be fully tested by selecting a constraint with known violations from the list, confirming each violating resource is listed with its identifying details and violation message, clicking one, and confirming the app navigates to that resource's Inspector view with the correct object loaded.

**Acceptance Scenarios**:

1. **Given** a constraint with one or more entries in `.status.violations`, **When** the operator selects that constraint, **Then** they see a list of violating resources, each showing namespace, kind, name, and the violation message reported by Gatekeeper.
2. **Given** a violating resource is listed, **When** the operator clicks it, **Then** the app opens the existing resource Inspector scoped to that exact object (namespace/kind/name).
3. **Given** a constraint's violation count exceeds the number of entries Gatekeeper records in `.status.violations` (Gatekeeper truncates detailed violations after a configured limit), **When** the operator views the drill-down, **Then** the app shows the truncation honestly (e.g. "showing 20 of 143 violations — increase Gatekeeper's `constraintViolationsLimit` to see more") rather than implying the list is complete.

---

### User Story 3 - Honest empty state when Gatekeeper isn't installed (Priority: P2)

An operator on a cluster that does not run OPA Gatekeeper opens the pane (or the app in general) and is told plainly that Gatekeeper isn't detected, instead of seeing a blank or broken screen, matching the same honest-empty-state pattern already used by the Flux and Hubble panels.

**Why this priority**: Most clusters Fathom connects to will not have Gatekeeper installed. Without this, operators would see either an error or an empty list they can't distinguish from "installed with zero constraints," which is confusing and inconsistent with the product's existing pattern.

**Independent Test**: Can be fully tested by pointing Fathom at a cluster with no Gatekeeper CRDs installed, opening the Gatekeeper pane, and confirming an explicit "Gatekeeper not installed" state is shown rather than an empty or error state.

**Acceptance Scenarios**:

1. **Given** a cluster with no `constraints.gatekeeper.sh` or `templates.gatekeeper.sh` CRDs registered, **When** the operator opens the Gatekeeper pane, **Then** they see an explicit "OPA Gatekeeper is not installed on this cluster" message, not a loading spinner that never resolves, an empty list, or an error.
2. **Given** Gatekeeper is installed but has zero `Constraint` resources defined, **When** the operator opens the pane, **Then** they see a distinct "Gatekeeper is installed but no constraints are defined" message, clearly different from the "not installed" state.
3. **Given** Gatekeeper CRDs are installed but the app's current RBAC context lacks permission to list/watch `Constraint` or `ConstraintTemplate` resources, **When** the operator opens the pane, **Then** the app surfaces the specific permission error rather than reporting "not installed."

### Edge Cases

- What happens when a `Constraint`'s `.status` field is entirely absent (Gatekeeper hasn't reconciled it yet, or the audit controller is disabled)? The pane shows the constraint with an "awaiting status" / unknown-violations indicator rather than assuming zero violations.
- How does the system handle a violating resource that no longer exists in the cluster (deleted after the violation was recorded but before audit re-runs)? Clicking it shows the Inspector's own "resource not found" state rather than the pane crashing.
- How does the system handle Gatekeeper's audit controller being disabled entirely (so `.status.violations` never populates even though constraints exist)? Constraints show as "awaiting status," consistent with the general awaiting-status edge case above.
- What happens when a constraint references a `ConstraintTemplate` that has since been deleted (orphaned constraint)? The constraint still appears in the list using the data available on the `Constraint` object itself, flagged as referencing a missing template.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST detect whether OPA Gatekeeper is installed on the connected cluster by checking for the presence of the `constrainttemplates.templates.gatekeeper.sh` and the `constraints.gatekeeper.sh` group's CRDs.
- **FR-002**: System MUST present a distinct, honest state for each of: Gatekeeper not installed, Gatekeeper installed with zero constraints, Gatekeeper installed with constraints (normal view), and insufficient RBAC permission to read Gatekeeper resources.
- **FR-003**: System MUST list all `Constraint` resources across all constraint kinds (i.e. across every CRD in the `constraints.gatekeeper.sh` API group), showing at minimum: constraint name, constraint kind (template), enforcement action, and total violation count.
- **FR-004**: System MUST source each constraint's violation count from `.status.totalViolations`, falling back to a count of `.status.violations` entries if `totalViolations` is absent, and MUST show a distinct "awaiting status" indicator when `.status` itself is absent.
- **FR-005**: System MUST keep the constraint list and violation counts live-updating for as long as the pane is open, using the same watch-based mechanism as the existing Flux panel, without requiring manual refresh.
- **FR-006**: System MUST allow the operator to select any listed constraint and view its individual violating resources, each showing namespace, kind, name, and violation message, drawn from `.status.violations`.
- **FR-007**: System MUST clearly indicate when `.status.violations` is truncated relative to `.status.totalViolations` (Gatekeeper's `constraintViolationsLimit` default), rather than presenting a partial list as complete.
- **FR-008**: System MUST allow the operator to navigate from any listed violating resource directly to the existing resource Inspector for that exact object.
- **FR-009**: System MUST NOT expose any mutation, edit, or deletion affordance for constraints, constraint templates, or violating resources from within this pane — it is strictly read-only.
- **FR-010**: System MUST NOT introduce any new external dependency (binary, sidecar, or library beyond what already provides CRD watch support) to implement detection or data retrieval — detection and live data MUST use the same kube-rs CRD watch mechanism as the existing Flux panel.
- **FR-011**: System MUST NOT provide any constraint-authoring, constraint-template-creation, or policy-editing UI; this is explicitly out of scope for this feature.

### Key Entities

- **Constraint**: A live Gatekeeper policy instance (one of many kinds under the `constraints.gatekeeper.sh` API group). Key attributes: name, constraint kind, enforcement action, total violation count, list of individual violations (each with namespace, kind, name, and message), and reference to the `ConstraintTemplate` it was generated from.
- **ConstraintTemplate**: The Gatekeeper CRD that defines a constraint kind's policy logic and schema. Used here only to confirm Gatekeeper's presence and to resolve a constraint's template name/kind for display — not edited or created by this feature.
- **Violation**: A single entry within a Constraint's status reporting one non-compliant cluster resource (namespace, kind, name, message).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can determine whether any Gatekeeper policy is currently being violated, and by how much, within 5 seconds of opening the Gatekeeper pane, with no manual refresh.
- **SC-002**: An operator can go from seeing a nonzero violation count to viewing the specific offending resource's Inspector in 2 clicks or fewer.
- **SC-003**: On a cluster without Gatekeeper installed, 100% of operators see an explicit "not installed" message rather than an ambiguous empty or error state.
- **SC-004**: The pane never presents a truncated violation list as if it were complete — every truncated view visibly states how many violations exist versus how many are shown.

## Assumptions

- Gatekeeper's `Constraint` and `ConstraintTemplate` CRDs follow the upstream OPA Gatekeeper API shape (`constraints.gatekeeper.sh/v1beta1` and `templates.gatekeeper.sh/v1`), including the conventional `.status.totalViolations` and `.status.violations[]` fields; non-standard or heavily customized Gatekeeper forks are out of scope.
- The existing resource Inspector already supports opening an arbitrary namespace/kind/name reference, since this pattern is reused (not introduced) by the Flux and Hubble panels' link-outs.
- RBAC context and permission-error surfacing follow the same pattern already established for other read-only CRD-backed panels (Flux, Hubble) — no new permission model is introduced.
- "Live" means driven by a kube-rs watch on the relevant CRDs (consistent with the Flux panel), not a fixed polling interval, though a bounded poll/reconcile fallback consistent with the Flux panel's own implementation is acceptable if that is the existing pattern.
- Because Gatekeeper's own audit controller (not this feature) is responsible for populating `.status.violations`, the pane reflects whatever cadence that controller runs on and does not force a re-audit.
