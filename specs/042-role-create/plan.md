# Implementation Plan: Role and ClusterRole create/edit

**Branch**: `042-role-create` | **Date**: 2026-09-26 | **Spec**: [spec.md](spec.md)

## Summary

A form-driven editor for Roles and ClusterRoles, reached from the Access Control panes (create) and the inspector (edit). The form is pure client-side logic that turns rule rows into a manifest. The write reuses the existing YAML edit-session machinery (`CreateResourceYaml` for create, `ApplyResourceYaml` with `expected_resource_version` for edit), which already carries confirm, audit (026), conflict handling and 040 manifest export. The only backend addition is a read-only discovery command feeding the pickers.

## Technical Context

- **Backend**: Rust, `kube`/`k8s-openapi`. New `RefreshApiResources` → `ResourceReadEvent::ApiResourcesUpdated(ApiResourceCatalog)`, in `src/backend/rbac_discovery.rs`. Discovery GETs only; classified `ReadOnly` in `mutation.rs`, section `access/roles`.
- **UI**: React/TS. `ui/src/components/rbac/`:
  - `roleYaml.ts`: pure logic (manifest build, presets, validation, edit patching, escalation parsing, picker options).
  - `TokenInput.tsx`: chip input with a datalist.
  - `RuleEditor.tsx`: preset select and rule rows.
  - `RoleForm.tsx`: form, review (manifest/diff + `ManifestActions`), apply, warnings, bind shortcut.
  - `RoleCreateLauncher.tsx` / `RoleEditLauncher.tsx`: gated buttons opening a Sheet. The create launcher swaps to `RoleBindingForm` for the bind shortcut.
  - `RoleBindingForm.tsx`: new optional `initial` for create mode.
  - `PaneHost.tsx`: toolbar buttons for `access/roles` and `access/clusterroles`, and the Edit button in the inspector header.
- **Store**: `rbacAccess.apiResources`, set by the `ApiResourcesUpdated` handler.
- **Devbridge guard**: lets `fathom-e2e-*` ClusterRole create/apply/delete through (never in strict mode) for the live suite.
- **Testing**: Vitest (`tests/unit/roleYaml.test.ts`), Playwright mock (`tests/e2e/rbac-role-create.spec.ts`), Playwright live on kind (`tests/live/rbac-role-create.spec.ts`), Rust serde and guard tests.

## Constitution Check

- Never blackbox the cluster: the exact manifest is shown, copyable and downloadable before apply. ✔
- RBAC-aware UI (018): gated buttons and a disabled Apply with a reason. Unknown stays enabled. ✔
- Mutations audited (026): writes go through existing audited commands. ✔
- Live tests are fenced: fathom-dev namespace, `fathom-e2e-*` cluster objects only, cleanup in `afterEach`. ✔

## Design decisions

- **Edit patches only `rules`**: keeps metadata, `aggregationRule` and `resourceVersion` intact, so there is no silent loss of fields the form does not model.
- **Refresh after apply**: RBAC lists are fetched, not watched, so the form sends `RefreshRoles`/`RefreshClusterRoles` after a successful write, and `SelectRole`/`SelectClusterRole` after an edit.
- **Discovery tolerant of broken groups**: `kube::Discovery::run` fails as a whole when one aggregated API is down, so the walk is done per group and failures are collected.
