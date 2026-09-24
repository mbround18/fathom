# Phase 0 Research: Ephemeral Debug Containers

## Decision: New command shape

**Decision**: Add `BackendCmdV1::AddEphemeralContainer { namespace: String, pod: String,
target_container: Option<String>, image: String, ephemeral_container_name: Option<String> }` to
`src/protocol/v1/cmd.rs`, alongside the existing `DeletePod`/`StartPodExec` pod commands.

- `namespace` + `pod`: identify the target pod (matches the `namespace`/`name` shape used by
  `DeletePod`, but the field is named `pod` here rather than `name` to avoid ambiguity with the
  ephemeral container's own name).
- `target_container`: which existing container's process namespace to share
  (`spec.ephemeralContainers[].targetContainerName`). `Option<String>` because a single-container
  pod can auto-select its only container (spec Assumption); `None` means "let the backend pick
  the sole container or error if there is more than one."
- `image`: the debug image reference, free text per spec FR-002 (no catalog).
- `ephemeral_container_name`: `Option<String>` — if not supplied, the backend generates a name
  (e.g. `debugger-<short-random>`), matching `kubectl debug`'s default naming behavior, so the
  user is not forced to invent a unique name.

**Rationale**: Mirrors existing pod-command shape conventions (`DeletePod`, `StartPodExec`) for
consistency and to keep the `mutation.rs` match arm trivial to review. Keeping the container name
optional avoids adding a required text field the user has no strong opinion about, which keeps
the confirm dialog focused on the two things that actually matter (image, target container).

**Alternatives considered**:
- Requiring an explicit ephemeral container name from the user: rejected — adds a field with no
  meaningful default guidance and a naming-collision failure mode for no user benefit; `kubectl
  debug` itself auto-generates names by default.
- Modeling this as a generic "PatchPod" command: rejected — violates the one-arm-per-variant
  exhaustive-match design explicitly called out in `mutation.rs`'s file header, and loses
  type-safety on the ephemeral-container-specific fields.

## Decision: Mutation classification

**Decision**: Classify `AddEphemeralContainer` in `mutation.rs` as
`MutationTarget::Namespaced { namespaces: vec![namespace] }`, in the same match arm group as
`DeletePod`/`ScaleDeployment`/etc.

**Rationale**: It is a genuine namespaced cluster write (patches a Pod). This makes
`writes_cluster()` return `true`, which is what routes it through the mutation-guard/confirmation
path used by every other write in the app.

**Alternatives considered**: None — this is a direct, unambiguous application of the existing
enum; no new `MutationTarget` variant is needed.

## Decision: Kubernetes API call shape

**Decision**: Use `kube`'s typed `Api<Pod>::patch_ephemeral_containers(&pod_name, &PatchParams,
&Patch::Strategic(...))` (or JSON merge patch against `pods/ephemeralcontainers` if the installed
`kube` version does not expose the strategic helper) to append one entry to
`spec.ephemeralContainers`, setting `name`, `image`, and `targetContainerName`. This subresource
has been GA since Kubernetes 1.25 (matches the spec's Assumptions section).

**Rationale**: This is the direct Rust-client equivalent of `kubectl debug`'s
`AddEphemeralContainer` call and of the raw `PATCH .../ephemeralcontainers` HTTP call. Using the
subresource-specific API avoids clobbering the rest of the pod spec (a full-object PUT/replace
would risk conflicting with concurrent controller writes).

**Alternatives considered**:
- Full pod replace via `Api<Pod>::replace`: rejected — races with the pod's own controllers and
  risks unrelated field drift; the ephemeral-containers subresource exists precisely to avoid
  this.
- Shelling out to `kubectl debug`: rejected outright — violates constitution principle II (all
  cluster access lives in the Rust backend via the typed client, not by spawning `kubectl`).

## Decision: Surfacing ephemeral container status

**Decision**: Add `is_ephemeral: bool` to the existing `ContainerLifecycle` struct in
`src/protocol/v1/pods.rs`, and populate `PodDetails.containers_lifecycle` with one entry per
`status.ephemeralContainerStatuses[]` entry (same `ContainerState`/`ready`/`restart_count` mapping
already used for regular containers), read via the pod's existing refresh/describe path.

**Rationale**: `ContainerLifecycle` already models exactly the shape needed (name, image, state,
ready, restart_count) and is already rendered in the pod detail UI (008-pod-lifecycle-diagnostics).
Reusing it means the container list, `PodLogsTab`, and `PodTerminalTab` all "just work" against
ephemeral containers once they exist in this array — none of those components need to know
ephemeral containers are a distinct concept.

**Alternatives considered**:
- A separate `ephemeral_containers: Vec<EphemeralContainerStatus>` field on `PodDetails`:
  rejected — forces every downstream consumer (container list, logs tab, exec tab) to be taught
  about a second collection, when the existing one already has the right shape plus one boolean
  flag.

## Decision: Reusing logs/exec instead of building new plumbing

**Decision**: No changes to `StartPodLogs`/`StopPodLogs` or
`StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec`. Both already accept a
`container: Option<String>` by name and don't distinguish "regular" vs "ephemeral" containers —
the Kubernetes logs/exec APIs themselves don't distinguish them either (an ephemeral container is
just another container name once it exists). `PodLogsTab.tsx` and `PodTerminalTab.tsx` need no
code changes; the pod's container-picker UI (already fed from `containers_lifecycle`) will simply
include the new entry once `is_ephemeral` support lands in the DTO and the picker doesn't filter
it out.

**Rationale**: Confirmed in prior codebase research (this session) that pod exec is fully
implemented end-to-end and already keys off container name; verified `StartPodLogs`/`StartPodExec`
take `container: Option<String>` with no container-kind filtering. Minimal-diff reuse matches the
spec's explicit goal (FR-011, FR-014) of not building new streaming plumbing.

**Alternatives considered**: None needed — this was a verification task, not a design choice.

## Decision: Confirm dialog wiring

**Decision**: Define the action via a small dedicated helper (e.g. `addEphemeralContainerAction()`
in `ui/src/resources/pods.tsx`, alongside `deleteAction<PodSummary>()`), producing an
`ActionDescriptor` with `destructive: true` (irreversible, even though not literally
"delete"-shaped) and a custom `detail` string rendered by `ActionDialogHost.detailFor()` that
states the target pod, chosen target container, and image, plus the fixed warning: "Ephemeral
containers cannot be removed once added and will persist for the life of the pod." The dialog
itself continues to use the existing generic `ConfirmDialog.tsx` — no new dialog component.

**Rationale**: Matches spec FR-004/005/006 and reuses 100% of the existing
`ConfirmDialog`/`ActionDialogHost` machinery; `destructive: true` gets the same red/warning
styling already used for delete actions, which is an accurate signal for an irreversible action
even though nothing is being deleted.

**Alternatives considered**:
- A bespoke modal for this one action: rejected — unnecessary duplication of styling/focus-trap
  logic already solved generically; also inconsistent with how every other mutating action in the
  app is confirmed.

## Open questions resolved

All "NEEDS CLARIFICATION" items from the Technical Context are resolved above; none remain.
