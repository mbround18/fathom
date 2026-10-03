# Phase 1 Contract: protocol additions

**Feature**: [spec.md](../spec.md) | **Data model**: [data-model.md](../data-model.md)

Principle II: defined once in Rust, generated into `ui/src/protocol/generated/`, drift fails
`scripts/ci.sh`. Every variant below needs a round-trip test (Principle III) and a
`hosted_policy` arm (the match in `src/protocol/v1/mutation.rs` is exhaustive with no wildcard, so
the compiler enforces this).

---

## Commands — `BackendCmdV1` additions

### Presence and inventory

| Variant | Payload | Notes |
|---|---|---|
| `RefreshContainerRuntimePresence` | — | re-runs the probe; bounded by the existing timeout |
| `RefreshContainerInventory` | `{ classes: Vec<ObjectClass> }` | one batched query per requested class; the scene asks for what it is showing, nothing more |
| `LoadProjectDefinition` | `{ project: String }` | reads the definition file to name it; never writes |

### Adoption

| Variant | Payload | Notes |
|---|---|---|
| `AdoptContainerProject` | `{ project: String }` | persists the grant |
| `ReleaseContainerProject` | `{ project: String }` | returns it to read-only; refuses for `ImplicitViaManagedCluster` |

### Preview, then act

Every mutation is two commands: build a preview, then issue the previewed operation. The issue
command carries the preview's identity so the backend can refuse to run something the operator did
not see.

| Variant | Payload | Notes |
|---|---|---|
| `PreviewContainerOperation` | `{ request: OperationRequest }` | returns `ContainerOperationPreviewed`; no side effect |
| `IssueContainerOperation` | `{ preview_id: String, typed_confirmation: Option<String> }` | `typed_confirmation` required and checked when the preview is `destructive` (FR-023); the backend re-derives the argv from `preview_id` and MUST refuse if the inventory moved under it in a way that changes the removal set |

`OperationRequest` is an enum covering the operation surface, each carrying only documented object
fields — there is deliberately no free-text command variant (FR-019):

- `Lifecycle { action: LifecycleAction, scope: OperationScope }` where `LifecycleAction` is
  `Start | Stop | Restart | Pause | Unpause | Kill` and `OperationScope` is
  `Container { id: String } | Project { name: String }`
- `Remove { objects: Vec<ObjectRef> }` — targeted by identity, which is how FR-025's exclusion is
  possible at all (research D3)
- `PruneSelection { class: ObjectClass, filter: PruneFilter }` — resolves to an enumerated,
  excludable removal set, never a broad prune handed to the runtime
- `CreateContainer { spec: ContainerSpec }`
- `BuildImage { spec: ImageBuildSpec }`
- `CreateNetwork { spec: NetworkSpec }` / `CreateVolume { spec: VolumeSpec }`
- `ProjectLifecycle { project: String, action: ProjectAction }` where `ProjectAction` is
  `Up | Down | Restart`

`ContainerSpec`, `ImageBuildSpec`, `NetworkSpec` and `VolumeSpec` enumerate their fields
explicitly. **No field of type "extra arguments", "raw command" or equivalent may be added to any
of them**; such a field would make the preview guarantee unkeepable by construction (FR-019,
SC-007).

### Logs, sessions, usage

| Variant | Payload | Notes |
|---|---|---|
| `StartContainerLogStream` | `{ id: String, follow: bool }` | allowed regardless of adoption (FR-035) |
| `StopContainerLogStream` | `{ id: String }` | |
| `StartContainerSession` | `{ id: String }` | **requires adoption** (FR-013, FR-036) |
| `StopContainerSession` | `{ session_id: String }` | ends the PTY, leaves no process |
| `SendContainerSessionInput` | `{ session_id: String, data: String }` | |
| `ResizeContainerSession` | `{ session_id: String, cols: u16, rows: u16 }` | |
| `StartContainerUsageSampling` | `{ ids: Vec<String> }` | scoped to visible rows; no sampling when the view is hidden (FR-045, research D4) |
| `StopContainerUsageSampling` | — | |

### Hosted policy

All of the above: `HostedPolicy::Denied(HostedDenial::ContainerRuntime)`.

New variant on `HostedDenial`:

```rust
/// Local container runtime management. A server has no local runtime, and exposing the host's
/// daemon to signed-in users would grant them privileges they do not have.
ContainerRuntime,
```

Reusing `HostedDenial::Kind` is rejected: its documented meaning is local kind provisioning, and
FR-043 requires the denial to state the actual reason.

---

## Events — `StreamEvent` additions

| Variant | Payload | Notes |
|---|---|---|
| `ContainerRuntimePresenceUpdated` | `{ presence: RuntimePresence }` | drives whether the scene exists at all |
| `ContainerInventoryUpdated` | `{ groups: Vec<ProjectGroup>, objects: Vec<RemovableObject>, as_of: String }` | `as_of` lets the UI mark a list stale rather than blanking it when the runtime goes away mid-scene (US1 scenario 5) |
| `ContainerInventoryUnavailable` | `{ presence: RuntimePresence, detail: String }` | the runtime became unreachable while the scene was open; `detail` verbatim |
| `ContainerAdoptionUpdated` | `{ adopted: Vec<Adoption>, fell_back: bool }` | `fell_back` true when the record was unreadable and the empty set was used — the operator is told, not silently un-adopted (FR-047) |
| `ContainerOperationPreviewed` | `{ preview_id: String, preview: OperationPreview }` | |
| `ContainerOperationFinished` | `{ preview_id: String, outcome: OperationOutcome }` | |
| `ContainerOperationRefused` | `{ preview_id: String, reason: OperationRefusal }` | `NotAdopted(String) | PreviewStale | ConfirmationMismatch | PresenceLost` — a refusal is never silent |
| `ContainerLogBatch` | `{ id: String, lines: Vec<String>, truncated: bool }` | mirrors the existing pod-log batching, truncation explicit |
| `ContainerSessionOutput` | `{ session_id: String, data: String }` | |
| `ContainerSessionClosed` | `{ session_id: String, detail: Option<String> }` | |
| `ContainerUsageSampled` | `{ samples: Vec<UsageSample>, at_cap: bool }` | `at_cap` makes FR-038's bound visible |
| `ContainerClusterLinksUpdated` | `{ links: Vec<ClusterLink>, }` | both directions (FR-039, FR-040) |

`ClusterLink`: `{ cluster: String, container_ids: Vec<String>, managed_by_fathom: bool }`.
`managed_by_fathom` is what FR-016's implicit adoption is computed from.

---

## Contract tests required

1. **Round-trip** per command and event variant (`src/protocol/v1/cmd.rs`, `event.rs` test modules
   — follow the existing `roundtrip(...)` pattern).
2. **Hosted policy exhaustiveness** — every new command denies with `ContainerRuntime`; asserted,
   not assumed.
3. **Bindings drift** — `cargo test -p fathom --test export_bindings` produces no diff, which
   `scripts/ci.sh` already enforces.
4. **The argv invariant** — `IssueContainerOperation` executes byte-identical argv to what the
   matching `ContainerOperationPreviewed` carried. This is the single most important test in the
   feature; without it the preview is decoration.
5. **No raw-command field** — a test (or a review checklist item backed by a grep) asserting that
   no spec type carries a free-text argument field, so FR-019 cannot erode later.
