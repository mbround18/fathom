---
description: "Task list for Flux mutate (suspend, resume, reconcile)"
---

# Tasks: Flux mutate (suspend, resume, reconcile)

**Input**: `specs/049-flux-mutate/{spec,plan}.md`, `contracts/protocol.md`

## Phase 1: Foundational (protocol)

- [ ] T001 `FluxVerb`, `FluxMutationResult`, `FluxResourceSummary.managed` in `src/protocol/v1/flux.rs`
- [ ] T002 `SuspendFluxResource`, `ReconcileFluxResource` in `src/protocol/v1/cmd.rs`; `FluxMutationApplied/Failed` in `event.rs`; `AuditActionKind::FluxControl` in `audit.rs`
- [ ] T003 `mutation.rs` arms: Namespaced target, `flux_resource()` section, FluxControl audit summary, hosted Allowed; `server/src/filter.rs` table rows
- [ ] T004 Round-trip tests in `tests/protocol_serde.rs` (2 cmds, 2 events); regenerate ts bindings

## Phase 2: US1 suspend/resume

- [ ] T010 `managed()` from `status.inventory` (Kustomization) / release name (HelmRelease) in `src/backend/flux.rs`, with a unit test for inventory-id parsing
- [ ] T011 `suspend_flux_resource()` merge patch + re-read + overview refresh; dispatch arm
- [ ] T012 Inspector verbs Suspend/Resume with `ConfirmDialog` listing managed objects (or dependents for a source) in `ui/src/resources/flux-resource.tsx`; store handlers for the two events (toast, details update)
- [ ] T013 e2e `ui/tests/e2e/us1-flux-suspend.spec.ts` on the fake backend

## Phase 3: US2 reconcile

- [ ] T020 `reconcile_flux_resource()` annotation patch, with-source resolution of `spec.sourceRef` (kind, name, namespace fallback); dispatch arm
- [ ] T021 Inspector verb Reconcile with with-source checkbox (Kustomization/HelmRelease only), suspended warning
- [ ] T022 e2e `ui/tests/e2e/us2-flux-reconcile.spec.ts`

## Phase 4: US3 pane

- [ ] T030 `FluxPanel.tsx` lists every resource grouped by kind, not-Ready first, `onSelect` opens the inspector
- [ ] T031 e2e `ui/tests/e2e/us3-flux-pane.spec.ts`

## Phase 5: Verification and close

- [ ] T040 Live: suspend, resume and reconcile-with-source the podinfo HelmRelease on `kind-fathom-dev` from the built app or a backend integration test; record in this file
- [ ] T041 Promo shot of the Flux pane with verbs (last); `docs/MVP.md` row; hygiene
