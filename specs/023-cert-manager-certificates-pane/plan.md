# Implementation Plan: Certificates (cert-manager) Pane

**Branch**: `023-cert-manager-certificates-pane` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/023-cert-manager-certificates-pane/spec.md`

## Summary

Add a read-only Certificates tool-pane that detects cert-manager by its CRDs, lists Certificate
resources with expiry countdown and ready/renewal status, and lets the operator drill into a
Certificate's lifecycle chain (Certificate → CertificateRequest → Order → Challenge) to see which
stage is stuck and why. This mirrors the shipped Flux GitOps panel exactly: CRD-presence
detection + dynamic-object listing in the Rust backend (`src/backend/cert_manager.rs`, modeled on
`src/backend/flux.rs`), a 20s background poll task wired the same way as `ensure_flux_poll` in
`src/backend/sync.rs`, new protocol types/events/commands in `src/protocol/v1/`, and a new React
tool-pane (`ui/src/components/panels/CertManagerPanel.tsx`) plus a resource descriptor
(`ui/src/resources/certificate-resource.tsx`) so chain entries open in the existing Inspector. No
new external dependency: cert-manager types are read as `kube::core::DynamicObject`, the same
technique already used for Flux and Longhorn.

## Technical Context

**Language/Version**: Rust (edition 2024, existing workspace toolchain); TypeScript (strict) for
the frontend.

**Primary Dependencies**: `kube-rs` (`DynamicObject`, `Api`, `ListParams` — already a dependency,
no version bump), `k8s-openapi` (`CustomResourceDefinition`, already used by `flux.rs`), `ts-rs`
for generated protocol bindings, React/Zustand/shadcn on the frontend (all existing).

**Storage**: N/A — no persisted state; cert-manager overview is scanned live from the cluster on
each poll tick, same as Flux.

**Testing**: `cargo test` (protocol serialization round-trips, chain-correlation/expiry-countdown
unit tests), Vitest (store/selector and countdown-formatting unit tests), Playwright
(`ui/tests/e2e/`) end-to-end test against a scripted fake backend.

**Target Platform**: Same as the rest of Fathom — Tauri v2 desktop shell, Tier 1 Ubuntu 26 /
Windows 11 / macOS 26 (Constitution §Supported Platforms). No platform-specific work.

**Project Type**: Desktop app (single Tauri project — Rust backend + `ui/` React frontend), no new
project structure.

**Performance Goals**: Same envelope as the Flux panel — background scan every 20s, input handled
under 100ms even with hundreds of Certificates listed; idle CPU budget unaffected (poll task is a
lightweight interval, not a busy loop).

**Constraints**: Read-only — no command may create, edit, delete, or trigger reissuance of any
cert-manager resource (spec FR-009). Must not introduce a new crate. Must degrade honestly (not
silently) when cert-manager CRDs are absent or when chain resources have been garbage-collected.

**Scale/Scope**: One new backend module, ~4 new protocol types, one new `BackendCmd` refresh
variant + one `SelectCertificateChain`-style detail command, one new poll task, one new frontend
panel + one resource descriptor, one Playwright spec. Comparable in size to the Flux panel
feature.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. The feature is read-only end-to-end (FR-009); no
  confirmation flow is needed because there is nothing to confirm. Backend scan errors surface via
  the existing `events::notify_error` path verbatim, matching Flux's `poll_flux_overview`.
- **II. One Contract, Backend Owns the Cluster**: PASS. All CRD discovery/listing lives in
  `src/backend/cert_manager.rs`; the UI only sees generated protocol types
  (`CertManagerOverview`, `CertificateSummary`, `CertificateChain`, etc.) via `ui/src/protocol/types`.
  No direct cluster or `~/.fathom/` access from the UI.
- **III. Tested at the Boundaries**: PASS (planned). Protocol round-trip tests for every new
  command/event variant (mirrors `select_flux_resource_roundtrip` in `cmd.rs`), a scripted-backend
  Playwright e2e test covering the three user stories, and unit tests for the "stuck stage"
  classification and expiry-countdown formatting (pure functions, easy to unit test on both sides).
- **IV. Density Scales With Width**: PASS. Reuses `IntegrationSummary`/tool-pane conventions and
  the existing Inspector for the chain drill-down rather than inventing a new full-screen layout;
  no horizontal scroll, table columns follow existing priority/hide rules.
- **V. Fast and Quiet**: PASS. 20s poll interval, identical to Flux/Hubble; no watches added (a
  poll is sufficient for a pane whose own success criteria tolerate "live-update latency window
  same as Flux", SC-004). Idle CPU unaffected.
- **VI. User Data Is Sacred**: PASS. No new on-disk state. RBAC scoping is inherited automatically
  because all reads go through the operator's existing `kube::Client` (same credentials/context as
  every other pane).

No violations — Complexity Tracking table is not needed.

## Project Structure

### Documentation (this feature)

```text
specs/023-cert-manager-certificates-pane/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── protocol.md      # Phase 1 output: new BackendCmd/BackendEvent/type contracts
└── tasks.md             # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── cert_manager.rs         # NEW — CRD discovery + summarize + chain correlation (models flux.rs)
│   ├── sync.rs                 # MODIFIED — ensure_cert_manager_poll + poll_cert_manager_overview
│   ├── state.rs                # MODIFIED — cert_manager_poll_task handle on WatchTasks
│   └── cmd/
│       └── integrations.rs     # MODIFIED — RefreshCertManager + SelectCertificateChain handlers
├── protocol/v1/
│   ├── cert_manager.rs         # NEW — CertManagerOverview, CertificateSummary, CertificateChain, ChainStage types
│   ├── cmd.rs                  # MODIFIED — RefreshCertManager, SelectCertificateChain commands
│   ├── event.rs                # MODIFIED — CertManagerOverviewUpdated, CertificateChainUpdated events
│   ├── mutation.rs             # MODIFIED — classify new commands as non-mutating + section routing
│   ├── route.rs                # MODIFIED — section_keys::integrations_cert_manager(), certificate_chain()
│   └── mod.rs                  # MODIFIED — re-export cert_manager module

ui/src/
├── protocol/generated/          # MODIFIED (auto) — ts-rs bindings for new types
├── components/panels/
│   └── CertManagerPanel.tsx     # NEW — tool-pane (list + expiry countdown + chain drill-down)
├── resources/
│   └── certificate-resource.tsx # NEW — ResourceDescriptor so chain entries open in the Inspector
├── store/
│   ├── index.ts                 # MODIFIED — certManager: { overview?, chain? } state slice
│   └── handlers/system.ts       # MODIFIED — ingest CertManagerOverviewUpdated/CertificateChainUpdated
└── tests/e2e/
    └── cert-manager-pane.spec.ts # NEW — Playwright scripted-backend e2e test
```

**Structure Decision**: Single Tauri project, no new top-level directories. Every new file sits
alongside its direct Flux-panel analogue (`src/backend/cert_manager.rs` next to `flux.rs`,
`CertManagerPanel.tsx` next to `FluxPanel.tsx`, etc.) so the pattern is trivially discoverable by
future maintainers.

## Complexity Tracking

*No violations — table intentionally omitted.*
