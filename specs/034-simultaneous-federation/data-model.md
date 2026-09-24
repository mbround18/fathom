# Data Model: Simultaneous Federation

## FederationConnectionState (enum, bare-string serialized)
`Pending | Live | Reconnecting | Unreachable | AuthError`

## FederationContextRollup (new, `src/protocol/v1/federation.rs`)
| Field | Type | Notes |
|---|---|---|
| context | String | key |
| state | FederationConnectionState | |
| generation | u64 | monotonic per federate; stale events dropped |
| error | Option<String> | verbatim; only Unreachable/AuthError |
| nodes_ready / nodes_total | Option<u32> | None = no access or not live |
| pods_running / pods_pending / pods_failed / pods_unhealthy | Option<u32> | unhealthy per 027 classifier |
| deployments_unavailable | Option<u32> | |
| warning_events_1h | Option<u32> | |
| cluster_version | Option<String> | reused from 027 |
| restricted | Vec<String> | metrics with no RBAC access (027 convention) |
| last_updated | String | ISO8601 |

Invariant: numeric fields are None when state is Pending, Unreachable or AuthError. Reconnecting may carry last-known values but the UI marks them stale, never live.

## FederationCombinedRollup
`contributing_contexts: u32`, `total_contexts: u32`, plus summed nodes, pods, unhealthy, deployments_unavailable and warnings over Live rows only.

## FederatedSet (persisted `~/.fathom/federation.json`)
`{ "version": 1, "contexts": ["dev","staging"], "softCap": 8 }`. Names only; no credentials.

## Runtime (backend `FederationState`)
`HashMap<String, FederationHandle { generation, client, task_handles, counters }>` plus a `ContextTier` arbiter shared with `FleetState`.

## State transitions
Pending -> Live -> Reconnecting -> Live | Unreachable | AuthError; Unreachable -> Reconnecting (backoff) -> Live; any -> removed on Unfederate.
