# Implementation Plan: Pod Lifecycle Diagnostics

**Branch**: `008-pod-lifecycle-diagnostics` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/008-pod-lifecycle-diagnostics/spec.md`

## Summary

Add five read-only, additive sections to the Pod Inspector (`ui/src/resources/pods.tsx`): init
containers (distinct from app containers), per-container probe configuration + best-effort
failing state, per-container restart history, per-container lifecycle hooks + failure
correlation, and a Pod-scoped Events timeline. The backend assembles one new nested type,
`ContainerLifecycle`, per container (app and init) from the existing `Pod` object already fetched
by `fetch_pod_details`, and adds it as a new field on `PodDetails` (additive — no existing field
removed or changed). Probe/hook *failure* state is derived on the frontend by joining this new
structural data against the Pod's own Events, which are already fetched via the existing
`RefreshEvents` command and already available through the store's `eventsFor` selector
(`ui/src/store/index.ts`) — no second events-fetching path, per the spec's assumption and FR-016.

## Technical Context

**Language/Version**: Rust 2024 edition (backend, `kube-rs`/`k8s-openapi`), TypeScript 5 /
React 18 (frontend, Vite).

**Primary Dependencies**: `kube`, `k8s-openapi`, `serde`, `ts-rs` (backend → protocol bindings);
React, Zustand-style store (`ui/src/store`), Playwright (frontend). No new dependency needed —
confirmed against `Cargo.toml` and `ui/package.json`.

**Storage**: N/A (live cluster reads only; no new persistence).

**Testing**: `cargo test` (protocol round-trip + backend unit tests), `pnpm -C ui test` (Vitest
component/unit tests), Playwright e2e (`ui/tests/e2e`).

**Target Platform**: Tauri v2 desktop app, Tier 1 (Ubuntu 26 / Windows 11 / macOS 26).

**Project Type**: Desktop app (Rust backend + React/TS frontend), single repo.

**Performance Goals**: No new streaming/polling; the new data rides the existing `SelectPod` +
`RefreshEvents` reads, so no additional perf budget beyond constitution V's existing baselines.

**Constraints**: Read-only (constitution I mostly inapplicable — no new mutating actions,
confirmed by FR-017); backend-owns-cluster (constitution II — all new fields assembled in
`src/backend/pods.rs`, UI reads them via the protocol only); additive-only to the Pod Inspector
(FR-018) and to `PodDetails` (no field removed/renamed, so no protocol version bump needed).

**Scale/Scope**: One Kubernetes kind (Pod). Up to `spec.containers.len() + spec.initContainers.len()`
entries per Pod (typically single digits).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. This feature adds zero mutating commands. No
  confirmation/diff UI is needed since nothing here changes cluster state (FR-017, SC-006).
- **II. One Contract, Backend Owns the Cluster**: PASS. All new data is assembled in
  `src/backend/pods.rs` from the `Pod` object already fetched there; the UI reads it only through
  the extended `PodDetails` protocol type and the existing `eventsFor` store selector (itself
  fed by the existing `RefreshEvents` command/`EventsUpdated` event). No direct cluster access
  from the UI. `PodDetails` gains one new field; existing fields are untouched, so no client-facing
  breaking change and no protocol version bump.
- **III. Tested at the Boundaries**: PASS (planned). New protocol type gets a serde round-trip
  test in `tests/protocol_serde.rs`. New backend assembly logic gets unit tests in
  `src/backend/pods.rs`'s `#[cfg(test)] mod tests` using fixture `Pod`/`Event` objects (no live
  cluster). New frontend probe/hook-failure correlation logic gets Vitest unit tests. Each user
  story gets a Playwright e2e spec against the fake backend, named `us{N}-pod-lifecycle-*.spec.ts`.
- **IV. Density Scales With Width**: PASS. New sections are additional `DetailSection`s in the
  existing Details tab, which already scrolls vertically inside the inspector pane at every width
  class; no new pane, no horizontal scroll introduced.
- **V. Fast and Quiet**: PASS. No new polling/streaming; assembly is synchronous over data already
  in memory (`Pod` struct, already-fetched events list).
- **VI. User Data Is Sacred**: PASS. No `~/.fathom/` state touched; no secrets newly displayed
  (probe/hook `exec` commands and `httpGet` paths are spec content, not credentials, and the spec's
  edge cases explicitly say to show them verbatim, same as Fathom's existing YAML/spec display).

No violations. Complexity Tracking table is not needed.

## Project Structure

### Documentation (this feature)

```text
specs/008-pod-lifecycle-diagnostics/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── pod-lifecycle.md
└── tasks.md             # Phase 2 output (speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/pods.rs       # + ContainerLifecycle, ContainerState, ProbeSpec, LifecycleHookSpec
│                              #   + PodDetails.containers_lifecycle field (additive)
└── backend/pods.rs           # + build_container_lifecycle(&Pod) -> Vec<ContainerLifecycle>
                               #   wired into fetch_pod_details; + #[cfg(test)] unit tests

tests/
└── protocol_serde.rs         # + round-trip tests for the new types

ui/src/
├── resources/
│   ├── pods.tsx               # + "Lifecycle" DetailSection(s), additive after "Related"
│   └── podLifecycle.ts        # + probe/hook failing-state correlation (pure, unit-testable)
├── resources/podLifecycle.test.ts   # Vitest unit tests for the correlation logic
└── components/
    └── PodLifecyclePanels.tsx # + presentational components (init containers, probes,
                                #   restart history, hooks, events timeline), data-testid
                                #   values prefixed `pod-lifecycle-`

ui/tests/
├── fixtures/gen-pods.ts        # + lifecycle-bearing PodDetails fixture builder
└── e2e/
    ├── us1-pod-lifecycle-init-containers.spec.ts
    ├── us2-pod-lifecycle-probes.spec.ts
    ├── us3-pod-lifecycle-restarts.spec.ts
    ├── us4-pod-lifecycle-hooks.spec.ts
    └── us5-pod-lifecycle-events-timeline.spec.ts
```

**Structure Decision**: Existing single-repo Tauri layout (`src/` Rust backend + protocol,
`ui/src` React frontend). This feature only adds files/fields; it does not restructure the
existing Pod Inspector or protocol layout, per FR-018 and the spec's Assumptions.

## Complexity Tracking

Not applicable — no Constitution Check violations.
