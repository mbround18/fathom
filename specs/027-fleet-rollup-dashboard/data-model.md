# Phase 1 Data Model: Fleet Rollup Dashboard

## `FleetHealthState` (new, `src/protocol/v1/fleet.rs`)

Fieldless enum, serialized as a bare string (matches `ContextConnectionStatus`/
`NotificationLevel`'s convention):

| Value | Meaning |
|---|---|
| `Healthy` | Reachable, 0 unhealthy pods, node/version fetched successfully. |
| `Degraded` | Reachable, but at least one unhealthy pod (or a node NotReady). |
| `Unreachable` | Poll timed out or the connection failed (network/API-server error). |
| `AuthError` | Poll failed specifically due to credential/auth rejection (distinguished from a
generic `Unreachable` per FR-005 and spec 012's existing verbatim-error precedent). |
| `Pending` | No successful poll yet (first cycle since the poller started for this context). |

## `FluxRollupStatus` (new, `src/protocol/v1/fleet.rs`)

Fieldless enum:

| Value | Meaning |
|---|---|
| `Synced` | Flux installed; every Kustomization/HelmRelease `Ready`. |
| `OutOfSync` | Flux installed; at least one Kustomization/HelmRelease not `Ready` (Failed or stuck Reconciling past its interval). |
| `NotInstalled` | No Flux CRDs detected on this cluster — displayed as "not applicable," never as an error (FR-003). |

Derived by reducing the existing `FluxOverview` (`src/protocol/v1/flux.rs`) — `installed:
false` → `NotInstalled`; `installed: true` and every resource's `ready == Ready` → `Synced`;
otherwise → `OutOfSync`.

## `FleetContextSummary` (new, `src/protocol/v1/fleet.rs`)

```rust
pub struct FleetContextSummary {
    pub context: String,
    pub health: FleetHealthState,
    pub error: Option<String>,          // verbatim, only set when health is Unreachable/AuthError
    pub node_ready_count: Option<u32>,   // None while Pending or on failure
    pub node_total_count: Option<u32>,
    pub unhealthy_pod_count: Option<u32>,
    pub cluster_version: Option<String>,
    pub flux: Option<FluxRollupStatus>,  // None == not yet known; distinct from NotInstalled
    pub last_updated: String,            // ISO8601, set on every successful OR failed poll attempt
}
```

Validation rules (enforced by construction in `src/backend/fleet.rs`, mirroring
`context_status.rs`'s `error`-only-on-`Error` invariant):
- `error` MUST be `None` unless `health` is `Unreachable` or `AuthError`.
- `node_ready_count`/`node_total_count`/`unhealthy_pod_count`/`cluster_version` MUST all be
  `None` together when `health` is `Pending`, `Unreachable`, or `AuthError` (a failed poll
  does not report stale numbers — Edge Cases: "loading/pending state without... rendering
  stale data").
- `last_updated` is set every poll attempt regardless of success, so the frontend can show
  "last checked" even during sustained failure.

## `SystemEvent::FleetSummaryUpdated` (new variant, `src/protocol/v1/event.rs`)

```rust
FleetSummaryUpdated(FleetContextSummary),
```

- Routed `Route::global(section_keys::system())`, matching `ContextStatusUpdated`'s pattern —
  the frontend keys its own map by `.context` on receipt, so no per-context Route scoping is
  needed (mirrors how `ContextStatusUpdated` already works for the same reason: many small,
  independent per-context updates on one shared channel).
- Emitted once per poll attempt (success or failure) by each context's poller loop in
  `src/backend/fleet.rs`.

## `BackendCmd::StartFleetPolling` / `StopFleetPolling` (new variants, `src/protocol/v1/cmd.rs`)

```rust
StartFleetPolling,
StopFleetPolling,
```

- `StartFleetPolling`: spawns one poller per context currently `Connected` in
  `contextStatuses` (backend-side equivalent: every context for which `FleetState.pollers`
  doesn't already have a running task), and emits an immediate `FleetSummaryUpdated{health:
  Pending}` for each so the UI has something to render before the first real poll settles.
  Idempotent — calling it while already started is a no-op per context.
- `StopFleetPolling`: aborts and clears every handle in `FleetState.pollers` (reuses the
  existing `abort_task` helper in `state.rs`), and is also called from
  `cleanup_all_tasks()`/app shutdown so a closed dashboard (or a closed app) never leaves a
  poller running — this is the concrete mechanism behind research.md R2's "only alive while
  the view is open."

## `FleetState` (new, `src/backend/state.rs`, `pub(super)` — internal only, no protocol exposure)

```rust
pub(super) struct FleetState {
    pub(super) pollers: HashMap<String, tokio::task::JoinHandle<()>>,
    pub(super) summaries: HashMap<String, FleetContextSummary>,
}
```

- `pollers` keyed by context name, one `JoinHandle` per running poll loop — same keyed-map
  lifecycle idiom as `NodeActionState.drain_tasks` (`cancel()`/`abort_all()` equivalents:
  `stop_context()`/`stop_all()`).
- `summaries` is a read cache only, updated by each poller after every attempt, consulted so a
  freshly (re)connected dashboard view or a late-joining poller doesn't need to wait a full
  interval before there's *something* to show (though `StartFleetPolling` also emits a
  `Pending` placeholder immediately, per above, for the true first-open case).
- `abort_all()` on `FleetState` is added to `cleanup_all_tasks()` in `state.rs`, alongside the
  existing `watch_tasks.abort_all()` etc.

## Frontend: `fleetSummaries` (new field, `ui/src/store/index.ts`)

```ts
type FleetSummaries = Record<string, FleetContextSummary>  // keyed by context name
```

- Updated only by the new `FleetSummaryUpdated` handler case (`ui/src/store/handlers/fleet.ts`):
  last write per context wins (same convention as `contextStatuses`).
- Not persisted — runtime-only, matching every other session-scoped map in this store.
- `startFleetPolling()`/`stopFleetPolling()` store actions send `StartFleetPolling`/
  `StopFleetPolling`; called from `FleetDashboardView`'s mount/unmount effect (FR-004/R2).
- A context disconnecting (its entry disappears from `session.contexts`, or its
  `contextStatuses[name].status` moves away from `Connected`) removes its entry from
  `fleetSummaries` client-side immediately (FR-006's "removed... automatically") rather than
  waiting for a backend round trip — the backend independently stops that context's poller on
  the same disconnect signal.

## Key Entities cross-reference (spec.md)

- **Context Summary** (spec.md) == `FleetContextSummary` above.
- **Fleet Dashboard View** (spec.md) == `FleetDashboardView.tsx`, composed from
  `fleetSummaries` filtered/ordered by `session.contexts`.

## Clarification update (2026-09-24)

- `FleetContextSummary` gains `restricted: Vec<String>` (TS `string[]`): resource groups (`"nodes"`, `"pods"`) the credentials could not list (HTTP 403). The matching count is `None` and the card shows "no access". Always empty for `Pending`/`Unreachable`/`AuthError`. For `Healthy`/`Degraded`, a count is `None` only when its group is in `restricted`, so the earlier "counts all-null or all-set" invariant applies to `Pending`/`Unreachable`/`AuthError` only.
- `Healthy`/`Degraded` are derived from the readable data only (all groups denied plus a good version call => `Healthy`). `AuthError` is reserved for 401/403 on client construction or the version call.
- "Unhealthy pod" = phase `Failed`/`Unknown`; `Running` with an unready container; any container or init container waiting with `ImagePullBackOff`/`ErrImagePull`/`CrashLoopBackOff`/`CreateContainerError`; or `Pending` for more than 5 minutes.
- `FleetState` also holds a shared semaphore (8 permits).
