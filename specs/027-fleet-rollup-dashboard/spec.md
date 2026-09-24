# Feature Specification: Fleet Rollup Dashboard

**Feature Branch**: `027-fleet-rollup-dashboard`

**Created**: 2026-09-23

**Status**: Draft (clarified 2026-09-24)

**Input**: User description: "Fleet-style multi-context rollup dashboard — a single dashboard view showing health/status across ALL connected contexts at once (node counts, unhealthy-pod counts, GitOps/Flux sync status if applicable, cluster version) instead of switching contexts one at a time."

## Clarifications

### Session 2026-09-24

- Q: What does "connected context" mean, given only one context holds a live connection at a time (spec 012)? -> A: Every context listed in the operator's kubeconfig. The dashboard builds its own short-lived read-only clients, so the list of cards is the kubeconfig context list, not the single live-connected context. (Matches the shipped backend.)
- Q: Where does the dashboard live? -> A: A dialog opened from a Fleet button in the top bar, available on every screen; not a nav section.
- Q: On card click (FR-007), what happens to the dashboard? -> A: The active context switches and the Fleet dialog closes, landing the operator on that cluster's normal view.
- Q: How is a reachable context with limited RBAC shown? -> A: Gracefully. Show whatever is readable (e.g. version) and mark each unreadable count as "no access"; the card is not flagged Auth error unless the API server itself rejects the credentials outright.
- Q: What counts as an unhealthy pod? -> A: Phase Failed/Unknown; a Running pod with an unready container; a container waiting in ImagePullBackOff, ErrImagePull, CrashLoopBackOff or CreateContainerError; or a pod Pending for more than 5 minutes. A young Pending pod is not unhealthy.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See fleet-wide health at a glance (Priority: P1)

An operator managing several Kubernetes clusters through Fathom's configured contexts wants to know, without switching contexts one at a time, which clusters are healthy and which need attention right now.

**Why this priority**: This is the entire value proposition of the feature — a single screen that replaces manually clicking through each context's dashboard. Without this, the feature delivers nothing.

**Independent Test**: With two or more contexts configured, open the Fleet Dashboard view and confirm a summary card/row renders for every configured context showing node count, unhealthy-pod count, and cluster version, without changing the currently active context.

**Acceptance Scenarios**:

1. **Given** two configured contexts, one fully healthy and one with an unhealthy pod, **When** the operator opens the Fleet Dashboard, **Then** both contexts' summaries render simultaneously with correct health indicators, and the active context selection elsewhere in the app is unchanged.
2. **Given** a context whose cluster becomes unreachable while the dashboard is open, **When** the next poll cycle occurs, **Then** that context's card shows a "connection lost"/error status instead of stale or crashed UI, while other contexts' cards continue to update normally.
3. **Given** the operator clicks a context's summary card, **When** the click is registered, **Then** Fathom switches the active context to that cluster and closes the Fleet dialog, landing on that cluster's normal (existing, single-context) view — reusing the existing context-switch mechanism from the federation switcher.

---

### User Story 2 - Spot GitOps drift across the fleet (Priority: P2)

An operator using Flux for GitOps on some or all clusters wants to see, from the same rollup view, whether any cluster's Flux sync has drifted or failed, without opening each cluster individually.

**Why this priority**: GitOps sync status is a high-signal health indicator for operators who use Flux, but it is only applicable to a subset of clusters/installations, so it is secondary to the baseline node/pod health rollup.

**Independent Test**: With one configured context running Flux and reporting a failed reconciliation, and another context with no Flux installation, open the Fleet Dashboard and confirm the Flux-enabled context shows its sync status (e.g., "out of sync"/"failed") while the non-Flux context shows no Flux indicator (or an explicit "not applicable") rather than an error.

**Acceptance Scenarios**:

1. **Given** a context with Flux installed and all Kustomizations/HelmReleases synced, **When** the dashboard renders that context's card, **Then** it shows a "synced" GitOps indicator.
2. **Given** a context with Flux installed and at least one Kustomization/HelmRelease failing, **When** the dashboard renders that context's card, **Then** it shows an "out of sync"/"failed" GitOps indicator distinguishable from the baseline node/pod health indicator.
3. **Given** a context with no Flux CRDs installed, **When** the dashboard renders that context's card, **Then** no GitOps status is shown for it (and its absence does not block or delay the rest of that card's data).

---

### User Story 3 - Keep watching the fleet without babysitting it (Priority: P3)

An operator leaves the Fleet Dashboard open on a secondary monitor and expects the data to refresh periodically on its own, and expects that adding or removing a context in the kubeconfig (picked up on the app's existing context-list refresh) updates the dashboard's list of cards without closing and reopening it.

**Why this priority**: This is a quality-of-life/operational-monitoring enhancement on top of the core rollup (P1) and GitOps (P2) value; the dashboard is still useful as a manually-refreshed snapshot without it, so it is lower priority.

**Independent Test**: Open the dashboard, wait for one automatic refresh interval, and confirm at least one card's data timestamp/values update without user interaction; then add a context to the kubeconfig, refresh contexts, and confirm a new card appears without reopening the dashboard.

**Acceptance Scenarios**:

1. **Given** the Fleet Dashboard is open, **When** the configured polling interval elapses, **Then** each context's card refreshes its data independently of the others.
2. **Given** the Fleet Dashboard is open, **When** a new context appears in the kubeconfig and the app's context list refreshes, **Then** a new card for that context appears on the dashboard automatically.
3. **Given** the Fleet Dashboard is open, **When** a context is removed from the kubeconfig and the app's context list refreshes, **Then** that context's card is removed from the dashboard automatically.

---

### Edge Cases

- What happens when a context's credentials expire or are revoked mid-poll? The card shows an authentication-error status distinct from a generic connection error; other contexts are unaffected.
- What happens when zero contexts are configured? The dashboard shows an empty state directing the operator to connect a context, not a blank screen or error.
- What happens when the credentials can list nodes but not pods (or vice versa)? The card shows what is readable and marks the rest "no access" rather than failing the card (FR-013).
- What happens when there is exactly one configured context? The dashboard still renders (a fleet of one), so operators don't need a separate code path.
- How does the system handle a context that responds very slowly (e.g., high-latency cluster)? That context's card shows a loading/pending state without blocking the rendering or refresh of other contexts' cards.
- What happens if the number of configured contexts is large (e.g., 20+)? The dashboard must remain responsive; polling is expected to be staggered/bounded rather than firing all requests in one burst (see Success Criteria).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a single dashboard view, opened from the top bar, that displays a summary for every context in the kubeconfig simultaneously, without requiring the operator to switch the active context.
- **FR-002**: Each context's summary MUST include: node count (ready/total), unhealthy-pod count, and cluster (Kubernetes) version.
- **FR-003**: Each context's summary MUST include GitOps/Flux sync status when Flux CRDs are detected on that cluster, and MUST omit or mark as "not applicable" the GitOps indicator when Flux is not installed on that cluster.
- **FR-004**: The system MUST continue to poll and refresh each configured context's summary data on a recurring interval while the dashboard is open, independently per context (a slow or failing context MUST NOT block or delay other contexts' updates).
- **FR-005**: The system MUST visually distinguish, per context, at least the following states: healthy, degraded (e.g., unhealthy pods present), unreachable/connection error, authentication error, and loading/pending.
- **FR-006**: The dashboard's list of context cards MUST reflect connects/disconnects made through the existing federation context-switcher without requiring a manual page reload.
- **FR-007**: Selecting a context's card MUST switch the active context using the existing single-context-switch mechanism and navigate to that context's normal dashboard.
- **FR-008**: The dashboard MUST NOT expose any bulk or cross-cluster mutation actions (e.g., delete, scale, apply) — it is view-only.
- **FR-009**: The dashboard MUST NOT require or provide any cluster grouping, tagging, or filtering UI in this version — a flat list of all configured contexts is sufficient.
- **FR-010**: When a context becomes unreachable or its credentials fail during polling, the system MUST surface that context's error state on its own card without crashing or blanking the rest of the dashboard.
- **FR-011**: When zero contexts are configured, the dashboard MUST show an empty state guiding the operator to connect a context rather than an error or blank view.

- **FR-012**: Polling MUST be bounded: at most 8 contexts are polled concurrently and their first polls are not all fired in one burst, so a large fleet (20+) does not spike the operator's machine or API servers. Polling exists only while the dashboard is open.
- **FR-013**: When a reachable context denies list access to some resources, the card MUST show the readable data and mark each unreadable count "no access"; it MUST NOT be flagged Auth error unless the API server rejects the credentials outright.
- **FR-014**: "Unhealthy pod" MUST be defined as in Clarifications (Failed/Unknown phase; Running with an unready container; ImagePullBackOff/ErrImagePull/CrashLoopBackOff/CreateContainerError waiting; Pending for more than 5 minutes).
- **FR-015**: Each card MUST show how recent its data is (e.g. "updated 12s ago") so a stale card is distinguishable from a fresh one.
- **FR-016**: Activating a card MUST close the Fleet dialog after switching the active context (FR-007).

### Key Entities

- **Context Summary**: A per-context rollup record consisting of context identifier/display name, node counts, unhealthy-pod count, cluster version, GitOps sync status (optional), overall health state, and last-updated timestamp.
- **Fleet Dashboard View**: The aggregate, read-only screen composed of one Context Summary per currently configured context.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator with multiple configured clusters can identify which clusters are unhealthy within 5 seconds of opening the Fleet Dashboard, without switching context.
- **SC-002**: At desktop width, up to 10 contexts' summaries are visible with no more than one scroll action; at phone width cards stack in one column with no horizontal scroll.
- **SC-003**: A context that is slow or unreachable does not delay the display of other contexts' data by more than the configured per-context timeout (i.e., other cards render as soon as their own data arrives).
- **SC-004**: The dashboard's automatic refresh keeps each card's data no more than one polling interval stale during normal operation.
- **SC-005**: 100% of dashboard opens with zero configured contexts show the guided empty state rather than an error.

## Assumptions

- The operator's kubeconfig context list (as loaded by Fathom's existing context refresh, spec 012) is the source of truth for which contexts get a card; this feature adds no separate connect/disconnect flow and does not hold live connections.
- Node counts, pod health, and cluster version are obtainable per-context using the same read-only Kubernetes API calls Fathom's existing single-context dashboard already uses.
- GitOps status is scoped to Flux (the GitOps tool already referenced in prior research); other GitOps tools (e.g., Argo CD) are out of scope for v1 unless a future spec extends this.
- The polling interval and per-context timeout are implementation details to be finalized during planning, not fixed by this spec; a reasonable default (e.g., tens of seconds) is expected.
- This dashboard is additive: the existing per-context dashboard and context-switcher continue to work unchanged for operators who only care about one cluster at a time.
- No new authentication/authorization model is introduced; each context's existing kubeconfig/credentials are reused for polling.
