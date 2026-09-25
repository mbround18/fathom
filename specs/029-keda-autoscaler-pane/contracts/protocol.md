# Protocol Contract: KEDA Autoscaler Pane

All additions are backward-compatible (new enum variants/structs only) — no protocol version
bump required per constitution II.

## Commands (`src/protocol/v1/cmd.rs`, `BackendCmdV1`)

```rust
/// Refresh the dashboard-level KEDA overview (installed flag + all ScaledObjects/ScaledJobs
/// in the current namespace scope). Mirrors RefreshFlux.
RefreshKeda,

/// Single-resource lookup for the Inspector (read-only; distinct from RefreshKeda's aggregate).
/// Mirrors SelectFluxResource.
SelectScaledObject {
    namespace: String,
    name: String,
},
SelectScaledJob {
    namespace: String,
    name: String,
},
```

## Events (`src/protocol/v1/event.rs`, `SystemEvent`)

```rust
/// Dashboard-level aggregate, emitted on RefreshKeda and by the 20s background poller
/// (ensure_keda_poll, mirroring ensure_flux_poll).
KedaOverviewUpdated(KedaOverview),

/// Single-resource detail, emitted in response to SelectScaledObject/SelectScaledJob.
KedaResourceDetailsUpdated(KedaResourceDetails),

/// Emitted instead of KedaResourceDetailsUpdated when the named resource no longer exists
/// (e.g. deleted between list-render and click), satisfying spec edge case for "target not
/// found" and the generic "not found" inspector state pattern already used by Flux/Applications.
KedaResourceNotFound {
    kind: String,
    namespace: String,
    name: String,
},
```

## Related-resources extension (`src/backend/related.rs`, `RefreshRelatedResources` command — unchanged signature)

New `match kind` arms (kind strings are the same ones already used elsewhere in the protocol for
resource kinds):

- `"ScaledObject" | "ScaledJob"` → returns `scales` (target workload) and transitively `owns`
  (target workload's currently running Pods) `RelatedResourceRef`s.
- `"Pod"` (existing arm, extended) → additionally returns a `scaledBy` `RelatedResourceRef` when
  the pod's owning workload is targeted by a `ScaledObject`/`ScaledJob` in the same namespace.

No change to the `RelatedResourceRef` type itself (already generic: kind/namespace/name/relation
label).

## Frontend consumption contract

- `ui/src/store/index.ts`: new `keda: { overview?: KedaOverview }` slice (mirrors `flux`), actions
  `refreshKeda()` (sends `RefreshKeda`) and `selectKedaResource(kind, namespace, name)` (sends
  `SelectScaledObject`/`SelectScaledJob` based on `kind`).
- `ui/src/store/handlers/system.ts`: handle `KedaOverviewUpdated` (updates the slice),
  `KedaResourceDetailsUpdated` / `KedaResourceNotFound` (feed the Inspector's selected-resource
  state, mirroring the existing Flux/Applications "not found" inspector pattern).
- `ui/src/components/panels/KedaPanel.tsx`: list pane. When `!overview.installed`, renders the
  "KEDA not detected" honest-empty-state (FR-003). When `installed && scaled_objects.length === 0
  && scaled_jobs.length === 0`, renders the distinct "no autoscalers configured" empty state
  (FR-004).
- `ui/src/resources/keda.tsx`: Inspector rendering — triggers (type + key/value metadata table +
  optional `current_value`/`threshold` line), `reasoning.message`, and the related-resources
  section (reused generic renderer, unchanged).

## Test contracts

- `tests/protocol_serde.rs` (or wherever the existing round-trip tests for `cmd.rs`/`event.rs`
  live): add round-trip serialization tests for `RefreshKeda`, `SelectScaledObject`,
  `SelectScaledJob`, `KedaOverviewUpdated`, `KedaResourceDetailsUpdated`, `KedaResourceNotFound`,
  matching the existing test pattern (e.g. the `roundtrip(BackendCmdV1::SelectFluxResource {...})`
  test already in `cmd.rs`).
- Rust unit tests for `ScaleReasoning` derivation (`src/backend/keda.rs`): one test per rule in
  data-model.md's "Derivation rules" list (floor, ceiling, mid-range, paused, scaled-to-zero,
  target-not-found).
- `ui/tests/e2e/keda-autoscaler-pane.spec.ts`: see quickstart.md.
