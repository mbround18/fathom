# Research: Port Forward Interface (009)

All backend surface this feature needs already ships. No NEEDS CLARIFICATION items remain after
reading the backend/protocol source directly (no research agents were needed; findings below come
from reading `src/backend/portforward.rs`, `src/backend/cmd/port_forwards.rs`,
`src/protocol/v1/{cmd,event,mutation}.rs`, `src/backend/related.rs`, and the existing frontend
store/panel code).

## Decision: reuse the existing `network/portforwards` `ToolKind`, wire it in `PaneHost`

`ui/src/layout/defaults.ts` and `ui/src/components/shell/labels.ts` already declare a
`'portforwards'` `ToolKind` and a `Port Forwards` nav entry (`ui/src/resources/nav.ts`,
`network/portforwards`). `PaneHost.tsx`'s `PaneBody` switch has no case for it yet, so it falls
into the generic "Not ported yet" `Empty` state. Alternatives considered: adding a new tool kind —
rejected, the plumbing already exists end-to-end (nav entry, layout slot budget, pane title), only
`PaneHost`'s render branch is missing.

## Decision: `StreamEvent::PortForwardTunnel` and `store/handlers/stream.ts`'s `tunnels` map already exist

The frontend store already has `StoreData.tunnels: Record<string, TunnelState>` and
`handleStream`'s `'PortForwardTunnel'` case populates it. This feature's list component merges
`lists[…PortForwardsUpdated…]` (persisted `PortForward.status`) with `tunnels[id]` (live
`TunnelStatus`) for the row's real-time indicator, preferring the live tunnel state when present
(FR-003, SC-002, SC-006).

## Decision: the backend does not currently emit `PortForwardTunnel` from the Tauri command path

`src/backend/cmd/port_forwards.rs` only emits `ResourceListEvent::PortForwardsUpdated` after
start/stop/delete/update/refresh; `StreamEvent::PortForwardTunnel` is only consumed today by the
legacy egui `app/mailbox.rs` (dead code path per the constitution's "old UI removed once parity is
reached"). This is a pre-existing backend gap, not part of this feature's frontend-only scope
(explicit non-goal: "any change to the backend forwarding/persistence logic"). The panel still
subscribes to `PortForwardTunnel` (FR-003 requires it, and e2e tests inject it via the fake
backend to prove SC-002/SC-006), so a future backend change that starts emitting it needs no
frontend follow-up. In practice, `PortForwardsUpdated` alone already carries `Connecting`/`Active`/
`Error` on start and `Closed` on stop, which covers every acceptance scenario in this spec.

## Decision: local port availability check reuses `CheckPortAvailability`/`SystemEvent::PortAvailability`

Already wired end-to-end: `ui/src/store` has `integrations.portAvailability: Record<number,
boolean>`; `store/handlers/system.ts` needs a case for `PortAvailability` (confirmed absent —
added by this feature, see data-model.md). The port picker debounces edits and dispatches
`CheckPortAvailability`, reading the result from that map keyed by port number.

## Decision: Service → backing pod resolution reuses `RefreshRelatedResources`

`src/backend/related.rs::related_for_service` already resolves a Service's selector to its
backing Pods (relation `"selects"`) via `list_pods_by_selector`, returned as
`RelatedResourceRef { kind: "Pod", namespace, name, status_hint (= pod phase) }`. No frontend
descriptor currently sends `RefreshRelatedResources` for `Service` (only `Pod`, `Node`,
`HelmRelease` do, per `ui/src/resources/{pods,nodes,helm}.tsx`). The port-forward-from-Service flow
sends this command itself when opened, filters the reply to `kind === 'Pod'`, prefers a pod whose
`status_hint === 'Running'`, and shows an explicit error if none exist or none are `Running`
(edge case: "Service's backing pods are not yet ready or none currently exist"). This matches
spec.md's Assumption ("transparent single-pod resolution, surfaced in the confirmation step").

## Decision: no protocol/backend changes

FR-013 requires using only the existing protocol surface. Nothing in this feature needs a new
command, event, or field. `PortForward` has no "originating service" field — FR-003's "originating
service, when applicable" is satisfied by an ephemeral, frontend-only association (matched by
namespace/pod/local_port/remote_port right after a Service-launched `StartPortForward` succeeds,
via the next `PortForwardsUpdated`), not a persisted backend concept; it is lost on app restart or
in a second UI instance, which is acceptable since it is decorative context, not the tunnel's
identity.

## Decision: mutation confirmation reuses `ConfirmDialog`

Every mutating command (`StartPortForward`, `StopPortForward`) goes through the existing
`ConfirmDialog` component (`ui/src/components/ConfirmDialog.tsx`), the same component
006-helm-mutate's uninstall/rollback flows use, not the generic bulk `ActionDescriptor`/
`ActionMenu` machinery (that framework is for list-row bulk actions keyed by `namespace/name`;
port-forward's mutations are single-target and inspector- or panel-launched). This satisfies
constitution Principle I directly without inventing a second confirmation pattern.

## Decision: test isolation

New e2e specs: `ui/tests/e2e/us1-port-forward-start.spec.ts`,
`us2-port-forward-manage.spec.ts`, `us3-port-forward-port-picker.spec.ts`,
`us4-port-forward-copy-address.spec.ts` (mirrors 006-helm-mutate's `us1-`/`us2-`/`us3-` naming).
Every new `data-testid` is prefixed `port-forward-` (e.g. `port-forward-panel`,
`port-forward-list`, `port-forward-row`, `port-forward-start`, `port-forward-stop`,
`port-forward-restart`, `port-forward-copy-address`, `port-forward-local-port`).
