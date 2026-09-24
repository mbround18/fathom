# Feature Specification: Local Mutation Audit-Log Viewer

**Feature Branch**: `026-mutation-audit-log-viewer`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Local mutation audit-log viewer — persist an append-only local record of every mutating action Fathom performs (what, who/which context, when, target resource, outcome), and add a viewer pane to browse/filter/search that history. Since every mutation already emits a completion event via the existing guard pattern, this is additive: hook a persistence layer onto that existing emission point and add read-only local storage reusing whatever mechanism already persists things outside the cluster. No cluster-side audit log reading, no export/sharing, no retention-policy configuration UI."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Review recent mutating actions across all contexts (Priority: P1)

As an operator who runs mutating actions against clusters through Fathom (applying YAML, upgrading/rolling back Helm releases, cordoning/draining/tainting nodes, changing RBAC), I want to open a single history pane and see a chronological list of every mutating action Fathom has performed locally, regardless of which cluster context I was connected to at the time, so I can reconstruct "what did I (or my team) do and when" without digging through cluster events or my own memory.

**Why this priority**: This is the core value of the feature — without a browsable record, nothing else (filtering, searching) matters. It must work standalone as a minimum viable slice.

**Independent Test**: Perform a mutating action (e.g. cordon a node), open the audit-log viewer pane, and confirm the action appears with its target resource, the cluster context it ran against, a timestamp, and its outcome (success/failure).

**Acceptance Scenarios**:

1. **Given** Fathom has just completed a mutating action, **When** the user opens the audit-log viewer, **Then** the action appears at the top of the list with action type, target resource, cluster/context, timestamp, and outcome.
2. **Given** the user has performed mutating actions across two different cluster contexts, **When** they open the viewer without applying any filter, **Then** entries from both contexts are shown together in a single chronological list.
3. **Given** a mutating action fails (e.g. a Helm upgrade is rejected), **When** the user opens the viewer, **Then** the failed action appears with a clearly distinguishable "failed" outcome and any available failure reason.

---

### User Story 2 - Filter and search the history to find a specific action (Priority: P2)

As an operator investigating a specific incident or change, I want to filter and search the audit-log history (by action type, target resource/namespace, cluster context, outcome, and time range, and by free-text search over the resource name) so I can quickly locate the one or few entries relevant to my investigation instead of scrolling a long list.

**Why this priority**: Once history exists (P1), the list will grow quickly; filtering/search is what makes the history actually usable rather than a dumping ground.

**Independent Test**: With a history containing entries of multiple action types and outcomes, apply a filter (e.g. action type = "node cordon", or outcome = "failed", or a text search matching a resource name) and confirm only matching entries remain visible, and confirm clearing the filter restores the full list.

**Acceptance Scenarios**:

1. **Given** a history containing multiple action types, **When** the user filters by a specific action type, **Then** only entries of that type are shown.
2. **Given** a history containing both successful and failed actions, **When** the user filters by outcome = failed, **Then** only failed entries are shown.
3. **Given** a history spanning multiple cluster contexts, **When** the user filters by a specific context, **Then** only entries from that context are shown.
4. **Given** the user types a search term matching part of a target resource's name, **When** the search is applied, **Then** only entries whose target resource matches are shown, and the match is case-insensitive.
5. **Given** an active filter/search, **When** the user clears it, **Then** the full (unfiltered, retention-bounded) history is shown again.

---

### User Story 3 - Trust that the history reflects local retention limits, not a silent gap (Priority: P3)

As an operator relying on this history for accountability, I want to understand that the log is local-only, retains a bounded window of recent activity (not unlimited history), and is not a substitute for the cluster's own audit trail, so I don't mistake it for a complete or cluster-wide compliance record.

**Why this priority**: Lower priority because it's about correct expectations rather than core functionality, but important to avoid the feature being misused or misunderstood as a compliance/export tool.

**Independent Test**: Generate enough mutating actions to exceed the fixed retention limit, then confirm the oldest entries beyond that limit are no longer present in the viewer, and confirm the viewer surfaces no cluster-side (Kubernetes API server) audit events — only Fathom-initiated actions.

**Acceptance Scenarios**:

1. **Given** the number of recorded mutating actions has reached the fixed retention limit, **When** a new mutating action completes, **Then** the oldest entry beyond the limit is no longer shown in the viewer.
2. **Given** the user opens the audit-log viewer, **When** they look at any entry, **Then** every visible entry corresponds to an action Fathom itself performed (never a cluster-side/Kubernetes audit-log event Fathom did not initiate).

### Edge Cases

- What happens when a mutating action is still in progress (not yet completed) when the viewer is opened? It should not yet appear as a completed entry; only actions that have reached a terminal outcome (success or failure) are recorded.
- How does the system handle a mutating action whose target resource or context information is only partially available at completion time (e.g. connection dropped mid-action)? The entry should still be recorded with whatever fields are known, marked with the best-determinable outcome (failure, unless proven otherwise).
- What happens if local storage is unavailable/corrupted at startup? The viewer should show an empty or degraded history and Fathom's normal mutating-action functionality must not be blocked — audit persistence is best-effort and never gates a mutation from proceeding.
- What happens when the user has never performed a mutating action? The viewer shows an empty state explaining no actions have been recorded yet.
- What happens when filters/search match zero entries? The viewer shows a clear "no matching entries" state distinct from "no history exists at all."

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST record a local, append-only history entry for every mutating action performed by Fathom upon that action reaching a terminal outcome (success or failure), with no user opt-in/opt-out required.
- **FR-002**: Each recorded entry MUST capture: the action type (e.g. YAML apply, Helm upgrade/rollback/uninstall, node cordon/drain/taint, RBAC change, etc.), the acting user/session context, the cluster/context the action targeted, the specific target resource (kind, name, and namespace where applicable), the timestamp the action completed, and the outcome (success or failure, with a failure reason when available).
- **FR-003**: System MUST provide a dedicated viewer pane where the user can browse the recorded history in reverse-chronological order (most recent first).
- **FR-004**: Users MUST be able to filter the history by action type, outcome, and cluster/context.
- **FR-005**: Users MUST be able to search the history by free-text matching against the target resource name, case-insensitively.
- **FR-006**: Users MUST be able to filter the history by a time range (e.g. last hour/day/week or a custom range).
- **FR-007**: System MUST retain only a bounded, fixed amount of history (a fixed count of most-recent entries and/or a fixed time window) and automatically discard entries beyond that bound as new entries are recorded; this bound is not user-configurable in v1.
- **FR-008**: System MUST persist the history locally on the machine running Fathom, independent of any cluster, and MUST NOT read, display, or merge in any cluster-side (Kubernetes API server) audit log data.
- **FR-009**: System MUST record failed mutating actions (not only successful ones) in the history, distinguishably marked as failed.
- **FR-010**: The audit-log viewer MUST be read-only: it MUST NOT provide the ability to edit or delete individual history entries, re-run a past action, export/share the history, or configure retention settings, in v1.
- **FR-011**: Failure of the audit-persistence layer (e.g. local storage unavailable) MUST NOT prevent or delay any mutating action from proceeding; audit recording is best-effort and must fail silently from the end user's perspective (the mutation itself still succeeds or fails on its own merits).
- **FR-012**: History entries MUST survive an application restart (the history persists across Fathom sessions, up to the retention bound).

### Key Entities

- **Audit Log Entry**: A single record of one completed mutating action. Attributes: unique identifier, action type/category, acting user/session context, cluster/context identifier, target resource descriptor (kind, namespace, name), completion timestamp, outcome (success/failure), and failure reason (when applicable and available).
- **Audit Log Filter/Search State**: The transient, viewer-local combination of active filters (action type, outcome, context, time range) and free-text search term applied to the displayed list; not persisted beyond the current viewing session.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every mutating action performed through Fathom appears in the audit-log viewer within 2 seconds of the action completing, with 100% of the required fields (type, context, target, timestamp, outcome) populated when available at completion time.
- **SC-002**: A user can locate a specific past action (by type, outcome, context, or resource-name search) among at least 200 history entries in under 10 seconds.
- **SC-003**: Recording a mutating action's history entry adds no perceptible delay (under 100ms) to the completion of that action from the user's perspective.
- **SC-004**: A user restarting Fathom sees their prior session's history still present in the viewer (up to the retention bound), with zero data loss for entries within that bound.
- **SC-005**: When local audit storage is deliberately made unavailable, 100% of mutating actions still complete normally (audit failure never blocks or alters a mutation's outcome).

## Assumptions

- The existing shared "guard pattern" that every mutating action already flows through, and which already emits a completion event, is the single point of truth for "a mutating action happened, with this outcome" — this feature observes/hooks that existing event rather than instrumenting each mutation type individually.
- Fathom already has some local, outside-the-cluster persistence mechanism (used for at least one other feature, such as team/cluster profile settings) that this feature should reuse for storing history entries, rather than introducing a new persistence technology, unless that existing mechanism is genuinely unsuited to an append-only, growing log (to be confirmed during planning).
- "Fixed retention default" means a specific, hardcoded bound (e.g. most recent N entries, such as 500, or most recent N days, such as 30) chosen during planning/implementation and documented, not exposed as a user setting in v1.
- "Who/which context" refers to the local user/session identity Fathom already knows about (e.g. the active team member/profile and the active cluster context), not a new authentication or identity system.
- The viewer is a new pane within Fathom's existing three-pane shell UI, consistent with how other read-only views (e.g. existing list/detail views) are presented, not a separate window or application.
- History entries are per-installation (local to the machine running Fathom), not synced across machines or team members.
