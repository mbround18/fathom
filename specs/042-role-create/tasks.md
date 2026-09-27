# Tasks: Role and ClusterRole create/edit

## Phase 1: Backend discovery

- [x] T001 Add `ApiResourceName`, `ApiGroupResources`, `ApiResourceCatalog` in `src/protocol/v1/roles.rs`
- [x] T002 Add `RefreshApiResources` (cmd.rs) and `ApiResourcesUpdated` (event.rs); classify it ReadOnly, section `access/roles` (mutation.rs)
- [x] T003 Implement the group-tolerant discovery walk in `src/backend/rbac_discovery.rs` and dispatch it from `cmd/resources.rs`
- [x] T004 Serde round-trip tests (`tests/protocol_serde.rs`, cmd.rs, event.rs); regenerate `ui/src/protocol/generated`
- [x] T005 Devbridge guard: allow `fathom-e2e-*` ClusterRole writes outside strict mode, with tests

## Phase 2: Form logic (US1–US3)

- [x] T010 `roleYaml.ts`: manifest build/YAML, presets, token parsing, validation, edit patching, warnings, escalation parsing, picker options
- [x] T011 Unit tests `ui/tests/unit/roleYaml.test.ts`
- [x] T012 `TokenInput.tsx` and `RuleEditor.tsx`
- [x] T013 `RoleForm.tsx`: create/edit, review (manifest + diff, `ManifestActions`), confirm-gated Apply, access-denied reason, escalation block, `system:`/aggregation warnings
- [x] T014 Store: `rbacAccess.apiResources` fed by `ApiResourcesUpdated`

## Phase 3: Entry points and bind shortcut (US1, US2, US5)

- [x] T020 `RoleCreateLauncher.tsx` and `RoleEditLauncher.tsx`; wire into `PaneHost.tsx` (pane toolbars, inspector header)
- [x] T021 `RoleBindingForm` `initial` prop; bind shortcut after create (RoleBinding / ClusterRoleBinding / RoleBinding for a ClusterRole)
- [x] T022 Refresh the Roles/ClusterRoles list after a write, and reselect the object after an edit

## Phase 4: E2E

- [x] T030 Mock spec `ui/tests/e2e/rbac-role-create.spec.ts`: create + bind shortcut, denied apply, escalation error, aggregated `system:` edit
- [x] T031 Live spec `ui/tests/live/rbac-role-create.spec.ts` + `e2eClusterRole` helper: create, edit, delete a Role in `fathom-dev` and a `fathom-e2e-*` ClusterRole, with cleanup

## Follow-ups (not in 042)

- [ ] Pre-flight escalation hints (SSAR per rule) before apply
- [ ] Aggregation-label editor for aggregated ClusterRoles
