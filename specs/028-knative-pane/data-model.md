# Data Model: Knative Pane

## Protocol types (`src/protocol/v1/knative.rs`, generated to `ui/src/protocol/generated/*.ts`)

### `KnativeScaleReason` (enum)

Discriminated reason for a Revision's actual-vs-desired replica gap, always paired with the raw
numbers so the UI never has to guess when the enum is `Unavailable`.

```rust
pub enum KnativeScaleReason {
    AtDesiredScale,
    ScaledToZero { idle_since: Option<String> },   // ISO8601 lastTransitionTime, if known
    ScalingUp { pending_requests: Option<u32> },     // count, if the autoscaler exposed it
    Unavailable,                                     // fields missing/unrecognized
}
```

### `KnativeRevisionSummary`

- `name: String`, `namespace: String`
- `service_name: String` — owning Knative Service (from owner ref or `serving.knative.dev/service` label)
- `ready: FluxReadyState`-shaped tri-state — reuse a `KnativeReadyState` enum (`Ready | NotReady | Unknown`) rather than importing Flux's type (different domain), to keep protocol modules independent
- `actual_replicas: u32`
- `desired_replicas: u32`
- `scale_reason: KnativeScaleReason`
- `traffic_percent: Option<u32>` — convenience denormalization: the total percent this Revision currently receives across all Routes targeting it (0 if none); the per-Route breakdown lives in `KnativeRouteSummary.traffic`

### `KnativeRouteTrafficTarget`

- `revision_name: String` — always resolved to a concrete Revision name (Configuration targets are pre-resolved from `status.traffic`, see research.md R2)
- `percent: u32`
- `tag: Option<String>`
- `revision_missing: bool` — true if `revision_name` does not match any Revision this scan found (FR-007's stale-target case)

### `KnativeRouteSummary`

- `name: String`, `namespace: String`
- `service_name: String`
- `ready: KnativeReadyState`
- `url: Option<String>`
- `traffic: Vec<KnativeRouteTrafficTarget>`

### `KnativeServiceSummary`

- `name: String`, `namespace: String`
- `ready: KnativeReadyState`
- `latest_ready_revision: Option<String>`
- `latest_created_revision: Option<String>`

### `KnativeOverview`

- `installed: bool`
- `services: Vec<KnativeServiceSummary>`
- `revisions: Vec<KnativeRevisionSummary>`
- `routes: Vec<KnativeRouteSummary>`

Sorted by `(namespace, name)` within each list, matching `flux.rs`'s existing sort convention.

## Frontend store slice (`ui/src/store/index.ts`)

```ts
knative: { overview?: KnativeOverview }
```

Populated by the `KnativeOverviewUpdated` event handler in `ui/src/store/handlers/system.ts`
(mirrors the existing `FluxOverviewUpdated` handler).

## Topology extensions (`ui/src/topology/types.ts`)

- `TopologyEdge.label?: string` — new optional field; when set, `TopologyCanvas.draw()` renders it
  at the edge's screen-space midpoint. `undefined` for every existing (non-Knative) edge, so
  current rendering is unchanged.
- `TopologyNodeKind` gains `'KnativeRevision' | 'KnativeRoute'` (Knative "Service" reuses no
  existing kind and is represented only as the graph's implicit grouping label in the pane header,
  not as its own node, since a Route already names its owning Service and the traffic graph's
  interesting structure is Route → Revision).

## Validation rules (from Requirements)

- `installed` is `false` only when none of the four CRDs are served (FR-001); a partially-served
  CRD set still sets `installed: true` and simply yields empty lists for the unserved kinds.
- `KnativeRouteTrafficTarget.percent` values are displayed exactly as read; the system never
  renormalizes them to sum to 100 (edge case: "don't fabricate a corrected total").
- `scale_reason` is `Unavailable` whenever the classifying condition/fields can't be parsed —
  never inferred from replica counts alone when the counts already match (`AtDesiredScale` takes
  priority whenever `actual_replicas == desired_replicas`, per FR-009).
- No field or command in this model has a corresponding mutate path; `RefreshKnative` is the only
  new command and it is a read trigger, matching FR-011.

## State transitions

None — all types here are point-in-time snapshots refreshed wholesale on each poll/refresh, same
as `FluxOverview`. No partial/incremental update model.
