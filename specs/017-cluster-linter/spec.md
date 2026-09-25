# Feature Specification: Cluster Sanitizer / Linter

**Feature Branch**: `017-cluster-linter`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Popeye-style cluster sanitizer/linter — a read-only static-analysis pane over resources Fathom already fetches: flags missing requests/limits, missing liveness/readiness/startup probes, deprecated API versions, unused ConfigMaps/Secrets/ServiceAccounts, naked pods (no owning controller), RBAC over-permissioning (wildcard verbs/resources), and missing PodDisruptionBudgets on multi-replica workloads. Zero new cluster reads, zero mutation surface — pure client-side (or backend, your call per plan) analysis of already-streamed data, scored findings, click-through to the offending resource's Inspector. Comparable shippability to spec 007 (topology). Non-goals: no auto-fix/remediation actions (read-only findings only, this spec doesn't add a mutation path), no custom rule authoring UI (fixed rule set for v1), no historical trend tracking."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See cluster health at a glance (Priority: P1)

An operator connected to a cluster in Fathom opens a "Linter" pane and sees a scored list of findings — each finding names a rule, a severity, and the specific resource (namespace/kind/name) that triggered it — computed entirely from resources Fathom has already streamed into its cache, with no additional cluster calls and no waiting on a new fetch.

**Why this priority**: This is the entire value proposition of the feature — surfacing latent misconfiguration risk that would otherwise require a separate CLI tool (Popeye) or manual review. Without this, there is no feature.

**Independent Test**: Connect to a cluster containing at least one resource that violates a rule (e.g., a pod with no resource limits). Open the Linter pane and confirm the finding appears with rule name, severity, and resource identity, without triggering any new watch/list calls to the cluster.

**Acceptance Scenarios**:

1. **Given** a connected cluster with a pod that has no `resources.limits` set, **When** the user opens the Linter pane, **Then** a finding is listed identifying that pod, the missing-limits rule, and a severity level.
2. **Given** a connected cluster with no rule violations among cached resources, **When** the user opens the Linter pane, **Then** the pane shows a clean/no-findings state rather than an empty or broken view.
3. **Given** the Linter pane is open while the underlying resource cache updates (e.g., a new pod is created), **When** the cache updates, **Then** findings recompute to reflect the current cache state without the user manually refreshing.

---

### User Story 2 - Jump from a finding to the offending resource (Priority: P1)

An operator reviewing findings clicks on one and is taken directly to that resource's Inspector view, with the resource already selected/loaded, so they can investigate and (outside this feature) remediate manually.

**Why this priority**: A list of findings with no path to action is far less useful — click-through to Inspector is what makes the pane a workflow tool rather than a static report, and it's explicitly called out as required in the feature description.

**Independent Test**: With at least one finding present, click it and confirm the Inspector opens scoped to the exact resource (namespace, kind, name) named in the finding.

**Acceptance Scenarios**:

1. **Given** a finding for a specific Pod, **When** the user clicks the finding, **Then** the Inspector pane opens showing that Pod's detail view.
2. **Given** a finding for a cluster-scoped resource (e.g., a ClusterRole), **When** the user clicks the finding, **Then** the Inspector opens showing that ClusterRole, without requiring a namespace to be selected first.

---

### User Story 3 - Understand and prioritize findings by severity and category (Priority: P2)

An operator with many findings wants to triage by severity (e.g., "show me only high severity") and by rule category (e.g., "just RBAC issues") so they can focus remediation effort.

**Why this priority**: Valuable for larger clusters where dozens of findings could otherwise overwhelm the pane, but the feature still delivers value at P1 scope even without filtering (a small/demo cluster is usable as a flat list).

**Independent Test**: With findings spanning multiple severities and categories, apply a severity filter and a category filter independently and confirm the visible list narrows correctly and a summary count/score updates accordingly.

**Acceptance Scenarios**:

1. **Given** findings of mixed severity, **When** the user filters to "high" severity only, **Then** only high-severity findings remain visible and the displayed count reflects the filtered set.
2. **Given** findings across multiple rule categories, **When** the user filters to a single category (e.g., RBAC), **Then** only findings from that category remain visible.
3. **Given** an active filter, **When** the user clears it, **Then** the full finding set is restored.

---

### Edge Cases

- What happens when a resource kind the linter depends on (e.g., PodDisruptionBudgets, RBAC bindings) has not been fetched/watched for the current cluster/namespace scope (e.g., user lacks RBAC to list it)? The affected rule(s) MUST be skipped for that data (not reported as false "violations"), and the pane MUST indicate that some checks were skipped due to limited visibility rather than silently under-reporting.
- How does the system handle a workload managed by a controller Fathom doesn't recognize (e.g., a CRD-based operator) when checking "naked pod" ownership? A pod with any `ownerReferences` entry is treated as owned, regardless of whether Fathom recognizes the owning kind — only pods with zero owner references are flagged as naked.
- How does the system handle multi-replica detection for workload kinds beyond Deployment/StatefulSet (e.g., a ReplicaSet not owned by a Deployment, or a raw scale on a custom resource)? Only Deployments, StatefulSets, and ReplicaSets not owned by a Deployment are evaluated for the missing-PodDisruptionBudget rule in v1; other multi-replica-capable kinds are out of scope.
- What happens when the same underlying misconfiguration would otherwise produce duplicate findings (e.g., a Deployment and its own generated ReplicaSet both lacking limits)? Findings are attributed to the resource that directly owns the pod template judged non-compliant (the Deployment/StatefulSet/DaemonSet, not its generated ReplicaSet), avoiding duplicate noise.
- What happens if the user switches clusters/contexts while the Linter pane is open? Findings are cleared and recomputed against the newly active cluster's cached resources.
- How does scoring handle a cluster with zero applicable resources for a given rule (e.g., no RBAC bindings exist at all)? The rule contributes no findings and is excluded from the score's denominator rather than counted as a pass or a violation.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a dedicated Linter pane, accessible from Fathom's existing navigation, that lists findings computed from resources already present in Fathom's cluster resource cache.
- **FR-002**: System MUST NOT introduce any new backend command, protocol surface, or Kubernetes
  API access pattern to populate the Linter pane. It MUST reuse only commands that already exist
  in Fathom's protocol today — both the bulk list refreshes existing views already trigger, and
  the per-object detail reads (e.g., selecting a Pod, Role, ClusterRole, or workload) that already
  exist for the Inspector but today only fire when an operator manually opens an object. The
  Linter MAY be the first caller to trigger those existing per-object reads automatically, for
  objects already visible in the current list scope, in order to evaluate rules whose data (e.g.
  a Pod's resource requests/limits and probes, a Role's policy rules) is only present in the
  per-object detail shape, not the lightweight list-row shape.
- **FR-003**: System MUST NOT expose any action from the Linter pane that mutates, patches, deletes, or otherwise changes a cluster resource; the pane is strictly read-only/informational.
- **FR-004**: System MUST evaluate the following fixed rule set for v1:
  - Workload containers missing CPU/memory requests and/or limits.
  - Pods missing liveness, readiness, or startup probes where the pod is part of a multi-replica or long-running workload.
  - Resources using a deprecated/removed Kubernetes API version (relative to the connected cluster's server version where available).
  - ConfigMaps and Secrets that are not referenced by any Pod, workload spec, or other resource Fathom has cached (env, envFrom, volume, or projected volume references).
  - ServiceAccounts that are not referenced by any Pod's `serviceAccountName` and are not the namespace default.
  - "Naked" Pods — Pods with no `ownerReferences` (not created by a Deployment/StatefulSet/DaemonSet/Job/ReplicaSet/CronJob or other controller).
  - RBAC Roles/ClusterRoles that grant wildcard (`*`) verbs or wildcard resources.
  - Deployments/StatefulSets/ReplicaSets (not owned by a Deployment) with more than one desired replica that have no matching PodDisruptionBudget.
- **FR-005**: Each finding MUST include: the rule that produced it, a human-readable description, a severity level, and a reference to the specific offending resource (kind, namespace if applicable, name).
- **FR-006**: System MUST allow the user to navigate from a finding directly to the existing Inspector view for the referenced resource, with that resource selected/loaded.
- **FR-007**: System MUST compute an overall cluster/namespace-scoped score or summary (e.g., counts by severity) derived from current findings, and update it whenever findings recompute.
- **FR-008**: System MUST recompute findings automatically when the underlying cached resources change (e.g., a watch event adds/updates/removes a relevant resource) while the pane is open, without requiring a manual user-triggered refresh.
- **FR-009**: System MUST allow the user to filter the visible findings list by severity and by rule category.
- **FR-010**: System MUST clearly indicate, per rule or overall, when a rule could not be evaluated because the resource kind it depends on is not present in the current cache (e.g., due to RBAC visibility restrictions), rather than silently omitting or misreporting it as clean.
- **FR-011**: System MUST scope findings to whatever cluster/namespace scope is currently active in Fathom (i.e., respect existing namespace filtering/selection), consistent with other panes.
- **FR-012**: System MUST NOT provide any UI for authoring, editing, enabling/disabling, or reordering individual rules in v1 — the rule set is fixed and ships as part of the application.
- **FR-013**: System MUST NOT persist or display historical trends of findings over time in v1 — only the current, live-computed state is shown.
- **FR-014**: System MUST recompute and clear findings when the active cluster/context connection changes.

### Key Entities

- **Finding**: A single detected issue. Attributes: rule identifier, category (e.g., resource-limits, probes, api-deprecation, unused-object, naked-pod, rbac, pdb), severity (e.g., high/medium/low), human-readable message, and a reference to the offending resource (kind, namespace, name, and enough identity to open it in the Inspector).
- **Rule**: A fixed, built-in check definition (one of the eight categories in FR-004) that consumes cached resources of one or more kinds and produces zero or more Findings.
- **Lint Report**: The aggregate result for the current cluster/namespace scope at a point in time: the full set of current Findings, a summary/score, and a list of any rules that were skipped due to missing visibility into a required resource kind.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can open the Linter pane against a typical namespace (dozens, not
  thousands, of objects) and see findings for every rule category fully populated within 5
  seconds, with findings for rule categories that need no per-object detail fetch (RBAC-adjacent
  list-only checks, unused ConfigMap/Secret presence) appearing immediately and other categories
  filling in progressively as their underlying per-object reads complete, rather than the pane
  appearing empty or stuck the whole time.
- **SC-002**: 100% of findings link to an Inspector view that correctly identifies the exact resource that produced the finding, verified across all eight rule categories.
- **SC-003**: Findings reflect cache changes (a new violating resource appearing, or an existing violation being resolved) within the same latency the rest of Fathom's UI already uses to reflect watch updates, with no manual refresh required.
- **SC-004**: An operator with a cluster containing 5+ distinct known-bad resources across at least 4 of the 8 rule categories can identify all of them from the Linter pane without consulting any other tool.
- **SC-005**: Zero cluster mutation requests are ever issued as a result of any interaction within the Linter pane, verified by code/architecture review (no mutate-capable command wired to any Linter-pane control).

- What happens in a very large "All Namespaces" scope where auto-fetching per-object detail for
  every Pod/Role/ClusterRole/workload would be expensive? The Linter fetches detail for the
  currently active namespace scope only (respecting FR-011); switching to "All Namespaces" fetches
  detail in bounded, throttled batches and the pane shows a visible "still evaluating N objects"
  progress indicator rather than blocking or silently doing nothing, consistent with SC-001's
  progressive-fill behavior.

## Assumptions

- Fathom already streams/caches, for the active cluster connection, the resource kinds needed by these rules (Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, ConfigMaps, Secrets, ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings, PodDisruptionBudgets) at least at the level already exposed by existing views (e.g., RBAC administration, workloads). If a given kind is not currently cached for a user's RBAC scope, the corresponding rule(s) degrade to "skipped" per FR-010 rather than blocking the whole pane.
- "Deprecated API version" detection uses a static, ships-with-the-app mapping of known deprecated/removed `apiVersion` + `kind` combinations (as documented by upstream Kubernetes deprecation guides current as of build time), checked against each object's `metadata.managedFields[].apiVersion` entries (the standard Kubernetes mechanism for recording which API version each field manager actually wrote with, since the API server always serves an object's current representation in whichever version was requested, not the version it was created with) — not a live query against the cluster's OpenAPI/discovery document beyond what Fathom already fetches.
- Rules that need data only present in an object's detail shape (resource requests/limits and probes for Pods; policy rules for Roles/ClusterRoles; pod-template label selector for workloads; `managedFields` for API-version history) trigger Fathom's existing per-object detail-read commands (e.g. `SelectPod`, `SelectRole`) for objects in the active scope, since the bulk list commands only return lightweight summaries. This is the one respect in which the Linter causes more cluster reads to happen than would occur if the pane were never opened — it uses no new command or protocol capability, only existing per-object reads, scoped and throttled per the Edge Cases above.
- Severity levels and default category groupings follow common static-analysis conventions (e.g., high = security/availability risk such as wildcard RBAC or naked pods in production namespaces; medium = missing probes/limits; low = unused objects) and are fixed constants for v1, not user-configurable.
- The Linter pane is a single pane comparable in scope/complexity to the existing topology pane (spec 007) — one primary view with list + filter + summary, not a multi-page dashboard.
- Scoring is a simple derived summary (e.g., counts per severity, or a percentage of passing checks) rather than a novel scoring algorithm requiring its own specification.
