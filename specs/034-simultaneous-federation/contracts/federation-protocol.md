# Contract: Federation protocol (additive, protocol v1)

Types are defined in Rust (`src/protocol/v1/federation.rs`) and exported by ts-rs.

## Commands (`BackendCmd`)
- `FederateContext { context: String }`: validates the context exists in kubeconfig, persists, spawns the live tier. Unknown context yields a verbatim error notification.
- `UnfederateContext { context: String }`: aborts tasks, drops the client, persists.
- `RequestFederationSnapshot`: re-emits current state and combined rollup (UI mount).

## Events (`SystemEvent`, `Route::global(system)`)
- `FederationStateUpdated { federated: Vec<String>, soft_cap: u32, over_cap: bool }`
- `FederationRollupUpdated { context: FederationContextRollup, combined: FederationCombinedRollup }` (debounced 500ms per context; emitted on change only)
- `FleetSummaryUpdated` (027, existing) is also emitted for federated contexts so 027 cards render; additive optional `live` marker.

## Guarantees
- Events carry `generation`; consumers drop older ones.
- No secret material in any payload.
- Serde round-trip test for every variant; generated bindings drift-checked in CI.
- Empty federated set: no events except an initial `FederationStateUpdated`.
