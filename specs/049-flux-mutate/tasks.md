---
description: "Task list for Flux mutate (suspend, resume, reconcile)"
---

# Tasks: Flux mutate (suspend, resume, reconcile)

**Input**: `specs/049-flux-mutate/{spec,plan}.md`, `contracts/protocol.md`

## Phase 1: Foundational (protocol)

- [x] T001 `FluxResourceSummary.managed` in `src/protocol/v1/flux.rs` (`FluxVerb`/`FluxMutationResult` dropped: see contract, Events)
- [x] T002 `SuspendFluxResource`, `ReconcileFluxResource` in `src/protocol/v1/cmd.rs`; `AuditActionKind::FluxControl` in `audit.rs` (no new events: `CommandFinished` + `notify_info`)
- [x] T003 `mutation.rs` arms: Namespaced target, `flux_resource()` section, FluxControl audit summary, hosted Allowed (`server/src/filter.rs` derives its table; no rows to add)
- [x] T004 Round-trip tests in `tests/protocol_serde.rs` (2 cmds); ts bindings regenerated

## Phase 2: US1 suspend/resume

- [x] T010 `managed()` from `status.inventory` (Kustomization) / release name (HelmRelease) in `src/backend/flux.rs`, with a unit test for inventory-id parsing
- [x] T011 `suspend_flux_resource()` merge patch + re-read + overview refresh; dispatch arm
- [x] T012 Inspector verbs Suspend/Resume with `ConfirmDialog` listing managed objects (or dependents for a source) in `ui/src/resources/flux-resource.tsx`; `ActionDescriptor.detail`/`applies`; the overview doubles as the section's list so the inspector menu finds its row
- [x] T013 e2e `ui/tests/e2e/us1-flux-suspend.spec.ts` on the fake backend

## Phase 3: US2 reconcile

- [x] T020 `reconcile_flux_resource()` annotation patch, with-source resolution of `spec.sourceRef` (kind, name, namespace fallback); dispatch arm
- [x] T021 Inspector verbs Reconcile and Reconcile with source (the latter Kustomization/HelmRelease only), suspended warning
- [x] T022 e2e `ui/tests/e2e/us2-flux-reconcile.spec.ts`

## Phase 4: US3 pane

- [x] T030 `FluxPanel.tsx` lists every resource grouped by kind, not-Ready first, `onSelect` opens the inspector
- [x] T031 e2e `ui/tests/e2e/us3-flux-pane.spec.ts`

## Phase 5: Verification and close

- [x] T040 Live: `flux_verbs_against_live_podinfo` (ignored test in `src/backend/flux.rs`, run with `FLUX_LIVE_CONTEXT=<context>`) suspended, re-read (`Suspended`, managed = `Helm release fathom-dev/podinfo`), resumed and reconciled-with-source the podinfo HelmRelease. Run 2026-10-08 against `kind-fathom-dev` (1.3s) and, through `~/.kube/fathom-agent.yaml`, `prod-fathom-agent` (5.8s); with-source stamped `HelmChart fathom-dev/fathom-dev-podinfo` then the release; both releases left resumed and Ready. Note: the first run went to the kubeconfig's current context (`default`, prod admin) before the test required an explicit context; it only touched `fathom-dev/podinfo` and left it as found.
- [x] T041 `docs/MVP.md` row; promo `wide-flux` reshot (the pane now lists every resource); hygiene clean
