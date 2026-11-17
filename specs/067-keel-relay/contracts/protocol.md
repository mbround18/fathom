# Protocol contract: zones, relays, paths and queries

All changes are additive to `crates/keel-protocol` (Principle II); no protocol version bump.
ts-rs regenerates `apps/keel/web/src/protocol/generated`. Every new command and event gets a
serde round-trip test (Principle III).

## Types (`host.rs`, new `relay.rs`)

```rust
pub struct ZoneInfo { id: Uuid, workspace: Uuid, name: ZoneName, description: String,
                      hosts: u32, relays: u32 }

pub enum RelayState { Connecting, Connected, Unreachable, Revoked }

pub struct RelayInfo {
    id: Uuid, workspace: Uuid, zone: Uuid, name: ZoneName /* same rule */,
    dial_address: String, host_key_fingerprint: String, host: Option<Uuid>,
    state: RelayState, since: Option<String>, rtt_ms: Option<u32>, version: Option<String>,
    streams: Option<u32>, last_error: Option<String>, revoked_at: Option<String>,
}

pub enum RunnerPath { Direct, Relay { relay: Uuid } }      // advisory peer is not exposed

pub enum HostReach {                                        // derived, data-model.md table
    Connected { path: RunnerPath }, Disconnected, RelayUnreachable { relays: Vec<Uuid> },
    RelayRevoked { relay: Uuid }, NoPath,
}

pub struct RelayLinkKey { fingerprint: String, public_line: String, state: RelayLinkKeyState }

pub enum QueryKind { Facts, AppliedRevision, UnitState, DiskHealth, PathFree }
pub struct HostQuery { id: Uuid, host: Uuid, kind: QueryKind, params: serde_json::Value,
                       asked_by: String, asked_at: String,
                       outcome: Option<QueryOutcome>, answer: Option<serde_json::Value>,
                       refusal: Option<String>, answered_at: Option<String> }
```

`HostInfo` gains `zone: Option<Uuid>` and `reach: HostReach` (both `#[serde(default)]`).

## Commands (`cmd.rs`, new section "Zones and relays")

| Command | Reply | Permission | Notes |
|---|---|---|---|
| `ListZones { workspace }` | `Zones` | View | |
| `PreviewZoneCreate { workspace, name, description }` | `Preview` → `Zones` | workspace admin | |
| `PreviewZoneUpdate { zone, name, description }` | `Preview` → `Zones` | workspace admin | |
| `PreviewZoneRemove { zone }` | `Preview` → `Zones` | workspace admin | Preview lists blocking hosts and relays. |
| `PreviewHostZone { host, zone: Option<Uuid> }` | `Preview` → `Hosts` | workspace admin | |
| `ListRelays { workspace }` | `Relays` | View | |
| `PreviewRelayRegister { workspace, zone, name, dial_address, host_key, host: Option<Uuid> }` | `Preview` → `Relays` | workspace admin, step-up | Preview shows the fingerprint, and Keel's relay-link key line to pin. |
| `PreviewRelayUpdate { relay, zone, name, dial_address, host_key, host }` | `Preview` → `Relays` | workspace admin, step-up | Changing `host_key` restarts the link. |
| `PreviewRelayRevoke { relay }` | `Preview` → `Relays` | workspace admin | Preview lists hosts currently connected through it. |
| `GetRelayLinkKeys` | `RelayLinkKeys` | instance admin | Public lines only. |
| `PreviewRelayLinkKeyRotate` / `PreviewRelayLinkKeyActivate { fingerprint }` / `PreviewRelayLinkKeyRetire { fingerprint }` | `Preview` → `RelayLinkKeys` | instance admin, step-up | Mirrors `PreviewSshCaRotate`. |
| `AskHost { host, kind, params }` | `HostQuery` (pending), then event | workspace operator | Refused early when the host is offline or lacks `query-v1`. |
| `ListHostQueries { host, limit }` | `HostQueries` | View | |

All `Preview*` go through the existing preview/confirm flow and write audit in the same
transaction as the change (Principle VI).

## Events (`event.rs`)

| Event | When |
|---|---|
| `RelayChanged(RelayInfo)` | Live state change, registration, edit, revoke. Not on every `Pong`: RTT and stream count are pushed at most every 30 s. |
| `HostReachChanged { host, reach }` | A host's derived reach changed. |
| `HostQueryAnswered(HostQuery)` | Answer, refusal or timeout (default 15 s). |
