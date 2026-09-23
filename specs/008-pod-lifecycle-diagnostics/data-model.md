# Phase 1 Data Model: Pod Lifecycle Diagnostics

All new types live in `src/protocol/v1/pods.rs` (Rust, `#[derive(Serialize, Deserialize, TS)]`,
mirrored to `ui/src/protocol/generated/*.ts` by `cargo test --test export_bindings`). `PodDetails`
gains exactly one new field; every existing field is unchanged.

## `ContainerLifecycle`

One entry per container in `spec.containers` and `spec.initContainers` (app and init containers in
the same `Vec`, distinguished by `is_init`; the UI splits them into separate sections per FR-001).

| Field | Type | Source | Notes |
|---|---|---|---|
| `name` | `String` | `container.name` | |
| `is_init` | `bool` | which spec list it came from | FR-001 |
| `image` | `String` | `container.image` | |
| `ready` | `bool` | `ContainerStatus.ready` | app containers only meaningfully `true`; init containers report `false` until terminated-success, matching kubelet semantics |
| `state` | `ContainerState` | `ContainerStatus.state` | current state (FR-002) |
| `restart_count` | `i32` | `ContainerStatus.restart_count` | FR-008 |
| `last_state` | `Option<ContainerState>` | `ContainerStatus.last_state` (`None` when the variant is `ContainerStateTerminated`'s absence, i.e. `Waiting`/`Running` with no prior termination) | FR-008, FR-009; edge case: `restartPolicy: Never` pods may have `last_state` empty even though `state` shows terminated — `state` (not `last_state`) always reflects current/last observed status |
| `probes` | `Vec<ProbeSpec>` | `container.liveness_probe`/`readiness_probe`/`startup_probe` | 0..3 entries; empty = FR-007's "no probes configured" |
| `hooks` | `Vec<LifecycleHookSpec>` | `container.lifecycle.post_start`/`pre_stop` | 0..2 entries; empty = FR-012's "none configured" |

## `ContainerState`

| Field | Type | Notes |
|---|---|---|
| `phase` | `String` | one of `"waiting"`, `"running"`, `"terminated"` |
| `reason` | `Option<String>` | waiting/terminated reason (e.g. `CrashLoopBackOff`, `Completed`, `Error`) |
| `exit_code` | `Option<i32>` | terminated only |
| `started_at` | `Option<String>` | RFC3339, running/terminated |
| `finished_at` | `Option<String>` | RFC3339, terminated only |

## `ProbeSpec`

| Field | Type | Notes |
|---|---|---|
| `probe_type` | `String` | `"liveness"` \| `"readiness"` \| `"startup"` |
| `mechanism` | `String` | `"httpGet"` \| `"tcpSocket"` \| `"exec"` \| `"grpc"` |
| `target` | `String` | human-readable, e.g. `GET /healthz:8080`, `tcp:8080`, `exec: cat /tmp/ready`, `grpc:9000/svc` — shown verbatim per spec's edge case on long/query-string paths |
| `period_seconds` | `i32` | |
| `failure_threshold` | `i32` | |
| `success_threshold` | `i32` | |

Current pass/fail state is **not** stored here — it is derived in the frontend
(`ui/src/resources/podLifecycle.ts`) by joining `ProbeSpec` + `ContainerLifecycle.ready` +
Pod Events, per research.md's decision. This keeps `ContainerLifecycle` a pure snapshot of spec +
status (backend's job, constitution II) while correlation against Events (frontend's existing
data) stays in the frontend (constitution II: no duplicate Event fetch in the backend).

## `LifecycleHookSpec`

| Field | Type | Notes |
|---|---|---|
| `hook_type` | `String` | `"postStart"` \| `"preStop"` |
| `mechanism` | `String` | `"exec"` \| `"httpGet"` \| `"sleep"` |
| `target` | `String` | human-readable, e.g. `exec: /bin/sh -c sleep 5`, `GET /shutdown:8080`, `sleep: 15s` |

Failure state is likewise derived in the frontend from Pod Events (`FailedPostStartHook` /
`FailedPreStopHook`), not stored here.

## `PodDetails` (extended, additive)

```rust
pub struct PodDetails {
    // ...existing fields unchanged...
    pub raw_json: String,

    /// NEW (008): app + init container lifecycle detail. Empty containers list is impossible
    /// (every Pod has >=1 container); an empty `is_init=true` subset is the FR-004 "no init
    /// containers" case, handled in the UI by filtering this Vec.
    pub containers_lifecycle: Vec<ContainerLifecycle>,
}
```

Field is appended at the end of the struct (Rust struct field order doesn't affect serde's
struct-as-object JSON encoding, but appending keeps the diff minimal and obviously additive).

## Frontend-only: probe/hook failing state (not a protocol type)

`ui/src/resources/podLifecycle.ts` exports pure functions over `ContainerLifecycle[]` +
`KubernetesEvent[]` (already-typed protocol types) that return per-container:

- `probeState(container, probes, events): { probe: ProbeSpec; failing: boolean }[]`
- `hookState(container, hooks, events): { hook: LifecycleHookSpec; failed: boolean }[]`

These are plain TypeScript (no new protocol types), unit-tested directly (no store/React needed).

## Pod Event (existing, reused — FR-013 through FR-016, FR-019)

No new type. `KubernetesEvent` (`src/protocol/v1/events.rs`, already bound to
`ui/src/protocol/generated/KubernetesEvent.ts`) is reused as-is via the store's existing
`eventsFor(s, 'Pod', namespace, name)` selector. The new "Events timeline" UI section is a
presentational wrapper around that selector's already-live-updating result (FR-015), filtered the
same way the existing generic Events tab already filters (`ui/src/components/Inspector.tsx`'s
`EventsFor`), so every event — recognized reason or not — is shown (FR-019); no client-side
reason filtering is added.
