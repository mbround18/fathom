# Feature Specification: Simultaneous Multi-Context Federation

**Feature Branch**: `034-simultaneous-federation`
**Created**: 2026-09-24
**Status**: Draft
**Input**: docs/MVP.md §14 Tier D P2 — true simultaneous federation: multiple live clients at once and a cross-cluster aggregate overview, building on the 027 fleet pollers rather than duplicating them.

## Clarifications (informed defaults, no questions asked)

- Federation is opt-in per context; the default federated set is empty and behavior is exactly today's (one active client), carrying 012 spec US2 scenario 3.
- The federated set is distinct from the primary active context (012 Edge Cases): switching one never changes the other.
- "Live" means a persistent, reused `kube::Client` per federated context plus bounded watches on a small fixed resource set (Pods, Nodes, Deployments, Warning Events). It does not mean full per-cluster resource stores; the ~50 single-client resource modules are untouched.
- The federated set is persisted (additive, per Principle VI) and restored on launch, but restoring only reconnects contexts the operator previously chose.
- Soft cap default of 8 federated contexts (matches 027's `MAX_CONCURRENT_POLLS`); exceeding it warns, never auto-disconnects.
- The aggregate view is read-only; drill-down switches the primary active context (same as 027 card click).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Federate contexts and see live cross-cluster state (Priority: P1)

An operator marks two or more reachable contexts as federated. Fathom holds a live connection to each and shows an aggregate overview: per-cluster and combined totals (nodes ready/total, pods by phase, unhealthy pods, deployments not fully available, recent Warning events) updating within seconds of a change, without switching the active context.

**Why this priority**: It is the whole capability; nothing else has value without it.

**Independent Test**: Federate two contexts, change a pod in one, and confirm the aggregate and that cluster's row update within 5s while the primary active context is unchanged.

**Acceptance Scenarios**:
1. **Given** contexts dev and staging are federated, **When** a pod in staging starts crash-looping, **Then** staging's unhealthy count and the combined total update within 5s and dev is unaffected.
2. **Given** two federated contexts, **When** one loses connectivity, **Then** its row shows an error state (verbatim error, last-updated time) with no stale numbers presented as live, and the other keeps updating; the dropped one retries with backoff.
3. **Given** no context has been federated, **When** the app is used, **Then** no extra clients, watches or CPU are used.
4. **Given** the operator un-federates a context, **Then** its client and watches are released and its row disappears.

### User Story 2 - Reuse the fleet rollup as the aggregate's base layer (Priority: P2)

The 027 Fleet Dashboard continues to work unchanged for non-federated contexts (short-lived polling, only while open). For federated contexts its cards are fed by the live connection instead of re-polling, so a context is never polled twice, and cards show a "live" versus "polled" indicator.

**Why this priority**: Avoids duplicate load and duplicate code, but the aggregate (P1) is useful without it.

**Independent Test**: With one context federated and the Fleet dialog open, assert exactly one client exists for that context and its card is marked live; the other card remains polled.

**Acceptance Scenarios**:
1. **Given** a federated context and the Fleet dialog open, **Then** no separate fleet poller runs for it.
2. **Given** the context is un-federated while the dialog is open, **Then** the card falls back to polling without flicker or duplicate cards.

### User Story 3 - Persisted, bounded, restraint-aware federation (Priority: P3)

The federated set survives restart; a warning appears above the soft cap; events from a removed context or superseded connection are never rendered.

**Why this priority**: Operational polish on top of P1/P2.

**Independent Test**: Federate two contexts, restart, confirm both reconnect; federate a 9th and see the warning while all remain connected.

**Acceptance Scenarios**:
1. **Given** a persisted set containing a context since removed from kubeconfig, **Then** it is shown as unavailable and pruned only on operator action.
2. **Given** Pods RBAC is denied on a federated context, **Then** that metric shows "no access" and the rest of the row still works.

### Edge Cases

- Same API server under two contexts: two independent entries, no de-duplication (012).
- Watch stream expiry / 410 Gone: relist and resume without UI reset.
- Primary active context is also federated: primary views use the existing client; the federation layer holds a separate client, and the aggregate reads only federation counters.
- Events tagged with a superseded generation (un-federate then re-federate) are discarded.
- Exec-plugin credentials that expire mid-session (relevant to future P3) surface as an auth error on that row only.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Operator MUST be able to add/remove any kubeconfig context to/from a federated set via explicit commands; default set is empty.
- **FR-002**: For each federated context the backend MUST hold one persistent `kube::Client` and maintain bounded watches for Pods, Nodes, Deployments and Warning Events.
- **FR-003**: The backend MUST emit per-context summaries and a combined cross-cluster rollup through the versioned protocol; the UI MUST NOT contact clusters.
- **FR-004**: One context's failure MUST NOT block or delay another's updates; each row shows its own state (Pending, Live, Reconnecting, Unreachable, AuthError).
- **FR-005**: Errors MUST reach the operator verbatim; failed contexts MUST NOT display stale numbers as current.
- **FR-006**: The federated set MUST persist additively in `~/.fathom/` and load with no manual steps; an unreadable file falls back to an empty set without deletion.
- **FR-007**: The federation layer MUST reuse the 027 machinery (client construction, concurrency cap, staggered start, unhealthy-pod classification, `FleetContextSummary`) rather than duplicate it; a context MUST NOT be both polled and live at once.
- **FR-008**: The primary active context and the federated set MUST be independent.
- **FR-009**: Above the soft cap (default 8) the UI MUST warn; the system MUST NOT auto-disconnect.
- **FR-010**: The feature MUST be read-only; drill-down uses the existing SetContext.
- **FR-011**: Stale-generation events MUST be discarded (Principle V).
- **FR-012**: Secrets/credentials MUST NOT be persisted or logged; the persisted set holds context names only.
- **FR-013**: Watch changes MUST be debounced (default 500ms) into summary events, not one event per object change.

### Key Entities

- **FederatedSet**: persisted ordered list of context names plus soft cap.
- **FederationContextState**: per-context connection state, generation, last-updated, error.
- **FederationRollup**: per-context counts (nodes, pods by phase, unhealthy, deployments unavailable, warning events in last hour) plus combined totals across Live contexts only.

## Success Criteria *(mandatory)*

- **SC-001**: A change in a federated cluster is reflected in the aggregate within 5s.
- **SC-002**: With 5 federated contexts idle, Fathom stays under 2% of one core and the UI meets the 100ms input target.
- **SC-003**: With an empty federated set, no additional clients or tasks exist (verified by test).
- **SC-004**: An unreachable context shows an error within 15s and recovers automatically within one backoff cycle of connectivity returning.
- **SC-005**: Restart restores the federated set with zero manual steps.

## Assumptions

- Builds on 012 P1 (context list/status) and 027 (fleet poller, summary type, classifier).
- Tier D P3 (cloud auth plugins) and P4 (daemon) are out of scope; the design must not preclude either.
- Combined totals sum only Live contexts and label how many clusters contribute.
