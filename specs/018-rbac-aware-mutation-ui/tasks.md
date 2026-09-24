# Tasks: Proactive RBAC-Aware Mutation UI

Feature dir: `specs/018-rbac-aware-mutation-ui`. Dependency-ordered; complete in order.

## Phase 1: Protocol and backend (foundation)

- [x] T001 Add `group: Option<String>` (`#[serde(default)]`) to `RbacAccessCheck` in `src/protocol/v1/rbac_access.rs`; use it in `handle_refresh_rbac_self_access` (`src/backend/rbac_access.rs`) instead of the hardcoded RBAC group; fix all Rust construction sites and `tests/protocol_serde.rs`.
- [x] T002 Add `CheckResourceMutationAccess` command (`src/protocol/v1/cmd.rs`) and `ResourceMutationAccessChecked` event (`src/protocol/v1/event.rs`); add arms in `src/protocol/v1/mutation.rs` (ReadOnly), `src/app/mailbox.rs`, `tests/protocol_serde.rs` (round trip + exhaustive matches).
- [x] T003 Extract a reusable `resolve_api_resource(client, api_version, kind)` helper in `src/backend/apply.rs`; implement `handle_check_resource_mutation_access` in `src/backend/rbac_access.rs` and dispatch it in `src/backend/cmd/resources.rs`; unit-test the pure parts.
- [x] T004 Regenerate ts-rs bindings (`CARGO_BUILD_JOBS=2 cargo test -p fathom --test export_bindings`).

## Phase 2: Frontend shared plumbing

- [ ] T005 Fix `ui/src/store/handlers/rbacAccess.ts` to merge `selfAccess` by `(verb, group, resource, namespace)`; handle `ResourceMutationAccessChecked` (ingest/read/index routing where `RbacSelfAccessUpdated` is routed); Vitest coverage.
- [ ] T006 Add `ui/src/hooks/useMutationAccess.ts` (dispatch missing checks once, return `{status: 'unknown'|'allowed'|'denied', reason}`; unknown never blocks, FR-006) plus a `DisabledReason` tooltip wrapper (`ui/src/components/GatedButton.tsx`) using the existing `Tooltip`; Vitest coverage.
- [ ] T007 Refactor `RoleBindingForm.tsx` onto the shared hook (no behavior change; existing e2e specs must stay green).

## Phase 3: Wire surfaces

- [ ] T008 Node actions: gate cordon/uncordon/drain/taint in `NodeActionsPanel.tsx` (`update` on `nodes`, core group).
- [ ] T009 Helm: gate upgrade (`HelmUpgradePanel.tsx`), rollback (`resources/helm.tsx`), uninstall (`HelmReleaseActionsPanel.tsx`) via secrets update/delete in the release namespace.
- [ ] T010 YAML apply: gate the Apply button in `YamlEditTab.tsx` using `CheckResourceMutationAccess`.
- [ ] T011 RBAC admin delete: add optional `ActionDescriptor.accessCheck` (`resources/types.ts`), make `ActionMenu.tsx` consult it (disabled + tooltip), set it on `resources/rolebindings.tsx` and `resources/clusterrolebindings.tsx` delete.

## Phase 4: Tests and polish

- [ ] T012 Playwright e2e `ui/tests/e2e/us1-mutation-access-gating.spec.ts` (node cordon denied vs allowed; Chromium).
- [ ] T013 Full validation: `cargo test`, `cargo clippy -- -D warnings`, `pnpm typecheck`, `pnpm lint`, `pnpm test`, e2e; fix failures.
