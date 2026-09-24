# Quickstart: Validating the Velero Backup Pane

## Prerequisites

- A running Fathom dev build (`cargo tauri dev` or equivalent) connected to a kubeconfig context.
- For the "installed" path: a cluster (kind/minikube is fine) with Velero's CRDs applied — either
  a real Velero install, or just the four CRDs (`Backup`, `Schedule`, `Restore`,
  `BackupStorageLocation` under `velero.io`) plus a couple of sample custom resources, since this
  feature only reads CRDs/CRs and never calls the Velero server itself.
- For the "not installed" path: any cluster without those CRDs (a fresh kind cluster works).

## Manual validation

1. **Not-installed empty state** (User Story 4 / FR-003):
   - Connect Fathom to a cluster without `velero.io` CRDs.
   - Open the Integrations → Velero pane.
   - Expect: "Velero is not installed on this cluster" (or equivalent honest empty state), no
     errors in the notification tray, no blank/loading-forever state.

2. **Backup statuses and age** (User Story 1 / FR-004, FR-008, SC-001, SC-002):
   - Apply a completed `Backup` and a failed `Backup` (see
     `ui/tests/e2e/fixtures/velero-*.json` or equivalent test fixture created in this feature's
     tasks for exact YAML/JSON shapes).
   - Open the Velero pane.
   - Expect: both backups listed, visually distinguishable status (e.g. green/red tone), and a
     human-readable age relative to their completion/failure time.

3. **Schedule last-backup age** (User Story 2 / FR-005, SC-003):
   - Apply a `Schedule` plus a `Backup` labeled `velero.io/schedule-name: <schedule-name>` with a
     known `completionTimestamp`.
   - Open the Velero pane.
   - Expect: the schedule shows an age consistent with "now − completionTimestamp", accurate to
     within the pane's refresh interval (20s, or immediately after clicking refresh / opening the
     pane).
   - Apply a second `Schedule` with no matching Backup.
   - Expect: "no backups yet" rather than a blank or zero age.

4. **Restore history and storage-location health** (User Story 3 / FR-006, FR-007):
   - Apply a completed `Restore` and a failed `Restore`.
   - Apply a `BackupStorageLocation` with `status.phase: Available` and one with
     `status.phase: Unavailable` plus a `status.message`.
   - Open the Velero pane.
   - Expect: both restores shown with distinguishable status; both storage locations shown
     individually with correct health and, for the unavailable one, the reported message.

5. **Permission-denied vs. not-installed** (edge case / FR-012):
   - Connect with a kubeconfig identity that can list `CustomResourceDefinition` but is denied
     `list` on `backups.velero.io` (or similar) via RBAC.
   - Expect: a distinct "permission denied" indication, not the same message as "not installed"
     and not a silently empty list.

6. **Inspector link** (FR-011):
   - From the Velero pane, select a listed Backup/Restore/Schedule/BackupStorageLocation row.
   - Expect: it opens in the existing generic CRD Inspector (the same one used for any other
     custom resource), showing full spec/status/YAML.

## Automated validation

- `cargo test -p fathom` — unit tests for `velero::summarize`/status classification/age
  derivation, plus protocol round-trip tests for the new `SystemEvent`/`BackendCmdV1` variants.
- `cargo test -p fathom -- --ignored scan_velero_against_live_cluster --nocapture` — optional,
  mirrors `flux.rs`'s live-cluster smoke test; requires a real cluster with Velero installed.
- `pnpm -C ui test` — Vitest coverage for `VeleroPanel`'s status→tone mapping and empty states.
- `pnpm -C ui exec playwright test ui/tests/e2e/velero-pane.spec.ts` — the two-scenario e2e spec
  (completed+failed Backup with correct status/age against a fixture cluster; "Velero not
  installed" honest empty state) required by this feature.
