# Contract: Fleet Rollup Dashboard Protocol (`src/protocol/v1/fleet.rs` + touch points)

This is the versioned command/event contract (Constitution II) the UI uses to drive the Fleet
Dashboard. All types are defined once in Rust and exported to TypeScript via `ts-rs`, per
existing convention (see `src/protocol/v1/flux.rs`, `context_status`-related types in
`event.rs` for the established pattern this mirrors).

## Commands (`BackendCmd`, `src/protocol/v1/cmd.rs`)

### `StartFleetPolling`

```json
{ "type": "StartFleetPolling" }
```

- Sent when the Fleet Dashboard view mounts.
- Backend spawns a poller for every context currently `Connected` (per
  `session.contextStatuses` — the backend's own equivalent state, not the frontend's copy: the
  backend derives "connected" the same way `context_status.rs` already tracks it) that does not
  already have a running poller.
- Idempotent: re-sending while pollers are already running for the same set of contexts is a
  no-op for those contexts.
- Immediately emits one `FleetSummaryUpdated{health: Pending, ...}` per newly-started context.

### `StopFleetPolling`

```json
{ "type": "StopFleetPolling" }
```

- Sent when the Fleet Dashboard view unmounts.
- Aborts every running fleet poller and clears `FleetState.summaries`.
- Also invoked implicitly by `cleanup_all_tasks()` on app shutdown/disconnect — the dashboard
  never leaks a poller past the app's own lifetime even if the frontend fails to send this.

*(No per-context start/stop command is exposed — FR-006 requires the dashboard's card list to
follow connects/disconnects automatically; the backend derives which contexts to poll from the
existing connected-context set on every `StartFleetPolling` and via the same reactive triggers
`context_status::spawn_status_probes` already listens to — `Initialize`, `RefreshContexts`,
`SetContext` — a context connecting after the dashboard is already open starts its own poller
without a new explicit command, and disconnecting stops it the same way.)*

## Events (`BackendEvent::System(SystemEvent::...)`, `src/protocol/v1/event.rs`)

### `FleetSummaryUpdated`

```json
{
  "type": "FleetSummaryUpdated",
  "data": {
    "context": "prod-east",
    "health": "Degraded",
    "error": null,
    "nodeReadyCount": 5,
    "nodeTotalCount": 6,
    "unhealthyPodCount": 2,
    "clusterVersion": "v1.31.2",
    "flux": "OutOfSync",
    "lastUpdated": "2026-09-23T18:04:12Z"
  }
}
```

- Emitted once per poll attempt (success or failure), per context, while that context's poller
  is running.
- Routed globally (`Route::global(section_keys::system())`) — the frontend keys its own
  `fleetSummaries` map by `data.context` on receipt (matches `ContextStatusUpdated`'s existing
  routing/keying pattern; no new `Route` variant needed).
- `error` populated only when `health` is `Unreachable` or `AuthError` (contract invariant,
  unit-tested in `fleet.rs`, mirroring `context_status.rs`'s existing test for the analogous
  invariant on `ContextStatusUpdated`).

## Interaction with existing context-switch contract

- Clicking a Fleet Dashboard card sends the **existing** `BackendCmd::SetContext { context }`
  (spec 012) — no new command. This is the only mutation-adjacent action the dashboard performs,
  and it is not a mutation of cluster state, only of which context is "active" in Fathom's UI.
- The Fleet Dashboard does not read or write `ClusterContext.client`/`current_context` at all;
  it is entirely independent of "the" active context except for this one existing command.

## Backward compatibility

- Purely additive: two new `BackendCmd` variants, one new `SystemEvent` variant, one new
  protocol module (`fleet.rs`). No existing variant's shape changes. Per Constitution II, this
  does not require a protocol major-version bump (additive, non-breaking).
