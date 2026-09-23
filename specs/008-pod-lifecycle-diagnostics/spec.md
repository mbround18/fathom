# Feature Specification: Pod Lifecycle Diagnostics

**Feature Branch**: `008-pod-lifecycle-diagnostics`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Pod Inspector 'Lifecycle' gaps identified in docs/MVP.md §7 (Workloads → Pod Inspector → Lifecycle, marked 'Partial'): init containers listed distinctly from app containers, container lifecycle hooks (postStart/preStop) with hook-failure correlation from Events, liveness/readiness/startup probe configuration correlated with current probe-failure state, per-container restart history (count, last termination reason, exit code, last-terminated timestamp), and a pod Events timeline inside the inspector — all read-only, additive to the existing Pod Inspector (Metadata, Status, Spec, Labels, Annotations, Related sections already shipped in ui/src/resources/pods.tsx)."

## User Scenarios & Testing *(mandatory)*

<!--
  IMPORTANT: User stories should be PRIORITIZED as user journeys ordered by importance.
  Each user story/journey must be INDEPENDENTLY TESTABLE - meaning if you implement just ONE of them,
  you should still have a viable MVP (Minimum Viable Product) that delivers value.

  Assign priorities (P1, P2, P3, etc.) to each story, where P1 is the most critical.
  Think of each story as a standalone slice of functionality that can be:
  - Developed independently
  - Tested independently
  - Deployed independently
  - Demonstrated to users independently
-->

### User Story 1 - Tell init-container failures apart from app-container failures (Priority: P1)

An operator opens a Pod that is stuck in `Init:Error` or `Init:CrashLoopBackOff` and needs to see,
at a glance, which init container is failing, its own status and exit code, and its own logs —
without it being mixed into the same list as the pod's regular (app) containers, which are not
even running yet.

**Why this priority**: Init container failures are one of the most common "pod never starts"
causes, and today the Pod Inspector's Spec section lists every container name — init and app —
in one undifferentiated list with no status, so the operator cannot tell which container the
problem is even in without leaving Fathom to run `kubectl describe pod`.

**Independent Test**: Deploy a Pod with two init containers (one that succeeds, one that exits
non-zero) and one app container that never starts as a result. Open the Pod Inspector and confirm
init containers appear in a section distinct from app containers, each showing its own
run/waiting/terminated status and, for the failing one, its exit code — and that its logs are
reachable the same way an app container's logs are.

**Acceptance Scenarios**:

1. **Given** a Pod with one or more `spec.initContainers` entries, **When** the operator opens
   that Pod's inspector, **Then** the init containers are shown in a section separate from the
   pod's regular containers, each labeled as an init container.
2. **Given** an init container that has completed successfully, **When** the operator views the
   init containers section, **Then** its status is shown as completed/terminated with exit code 0.
3. **Given** an init container that exited with a non-zero code, **When** the operator views the
   init containers section, **Then** its status clearly shows it failed and its exit code is
   visible, and its logs are reachable via the same log-viewing mechanism already used for app
   containers.
4. **Given** a Pod with no `spec.initContainers`, **When** the operator opens that Pod's inspector,
   **Then** no init containers section (or an explicit "none" state) is shown — its absence is not
   presented as an error.

---

### User Story 2 - See probe configuration and current probe-failure state per container (Priority: P1)

An operator investigating a Pod that is `Running` but not `Ready` wants to know, per container,
what liveness/readiness/startup probes are configured and whether one of them is currently
failing — instead of only seeing the pod-level `Ready` condition, which doesn't say which
container or which probe is the cause.

**Why this priority**: "Running but not Ready" is one of the most common operator investigations
in Kubernetes, and today Fathom shows only the aggregate pod-level Ready state, giving no path to
the actual cause without leaving the app.

**Independent Test**: Deploy a Pod with a container whose `readinessProbe` is deliberately
misconfigured to fail (e.g. wrong path). Open the Pod Inspector and confirm the container's
configured probes (type, thresholds, period) are shown, and that the readiness probe is visibly
flagged as currently failing, distinct from a healthy probe on another container.

**Acceptance Scenarios**:

1. **Given** a container with a `livenessProbe`, `readinessProbe`, and/or `startupProbe`
   configured, **When** the operator views that container's detail, **Then** each configured
   probe's type (HTTP GET, TCP socket, exec command, or gRPC), threshold counts, and period are
   shown.
2. **Given** a container whose readiness probe is currently failing, **When** the operator views
   that container's detail, **Then** the readiness probe is visibly distinguished as failing
   (not just the pod-level `Ready` condition), consistent with how failed/unhealthy states are
   shown elsewhere in Fathom.
3. **Given** a container with no probes configured, **When** the operator views that container's
   detail, **Then** an explicit "no probes configured" state is shown rather than an empty or
   missing section.
4. **Given** all of a container's configured probes are currently passing, **When** the operator
   views that container's detail, **Then** each probe is shown in a clearly healthy state.

---

### User Story 3 - See per-container restart history instead of only the pod-level count (Priority: P2)

An operator looking at a Pod with a nonzero restart count wants to know, per container, why it
last restarted (reason, exit code) and when — instead of only the single pod-level restart number
already shown, which doesn't say which container restarted or why.

**Why this priority**: Multi-container Pods routinely have only one container flapping; today's
pod-level restart count conflates all containers, forcing a `kubectl describe` to find the actual
offender. This is valuable but secondary to the init-container and probe stories, which surface
init-time failures a restart count won't show.

**Independent Test**: Deploy a Pod with two containers where one has crashed and restarted at
least once (e.g. via a command that exits 1). Open the Pod Inspector and confirm the crashed
container shows its own restart count, last termination reason, exit code, and last-terminated
timestamp, distinct from the healthy container's restart history.

**Acceptance Scenarios**:

1. **Given** a container that has restarted at least once, **When** the operator views that
   container's detail, **Then** its restart count, last termination reason (e.g. `Error`,
   `OOMKilled`), exit code, and last-terminated timestamp are shown.
2. **Given** a container that has never restarted, **When** the operator views that container's
   detail, **Then** its restart count is shown as zero and no stale "last termination" data is
   displayed.
3. **Given** a Pod with two containers where only one has restarted, **When** the operator views
   the Pod Inspector, **Then** the two containers' restart histories are clearly attributed to the
   correct container, not conflated.

---

### User Story 4 - See configured lifecycle hooks and whether they failed (Priority: P2)

An operator investigating a slow or stuck rollout wants to know whether a container has a
`postStart` or `preStop` hook configured and, if so, whether that hook actually ran and failed —
a failure mode that is currently invisible anywhere in Fathom and can silently block a rollout or
delay pod termination.

**Why this priority**: Hook failures are a real but less frequent cause of stuck rollouts than
init-container or probe failures, and diagnosing them today requires cross-referencing the
cluster-wide Events list by hand.

**Independent Test**: Deploy a container with a `postStart` hook that is deliberately made to
fail. Open the Pod Inspector and confirm the hook's configuration is shown and that a hook-failure
indication (sourced from the Pod's Events) appears attached to that container.

**Acceptance Scenarios**:

1. **Given** a container with a `postStart` and/or `preStop` hook configured, **When** the
   operator views that container's detail, **Then** the configured hook(s) are shown (their type —
   exec command, HTTP GET, or sleep — and target).
2. **Given** a container whose `postStart` hook failed, **When** the operator views that
   container's detail, **Then** a hook-failure indication is shown, correlated from the Pod's
   Events (e.g. a `FailedPostStartHook` event).
3. **Given** a container with no lifecycle hooks configured, **When** the operator views that
   container's detail, **Then** no hook section (or an explicit "none configured" state) is shown.
4. **Given** a hook that ran without failing, **When** the operator views that container's detail,
   **Then** the hook's configuration is shown without any failure indication.

---

### User Story 5 - See the Pod's own Events as a timeline inside the inspector (Priority: P2)

An operator on any Pod's inspector wants to see that Pod's own scheduling/image-pull/probe/hook
events in chronological order without leaving the inspector to manually filter the cluster-wide
Events list by the Pod's name.

**Why this priority**: This is the connective layer that makes Stories 2–4's hook- and
probe-failure correlations visible in context, and stands alone as a usability improvement even
before those other stories are read — but it depends on nothing being broken to be useful, unlike
the failure-diagnosis stories above, so it is ranked after them.

**Independent Test**: Open the inspector of a Pod that has generated several Events (e.g.
`Scheduled`, `Pulling`, `Pulled`, `Started`). Confirm all of them appear inside the Pod Inspector
in chronological order, without navigating to the separate cluster-wide Events view.

**Acceptance Scenarios**:

1. **Given** a Pod that has one or more Kubernetes Events whose involved object is that Pod,
   **When** the operator views the Pod Inspector, **Then** those Events are shown as a
   chronological timeline within the inspector, each with its reason, message, type
   (Normal/Warning), and timestamp.
2. **Given** a Pod with no Events (or none within the retained window), **When** the operator
   views the Pod Inspector, **Then** an explicit "no events" state is shown, not an empty or
   missing section.
3. **Given** new Events arrive for a Pod currently open in the inspector, **When** the operator is
   still viewing it, **Then** the timeline updates to include them, consistent with how the rest
   of Fathom's inspector reflects live cluster state.
4. **Given** the operator has the cluster-wide Events list open in another part of the app,
   **When** they open a Pod's inspector, **Then** the Pod Inspector's Events timeline shows the
   same underlying Event objects filtered to that Pod, not a separately-fetched or inconsistent
   copy.

---

### Edge Cases

- A container has zero configured probes, zero lifecycle hooks, and zero restarts: its detail
  shows explicit "not configured" / "zero" states for each, never an empty gap that could be
  mistaken for missing data.
- An init container is still running (a slow, long-running init container): it is shown as
  currently running, not as failed or completed, and its logs are reachable while it runs.
- A Pod has `restartPolicy: Never` and a container has terminated (success or failure) without
  restarting: `ContainerStatus.lastState` may be empty even though the container has stopped —
  the current/last state is still shown from `ContainerStatus.state`, not presented as "no data."
- A probe's `exec` command or `httpGet` path is long or contains sensitive-looking values (e.g. a
  path with a query string): it is shown verbatim and read-only, consistent with Fathom's existing
  practice of never redacting operator-visible spec content that isn't a credential.
- The Pod's Events include ones whose `reason` isn't one Fathom specifically recognizes (i.e. not
  one of Scheduled/Pulling/Pulled/Started/FailedPostStartHook/FailedPreStopHook/Unhealthy/
  BackOff): the event is still shown in the timeline with its raw reason/message rather than being
  filtered out, so the timeline is never silently incomplete.
- A Pod is deleted while its inspector is open with this data visible: the inspector falls back to
  the existing "resource no longer exists" handling already used by every other resource kind,
  rather than a partial/frozen lifecycle panel.
- A container's restart count is very high (e.g. thousands, from a long-lived `CrashLoopBackOff`):
  the count is shown as-is; only the *last* termination's reason/exit code/timestamp are shown,
  not a full history of every past restart (Kubernetes itself does not retain that).

## Requirements *(mandatory)*

<!--
  ACTION REQUIRED: The content in this section represents placeholders.
  Fill them out with the right functional requirements.
-->

### Functional Requirements

- **FR-001**: The Pod Inspector MUST show init containers (from `spec.initContainers`) in a
  section distinct from the pod's regular (app) containers, each labeled as an init container.
- **FR-002**: Each init container MUST show its current status (waiting/running/terminated) and,
  when terminated, its exit code and termination reason.
- **FR-003**: Each init container's logs MUST be reachable via the same log-viewing mechanism
  already used for the pod's app containers.
- **FR-004**: A Pod with no init containers MUST show an explicit empty/absent state for this
  section rather than omitting it silently or presenting it as an error.
- **FR-005**: The Pod Inspector MUST show, per (app and init) container, the configuration of any
  `livenessProbe`, `readinessProbe`, and `startupProbe` present on that container: probe type
  (HTTP GET, TCP socket, exec command, or gRPC), failure/success threshold counts, and period.
- **FR-006**: For each configured probe, the Pod Inspector MUST show the probe's current
  passing/failing state, derived from the container's current status and/or the Pod's Events, not
  only the aggregate pod-level `Ready`/`ContainersReady` condition.
- **FR-007**: A container with no probes configured MUST show an explicit "no probes configured"
  state rather than an empty section.
- **FR-008**: The Pod Inspector MUST show, per container, its restart count, last termination
  reason, last exit code, and last-terminated timestamp, sourced from
  `ContainerStatus.restartCount` and `ContainerStatus.lastState`.
- **FR-009**: A container with a restart count of zero MUST show that explicitly and MUST NOT show
  stale or placeholder "last termination" data.
- **FR-010**: The Pod Inspector MUST show, per container, any configured `postStart` and/or
  `preStop` lifecycle hook, including the hook's type (exec command, HTTP GET, or sleep) and
  target.
- **FR-011**: When a container's lifecycle hook has failed, the Pod Inspector MUST show a
  hook-failure indication for that container, correlated from the Pod's own Events (e.g. a
  `FailedPostStartHook`/`FailedPreStopHook` event).
- **FR-012**: A container with no lifecycle hooks configured MUST show an explicit "none
  configured" state rather than an empty section.
- **FR-013**: The Pod Inspector MUST show the Pod's own Events (Kubernetes `Event` objects whose
  involved object is that Pod) as a chronological timeline within the inspector, each entry
  showing its reason, message, type (Normal/Warning), and timestamp.
- **FR-014**: A Pod with no Events available MUST show an explicit "no events" state within the
  timeline rather than an empty or missing section.
- **FR-015**: The Events timeline MUST update to reflect new Events for the currently-open Pod
  while its inspector remains open, consistent with how the rest of Fathom's inspector reflects
  live state.
- **FR-016**: The Events timeline MUST source from the same underlying Event data already used by
  Fathom's cluster-wide Events list (filtered to the open Pod), not a separately-fetched or
  potentially inconsistent copy.
- **FR-017**: All data introduced by this feature MUST be read-only: this feature MUST NOT
  introduce any way to edit, retry, delete, or otherwise mutate a probe, hook, container, or init
  container from the Pod Inspector.
- **FR-018**: All new sections introduced by this feature (init containers, probes, per-container
  restart history, lifecycle hooks, Events timeline) MUST be additive to the Pod Inspector's
  existing Metadata, Status, Spec, Labels, Annotations, and Related sections — none of those
  existing sections' current content is removed or restructured by this feature.
- **FR-019**: An Event whose `reason` is not one Fathom specifically recognizes for hook/probe
  correlation MUST still appear in the Events timeline with its raw reason and message, rather
  than being filtered out.

### Key Entities *(include if feature involves data)*

- **Init Container**: A container from `spec.initContainers` that runs to completion before the
  Pod's app containers start — has a name, image, current status (waiting/running/terminated),
  and, once terminated, an exit code and reason; distinct from an app container in that it is not
  expected to remain running.
- **Container Probe**: A `livenessProbe`, `readinessProbe`, or `startupProbe` configured on a
  container — has a type (HTTP GET/TCP socket/exec/gRPC), a target (path/port/command), threshold
  counts, a period, and a current passing/failing state as of the last check.
- **Container Lifecycle Hook**: A `postStart` or `preStop` handler configured on a container — has
  a type (exec/HTTP GET/sleep), a target, and — derived from the Pod's Events, not the spec — an
  observed ran/failed outcome.
- **Container Restart Record**: The most recent termination of a specific container within a Pod —
  has a restart count, a termination reason, an exit code, and a last-terminated timestamp; one
  record exists per container, not per historical restart.
- **Pod Event**: A Kubernetes `Event` object whose involved object is the open Pod — has a reason,
  a human-readable message, a type (Normal/Warning), and a timestamp; the same underlying entity
  already shown in Fathom's cluster-wide Events list, filtered by involved object.

## Success Criteria *(mandatory)*

<!--
  ACTION REQUIRED: Define measurable success criteria.
  These must be technology-agnostic and measurable.
-->

### Measurable Outcomes

- **SC-001**: An operator can identify which specific init container caused a Pod to be stuck in
  an `Init:` state, and see its exit code, within one inspector view — no separate CLI lookup.
- **SC-002**: An operator can identify which specific container and which specific probe is
  causing a `Running`-but-not-`Ready` Pod, within one inspector view — no separate CLI lookup.
- **SC-003**: An operator can determine which container in a multi-container Pod is responsible
  for that Pod's restart count, and why it last restarted, within one inspector view.
- **SC-004**: An operator can determine whether a stuck rollout is caused by a failing
  `postStart`/`preStop` hook without manually cross-referencing the cluster-wide Events list.
- **SC-005**: 100% of a Pod's own Events are visible from within that Pod's inspector, in
  chronological order, without navigating away from it.
- **SC-006**: None of the five new capabilities in this feature introduce a way to change cluster
  state — a review of the shipped feature finds zero new mutating actions.

## Assumptions

<!--
  ACTION REQUIRED: The content in this section represents placeholders.
  Fill them out with the right assumptions based on reasonable defaults
  chosen when the feature description did not specify certain details.
-->

- **Per-container restart history is "last state only," not a full log of every restart.**
  Kubernetes itself only retains `ContainerStatus.lastState` (the single most recent prior
  termination) via the API, not an arbitrary-length history; a full restart history would require
  Fathom to persist its own record over time, which is out of scope for this feature. Resolved as
  an assumption rather than a `[NEEDS CLARIFICATION]` marker because the underlying Kubernetes API
  makes this the only reasonable default.
- **Probe-failure correlation is best-effort, sourced from `ContainerStatus` plus Events, not a
  new polling mechanism.** Kubernetes does not expose a dedicated "current probe failure state"
  field; the closest available signals are the container's readiness/liveness reflected in
  `ContainerStatus` and `Unhealthy`-reason Events. This feature treats those as sufficient and does
  not add active probe execution or synthetic health-checking from Fathom itself.
- **The Events timeline reuses Fathom's existing cluster-events plumbing** (the same
  `RefreshEvents` command and involved-object filtering already used by the cluster-wide Events
  list and referenced by other inspectors), rather than introducing a second events-fetching path.
- **Hook-failure detection depends on the Event still being present in the cluster's retained
  Events window.** Kubernetes Events are garbage-collected (default ~1 hour); a hook failure whose
  Event has already expired will show the hook's configuration but no failure indication. This is
  treated as a boundary condition of the underlying data source, not a gap in this feature.
- **This feature extends only the Pod kind's inspector**, not the generic `WorkloadDetails` shared
  by Deployments/StatefulSets/DaemonSets/etc. — those workload kinds already drill into their
  owned Pods (via the existing `PodLink` mechanism in `ui/src/resources/workloads.tsx`), where this
  feature's data becomes visible once the operator opens a specific Pod.
- **Existing Pod Inspector sections (Metadata, Status, Spec, Labels, Annotations, Related) are
  out of scope for modification.** This feature only adds new sections/data; it does not restructure
  or remove any currently-shipped section.
