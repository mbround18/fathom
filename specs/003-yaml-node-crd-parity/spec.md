# Feature Specification: Resource Editing, Node Actions & Custom Resource Browsing

**Feature Branch**: `003-yaml-node-crd-parity`

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "Close the three biggest P0 feature-parity gaps against Lens/OpenLens/Headlamp/K9s/Rancher Dashboard: (1) Edit YAML + apply for any resource with a diff/preview step before applying, (2) Node actions — cordon, uncordon, drain, taint — gated behind the existing write-action confirmation UI, (3) Generic Custom Resource Definition browsing (not limited to the two hardcoded Flux/Longhorn integrations), read-only for this feature."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Edit a resource's YAML and apply the change (Priority: P1)

An operator viewing a Deployment, ConfigMap, or other resource in the Inspector needs to make a
targeted change (bump a replica count, fix an env var, patch an annotation) without leaving the
app or dropping to a separate `kubectl edit` terminal. They open the resource's YAML, edit it
in place, review exactly what will change against the live cluster object, and explicitly confirm
before anything is sent to the cluster.

**Why this priority**: This is the single most-used "power" capability in every competing
Kubernetes GUI (Lens, Headlamp, Rancher Dashboard, K8s Dashboard) and the most-cited gap: today
Fathom can only delete/scale/restart resources, not edit their spec directly. Operators
repeatedly need to change fields no dedicated action covers.

**Independent Test**: Can be fully tested by opening any resource's YAML tab, changing a value,
reviewing the diff, applying it, and confirming the live object reflects the change — independent
of node actions or CRD browsing.

**Acceptance Scenarios**:

1. **Given** a resource is selected and its YAML tab is open, **When** the operator chooses to
   edit, **Then** the YAML becomes editable and shows the resource exactly as currently stored on
   the cluster (including server-populated fields like `resourceVersion`).
2. **Given** the operator has changed one or more fields in the editor, **When** they choose to
   review the change, **Then** the system shows a diff between the original cluster object and the
   edited content before any request is sent.
3. **Given** a reviewed diff is showing, **When** the operator explicitly confirms apply, **Then**
   the system sends the change to the cluster and the operator sees a clear success indicator once
   the cluster accepts it.
4. **Given** the operator has made edits but not yet applied them, **When** they close the tab,
   switch resources, or otherwise navigate away, **Then** the system warns them their unapplied
   edits will be lost rather than silently discarding them.
5. **Given** the cluster object changed after the operator opened the editor (e.g. another operator
   or a controller updated it) but before they apply, **When** they attempt to apply, **Then** the
   system detects the conflict (stale `resourceVersion`) and tells the operator to re-open the
   latest version rather than silently overwriting the newer change.
6. **Given** the operator submits YAML that is not valid (malformed syntax, or the cluster rejects
   it as invalid for the resource's schema), **When** they attempt to apply, **Then** the system
   shows a specific, actionable error and leaves their edits in place for correction — it does not
   discard the edit or apply a partial change.

---

### User Story 2 - Cordon, drain, uncordon, and taint a node (Priority: P2)

A cluster operator preparing to perform maintenance on a node — a kernel upgrade, hardware repair,
scaling event — needs to mark the node unschedulable, safely evict its workloads respecting
disruption budgets, and later bring it back into service. They may also need to apply or remove a
taint to control what can schedule there.

**Why this priority**: Node lifecycle management is standard in every competing dashboard and is
the primary reason operators keep a separate `kubectl`/K9s session open even when using a GUI
dashboard for everything else. It's high-value but narrower in audience than resource editing
(cluster-admin-level operators, not every user), hence P2.

**Independent Test**: Can be fully tested by selecting a node, cordoning it (confirm it becomes
unschedulable), draining it (confirm its evictable pods are evicted and rescheduled elsewhere
while respecting disruption budgets), and uncordoning it (confirm it becomes schedulable again) —
independent of resource-YAML-editing or CRD browsing.

**Acceptance Scenarios**:

1. **Given** a schedulable node is selected, **When** the operator chooses to cordon it, **Then**
   the system asks for explicit confirmation, and once confirmed, the node is marked unschedulable
   and the UI reflects the new state without a page reload.
2. **Given** a cordoned node, **When** the operator chooses to uncordon it, **Then** the system
   asks for explicit confirmation and the node becomes schedulable again.
3. **Given** a node the operator wants to fully evacuate, **When** they choose to drain it and
   confirm, **Then** the system cordons it (if not already) and evicts its evictable pods one by
   one, respecting PodDisruptionBudgets, and shows live progress (e.g. "3 of 7 pods evicted").
4. **Given** a drain in progress where a pod cannot be evicted because doing so would violate a
   PodDisruptionBudget, **When** that pod blocks progress, **Then** the operator sees which pod is
   blocked and why, rather than the drain silently hanging with no explanation.
5. **Given** a node has pods managed by a DaemonSet, **When** the operator starts a drain, **Then**
   they are offered the option to ignore DaemonSet-managed pods (matching standard `kubectl drain
   --ignore-daemonsets` behavior), since those pods cannot be meaningfully evicted (the DaemonSet
   controller immediately recreates them on the same node).
6. **Given** the operator wants to control what can schedule onto a node, **When** they add or
   remove a taint (key, value, effect), **Then** the system asks for explicit confirmation before
   applying the change and reflects the current taints on the node afterward.
7. **Given** any of cordon/uncordon/drain/taint is requested, **When** the confirmation is shown,
   **Then** the system does not perform the action until the operator explicitly confirms — there
   is no one-click, no-confirmation path for any of these.

---

### User Story 3 - Browse any installed Custom Resource (Priority: P3)

An operator working on a cluster with CRDs beyond Fathom's two built-in integrations (Flux,
Longhorn) — e.g. cert-manager Certificates, Istio VirtualServices, Crossplane composites — needs
to see what custom resources exist, inspect one, and read its YAML, the same way they already can
for built-in kinds like Pods and Deployments.

**Why this priority**: Valuable and commonly requested, but narrower than P1/P2: it's read-only
discovery rather than an action operators are blocked without, and many clusters' day-to-day
operation doesn't require touching CRDs outside the two Fathom already understands. Hence P3.

**Independent Test**: Can be fully tested by connecting to a cluster with at least one
non-Flux/non-Longhorn CRD installed, confirming it appears in a discoverable list, and confirming
its instances can be listed and inspected — independent of resource editing or node actions.

**Acceptance Scenarios**:

1. **Given** a cluster has one or more CustomResourceDefinitions installed, **When** the operator
   opens the custom-resources section, **Then** they see every installed CRD listed by its kind and
   group, not only Flux/Longhorn kinds.
2. **Given** a CRD is selected from that list, **When** the operator views it, **Then** they see
   the custom resource instances that exist for it, scoped to the current namespace for
   namespace-scoped CRDs or cluster-wide for cluster-scoped ones, consistent with how built-in
   resource kinds are already scoped in the app.
3. **Given** a custom resource instance is selected, **When** the operator inspects it, **Then**
   they see its details and full YAML using the same inspector pattern already used for built-in
   kinds (Details/YAML/Events tabs).
4. **Given** a cluster has zero CRDs beyond the built-in Kubernetes types, **When** the operator
   opens the custom-resources section, **Then** they see a clear empty state, not an error.
5. **Given** the operator does not have list/get permission for a particular CRD's resources,
   **When** they attempt to view it, **Then** they see a clear permission-denied indicator for that
   CRD specifically, without breaking the rest of the custom-resources list.

### Edge Cases

- What happens if the operator's edited YAML changes the resource's `kind` or `apiVersion`, or its
  `name`/`namespace` (effectively targeting a different object)? The system MUST reject this as an
  invalid edit rather than attempting to create or redirect to a different object — YAML editing
  targets the one resource that was opened, not general resource creation.
- What happens when a drain is requested on a node that hosts pods with no controller (bare pods)?
  Bare pods cannot be safely evicted and rescheduled elsewhere; the operator MUST be told this
  explicitly (matching standard `kubectl drain` behavior of refusing bare pods unless force is
  used) rather than the drain silently skipping or hanging on them.
- What happens if the operator cancels a drain partway through? Already-evicted pods stay evicted
  (that action cannot be undone); the system MUST stop evicting further pods and clearly show how
  far it got.
- What happens if a CRD is deleted from the cluster while the operator is viewing its instances?
  The instance list MUST show a clear "this custom resource type is no longer available" state
  rather than an unexplained empty list or silent failure.
- What happens if the operator opens the YAML editor for a resource, and that resource is deleted
  by someone/something else before they apply? The apply MUST fail with a clear "resource no
  longer exists" error, not silently succeed or create a new object.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST allow an operator to open an editable view of any inspectable
  resource's YAML, pre-populated with its current live cluster content.
- **FR-002**: The system MUST show a diff between the resource's live cluster content and the
  operator's edited content before any change is sent to the cluster.
- **FR-003**: The system MUST require an explicit, separate confirmation step before applying an
  edited resource — edits are never applied as a side effect of editing or closing the diff view.
- **FR-004**: The system MUST detect when the live cluster object has changed since the editor was
  opened (a version conflict) and prevent applying without the operator explicitly re-reviewing the
  newer version.
- **FR-005**: The system MUST reject, with a specific error, an edit that changes the resource's
  identity (`kind`, `apiVersion`, `metadata.name`, or `metadata.namespace`).
- **FR-006**: The system MUST warn the operator before discarding unapplied YAML edits (navigating
  away, closing the tab/resource, switching resources).
- **FR-007**: The system MUST show a specific, actionable error when an apply is rejected by the
  cluster (validation failure, conflict, permission denied), and preserve the operator's edits so
  they can correct and retry.
- **FR-008**: The system MUST allow an operator to cordon and uncordon a node, each gated behind an
  explicit confirmation step.
- **FR-009**: The system MUST allow an operator to drain a node: cordon it if not already cordoned,
  then evict its evictable pods respecting PodDisruptionBudgets, gated behind an explicit
  confirmation step before starting.
- **FR-010**: The system MUST show live progress of an in-progress drain (e.g. pods evicted so far
  out of total) and MUST surface which specific pod is blocking progress and why, when eviction
  cannot proceed (e.g. a PodDisruptionBudget would be violated).
- **FR-011**: The system MUST offer an option to ignore DaemonSet-managed pods when draining a
  node, since they are recreated on the same node immediately after eviction.
- **FR-012**: The system MUST refuse to evict bare pods (pods with no owning controller) during a
  drain unless the operator explicitly opts in to force-evicting them, and MUST clearly identify
  which pods are bare when this applies.
- **FR-013**: The system MUST allow an operator to add or remove a taint (key, value, effect) on a
  node, gated behind an explicit confirmation step, and MUST reflect the node's current taints
  after the change.
- **FR-014**: The system MUST allow an in-progress drain to be cancelled, stopping further
  evictions while leaving already-evicted pods evicted, and MUST clearly report how far the drain
  got before it was cancelled.
- **FR-015**: The system MUST discover and list all CustomResourceDefinitions installed on the
  connected cluster, not only the two kinds (Flux, Longhorn) it has dedicated integrations for.
- **FR-016**: The system MUST allow an operator to list the custom resource instances of any
  discovered CRD, scoped to the current namespace when the CRD is namespace-scoped, or cluster-wide
  when it is cluster-scoped.
- **FR-017**: The system MUST allow an operator to inspect a custom resource instance's details and
  full YAML using the same inspector presentation already used for built-in resource kinds.
- **FR-018**: The system MUST show a clear empty state when a cluster has no CRDs beyond built-in
  Kubernetes types, and a clear "type no longer available" state if a CRD is removed while its
  instances are being viewed.
- **FR-019**: The system MUST show a clear, CRD-specific permission-denied indicator when the
  operator lacks list/get access to a particular custom resource type, without that failure
  affecting the rest of the custom-resources list.
- **FR-020**: Generic custom resource browsing introduced by this feature MUST remain read-only —
  it MUST NOT expose any create, delete, or field-level edit action for CRD instances beyond what
  the YAML-edit-and-apply capability (FR-001–FR-007) already provides once a custom resource
  instance is opened through the same generic resource inspector.
- **FR-021**: Every mutating action introduced by this feature (apply an edited resource, cordon,
  uncordon, drain, taint) MUST require explicit operator confirmation and MUST NOT be performed
  optimistically — the UI reflects the new state only after the cluster confirms the change
  succeeded.

### Key Entities

- **Resource Edit Session**: The in-progress editing state for a single resource — the resource
  being edited (kind, namespace, name), its original live content and version at the time editing
  started, and the operator's current edited content. Exists only client-side until applied;
  discarded on navigate-away (after warning) or on successful/failed apply.
- **Node Action**: A cordon, uncordon, drain, or taint request targeting one node. A drain has its
  own sub-state (in progress / evicted-so-far / blocked-on / cancelled / complete) distinct from
  the simpler one-shot cordon/uncordon/taint actions.
- **Custom Resource Definition (CRD) Catalog Entry**: A discovered CRD's identity (group, version,
  kind, scope — namespaced or cluster) and current availability, used to populate the
  custom-resources browsing list.
- **Custom Resource Instance**: A specific object of a discovered CRD kind, presented through the
  same generic resource details/YAML pattern already used for built-in kinds.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can go from "resource selected" to "edit applied and confirmed" for a
  simple field change (e.g. a replica count or annotation) in under 60 seconds, including reviewing
  the diff.
- **SC-002**: 100% of apply attempts that would change a resource's identity (kind/apiVersion/
  name/namespace) or that target a stale cluster version are rejected before any cluster mutation
  occurs — zero silent overwrites or misdirected applies in testing.
- **SC-003**: An operator can cordon a node, confirm it is unschedulable, and uncordon it again in
  under 30 seconds total, with zero ambiguity about the node's current schedulable state at any
  point.
- **SC-004**: A drain of a node with 10 evictable pods and no blocking PodDisruptionBudget
  completes with 100% of those pods evicted and rescheduled, with visible progress at every step.
- **SC-005**: When a drain is blocked by a PodDisruptionBudget, the operator can identify which
  pod and which budget is blocking it without consulting any tool outside the app.
- **SC-006**: An operator on a cluster with unfamiliar CRDs (not Flux/Longhorn) can find, list, and
  inspect the YAML of a custom resource instance without prior knowledge of its group/version/kind,
  in under 45 seconds from opening the custom-resources section.
- **SC-007**: Zero mutating actions introduced by this feature (apply, cordon, uncordon, drain,
  taint) execute without an explicit confirmation step having been shown and accepted by the
  operator, verified across all acceptance scenarios in testing.

## Assumptions

- The operator's configured cluster credentials already carry whatever RBAC is required for these
  actions (edit resources, cordon/drain/taint nodes, list CRDs and their instances); this feature
  does not add a permissions model of its own, it surfaces the cluster's own authorization
  decisions (allow, or a clear denial) rather than pre-checking or second-guessing them.
- "Apply" means a full-object update of the one resource that was opened (equivalent to
  `kubectl apply`/`replace` semantics on that object), not a three-way strategic merge across
  multiple concurrently-edited resources or a Helm-release-level operation.
- Drain follows standard `kubectl drain` semantics and defaults (respect PodDisruptionBudgets,
  offer to skip DaemonSet-managed pods, refuse bare pods without explicit force) since that is the
  behavior operators coming from `kubectl`/K9s/Lens already expect.
- Generic CRD browsing in this feature is discovery and read-only inspection only; creating,
  deleting, or editing custom resource instances beyond what the general YAML-edit-and-apply
  capability already provides, and any CRD-specific schema-aware form editing, are out of scope for
  this feature.
- Helm release management (install/upgrade/rollback/uninstall), a SelfSubjectAccessReview-style
  "who can do X" view, and a plugin/extension system are explicitly out of scope for this feature —
  each is a separate, larger gap identified in the broader parity analysis and will be specified
  independently if prioritized.
