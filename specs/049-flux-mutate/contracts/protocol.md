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

## Events (`SystemEvent`)

```rust
FluxMutationApplied(FluxMutationResult),
FluxMutationFailed(FluxMutationResult),
```

```rust
pub enum FluxVerb { Suspend, Resume, Reconcile, ReconcileWithSource }

pub struct FluxMutationResult {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub verb: FluxVerb,
    /// Empty on success; the API server's message on failure.
    pub message: String,
}
```

After `FluxMutationApplied` the backend also emits `FluxResourceDetailsUpdated` for the resource (re-read) and `FluxOverviewUpdated`.

## Summary extension

`FluxResourceSummary` gains `managed: Vec<String>`: for a Kustomization, `status.inventory.entries[].id` rendered as `Kind namespace/name`; for a HelmRelease, the Helm release it owns as `HelmRelease-release namespace/name`; empty for sources (their dependents are computed in the UI from the overview's `source_ref`).

## Audit

`AuditActionKind::FluxControl` (new variant), summary `"<verb> <Kind> <namespace>/<name>"`.
