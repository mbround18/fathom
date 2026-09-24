# Quickstart: In-App Local Kind Cluster Provisioning

## Prerequisites

- `kind` and Docker (or Podman) installed and running on the dev machine (for manual/live
  validation; the Playwright e2e suite runs against a scripted fake backend and needs neither).
- Fathom built from this branch (`cargo build`, `pnpm -C ui install`).

## Manual validation scenarios

1. **Prerequisite detection**
   - Rename/hide the `kind` binary from `PATH` temporarily (or run in a container without it).
   - Open Fathom → Settings → Cluster management (or wherever `KindClusterPanel` is mounted).
   - Expect: "kind is not installed" guidance with an install link; create action disabled.
   - Restore `kind` on `PATH`, refresh the panel → expect ready state.

2. **List existing clusters**
   - Run `kind create cluster --name manual-check` from a terminal (simulating pre-existing
     state) before opening Fathom.
   - Open the panel → expect `manual-check` listed with status `ready`.

3. **Create a cluster in-app**
   - Click "Create cluster", enter a valid name (e.g. `fathom-demo`), leave profile as "Bare".
   - Expect a confirmation dialog summarizing name/profile/node count before anything runs.
   - Confirm → expect a progress view updating every few seconds until success.
   - Expect `fathom-demo` to appear in both the panel's list and the app's cluster/context
     switcher, immediately selectable, with no separate import step.

4. **Delete a cluster in-app**
   - Select `fathom-demo` → "Delete" → expect a destructive confirmation naming
     `fathom-demo` explicitly.
   - Confirm → expect the cluster to disappear from both the panel's list and the context
     switcher once the operation finishes.

5. **Invalid/colliding name rejection**
   - Attempt to create a cluster named `Manual_Check!` (invalid characters) → expect inline
     rejection before any confirmation dialog is reachable.
   - Attempt to create a cluster named `manual-check` (already exists) → expect inline
     rejection.

## Automated validation

- `cargo test` — protocol round-trip tests for the new command/event variants, name-validation
  unit tests, kubeconfig-merge non-destructiveness fixture test.
- `cargo clippy --all-targets -- -D warnings`
- `pnpm -C ui typecheck`, `pnpm -C ui lint`
- `pnpm -C ui test:e2e -- kind-cluster-provisioning` (or the project's equivalent Playwright
  invocation) — runs the four scripted scenarios above (prerequisite empty state, create
  confirm+progress, delete confirm, context-switcher appearance) against the fake backend.
