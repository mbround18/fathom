# Feature Specification: Cluster-Admin RBAC / OIDC-OAuth Administration

**Feature Branch**: `011-rbac-oidc-administration`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Cluster-admin RBAC / OIDC-OAuth administration (MVP.md §14 Tier C): a UI/workflow layer over Kubernetes' own OIDC-auth and RBAC primitives, not a parallel auth system. Provide a cluster-admin-only OIDC issuer/audience/claim-mapping configuration surface (live-edit where the platform allows it, otherwise a guided doc/config-generation flow, since most managed Kubernetes and kubeadm clusters require API-server flag changes that are not runtime-editable via the Kubernetes API); a role-assignment UI to browse Role/ClusterRole objects and create/edit RoleBinding/ClusterRoleBinding objects mapping OIDC groups/claims to roles, reusing the existing YAML-apply diff-preview/confirmation pattern from 003-yaml-node-crd-parity; and a read-only namespace-assignment view showing which namespaces a given identity can act in and with what verbs. Ties into Tier B's RBAC-scoped UI item. Design constraint: prefer Kubernetes-native primitives (OIDC, RBAC objects, kubeconfig contexts) over inventing new protocols."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Browse existing Roles and ClusterRoles (Priority: P1)

A cluster-admin operator wants to understand what permission sets already exist on the cluster
before assigning any of them to an identity, so they can pick the right one (or confirm none of
the existing ones fit) instead of guessing from `kubectl get clusterrole -o yaml`.

**Why this priority**: Every other story in this feature (creating bindings, resolving namespace
access) depends on the operator being able to see what roles exist and what they actually grant.
Without this, the role-assignment UI would ask the operator to type a role name blind.

**Independent Test**: Can be fully tested by opening the RBAC administration surface, browsing the
list of Roles (namespace-scoped) and ClusterRoles (cluster-scoped), and opening one to see its
resolved rules (API groups, resources, verbs) — no binding needs to exist yet, and no mutation
occurs.

**Acceptance Scenarios**:

1. **Given** the operator has opened the RBAC administration surface, **When** they view the
   Roles/ClusterRoles list, **Then** the system shows every `Role` (grouped or filterable by
   namespace) and `ClusterRole` visible to the operator's current identity, with a clear visual
   distinction between namespace-scoped and cluster-scoped entries.
2. **Given** the list is showing, **When** the operator selects a specific Role or ClusterRole,
   **Then** the system shows its resolved rule set (API groups, resources, resource names where
   set, and verbs) in a readable form, not raw unformatted YAML only.
3. **Given** the operator's identity cannot list Roles/ClusterRoles in a given namespace (or
   cluster-wide for ClusterRoles), **When** they open the surface, **Then** the system shows
   exactly which namespaces/scopes were inaccessible and why (the backend error verbatim), rather
   than silently omitting them or showing an empty list with no explanation.

---

### User Story 2 - Create a RoleBinding or ClusterRoleBinding (Priority: P1)

A cluster-admin operator wants to grant an OIDC group (or a specific user, or a ServiceAccount)
access to a namespace or the whole cluster by binding it to an existing Role or ClusterRole,
without hand-writing YAML, while still reviewing exactly what will be created before it takes
effect.

**Why this priority**: This is the actual "administration" action the feature exists for — turning
role browsing into a granted permission. It is the smallest end-to-end slice that delivers real
value (an identity gains access) and it is independently testable and demoable on its own.

**Independent Test**: Can be fully tested by selecting an existing Role or ClusterRole, entering a
subject (an OIDC group name, username, or ServiceAccount reference), reviewing the generated
`RoleBinding`/`ClusterRoleBinding` diff, confirming, and then observing the new binding appear in
a subsequent list of bindings for that Role/ClusterRole — independent of the namespace-assignment
view (Story 4) and the OIDC issuer config-generator (Story 5).

**Acceptance Scenarios**:

1. **Given** the operator has selected an existing Role (namespace-scoped) or ClusterRole
   (cluster-scoped) and entered one or more subjects (OIDC group, username, or ServiceAccount) and,
   for a Role, a target namespace, **When** they choose to create the binding, **Then** the system
   shows a diff preview of the exact `RoleBinding`/`ClusterRoleBinding` object that will be created
   (kind, namespace/name, `roleRef`, `subjects`) before any cluster write occurs.
2. **Given** the diff preview is showing, **When** the operator explicitly confirms, **Then** the
   system creates the object on the cluster and the UI reflects the new binding only after the
   backend confirms the write succeeded — never optimistically before that.
3. **Given** the diff preview is showing, **When** the operator closes the preview or navigates
   away without confirming, **Then** no object is created on the cluster.
4. **Given** the operator's own identity lacks permission to create `RoleBinding`/
   `ClusterRoleBinding` objects (verified via a live permission check against the API server, not
   assumed from local state), **When** they open this surface, **Then** the create action is
   visibly disabled with an explanation, rather than only failing after they attempt it.
5. **Given** the operator attempts to create a binding and the API server rejects it (for example,
   because it would grant privileges the operator's own identity does not itself hold — the API
   server's built-in escalation check), **When** the rejection occurs, **Then** the system shows
   the backend's error verbatim and performs no partial write.

---

### User Story 3 - Edit or remove an existing RoleBinding/ClusterRoleBinding (Priority: P2)

A cluster-admin operator needs to change who a binding applies to (add/remove a subject) or revoke
a grant entirely by removing the binding, using the same review-before-apply discipline as
creating one.

**Why this priority**: Access grants need to be correctable and revocable, but this is naturally
sequenced after creation (Story 2) exists — editing/removing is the same object-mutation
machinery applied to an existing object instead of a new one.

**Independent Test**: Can be fully tested by opening an existing binding created in Story 2,
changing its subject list or deleting it, reviewing the diff (or deletion confirmation naming the
exact object), confirming, and observing the binding's new state (or absence) afterward.

**Acceptance Scenarios**:

1. **Given** an existing `RoleBinding`/`ClusterRoleBinding` is open for editing, **When** the
   operator changes its subjects, **Then** the system shows a diff between the live object and the
   edited version before any write, and requires explicit confirmation to apply it.
2. **Given** the operator chooses to remove a binding, **When** they confirm, **Then** the system
   names the exact object (kind, namespace/name) in the confirmation before deleting it, and the
   binding disappears from the list only after the backend confirms the deletion.
3. **Given** the binding was changed or deleted by someone/something else since the operator opened
   it (a stale/version conflict), **When** the operator attempts to apply their edit, **Then** the
   system rejects the apply with a clear conflict message and requires the operator to re-review
   the current state before trying again.

---

### User Story 4 - Look up an identity's effective namespace access (Priority: P2)

A cluster-admin operator (or an operator auditing access) wants to know, for a specific user or
group, which namespaces they can act in and with what verbs — without manually cross-referencing
every RoleBinding and ClusterRoleBinding by hand.

**Why this priority**: This is the audit/verification half of the feature — it confirms that the
bindings created in Stories 2-3 actually produce the intended access, and it is the read-only
"namespace-assignment view" MVP.md calls for explicitly as read-only-first. It depends on bindings
existing to be useful, so it is sequenced after binding creation.

**Independent Test**: Can be fully tested by entering or selecting a known subject (user, group, or
ServiceAccount) and viewing the resulting namespace/verb summary, entirely from data already
visible via Roles/ClusterRoles/RoleBindings/ClusterRoleBindings — no cluster write occurs.

**Acceptance Scenarios**:

1. **Given** the operator enters or selects a subject (user, group, or ServiceAccount identity),
   **When** they request its effective access, **Then** the system shows, for every namespace
   where that subject has any binding (directly or via a ClusterRoleBinding), the resolved set of
   verbs/resources it can act on, computed from the currently-visible Role/ClusterRole and
   RoleBinding/ClusterRoleBinding objects.
2. **Given** a subject has cluster-wide access via a `ClusterRoleBinding`, **When** its effective
   access is shown, **Then** the system clearly marks that access as cluster-wide rather than
   listing every namespace individually as if each had a separate grant.
3. **Given** a subject has no bindings at all, **When** the operator looks it up, **Then** the
   system clearly states it found no bindings for that subject, rather than showing an empty or
   ambiguous result.
4. **Given** this view is read-only, **When** the operator is looking at it, **Then** no action in
   this view can create, edit, or delete any cluster object.

---

### User Story 5 - Generate OIDC issuer configuration for the API server (Priority: P3)

An operator who wants to enable OIDC login against the cluster needs to know exactly what to
configure on the API server (issuer URL, client ID, audience, username claim, groups claim), and
since Fathom cannot change those API-server flags itself on the great majority of clusters it
connects to, it should generate the exact configuration/instructions for the operator (or whoever
controls the control plane) to apply outside Fathom.

**Why this priority**: This is valuable but is the smallest and most self-contained slice — it
does not depend on, or feed, the role-assignment/namespace-lookup stories, and most operators will
already have OIDC configured by the time they use Stories 1-4 (which only need RBAC objects, not a
live OIDC login). It is explicitly the lowest-priority story.

**Independent Test**: Can be fully tested by filling in the issuer URL, client ID, audience,
username-claim, and groups-claim fields and confirming the system produces a correct, copyable
kube-apiserver flag set and provider-specific guidance, with no cluster call made and no claim
that anything was changed on the cluster.

**Acceptance Scenarios**:

1. **Given** the operator opens the OIDC configuration surface, **When** the system inspects the
   connected cluster, **Then** it shows, where determinable, whether the cluster appears to be a
   type where Fathom could plausibly detect existing OIDC flags (e.g. a kubeadm-managed cluster
   exposing its `ClusterConfiguration`) versus a managed offering where API-server flags are
   controlled outside the Kubernetes API entirely — and in all cases states plainly that Fathom
   cannot itself apply API-server flag changes through the Kubernetes API.
2. **Given** the operator fills in issuer URL, client ID, audience, username-claim, groups-claim,
   and any extra scopes, **When** they request the generated configuration, **Then** the system
   produces the exact `kube-apiserver` flags for a kubeadm-style cluster and separate,
   provider-labeled instructions/links for the managed offerings Fathom supports connecting to,
   and offers to copy the output or export it to a file.
3. **Given** the generated output is showing, **When** the operator reviews it, **Then** it is
   visibly and unambiguously labeled as "apply this yourself outside Fathom" — it is never
   presented alongside, or styled like, the diff-preview/confirm pattern used for actual cluster
   mutations (Stories 2-3), since generating this text does not change the cluster.

---

### Edge Cases

- What happens when the operator's identity has permission to create bindings but not to list
  existing Roles/ClusterRoles (or vice versa)? Each capability (browse, create/edit, look up
  namespace access) is gated on its own permission check; a missing read permission for
  Roles/ClusterRoles must not block the create-binding UI from working if the operator already
  knows the exact role name, but the system should surface that the list is degraded/partial.
- How does the system handle a `RoleBinding`/`ClusterRoleBinding` whose `roleRef` points at a
  Role/ClusterRole that no longer exists? It must be shown clearly as broken/dangling in both the
  binding list and any namespace-access resolution that touches it, rather than silently
  omitted or causing a resolution error for unrelated bindings.
- What happens when a subject name entered for a new binding contains characters that are not
  valid in Kubernetes RBAC `subjects[].name`, or is empty? The system MUST reject it before
  attempting a cluster write, with a specific validation message, the same way the existing
  YAML-edit-and-apply flow rejects invalid edits.
- What happens when the operator requests namespace-access resolution (Story 4) for a subject name
  that matches many bindings across many namespaces (a very broad group like `system:authenticated`
  or an org-wide OIDC group)? The system must still render a complete, correct result; very large
  result sets may be grouped/summarized but never truncated silently without indicating more exist.
- What happens on a cluster where `rbac.authorization.k8s.io/v1` bindings exist that reference
  subjects of `kind: Group`/`kind: User` with no way for Fathom to know whether those names
  correspond to real OIDC claims (since Fathom has no IdP integration)? The system treats subject
  names as opaque strings throughout — it never validates them against an identity provider, and
  says so where relevant (e.g. in the namespace-lookup input) so operators aren't misled into
  thinking a typo would be caught.
- What happens if the operator tries to create a binding to a `ClusterRole` widely recognized as
  highly privileged (e.g. `cluster-admin`)? The system does not add its own extra restriction
  beyond the diff-preview + confirmation already required for every binding; the API server's own
  RBAC escalation check (rejecting grants of privileges the requesting identity doesn't itself
  hold) is the authority on whether the grant is allowed, and its rejection (if any) is surfaced
  verbatim per Story 2's acceptance scenario 5.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a cluster-admin-oriented RBAC administration surface that
  lists existing `Role` (namespace-scoped) and `ClusterRole` (cluster-scoped) objects visible to
  the operator's current identity, with their resolved rules (API groups, resources, resource
  names, verbs) shown in readable form.
- **FR-002**: The system MUST allow the operator to create a new `RoleBinding` (namespaced) or
  `ClusterRoleBinding` (cluster-scoped) that references an existing `Role`/`ClusterRole` and one or
  more subjects (`Group`, `User`, or `ServiceAccount`), entered as plain text/reference fields —
  the system does not validate `Group`/`User` subject names against any identity provider.
- **FR-003**: Every create or edit of a `RoleBinding`/`ClusterRoleBinding` MUST show a diff between
  the object's current state (if it exists) and the proposed state, and MUST require an explicit,
  separate confirmation step before writing to the cluster — no such write may occur as a side
  effect of editing or closing the diff view, matching the pattern established for YAML-edit-apply
  in `003-yaml-node-crd-parity` (its FR-002/FR-003).
- **FR-004**: The system MUST reject applying an edit to a `RoleBinding`/`ClusterRoleBinding` if
  the live object has changed since it was opened for editing (a stale-version conflict), and MUST
  require the operator to re-review the current state before retrying, matching the conflict
  handling established in `003-yaml-node-crd-parity`.
- **FR-005**: The system MUST allow the operator to delete an existing `RoleBinding`/
  `ClusterRoleBinding`, gated behind an explicit confirmation that names the exact object (kind,
  namespace/name) being removed.
- **FR-006**: The system MUST surface backend/API-server errors from any RBAC object read or write
  verbatim to the operator, including rejections caused by the API server's own RBAC escalation
  checks, rather than replacing them with a generic message.
- **FR-007**: The system MUST check the operator's own current permissions (via the Kubernetes API,
  e.g. an access/rules review) before offering the create/edit/delete actions for `RoleBinding`/
  `ClusterRoleBinding` objects, and MUST visibly disable or hide those actions when the operator's
  identity lacks the corresponding RBAC-edit permission, rather than only failing after an attempt.
  This check governs only this feature's own mutating actions; it does not gate any other feature
  or provide a general-purpose permission framework for the rest of the application.
- **FR-008**: The system MUST NOT perform, and MUST NOT claim to perform, any live mutation of
  `kube-apiserver` process configuration (OIDC issuer, audience, or claim-mapping flags) through
  the Kubernetes API — no such API exists for Fathom, as an external client, to call.
- **FR-009**: The system MUST provide a guided OIDC configuration generator: given operator-entered
  issuer URL, client ID, audience, username-claim, and groups-claim (and optional extra scopes),
  it MUST produce (a) the exact `kube-apiserver` flag set for a kubeadm-style cluster, and (b)
  provider-labeled instructions/links for connecting managed Kubernetes offerings' own OIDC
  configuration paths, and MUST let the operator copy or export this output.
- **FR-010**: The system MUST visually and textually distinguish the OIDC configuration generator's
  output (informational, "apply this yourself," no cluster write occurs) from the diff-preview/
  confirmation pattern used for actual RoleBinding/ClusterRoleBinding mutations, so operators cannot
  mistake generating instructions for having changed the cluster.
- **FR-011**: The system MUST provide a read-only view that, given a subject (user, group, or
  ServiceAccount), resolves and displays every namespace in which that subject has access via an
  existing `RoleBinding`, plus any cluster-wide access via a `ClusterRoleBinding`, together with the
  resolved verbs/resources for each, computed entirely from `Role`/`ClusterRole`/`RoleBinding`/
  `ClusterRoleBinding` objects already readable by the operator's identity — this view MUST NOT
  create, edit, or delete any cluster object.
- **FR-012**: The namespace-access lookup (FR-011) MUST clearly state when a subject has no
  bindings found, and MUST clearly distinguish cluster-wide access from per-namespace access rather
  than enumerating cluster-wide access as if it were a separate grant per namespace.
- **FR-013**: The system MUST clearly indicate, per binding, when a `RoleBinding`/
  `ClusterRoleBinding`'s `roleRef` points to a `Role`/`ClusterRole` that no longer exists (a
  dangling reference), in both the binding browser and namespace-access resolution.
- **FR-014**: The system MUST validate subject names and namespace/role references for a new or
  edited `RoleBinding`/`ClusterRoleBinding` before attempting a cluster write, rejecting invalid
  input with a specific, actionable message rather than sending an invalid object to the API
  server.
- **FR-015**: This feature's mutating actions (create/edit/delete `RoleBinding`/
  `ClusterRoleBinding`) MUST be classified and routed the same way every other cluster write in
  Fathom is classified for guard/audit purposes: namespaced-object writes as writes scoped to their
  namespace, cluster-scoped-object writes as cluster-scoped writes — consistent with how the rest
  of the application already distinguishes namespaced from cluster-scoped mutations.

### Key Entities

- **Role / ClusterRole**: Existing Kubernetes RBAC permission sets (`rbac.authorization.k8s.io/v1`).
  This feature only reads them; it never creates, edits, or deletes them.
- **RoleBinding / ClusterRoleBinding**: The objects this feature creates, edits, and deletes. Each
  references exactly one `Role`/`ClusterRole` (`roleRef`) and one or more `subjects`. A
  `RoleBinding` is namespace-scoped; a `ClusterRoleBinding` is cluster-scoped.
- **Subject**: A `Group`, `User`, or `ServiceAccount` reference inside a binding. Group/User names
  are opaque strings from this feature's point of view (they are expected to correspond to values
  an identity provider would assert as OIDC claims, but Fathom does not validate or look them up
  against any identity provider).
- **Effective Namespace Access (derived, not persisted)**: For a given subject, the set of
  namespaces (plus cluster-wide, if applicable) and the resolved verbs/resources available in
  each, computed on demand from existing Role/ClusterRole/RoleBinding/ClusterRoleBinding objects.
  Not a stored object — recomputed each time it is requested.
- **OIDC Issuer Configuration (generated output, not a cluster object)**: The operator-entered
  issuer URL, client ID, audience, username-claim, groups-claim, and scopes, and the
  `kube-apiserver` flags/provider instructions generated from them. This exists only as
  generated, exportable text; it is never sent to, or stored on, the cluster by this feature.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can go from "RBAC administration surface opened" to "a new
  RoleBinding/ClusterRoleBinding created and visible in the binding list" — including reviewing
  the diff — in under 2 minutes for a role and subject they already know, without writing any YAML
  by hand.
- **SC-002**: 100% of RoleBinding/ClusterRoleBinding create, edit, and delete actions in this
  feature show a diff or a named-object deletion confirmation and require explicit confirmation
  before any cluster write — zero such writes occur without one.
- **SC-003**: 100% of RoleBinding/ClusterRoleBinding edit attempts against an object that changed
  since it was opened are rejected before any cluster write, with the operator shown the conflict.
- **SC-004**: An operator can look up any subject with existing bindings and see its complete,
  correctly-resolved namespace/verb access summary, including correct handling of cluster-wide
  grants, with zero cluster mutations performed by that lookup.
- **SC-005**: An operator on a cluster where Fathom cannot live-edit OIDC settings (true for the
  large majority of managed and kubeadm clusters) can still obtain a correct, complete
  `kube-apiserver` OIDC flag set (or provider-specific instructions) to hand off, without needing
  to consult external OIDC documentation to know which flags to set.
- **SC-006**: Zero instances of the OIDC configuration generator's output being mistaken, in the
  UI, for a live cluster change — verified by the generator's output always appearing in a visibly
  distinct, clearly-labeled "apply this yourself" presentation, never inside the diff-preview/
  confirm flow used for actual mutations.
- **SC-007**: An operator whose identity lacks RBAC-edit permission sees the create/edit/delete
  actions disabled with an explanation in 100% of cases, rather than discovering the restriction
  only after attempting the action and receiving an API-server rejection.

## Assumptions

- The operator using this feature is already authenticated to the cluster via their existing
  kubeconfig context (however that context obtains its credentials — OIDC-issued token, cloud IAM
  plugin, client cert, etc.); this feature does not implement or modify any login/authentication
  flow itself.
- The Kubernetes API server does not expose any endpoint to read or mutate its own OIDC
  authentication flags (`--oidc-issuer-url` and related) at runtime; those are static process
  flags set at cluster bootstrap or through each managed provider's own control-plane
  configuration path, outside the Kubernetes API. This feature's OIDC surface is therefore
  read-only detection plus configuration/instruction generation, never a live edit, and this is a
  permanent scoping decision for this feature, not a placeholder pending a future Kubernetes API
  capability.
- `Group` and `User` subject names entered when creating a binding are free-text strings the
  operator supplies from their own knowledge of their identity provider's claim values; Fathom has
  no OIDC/IdP integration in this feature and performs no live validation of those names against
  any provider.
- Enforcement of "does this grant escalate privileges beyond what the operator's own identity
  holds" is the Kubernetes API server's own responsibility (its built-in RBAC escalation
  prevention on `RoleBinding`/`ClusterRoleBinding` writes); this feature does not reimplement that
  check client-side, only surfaces the API server's rejection verbatim if it occurs.
- This feature's own action-gating (FR-007) is scoped narrowly to its own create/edit/delete
  actions for `RoleBinding`/`ClusterRoleBinding` objects. It is not the general RBAC-scoped-UI
  framework described in MVP.md's Tier B (hiding/disabling arbitrary actions app-wide based on
  role) — that remains a separate, future item this feature does not implement, though the same
  permission-check mechanism could plausibly be extended to it later.
- Deleting a `Role` or `ClusterRole` itself (as opposed to a binding) is out of scope for this
  feature; that is already covered by Fathom's general YAML-edit-and-apply / object-delete
  capabilities for any resource type, and this feature does not duplicate it.
- LDAP integration and OAuth-based cluster login (both listed under MVP.md's Tier B) are separate,
  out-of-scope items; this feature covers only the RBAC-object administration and OIDC
  configuration-generation surfaces described above.
