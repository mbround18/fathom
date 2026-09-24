# Feature Specification: Ephemeral Debug Containers

**Feature Branch**: `025-ephemeral-debug-containers`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Ephemeral debug containers (kubectl debug-style) — a new mutating command that adds an ephemeral container to a running pod (Kubernetes' pods/ephemeralcontainers subresource, AddEphemeralContainer-equivalent), for debugging a pod without a shell (e.g. distroless images) by attaching a debug image (e.g. busybox/nicolaka/netshoot) that shares the target container's process namespace. This is distinct from and complements the sibling spec 024-pod-exec-terminal (being built concurrently) — once the ephemeral container is added and running, exec into it can reuse that spec's terminal/exec mechanism if it lands first; if not, note the dependency clearly in this spec's plan and design this feature's own minimal read/status path so it's independently useful even before exec support exists (e.g. showing the ephemeral container's logs via the existing log-viewing mechanism). Non-goals: this is a genuine cluster mutation (adding a container) — gate it through mutation.rs's existing guard/confirmation pattern like every other mutating action, with a clear warning that ephemeral containers cannot be removed once added. No ephemeral-container removal action. No custom debug-image catalog UI — a plain text field for image name is sufficient for v1."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Attach a debug container to a running pod (Priority: P1)

An operator is investigating a pod running a distroless (shell-less) image and cannot `kubectl exec` into it to inspect the filesystem or running processes. From the pod detail view, they choose to add a debug container, pick (or type) a debug image such as `busybox` or `nicolaka/netshoot`, select which existing container's process namespace to share, and confirm the mutation after being warned that the container cannot later be removed. Fathom adds the ephemeral container to the pod via the Kubernetes API, and the pod's container list updates to show the new ephemeral container and its status (waiting/running/terminated).

**Why this priority**: This is the core value of the feature — without it, users with distroless/minimal images have no way to debug a live pod through Fathom at all. Every other behavior in this spec exists to support this flow.

**Independent Test**: Can be fully tested by opening a running pod's detail view, launching "Add debug container," entering an image name and target container, confirming through the guarded dialog, and observing the new ephemeral container appear in the pod's container list with a status that updates as it starts.

**Acceptance Scenarios**:

1. **Given** a running pod with one or more containers, **When** the user opens "Add debug container" and submits a debug image name and target container, **Then** a confirmation dialog appears warning that ephemeral containers cannot be removed once added.
2. **Given** the confirmation dialog is showing, **When** the user confirms, **Then** Fathom issues the mutation against the pod's `ephemeralcontainers` subresource and the dialog closes.
3. **Given** the mutation succeeds, **When** the pod detail view next refreshes (or receives a live update), **Then** the new ephemeral container appears in the pod's container list with its current status (waiting, running, or terminated) and the debug image name shown.
4. **Given** the confirmation dialog is showing, **When** the user cancels, **Then** no mutation is sent and no ephemeral container is added.

---

### User Story 2 - View debug container output before exec support exists (Priority: P2)

After adding an ephemeral debug container, the operator wants to see what it's doing (e.g. confirm it started, see its own diagnostic output) without necessarily having an interactive shell into it yet. They select the ephemeral container in the pod's container list and view its logs using the same log-viewing mechanism already used for regular pod containers.

**Why this priority**: This makes the feature independently useful even if the sibling interactive-exec feature is not yet available, and gives the user a way to confirm the debug container actually started successfully.

**Independent Test**: Can be fully tested by adding an ephemeral container to a pod, selecting it in the container list, and confirming its log stream opens and displays output using the existing pod log viewer, exactly as for a regular container.

**Acceptance Scenarios**:

1. **Given** an ephemeral container has been added and is running, **When** the user selects it in the pod's container list, **Then** they can start streaming its logs through the existing log viewer.
2. **Given** an ephemeral container has not yet started (status is waiting, e.g. still pulling its image), **When** the user attempts to view its logs, **Then** the UI shows the container's current status instead of an error, and offers to view logs once it starts running.

---

### User Story 3 - Understand irreversibility before committing (Priority: P3)

A cautious operator wants to be certain, before confirming, exactly what image will be attached, which container's process namespace it will share, and that the action is permanent for the life of the pod.

**Why this priority**: Reduces accidental or careless mutations to running pods; reinforces the mutation-guard pattern already used elsewhere in Fathom, applied honestly to a case where there is no undo.

**Independent Test**: Can be fully tested by opening the "Add debug container" dialog and verifying the summary (target pod, target container for shared process namespace, debug image) and the irreversibility warning are both visible before the confirm action is enabled.

**Acceptance Scenarios**:

1. **Given** the "Add debug container" form is filled in, **When** the confirmation step is reached, **Then** the dialog displays the target pod name, the container whose process namespace will be shared, the debug image name, and an explicit statement that ephemeral containers cannot be removed once added and will persist for the pod's lifetime.

### Edge Cases

- What happens when the debug image name is left blank or is not a syntactically valid image reference? The submit action stays disabled until a non-empty image reference is provided.
- What happens when the pod has zero regular containers (should not normally occur, but init-container-only or terminating pods might present oddly)? The target-container selector is disabled and the user is informed a target container is required.
- What happens when the same ephemeral container name is requested twice (e.g. duplicate submit, or the user re-runs "Add debug container" with a name that already exists on the pod)? The Kubernetes API rejects the duplicate name; Fathom surfaces that server-side error to the user without adding a duplicate.
- What happens when the user lacks RBAC permission to patch `pods/ephemeralcontainers`? The action is surfaced as denied through Fathom's existing permission/mutation-error handling, consistent with other RBAC-denied mutations.
- What happens when the pod is deleted or evicted after the mutation is sent but before the response is processed? The command fails with a not-found error and the UI reports it without erroring the whole pod view.
- How does the system handle a debug image that never starts (e.g. bad image reference)? The ephemeral container's status reflects the real Kubernetes container state (e.g. `ImagePullBackOff`); Fathom shows this status rather than treating it as its own error, matching how regular container statuses are already surfaced.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide an "Add debug container" action from a running pod's detail view.
- **FR-002**: System MUST let the user specify a debug container image via a plain text field (no built-in image catalog/picker required for v1).
- **FR-003**: System MUST let the user choose which existing container on the pod the ephemeral container will share the process namespace with (`targetContainerName`), when the pod has more than one container; when the pod has exactly one container it MAY be preselected.
- **FR-004**: System MUST route this action through Fathom's existing mutation guard/confirmation pattern before sending any request to the cluster.
- **FR-005**: The confirmation step MUST explicitly state that ephemeral containers cannot be removed once added and will persist for the life of the pod.
- **FR-006**: The confirmation step MUST display the target pod, the chosen target container (process-namespace share target), and the debug image name before the user can confirm.
- **FR-007**: System MUST classify this action as a namespaced cluster-write mutation (writes to the target pod's namespace), consistent with how other namespaced mutations are classified.
- **FR-008**: Upon confirmation, system MUST add the ephemeral container to the pod by updating the pod's `ephemeralcontainers` subresource (equivalent to `kubectl debug`'s `AddEphemeralContainer` behavior).
- **FR-009**: System MUST NOT offer any action to remove or delete an ephemeral container, since the Kubernetes API does not support removing one.
- **FR-010**: After a successful mutation, the pod's container list MUST reflect the new ephemeral container and its live status (waiting/running/terminated), sourced the same way existing container statuses are sourced (via pod refresh/live update).
- **FR-011**: System MUST allow viewing an ephemeral container's logs through the same log-viewing mechanism already used for regular pod containers, once the ephemeral container exists.
- **FR-012**: System MUST surface Kubernetes API errors from the add-ephemeral-container request (e.g. RBAC denial, duplicate name, invalid image reference, not-found pod) to the user without crashing or hanging the pod detail view.
- **FR-013**: System MUST disable/prevent submission of the "Add debug container" form when no image name has been entered or no target container is selected (when a target container is required).
- **FR-014**: Fathom's interactive pod-exec terminal (already implemented independently of this feature) MUST be launchable against a running ephemeral container the same way it is launched against a regular container, once the ephemeral container reaches a running state, with no ephemeral-container-specific changes required to the exec mechanism itself.

### Key Entities

- **Ephemeral Debug Container Request**: The user's in-flight input for the action — target pod (name + namespace), target container name (whose process namespace to share), and debug image reference. Exists only in UI state until confirmed and sent.
- **Ephemeral Container (cluster state)**: The Kubernetes-side object added to a pod's `spec.ephemeralContainers` and reported in `status.ephemeralContainerStatuses` — has a name, image, and a status (waiting/running/terminated) that Fathom reads the same way it reads regular container statuses. Fathom does not maintain a separate local model for this beyond what it already renders for pod containers.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user with a running pod and a distroless/shell-less container can attach a working debug container and see it appear in the pod's container list in under 30 seconds of confirming the action (excluding image pull time).
- **SC-002**: 100% of "Add debug container" confirmations display the irreversibility warning and the target pod/container/image summary before the confirm control can be activated.
- **SC-003**: Users can view an added debug container's log output without needing any capability beyond what this feature and the existing log viewer already provide, in builds where interactive exec is not yet available.
- **SC-004**: Server-side failures (RBAC denial, duplicate name, invalid image, pod not found) are visible to the user as a specific, readable error within the same interaction, with zero silent failures or UI hangs.

## Assumptions

- The target cluster's Kubernetes API server is new enough to support the `ephemeralcontainers` pod subresource as a stable (non-alpha-gated) feature; this has been GA since Kubernetes 1.25, which is assumed to be at or below the minimum supported cluster version.
- The interactive pod-exec terminal (the sibling capability this spec was originally written to depend on) is already implemented in the codebase (`ui/src/components/panels/PodTerminalTab.tsx`, `src/backend/exec.rs`, `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` in the protocol) and did not need to land separately; this feature reuses it as-is via container selection (FR-014) rather than building any new exec plumbing. This spec's primary UI value (adding a debug container and observing its status/logs) does not depend on exec at all — it is additive.
- Interactive shell selection ("which shell to run" inside the debug container) is out of scope — the debug image's own entrypoint/default command runs as-is, matching plain `kubectl debug`'s default behavior without a `--` override.
- Only one target container needs to be selectable per debug-container request; adding multiple ephemeral containers in a single action is out of scope (the user can repeat the action to add more, one at a time).
- No persistent record of "debug sessions" is kept by Fathom beyond what the Kubernetes API itself reports on the pod object; Fathom does not need its own history/audit log for this feature beyond existing mutation-outcome reporting.
