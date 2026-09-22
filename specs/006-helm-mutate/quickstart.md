# Quickstart: Helm Mutate (006)

Validates all three mutate flows end-to-end against a real cluster. No live cluster is required for the diff-preview computation logic or rollback/uninstall backend logic themselves (Constitution Principle III) — those get unit tests against fixture data — but this guide exercises the full stack including the real `helm` binary dependency (R4).

## Prerequisites

- A running Kubernetes context reachable by `kubectl`/`kube-rs` (the dev sandbox, `dev/sandbox/`, already provides one Flux-managed release — `podinfo`, `dev/sandbox/helm.yaml` — useful for GitOps-warning scenarios).
- The `helm` binary on `PATH` (required for upgrade preview/apply and uninstall — R4). Verify with `helm version`. If absent, upgrade/uninstall MUST surface FR-012's clear error rather than silently doing nothing — confirm this degraded path too.
- A second, bare (non-GitOps) Helm release installed directly for testing the "no warning" path, e.g.:
  ```sh
  helm install quickstart-bare oci://ghcr.io/stefanprodan/charts/podinfo -n default
  ```

## Scenario 1 — Rollback (US1, P1)

1. In the Helm Releases view, open `podinfo` (or `quickstart-bare`) and look at its revision history (already shown read-only from 005).
2. Bump a value and upgrade once via `helm upgrade` from the CLI directly (to create a second revision to roll back from), or use Scenario 2 first.
3. In the app, select "Roll back" on revision 1. **Expect**: a diff-preview appears showing the manifest differences between the current live revision and revision 1's stored manifest, computed instantly (no chart rendering — R4) and with zero cluster mutation before confirming (verify via `kubectl get -n <ns> <resource> -o yaml` unchanged).
4. Confirm. **Expect**: the release's revision count increases by one; the new revision's live manifest matches revision 1's; verify via `helm history <release> -n <ns>`.
5. Repeat, but cancel at the preview step. **Expect**: no revision added, no resource changed.
6. Repeat against `podinfo` (Flux-managed). **Expect**: the GitOps-ownership warning appears in the confirm step, linking back to the owning Flux `HelmRelease`. Repeat against `quickstart-bare`. **Expect**: no warning.

## Scenario 2 — Upgrade (US2, P2)

1. Open `quickstart-bare`, select "Upgrade". **Expect**: the existing YAML editor pre-loads the release's current effective values (no new form UI).
2. Edit a value (e.g. `replicaCount`), request a preview. **Expect**: a diff-preview shows the rendered-manifest difference produced by `helm upgrade --dry-run` against the edited values, with zero cluster mutation.
3. Confirm. **Expect**: a new revision appears with the edited value live; verify via `kubectl get`.
4. Repeat with deliberately invalid YAML in the editor. **Expect**: a clear, specific error (FR-012) at the preview step, no revision created.
5. Repeat against `podinfo`. **Expect**: the GitOps-ownership warning appears before the diff-preview step.

## Scenario 3 — Uninstall (US3, P3)

1. Open `quickstart-bare`, select "Uninstall". **Expect**: `ConfirmDialog` names the release by namespace/name; no typed-name input required (single-target action, R3).
2. Confirm. **Expect**: the release disappears from the release list; its owned resources are garbage-collected via owner references (verify via `kubectl get all -n default -l app.kubernetes.io/instance=quickstart-bare` returning nothing shortly after).
3. Re-install `quickstart-bare` and repeat, cancelling instead. **Expect**: release untouched.
4. Repeat against `podinfo`. **Expect**: the GitOps-ownership warning appears in the confirm dialog, since Flux will likely recreate the release on its next reconciliation.

## Automated coverage (run instead of/alongside manual validation)

- `cargo test` — protocol round-trip tests for all 5 new `BackendCmdV1` / 3 new `SystemEvent` variants (`tests/protocol_serde.rs`); unit tests for rollback diff computation and stale-base-revision detection against fixture `HelmReleaseDetails`/`HelmRevisionSummary` data (no live cluster).
- `pnpm -C ui test` — unit tests for the diff-view generalization and the GitOps-warning banner rendering logic.
- `pnpm -C ui e2e` — scripted-fake-backend E2E tests per user story (Constitution Principle III), driving the UI by injecting `HelmMutationPreviewReady`/`Applied`/`Failed` events and asserting the emitted `Preview*`/`Apply*`/`UninstallHelmRelease` commands, including the GitOps-warning-present and GitOps-warning-absent cases from FR-009/FR-010.
