# Implementation Plan: Ephemeral Debug Containers

**Branch**: `025-ephemeral-debug-containers` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/025-ephemeral-debug-containers/spec.md`

## Summary

Add a new mutating command, `AddEphemeralContainer`, that patches a running pod's
`ephemeralcontainers` subresource with a new ephemeral container (debug image + target
container to share the process namespace with), following the existing mutation-guard pattern
(`ConfirmDialog`/`ActionDialogHost` in the UI, `MutationTarget::Namespaced` classification in
`src/protocol/v1/mutation.rs`). The confirm dialog explicitly warns that the container cannot be
removed. Once added, the ephemeral container's status is surfaced through the existing
`ContainerLifecycle` model (extended with an `is_ephemeral` flag) so it appears in the pod's
container list, and its logs/exec are reachable via the already-implemented `PodLogsTab` /
`PodTerminalTab` components — no new streaming plumbing is needed for either.

## Technical Context

**Language/Version**: Rust 1.75+ (backend, `kube-rs`), TypeScript 5 / React 18 (UI, Vite)

**Primary Dependencies**: `kube` crate (`Api<Pod>::patch_ephemeral_containers` or
strategic-merge/JSON patch to the `ephemeralcontainers` subresource), existing protocol codegen
(`ts-rs`), existing Zustand store (`ui/src/store/`), shadcn/Radix `Dialog` primitives already
used by `ConfirmDialog.tsx`.

**Storage**: N/A (no new persistence; ephemeral container state lives entirely in the
Kubernetes API server, read back via the existing pod watch/refresh path).

**Testing**: `cargo test` (protocol round-trip + mutation classification unit tests),
Playwright e2e in `ui/tests/e2e/` against the existing scripted fake backend harness.

**Target Platform**: Desktop (Tauri v2: Linux/Windows/macOS), same as the rest of Fathom.

**Project Type**: Desktop app — Rust backend + React/TS frontend (existing structure, no new
top-level project).

**Performance Goals**: No new perf budget; the mutation is a single API call and status updates
ride the existing pod refresh/watch cadence.

**Constraints**: Must not add any ephemeral-container *removal* affordance (Kubernetes API does
not support it). Must degrade gracefully on clusters where `ephemeralcontainers` PATCH is denied
by RBAC or unsupported.

**Scale/Scope**: One new command/event pair, one new confirm-gated UI action, one additive field
on `ContainerLifecycle`, reuse of two existing UI components (logs, exec) — no new panels.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. The action is gated by `ConfirmDialog` with an
  explicit "cannot be removed" warning and names the target pod/container/image before
  confirmation, matching the existing pattern (e.g. `DeletePod`). No optimistic UI update — the
  container list only reflects the ephemeral container once the backend reports pod state that
  includes it (via the same refresh/watch path as all other container status).
- **II. One Contract, Backend Owns the Cluster**: PASS. The Kubernetes API call (patch
  `ephemeralcontainers` subresource) lives entirely in `src/backend/`. The UI only sends
  `AddEphemeralContainer` and reads back `PodDetails`/`ContainerLifecycle` through the existing
  protocol. No new UI-side cluster access.
- **III. Tested at the Boundaries (NON-NEGOTIABLE)**: PASS (planned). New command/event variants
  get serialization round-trip tests; `mutation.rs`'s classification gets a unit test; a
  Playwright e2e exercises the full confirm → mutate → status-appears flow against the fake
  backend, per the task brief's requirement of one hyper-focused scenario.
- **IV. Density Scales With Width**: PASS. Reuses existing container-list and dialog layout
  patterns; no new pane/breakpoint behavior introduced.

No violations. Complexity Tracking table is not needed.

## Project Structure

### Documentation (this feature)

```text
specs/025-ephemeral-debug-containers/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── add-ephemeral-container.md
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── cmd.rs            # + BackendCmdV1::AddEphemeralContainer { namespace, name, target_container, image, container_name }
│   ├── event.rs          # + outcome reporting via existing ActionOutcome/CommandFinished path (no new event variant expected; confirm during Phase 0)
│   ├── mutation.rs        # + AddEphemeralContainer classified as MutationTarget::Namespaced
│   └── pods.rs             # + ContainerLifecycle.is_ephemeral: bool
├── backend/
│   ├── cmd/workloads.rs   # + dispatch AddEphemeralContainer -> pods module
│   └── pods.rs (or equivalent cluster-access module) # + patch_ephemeral_containers Kubernetes call
└── app/views/pods.rs        # (if UI-adjacent Rust glue exists here; verify during implementation)

ui/src/
├── resources/pods.tsx       # + addEphemeralContainerAction() alongside deleteAction<PodSummary>()
├── components/
│   ├── ActionDialogHost.tsx # + branch for the new action's confirm copy/detail (warning text)
│   └── panels/
│       ├── PodLogsTab.tsx    # no change expected; verify it works by container name alone
│       └── PodTerminalTab.tsx # no change expected; verify container picker includes ephemeral containers
└── tests/e2e/
    └── ephemeral-debug-container.spec.ts  # new, single focused scenario
```

**Structure Decision**: Existing single-repo desktop-app layout (Rust backend in `src/`, React
UI in `ui/src/`) is reused as-is. This feature adds one protocol command/classification, one
backend handler, one additive field on an existing DTO, and one new UI action wired through
existing generic components — no new modules/panels/projects.

## Complexity Tracking

*No constitution violations — table intentionally omitted.*
