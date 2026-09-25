# Implementation Plan: Gatekeeper Policy Pane

**Branch**: `020-gatekeeper-policy-pane` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/020-gatekeeper-policy-pane/spec.md`

## Summary

Add a read-only Gatekeeper policy pane that detects OPA Gatekeeper via its `ConstraintTemplate`/
`Constraint` CRDs, shows a live list of constraints with violation counts, and lets the operator
drill into a constraint to see its violating resources with a link-out to each one's existing
Inspector. This is architecturally identical to the shipped Flux panel (specs 004/005/006) and
Hubble panel: a backend CRD-discovery/scan function, a 20s background poll task guarded against
double-start, a `SystemEvent` carrying the aggregated result, and a frontend tool-pane subscribing
to that event through the existing `IntegrationSummary` pattern. No new architecture, no new
external dependency — pure kube-rs dynamic-object CRD discovery, exactly as `src/backend/flux.rs`
already does.

## Technical Context

**Language/Version**: Rust 1.75+ (backend, `src/`), TypeScript 5 / React 18 (frontend, `ui/`)

**Primary Dependencies**: `kube-rs` (dynamic CRD discovery + listing, already a dependency via
Flux/Longhorn), `k8s-openapi` (`CustomResourceDefinition` type), `ts-rs` (protocol codegen),
existing frontend: Zustand store, `@phosphor-icons/react`, shadcn/tailwind components
(`IntegrationSummary`, `MetricsUnavailable`). No new crate or npm package.

**Storage**: N/A — no persisted state; live cluster reads only, same as Flux/Hubble.

**Testing**: `cargo test` (protocol round-trip tests, `summarize`/`scan` unit tests, an `#[ignore]`
live-cluster test mirroring `flux.rs`'s `scan_flux_against_live_cluster`), `pnpm -C ui test`
(Vitest unit tests for the panel/store slice), Playwright e2e in `ui/tests/e2e/` against a
scripted fake backend (per Constitution Principle III).

**Target Platform**: Same as the rest of Fathom — Tauri v2 desktop app, Tier 1 Ubuntu 26.04 /
Windows 11 / macOS 26.

**Project Type**: Desktop app (single Tauri project: Rust backend + React frontend, existing
repo layout — not a new project).

**Performance Goals**: Consistent with Constitution Principle V — pane interaction under 100ms;
background scan runs on the existing 20s poll cadence (matches Flux/Hubble), not a tighter loop,
so idle CPU stays negligible.

**Constraints**: Read-only, zero mutation surface (FR-009); zero new external dependencies
(FR-010); must not block or degrade existing panels' poll tasks.

**Scale/Scope**: Clusters with anywhere from zero to hundreds of constraint kinds/instances and
up to Gatekeeper's default `constraintViolationsLimit` (20) violations reported per constraint
in `.status.violations`; totals beyond that are summarized, not fetched further (FR-007).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. This pane has no mutation surface at all
  (FR-009) — strictly a read/display feature, so the confirmation-gate requirement doesn't apply.
- **II. One Contract, Backend Owns the Cluster**: PASS. All Gatekeeper CRD access lives in a new
  `src/backend/gatekeeper.rs`, exposed to the UI only via a new `SystemEvent::GatekeeperOverviewUpdated`
  and `BackendCmd::RefreshGatekeeper`, generated into `ui/src/protocol/generated/` the same way
  Flux's types are — no direct cluster access from the UI.
- **III. Tested at the Boundaries**: PASS (see Testing above) — protocol round-trip tests, unit
  tests for detection/summarization, and a hyper-focused 2-scenario Playwright e2e spec against a
  scripted fake backend, per the task brief.
- **IV. Density Scales With Width**: PASS. Reuses `IntegrationSummary`, which already implements
  the mobile-first responsive tile/row layout used by Flux/Hubble/Argo/Grafana panes — no new
  layout primitive introduced.
- **V. Fast and Quiet**: PASS. Same 20s poll cadence as Flux/Hubble; no new watch stream; violating
  resources are read from `.status.violations` already present on the `Constraint` object, not a
  second per-resource fetch loop.
- **VI. User Data Is Sacred**: PASS. No persisted state, no credentials involved.

No violations. Complexity Tracking table is not needed.

## Project Structure

### Documentation (this feature)

```text
specs/020-gatekeeper-policy-pane/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── protocol.md       # Phase 1 output: new SystemEvent/BackendCmd/types
└── tasks.md              # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── gatekeeper.rs          # NEW: CRD discovery, Constraint/violation summarization (mirrors flux.rs)
│   ├── sync.rs                # MODIFIED: ensure_gatekeeper_poll() + poll_gatekeeper_overview(), mirrors ensure_flux_poll
│   └── cmd/
│       └── integrations.rs    # MODIFIED: BackendCmd::RefreshGatekeeper handler, mirrors RefreshFlux
├── protocol/
│   └── v1/
│       ├── gatekeeper.rs      # NEW: GatekeeperOverview, GatekeeperConstraintSummary, GatekeeperViolation types
│       ├── cmd.rs             # MODIFIED: + RefreshGatekeeper variant
│       ├── event.rs           # MODIFIED: + SystemEvent::GatekeeperOverviewUpdated variant
│       ├── mutation.rs        # MODIFIED: route classification + read-only mutation-target mapping for the new command
│       ├── route.rs           # MODIFIED: + section_keys::integrations_gatekeeper()
│       └── mod.rs             # MODIFIED: re-export new gatekeeper module types

ui/
├── src/
│   ├── protocol/generated/    # MODIFIED (auto): GatekeeperOverview.ts, GatekeeperConstraintSummary.ts, etc.
│   ├── components/panels/
│   │   └── GatekeeperPanel.tsx  # NEW: tool-pane, mirrors FluxPanel.tsx + a drill-down list for violations
│   └── store/
│       └── index.ts            # MODIFIED: gatekeeper slice + refreshGatekeeper(), mirrors flux slice
└── tests/
    └── e2e/
        └── gatekeeper-panel.spec.ts  # NEW: 2-scenario Playwright spec (violations render + not-installed)
```

**Structure Decision**: Single existing Tauri project (Rust backend `src/` + React frontend
`ui/`). No new top-level directory. Every new file has a direct 1:1 counterpart already shipped
for the Flux panel, confirming this is a template-reuse feature, not a new subsystem.

## Complexity Tracking

*No violations — table omitted.*
