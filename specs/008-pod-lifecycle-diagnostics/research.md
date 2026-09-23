# Phase 0 Research: Pod Lifecycle Diagnostics

No `NEEDS CLARIFICATION` markers remained in spec.md (the checklist confirms this), so this phase
records the technical decisions needed to implement the spec's Assumptions, resolved by reading
the existing codebase rather than external research.

## Decision: Where probe/hook "failing" state is computed

**Decision**: Compute probe-failing and hook-failed booleans on the **frontend**, joining the
backend's new structural `ContainerLifecycle` data against the Pod's own Events, which the
frontend already has via `eventsFor()` (`ui/src/store/index.ts:1069`) — itself fed by the
existing `RefreshEvents` command / `EventsUpdated` event that `pods.tsx`'s `read()` already
requests (`ui/src/resources/pods.tsx:199`).

**Rationale**: The spec's Assumptions section is explicit: "The Events timeline reuses Fathom's
existing cluster-events plumbing... rather than introducing a second events-fetching path."
Kubernetes itself exposes no dedicated "current probe state" API field (confirmed against
`k8s_openapi::api::core::v1::ContainerStatus`, `ContainerState`, `Pod::Spec::containers[].{
livenessProbe,readinessProbe,startupProbe}` — no probe-failure field anywhere in the type). The
only two signals available are (a) `ContainerStatus.ready` (a boolean, already known cluster-side
per container) and (b) `Unhealthy`/`FailedPostStartHook`/`FailedPreStopHook` Events. Since the
Events are already fetched and filterable by involved-object in the frontend store, and the
backend does not otherwise fetch Events for Pod reads (`fetch_pod_details` in
`src/backend/pods.rs` has no Event API calls today), doing the join in the backend would mean a
second, duplicate Event fetch — contradicting the assumption and adding backend complexity for no
benefit, since the join is pure data (no side effects) and cheap at Pod scale (single digits of
containers, tens of events).

**Alternatives considered**:
- *Backend fetches Events itself and does the join server-side*: rejected — duplicates the
  frontend's existing Event fetch (a second events-fetching path), and the backend would need to
  either poll Events again on every Pod detail refresh or maintain a watch, adding complexity
  disproportionate to a read-only diagnostic join constitution V would flag.
- *No failing-state correlation at all, spec-only display*: rejected — this is exactly what FR-006
  and FR-011 require (probe/hook failure indication distinct from the aggregate `Ready` condition),
  and is the core value proposition of User Stories 2 and 4.

## Decision: `ContainerLifecycle` structural fields sourced entirely from the existing `Pod` fetch

**Decision**: All structural fields (container name, is_init, image, current `ContainerState`,
`restart_count`, `last_state`, configured probes, configured hooks, `ready`) come from the single
`Pod` object `fetch_pod_details` already fetches (`api.get(name)` in `src/backend/pods.rs:133`) —
specifically `pod.status.container_statuses` / `pod.status.init_container_statuses` (for state,
restart count, last state, ready) and `pod.spec.containers` / `pod.spec.init_containers` (for
probes and lifecycle hooks). No new API call is added.

**Rationale**: `k8s_openapi::api::core::v1::Pod` already carries everything FR-001 through
FR-012 need in one object; `kube-rs`'s `Api<Pod>::get` already returns the full object (confirmed:
`fetch_pod_details` currently discards `pod.status.container_statuses` and
`pod.spec.init_containers` entirely — they exist on the struct, just unused today). Adding a
second `Api<Pod>::get` or a `list`/`watch` would be redundant.

**Alternatives considered**: none seriously — the data is already present in memory during
`fetch_pod_details`; only new field extraction is needed, not new fetching.

## Decision: Probe type/target representation

**Decision**: `ProbeSpec.mechanism` is a string enum-like value (`"httpGet"`, `"tcpSocket"`,
`"exec"`, `"grpc"`) matching Kubernetes' own field names (not Fathom-invented names), with a single
`target: String` field holding a human-readable rendering (e.g. `GET /healthz:8080`,
`exec: ["cat","/tmp/ready"]`, `tcp:8080`, `grpc:9000/health`) built at assembly time in
`src/backend/pods.rs`, since FR-005 only requires showing "type... threshold counts, and period",
not a fully structured target the UI must re-render per mechanism.

**Rationale**: Mirrors the existing pattern in this codebase (`ContainerPort`, `PodOwner`) of
flattening Kubernetes' nested spec shapes into simple, UI-ready strings rather than mirroring
`k8s_openapi`'s deeply nested `Probe`/`ProbeHandler` enum in the protocol, keeping the frontend
free of Kubernetes API-shape knowledge (consistent with constitution II's "UI must not... read
`~/.fathom/` directly" spirit of keeping cluster-shape knowledge backend-side).

**Alternatives considered**: *Mirror `k8s_openapi::ProbeHandler` 1:1 as a tagged union in the
protocol* — rejected as unnecessary complexity for a read-only display feature; would also require
generating additional `ts-rs` bindings for little display benefit.

## Decision: Hook-failure and probe-failure event matching

**Decision**: Match `FailedPostStartHook`/`FailedPreStopHook` events to a specific container by
substring match of the container name in the event's `message` (Kubernetes always includes the
container name in these event messages, e.g. `Exec lifecycle hook ... in container "api" failed`)
combined with the event's `involved_object_name` already being the Pod (confirmed structurally
correct since `eventsFor` already filters by `involved_object_kind === 'Pod'` and
`involved_object_name === pod.name`). Match `Unhealthy` events similarly by container-name
substring plus a `Liveness probe failed` / `Readiness probe failed` / `Startup probe failed`
substring in the message to decide *which* probe type failed. Readiness-probe-failing also
falls back to `ContainerStatus.ready === false` on a `running` container even absent a matching
Event (satisfies FR-006's "and/or", and the edge case where Events have already been
garbage-collected per the spec's Assumptions).

**Rationale**: This is exactly the substring-matching pattern Kubernetes' own `kubectl describe
pod` output relies on (Events have no structured "which container" field), and is the best-effort
approach the spec's Assumptions section explicitly sanctions ("Probe-failure correlation is
best-effort, sourced from `ContainerStatus` plus Events... This feature treats those as
sufficient").

**Alternatives considered**: *Only use `ContainerStatus.ready`*: rejected — loses the
liveness/startup-probe-specific and hook-failure signals FR-006/FR-011 require, which are only
observable via Events.
