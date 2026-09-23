# Feature Specification: Port Forward Interface

**Feature Branch**: `009-port-forward-ui`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the frontend panel for Fathom's Port Forward Interface (docs/MVP.md §10). The backend (`PortForwardManager` in `src/backend/portforward.rs`) and its full protocol surface (start/stop/delete/update/refresh commands, port-availability check, live list and per-tunnel status events) already ship and are already used internally by the Prometheus and Hubble integrations. A `Port Forwards` nav entry, generated protocol types, and a store slice already exist but render nothing — the tool pane falls back to the generic empty state today. This is the same shape of gap Flux and Hubble were in before their panels landed. Scope: one-click forward from a Service/Pod inspector, a live list with stop/restart controls, a local port picker with conflict detection, a copy-address button, and correctly reflecting the existing context-scoped teardown lifecycle. Non-goals: generic TCP/SSH tunnels, non-localhost binding, bulk operations, any change to the backend forwarding/persistence logic, and a dedicated auto-start scheduling UI beyond surfacing the existing `auto_start` field."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Start a port forward from a resource inspector (Priority: P1)

An operator is inspecting a Service or Pod, wants to reach it locally (e.g. to hit an API, connect `psql`, or open a dashboard), and starts a forward without leaving the inspector or having to already know an open local port.

**Why this priority**: This is the entry point the whole feature exists to serve — the backend already reliably forwards traffic, but today there is no discoverable way for an operator to trigger it at all. Without this, the rest of the panel has nothing to manage.

**Independent Test**: Can be fully tested by opening a Pod or Service inspector, invoking "Forward" on a specific port, confirming the action, and observing a new entry appear in the forwards list with an `Active` tunnel status and a bound local port — without touching the stop/restart or port-picker conflict paths.

**Acceptance Scenarios**:

1. **Given** a Pod inspector showing its containers' ports, **When** the operator selects "Forward" on a specific container port, **Then** a confirmation step appears naming the exact namespace, pod, container, and target port before anything is dispatched to the backend.
2. **Given** a Service inspector showing its ports, **When** the operator selects "Forward" on a specific service port, **Then** the system resolves a backing pod for that service and the same named confirmation step appears before dispatch (mirrors scenario 1, service-to-pod resolution is transparent to the operator).
3. **Given** the confirmation step is showing with a suggested local port, **When** the operator confirms, **Then** `StartPortForward` is dispatched, the entry appears in the forwards list with status `Connecting` and then `Active` once the backend confirms, and no cluster/tunnel state is shown as active before the backend has reported it.
4. **Given** the confirmation step is showing, **When** the operator cancels, **Then** no command is dispatched and no forward is created.
5. **Given** the backend reports an error while starting (e.g. bind failure, pod unreachable), **When** this occurs, **Then** the operator sees the backend's error message verbatim, and no forward entry is left in a misleadingly active state.

---

### User Story 2 - Manage the live list of active forwards (Priority: P1)

An operator has one or more forwards running and needs to see their status at a glance, stop a forward they no longer need, or restart one that has dropped into an error state — all from a single list scoped to the current cluster context.

**Why this priority**: Once a forward can be started, an operator immediately needs visibility and control over it; a start-only panel without list management would leave forwards running invisibly and unmanageable, which is worse than not having the panel. This is co-equal in priority to User Story 1 because the two together are the minimum useful slice.

**Independent Test**: Can be fully tested by seeding the fake backend with two or three forwards in different states (`Connecting`, `Active`, `Error`), opening the Port Forwards panel, and verifying each row shows local port, target namespace/pod/port, and live status, then exercising stop and restart per row — without depending on the inspector-launch flow from User Story 1.

**Acceptance Scenarios**:

1. **Given** the backend has active forwards for the current cluster context, **When** the operator opens the Port Forwards panel, **Then** every forward is listed showing local port, target namespace, target pod (and originating service, when applicable), target/remote port, and current tunnel status.
2. **Given** a forward is listed with status `Active`, **When** the operator selects "Stop" and confirms, **Then** `StopPortForward` is dispatched, the row updates to `Closed` only after the backend confirms, and the local port becomes available again.
3. **Given** a forward is listed with status `Error` or `Closed`, **When** the operator selects "Restart", **Then** the operator is asked to confirm re-establishing the forward (same namespace/pod/port), and on confirmation a new tunnel is started reusing the same local port when it is still free, or prompting for a new one via the port picker (User Story 3) when it is not.
4. **Given** the backend emits a `PortForwardTunnel` status update (e.g. `Active` → `Error` because the pod restarted), **When** this event arrives, **Then** the corresponding row's status updates live without a manual refresh.
5. **Given** the operator switches Kubernetes context, **When** the switch completes, **Then** the forwards list shows only forwards for the new context (the prior context's forwards were already torn down by the backend) — no stale rows persist across the switch.
6. **Given** the app is about to exit while forwards are active, **When** shutdown proceeds, **Then** the UI does not block or warn the operator about losing forwards beyond what the app's normal exit behavior already provides (backend teardown on exit is a given, not something the UI needs to confirm separately).

---

### User Story 3 - Pick a local port with conflict detection (Priority: P2)

An operator starting a forward is offered a sensible default local port but can choose a different one, and is warned immediately if the port they pick (or the app's own suggestion) is already taken on their machine, before they attempt to connect a client to it.

**Why this priority**: This materially improves the reliability of User Story 1 (avoids a confusing silent bind failure) but the forward-start flow can ship with a naive default-port suggestion and still deliver value; conflict detection is the refinement, not the entry point.

**Independent Test**: Can be fully tested by occupying a local port outside the app (e.g. binding it with a throwaway listener), opening the forward-start flow, entering that port, and verifying the system surfaces a conflict warning via `CheckPortAvailability`/`PortAvailability` before the operator can confirm — independent of which resource type triggered the flow.

**Acceptance Scenarios**:

1. **Given** the forward-start flow is open, **When** it loads, **Then** the local port field is pre-filled with a suggested port (e.g. the target/remote port when free, otherwise the next available port) that has already been checked for availability.
2. **Given** the operator edits the local port field to a value that is currently bound by another process, **When** the value settles (e.g. on blur or debounce), **Then** `CheckPortAvailability` is dispatched and the field shows a clear "port in use" warning before the operator can confirm.
3. **Given** the local port field shows a conflict warning, **When** the operator changes it to a free port, **Then** the warning clears and the confirm action becomes available.
4. **Given** the operator confirms despite an unresolved conflict warning (e.g. race between check and bind), **When** the backend actually fails to bind, **Then** the backend's verbatim bind error is shown (per User Story 1 Scenario 5) rather than the UI claiming success.

---

### User Story 4 - Copy the local address for use elsewhere (Priority: P3)

An operator with an active forward wants to quickly copy `localhost:<port>` to paste into a browser tab, `psql`, `curl`, or another client tool, without retyping or misreading the port from the list.

**Why this priority**: A pure convenience affordance layered on top of an already-functional list; valuable but not blocking for the panel to be useful.

**Independent Test**: Can be fully tested by opening the forwards list with at least one `Active` entry, selecting its copy-address action, and verifying the system clipboard contains exactly `localhost:<local_port>` — independent of the start/stop/conflict flows.

**Acceptance Scenarios**:

1. **Given** an active forward row, **When** the operator selects the copy-address action, **Then** `localhost:<local_port>` is copied to the clipboard and the operator receives a brief confirmation that the copy succeeded.
2. **Given** a forward row that is not yet `Active` (e.g. `Connecting`), **When** the operator views the row, **Then** the copy-address action is either disabled or copies the address anyway with a note that the tunnel is not yet ready (system MUST NOT silently copy a non-functional address without indicating tunnel state — the tone/wording is an implementation choice, but the state must be visible).

---

### Edge Cases

- What happens when an operator tries to start a forward for a pod/service that has no discoverable exposed port (e.g. no `containerPort` declared)? The "Forward" action MUST let the operator specify an arbitrary target port manually rather than being blocked, since kube's pod-portforward works against any port the container listens on regardless of whether it is declared.
- What happens when a Service's backing pods are not yet ready or none currently exist? The forward-start flow MUST surface a clear error (from the backend's own resolution failure) rather than silently forwarding to a nonexistent pod or picking an arbitrary unrelated pod.
- What happens when two operators-in-one (two browser/UI instances, or a UI restart) both try to claim the same local port at nearly the same time? The `CheckPortAvailability` check is a point-in-time hint, not a lock; the authoritative outcome is always the backend's actual bind result, and a late bind failure MUST surface the verbatim backend error (consistent with User Story 1 Scenario 5 and User Story 3 Scenario 4).
- What happens to a forward's row when its target pod is deleted while the tunnel is active? The tunnel status MUST transition to `Error` (reflecting the backend's own connection failure) rather than silently disappearing from the list — the operator needs to see and explicitly stop/delete it.
- What happens when the operator navigates away from the Port Forwards panel while a forward is active? The forward MUST keep running (it is backend-owned, not tied to panel visibility) and its state MUST still be reflected correctly when the operator returns to the panel.
- What happens on first load of the panel with zero forwards in the current context? The panel MUST show a clear empty state (distinct from the generic tool-pane fallback it shows today) that explains how to start a forward (e.g. pointing at the Pod/Service inspector action), not just a blank list.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a one-click "Forward" action on Pod and Service inspectors that lets the operator pick a target port (from the resource's declared ports, or a manually entered port for pods/services with none declared) without requiring a separate standalone form as the only entry point.
- **FR-002**: System MUST require explicit operator confirmation, naming the target namespace, pod (and container, when applicable), and remote port, before dispatching `StartPortForward` — consistent with the app's confirmation pattern for mutating actions.
- **FR-003**: System MUST render a live list of all port forwards for the current cluster context, each row showing local port, target namespace, target pod (and originating service when the forward was started from a Service), target/remote port, and current tunnel status, sourced from `ResourceListEvent::PortForwardsUpdated` and kept current via `StreamEvent::PortForwardTunnel` status events.
- **FR-004**: System MUST provide a "Stop" control per active/connecting forward row that requires explicit confirmation naming the forward's target before dispatching `StopPortForward`, and MUST NOT mark the row as stopped until the backend confirms.
- **FR-005**: System MUST provide a "Restart" control per forward row in `Error` or `Closed` status that re-establishes the same namespace/pod/port forward (via a fresh `StartPortForward`), running it through the same confirmation and port-conflict-detection paths as a new forward.
- **FR-006**: System MUST offer a local port picker as part of the forward-start flow that pre-fills a suggested port and checks it (and any operator-entered port) for local availability via `CheckPortAvailability`/`PortAvailability` before the operator can confirm, surfacing a clear in-context conflict warning rather than allowing a blind attempt.
- **FR-007**: System MUST provide a copy-address action per forward row that copies `localhost:<local_port>` to the clipboard and confirms the copy to the operator.
- **FR-008**: System MUST scope the rendered forwards list to the current cluster context and MUST NOT display forwards from a prior context after a context switch (relying on the backend's existing per-context teardown, not reimplementing it).
- **FR-009**: System MUST surface backend errors from any port-forward command (start, stop, restart, port-check) verbatim to the operator, never replaced with a generic message.
- **FR-010**: System MUST NOT show a forward as `Active` in the UI until the backend has reported that status; UI state MUST always trail confirmed backend state, never anticipate it.
- **FR-011**: System MUST show a dedicated empty state on the Port Forwards panel when no forwards exist in the current context, distinct from the app's generic tool-pane fallback, that explains how to start one.
- **FR-012**: System MUST reflect a forward's `auto_start` flag in the list (e.g. as a badge/indicator) using the existing `UpdatePortForward` command if the panel allows toggling it, but MUST NOT introduce new auto-start scheduling behavior beyond surfacing/toggling this existing field.
- **FR-013**: System MUST use only the existing generated protocol commands and events (`StartPortForward`, `StopPortForward`, `DeletePortForward`, `UpdatePortForward`, `RefreshPortForwards`, `CheckPortAvailability`, `ResourceListEvent::PortForwardsUpdated`, `StreamEvent::PortForwardTunnel`) and MUST NOT implement any local port-binding, forwarding, or persistence logic in the frontend.

### Key Entities

- **Port Forward (`PortForward`)**: A tracked tunnel — id, namespace, pod, local port, remote port, optional container, persisted status, and `auto_start` flag. Owned and persisted by the backend; the UI only renders and requests changes to it.
- **Tunnel Status (`TunnelStatus`)**: The live connection state of a specific tunnel (`Connecting`/`Active`/`Error`/`Closed`), streamed independently of the persisted `PortForward.status` and used to drive the list row's real-time indicator.
- **Forward Target**: The Pod, Container, or Service a forward originates from in the inspector-launch flow (User Story 1) — not a new backend concept, just the UI's framing of which existing resource inspector triggered the action.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can go from viewing a Pod or Service inspector to a confirmed, active local forward in three or fewer interactions (open action, confirm port, confirm start).
- **SC-002**: 100% of forward status changes reported by the backend (`Connecting`→`Active`→`Error`/`Closed`) are reflected in the list within one UI render frame of the corresponding event, with no manual refresh required.
- **SC-003**: An operator attempting to use an already-occupied local port is warned before confirming the forward in at least 95% of cases where the conflict is detectable ahead of time (i.e., excluding true races that only the backend's own bind attempt can catch).
- **SC-004**: Zero forwards from a previous cluster context are ever visible in the list after a context switch completes.
- **SC-005**: Copying a forward's address succeeds and matches the displayed `localhost:<port>` value in 100% of attempts on an `Active` forward.
- **SC-006**: No forward's UI-displayed status ever shows `Active` before the backend has actually confirmed the tunnel is active (verified by boundary tests injecting out-of-order/delayed events).

## Assumptions

- **Service-to-pod resolution for "Forward" from a Service inspector** (User Story 1, Scenario 2) is resolved as: the UI selects one ready backing pod (consistent with how the app already resolves "exec into a Service" or similar existing per-resource actions elsewhere, if such a pattern exists) and forwards to that pod's matching container port; if the app has no existing service-to-pod convention, the panel presents the resolved pod's identity in the confirmation step so the operator knows exactly what they're targeting. This was a [NEEDS CLARIFICATION] candidate, resolved autonomously in favor of transparent single-pod resolution rather than blocking the story on multi-pod selection UI, since MVP.md frames this as mirroring existing single-target inspector actions.
- **Default/suggested local port** (User Story 3, Scenario 1) defaults to the same number as the remote/target port when that local port is free, falling back to the next free ephemeral-range port otherwise. This was resolved autonomously as the least-surprising default (matching ports is the common operator expectation, e.g. forwarding a Service's `5432` to local `5432`), rather than adding a settings-level configurable default in this feature's scope.
- **Restart semantics** (User Story 2, Scenario 3) mean a fresh `StartPortForward` against the same namespace/pod/port after the previous tunnel is confirmed torn down — Helm-mutate-style "retry the recorded target" rather than resuming any in-flight connection state, since the backend has no resume concept and none is implied by MVP.md.
- Cluster-context-scoped teardown (list no longer showing prior-context forwards, and clean shutdown on app exit) is fully implemented at the backend/persistence level already; this feature's only obligation is correct rendering of that state, not reimplementing teardown logic.
- The forwards panel is a single, non-bulk, non-multi-cluster-context view — consistent with MVP.md's non-goal framing and this project's precedent of scoping tightly per feature (e.g. 006-helm-mutate excluding bulk operations).
- Authentication/RBAC for whether an operator is allowed to port-forward a given pod is enforced entirely by the cluster (via the same kubeconfig/RBAC context the rest of the app already respects, per constitution Principle VI) — the UI does not add its own permission logic beyond surfacing whatever error the backend/cluster returns.
