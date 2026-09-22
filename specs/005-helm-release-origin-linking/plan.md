# Implementation Plan: Helm Release View & Origin-Context Linking

**Branch**: `005-helm-release-origin-linking` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/005-helm-release-origin-linking/spec.md`

## Summary

Add a new read-only "Helm Releases" nav section that reads Helm 3's own release-storage
Secrets/ConfigMaps directly (base64+gzip+JSON decode in Rust — no `helm` binary shellout), showing
chart/version/revision/status plus user-supplied vs. effective values, the rendered manifest, and
revision history. Extend 004's generic related-resources resolver with a new `"reconciled-by"`
relation that, for Flux-managed releases, links to the owning Flux `HelmRelease`/`Kustomization`
and opens a minimal live health view (Ready/Failed/Reconciling/Suspended, message, revision,
suspended, last_transition) via the existing Flux discovery code — no new poll loop, no new
Kubernetes watch. Zero new mutating commands; one new Rust dependency (`flate2`, for gzip decode).

## Technical Context

**Language/Version**: Rust (edition 2024, backend), TypeScript strict (frontend) — unchanged.

**Primary Dependencies**: `kube` 2.0.1, `base64` 0.22.1 (already present); **new**: `flate2 = "1"`
(gzip decode for Helm's release-storage payload — research.md R2, the one dependency gap found).
Frontend: `react` 19, `zustand` 5 (already present, no additions — research.md R7).

**Storage**: N/A — no new persisted `~/.fathom/` state; all release data is read live from the
cluster's own `Secret`/`ConfigMap` objects each refresh (data-model.md).

**Testing**: `cargo test` (protocol round-trip tests, `decode_release_payload` fixture-blob unit
tests requiring no live cluster, `related_for_helm_release` unit tests), Vitest (unit), Playwright
against the scripted fake backend (E2E per user story) — all existing harnesses.

**Target Platform**: Ubuntu 26.04, Windows 11, macOS 26 (Tier 1) — unaffected; `flate2`'s default
`miniz_oxide` backend has no system zlib/protoc prerequisite on any of the three (research.md R2).

**Project Type**: Desktop app (Tauri v2 + Rust backend), existing single-repo structure.

**Performance Goals**: Release list and revision history stay within existing list-rendering
budgets (Constitution Principle V); revision-history decode only fully parses the currently
selected revision's manifest/values, not every historical revision's payload (data-model.md), to
keep a single "select release" read cheap even for a release with dozens of prior revisions.

**Constraints**: Read-only — no `mutation.rs` changes (FR-015). No `helm` CLI shellout (FR-001). No
revision diffing (Assumption). No Argo CD reads — only the origin-reference shape is made generic
enough for a future Argo source to plug in (data-model.md `GitOpsController`).

**Scale/Scope**: One new protocol module (`helm.rs`), 3 new commands, 4 new event variants, one new
backend module (`helm.rs`) plus one new match arm in the existing `related.rs` resolver, two new
frontend descriptors (`helm.tsx`, `flux-resource.tsx`), one new nav entry.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **I. Operator-First, Explicit Mutation**: Trivially satisfied — 100% read/navigate, no mutating
  command added (FR-015 explicit non-goal). **PASS**.
- **II. One Contract, Backend Owns the Cluster**: All release-storage decoding (base64/gzip/JSON)
  happens in `src/backend/helm.rs`; the frontend never receives raw Secret/ConfigMap bytes, only
  decoded `HelmReleaseSummary`/`HelmReleaseDetails` (research.md R1). Flux health lookups reuse the
  existing backend-owned `flux.rs` discovery code (research.md R3) rather than trusting a
  client-cached snapshot. **PASS**.
- **III. Tested at the Boundaries**: Every new command/event variant gets a round-trip test
  (contracts/protocol.md); `decode_release_payload` gets fixture-blob unit tests needing no live
  cluster; `related_for_helm_release` gets unit tests for both the "owns" and "reconciled-by" match
  arms; each user story gets a scripted-fake-backend E2E test (quickstart.md). **PASS**.
- **IV. Density Scales With Width**: Reuses the existing list-pane/Inspector components, column-
  priority system, and the existing YAML viewer from 003 unchanged — no new layout primitive.
  **PASS**.
- **V. Fast and Quiet**: No new watch/poll loop — releases are computed on-demand via
  `RefreshHelmReleases`/`SelectHelmRelease`; the Flux origin lookup is a single on-demand read
  (`SelectFluxResource`) triggered only when the operator opens an origin link, not a background
  poll (research.md R3). **PASS**.
- **VI. User Data Is Sacred**: No new `~/.fathom/` persisted state. **PASS**.

No violations. Complexity Tracking table omitted (nothing to justify).

## Project Structure

### Documentation (this feature)

```text
specs/005-helm-release-origin-linking/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── protocol.md       # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── helm.rs                 # NEW: HelmReleaseSummary, HelmReleaseDetails, HelmReleaseStatus,
│   │                              HelmRevisionSummary, GitOpsOriginRef, GitOpsController
│   ├── cmd.rs                  # + RefreshHelmReleases, SelectHelmRelease, SelectFluxResource
│   ├── event.rs                 # + HelmReleasesUpdated, HelmReleaseDetailsUpdated,
│   │                              FluxResourceDetailsUpdated, FluxResourceNotFound
│   └── route.rs                  # + section_keys::helm_releases(), section_keys::flux_resource()
├── backend/
│   ├── helm.rs                  # NEW: release-storage Secret/ConfigMap reader + base64/gzip/JSON
│   │                               decode (research.md R1), fixture-testable decoder
│   ├── related.rs                # + related_for_helm_release: "owns" (label-based, research.md
│   │                               R6) + "reconciled-by" (research.md R3)
│   └── cmd/                       # + dispatch wiring for the 3 new commands (all read-only)
Cargo.toml                         # + flate2 = "1" (research.md R2)
tests/
└── protocol_serde.rs              # + round-trip tests for every new variant/type

ui/src/
├── resources/
│   ├── helm.tsx                  # NEW: ResourceDescriptor<HelmReleaseSummary, HelmReleaseDetails>
│   ├── flux-resource.tsx          # NEW: minimal ResourceDescriptor<FluxResourceSummary,
│   │                                FluxResourceSummary> for the origin-link detail target
│   ├── nav.ts                     # + 'helm-releases' NAV_GROUPS entry
│   └── index.ts                   # + both new descriptors appended to ALL_DESCRIPTORS
├── protocol/generated/             # ts-rs regenerates HelmReleaseSummary.ts, HelmReleaseDetails.ts,
│                                     HelmReleaseStatus.ts, HelmRevisionSummary.ts,
│                                     GitOpsOriginRef.ts, GitOpsController.ts, BackendCmdV1.ts,
│                                     ResourceListEvent.ts, ResourceReadEvent.ts, SectionKey.ts
```

**Structure Decision**: Single-project Tauri app (existing layout, unchanged) — `src/protocol/v1/`
for the shared contract, `src/backend/` for the Rust implementation, `ui/src/` for the frontend. No
new top-level directory. `helm.rs` (protocol and backend) follows the same one-module-per-kind
convention `applications.rs` established in 004; `related.rs` and the `RelatedResourceRef`
mechanism are extended in place rather than duplicated, matching 004's own precedent for reusing
existing patterns over inventing new ones.

## Complexity Tracking

*No entries — Constitution Check passed with no violations.*
