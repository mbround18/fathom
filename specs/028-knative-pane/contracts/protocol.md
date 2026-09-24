# Contract: Protocol additions for the Knative pane

This feature adds to the existing versioned command/event protocol in `src/protocol/v1/`
(Constitution Principle II). All types below are `derive(Serialize, Deserialize, TS)` so
`ts-rs`'s existing build step regenerates the matching `ui/src/protocol/generated/*.ts` files —
no hand-written frontend types.

## Command: `RefreshKnative`

```rust
// src/protocol/v1/cmd.rs — new variant on BackendCmdV1
RefreshKnative,
```

- **Trigger**: sent once when `KnativePanel` mounts (mirrors `FluxPanel`'s `useEffect`), and
  available for a manual refresh affordance if the pane adds one later.
- **Effect**: re-runs the same discovery used by the background poll and emits
  `KnativeOverviewUpdated` immediately, without waiting out the rest of the 20s interval.
- **Mutating**: No.

## Event: `KnativeOverviewUpdated(KnativeOverview)`

```rust
// src/protocol/v1/event.rs — new variant on SystemEvent
KnativeOverviewUpdated(KnativeOverview),
```

- **Emitted by**: `poll_knative_overview` (20s background loop, started via
  `ensure_knative_poll`, mirroring `ensure_flux_poll`/`poll_flux_overview` in `src/backend/sync.rs`)
  and by the `RefreshKnative` command handler.
- **Consumed by**: `ui/src/store/handlers/system.ts`, writing into `store.knative.overview`.

## Round-trip test contract

Per constitution Principle III, add to `src/protocol/v1/cmd.rs`'s existing `#[cfg(test)] mod
tests` (alongside `select_flux_resource_roundtrip`):

```rust
#[test]
fn refresh_knative_roundtrip() {
    roundtrip(BackendCmdV1::RefreshKnative);
}
```

And an equivalent roundtrip test for `SystemEvent::KnativeOverviewUpdated(KnativeOverview::default())`
in `event.rs`'s existing test module, following the same pattern already used for
`FluxOverviewUpdated`.

## No REST/HTTP surface

This is an internal Tauri command/event contract (IPC), not a network API — there is no OpenAPI/
JSON-schema contract to publish externally, consistent with how the Flux integration is
documented.
