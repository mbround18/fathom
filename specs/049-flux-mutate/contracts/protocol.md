# Protocol contract: Flux mutate (049)

Defined once in Rust (`src/protocol/v1/{cmd,event,flux,audit,mutation}.rs`), exported to TypeScript through ts-rs.

## Commands (`BackendCmdV1`)

```rust
/// Suspend or resume one Flux resource (merge patch of `spec.suspend`).
SuspendFluxResource { kind: String, namespace: String, name: String, suspend: bool },
/// Request reconciliation now (`reconcile.fluxcd.io/requestedAt` annotation).
/// `with_source` first annotates the resource's `spec.sourceRef`; ignored for sources.
ReconcileFluxResource { kind: String, namespace: String, name: String, with_source: bool },
```

Classification (`mutation.rs`): both are `MutationTarget::Namespaced`, section key `flux_resource()`, audit summary `AuditActionKind::FluxControl` with the resource's kind, namespace and name, hosted policy `Allowed`.

## Events

No new event variants. Both commands are ordinary mutations: the backend answers with
`CommandFinished` (the UI's action runner turns it into the success or error toast) and a
`notify_info` line naming what happened ("Suspended HelmRelease fathom-dev/podinfo",
"Reconcile requested for HelmChart fathom-dev/fathom-dev-podinfo, then HelmRelease fathom-dev/podinfo").
After a successful patch the backend re-reads the resource (`FluxResourceDetailsUpdated`) and
rescans the overview (`FluxOverviewUpdated`), so the inspector and pane update without a poll tick.

The `FluxVerb` / `FluxMutationResult` / `FluxMutationApplied|Failed` shapes in the first draft of
this contract were dropped during implementation: they duplicated what `CommandFinished` already
carries for every other verb (CronJob suspend, node cordon).

## Summary extension

`FluxResourceSummary` gains `managed: Vec<String>`: for a Kustomization, `status.inventory.entries[].id` rendered as `Kind namespace/name`; for a HelmRelease, the Helm release it owns as `Helm release <storageNamespace|targetNamespace|namespace>/<releaseName|name>`; empty for sources (their dependents are computed in the UI from the overview's `source_ref`).

## Audit

`AuditActionKind::FluxControl` (new variant, label "Flux Control"); the audit summary carries the resource's kind, namespace and name.
