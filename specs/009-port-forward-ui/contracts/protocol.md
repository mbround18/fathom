# Protocol Contract: Port Forward Interface (009)

No protocol changes. This feature consumes only existing, already-tested commands/events. Listed
here for traceability against FR-013.

## Commands consumed (`BackendCmdV1`, `src/protocol/v1/cmd.rs`)

| Command | Sent when |
|---|---|
| `StartPortForward { namespace, pod, local_port, remote_port, container }` | Operator confirms the forward-start dialog (US1) or a restart (US2 Scenario 3) |
| `StopPortForward { tunnel_id }` | Operator confirms "Stop" on an active/connecting row (US2) |
| `DeletePortForward { tunnel_id }` | Not used by this feature's UI (no row action maps to it; `Stop` covers the spec's needs — `Delete` is available for a future "remove from list" action, out of this feature's scope) |
| `UpdatePortForward { tunnel_id, local_port, remote_port, auto_start }` | Only if the panel exposes an `auto_start` toggle (FR-012); implemented as a simple checkbox per row that re-sends the row's current `local_port`/`remote_port` unchanged |
| `RefreshPortForwards` | Panel mount (mirrors every other tool panel's `useEffect` refresh-on-mount pattern) |
| `CheckPortAvailability { port }` | Port picker: on mount (suggested port) and debounced on edit |
| `RefreshServices` / `SelectService` | New Service descriptor (prerequisite, see data-model.md) |
| `RefreshRelatedResources { section, kind: "Service", namespace, name }` | Service inspector's own "Related" tab (existing pattern) and the Forward-from-Service flow (reads the same cached reply) |

## Events consumed

| Event | Effect |
|---|---|
| `ResourceListEvent::PortForwardsUpdated(Vec<PortForward>)` | `lists[network/portforwards]` (generic handler, already works) |
| `StreamEvent::PortForwardTunnel { tunnel_id, status, local_endpoint }` | `tunnels[tunnel_id]` (generic handler, already works) |
| `SystemEvent::PortAvailability { port, available }` | `integrations.portAvailability[port]` (generic handler, already works) |
| `ResourceReadEvent::RelatedResourcesUpdated { ..., related }` | Service → backing-pod resolution |
| `ResourceReadEvent::ServiceDetailsUpdated` / `ResourceListEvent::ServicesUpdated` | New Service descriptor |

## UI contract additions (not protocol — internal component contracts)

- `PortForwardsPanel` (`ui/src/components/panels/PortForwardsPanel.tsx`): tool-shaped content
  rendered inside a `list`-kind pane routed to `SECTION.portForwards` (mirrors
  `CustomResourcesSection`'s precedent of a bespoke body inside a list pane rather than the
  closed descriptor registry).
- `PortForwardStartDialog` (`ui/src/components/panels/PortForwardStartDialog.tsx`): shared by the
  inspector "Forward" tab (US1) and the list panel's "Restart" action (US2 Scenario 3). Props:
  `target: ForwardTarget`, `open`, `onOpenChange`.
- `PortForwardActionsPanel` (`ui/src/components/panels/PortForwardActionsPanel.tsx`): the Pod/
  Service Inspector "Forward" tab body — lists declared ports (+ manual port entry) and opens
  `PortForwardStartDialog` per selection.

## Test-isolation contract (explicit user instruction)

- Every new `data-testid` is prefixed `port-forward-`.
- New e2e specs: `ui/tests/e2e/us1-port-forward-start.spec.ts`,
  `ui/tests/e2e/us2-port-forward-manage.spec.ts`,
  `ui/tests/e2e/us3-port-forward-port-picker.spec.ts`,
  `ui/tests/e2e/us4-port-forward-copy-address.spec.ts`.
