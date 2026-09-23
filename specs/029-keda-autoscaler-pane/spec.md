# Feature Specification: KEDA Autoscaler Pane

**Feature Branch**: `029-keda-autoscaler-pane`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "KEDA autoscaler pane — detect KEDA via its CRDs (keda.sh API group: ScaledObject, ScaledJob, TriggerAuthentication, ClusterTriggerAuthentication), show a read-only tool-pane listing ScaledObjects/ScaledJobs with current replica count, which trigger (queue depth, cron, Prometheus metric, etc.) is driving the current scale decision, and why current replicas differ from min/max bounds. If Fathom's metrics stack (from the existing Hubble/Prometheus integration work) is already available, optionally surface the live trigger metric value driving the decision — but don't make this a hard dependency; degrade gracefully to trigger-type + threshold display if metrics aren't available. Complement the existing pod-lifecycle timeline feature by linking a scaled workload's pods to their KEDA-driven scale history where reasonable. Non-goals: no ScaledObject creation/editing UI (read-only for v1), no custom trigger-type-specific configuration UI beyond a generic key/value display of trigger metadata."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See whether KEDA is scaling my workloads (Priority: P1)

An operator connects Fathom to a cluster that runs KEDA and wants a single place to see every autoscaled workload (Deployment or Job) without inspecting raw `ScaledObject`/`ScaledJob` YAML by hand.

**Why this priority**: This is the entry point for the whole feature. Without a working list of autoscaled workloads, there is nothing to inspect and no scaling reasoning to surface.

**Independent Test**: Can be fully tested by opening the new "Autoscalers" (KEDA) section against a cluster with the KEDA CRDs installed and one or more `ScaledObject`/`ScaledJob` resources, and confirming each appears exactly once with its target workload, current replica count, and min/max bounds.

**Acceptance Scenarios**:

1. **Given** a cluster with KEDA installed and a `ScaledObject` named "queue-worker" targeting a Deployment with `minReplicaCount: 1` and `maxReplicaCount: 10`, currently at 4 replicas, **When** the operator opens the Autoscalers section, **Then** "queue-worker" appears as a row showing its target workload, current replica count (4), and its min/max bounds (1/10).
2. **Given** the currently selected namespace filter excludes the namespace containing "queue-worker", **When** the operator views the Autoscalers list, **Then** "queue-worker" is not shown, consistent with how every other resource list in Fathom already respects the namespace filter.
3. **Given** a cluster with both `ScaledObject` and `ScaledJob` resources, **When** the operator views the list, **Then** both kinds appear, distinguishable by a type indicator (Deployment-style autoscaler vs. Job-style autoscaler).

---

### User Story 2 - Understand why a workload is scaled the way it is (Priority: P1)

An operator selects one autoscaled workload to understand which trigger is driving the current scale decision and why the current replica count sits where it does relative to the configured min/max bounds.

**Why this priority**: The list alone (User Story 1) only shows that autoscaling exists; the inspector is where the operator gets enough information to decide whether the behavior is expected (e.g., "why is this pinned at max replicas during business hours") without going to `kubectl describe`.

**Independent Test**: Can be fully tested by selecting a `ScaledObject`/`ScaledJob` row and confirming the Inspector shows the trigger type(s), the trigger metadata driving the decision, and an explanation of the current-vs-min/max relationship.

**Acceptance Scenarios**:

1. **Given** a `ScaledObject` with a single `prometheus` trigger with `threshold: 50` and `minReplicaCount: 1`/`maxReplicaCount: 10`, currently at 4 replicas, **When** the operator opens its Inspector, **Then** it shows the trigger type ("Prometheus"), the trigger's configured metadata (server address, query, threshold) as a generic key/value list, and a plain-language explanation such as "scaled above minimum because the trigger condition is active; below maximum because demand has not reached the configured threshold ceiling."
2. **Given** a `ScaledObject` with a `cron` trigger, **When** the operator opens its Inspector, **Then** it shows "Cron" as the trigger type and the cron schedule/timezone metadata as key/value pairs, without attempting to render a custom calendar widget.
3. **Given** a `ScaledObject` at exactly its `minReplicaCount` (or the paused/zero state when `minReplicaCount: 0`), **When** the operator opens its Inspector, **Then** the explanation clarifies it is at its floor because no trigger currently reports active demand (or KEDA has scaled to zero), rather than implying an error.
4. **Given** a `ScaledObject` at exactly its `maxReplicaCount`, **When** the operator opens its Inspector, **Then** the explanation clarifies it is capped at the configured ceiling and that demand may exceed what the maximum allows.
5. **Given** a `ScaledObject` with multiple triggers, **When** the operator opens its Inspector, **Then** all configured triggers are listed, with an indication of which trigger(s) currently report active/inactive status if that status is available from the cluster.

---

### User Story 3 - See live trigger metric values when metrics are available (Priority: P2)

An operator whose cluster already has Fathom's metrics stack (Prometheus) configured wants to see the actual live value behind a trigger (e.g., current queue depth, current Prometheus query result) next to its configured threshold, instead of only the static threshold.

**Why this priority**: This meaningfully upgrades the "why" explanation from User Story 2 with a live number, but the feature is fully useful without it — many clusters won't have the metrics stack configured, and this must never block the core read-only pane.

**Independent Test**: Can be fully tested by opening the Inspector for a trigger type whose live value Fathom's existing metrics integration can source, with the metrics stack configured, and confirming the live value renders alongside the threshold; then repeating with the metrics stack unavailable and confirming a graceful fallback with no error.

**Acceptance Scenarios**:

1. **Given** the metrics stack is configured and reachable, and a `ScaledObject` has a trigger type whose current value Fathom can source through that stack, **When** the operator opens its Inspector, **Then** the current live metric value is shown alongside the configured threshold (e.g., "current: 73 / threshold: 50").
2. **Given** the metrics stack is not configured, unreachable, or the trigger type is not one Fathom can source a live value for, **When** the operator opens the Inspector, **Then** only the trigger type and its configured threshold/metadata are shown, with no error, spinner-forever, or broken panel.
3. **Given** the live metric value fetch fails after the pane has already rendered, **When** the failure occurs, **Then** the pane continues showing the trigger-type-and-threshold view rather than replacing already-rendered content with an error state.

---

### User Story 4 - Trace a scaled workload's pods to its scaling history (Priority: P3)

An operator inspecting a pod that belongs to a KEDA-autoscaled workload wants a link from that pod (and from the existing pod-lifecycle timeline) to the `ScaledObject`/`ScaledJob` responsible, and vice versa, so they can correlate a pod's creation/termination with a scaling event.

**Why this priority**: This is a valuable connective enhancement that complements two existing shipped features (pod-lifecycle timeline, generic related-resource interlinking), but the KEDA pane itself is complete and useful without it, so it is the lowest-priority slice.

**Independent Test**: Can be fully tested by opening a pod belonging to a workload targeted by a `ScaledObject`/`ScaledJob` and confirming a related-resource link to that autoscaler appears, and by opening the autoscaler's Inspector and confirming its related-resources list includes the currently running pods of its target workload.

**Acceptance Scenarios**:

1. **Given** a pod belonging to a Deployment targeted by `ScaledObject` "queue-worker", **When** the operator inspects that pod, **Then** a related-resource link to "queue-worker" appears alongside the pod's other existing relationships (e.g., its owning Deployment, its Node).
2. **Given** the operator is viewing the pod-lifecycle timeline for a workload targeted by a `ScaledObject`, **When** they view the timeline, **Then** an indicator or link ties the timeline to the responsible `ScaledObject`, without requiring the pod-lifecycle feature to duplicate scaling logic.
3. **Given** the operator opens a `ScaledObject`'s Inspector, **When** they view its related resources, **Then** they see the target workload's currently running pods as clickable links, consistent with how other inspectors already expose related pods.

---

### Edge Cases

- What happens when the KEDA CRDs are not installed on the connected cluster? The Autoscalers section is still reachable but shows an honest empty state explaining that KEDA is not detected, rather than being hidden entirely or showing an error.
- What happens when the KEDA CRDs are installed but no `ScaledObject`/`ScaledJob` resources exist yet? The section shows an honest "no autoscalers configured" empty state, distinct from the "KEDA not installed" state.
- What happens when a `ScaledObject`/`ScaledJob` targets a workload that no longer exists (e.g., the Deployment was deleted but the `ScaledObject` remains)? The row still appears with an explicit "target not found" indication instead of erroring or being silently dropped.
- What happens when `TriggerAuthentication`/`ClusterTriggerAuthentication` resources are referenced by a trigger? Their existence is acknowledged (e.g., "uses authentication: <name>") without exposing secret values, since this feature is read-only and must not leak credential material.
- How does the system handle a `ScaledObject` using an external/unrecognized trigger type not explicitly covered by name (e.g., an obscure or custom scaler)? It still renders via the generic trigger-type + key/value metadata display rather than being omitted or crashing the pane.
- What happens when KEDA reports the workload paused (annotation-based pause) or scaled to zero? The explanation states plainly that the workload is paused or idled at zero, not that something is broken.
- How does the system handle many autoscalers (e.g., 200+ `ScaledObject`s)? The list remains usable (e.g., scrollable) without freezing the UI, consistent with how other resource lists in Fathom handle large counts.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST detect whether KEDA is installed on the connected cluster by checking for the presence of its CRDs (`keda.sh` API group: `ScaledObject`, `ScaledJob`, `TriggerAuthentication`, `ClusterTriggerAuthentication`), consistent with how Fathom already detects other optional CRD-based integrations (e.g., Flux).
- **FR-002**: The system MUST provide an "Autoscalers" tool-pane, consistent in placement and interaction with Fathom's other tool-panes (list pane + right-hand Inspector, no separate route/page).
- **FR-003**: When KEDA is not detected on the connected cluster, the Autoscalers pane MUST show an honest empty state explaining that KEDA is not installed, rather than an error or a silently empty list indistinguishable from "no autoscalers configured."
- **FR-004**: When KEDA is detected but no `ScaledObject`/`ScaledJob` resources exist, the pane MUST show a distinct "no autoscalers configured" empty state.
- **FR-005**: The Autoscalers list MUST display, per `ScaledObject`/`ScaledJob`: name, namespace, kind (ScaledObject/ScaledJob), target workload reference, current replica count, and configured min/max replica bounds.
- **FR-006**: The Autoscalers list MUST respect the same namespace-filter and multi-context behavior already applied to every other resource list in Fathom.
- **FR-007**: Selecting a `ScaledObject`/`ScaledJob` MUST open a read-only Inspector (no navigation away from the list) showing all of its configured triggers.
- **FR-008**: For each trigger, the Inspector MUST show the trigger type (e.g., queue depth, cron, Prometheus metric, or other) and its full configured metadata as a generic key/value display, without trigger-type-specific custom configuration UI.
- **FR-009**: The Inspector MUST show a plain-language explanation of why the current replica count sits where it does relative to the configured min/max bounds (e.g., at floor because no active demand, at ceiling because demand exceeds the configured maximum, paused, or scaled to zero).
- **FR-010**: When a trigger references a `TriggerAuthentication`/`ClusterTriggerAuthentication`, the Inspector MUST reference it by name without displaying any secret/credential values it resolves to.
- **FR-011**: When Fathom's existing metrics integration is configured and reachable, and the live current value for a trigger's underlying metric can be sourced through it, the Inspector MUST show that live value alongside the trigger's configured threshold.
- **FR-012**: When the metrics integration is not configured, unreachable, or does not support sourcing a live value for a given trigger type, the Inspector MUST fall back to showing only the trigger type and its configured threshold/metadata, without an error state.
- **FR-013**: A live metric value fetch failure occurring after the pane has rendered MUST NOT replace already-rendered trigger-type/threshold content with an error; the pane MUST continue showing what it already has.
- **FR-014**: When a `ScaledObject`/`ScaledJob`'s target workload no longer exists, its row/Inspector MUST show an explicit "target not found" indication rather than erroring or being silently omitted.
- **FR-015**: The `ScaledObject`/`ScaledJob` Inspector MUST show the target workload's currently running pods as clickable related-resource links, consistent with Fathom's existing generic related-resource interlinking mechanism.
- **FR-016**: A Pod inspector for a pod belonging to a workload targeted by a `ScaledObject`/`ScaledJob` MUST show a related-resource link to that `ScaledObject`/`ScaledJob`.
- **FR-017**: The existing pod-lifecycle timeline for a workload targeted by a `ScaledObject`/`ScaledJob` MUST surface a link/reference to the responsible autoscaler.
- **FR-018**: The Autoscalers list MUST remain usable (e.g., scrollable) with a large number of autoscalers, consistent with how other resource lists in Fathom handle large counts.
- **FR-019**: This feature MUST NOT provide any UI for creating or editing `ScaledObject`/`ScaledJob`/`TriggerAuthentication`/`ClusterTriggerAuthentication` resources; it is read-only for v1.
- **FR-020**: This feature MUST NOT provide trigger-type-specific configuration UI beyond the generic key/value metadata display (e.g., no custom cron calendar widget, no custom Prometheus query builder).

### Key Entities

- **Autoscaler (ScaledObject/ScaledJob)**: A KEDA resource describing how a target workload (Deployment, StatefulSet, or Job) is scaled. Attributes: name, namespace, kind, target workload reference, current replica count, min/max replica bounds, pause state, and its configured triggers.
- **Trigger**: A named scaling condition attached to an Autoscaler (e.g., queue depth, cron, Prometheus metric, or other). Attributes: trigger type, configured metadata (key/value pairs, e.g., threshold, query, schedule), optional reference to a Trigger Authentication, and, when available, a live current metric value and active/inactive status.
- **Trigger Authentication**: A `TriggerAuthentication`/`ClusterTriggerAuthentication` resource referenced by a trigger for authenticating to an external metric source. Attributes: name, scope (namespaced/cluster). Credential values are never displayed.
- **Scale Reasoning**: A derived, non-Kubernetes-native explanation of why an Autoscaler's current replica count relates to its min/max bounds as it does (e.g., at floor, at ceiling, mid-range and actively driven by a trigger, paused, or scaled to zero).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can identify every KEDA-autoscaled workload in the current namespace scope and its current replica count within 10 seconds of opening the Autoscalers pane, without running any CLI command.
- **SC-002**: An operator can determine which trigger is driving a given autoscaler's current scale decision, and why its replica count sits where it does relative to min/max, entirely from the Inspector, without needing to view raw YAML.
- **SC-003**: 100% of `ScaledObject`/`ScaledJob` resources present in the currently selected namespace scope appear in the Autoscalers list; none are silently dropped.
- **SC-004**: When the metrics stack is unavailable, the Inspector still renders complete trigger-type and threshold information for 100% of autoscalers, with zero error states shown for that reason alone.
- **SC-005**: An operator can navigate from an autoscaled workload's pod to its responsible autoscaler, and from the autoscaler to the workload's running pods, using only clicks within the Inspector, in 2 clicks or fewer each direction.
- **SC-006**: When KEDA is not installed on the connected cluster, 100% of operators see an explicit "KEDA not detected" message rather than an empty or broken pane.

## Assumptions

- Fathom already has read access to `keda.sh` API group resources once RBAC permits it; no new RBAC verbs beyond list/watch/get on these CRDs and their target workloads are required.
- "Current replica count" and min/max bounds are read directly from the `ScaledObject`/`ScaledJob` spec/status fields already reconciled by KEDA; Fathom does not compute scaling decisions itself.
- Fathom's existing metrics integration (from the Hubble/Prometheus integration work) is reused as-is for optional live trigger-value display; if it cannot readily source a given trigger type's live value, that trigger falls back to trigger-type + threshold display only, per the explicit non-hard-dependency requirement.
- The generic related-resource interlinking mechanism (introduced in the Applications feature) is reused to wire Autoscaler ↔ Pod ↔ target-workload links rather than building a parallel mechanism.
- The pod-lifecycle timeline integration is a lightweight reference/link addition, not a rework of that feature's existing data model.
- This feature targets the existing supported platforms (Ubuntu 26.04, Windows 11, macOS 26) with no new platform-specific behavior.
- KEDA versions in common use expose `status.currentReplicas` (or equivalent) sufficient to read the current replica count without Fathom needing to infer it from the target workload's own status.
