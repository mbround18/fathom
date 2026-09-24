# Feature Specification: Pod Exec Session Gate (Confirmation + RBAC Pre-Check)

**Feature Branch**: `024-pod-exec-terminal`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "In-app terminal / pod-exec — an interactive shell into a running container... gate it through mutation.rs's existing guard/confirmation pattern... and check the target via resolve_namespace_access()-style RBAC lookup before allowing it."

**Scope note (discovered during specification)**: Fathom already ships an in-app pod-exec terminal (spec `002-pod-logs-terminal`, User Story 2 — `src/backend/exec.rs`, `ui/src/components/panels/PodTerminalTab.tsx`, `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` in the protocol). That implementation currently classifies exec commands as `MutationTarget::ReadOnly` in `src/protocol/v1/mutation.rs` and relies solely on the Kubernetes API server rejecting the attach call for an unauthorized operator (a reactive, post-attempt error). This spec does **not** rebuild pod-exec; it adds the missing safety gate around the existing feature: an explicit confirmation before a session opens, and a proactive permission check that blocks the attempt before it ever reaches the cluster. Every acceptance scenario below assumes the underlying terminal UI and stream plumbing from spec 002 already exist.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Confirm before opening a shell into a container (Priority: P1)

An operator selects a running pod and opens its Terminal tab. Because an interactive shell is effectively remote code execution inside the cluster, the operator must explicitly acknowledge what they're about to do — which pod, namespace, and container — before the session opens, exactly as they already must for delete/scale/restart actions.

**Why this priority**: This is the core safety gap: exec is the single riskiest action in the app (arbitrary command execution against a live workload) yet today it opens with zero confirmation, unlike every other consequential action. Closing this gap is the primary reason this spec exists.

**Independent Test**: Select a running pod, open its Terminal tab, and observe a confirmation step naming the exact namespace/pod/container before any shell input is possible; confirming proceeds to the existing terminal experience, declining leaves the pod's cluster state and the Inspector untouched.

**Acceptance Scenarios**:

1. **Given** a running pod is selected in the Inspector, **When** the operator opens the Terminal tab (or picks a container, if the pod has more than one), **Then** the operator sees an explicit confirmation naming the namespace, pod, and container before any exec session starts.
2. **Given** the confirmation is showing, **When** the operator confirms, **Then** the exec session starts exactly as it does today (attach, stream, resize, teardown all unchanged).
3. **Given** the confirmation is showing, **When** the operator declines or dismisses it, **Then** no exec session is started, no request reaches the cluster, and the Inspector returns to its prior state.
4. **Given** the operator previously confirmed and closed a session, **When** they reopen the Terminal tab (same or a different pod), **Then** they see the confirmation again — confirmation is per-session-open, never remembered or skipped.

---

### User Story 2 - Block exec attempts the operator isn't permitted to make (Priority: P2)

An operator without `pods/exec` permission in the target namespace tries to open a Terminal tab. Today the app only finds out when the Kubernetes API rejects the attach; this story makes that check happen before the confirmation is even shown, so the operator gets an immediate, clear "not permitted" message instead of a confirmation for an action that would fail anyway.

**Why this priority**: Valuable defense-in-depth and a much better error experience, but strictly additive to Story 1 — the confirmation gate is the more urgent fix, and this story can ship right after it using the same gate hook.

**Independent Test**: As an operator whose role/binding disallows `pods/exec` in a namespace, open the Terminal tab for a pod in that namespace and confirm a permission-denied message appears immediately, with no confirmation dialog and no exec attempt reaching the cluster; as an operator with permission, confirm the flow proceeds to Story 1's confirmation unaffected.

**Acceptance Scenarios**:

1. **Given** the signed-in operator's roles/bindings do not grant `create` on `pods/exec` in the target namespace, **When** they attempt to open the Terminal tab for a pod in that namespace, **Then** they see a clear permission-denied message immediately, with no confirmation dialog shown and no attach request sent to the cluster.
2. **Given** the signed-in operator's roles/bindings do grant `create` on `pods/exec` in the target namespace, **When** they open the Terminal tab, **Then** the flow proceeds exactly as in User Story 1 (confirmation, then attach).
3. **Given** the permission pre-check itself fails to complete (e.g. the access-review call errors), **When** the operator attempts to open the Terminal tab, **Then** the app fails safe — it shows the error verbatim and does not open a session — rather than silently allowing or silently blocking.

### Edge Cases

- What happens if the operator's permissions change (revoked) between confirming and the session actually attaching? The Kubernetes API's own rejection at attach time (existing spec 002 behavior, FR-013) remains the final authority; the pre-check is a UX improvement, not a substitute for the API server's enforcement.
- What happens if the operator opens the Terminal tab for a second pod while a confirmed session is already active elsewhere? The existing singleton session behavior (spec 002 FR-012: prior session cleanly ends first) is unchanged; the new session still requires its own confirmation and its own permission pre-check.
- What happens when the pod has multiple containers? The container picker (existing, spec 002 FR-008) resolves the container *before* the confirmation and permission pre-check run, so both name the specific container that was chosen.
- What happens if the permission pre-check reports "allowed" but the container has no usable shell? Unchanged — the existing attach-failure handling (spec 002 FR-013) still applies after the gate passes.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST treat "open a pod-exec session" as a gated action, distinct from ordinary reads, using the same guard mechanism (`MutationTarget`-style classification in `src/protocol/v1/mutation.rs`) already used for cluster-mutating actions — even though exec does not create/update/delete a Kubernetes object.
- **FR-002**: The system MUST require an explicit, per-attempt operator confirmation — naming the target namespace, pod name, and container — before any exec session's attach request is sent to the cluster.
- **FR-003**: Declining or dismissing the confirmation MUST leave the cluster and the Inspector state unchanged, and MUST NOT send any exec-related request to the cluster.
- **FR-004**: Before showing the confirmation, the system MUST perform a live, proactive permission check for the signed-in operator's ability to exec into the target pod/namespace (equivalent to `create` on `pods/exec`), and MUST skip the confirmation and attach entirely when that check reports the operator is not permitted.
- **FR-005**: When the proactive permission check reports the operator is not permitted, the system MUST show a clear, specific permission-denied message distinguishable from other attach-failure reasons (e.g. "no shell available", "connection error").
- **FR-006**: When the proactive permission check itself cannot complete (errors out), the system MUST fail safe: show the error verbatim and not open a session, never silently proceed as if permitted.
- **FR-007**: The permission pre-check MUST NOT replace or weaken the Kubernetes API server's own enforcement at attach time (spec 002 FR-013 remains in force) — it is an additional, earlier check, not a trust boundary the backend can be the sole enforcer of.
- **FR-008**: None of the existing pod-exec session behavior (attach, stdin/stdout streaming, resize, teardown on tab close, singleton-session enforcement) changes as a result of this gate; this spec only adds the confirmation and the pre-check ahead of the existing `StartPodExec` flow.

### Key Entities

- **Exec Session Gate**: The confirmation + permission pre-check wrapped around an existing exec session's start, keyed by namespace/pod/container; produces either "proceed to attach" or "blocked" (with a reason: declined, or not-permitted).
- **Permission Pre-Check Result**: The outcome of the proactive check — allowed, denied (with reason), or check-failed (with the underlying error) — surfaced to the operator before any confirmation is shown.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of pod-exec session opens require an explicit operator confirmation naming the exact target before any shell input is possible.
- **SC-002**: An operator lacking exec permission for a namespace sees a permission-denied message immediately (before any confirmation dialog appears) in 100% of attempts, with zero attach requests reaching the cluster.
- **SC-003**: Operators with permission see no added friction beyond the one confirmation step — the rest of the terminal experience (attach time, streaming, resize) is unchanged from today's behavior.
- **SC-004**: Declining the confirmation, in 100% of cases, results in zero cluster-directed requests and an unchanged Inspector state.

## Assumptions

- The existing pod-exec terminal (spec 002) remains the transport, UI, and session-lifecycle implementation; this spec only adds a gate in front of `StartPodExec`.
- "Permission to exec" is evaluated using the same live `SelfSubjectAccessReview`-style mechanism the app already uses for other self-permission checks (spec 011's `RefreshRbacSelfAccess`/`RbacAccessCheck`), scoped to `pods/exec` `create` in the target namespace — not the admin-facing `resolve_namespace_access()` role/binding resolver, which answers "what can an arbitrary subject access" for RBAC administration screens rather than "can the current session do this right now." This substitution is called out explicitly because the feature brief named `resolve_namespace_access()`; the live self-access-review path is the functionally correct primitive for a real-time gate and is documented as a deviation in `plan.md`/`research.md`.
- Session recording or an audit log of commands typed inside an exec session is explicitly out of scope for this spec (a separate future spec, if ever wanted).
- Exec into ephemeral debug containers (`kubectl debug`-style) is explicitly out of scope for this spec; it is covered by the sibling spec `025-ephemeral-debug-containers`, built concurrently.
- Multi-pane / split-terminal support is out of scope.
- The confirmation is shown once per session-open attempt and is never remembered/skipped for future attempts, consistent with Constitution Principle I (explicit mutation, no optimistic/implicit action).
