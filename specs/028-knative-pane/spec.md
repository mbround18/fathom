# Feature Specification: Knative Pane

**Feature Branch**: `028-knative-pane`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Knative pane — detect Knative Serving via its CRDs (serving.knative.dev API group: Service, Revision, Route, Configuration), show a read-only tool-pane with Services/Revisions/Routes and, as the standout feature, a traffic-split visualization (Route → Revision percentage splits) and an explanation of why the autoscaler's actual replica count differs from desired (e.g. 'scaled to zero: no traffic in last 5m' or 'scaling up: N pending requests') — read this from the Revision/PodAutoscaler status fields. This traffic-split visualization should extend the existing topology/graph component rather than building a new graph renderer from scratch. Non-goals: no Knative Eventing surface (Triggers/Brokers/Channels), no traffic-split editing (read-only visualization, no mutation of Route weights)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See Knative workloads at a glance (Priority: P1)

A cluster operator opens Fathom against a cluster running Knative Serving and wants to see, without running `kubectl`, which Knative Services exist, what Revisions back them, and what Routes expose them.

**Why this priority**: This is the baseline value of the pane — without a working inventory list, nothing else in the feature matters. It mirrors the existing Flux panel pattern: detect-CRD-and-show.

**Independent Test**: Point Fathom at a cluster (or fixture) with the `serving.knative.dev` CRDs installed and at least one Service/Revision/Route, open the Knative pane, and confirm the three resource lists populate with correct names, namespaces, and status.

**Acceptance Scenarios**:

1. **Given** a cluster with Knative Serving CRDs installed and one Knative Service with one Revision and one Route, **When** the user opens the Knative pane, **Then** the Service, its Revision, and its Route are each listed with name, namespace, and ready/status indicator.
2. **Given** a cluster without any Knative Services, **When** the user opens the Knative pane, **Then** the pane shows an empty state indicating no Knative Services exist (distinct from "Knative not installed").

---

### User Story 2 - Understand traffic distribution across Revisions (Priority: P1)

An operator investigating a canary rollout wants to see what percentage of traffic each Revision of a Service is receiving, without manually reading Route YAML.

**Why this priority**: This is the standout, differentiating feature called out for this pane — traffic-split visualization is the primary reason this pane exists beyond a plain list view.

**Independent Test**: Given a Route whose spec/status traffic list splits traffic across two Revisions (e.g. 80/20), open the pane's traffic-split visualization and confirm both percentages render attached to the correct Revision nodes.

**Acceptance Scenarios**:

1. **Given** a Route with traffic split 80% to Revision A and 20% to Revision B, **When** the user views the traffic-split visualization, **Then** the visualization shows a Route node connected to both Revision nodes, each edge labeled with its percentage, and the percentages sum to 100%.
2. **Given** a Route with 100% of traffic pinned to a single "latest ready" Revision, **When** the user views the visualization, **Then** a single edge labeled 100% is shown from the Route to that Revision.
3. **Given** a Route whose traffic targets a Revision that no longer exists (stale target), **When** the user views the visualization, **Then** that target is shown distinctly (e.g. flagged as missing) rather than silently dropped or causing an error.

---

### User Story 3 - Understand why a Revision's replica count differs from desired (Priority: P2)

An operator sees a Revision running fewer (or more) pods than expected and wants a plain-language reason, without cross-referencing autoscaler internals.

**Why this priority**: This is the second standout feature but depends on User Story 1's inventory existing first; it adds diagnostic depth on top of the base list.

**Independent Test**: Given a Revision that has scaled to zero, open its detail and confirm the pane shows a human-readable reason (e.g. "scaled to zero: no traffic in last 5m"). Given a Revision that is scaling up due to pending requests, confirm the reason reflects that instead.

**Acceptance Scenarios**:

1. **Given** a Revision currently at 0 replicas because of inactivity, **When** the user views that Revision's detail, **Then** the pane displays a reason consistent with "scaled to zero" and, when available, the observed idle duration.
2. **Given** a Revision that is scaling up because of pending request load, **When** the user views that Revision's detail, **Then** the pane displays a reason consistent with "scaling up" and, when available, the number of pending requests driving that decision.
3. **Given** a Revision whose actual replica count already matches its desired count, **When** the user views that Revision's detail, **Then** the pane shows a neutral "at desired scale" state rather than a fabricated discrepancy explanation.
4. **Given** the underlying autoscaler status fields needed to explain a discrepancy are absent or unrecognized, **When** the user views that Revision's detail, **Then** the pane shows the raw actual-vs-desired numbers with a generic "reason unavailable" note instead of guessing.

---

### User Story 4 - Honest empty state when Knative isn't installed (Priority: P3)

An operator on a cluster without Knative Serving opens the pane (or looks for it) and gets a clear, honest signal rather than a broken or empty-looking screen.

**Why this priority**: Matches the existing detect-CRD-and-show pattern used by other optional-integration panels (e.g. Flux) so the UI never implies a capability that isn't present.

**Independent Test**: Point Fathom at a cluster without the `serving.knative.dev` CRDs installed and confirm the pane (or its entry point) clearly states Knative Serving is not detected, with no partial/errored resource lists shown.

**Acceptance Scenarios**:

1. **Given** a cluster without the `serving.knative.dev` CRD group installed, **When** the user opens or looks for the Knative pane, **Then** the UI clearly states Knative Serving was not detected on this cluster, and no resource list, traffic-split visualization, or scaling explanation is rendered.

---

### Edge Cases

- A Knative Service exists but has zero Revisions yet (still being created): the pane must show the Service without erroring on missing Revision data.
- A Route splits traffic across more than two Revisions: the visualization must scale to N targets without becoming unreadable, and percentages must still be traceable to their Revision.
- Traffic percentages in the Route status don't sum to exactly 100 (rounding, or a transient inconsistent state): the pane must display the values it read without fabricating a corrected total.
- CRDs for `serving.knative.dev` are installed but the specific `Service`/`Revision`/`Route`/`Configuration` kinds are only partially registered (e.g. mid-upgrade): the pane should reflect what it can detect kind-by-kind rather than treating detection as all-or-nothing.
- A Revision's autoscaler status uses fields from a different/older Knative version than expected: the "why" explanation must fall back to a generic message rather than showing an incorrect one.
- The user navigates away from the pane while a watch/poll is in flight: no stale data should be shown as current on return without a refresh indicator.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST detect whether Knative Serving is installed on the connected cluster by checking for the `serving.knative.dev` CRD group (Service, Revision, Route, Configuration kinds), following the same detect-and-show pattern used for other optional cluster integrations.
- **FR-002**: System MUST expose a dedicated, read-only Knative tool-pane that is only presented as active when Knative Serving is detected, and that clearly communicates non-detection otherwise (per User Story 4).
- **FR-003**: The Knative pane MUST list Knative Services with at least name, namespace, and ready/status condition.
- **FR-004**: The Knative pane MUST list Revisions with at least name, namespace, owning Service, ready/status condition, and actual vs. desired replica counts.
- **FR-005**: The Knative pane MUST list Routes with at least name, namespace, owning Service, and overall ready/status condition.
- **FR-006**: System MUST render a traffic-split visualization showing, for each Route, the percentage of traffic directed to each target Revision (or Configuration's latest-ready Revision), by extending the existing topology/graph view component rather than a new graph renderer.
- **FR-007**: The traffic-split visualization MUST display a distinct, non-erroring state for a traffic target that references a Revision that cannot be found.
- **FR-008**: System MUST derive and display a human-readable explanation for why a Revision's actual replica count differs from its desired replica count, sourced from Revision/PodAutoscaler status fields (e.g. scaled-to-zero due to inactivity, scaling up due to pending requests).
- **FR-009**: When actual and desired replica counts match, the pane MUST present a neutral "at desired scale" state rather than inventing a discrepancy reason.
- **FR-010**: When the autoscaler status fields needed to explain a discrepancy are missing or unrecognized, the pane MUST show the raw actual/desired numbers with a generic "reason unavailable" note rather than a guessed explanation.
- **FR-011**: All data shown in the pane MUST be read-only; the system MUST NOT provide any UI affordance to mutate Route traffic weights, Revision scale, or any other Knative resource from this pane.
- **FR-012**: The Knative pane MUST update its displayed data when the underlying resources change on the cluster (via the same watch/event mechanism used by other resource panels), without requiring a manual full-app refresh.
- **FR-013**: The system MUST NOT surface Knative Eventing resources (Triggers, Brokers, Channels, or related kinds) in this pane; Eventing is explicitly out of scope for this feature.
- **FR-014**: The Knative pane MUST respect existing namespace/context scoping used elsewhere in the app (i.e., it shows Knative resources for the currently selected context/namespace scope, not an unscoped cluster-wide dump when the app is otherwise namespace-scoped).

### Key Entities

- **Knative Service**: A user-facing Knative Serving resource representing a deployable app; owns one or more Configurations/Revisions and is targeted by a Route.
- **Revision**: An immutable snapshot of a Knative Service's spec at a point in time; carries current status including actual replica count, desired replica count, and autoscaler-derived state used to explain scaling behavior.
- **Route**: Maps traffic to one or more Revisions (directly or via a Configuration's latest-ready Revision), with a percentage weight per target; the source of truth for the traffic-split visualization.
- **Configuration**: Tracks the desired state for a Service's Revisions and points to the latest-ready Revision; referenced when a Route targets "latest" rather than a pinned Revision.
- **Autoscaler status signal**: The subset of status fields (e.g. activation state, pending request count, last-active timestamp) read to build the plain-language "why" explanation; not a distinct Kubernetes object but a derived, display-only concept.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can identify all Knative Services, their Revisions, and their Routes on a cluster within 10 seconds of opening the pane, without running any CLI command.
- **SC-002**: An operator can correctly state the traffic-split percentages across a Route's target Revisions by looking at the visualization alone, with 100% accuracy against the underlying Route status.
- **SC-003**: An operator can correctly explain, in their own words, why a scaled-to-zero or scaling-up Revision is not at its expected replica count, using only the pane's displayed explanation, without consulting raw YAML.
- **SC-004**: On a cluster without Knative Serving installed, 100% of users understand from the pane's state alone that Knative is not present, with no ambiguity or apparent error.
- **SC-005**: No user action within the pane can alter cluster state (zero mutating requests originate from this pane in any usage session).

## Assumptions

- Knative Serving version compatibility targets the current stable `serving.knative.dev/v1` API; older beta/alpha Knative API versions are read on a best-effort basis and are not guaranteed to populate every field.
- The autoscaler in use is Knative's default Knative Pod Autoscaler (KPA); explanations are derived from the status fields KPA populates. Clusters using HPA-only autoscaling for Knative Revisions may show the generic "reason unavailable" fallback more often, which is acceptable for v1.
- The existing topology/graph component (spec 007-topology-graph-view) can be extended with weighted/labeled edges and multiple target nodes without a rewrite; if a specific capability is missing, this feature adds the minimal extension needed rather than forking a parallel renderer.
- Detection, watch, and event-driven refresh reuse the same architectural pattern as the existing Flux GitOps panel (CRD-presence detection, then live watch of the relevant kinds).
- Namespace/context scoping behavior mirrors the rest of the app's existing resource panels; no new scoping model is introduced for Knative resources.
- Knative Eventing resources (Triggers, Brokers, Channels, Sources) are out of scope for v1 and may be considered in a future feature.
- Traffic-split editing (changing Route weights) is explicitly out of scope for v1; the pane is read-only.
