# Tasks: Cluster-Admin RBAC / OIDC-OAuth Administration

**Input**: Design documents from `specs/011-rbac-oidc-administration/`
**Tests**: Explicitly requested by the user — included throughout.

## Phase 1: Setup

- [X] T001 Confirm `.specify/feature.json` points at `specs/011-rbac-oidc-administration` (done)
- [X] T002 [P] Read `src/protocol/v1/mutation.rs`, `src/backend/apply.rs`,
      `ui/src/store/editSession.ts`, `ui/src/components/panels/YamlDiffView.tsx`,
      `ui/src/components/ConfirmDialog.tsx`, `ui/tests/e2e/us1-apply-yaml.spec.ts` to confirm
      exact reuse points before writing any new code (done via survey)

## Phase 2: Foundational (protocol additions — blocks all UI work)

- [ ] T003 Add `section_keys::access_namespace_access()` and `section_keys::access_oidc_config()`
      to `src/protocol/v1/route.rs` (wire strings `"access/namespace-access"`,
      `"access/oidc-config"`), and register both in `access_control_sections()` in
      `src/sections/mod.rs` with `ScopeKind::Cluster`
- [ ] T004 [P] Add `RbacAccessCheck`, `RbacAccessResult`, `NamespaceAccessEntry`,
      `NamespaceAccessResolved`, `KubeadmDetectionUpdated` structs to `src/protocol/v1/event.rs`
      (or a new `src/protocol/v1/rbac_access.rs` module, matching how `rolebindings.rs`/
      `clusterrolebindings.rs` are split out under `src/protocol/v1/`), each deriving the same
      `Serialize, Deserialize, Clone, Debug, PartialEq, TS` set as sibling types, with `ts-rs`
      export attributes matching existing generated-type conventions
- [ ] T005 Add `RefreshRbacSelfAccess { checks: Vec<RbacAccessCheck> }`,
      `ResolveNamespaceAccess { subject_kind: String, subject_name: String, subject_namespace: Option<String> }`,
      `RefreshKubeadmDetection` variants to `BackendCmdV1` in `src/protocol/v1/cmd.rs`
- [ ] T006 Add `RbacSelfAccessUpdated { results: Vec<RbacAccessResult> }`,
      `NamespaceAccessResolved(NamespaceAccessResolved)`, `KubeadmDetectionUpdated(KubeadmDetectionUpdated)`
      variants to the appropriate `BackendEventV1`/`ResourceReadEvent`/`StreamEvent` enum in
      `src/protocol/v1/event.rs` (match existing routing convention for read-only section events)
- [ ] T007 Add match arms for all 3 new `BackendCmdV1` variants in
      `src/protocol/v1/mutation.rs`'s `mutation_target()` (→ `MutationTarget::ReadOnly`) and
      `section()` (→ the relevant `access_*` `SectionKey`, using the new keys from T003 for the
      two new commands and an existing `access_rolebindings`/`access_clusterrolebindings` key
      for `RefreshRbacSelfAccess` depending on which binding kind the check targets)
- [ ] T008 `cargo build` after T003-T007 to confirm the exhaustive-match compiler enforcement
      caught every arm (expect compile errors until all arms are added — this is intentional
      per the pattern's design)
- [ ] T009 Regenerate `ts-rs` bindings (`cargo test` triggers `ts-rs` export, or run the
      project's existing bindings-generation command/test — check `tests/` or `build.rs` for the
      exact trigger used by prior features) so `ui/src/protocol/generated/` picks up the 5 new
      types

**Checkpoint**: protocol compiles, new types generated for frontend — user story work can begin.

---

## Phase 3: User Story 1 - Browse Roles and ClusterRoles (Priority: P1) 🎯 MVP

**Goal**: Readable Roles/ClusterRoles/RoleBindings/ClusterRoleBindings list+detail views in the
React app (currently un-descriptored / generic fallback).

**Independent Test**: Open Access Control > Roles/ClusterRoles, browse list, open one, see
resolved rules table; a namespace/scope the operator can't list is named explicitly.

### Tests for User Story 1

- [ ] T010 [P] [US1] Vitest: dangling-`roleRef` detection helper (given a binding's `role_ref`
      string and the fetched Role/ClusterRole name sets, correctly flags missing refs) in
      `ui/src/resources/__tests__/rbacRoleRef.test.ts`
- [ ] T011 [P] [US1] Vitest: `PolicyRuleSummary` → readable rule-table row rendering
      (api groups/resources/resource names/verbs) in
      `ui/src/resources/__tests__/roles.test.tsx`
- [ ] T012 [US1] Playwright e2e `ui/tests/e2e/us1-rbac-browse-roles.spec.ts`: scripted fake
      backend emits `RolesUpdated`/`RoleBindingsUpdated`/`ClusterRolesUpdated`/
      `ClusterRoleBindingsUpdated` plus one degraded-namespace error; assert list rendering,
      rule-table detail view, dangling badge, and the degraded-scope message surfaces the
      backend error verbatim (`data-testid` prefix `rbac-`, e.g. `rbac-role-list`,
      `rbac-role-detail`, `rbac-degraded-scope`)

### Implementation for User Story 1

- [ ] T013 [P] [US1] Create `ui/src/resources/roles.tsx` descriptor: list columns
      (name, namespace, rule count), detail pane rendering `PolicyRuleSummary[]` as a readable
      table (api groups / resources / resource names / verbs), `data-testid="rbac-role-list"` /
      `"rbac-role-detail"`
- [ ] T014 [P] [US1] Create `ui/src/resources/clusterroles.tsx` descriptor, same shape, no
      namespace column, `data-testid="rbac-clusterrole-list"` / `"rbac-clusterrole-detail"`
- [ ] T015 [P] [US1] Create `ui/src/resources/rolebindings.tsx` list+detail descriptor: columns
      name/namespace/roleRef/subject count; detail shows subjects table and roleRef, with a
      dangling-roleRef badge (`data-testid="rbac-dangling-badge"`) computed client-side by
      joining against the fetched Roles/ClusterRoles lists (per data-model.md's resolved
      approach — no backend field needed)
- [ ] T016 [P] [US1] Create `ui/src/resources/clusterrolebindings.tsx` list+detail descriptor,
      same shape as T015 minus namespace, dangling check against ClusterRoles only
- [ ] T017 [US1] Register all 4 descriptors in `ui/src/resources/index.ts` (depends on
      T013-T016)
- [ ] T018 [US1] Verify `ui/src/resources/nav.ts`'s existing Access Control group entries
      (`access/roles`, `access/rolebindings`, `access/clusterroles`,
      `access/clusterrolebindings`) now render through the new descriptors instead of the
      generic fallback; adjust labels only if needed (file likely unchanged — confirm, don't
      blind-edit)
- [ ] T019 [US1] Add a degraded-scope banner component (reused by US1 and US4) at
      `ui/src/components/rbac/DegradedScopeBanner.tsx`, `data-testid="rbac-degraded-scope"`,
      rendering each inaccessible namespace/scope plus the verbatim backend error

**Checkpoint**: User Story 1 fully functional and independently testable/demoable.

---

## Phase 4: User Story 2 - Create a RoleBinding or ClusterRoleBinding (Priority: P1)

**Goal**: Structured create form → generated YAML → existing `ApplyResourceYaml` diff-preview →
confirm → cluster write, gated by a live self-permission check.

**Independent Test**: Select a Role/ClusterRole, enter subjects (+ namespace for Role), review
diff, confirm, see the new binding in the list; verify no write happens without confirmation;
verify create is disabled when self-access-review says not allowed.

### Tests for User Story 2

- [ ] T020 [P] [US2] Rust unit test in `src/backend/rbac_access.rs` (new module, see T024):
      `SelfSubjectAccessReview` result → `RbacAccessResult` mapping, using a fixture
      `kube::Client` test-double or the project's existing pattern for testing backend logic
      without a live cluster (check how `src/backend/apply.rs` or another backend module is
      unit-tested today — mirror that harness, not a new one)
- [ ] T021 [P] [US2] `tests/protocol_serde.rs`: add `sample_rbac_access_check()`,
      `sample_rbac_access_result()`, push into `all_cmds()`/`all_events()`, extend the
      `*_is_exhaustive` matches for `RefreshRbacSelfAccess` and `RbacSelfAccessUpdated`
- [ ] T022 [P] [US2] Vitest: RoleBinding/ClusterRoleBinding form → generated YAML string
      (roleRef/subjects/namespace mapped correctly, matches `RoleBindingDetails`/
      `ClusterRoleBindingDetails` shape) in `ui/src/resources/__tests__/rolebindingForm.test.ts`
- [ ] T023 [US2] Playwright e2e `ui/tests/e2e/us2-rbac-create-binding.spec.ts` mirroring
      `us1-apply-yaml.spec.ts`'s structure: fill form → `rbac-binding-review` opens
      `yaml-diff` (reused testid) → `yaml-apply` opens `ConfirmDialog` → `confirm-submit` →
      assert `ApplyResourceYaml` command fired with correct fields and the binding list updates
      only after the apply-succeeded event; separate test asserts cancel produces zero commands;
      separate test asserts the create action is disabled with an explanation
      (`data-testid="rbac-binding-create-disabled"`) when a scripted `RbacSelfAccessUpdated`
      event reports `allowed: false`

### Implementation for User Story 2

- [ ] T024 [US2] Create `src/backend/rbac_access.rs`: `handle_refresh_rbac_self_access` builds
      one `SelfSubjectAccessReview` per `RbacAccessCheck` via the existing `kube::Client` in
      backend state (mirror how `src/backend/roles.rs` obtains its `Api<...>`), submits via
      `Api::<SelfSubjectAccessReview>::create` (`authorization.k8s.io/v1`, already available via
      `k8s-openapi` `v1_34`), maps `status.allowed`/`status.reason` to `RbacAccessResult`,
      emits `RbacSelfAccessUpdated`
- [ ] T025 [US2] Wire `RefreshRbacSelfAccess` dispatch in `src/backend/cmd.rs` (same dispatch
      style as `ApplyResourceYaml`'s dedicated arm, or generic family dispatch — follow whichever
      convention the file already uses for read-only commands with custom handlers)
- [ ] T026 [P] [US2] Create `ui/src/components/rbac/SubjectEditor.tsx`: add/remove
      Group/User/ServiceAccount subject rows, `data-testid="rbac-subject-row"`,
      `"rbac-subject-add"`, `"rbac-subject-remove"`; client-side validation rejecting empty
      subject names before any YAML is generated (FR-014), `data-testid="rbac-subject-error"`
- [ ] T027 [US2] Create `ui/src/components/rbac/RoleBindingForm.tsx`: role/clusterrole picker
      (reusing T013/T014 list data), namespace field (RoleBinding only), `SubjectEditor` (T026),
      "Review" button building the target YAML object and opening the existing `YamlDiffView`
      session flow from `ui/src/store/editSession.ts` (create mode: live buffer = empty/absent,
      target = generated YAML, `expected_resource_version` unset/empty for create — confirm
      `apply.rs`'s create-vs-replace handling for an absent object and match it exactly, do not
      invent new semantics), `data-testid="rbac-binding-form"`, `"rbac-binding-review"`
- [ ] T028 [US2] Wire the self-access-review check (T024-T025) into T027: on mount, dispatch
      `RefreshRbacSelfAccess` for the relevant verb/resource/namespace; disable the
      "Create" entry point with an inline explanation
      (`data-testid="rbac-binding-create-disabled"`) until `allowed: true` is received (FR-007)
- [ ] T029 [US2] Add "New RoleBinding"/"New ClusterRoleBinding" entry points to the T015/T016
      list descriptors, opening T027's form

**Checkpoint**: User Stories 1 AND 2 both independently functional.

---

## Phase 5: User Story 3 - Edit or remove an existing RoleBinding/ClusterRoleBinding (Priority: P2)

**Goal**: Same diff-preview/confirm machinery applied to an existing binding (edit) and the
existing `Delete*` commands wired to a named-object `ConfirmDialog` (delete).

**Independent Test**: Open an existing binding, change subjects, review diff, confirm, see
updated state; delete a binding via confirm dialog naming the exact object; edit against a
binding that changed underneath (stale resourceVersion) is rejected with a conflict message.

### Tests for User Story 3

- [ ] T030 [P] [US3] Playwright e2e `ui/tests/e2e/us3-rbac-edit-delete-binding.spec.ts`: (a)
      open existing binding, edit subjects, diff shows only the subject delta, confirm, assert
      `ApplyResourceYaml` fired with the live object's `resourceVersion` as
      `expected_resource_version`; (b) scripted backend returns a `conflict:` error on apply,
      assert `yaml-diff-stale`/`yaml-apply-error` shown and a `yaml-reload` affordance, no
      silent overwrite; (c) delete flow: `ConfirmDialog` names kind+namespace+name exactly
      (`confirm-item` text), confirm sends `DeleteRoleBinding`/`DeleteClusterRoleBinding`, row
      removed from list only after the corresponding `RoleBindingsUpdated`/
      `ClusterRoleBindingsUpdated` refresh event lands — never optimistically

### Implementation for User Story 3

- [ ] T031 [US3] Extend `RoleBindingForm.tsx` (T027) to support edit mode: pre-populate from an
      existing `RoleBindingDetails`/`ClusterRoleBindingDetails`, set
      `expected_resource_version` from the object's live resourceVersion (need to confirm/add
      a `resource_version` field — check whether `RoleBindingDetails`/
      `ClusterRoleBindingDetails` currently expose it; if not, add
      `resource_version: String` to both structs in `src/protocol/v1/rolebindings.rs` /
      `clusterrolebindings.rs`, populate from the live object's `metadata.resourceVersion` in
      `src/backend/rolebindings.rs` / `clusterrolebindings.rs`, add `tests/protocol_serde.rs`
      coverage for the field, regenerate bindings)
- [ ] T032 [US3] Wire edit-mode diff/apply/conflict UX to reuse `ui/src/store/editSession.ts`'s
      existing `status`/`liveYamlChangedSinceReview` machinery unchanged (no new state machine)
- [ ] T033 [US3] Add delete entry points to T015/T016 list/detail views using the existing
      `ConfirmDialog` pattern (mirror another existing delete flow's wiring exactly, e.g. how
      `DeleteRole`/`DeleteClusterRole` are already wired if a frontend caller exists, or the
      generic delete pattern used elsewhere in `ui/src/resources/`), naming kind+namespace/name
      in `confirm-item`, sending `DeleteRoleBinding`/`DeleteClusterRoleBinding` on
      `confirm-submit`
- [ ] T034 [US3] Gate edit/delete entry points behind the same self-access-review check as T028
      (verb `update`/`patch` for edit, `delete` for delete)

**Checkpoint**: User Stories 1-3 all independently functional.

---

## Phase 6: User Story 4 - Look up an identity's effective namespace access (Priority: P2)

**Goal**: Read-only subject → namespace/verb resolution view.

**Independent Test**: Enter a subject with existing bindings, see per-namespace and cluster-wide
access correctly separated; a subject with no bindings shows an explicit empty state; dangling
`roleRef`s are flagged, not silently skipped.

### Tests for User Story 4

- [ ] T035 [P] [US4] Rust unit tests in `src/backend/rbac_access.rs` for the resolution
      algorithm using fixture Role/ClusterRole/RoleBinding/ClusterRoleBinding data: (a) subject
      with only namespaced RoleBinding access → correct `per_namespace` entries; (b) subject
      with a ClusterRoleBinding → `cluster_wide`, not enumerated per namespace (FR-012); (c) no
      matching bindings → `found_any: false`; (d) a binding whose `roleRef` doesn't resolve →
      listed in `dangling_bindings`, does not break resolution of unrelated bindings (edge case
      from spec.md); (e) a namespace the operator can't list → added to `degraded_scopes`, not
      silently dropped
- [ ] T036 [P] [US4] `tests/protocol_serde.rs`: add samples + exhaustive-match coverage for
      `ResolveNamespaceAccess`/`NamespaceAccessResolved`/`NamespaceAccessEntry`
- [ ] T037 [US4] Playwright e2e `ui/tests/e2e/us4-rbac-namespace-access.spec.ts`: enter subject,
      assert `rbac-namespace-access-lookup` shows resolved verbs per namespace; assert
      cluster-wide grant rendered distinctly (`rbac-namespace-access-clusterwide`, not one row
      per namespace); assert empty-result state (`rbac-namespace-access-empty`); assert
      dangling-binding note appears alongside a resolved result without breaking it; assert view
      never fires a write command (no `ApplyResourceYaml`/`Delete*`/etc. observed)

### Implementation for User Story 4

- [ ] T038 [US4] Implement the resolution algorithm in `src/backend/rbac_access.rs`
      (`resolve_namespace_access`): list Roles/ClusterRoles/RoleBindings/ClusterRoleBindings
      (reusing the existing list functions from `roles.rs`/`clusterroles.rs`/`rolebindings.rs`/
      `clusterrolebindings.rs` rather than duplicating API calls), filter bindings whose
      `subjects` match the requested `(subject_kind, subject_name, subject_namespace)`, join
      each surviving binding's `roleRef` against the Role/ClusterRole rule sets (dedup/sort
      verbs+resources), separate `RoleBinding`-sourced entries into `per_namespace` and
      `ClusterRoleBinding`-sourced entries into `cluster_wide`, collect dangling roleRefs and
      degraded (inaccessible) scopes per T035's cases
- [ ] T039 [US4] Wire `ResolveNamespaceAccess` dispatch in `src/backend/cmd.rs`, emitting
      `NamespaceAccessResolved`
- [ ] T040 [US4] Create `ui/src/resources/namespaceAccess.tsx` view (registered at
      `access/namespace-access` section, added as a new tuple in the existing Access Control
      `g(...)` call in `ui/src/resources/nav.ts`): subject-kind/name/namespace input
      (`data-testid="rbac-namespace-access-lookup"`), validation rejecting empty subject name
      client-side before dispatch, results table separating cluster-wide
      (`rbac-namespace-access-clusterwide`) from per-namespace rows, empty state
      (`rbac-namespace-access-empty`), dangling-binding notes reusing T019's/a similar badge,
      explicit note that Group/User names are opaque strings not validated against any IdP (per
      spec.md's edge cases)

**Checkpoint**: User Stories 1-4 all independently functional.

---

## Phase 7: User Story 5 - Generate OIDC issuer configuration (Priority: P3)

**Goal**: Frontend-only config generator + best-effort kubeadm detection, visually distinct from
the mutation flow, no cluster write.

**Independent Test**: Fill issuer/client-id/audience/claims, generate flags + provider
instructions, copy/export; confirm no cluster call occurs; confirm visual distinction from the
diff-preview pattern.

### Tests for User Story 5

- [ ] T041 [P] [US5] Rust unit test for the kubeadm-detection read in
      `src/backend/rbac_access.rs`: ConfigMap present → `{detected: true, checked: true}`;
      not found/forbidden/any error → `{detected: false, checked: false}`, never an error
      notification
- [ ] T042 [P] [US5] `tests/protocol_serde.rs`: sample + exhaustive-match coverage for
      `RefreshKubeadmDetection`/`KubeadmDetectionUpdated`
- [ ] T043 [P] [US5] Vitest: OIDC flag-generation pure function — given issuer/client-id/
      audience/username-claim/groups-claim/extra-scopes, produces the exact kube-apiserver flag
      set (`--oidc-issuer-url`, `--oidc-client-id`, `--oidc-username-claim`,
      `--oidc-groups-claim`, `--oidc-required-claim` for audience if applicable — confirm exact
      kube-apiserver flag names against current upstream docs before hardcoding) and
      provider-labeled instruction blocks, in `ui/src/resources/__tests__/oidcConfig.test.ts`
- [ ] T044 [US5] Playwright e2e `ui/tests/e2e/us5-rbac-oidc-generator.spec.ts`: fill form,
      generate, assert output panel has `data-testid="rbac-oidc-generator-output"` styled/marked
      distinctly from `yaml-diff` (different testid namespace entirely, assert no `yaml-diff`
      element exists on this page), assert copy and export actions work, assert zero backend
      commands fire other than the optional `RefreshKubeadmDetection` on mount, assert the
      "apply this yourself" disclaimer text is present

### Implementation for User Story 5

- [ ] T045 [US5] Implement `handle_refresh_kubeadm_detection` in `src/backend/rbac_access.rs`:
      read `kube-system/kubeadm-config` ConfigMap via existing `Api<ConfigMap>` pattern (see
      `src/backend/*.rs` for the established `Api<T>::get` + error-mapping convention), any
      error → `{detected: false, checked: false}`, never surfaced as a generic error
      notification (per research.md's decision)
- [ ] T046 [US5] Wire `RefreshKubeadmDetection` dispatch in `src/backend/cmd.rs`
- [ ] T047 [P] [US5] Create `ui/src/resources/oidcConfig.tsx` pure generator functions
      (`buildKubeadmFlags`, `buildProviderInstructions` for at least EKS/GKE/AKS given the
      managed-offerings Fathom already documents supporting per `docs/MVP.md`)
- [ ] T048 [US5] Create the OIDC Configuration view component wiring T047's functions to a form
      + visibly-distinct-from-diff-preview output panel (different container styling/class from
      `YamlDiffView`, explicit "apply this yourself outside Fathom" banner per FR-010),
      `data-testid="rbac-oidc-generator"`, copy button, `Blob`+anchor `download` export action,
      registered at the new `access/oidc-config` section (added as a tuple in the existing
      Access Control `g(...)` call in `ui/src/resources/nav.ts`)
- [ ] T049 [US5] On mount, dispatch `RefreshKubeadmDetection` and render its result as an
      informational badge (detected / not determinable), never blocking generation

**Checkpoint**: All 5 user stories independently functional.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T050 [P] `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
      clean
- [ ] T051 [P] `pnpm -C ui typecheck && pnpm -C ui lint` clean
- [ ] T052 [P] `pnpm -C ui test` (Vitest) clean
- [ ] T053 Run full Playwright suite for the 5 new `us{1..5}-rbac-*.spec.ts` files, clean
- [ ] T054 Run `specs/011-rbac-oidc-administration/quickstart.md` manually against a real
      cluster/context once, confirming each story's independent-test criterion
- [ ] T055 Update `docs/MVP.md`'s RBAC/OIDC row (line ~33, "Access Control (RBAC/OAuth/OIDC,
      role-scoped UI)") and the Tier C section (~line 591-598) to reflect what shipped
      (RoleBinding/ClusterRoleBinding CRUD + namespace-access lookup + OIDC generator; note that
      OAuth cluster-login and LDAP remain out of scope, and Tier B's app-wide RBAC-scoped-UI
      framework remains separate/future per spec.md's Assumptions)

---

## Dependencies & Execution Order

- Phase 1 (Setup) → Phase 2 (Foundational, protocol) blocks all of Phase 3-7.
- Phase 3 (US1) has no dependency on US2-5 and is the MVP.
- Phase 4 (US2) depends on Phase 2 only; reuses US1's list descriptors (T013-T016) for the
  role/clusterrole picker, so in practice sequence after US1 even though not strictly blocked.
- Phase 5 (US3) depends on Phase 4's `RoleBindingForm.tsx` (T027) for edit mode; delete is
  independent of US2 and could ship alone.
- Phase 6 (US4) depends on Phase 2 only (reads existing list data directly); independently
  shippable in parallel with US2/US3.
- Phase 7 (US5) depends on nothing but Phase 2 (kubeadm detection) — fully independent of
  US1-US4, most parallelizable story.
- Phase 8 depends on all desired stories being complete.

## Implementation Strategy

MVP = Phase 1 + 2 + 3 (US1 browse). Recommended incremental order given real dependencies:
US1 → US2 → US3 → US4 and US5 in parallel (both only need Phase 2).
