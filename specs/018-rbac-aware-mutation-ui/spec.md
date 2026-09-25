# Feature Specification: Proactive RBAC-Aware Mutation UI

**Feature Branch**: `018-rbac-aware-mutation-ui`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Proactive RBAC-aware UI: hide or disable mutation controls (buttons, menu items) for resources/namespaces the current user genuinely lacks permission to mutate, determined before the confirm dialog opens (today the UI likely lets you click through to a confirm dialog and only fails server-side or at guard-check time). Extend spec 011's resolve_namespace_access() to answer 'can this principal do this specific action on this specific resource' (SelfSubjectAccessReview-equivalent semantics), and wire every existing mutation entry point (YAML apply, node actions, Helm upgrade/rollback/uninstall, RBAC admin itself) to check it and disable/hide accordingly, with a tooltip explaining why when disabled. Non-goal: this does not change what mutations are allowed server-side (RBAC enforcement is still the API server's job) — it's purely a UI-affordance improvement so operators don't hit an avoidable permission-denied error after already committing to a confirm dialog. Don't build a full permission-explorer UI, just gate existing controls."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Mutation controls reflect real permissions before the confirm dialog (Priority: P1)

An operator whose identity is bound to a read-only (or narrowly-scoped) Role opens a resource view
and looks for the actions they can take. Today the button/menu item for an action they cannot
perform (e.g. delete a Deployment, cordon a Node, uninstall a Helm release) is fully clickable and
only reveals the permission problem after the operator has already opened and dismissed a confirm
dialog and submitted the action, at which point the API server rejects it. This story makes the
affordance itself reflect the operator's real, current permissions: the control is disabled (or
hidden, per FR below) before any dialog opens, with an explanation of why.

**Why this priority**: This is the entire point of the feature. Every other story is scaffolding
around this one experience. Without it, nothing user-visible has changed.

**Independent Test**: Can be fully tested by authenticating as an identity bound to a role that
excludes `delete` on `deployments` in a given namespace, opening that namespace's workload view,
and confirming the delete control is disabled with an explanatory tooltip — while a `list`/`get`
action (still permitted) remains fully enabled. No cluster mutation occurs during this test.

**Acceptance Scenarios**:

1. **Given** an operator's effective RBAC grants do not include the verb required for a mutation
   action on a specific resource/namespace, **When** the operator views the control for that
   action, **Then** the control is disabled (not removed, unless FR-003 hiding applies) and, on
   hover/focus, shows a tooltip stating that the action is not permitted and which verb/resource
   is missing.
2. **Given** an operator's effective RBAC grants do include the verb required, **When** the
   operator views the control, **Then** the control is enabled exactly as it behaves today, and
   clicking it proceeds to the existing confirm dialog and submission flow unchanged.
3. **Given** the backend cannot currently determine the operator's effective access (e.g. the
   access-resolution call has not completed yet, or errored), **When** the operator views a
   mutation control, **Then** the system does not silently pretend the action is allowed — it
   either shows the control in a loading/indeterminate state or falls back to enabled-with-normal
   server-side enforcement (see Assumptions), but it never fabricates a false "permitted" tooltip.

---

### User Story 2 - One access-resolution mechanism powers every mutation surface (Priority: P2)

A maintainer extending or auditing the codebase wants a single, well-tested place that answers
"can the current principal perform verb V on resource R (optionally in namespace N)?", reused by
every mutation entry point (YAML apply, node actions, Helm upgrade/rollback/uninstall, RBAC admin
itself), rather than each panel inventing its own ad hoc permission logic.

**Why this priority**: Without a shared mechanism, each of the ~4+ mutation surfaces would grow
its own slightly-different permission check, which is both extra work and a correctness risk (one
surface silently drifting out of sync with what the cluster actually enforces).

**Independent Test**: Can be fully tested by exercising the shared access-check entry point
directly (unit tests) against fixture Role/RoleBinding/ClusterRole/ClusterRoleBinding data and
confirming it returns the correct allow/deny verdict for a given (verb, resource, namespace)
tuple, independent of any specific UI panel.

**Acceptance Scenarios**:

1. **Given** fixture RBAC objects granting `update` on `deployments` in namespace `team-a` only,
   **When** the shared check is asked about `update`/`deployments`/`team-a`, **Then** it reports
   allowed; asked about `update`/`deployments`/`team-b`, it reports not allowed.
2. **Given** a ClusterRoleBinding granting a verb cluster-wide, **When** the shared check is asked
   about that verb in any namespace, **Then** it reports allowed for all namespaces (consistent
   with spec 011's existing cluster-wide semantics).
3. **Given** each of the four mutation surfaces (YAML apply, node actions, Helm actions, RBAC
   admin), **When** their code is inspected, **Then** each calls the same shared check rather than
   a locally duplicated one.

---

### User Story 3 - Cluster-scoped actions (node cordon/drain, RBAC admin edits) are also gated (Priority: P3)

An operator without cluster-scoped mutate permissions (e.g. no `patch`/`update` on `nodes`, or no
`create`/`update` on `rolebindings`) opens the Node actions panel or the RBAC administration
surface and sees those specific controls disabled with an explanation, consistent with how
namespaced resource actions behave in User Story 1.

**Why this priority**: Cluster-scoped resources are a smaller surface than namespaced workloads
but are equally prone to the same "click through to a rejected confirm dialog" problem, and are
explicitly named as in-scope entry points.

**Independent Test**: Can be fully tested by authenticating as an identity without `patch` on
`nodes`, opening the Node actions panel, and confirming cordon/drain/uncordon controls are
disabled with a tooltip, while an identity with that grant sees them enabled.

**Acceptance Scenarios**:

1. **Given** an operator lacks the verb needed for a Node action, **When** they open the Node
   actions panel, **Then** the relevant action control is disabled with an explanatory tooltip.
2. **Given** an operator lacks `create`/`update`/`delete` on `rolebindings`/`clusterrolebindings`,
   **When** they open the RBAC administration surface's binding-mutation controls, **Then** those
   controls are disabled with an explanatory tooltip, without affecting the surface's read-only
   browsing capability from spec 011.

---

### Edge Cases

- What happens when the operator's access changes mid-session (e.g. an admin edits their
  RoleBinding while they have a panel open)? The UI reflects the last-resolved access snapshot;
  it does not need to live-watch RBAC objects, but the existing refresh points (e.g. reopening the
  panel, existing periodic refresh where applicable) MUST re-resolve access rather than caching
  it indefinitely.
- What happens for actions that don't map cleanly onto a single Kubernetes `resource`/`verb` (e.g.
  a Helm uninstall, which is a client-side operation with cascading Kubernetes object deletes)?
  The system MUST use the closest reasonable resource/verb mapping (e.g. Helm release mutation
  actions map to `delete`/`secrets` or `update`/`secrets` in the release's storage namespace,
  since Helm stores release state as Secrets) and document that mapping, rather than skipping the
  check for those actions entirely.
- What happens when the access-resolution check itself errors (e.g. the API server is briefly
  unreachable)? Per Acceptance Scenario 1.3, the UI must not claim false certainty either way; it
  degrades to a loading/indeterminate state or to today's behavior (enabled, server-enforced) and
  never shows a misleading "permission granted" tooltip when the check actually failed.
- What happens for a cluster-admin identity with blanket wildcard (`*`) grants? All controls
  remain enabled, matching today's behavior — this feature only ever removes/disables affordances,
  it never adds friction for identities that already have full access.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a single mechanism, extending spec 011's namespace-access
  resolution, capable of answering "can the current principal perform verb V on resource R
  (optionally scoped to namespace N)?" for a specific action, not just an enumeration of all
  granted verbs/resources.
- **FR-002**: The system MUST evaluate this check using the same effective-RBAC semantics already
  established in spec 011 (Role/ClusterRole/RoleBinding/ClusterRoleBinding resolution, cluster-wide
  vs. per-namespace handling, dangling-binding tolerance) rather than a separate/parallel
  authorization model.
- **FR-003**: For every existing mutation entry point in scope (YAML apply, node actions [cordon,
  drain, uncordon, delete], Helm actions [upgrade, rollback, uninstall], RBAC admin binding
  create/edit/delete), the system MUST disable the corresponding UI control when the check
  reports the action is not permitted, rather than only relying on server-side rejection.
- **FR-004**: A disabled mutation control MUST expose a tooltip (or equivalent accessible
  explanation) stating that the action is not permitted for the current identity, including which
  verb and resource (and namespace, where applicable) is missing.
- **FR-005**: The system MUST NOT change server-side enforcement of any mutation — the Kubernetes
  API server (or Helm's own state store) remains the sole authority on whether an action actually
  succeeds; this feature only affects whether the client-side control is presented as available.
- **FR-006**: When the access-resolution result for a given action is not yet available or has
  errored, the system MUST NOT present a disabled control with a tooltip falsely claiming
  "not permitted" — it MUST either show an indeterminate/loading affordance or fall back to the
  pre-feature behavior (control enabled, subject to normal server-side/guard-check enforcement).
- **FR-007**: The system MUST re-resolve access at the existing natural refresh points of each
  surface (e.g. panel open/reopen, existing periodic refresh) rather than caching a single
  resolution indefinitely for the lifetime of the app session.
- **FR-008**: The system MUST map non-native-Kubernetes-verb actions (specifically Helm
  upgrade/rollback/uninstall) onto the closest equivalent Kubernetes resource/verb check (Helm
  release state is stored as `Secret` objects in the release namespace) and document that mapping
  for maintainers.
- **FR-009**: The system MUST continue to allow all existing mutation flows unchanged for
  identities that already have full access (no new confirmation steps, no new friction) — this
  feature is strictly permission-based hiding/disabling, not a new workflow layer.
- **FR-010**: The RBAC administration surface's own binding-mutation controls (create/edit/delete
  RoleBinding/ClusterRoleBinding) MUST themselves be gated by the same mechanism, consistent with
  every other mutation entry point in scope.

### Key Entities

- **Action Access Check**: A single (verb, resource, optional namespace, optional resource name)
  tuple representing one specific mutation a UI control would perform, and the yes/no/unknown
  verdict plus human-readable reason produced for it.
- **Mutation Control Binding**: The association between a specific UI control (button/menu item)
  in a specific panel and the Action Access Check(s) that gate it.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator without mutate permission on a given resource/namespace sees the
  relevant action control disabled before ever opening a confirm dialog, in 100% of the in-scope
  mutation surfaces (YAML apply, node actions, Helm actions, RBAC admin).
- **SC-002**: An operator's confirm-dialog-stage "permission denied" errors for actions they could
  have discovered were disallowed via the UI (i.e. excluding genuine mid-flight permission
  changes or server-side edge cases) are reduced to effectively zero in normal operation.
- **SC-003**: An operator with full access experiences no behavioral change: every previously
  enabled control remains enabled and every existing confirm-and-submit flow completes exactly as
  before.
- **SC-004**: Every disabled mutation control provides an explanation (verb/resource/namespace)
  that a reasonable operator can read and understand without consulting external documentation.

## Assumptions

- Spec 011's `resolve_namespace_access()` and its underlying Role/ClusterRole/RoleBinding/
  ClusterRoleBinding read mechanism is the correct foundation to extend; this feature adds a
  point-lookup ("can I do X on Y") capability on top of it rather than replacing it.
- When the access-resolution result is unavailable (loading or errored), falling back to the
  pre-feature enabled state (server-side/guard-check enforcement only) is an acceptable and safer
  default than either blocking all mutations app-wide or showing a false negative.
- Helm mutation actions (upgrade/rollback/uninstall) are reasonably approximated by
  `update`/`delete` verb checks against `secrets` in the release's namespace, since that is how
  Helm persists release state as Kubernetes objects; this is a documented approximation, not an
  exact modeling of Helm's internal RBAC needs.
- "Hide" vs. "disable" for a not-permitted control: this feature disables (keeps visible but
  inactive, with a tooltip) rather than hides, since a hidden control gives the operator no
  information about what capability exists but is unavailable to them — consistent with FR-004's
  requirement for an explanation.
- This feature does not need to watch RBAC objects live; re-resolving at existing natural refresh
  points (panel open, existing periodic refresh) is sufficient per Edge Cases.
