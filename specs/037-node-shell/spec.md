# Feature Specification: Node Shell (On-Demand Privileged Debug Pod)

**Feature Branch**: `037-node-shell`
**Created**: 2026-09-24
**Status**: Draft
**Input**: docs/MVP.md section 10 "Node Shell": an on-demand ephemeral pod scheduled onto a specific node (kubectl-debug-node style), with configurable image and optional image pull secret, distinct from pod exec.

**Scope note**: This reuses, and does not rebuild, the terminal from spec `002-pod-logs-terminal` (`src/backend/exec.rs`, `PodTerminalTab.tsx`, xterm stream plumbing) and the gate pattern from `024-pod-exec-terminal` (`MutationTarget` classification, `ConfirmDialog`, `SelfSubjectAccessReview` pre-check via `rbac_access.rs`). It follows spec `025-ephemeral-debug-containers` where relevant (irreversibility-honest confirmation, image as a plain text field, verbatim API errors). Unlike 025, no existing pod is modified: a brand-new pod is created and then deleted. Specs 024 and 025 are on unmerged branches; the plan states which of their pieces this spec depends on and how it behaves if they land first or later.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Open a shell on a node (Priority: P1)

From a Node's inspector, the operator chooses "Node shell". A confirmation names the node, the namespace and image of the pod that will be created, and states plainly that the pod is privileged with host PID, network and IPC and the node root filesystem mounted at `/host`. On confirm, Fathom creates the pod pinned to that node, waits until it is running, and attaches a terminal.

**Why this priority**: Core value. Operators need host-level access to a node (kubelet logs, disk, `crictl`, network) that pod exec cannot give.

**Independent Test**: Select a node, open Node shell, confirm, and observe a working terminal whose hostname is the node; the pod exists in the cluster while the session is open.

**Acceptance Scenarios**:
1. **Given** a node is selected, **When** the operator opens Node shell, **Then** a confirmation lists node name, target namespace, pod name, image, and the privileges granted, before anything is sent to the cluster.
2. **Given** the confirmation, **When** the operator declines or dismisses it, **Then** no request reaches the cluster.
3. **Given** the operator confirms, **When** the pod is created, **Then** the UI shows progress phases (creating, pulling/starting, attaching) driven by backend-reported state, never assumed.
4. **Given** the pod is running, **When** attach succeeds, **Then** the terminal behaves as the existing pod terminal (stream, resize, input).
5. **Given** a previously confirmed session was closed, **When** the operator reopens Node shell, **Then** confirmation is shown again (per-session-open, never remembered).

### User Story 2 - Pod is always cleaned up (Priority: P1)

Whatever ends the session (tab close, Stop button, terminal exit, cluster context switch, app quit, lost connection, crash), the debug pod is deleted. If Fathom cannot delete it, the operator is told which pod remains, by namespace/name.

**Why this priority**: A leftover privileged pod is a security liability. Cleanup is a safety requirement, not polish.

**Independent Test**: Start a session, then end it each of the ways listed; verify the pod is gone. Kill the app process and verify the pod self-expires and is swept on next launch/open.

**Acceptance Scenarios**:
1. **Given** an open session, **When** the operator closes the tab or presses Stop, **Then** the pod is deleted and the UI shows "removed" only after the backend reports deletion.
2. **Given** an open session, **When** the operator switches cluster context or quits Fathom, **Then** the pod is deleted before teardown completes (bounded wait), on the original context's client.
3. **Given** Fathom was killed, **When** the pod's safety deadline elapses, **Then** the cluster stops the pod without Fathom.
4. **Given** orphaned Fathom node-shell pods exist from a prior run, **When** the operator next opens Node shell or the app connects to that context, **Then** orphans are listed by namespace/name and offered for confirmed deletion (never silently deleted).
5. **Given** deletion fails, **Then** the error is shown verbatim together with the leftover pod's namespace/name and a retry action.

### User Story 3 - Understand why it cannot start (Priority: P2)

Before confirmation, Fathom checks the operator can create and delete pods and exec into pods in the target namespace and can read the node. If not, the operator sees a specific denied message naming the missing verb/resource, with no confirmation. Failures after confirmation (PodSecurity admission rejecting privileged pods, image pull failure, unschedulable/NotReady node, quota) are surfaced verbatim, and the partially created pod is cleaned up.

**Independent Test**: With a role lacking `pods/exec` or `pods` create, open Node shell and see the specific denial without a dialog. With a namespace enforcing the `restricted` PodSecurity level, confirm and see the admission error verbatim and no leftover pod.

**Acceptance Scenarios**:
1. **Given** any required permission is missing, **Then** a denied message lists each missing permission and no confirmation or create request occurs.
2. **Given** the access check itself errors, **Then** the app fails safe: error shown verbatim, no session.
3. **Given** admission or scheduling rejects the pod, **Then** the reason is shown verbatim and any created pod is deleted.
4. **Given** the pod stays Pending past a start timeout, **Then** the operator sees the pod's real status/conditions and can cancel, which deletes the pod.

### User Story 4 - Configure image, namespace, pull secret (Priority: P3)

In Cluster Settings the operator sets the node-shell image (default a small utility image such as `busybox`, plain text field), the namespace for the pod, and an optional image pull secret name. The per-session confirmation shows the effective values and allows override.

**Acceptance Scenarios**:
1. **Given** settings are saved, **Then** they persist per cluster context in `~/.fathom/` additively and survive upgrade.
2. **Given** a pull secret name that does not exist in the namespace, **Then** the resulting pull failure surfaces verbatim; Fathom never reads or displays secret contents.
3. **Given** invalid or unreadable settings, **Then** defaults are used and the file is left untouched.

### Edge Cases
- Node NotReady, cordoned, or tainted: the pod pins with `nodeName` and tolerates all taints, so cordon does not block; NotReady failures are surfaced honestly.
- Windows nodes: unsupported in v1; the action is disabled with the reason when the node OS label is `windows`.
- Second Node shell while one is active: the existing singleton-session rule applies; the prior session ends and its pod is deleted before the new one is confirmed.
- Terminal process exits (`exit`): the session ends and the pod is deleted.
- Pod name collisions: generated names include a random suffix; a collision is retried once, then reported.
- Operator loses connectivity mid-session: the deadline safety net and orphan sweep cover it; the UI shows the disconnected state.
- Events from a superseded session or previous context are discarded (Constitution V).

## Requirements *(mandatory)*

### Functional Requirements
- **FR-001**: Provide a "Node shell" action from a Node inspector, separate from the pod Terminal tab.
- **FR-002**: Classify starting a node shell as a namespaced cluster-write mutation (`MutationTarget::Namespaced` on the pod namespace) so it passes the existing confirm gate; the sandbox/devbridge guard MUST refuse it.
- **FR-003**: Require a per-attempt confirmation naming node, `namespace/pod-name`, image, and an explicit statement of privileged/hostPID/hostNetwork/hostIPC and host root mount, before any request to the cluster.
- **FR-004**: Before confirmation, run a proactive `SelfSubjectAccessReview` check for: create pods, delete pods, get pods, create pods/exec in the namespace, and get nodes. If any is denied, show each missing permission and skip confirmation. If the check errors, fail safe. The API server remains the final authority.
- **FR-005**: On confirm, create a pod with: `nodeName` set to the chosen node, `restartPolicy: Never`, `hostPID`, `hostNetwork`, `hostIPC` true, a privileged container with the node root mounted at `/host` (hostPath `/`), tolerations for all taints, a sleeping keep-alive command, `activeDeadlineSeconds` (default 4 hours) as a self-expiry net, optional `imagePullSecrets`, and labels `app.kubernetes.io/managed-by=fathom`, `fathom.io/node-shell=true`, `fathom.io/node=<node>`.
- **FR-006**: Attach a terminal using the existing exec stream plumbing; terminal behavior (input, output, resize, ring-buffered output with truncation marker) is unchanged.
- **FR-007**: Report lifecycle state (`Creating`, `Pending`, `Running`, `Attached`, `Terminating`, `Removed`, `Failed`) from backend events; the UI never assumes a state.
- **FR-008**: Guarantee cleanup: delete the pod on Stop, tab close, terminal exit, session supersede, context switch, and app exit (bounded wait), plus the `activeDeadlineSeconds` net, plus an orphan sweep (list by label, confirm by name, delete).
- **FR-009**: Deletion failures MUST be shown verbatim with the leftover `namespace/pod` and offer retry. "Removed" appears only after the backend confirms deletion or NotFound.
- **FR-010**: Start timeout (default 90 s, image pull included) surfaces real pod status and conditions; cancel deletes the pod.
- **FR-011**: Configurable image (default `busybox`-class utility image), namespace (default `default`), optional image pull secret name; persisted per context, additive format; the confirmation may override for one session.
- **FR-012**: All backend and API errors reach the operator verbatim (Constitution I).
- **FR-013**: The feature MUST NOT escalate privileges: it runs as the operator's kubeconfig identity only (Constitution VI). It MUST NOT log or display secret contents.
- **FR-014**: Node shell MUST NOT be exposed through the MCP server in v1.
- **FR-015**: Layout follows Constitution IV: usable at phone width (the terminal takes the full stack pane), no horizontal page scroll, keyboard operable, reduced-motion respected.

### Key Entities
- **NodeShellSession**: id, context, node, namespace, pod name, image, phase, created-at, deadline.
- **NodeShellSettings**: image, namespace, image pull secret name (per context).
- **NodeShellAccessResult**: per-permission allowed/denied/error.
- **OrphanedNodeShellPod**: namespace, name, node, created-at.

## Success Criteria *(mandatory)*
- **SC-001**: 100% of session starts require a confirmation naming node, pod and privileges.
- **SC-002**: With missing permission, the operator sees the denial in under 2 s and zero create requests reach the cluster.
- **SC-003**: After any normal end path, zero Fathom node-shell pods remain (verified per path in E2E and quickstart).
- **SC-004**: After an abnormal end, no pod runs past the deadline, and orphans are detected on next use in 100% of runs.
- **SC-005**: On a warm image, the terminal is interactive within 15 s of confirmation.
- **SC-006**: Failures always show the real API/pod reason, never a generic message.

## Assumptions
- Linux nodes only. The cluster permits privileged pods in the chosen namespace; otherwise the admission error is the outcome (surfaced, not bypassed).
- Default namespace `default` mirrors `kubectl debug node`; operators may pick `kube-system` or a dedicated one.
- No image catalog UI; plain text field (as in 025).
- Cleanup on hard kill relies on the deadline and orphan sweep, since no in-process code runs.
- 024 confirm/pre-check components and 025 conventions are reused if merged; otherwise this spec's tasks include the minimal equivalents to avoid a later duplicate.
