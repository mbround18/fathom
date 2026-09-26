# Implementation Plan: Manifest Export (never blackbox the cluster)

**Branch**: `040-manifest-export` | **Date**: 2026-09-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/040-manifest-export/spec.md`

## Summary

Every object Fathom shows can be exported as plain Kubernetes YAML, and every object it creates can be seen as YAML before it is applied. There are three parts:

1. **Backend fetch.** A new read-only protocol command, `FetchManifests`, reads any set of objects through the dynamic API (discovery by `apiVersion`/`kind`, the same way `ApplyResourceYaml` works). It returns their raw JSON. Secrets are redacted in the backend, so values never cross the protocol.
2. **Client-side rendering.** One TypeScript module (`ui/src/features/manifest-export/clean.ts`) implements the spec's clean-for-re-apply rules and renders YAML with the `yaml` library, as single or multi-document output. It is shared by the Inspector YAML tab (which uses the `raw_json` it already holds), the export dialog (row, header menu and bulk selection), and a reusable `ManifestActions` bar for the create/edit flows.
3. **Kubeconfig export.** Two desktop-only commands, `PreviewKubeconfigExport` and `ExportKubeconfig`. The backend builds a one-context kubeconfig from the kubeconfig file Fathom loaded, working on the untyped YAML so exec/auth-provider entries survive byte-for-byte in meaning. It classifies and redacts credentials for the preview, and writes the file itself with mode 0600 after the operator confirms. The unredacted kubeconfig never enters the webview.

## Technical Context

**Language/Version**: Rust 2024 (workspace toolchain), TypeScript ~6.0 / React 19

**Primary Dependencies**: kube-rs 2 (`discovery::oneshot::pinned_kind`, `Api<DynamicObject>`), serde_yaml/serde_json, ts-rs 12 (bindings), zustand, `yaml` (eemeli/yaml, new UI dependency for spec-correct YAML 1.2 output with string quoting), sonner toasts, Radix dropdown/dialog

**Storage**: None new. The kubeconfig export writes one operator-chosen file.

**Testing**: `cargo test` (unit tests in `src/backend/manifest_export.rs` and `src/backend/kubeconfig_export.rs`, protocol round-trips in `cmd.rs`/`event.rs`, hosted-policy classification), Vitest (clean rules, YAML round-trip, dialog and bar components), Playwright mock-mode (`ui/tests/e2e/manifest-export.spec.ts`) on chromium and webkit

**Target Platform**: Desktop (Tauri) Linux/macOS/Windows. Manifest export also runs in hosted browser mode. Kubeconfig export is desktop-only.

**Project Type**: Desktop app, Rust backend plus webview UI over the versioned protocol

**Performance Goals**: A 50-object bundle renders in under 3 s on a local cluster (SC-003). Fetches run 8 at a time; YAML rendering is synchronous and cheap.

**Constraints**: No credential value over the protocol or in logs (Principle VI). Export needs only `get`. The bundle cap is 500 objects.

**Scale/Scope**: About 2 Rust modules, 3 protocol commands and 3 events, 1 UI feature folder, small additive edits to Inspector, ActionMenu, the create flows and ContextPicker.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Assessment |
|---|---|
| I. Operator-first, explicit mutation | Manifest export is read-only. Kubeconfig export writes a local file only after a separate confirmation dialog that names the context, path and secret inclusion (FR-028), and backend errors are shown verbatim. **Pass** |
| II. One contract, backend owns the cluster | Cluster reads and kubeconfig access happen in Rust. The UI talks only through new `BackendCmdV1`/`SystemEvent` variants with generated TS bindings. Cleaning is presentation of data already received, so it runs client-side. **Pass** |
| III. Tested at the boundaries | Round-trip tests for every new variant, a binding drift check (`cargo test --test export_bindings`), Rust unit tests for the kubeconfig builder and Secret redaction, Vitest for the clean rules, and Playwright e2e for every user story. **Pass** |
| IV. Density scales with width | The dialog uses the existing responsive Dialog. The YAML toolbar wraps at narrow widths. Everything is keyboard and touch operable. **Pass** |
| V. Fast and quiet | No polling. The fetch is bounded (500 objects, 8 concurrent). Replies are keyed by request id, so a superseded request is discarded. **Pass** |
| VI. User data is sacred | Secrets are redacted before the protocol. The kubeconfig preview is redacted and the full file is written by the backend at 0600. Tokens are never logged. The source kubeconfig can never be the target. **Pass** |
| Hosted mode | `PreviewKubeconfigExport`/`ExportKubeconfig` → `HostedPolicy::Denied(Contexts)`, and the backend refuses when `!allows_local_features()`. The UI hides the entry when `transportKind() === 'hosted'`. **Pass** |

Post-design re-check: unchanged, all pass. No Complexity Tracking entries.

## Project Structure

### Documentation (this feature)

```text
specs/040-manifest-export/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── protocol.md        # new commands/events
│   └── clean-rules.md     # exact clean-for-re-apply rules (normative for tests)
├── checklists/requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/protocol/v1/
├── manifest_export.rs      # NEW: ManifestObjectRef, FetchedManifest, ManifestFetchError,
│                           #      KubeconfigCredentialKind, KubeconfigExportPreview, ...
├── cmd.rs                  # + FetchManifests, PreviewKubeconfigExport, ExportKubeconfig
├── event.rs                # + SystemEvent::ManifestsFetched / KubeconfigExportPreviewed /
│                           #   KubeconfigExportFinished
├── mutation.rs             # classification: ReadOnly / Local; hosted policy Denied(Contexts)
└── mod.rs
src/backend/
├── manifest_export.rs      # NEW: dynamic fetch + Secret redaction (unit-tested)
├── kubeconfig_export.rs    # NEW: standalone kubeconfig builder, classifier, redactor, 0600 writer
├── secrets.rs              # drop last-applied annotation from redacted raw_json (FR-011)
└── cmd.rs / cmd/resources.rs  # dispatch
server/src/filter.rs        # test list of hosted-denied commands
ui/src/features/manifest-export/
├── clean.ts                # clean rules + YAML rendering (single/multi-doc)
├── kinds.ts                # kind → apiVersion table for exportable built-ins; CRD support
├── io.ts                   # copyText / downloadText helpers
├── state.ts                # reducer for ManifestsFetched / Kubeconfig* events
├── ManifestActions.tsx     # Copy / Download (+ optional clean toggle) bar
├── ManifestExportDialog.tsx
├── KubeconfigExportDialog.tsx
└── *.test.ts(x)
ui/src/components/Inspector.tsx        # + ManifestActions above YAML tab (additive)
ui/src/components/ActionMenu.tsx       # + "Export YAML…" row/header item, selection-bar button
ui/src/components/ContextPicker.tsx    # + "Export kubeconfig…" (desktop only)
ui/src/components/panels/YamlEditTab.tsx, rbac/RoleBindingForm.tsx,
  features/roles/RoleAssignmentDialog.tsx, panels/HelmUpgradePanel.tsx,
  panels/HelmReleaseActionsPanel.tsx, panels/KindClusterPanel.tsx   # + ManifestActions
ui/tests/fixtures/scenarios.ts         # fake replies for the three commands
ui/tests/e2e/manifest-export.spec.ts   # NEW e2e
```

**Structure Decision**: The existing single-workspace layout (Rust backend in `src/`, UI in `ui/`). New UI code lives in one feature folder so edits to shared files (`Inspector.tsx`, `ActionMenu.tsx`, `helpers.ts`) stay small and additive, which limits merge conflicts with concurrent agents.

## Complexity Tracking

None.
