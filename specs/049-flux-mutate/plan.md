# Implementation Plan: Flux mutate (suspend, resume, reconcile)

**Branch**: `049-flux-mutate` | **Date**: 2026-10-08 | **Spec**: [spec.md](./spec.md)

## Summary

Add the three `flux` CLI verbs operators use day to day to the read-only Flux integration shipped in 005. Suspend/resume is a merge patch of `spec.suspend`; reconcile is the `reconcile.fluxcd.io/requestedAt` annotation, optionally on the source first. The existing served-CRD discovery and `summarize()` are reused unchanged for the post-apply re-read. The UI gets verbs in the Flux inspector, a confirm dialog that names the object and what it manages, and a pane that lists every resource with click-through.

## Technical Context

**Language/Version**: Rust edition 2024 backend and protocol; TypeScript strict frontend.
**Primary Dependencies**: `kube` (`Api<DynamicObject>::patch` with `Patch::Merge`), `chrono` (already in tree) for the RFC 3339 timestamp. No new crates.
**Testing**: `cargo test` (serde round-trips, inventory-id parsing unit test), Playwright against the scripted fake backend (one spec per story), and a live run against `kind-fathom-dev`'s podinfo HelmRelease.
**Constraints**: Constitution Principle I: explicit confirmation naming every object. Principle II: only the backend patches the cluster.

## Constitution Check

- **I Operator-first, explicit mutation**: PASS. Both verbs sit behind `ConfirmDialog` naming kind/namespace/name; the dialog lists managed objects.
- **II One contract, backend owns the cluster**: PASS. Patches happen in `src/backend/flux.rs`; the UI sends two new commands.
- **III Tested at the boundaries**: PASS, planned. Round-trips for 2 commands + 2 events + `FluxVerb`; e2e per story; live test.
- **IV Density**: N/A, reuses inspector and `IntegrationSummary`.
- **V Fast and quiet**: PASS. No new poll loop; one re-read and one overview scan per apply.
- **VI User data**: N/A.

## Project Structure

```text
src/protocol/v1/flux.rs        + FluxVerb, FluxMutationResult, FluxResourceSummary.managed
src/protocol/v1/cmd.rs         + SuspendFluxResource, ReconcileFluxResource
src/protocol/v1/event.rs       + FluxMutationApplied, FluxMutationFailed
src/protocol/v1/audit.rs       + AuditActionKind::FluxControl
src/protocol/v1/mutation.rs    + target/section/audit/hosted arms
src/backend/flux.rs            + managed() in summarize, suspend_flux_resource, reconcile_flux_resource
src/backend/cmd/<integrations>.rs  + dispatch arms
server/src/filter.rs           + hosted verdict rows
tests/protocol_serde.rs        + round-trips
ui/src/resources/flux-resource.tsx   + verbs, confirm dialogs
ui/src/components/panels/FluxPanel.tsx + full listing with click-through
ui/src/store/handlers/system.ts      + toast + inspector update on the two events
ui/tests/e2e/us1-flux-suspend.spec.ts, us2-flux-reconcile.spec.ts, us3-flux-pane.spec.ts
ui/tests/support/fake-backend scenarios  + flux overview fixture with inventory
```

**Structure Decision**: extends the 005 modules in place; no new module.

## Complexity Tracking

None. Direct commands (like `SuspendCronJob`) rather than a preview→apply pair (like Helm 006): the preview content (identity, inventory, dependents) is already in the summary the UI holds, so a preview round trip would add a command and an event for nothing.
