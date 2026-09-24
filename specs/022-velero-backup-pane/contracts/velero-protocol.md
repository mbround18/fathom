# Contract: Velero protocol surface (v1)

This is the backend↔frontend contract addition, following the same shape as the existing Flux
contract (`SystemEvent::FluxOverviewUpdated` + `BackendCmdV1::RefreshFlux`).

## Command: `RefreshVelero`

- **Variant**: `BackendCmdV1::RefreshVelero` (unit variant, no payload — matches `RefreshFlux`).
- **Direction**: Frontend → backend.
- **Effect**: Triggers an immediate `scan_velero` and emits a fresh `VeleroOverviewUpdated` event
  (or a `NotificationLevel::Error` event on failure), without waiting for the next 20s poll tick.
- **Mutation classification**: Non-mutating (read-only refresh) — classified in
  `src/protocol/v1/mutation.rs` alongside `RefreshFlux`, `RefreshHubble`, `RefreshCrdInstances`.
- **Section routing**: `section_keys::integrations_velero()`.
- **Preconditions**: None beyond an active cluster connection; if no client is connected, the
  handler emits a `notify_error` ("Not connected to a cluster.") exactly like `refresh_flux`.

## Event: `SystemEvent::VeleroOverviewUpdated(VeleroOverview)`

- **Direction**: Backend → frontend.
- **Emitted by**:
  1. The background poll task (`poll_velero_overview`, every 20s while a cluster is connected),
     started by `ensure_velero_poll` from the same call site as `ensure_flux_poll`
     (`src/backend/cmd/cluster.rs`).
  2. The `RefreshVelero` command handler, for an immediate on-demand refresh.
- **Route**: `Route::cluster(section_keys::integrations_velero(), context)`.
- **Payload**: `VeleroOverview` — see data-model.md. Always a full replacement snapshot; there is
  no incremental/delta variant, matching `FluxOverviewUpdated`.
- **Idempotency / staleness**: Carries the existing envelope `seq` used cluster-wide so a stale
  event from a superseded cluster context is discarded by the same mechanism already used for
  every other event (Constitution Principle V: "events from a superseded request MUST be
  discarded, never rendered").

## Error surface

- CRD-list failure (the top-level `Api<CustomResourceDefinition>::list` call) →
  `events::notify_error("Background Velero scan failed: {e}")` (poll path) or
  `events::notify_error("Failed to scan cluster for Velero resources: {e}")` (manual refresh
  path) — verbatim backend error text, per Constitution Principle I.
- Per-kind Forbidden/Unauthorized error while listing instances of an already-discovered CRD →
  does NOT raise a top-level error; folded into `VeleroOverview.permission_denied = true` so the
  UI can render a permission-denied state distinct from a transient scan failure (research.md R5).

## Frontend contract

- `ui/src/store/index.ts` gains `velero: { overview: VeleroOverview | null }` and
  `refreshVelero(): void`, mirroring `flux`/`refreshFlux`.
- `ui/src/store/handlers/system.ts` gains a case for `VeleroOverviewUpdated` that sets
  `velero.overview`, mirroring the existing `FluxOverviewUpdated` case.
- `ui/src/components/panels/VeleroPanel.tsx` renders `store.velero.overview` through
  `IntegrationSummary`, with a `useEffect` calling `refreshVelero()` on mount (mirrors
  `FluxPanel.tsx` exactly).
- `ui/src/components/shell/PaneHost.tsx` routes `pane.tool === 'velero'` to `<VeleroPanel />`.
- `src/sections/mod.rs` registers a `"Velero"` section (`section_keys::integrations_velero()`,
  `ScopeKind::Namespaced`) in the Integrations group, alongside Argo/Metrics/Grafana/Flux.

## Non-goals reaffirmed by this contract

- No `TriggerBackup`, `TriggerRestore`, `DeleteBackup`, or any other mutating command is added.
  `mutation.rs`'s classification function MUST route `RefreshVelero` to the non-mutating branch;
  a unit test asserting this (mirroring existing `RefreshFlux`/`RefreshHubble` coverage) is part
  of this feature's task list.
