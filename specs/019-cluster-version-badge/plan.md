# Implementation Plan: Kubernetes Cluster Version Badge

**Branch**: `019-cluster-version-badge` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/019-cluster-version-badge/spec.md`

## Summary

Add a small, read-only badge in the app shell (next to the existing `FederationContextSwitcher`)
that shows the connected cluster's Kubernetes server version (`GET /version`, already reachable
via `kube::Client::apiserver_version()` — the same call `context_status.rs` uses for reachability
probes), plus a warning affordance when any node's `status.nodeInfo.kubeletVersion` differs from
the server version. The backend computes this on connect and refreshes it periodically via a new
`ensure_cluster_version_poll` background task mirroring `sync::ensure_flux_poll` /
`poll_flux_overview` exactly (20s interval, one `SystemEvent` emission per tick, aborted/restarted
on context switch via `WatchTasks::abort_all`). A new protocol type, `ClusterVersionSummary`, and
a new `SystemEvent::ClusterVersionUpdated(ClusterVersionSummary)` variant carry the result. The
frontend stores it in a new `clusterVersion` slice (via the existing `system.ts` handler switch)
and renders it as a small pill component, `ClusterVersionBadge`, mounted beside
`FederationContextSwitcher` in `AppShell.tsx`, with a hover/click popover listing distinct kubelet
versions when skew is detected.

## Technical Context

**Language/Version**: Rust (edition 2024, backend) / TypeScript strict (frontend, Vite/React)

**Primary Dependencies**: `kube` 2.0.1 (`Client::apiserver_version()`, `Api::<Node>`), `tokio`
(background poll task), `serde`/`ts-rs` (protocol types + generated TS bindings), Zustand (frontend
store), Playwright (e2e)

**Storage**: N/A — live, in-memory only; nothing persisted to `~/.fathom/`

**Testing**: `cargo test` (protocol serde round-trip in `tests/protocol_serde.rs`), Playwright e2e
(`ui/tests/e2e/`), `pnpm -C ui typecheck` / `lint`

**Target Platform**: Existing Tier 1 desktop targets (Ubuntu 26.04, Windows 11, macOS 26); no new
platform surface

**Project Type**: Existing Tauri v2 desktop app (Rust backend + React/TS frontend), extending
established protocol/backend/frontend layers — no new project structure

**Performance Goals**: Poll every 20s (matches Flux/Hubble pollers); badge render adds no
measurable delay to app-shell first paint (Constitution V — idle CPU, no blocking fetch)

**Constraints**: Read-only (no mutation, no confirmation gating — Constitution I does not apply);
must not block app-shell render (FR-006); must gracefully show "unknown" when `/version` fails or
times out (mirrors `context_status.rs`'s bounded 5s probe pattern) rather than erroring the whole
event stream

**Scale/Scope**: One cluster/context at a time (matches existing single active `kube::Client`
model, Principle II); node counts are whatever the operator's RBAC already exposes via the
existing node-listing path

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: N/A — feature is strictly read-only, no mutating
  commands added. PASS.
- **II. One Contract, Backend Owns the Cluster**: All Kubernetes access (`/version` discovery,
  node listing) stays in the Rust backend behind a new `SystemEvent` variant in
  `src/protocol/v1/`; UI only reads the generated TS binding via the store. PASS.
- **III. Tested at the Boundaries**: New `ClusterVersionSummary` type gets a serde round-trip test
  in `tests/protocol_serde.rs` (and an `all_system_events_is_exhaustive` arm); new
  `ClusterVersionUpdated` event is exercised by a new, hyper-focused Playwright e2e test following
  `us1-federation-context-status.spec.ts`'s structure. PASS (planned, not yet written).
- **IV. Density Scales With Width**: Badge is a small persistent shell element, present at all
  width classes (same tier as `FederationContextSwitcher`, which the plan mounts it beside); no
  pane count regression. PASS.
- **V. Fast and Quiet**: Poll interval matches existing 20s pattern (idle-CPU friendly); no
  per-frame recompute; version poll is bounded and best-effort (errors become a neutral "unknown"
  state, never a blocking retry loop). PASS.
- **VI. User Data Is Sacred**: Nothing persisted; no credentials or secrets touched; uses the same
  client/context/RBAC as everything else. PASS.

No violations — Complexity Tracking table not needed.

## Project Structure

### Documentation (this feature)

```text
specs/019-cluster-version-badge/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── protocol.md      # Phase 1 output — new protocol type/event contract
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── backend.rs                       # add `mod cluster_version;`
├── backend/
│   ├── cluster_version.rs           # NEW: fetch_cluster_version() — server version + node
│   │                                 #   kubelet versions + skew detection (mirrors flux.rs)
│   ├── sync.rs                      # add ensure_cluster_version_poll() + poll task
│   │                                 #   (mirrors ensure_flux_poll/poll_flux_overview)
│   ├── state.rs                     # WatchTasks: add cluster_version_poll_task field
│   └── cmd/cluster.rs               # refresh_core_resources(): call
│                                     #   sync::ensure_cluster_version_poll(...)
├── protocol/v1/
│   ├── cluster_version.rs           # NEW: ClusterVersionSummary, NodeKubeletVersion types
│   ├── event.rs                     # add SystemEvent::ClusterVersionUpdated(..) variant
│   ├── route.rs                     # add section_keys::cluster_version() (or reuse system())
│   └── mod.rs                       # `mod cluster_version;` + `pub use cluster_version::*;`
└── (tests) tests/protocol_serde.rs  # sample_cluster_version_summary() + roundtrip + exhaustive
                                       # match arm

ui/src/
├── components/shell/
│   ├── AppShell.tsx                 # mount <ClusterVersionBadge /> beside
│   │                                 #   <FederationContextSwitcher />
│   └── ClusterVersionBadge.tsx      # NEW: badge + skew popover, mirrors
│                                     #   FederationContextSwitcher's status-dot conventions
├── store/
│   ├── handlers/system.ts           # add `case 'ClusterVersionUpdated'` → d.put('clusterVersion', ...)
│   └── index.ts                     # add `clusterVersion` slice to store shape
└── protocol/generated/              # ts-rs regenerated: ClusterVersionSummary.ts, updated
                                       # SystemEvent.ts (generated, not hand-written)

ui/tests/
├── fixtures/builders.ts             # (reuse existing `system()` builder — no change needed)
├── fixtures/index.ts / scenarios.ts # optionally register a `cluster-version` fixture batch
└── e2e/uc-cluster-version-badge.spec.ts   # NEW: hyper-focused e2e per spec's two user stories
```

**Structure Decision**: Extends the existing single Tauri app (Rust backend + React frontend) with
no new top-level directories — one new backend module, one new protocol module, one new frontend
component, one new e2e spec, following the Flux/Hubble poll-and-badge precedent end-to-end.

## Complexity Tracking

*No violations — table omitted.*
