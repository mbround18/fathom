# Tasks: Manifest Export (never blackbox the cluster)

**Input**: Design documents from `specs/040-manifest-export/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Requested (constitution III, plus the task brief). Rust unit tests, Vitest, and Playwright mock-mode e2e on chromium and webkit.

## Phase 1: Setup

- [x] T001 Add the `yaml` (eemeli/yaml v2) dependency to `ui/package.json` / `ui/pnpm-lock.yaml`
- [x] T002 Create the feature folder `ui/src/features/manifest-export/` and the Rust modules `src/protocol/v1/manifest_export.rs`, `src/backend/manifest_export.rs`, `src/backend/kubeconfig_export.rs` (registered in their `mod` lists)

## Phase 2: Foundational (protocol)

- [x] T003 Define `ManifestObjectRef`, `FetchedManifest`, `ManifestFetchError`, `KubeconfigCredentialKind`, `KubeconfigExportPreview` in `src/protocol/v1/manifest_export.rs` with `TS` export
- [x] T004 Add `BackendCmdV1::{FetchManifests, PreviewKubeconfigExport, ExportKubeconfig}` in `src/protocol/v1/cmd.rs` with round-trip tests
- [x] T005 Add `SystemEvent::{ManifestsFetched, KubeconfigExportPreviewed, KubeconfigExportFinished}` in `src/protocol/v1/event.rs` with round-trip tests
- [x] T006 Classify the new commands in `src/protocol/v1/mutation.rs`: `FetchManifests`/`PreviewKubeconfigExport` → ReadOnly, `ExportKubeconfig` → Local; section = global; hosted policy: FetchManifests Allowed, both kubeconfig commands `Denied(Contexts)`; update `server/src/filter.rs` test expectations
- [x] T007 Regenerate bindings (`cargo test -p fathom --test export_bindings`) and add the `manifestExport` reply slice (`ui/src/features/manifest-export/state.ts`), wired into `ui/src/store/index.ts` and `ui/src/store/handlers/system.ts`

## Phase 3: User Story 1 - per-resource export, clean for re-apply (P1) 🎯 MVP

**Goal**: Copy/Download of one object's YAML from the Inspector and from row/header menus, clean by default.

**Independent test**: Inspector YAML tab → Copy (clean) has no server-managed fields; toggle off → raw; Download filename `deployment-default-web.yaml`; row menu → Export YAML… dialog.

- [x] T008 [P] [US1] Implement `cleanManifest`, `renderManifest`, `renderBundle`, `exportFilename` per `contracts/clean-rules.md` C1-C8 in `ui/src/features/manifest-export/clean.ts`
- [x] T009 [P] [US1] Vitest for every clean rule C1-C8, YAML round-trip of tricky strings (FR-013), and filenames (FR-005) in `ui/src/features/manifest-export/clean.test.ts` (landed as `ui/tests/unit/manifest-export/clean.test.ts`; unit tests live under `ui/tests/unit/`)
- [x] T010 [P] [US1] Kind→apiVersion table and `manifestRefFor(descriptor,row)` (built-ins + CRD instances; synthetic kinds excluded) in `ui/src/features/manifest-export/kinds.ts`, with tests
- [x] T011 [P] [US1] `copyText`/`downloadText` helpers with toasts in `ui/src/features/manifest-export/io.ts`
- [x] T012 [US1] Backend `fetch_manifests` in `src/backend/manifest_export.rs`: cap 500, discovery per distinct GVK, `Api<DynamicObject>::get`, concurrency 8, per-index result; Secret redaction (`data`/`stringData` → `<redacted>`) and last-applied removal; unit tests for redaction, cap, index bookkeeping
- [x] T013 [US1] Dispatch `FetchManifests` in `src/backend/cmd.rs`, emitting `ManifestsFetched`
- [x] T014 [US1] Drop `kubectl.kubernetes.io/last-applied-configuration` in `redacted_raw_json` in `src/backend/secrets.rs` (FR-011) with a unit test
- [x] T015 [US1] `ManifestActions` bar (Copy, Download, optional clean toggle, test ids `manifest-copy`, `manifest-download`, `manifest-clean`) in `ui/src/features/manifest-export/ManifestActions.tsx`
- [x] T016 [US1] `InspectorYamlExportBar` mounted at the top of the YAML tab in `ui/src/components/Inspector.tsx` (additive, one line)
- [x] T017 [US1] `ManifestExportDialog` + `useExportDialog` store (fetch via `FetchManifests`, stale reply guard by request id, warnings, errors, clean toggle) in `ui/src/features/manifest-export/ManifestExportDialog.tsx`; host mounted from `ui/src/components/ActionDialogHost.tsx` (host mounted in `ui/src/components/shell/AppShell.tsx` beside the other global dialogs, not `ActionDialogHost`; stores in `dialog-stores.ts`)
- [x] T018 [US1] "Export YAML…" item in the row menu and pane header menus (shown even for kinds without mutating actions) in `ui/src/components/ActionMenu.tsx`
- [x] T019 [US1] Fake replies for `FetchManifests` (raw JSON for fixture rows, `not found` for unknown names) in `ui/tests/fixtures/scenarios.ts` (replies in `ui/tests/fixtures/manifest-export.ts`, wired from `scenarios.ts`)
- [x] T020 [US1] Component tests for `ManifestActions` / dialog in `ui/src/features/manifest-export/ManifestExportDialog.test.tsx` (landed as `ui/tests/unit/manifest-export/dialogs.test.tsx`)

## Phase 4: User Story 2 - bundle export (P1)

**Goal**: Multi-select → one multi-document YAML.

**Independent test**: Check 3 rows → selection bar Export YAML → 3 documents separated by `---`; failures listed.

- [x] T021 [US2] "Export YAML" button in `SelectionBar` in `ui/src/components/ActionMenu.tsx` (test id `bulk-export`)
- [x] T022 [US2] Bundle rendering, namespace/name sort, 500 cap refusal and per-object errors in `ManifestExportDialog.tsx`
- [x] T023 [US2] Vitest for bundle ordering/separators in `clean.test.ts`

## Phase 5: User Story 3 - manifest before mutation (P2)

**Goal**: Copy/Download of the manifest in every client-side create/edit flow.

**Independent test**: RoleBinding create → Review → Manifest view shows YAML, Copy equals the applied object.

- [x] T024 [P] [US3] YAML edit review: `ManifestActions` for the edited YAML in `ui/src/components/panels/YamlEditTab.tsx`
- [x] T025 [P] [US3] RoleBinding form review: Diff/Manifest toggle + `ManifestActions` in `ui/src/components/rbac/RoleBindingForm.tsx`
- [x] T026 [P] [US3] Role assignment preview: `ManifestActions` in `ui/src/features/roles/RoleAssignmentDialog.tsx`
- [x] T027 [P] [US3] Helm upgrade and rollback previews: `ManifestActions` for the target manifest in `ui/src/components/panels/HelmUpgradePanel.tsx` and `HelmReleaseActionsPanel.tsx` (rollback preview lives in `ui/src/resources/helm.tsx`)
- [x] T028 [P] [US3] kind create confirmation: show the kind config (bare constant mirrored from `src/backend/kind.rs` or the pasted YAML) with `ManifestActions` in `ui/src/components/panels/KindClusterPanel.tsx`; Vitest drift guard that the TS bare config equals the Rust `BARE_CONFIG` (drift guard: `ui/tests/unit/manifest-export/kind-config.test.ts`)

## Phase 6: User Story 4 - kubeconfig export (P2)

**Goal**: A standalone one-context kubeconfig, credential-flagged, written 0600 by the backend after confirmation.

**Independent test**: Context picker → Export kubeconfig… → redacted preview, credential type, embedded flag → Export → confirm → result path.

- [x] T029 [US4] Pure builder in `src/backend/kubeconfig_export.rs`: `build_export(source: &serde_json::Value, context, include_embedded) -> Result<Built>` (single context/cluster/user, `current-context`), `classify_credential`, `embedded_secret_fields`, `redact`, CA-file inlining, file-reference listing
- [x] T030 [US4] Writer in `src/backend/kubeconfig_export.rs`: `~` expansion, absolute path, parent exists, not the source kubeconfig, `create_new` unless overwrite, mode 0600 (and re-chmod on overwrite)
- [x] T031 [US4] Rust unit tests: exec kept verbatim, auth-provider secret keys stripped/redacted, token/client-key-data/password stripped unless included, preview never contains secrets, 0600, refusal of source path/relative path/missing dir/existing file
- [x] T032 [US4] Dispatch `PreviewKubeconfigExport`/`ExportKubeconfig` (refuse when `!access.allows_local_features()`; logs only context/path) in `src/backend/cmd.rs`
- [x] T033 [US4] `KubeconfigExportDialog` (context select, credential badge, embedded warning, include checkbox, file references, redacted preview, path, overwrite, confirm step via `ConfirmDialog`, result) in `ui/src/features/manifest-export/KubeconfigExportDialog.tsx`
- [x] T034 [US4] "Export kubeconfig…" entry in `ui/src/components/ContextPicker.tsx`, hidden when `transportKind() === 'hosted'`
- [x] T035 [US4] Fake replies for the two kubeconfig commands (a token-bearing fixture context and an exec one) in `ui/tests/fixtures/scenarios.ts` (in `ui/tests/fixtures/manifest-export.ts`)

## Phase 7: Polish & cross-cutting

- [x] T036 Playwright `ui/tests/e2e/manifest-export.spec.ts`: Inspector copy clean/raw, download, row-menu dialog, bulk export, RoleBinding manifest view, kind config preview, kubeconfig export with confirm (chromium + webkit, clipboard via `prepareClipboard`/`readClipboard`) (also asserts no cluster mutation is sent; the devbridge additionally pins both kubeconfig commands to its own context in `tools/devbridge/src/guard.rs`)
- [x] T037 Gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, e2e on chromium + webkit (workspace clippy is blocked by an environmental error inside the tauri 2.10.3 registry crate; clippy with `--exclude fathom-desktop` is clean)
- [x] T038 Live check on `kind-fathom-dev`: export a Deployment clean, `kubectl --context kind-fathom-dev apply --dry-run=server -f`; record the result in `specs/040-manifest-export/checklists/live-validation.md` (automated as `ui/tests/live/manifest-export.spec.ts`)
- [x] T039 Update `docs/FEATURE_BACKLOG.md`: mark the shipped priority items and keep deferred ones (SA kubeconfig minting, merge into `~/.kube/config`, namespace/Application bundles, zip, node-shell/debug-container manifests, Helm/kustomize, OIDC sync)

## Dependencies

- Phase 1 → Phase 2 → US1. US2 depends on US1's dialog (T017). US3 depends only on T015 (`ManifestActions`). US4 depends only on Phase 2.
- Polish runs after all stories.

## Parallel opportunities

- T008/T009/T010/T011 in parallel. T012-T014 (Rust) in parallel with the UI tasks T015-T018.
- US3 tasks T024-T028 touch different files and run in parallel. US4's Rust tasks (T029-T032) run in parallel with its UI tasks (T033-T034).

## Implementation strategy

MVP = Phases 1-3 (US1). Then US2 (small, reuses the dialog), then US3 and US4 in parallel, then polish, gates and the live check.
