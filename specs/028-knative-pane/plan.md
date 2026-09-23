# Implementation Plan: Knative Pane

**Branch**: `028-knative-pane` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/028-knative-pane/spec.md`

## Summary

Add a read-only Knative Serving tool-pane: detect the `serving.knative.dev` CRD group the same
way `src/backend/flux.rs` detects Flux, list Services/Revisions/Routes, and add a traffic-split
visualization plus a plain-language autoscaler "why" explanation. The visualization extends the
existing topology graph (`ui/src/components/topology/TopologyCanvas.tsx` + `ui/src/topology/*`,
spec 007) with a small, additive `TopologyEdge.label` field and two new node kinds
(`KnativeService`→already covered generically, `KnativeRevision`, `KnativeRoute`) rather than a
new renderer. Backend follows the Flux pattern exactly: a `backend/knative.rs` discovery module,
a `KnativeOverview` protocol type, a 20s background poll (mirroring `ensure_flux_poll`), a
`RefreshKnative` command, and a `KnativeOverviewUpdated` event.

## Technical Context

**Language/Version**: Rust 1.75+ (backend, `kube-rs`/`k8s-openapi`), TypeScript 5 / React 18 (frontend, Vite)

**Primary Dependencies**: `kube`, `k8s-openapi`, `serde`, `ts-rs` (backend); Zustand store, `@phosphor-icons/react`, existing `ui/src/topology/*` graph engine, Vitest, Playwright (frontend)

**Storage**: N/A — no persistence; live cluster reads only, held in in-memory store state like `flux.overview`

**Testing**: `cargo test` (backend unit + protocol roundtrip tests), `pnpm -C ui test` (Vitest unit tests for scaling-reason derivation and traffic-split math), `pnpm -C ui test:e2e` / Playwright spec in `ui/tests/e2e/`

**Target Platform**: Tauri v2 desktop app (Linux/Windows/macOS), same as the rest of Fathom

**Project Type**: Existing single desktop app with Rust backend (`src/`) + React frontend (`ui/`) — matches the "web application" split already in use, not a new project type

**Performance Goals**: Pane populates within the existing 20s Flux-style poll cadence, with an immediate on-open refresh (matches SC-001, "within 10 seconds of opening")

**Constraints**: Read-only — zero mutating kube-rs calls introduced by this feature (constitution Principle I + FR-011); UI must not talk to the cluster directly (Principle II) — all reads go through the versioned protocol

**Scale/Scope**: One new backend discovery module, one new protocol file, one new store slice, one new pane component, an extension to the existing topology types/build/draw code, and one Playwright e2e spec — no new services or renderers

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. This feature adds no mutating commands. No
  confirmation dialogs are needed because there is nothing to confirm — FR-011 forbids any
  mutation UI. Errors surfaced from `kube-rs` list calls are passed through, not replaced, mirroring
  `scan_flux`'s `unwrap_or_default()`-per-list behavior (missing/broken CRD listing degrades that
  bucket to empty rather than erroring the whole pane, exactly like Flux).
- **II. One Contract, Backend Owns the Cluster**: PASS. All cluster reads live in
  `src/backend/knative.rs`; the UI only sees `KnativeOverview` via the existing command/event
  protocol (`RefreshKnative` command, `KnativeOverviewUpdated` event), generated into
  `ui/src/protocol/generated/*.ts` via the existing `ts-rs` build step — no new bespoke bridge.
- **III. Tested at the Boundaries**: PASS (planned). Protocol roundtrip test for the new command/
  event variants; Rust unit tests for the traffic-split percentage extraction and the autoscaler
  "why" derivation (pure functions, easy to unit test against constructed `DynamicObject`/JSON
  fixtures, mirroring `flux.rs`'s `summarize`/`classify_ready` tests); a Vitest unit test for the
  frontend "why" label mapping; one focused Playwright e2e spec per the task brief.

No violations requiring the Complexity Tracking table.

## Project Structure

### Documentation (this feature)

```text
specs/028-knative-pane/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/            # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── knative.rs            # NEW — CRD discovery + summarize (mirrors flux.rs)
│   ├── sync.rs                # + ensure_knative_poll / poll_knative_overview (mirrors flux)
│   └── cmd/
│       └── integrations.rs    # + RefreshKnative handler (mirrors RefreshFlux)
├── protocol/v1/
│   ├── knative.rs             # NEW — KnativeOverview, KnativeServiceSummary,
│   │                          #       KnativeRevisionSummary, KnativeRouteSummary,
│   │                          #       KnativeScaleReason
│   ├── cmd.rs                  # + RefreshKnative variant
│   └── event.rs                 # + KnativeOverviewUpdated variant

ui/src/
├── protocol/generated/         # regenerated: KnativeOverview.ts, etc. (ts-rs build output)
├── store/
│   ├── index.ts                # + knative: { overview?: KnativeOverview } slice
│   └── handlers/system.ts       # + KnativeOverviewUpdated handling (mirrors FluxOverviewUpdated)
├── resources/
│   └── knative-resource.tsx    # NEW — mirrors flux-resource.tsx if a section/list is needed
├── components/panels/
│   └── KnativePanel.tsx         # NEW — mirrors FluxPanel.tsx, embeds the traffic-split graph
├── topology/
│   ├── types.ts                  # + TopologyEdge.label?, new node kinds for Route/Revision
│   ├── build.ts                   # + buildKnativeTrafficGraph (or extend buildTopologyGraph)
│   └── ...                        # layout.ts/status.ts reused unchanged where possible
└── components/topology/
    └── TopologyCanvas.tsx         # + draw edge label at midpoint when `edge.label` is set

tests/ (backend, alongside modules per repo convention: `#[cfg(test)] mod tests` in knative.rs)
ui/tests/e2e/
└── knative-pane.spec.ts          # NEW — Playwright e2e per task brief
ui/src/topology/*.test.ts, ui/src/components/panels/*.test.tsx  # NEW unit tests
```

**Structure Decision**: Extends the existing single Tauri app's two source trees (`src/` Rust
backend, `ui/` React frontend) with feature-scoped additions; no new top-level project, package,
or renderer is introduced. The traffic-split visualization is implemented as an additive
extension of `ui/src/topology/*` and `TopologyCanvas.tsx`, not a parallel graph implementation.

## Complexity Tracking

*No constitution violations — table not needed.*
