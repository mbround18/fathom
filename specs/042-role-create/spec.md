# Feature Specification: Role and ClusterRole create/edit

**Feature Branch**: `042-role-create`

**Created**: 2026-09-26

**Status**: Implemented

**Input**: "Role/ClusterRole create and edit under Access Control. New Role / New ClusterRole in the Roles and ClusterRoles panes. A form with name (+ namespace for Role) and a rules editor: one row per rule with apiGroups, resources, verb checkboxes for the standard verbs, optional resourceNames, add/remove rows. API group and resource pickers fed from discovery, with free text for CRDs. Presets read-only (get/list/watch), edit, custom. Edit is pre-filled from the live object and warns on `system:` roles and aggregated ClusterRoles. Review shows the exact YAML with Copy/Download (040 ManifestActions); Apply is confirm-gated, RBAC-aware (018 + SSAR), audited (026), and privilege-escalation errors are surfaced clearly. After create, 'Bind this role…' opens the RoleBinding form pre-filled."

## Clarifications

### Session 2026-09-26 (self-resolved with defaults; recorded for review)

- Q: Where does the resource picker's list come from? → A: A new read-only `RefreshApiResources` command that walks `/api` and `/apis` once, at each group's preferred version, subresources included. One broken aggregated group (typically `metrics.k8s.io`) is reported in `failed_groups` and does not fail the catalog. Free text is always accepted, so a CRD installed after the catalog loaded still works.
- Q: Which verbs get checkboxes? → A: The eight standard verbs, `get list watch create update patch delete deletecollection`. Anything else (`*`, `impersonate`, `bind`, `escalate`, `use`, ...) goes in a free-text "extra verbs" field on the same row.
- Q: What is the "edit" preset? → A: All eight standard verbs. "Read-only" is `get list watch`. "Custom" leaves rows alone. Picking a preset rewrites the verbs of every row; editing a checkbox afterwards makes the preset read "custom".
- Q: How is the core API group entered? → A: As the chip `core`, or `""`/`''`. It is stored as `""` in the manifest and displayed as `core (“”)`.
- Q: Does edit rebuild the object from the form? → A: No. Edit patches only `rules` onto the live object's JSON, so labels, annotations, `aggregationRule` and `resourceVersion` carry through untouched, and the write is an optimistic-concurrency replace (`ApplyResourceYaml` with `expected_resource_version`). A conflict offers Reload.
- Q: What about aggregated ClusterRoles? → A: Editing is allowed but warned: the aggregation controller overwrites `rules` from the selected ClusterRoles, so manual rule edits are lost. `system:` names are warned as well: they are managed by the control plane or add-ons and may be reconciled back.
- Q: What if the operator lacks create/update on roles? → A: Apply is disabled with the access-check reason (FR-006: an unknown result stays enabled). The form and the YAML review stay usable, so the operator can still copy the manifest and hand it to someone who can apply it.
- Q: How is privilege escalation reported? → A: The API server refuses a Role granting permissions the caller does not hold unless the caller has `escalate`. Fathom recognises that message and shows an explanation, the exact missing permissions the server listed, and the ways out (have an admin apply it, get `escalate` on roles, or narrow the rules).
- Q: What does the bind shortcut open? → A: After a Role is created: the RoleBinding form in the same namespace, `roleRef` Role/<name>. After a ClusterRole: a ClusterRoleBinding form, with a second option to bind it in one namespace through a RoleBinding. The binding name defaults to `<role>-binding`. Nothing is created until the operator applies that form.
- Q: Live writes to cluster-scoped objects? → A: The devbridge guard allows a ClusterRole write only when the name starts with `fathom-e2e-`, and never in strict mode, so the live suite can exercise ClusterRole create/edit/delete without being able to touch real cluster roles.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create a Role from a form (Priority: P1)

An operator wants a namespaced Role that lets a team read ConfigMaps. In Access Control → Roles they press "New Role", name it, pick the namespace, add `configmaps` from the resource picker, keep the read-only preset, review the YAML and apply.

**Independent Test**: Create `pod-reader` in `dev` with rule `core / pods / get,list,watch`, confirm the review shows exactly that manifest, apply, and confirm `CreateResourceYaml` carries it and the Roles list shows the new row.

**Acceptance Scenarios**:

1. **Given** the Roles pane, **When** the operator presses "New Role", **Then** a sheet opens with name, namespace (defaulting to the active namespace), a preset select and one rule row.
2. **Given** a rule row, **When** the operator focuses resources, **Then** the picker lists the discovered namespaced resources of the selected groups, and a typed name not in the list is still accepted.
3. **Given** an invalid form (no name, a rule without verbs or resources), **When** the operator looks at Review, **Then** Review is disabled and each problem is shown next to its row.
4. **Given** a valid form, **When** the operator presses Review, **Then** the exact YAML is shown with Copy and Download, and Apply asks for confirmation.
5. **Given** a confirmed apply, **When** the server accepts it, **Then** the sheet reports success, the Roles list refreshes, and the write is in the audit log.

### User Story 2 - Create a ClusterRole (Priority: P1)

Same flow from the ClusterRoles pane, without a namespace. The picker also lists cluster-scoped resources (`nodes`, `namespaces`, ...), and a rule may use non-resource URLs (`/healthz`, `/metrics`).

**Acceptance Scenarios**:

1. **Given** the ClusterRoles pane, **When** the operator presses "New ClusterRole", **Then** no namespace field is shown and cluster-scoped resources are offered.
2. **Given** a rule with only non-resource URLs, **When** reviewed, **Then** the manifest has `nonResourceURLs` and `verbs` and no `apiGroups`/`resources`.

### User Story 3 - Edit an existing Role or ClusterRole (Priority: P1)

From a Role or ClusterRole open in the inspector, "Edit" opens the same form pre-filled from the live object. The name (and namespace) are fixed. Review defaults to the diff against the live object.

**Acceptance Scenarios**:

1. **Given** a Role in the inspector, **When** the operator presses Edit, **Then** every rule (groups, resources, verbs, resourceNames, nonResourceURLs) is loaded into rows.
2. **Given** a name starting with `system:`, **Then** a warning explains the object is platform-managed and may be reconciled back.
3. **Given** a ClusterRole with `aggregationRule`, **Then** a warning explains that rule edits are overwritten by aggregation.
4. **Given** a confirmed edit, **Then** the write is `ApplyResourceYaml` with the object's `resourceVersion`. A conflict shows the server's message and Reload.

### User Story 4 - Access and escalation are explained (Priority: P1)

**Acceptance Scenarios**:

1. **Given** the SSAR check says the identity may not create (or update) roles in the target scope, **Then** Apply is disabled with the reason, and the form and YAML review still work.
2. **Given** the API server refuses with "attempting to grant RBAC permissions not currently held", **Then** the sheet names it a privilege-escalation refusal, lists the missing permissions from the server message, and explains the options.

### User Story 5 - Bind the new role (Priority: P2)

**Acceptance Scenarios**:

1. **Given** a Role was just created, **When** the operator presses "Bind this role…", **Then** the RoleBinding form opens with name `<role>-binding`, the Role's namespace and `roleRef` Role/<role>.
2. **Given** a ClusterRole was just created, **Then** both "ClusterRoleBinding" and "RoleBinding in a namespace" are offered, pre-filled with `roleRef` ClusterRole/<role>.

### Edge Cases

- Discovery fails for some groups: the catalog still loads and the editor says which groups are missing from the picker.
- Discovery has not loaded yet or failed entirely: free text still works.
- `*` in groups or resources: accepted, and the picker then offers every group's resources.
- The object was deleted while editing: the apply error is shown. The sheet can be closed.
- A Role picker never offers cluster-scoped resources. Typing one is still allowed, because Kubernetes accepts it (it has no effect in a Role).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Roles and ClusterRoles panes MUST offer "New Role" / "New ClusterRole", gated on `create` for `roles` / `clusterroles` in `rbac.authorization.k8s.io`.
- **FR-002**: The form MUST collect name, namespace (Role only) and one or more rules. Each rule has apiGroups, resources, verbs (checkboxes for the 8 standard verbs, plus free-text extras), optional resourceNames and, for ClusterRoles, nonResourceURLs. Rows can be added and removed.
- **FR-003**: API group and resource pickers MUST be fed from `RefreshApiResources` discovery and MUST accept free text.
- **FR-004**: Presets read-only (`get list watch`), edit (all 8 standard verbs) and custom MUST be offered.
- **FR-005**: Names MUST be valid path segments (non-empty, no `/`, no `%`, not `.`/`..`, no surrounding whitespace). Each rule MUST have a verb, and either resources with apiGroups, or (ClusterRole only) nonResourceURLs.
- **FR-006**: Before apply, the exact manifest MUST be shown with Copy and Download (040 `ManifestActions`). Edit also offers a diff against the live object.
- **FR-007**: Apply MUST be confirm-gated and disabled with a reason when the access check denies `create` (create) or `update` (edit). An unknown access result MUST NOT disable it.
- **FR-008**: Create MUST go through `CreateResourceYaml`, and edit through `ApplyResourceYaml` with `expected_resource_version`, so both are recorded by the 026 audit log.
- **FR-009**: Edit MUST patch only `rules` onto the live object, and MUST warn on `system:` names and on ClusterRoles with `aggregationRule`.
- **FR-010**: A privilege-escalation refusal MUST be explained, listing the permissions the server reported missing.
- **FR-011**: After a successful write the relevant list MUST refresh (RBAC lists are fetched, not watched), and after an edit the inspector MUST reload the object.
- **FR-012**: After a create, "Bind this role…" MUST open the binding form pre-filled with the new role as `roleRef`.
- **FR-013**: `RefreshApiResources` MUST be read-only and MUST tolerate individual group failures.

### Key Entities

- **RuleInput**: one editable rule: `apiGroups`, `resources`, `verbs`, `resourceNames`, `nonResourceURLs`.
- **ApiResourceCatalog**: discovered groups and their resources (name, namespaced, verbs), plus `failed_groups`.

## Success Criteria *(mandatory)*

- **SC-001**: An operator can create a working read-only Role without typing YAML.
- **SC-002**: The YAML shown in review is byte-for-byte what is sent to the API server.
- **SC-003**: A denied or escalating write never fails silently: the reason is on screen.
- **SC-004**: The live suite creates, edits and deletes a Role in `fathom-dev` and a `fathom-e2e-*` ClusterRole on kind, and leaves nothing behind.
