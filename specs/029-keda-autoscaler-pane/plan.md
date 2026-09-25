# Implementation Plan: KEDA Autoscaler Pane

**Branch**: `029-keda-autoscaler-pane` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/029-keda-autoscaler-pane/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command; its definition describes the execution workflow.

## Summary

Add a read-only "Autoscalers" tool-pane that detects KEDA via its `keda.sh` CRDs
(`ScaledObject`, `ScaledJob`, `TriggerAuthentication`, `ClusterTriggerAuthentication`), lists
every `ScaledObject`/`ScaledJob` with its current replica count and min/max bounds, and an
Inspector that shows each configured trigger's type and metadata plus a plain-language
explanation of why replicas sit where they do relative to min/max. When Fathom's existing
Prometheus metrics integration is configured and reachable, a `prometheus`-type trigger's live
current value is fetched (by reusing the already-generic `PrometheusClient::query_instant`
against the trigger's own `serverAddress`/`query` fields) and shown next to its threshold; every
other trigger type falls back to trigger-type + metadata display only, per the spec's explicit
non-hard-dependency and non-goal on custom trigger UI. The feature reuses three already-shipped
patterns end to end: `flux.rs`'s CRD-detect-and-summarize shape for the backend module, the
generic `related.rs` relationship resolver for Autoscaler↔Pod↔target-workload links (also
surfaced from the Pod inspector and referenced from the pod-lifecycle diagnostics view), and the
Flux dashboard-poll + single-resource-select command/event pair for backend↔frontend wiring.

## Technical Context

**Language/Version**: Rust 2021 (backend, `kube-rs`), TypeScript 5 / React 18 (frontend, `ui/`)

**Primary Dependencies**: `kube` (CRD discovery + `DynamicObject` listing, mirroring `flux.rs`),
existing `PrometheusClient`/`MetricsProviderTrait` (`src/backend/metrics_provider.rs`), existing
protocol codegen (`ts-rs`) from `src/protocol/v1/`, Zustand store (`ui/src/store/`), shadcn/ui
components already used by `FluxPanel.tsx`

**Storage**: N/A — no new persisted state; reads live cluster objects only, same as Flux/Helm
integrations

**Testing**: `cargo test` (protocol serde round-trip tests per constitution III, unit tests for
scale-reasoning derivation), Playwright e2e in `ui/tests/e2e/` against a scripted fake backend

**Target Platform**: Existing Tier 1 desktop targets (Ubuntu 26.04, Windows 11, macOS 26) — no
new platform-specific behavior

**Project Type**: Desktop app (Tauri v2): Rust backend + React/TS frontend, single repo

**Performance Goals**: Consistent with constitution V — Autoscalers list/Inspector updates must
not block input handling; large lists (200+ ScaledObjects) must remain scrollable/virtualized,
matching existing resource-list behavior

**Constraints**: Read-only (no mutation); metrics-value fetch MUST be strictly optional and
non-blocking (spec FR-012/FR-013); TriggerAuthentication secret values MUST never be resolved or
displayed (constitution VI)

**Scale/Scope**: One new backend module (`src/backend/keda.rs`), new protocol types in
`src/protocol/v1/keda.rs`, two new commands + up to three new events, two new `related.rs`
match arms, one new Zustand slice + panel component + resource inspector, one new e2e spec

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation** — PASS. This feature is entirely read-only (spec
  FR-019); no mutating command is introduced, so no confirmation-gate design is needed.
- **II. One Contract, Backend Owns the Cluster** — PASS. All `keda.sh` CRD discovery/reads and any
  Prometheus query happen in the Rust backend; the UI only receives protocol types
  (`KedaOverview`, `KedaResourceDetails`) via new `BackendCmd`/`SystemEvent` variants in
  `src/protocol/v1/`, generated for TS via the existing `ts-rs` pipeline. No new protocol version
  bump is needed (additive-only: new enum variants and structs).
- **III. Tested at the Boundaries** — PASS (planned). New command/event variants get serde
  round-trip tests (mirroring existing `cmd.rs`/`event.rs` test patterns); the scale-reasoning
  derivation (min/max comparison, paused/zero-state detection) gets Rust unit tests; a Playwright
  e2e spec drives the pane against a scripted fake backend per constitution requirement.
- **IV. Density Scales With Width** — PASS. Reuses the existing list-pane + Inspector shell
  (`FluxPanel.tsx`-equivalent), which already implements responsive column priority; no new
  layout primitive introduced.
- **V. Fast and Quiet** — PASS. Follows Flux's 20s background-poll cadence (no new streaming
  loop); optional Prometheus query is only performed on-demand when a `ScaledObject`/`ScaledJob`
  Inspector is open, not for every list row, avoiding N+1 query fan-out on the list view.
- **VI. User Data Is Sacred** — PASS. `TriggerAuthentication`/`ClusterTriggerAuthentication` are
  referenced by name only; no secret/env/configMap resolution is added (research.md R6).

No violations requiring the Complexity Tracking table.

## Project Structure

### Documentation (this feature)

```text
specs/029-keda-autoscaler-pane/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/
│   └── protocol.md      # Phase 1 output (/speckit-plan command)
└── tasks.md              # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── keda.rs                 # NEW — CRD detection, ScaledObject/ScaledJob summarization,
│   │                            #   scale-reasoning derivation, optional Prometheus trigger-value fetch
│   ├── related.rs               # MODIFIED — add "ScaledObject"/"ScaledJob" arms; extend Pod arm
│   ├── sync.rs                  # MODIFIED — add ensure_keda_poll/poll_keda_overview (mirrors Flux)
│   ├── cmd/
│   │   └── integrations.rs      # MODIFIED — add refresh_keda, select_scaled_object/job handlers
│   └── metrics_provider.rs      # UNCHANGED — PrometheusClient::query_instant reused as-is
├── protocol/v1/
│   ├── keda.rs                  # NEW — KedaOverview, KedaResourceSummary, KedaTrigger,
│   │                            #   ScaleReasoning, TriggerAuthRef types
│   ├── cmd.rs                   # MODIFIED — RefreshKeda, SelectScaledObject, SelectScaledJob
│   └── event.rs                 # MODIFIED — KedaOverviewUpdated, KedaResourceDetailsUpdated,
│                                 #   KedaResourceNotFound
└── (generated bindings)         # Regenerated via existing ts-rs export

ui/
├── src/
│   ├── store/
│   │   ├── index.ts              # MODIFIED — `keda` slice + refreshKeda()/selectKedaResource()
│   │   └── handlers/system.ts    # MODIFIED — handle Keda* events
│   ├── components/panels/
│   │   └── KedaPanel.tsx         # NEW — list pane, mirrors FluxPanel.tsx
│   └── resources/
│       └── keda.tsx              # NEW — Inspector rendering (triggers, reasoning, related pods)
└── tests/e2e/
    └── keda-autoscaler-pane.spec.ts   # NEW — hyper-focused e2e (see quickstart.md)
```

**Structure Decision**: Single Tauri desktop-app repo (existing layout). This feature is additive
within the existing `src/backend/`, `src/protocol/v1/`, and `ui/src/` trees — no new top-level
project or directory is introduced, matching every prior spec (001–015) in this repo.

## Complexity Tracking

*No constitution violations — table intentionally omitted.*
