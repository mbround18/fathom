# Feature Specification: Kubernetes Cluster Version Badge

**Feature Branch**: `019-cluster-version-badge`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "A small, low-risk, read-only feature: show the connected cluster's Kubernetes server version (and control-plane/node version skew, if any nodes run an older kubelet than the control plane) as a persistent badge/indicator in the app shell (e.g. near the cluster/context switcher), so an operator always knows at a glance what version of Kubernetes they're driving without opening a separate pane."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See the cluster's Kubernetes version at a glance (Priority: P1)

As an operator working in Fathom, I want to see the connected cluster's Kubernetes server version displayed persistently in the app shell, so I always know what version of Kubernetes I'm driving without navigating to a separate pane or running a separate command.

**Why this priority**: This is the core value of the feature — the whole point is ambient awareness of cluster version without extra navigation. Without this, the feature delivers nothing.

**Independent Test**: Connect Fathom to a cluster and verify a badge showing the server version (e.g. "v1.29.4") appears near the cluster/context switcher in the app shell, without opening any other pane.

**Acceptance Scenarios**:

1. **Given** Fathom is connected to a cluster, **When** the app shell renders, **Then** a badge near the cluster/context switcher shows the cluster's Kubernetes server version.
2. **Given** the operator switches to a different cluster context, **When** the new connection is established, **Then** the badge updates to reflect the newly connected cluster's server version.
3. **Given** the server version cannot be determined (e.g. transient discovery failure), **When** the app shell renders, **Then** the badge shows a neutral "unknown" state rather than a stale or incorrect version.

---

### User Story 2 - Be warned about control-plane/node version skew (Priority: P2)

As an operator, I want to be warned when one or more nodes are running an older kubelet version than the control plane, so I can identify potential compatibility issues or nodes overdue for upgrade without manually cross-checking each node.

**Why this priority**: Version skew is a common real-world operational concern (Kubernetes officially supports a bounded skew, but drift beyond that causes subtle issues). This builds on the base version display and is valuable but secondary to simply knowing the version at all.

**Independent Test**: Connect to a cluster with at least one node whose kubelet version differs from the control-plane server version, and verify the badge shows a distinct warning indicator; connect to a cluster where all node kubelet versions match, and verify no warning is shown.

**Acceptance Scenarios**:

1. **Given** all nodes report a kubelet version equal to the control-plane server version, **When** the badge renders, **Then** no skew warning is shown.
2. **Given** one or more nodes report a kubelet version older than the control-plane server version, **When** the badge renders, **Then** a visible warning indicator is shown on or adjacent to the badge.
3. **Given** a skew warning is shown, **When** the operator inspects the badge (e.g. hover or click), **Then** they can see which distinct kubelet version(s) are present versus the control-plane version, so they know the nature of the skew without leaving the app shell.
4. **Given** the cluster has no nodes visible to the operator's current RBAC permissions (or node data hasn't loaded yet), **When** the badge renders, **Then** it shows the server version without asserting a skew state (neither warned nor explicitly "all clear").

### Edge Cases

- What happens when the `/version` discovery call fails or times out? The badge shows a neutral "unknown version" state; it does not block the rest of the app shell from rendering.
- What happens when node data is unavailable (e.g. RBAC denies listing nodes)? The badge still shows the server version; skew detection is simply not evaluated (no false warning, no false all-clear).
- What happens when nodes report kubelet versions *newer* than the control plane (an unusual but possible skew direction)? This also counts as skew and surfaces the same warning treatment, since any control-plane/kubelet mismatch is operationally relevant regardless of direction.
- What happens on a cluster with a very large number of distinct kubelet versions? The badge shows a concise summary (e.g. count of distinct versions) with full detail available on inspection, rather than an unbounded list crowding the shell.
- What happens immediately after switching cluster context, before the new cluster's version data has loaded? The badge shows a neutral loading/unknown state rather than the previous cluster's stale version.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST display the connected cluster's Kubernetes server version as a persistent, always-visible badge/indicator in the app shell, located near the cluster/context switcher.
- **FR-002**: System MUST refresh the displayed server version whenever the cluster connection changes (e.g. switching context) and periodically thereafter, consistent with how other lightweight cluster facts are kept fresh elsewhere in the app.
- **FR-003**: System MUST determine, for each visible node, whether its kubelet version differs from the control-plane server version, and MUST surface a warning indicator on the badge when any such difference ("skew") is detected.
- **FR-004**: System MUST allow the operator to see the specific distinct kubelet version(s) contributing to a detected skew, without leaving the app shell (e.g. via hover tooltip or lightweight popover on the badge).
- **FR-005**: System MUST show a neutral "unknown"/"unavailable" state for the version badge when the server version cannot be determined, rather than omitting the badge or showing incorrect/stale data.
- **FR-006**: System MUST NOT block or delay rendering of the rest of the app shell while version/skew information is being fetched.
- **FR-007**: System MUST treat this feature as strictly read-only: it MUST NOT perform, suggest, or provide entry points for any cluster mutation (e.g. no upgrade actions, no confirmation flows).
- **FR-008**: System MUST scope version/skew reporting to the cluster and node level only; it MUST NOT display or aggregate per-pod or per-container version information as part of this feature.
- **FR-009**: System MUST NOT retain or display historical version data (e.g. version-over-time trends); only the current, live-observed version/skew state is shown.

### Key Entities

- **Cluster Version Summary**: Represents the currently observed Kubernetes version state for the connected cluster — the control-plane server version, the set of distinct node kubelet versions observed, and whether a skew condition currently exists between them.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can determine the connected cluster's Kubernetes server version within 1 second of the app shell being visible, without opening any additional pane or running an external command.
- **SC-002**: When node kubelet versions differ from the control-plane version, 100% of such cases are visibly flagged in the app shell without requiring the operator to manually inspect individual nodes.
- **SC-003**: Switching cluster context updates the displayed version information to match the newly connected cluster within the same refresh window used by other ambient cluster status indicators, with no stale data shown from the prior cluster.
- **SC-004**: The version badge introduces no perceptible delay (no added blocking wait) to the app shell's initial render.

## Assumptions

- The Kubernetes API server's `/version` (discovery) endpoint is reachable using the same credentials/context already used for other read operations; no additional permissions are requested for this feature.
- Node kubelet version data is sourced from `status.nodeInfo.kubeletVersion` on Node objects the operator's current RBAC already permits listing; no elevated access is requested.
- "Skew" is defined as any node kubelet version string differing from the control-plane server version string (older or newer); precise semantic version comparison beyond basic parsing is not required for v1.
- The badge is a compact, persistent UI element (not a full pane); detailed skew breakdown is available via a lightweight interaction (hover/click) rather than a dedicated navigable page.
- This feature does not gate on or interact with any mutating action, so Constitution Principle I (mutation confirmation) does not apply here.
