# Implementation Plan: Velero Backup Pane

**Branch**: `022-velero-backup-pane` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/022-velero-backup-pane/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command; its definition describes the execution workflow.

## Summary

Add a read-only Velero tool-pane that detects the `velero.io` CRDs (Backup, Schedule, Restore,
BackupStorageLocation) on the connected cluster and, when present, shows backup/restore status,
per-schedule last-backup age, and storage-location health. This is a structural clone of the
existing Flux GitOps pane: a backend CRD-discovery scan (`src/backend/flux.rs` →
`src/backend/velero.rs`), a 20s background poll task plus an on-open manual refresh command
(`sync.rs::ensure_flux_poll` / `RefreshFlux` → `ensure_velero_poll` / `RefreshVelero`), one
aggregated `SystemEvent` overview type, and a frontend tool-pane
(`FluxPanel.tsx` → `VeleroPanel.tsx`) built on the same `IntegrationSummary` presentation
component. No new external dependency, no mutation commands, no new architecture.

## Technical Context

**Language/Version**: Rust (edition 2024, existing workspace toolchain) for the backend; TypeScript (strict) for the frontend.

**Primary Dependencies**: `kube-rs` (`Client`, `Api<DynamicObject>`, `CustomResourceDefinition` discovery — already a workspace dependency, used identically by `flux.rs`/`longhorn.rs`); `tokio` for the poll task; React + Zustand + shadcn/ui on the frontend (existing `IntegrationSummary` panel component, existing `@/protocol/types` generated bindings).

**Storage**: N/A — no persistence; the pane reflects live cluster state only, refreshed every 20s or on demand.

**Testing**: `cargo test` (unit tests for status/age classification and CRD-presence detection, an `#[ignore]`d live-cluster smoke test mirroring `flux.rs`'s), protocol round-trip tests in `src/protocol/v1/event.rs`/`cmd.rs`, `pnpm -C ui test` (Vitest) for panel rendering logic, and a new focused Playwright e2e spec (`ui/tests/e2e/velero-pane.spec.ts`) covering the completed/failed-backup rendering case and the not-installed empty state.

**Target Platform**: Same Tier 1 desktop targets as the rest of Fathom (Ubuntu, Windows, macOS) via the Tauri v2 shell — no platform-specific code.

**Project Type**: Desktop app (Rust backend + Tauri/React frontend), matching the existing single-repo structure.

**Performance Goals**: Background scan MUST complete well within the 20s poll interval for a cluster with typical Velero resource counts (tens of Backups/Restores); pane render MUST not block the UI thread (data arrives via the existing event bus, same as Flux).

**Constraints**: Read-only — MUST NOT expose any mutating command (no trigger-backup/restore, no delete). MUST NOT add a new crate dependency. MUST reuse the existing CRD-discovery pattern (dynamic objects, no compiled Velero API types) so it works across Velero versions without pinning a schema, matching Flux's "no invented architecture" instruction.

**Scale/Scope**: One new backend module, one new protocol overview type + one new `SystemEvent` variant + one new read-only `BackendCmd` (`RefreshVelero`), one new frontend panel + section entry, one new Playwright spec. No egui-side work (see Constitution Check: legacy UI receives no new features, only parity-preservation work).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. This feature adds zero mutating commands. There is nothing to confirm because there is nothing to mutate. Backend scan errors are surfaced verbatim via the existing `events::notify_error` path, exactly as Flux does.
- **II. One Contract, Backend Owns the Cluster**: PASS. All `velero.io` CRD access lives in the Rust backend (`src/backend/velero.rs`); the frontend only reads `VeleroOverview` via the versioned protocol and generated TS bindings, and sends `RefreshVelero` — no direct cluster access from the UI.
- **III. Tested at the Boundaries (NON-NEGOTIABLE)**: PASS, addressed in Testing above — protocol round-trip tests, backend unit tests for classification logic, a Playwright e2e spec for the two priority-1 user stories (rendering statuses/age; not-installed empty state), and `cargo clippy -D warnings` / `cargo fmt --check` / full frontend lint+typecheck+test gates before merge.
- **IV. Density Scales With Width**: PASS. The pane reuses `IntegrationSummary`, which already implements the mobile-first tile/row layout used by Flux and Hubble; no new layout code needed.
- **V. Fast and Quiet**: PASS. Uses the same 20s interval-ticker background task pattern as Flux/Hubble (not a continuous watch), so idle CPU cost is bounded and matches existing integrations. Scan only runs while a poll task is alive (started on cluster connect), matching `ensure_flux_poll`.
- **VI. User Data Is Sacred**: PASS. No new on-disk state, no credentials involved beyond the existing kubeconfig/RBAC context already used for every other resource.
- **Stack**: PASS. No new dependency; reuses `kube-rs` dynamic-object discovery already in the workspace. No egui work is added — per the constitution, the legacy egui UI is frozen at parity with the old feature set and slated for removal, so new integration panes (matching sibling specs 020/021/023/026/029) are Tauri/React-only.

No violations. Complexity Tracking table is not needed.

## Project Structure

### Documentation (this feature)

```text
specs/022-velero-backup-pane/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
│   └── velero-protocol.md
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── velero.rs            # NEW: CRD discovery + summarize (mirrors flux.rs)
│   ├── sync.rs               # MODIFIED: ensure_velero_poll + poll_velero_overview
│   ├── state.rs              # MODIFIED: velero_poll_task field on watch-tasks struct
│   └── cmd/
│       └── integrations.rs   # MODIFIED: RefreshVelero handler (mirrors refresh_flux)
├── protocol/
│   └── v1/
│       ├── velero.rs          # NEW: VeleroOverview, VeleroBackupSummary, VeleroScheduleSummary,
│       │                      #      VeleroRestoreSummary, VeleroStorageLocationSummary, status enums
│       ├── event.rs           # MODIFIED: SystemEvent::VeleroOverviewUpdated(VeleroOverview)
│       ├── cmd.rs             # MODIFIED: BackendCmdV1::RefreshVelero
│       ├── mutation.rs        # MODIFIED: classify RefreshVelero as non-mutating, section = integrations_velero()
│       └── route.rs           # MODIFIED: section_keys::integrations_velero()
└── sections/
    └── mod.rs                 # MODIFIED: register "Velero" section under the Integrations group

ui/
├── src/
│   ├── components/panels/
│   │   ├── VeleroPanel.tsx    # NEW: mirrors FluxPanel.tsx, built on IntegrationSummary
│   │   └── shell/PaneHost.tsx # MODIFIED: route tool === 'velero' to VeleroPanel
│   ├── store/index.ts         # MODIFIED: velero.overview state slice + refreshVelero() action
│   └── protocol/generated/    # REGENERATED: VeleroOverview.ts, VeleroBackupSummary.ts, etc.
└── tests/
    └── e2e/
        └── velero-pane.spec.ts # NEW: fixture-driven Playwright spec (2 scenarios per spec direction)
```

**Structure Decision**: Single project, existing Fathom layout (Rust backend `src/`, Tauri/React
frontend `ui/`). No new top-level directories. Every new file has a direct one-to-one Flux
counterpart, per the explicit "reuse the Flux pattern, don't invent a new architecture"
direction from the feature owner.

## Complexity Tracking

*No violations — table omitted.*
