# Data Model: Port Forward Interface (009)

All wire types already exist (generated from Rust, `ui/src/protocol/generated/`). This feature
adds frontend-only state; no protocol change.

## Existing wire types (read, not modified)

- `PortForward` (`ui/src/protocol/generated/PortForward.ts`): `id, namespace, pod, local_port,
  remote_port, container: string | null, status: PortForwardStatus, auto_start: boolean`.
- `PortForwardStatus` ("persisted" status): `"Connecting" | "Active" | "Error" | "Closed"`.
- `TunnelStatus` (live status carried by `StreamEvent::PortForwardTunnel`): same four values,
  separate TS type by design (see `src/protocol/v1/event.rs` doc comment).
- `ResourceListEvent::PortForwardsUpdated(Vec<PortForward>)` — already ingested generically by
  `store/handlers/list.ts` (`rowId` already special-cases `'id' in row`).
- `StreamEvent::PortForwardTunnel { tunnel_id, status, local_endpoint }` — already ingested by
  `store/handlers/stream.ts` into `StoreData.tunnels: Record<string, TunnelState>`.
- `SystemEvent::PortAvailability { port, available }` — already ingested into
  `StoreData.integrations.portAvailability: Record<number, boolean>`.
- `RelatedResourceRef { section, kind: "Pod", namespace, name, relation: "selects", status_hint:
  Option<pod phase> }` — Service → backing-pod resolution (`related_for_service`).

## New frontend-only state

### `StoreData.portForwardOrigins: Record<string /* tunnel id */, { service?: string }>`

Ephemeral, not persisted. Set when a `StartPortForward` launched from a Service inspector
succeeds: the launch flow remembers `{ namespace, pod, localPort, remotePort, service }` for the
in-flight request; the next `PortForwardsUpdated` that contains a `PortForward` matching those
four fields and not already in `portForwardOrigins` gets tagged with `service`. Purely decorative
(FR-003's "originating service, when applicable"); lost on reload or in a second UI instance,
which is acceptable since the tunnel's identity (namespace/pod/port) never depends on it.

### Port-forward start flow (component-local state, not global store)

Modeled as a small state machine inside the new `PortForwardStartDialog` component:

```
idle → confirming { target, suggestedPort, portAvailable? } → submitting → (closed on success | error shown, stays open)
```

- `target`: `{ namespace, pod, container?: string, remotePort: number, service?: string }`.
- `suggestedPort`: defaults to `remotePort` if `portAvailability[remotePort] !== false`, else the
  first free port the operator can reach by editing the field (no backend "next free port" query
  exists beyond one-port-at-a-time `CheckPortAvailability`, so the UI just checks the remote port
  first and lets the operator pick another if it's taken — matches the assumption in spec.md
  literally: "the target/remote port when free, otherwise the next available port" is approximated
  by re-checking the operator's next choice, since there is no bulk/range check command).
- Port edits are debounced (300ms) before dispatching `CheckPortAvailability`.

### Restart flow

FR-005: `Restart` on an `Error`/`Closed` row re-runs the same confirm+port-check flow seeded from
the existing row's `namespace/pod/container/local_port/remote_port` (reusing `local_port` as the
suggested port instead of `remote_port`, since that's the port the operator chose last time).

## Key Entities (spec.md, restated as implementation shapes)

- **Port Forward** — rendered as a `PortForwardRow = PortForward & { tunnel?: TunnelState;
  originService?: string }`, computed by merging `lists[…PortForwardsUpdated]` with `tunnels` and
  `portForwardOrigins` in the panel component (not stored merged, to keep `ingest()` as the only
  writer of `lists`/`tunnels` per the store's existing rule).
- **Tunnel Status** — `tunnels[pf.id]?.status ?? pf.status` (falls back to the persisted status
  until a live event arrives, satisfying FR-010/SC-006: never shows `Active` before either source
  confirms it).
- **Forward Target** — the `{ namespace, pod, container?, remotePort, service? }` shape passed from
  the inspector action into `PortForwardStartDialog`; not persisted anywhere.

## New descriptor: `ui/src/resources/services.tsx` (prerequisite)

The frontend has no Service list/inspector at all yet (`ALL_DESCRIPTORS` in
`ui/src/resources/index.ts` only has Pod/Deployment/Node/Application/HelmRelease). FR-001/Scenario
2 requires a Service inspector to host "Forward" — this feature adds a minimal, read-only
`ResourceDescriptor<ServiceSummary, ServiceDetails>` (columns: name, type, cluster IP, port count;
detail sections: metadata, ports, selector, related — mirrors `nodes.tsx`'s "US2 prerequisite"
precedent exactly), registered in `ALL_DESCRIPTORS`. No new backend work: `RefreshServices` and
`SelectService` already exist in the protocol (`src/protocol/v1/cmd.rs`) and were simply unused by
the frontend. `SECTION.services` (`network/services`) and `SECTION.portForwards`
(`network/portforwards`) are added to `ui/src/protocol/commands.ts`'s `SECTION` map, plus their
`REFRESH` entries.
