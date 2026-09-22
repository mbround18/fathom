# Protocol Contract: v1 additions for Applications & Related Resources

All variants below are additive to `src/protocol/v1/{cmd,event}.rs`. None touch `mutation.rs` —
this feature adds zero mutating commands (spec.md FR-018), so every command below classifies as
`ReadOnly` and needs no `mutation_target()`/write-confirmation wiring. Every command and event
variant must get a serialization round-trip test in `tests/protocol_serde.rs` (Constitution
Principle III), following the exact pattern already used for `RefreshCrds`/`SelectCrdKind`/
`CrdCatalogUpdated`.

## Commands (`BackendCmdV1`)

### `RefreshApplications`
```
RefreshApplications
```
Synthesizes the current Application list (research.md R1–R4) from already-available Pod/
Deployment/StatefulSet/DaemonSet/Service/Ingress data plus the Flux `Kustomization`/`HelmRelease`
lists already read by the Flux integration. Emits `ResourceListEvent::ApplicationsUpdated`. Also
triggered automatically after `SetContext`, mirroring `RefreshCrds`'s existing on-context-switch
behavior.

### `SelectApplication`
```
SelectApplication {
    namespace: String,
    instance: String,
}
```
Looks up one application by its `(namespace, instance)` identity (matching `ApplicationSummary`).
Emits `ResourceReadEvent::ApplicationDetailsUpdated`, followed by
`ResourceReadEvent::ApplicationMetricsUpdated` once the aggregated Prometheus query (if a metrics
source is configured) resolves — same two-event split already used for
`SelectDeployment`/`DeploymentMetricsUpdated`.

### `RefreshRelatedResources`
```
RefreshRelatedResources {
    section: SectionKey,
    kind: String,
    namespace: Option<String>,
    name: String,
}
```
Runs the generic related-resources resolver (research.md R5, data-model.md's relationship
vocabulary table) for the identified resource. Emits
`ResourceReadEvent::RelatedResourcesUpdated`. Called by the frontend whenever an inspector opens
for a kind that has a defined relationship (Application, Deployment, StatefulSet, DaemonSet,
ReplicaSet, Pod, Node, Service, PersistentVolumeClaim) — for kinds whose existing `SelectX` read
already includes everything needed (e.g. a Deployment's owned pods, already present in
`WorkloadDetails.pods`), the frontend derives the `RelatedResourceRef`s from that read locally.
This command exists for cases requiring a resolver-side lookup not otherwise present on the
already-fetched details (Node → its Pods, Service → its Pods, PVC → its mounting Pods, Pod →
its Application, Pod → its owning workload when not already inlined).

## Events

### `ResourceListEvent::ApplicationsUpdated`
```
ApplicationsUpdated(Vec<ApplicationSummary>)
```
Reply to `RefreshApplications`. Respects the active namespace filter (spec.md FR-005), same as
every other `ResourceListEvent` variant.

### `ResourceReadEvent::ApplicationDetailsUpdated`
```
ApplicationDetailsUpdated(ApplicationDetails)
```
Reply to `SelectApplication`.

### `ResourceReadEvent::ApplicationMetricsUpdated`
```
ApplicationMetricsUpdated(WorkloadMetricsAggregate)
```
Reply to `SelectApplication`'s metrics half; reuses the existing `WorkloadMetricsAggregate` type
verbatim (no new metrics shape). When no metrics source is configured, this event still fires with
the same "unavailable" representation `DeploymentMetricsUpdated` already uses today, so the
frontend needs no new empty-state handling.

### `ResourceReadEvent::RelatedResourcesUpdated`
```
RelatedResourcesUpdated {
    section: SectionKey,
    kind: String,
    namespace: Option<String>,
    name: String,
    related: Vec<RelatedResourceRef>,
}
```
Reply to `RefreshRelatedResources`, echoing back the identifying fields so the frontend can
discard a stale reply that arrives after the operator has already navigated elsewhere (Constitution
Principle V: "events from a superseded request MUST be discarded").

## New shared types (`src/protocol/v1/applications.rs`, new module)

```
ApplicationManagedBy: Helm | Flux | LabelHeuristic     // fieldless enum, data-model.md

ApplicationSummary {
    instance: String,
    name: String,
    namespace: String,
    managed_by: ApplicationManagedBy,
    version: Option<String>,
    creation_timestamp: String,
    ready_count: i32,
    total_count: i32,
}

ApplicationDetails {
    instance: String,
    name: String,
    namespace: String,
    managed_by: ApplicationManagedBy,
    version: Option<String>,
    creation_timestamp: String,
    labels: BTreeMap<String, String>,
    urls: Vec<String>,
    related: Vec<RelatedResourceRef>,
}

RelatedResourceRef {
    section: SectionKey,
    kind: String,
    namespace: Option<String>,
    name: String,
    relation: String,
    status_hint: Option<String>,
}
```

## Existing types reused unchanged

- `WorkloadMetricsAggregate` (already defined for `DeploymentMetricsUpdated` etc.) — application
  metrics reuse it verbatim, no new metrics protocol type.
- `SectionKey` — gains one new value, `"applications"`, following the exact pattern by which
  `"custom-resources"` was added for CRD browsing.

## Frontend-only additions (no protocol impact)

- `PaneState.history: ObjectRef[]` (data-model.md, research.md R6) — pure UI state, not part of
  the wire protocol, not persisted to `~/.fathom/`.
