# Implementation Plan: Applications View & Rich Interlinking

**Branch**: `004-applications-interlinking` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-applications-interlinking/spec.md`

## Summary

Add a new read-only "Applications" nav section that synthesizes logical applications from
already-collected Helm-annotation and Flux `Kustomization`/`HelmRelease` ownership data (no new
external integration, no Helm-release-storage parsing), with a list + Inspector following the
exact pattern every other resource kind already uses. Alongside it, build one generic
related-resources resolver, keyed by kind + ownerReferences/selector, and wire it into every
inspector with a natural relationship (Application, Deployment/StatefulSet/DaemonSet/ReplicaSet,
Pod, Node, Service, PVC) as clickable links, with a breadcrumb/back-stack extending the Inspector
pane's existing `target`/`source` state. Zero new mutating commands; zero new dependencies.

## Technical Context

**Language/Version**: Rust (edition 2024, backend), TypeScript strict (frontend) — unchanged, no
new language/toolchain needs.

**Primary Dependencies**: `kube` 2.0.1 (`runtime`, `derive`, `ws` features, already present),
`tokio` 1.49 (already present); frontend: `react` 19, `zustand` 5 (already present). No additions
(research.md R7).

**Storage**: N/A — no new persisted `~/.fathom/` state; the drill-down/back-stack is
session-transient UI state only (data-model.md).

**Testing**: `cargo test` (protocol round-trip tests, resolver/attribution unit tests), Vitest
(unit), Playwright against the scripted fake backend (E2E per user story) — all existing harnesses,
no new tooling.

**Target Platform**: Ubuntu 26.04, Windows 11, macOS 26 (Tier 1, per constitution) — unaffected;
this feature adds no platform-specific code.

**Project Type**: Desktop app (Tauri v2 + Rust backend), existing single-repo structure.

**Performance Goals**: No new list exceeds existing Fathom list-rendering budgets (Constitution
Principle V: input handled <100ms with 10k objects); the related-resources list for an
application with hundreds of pods must stay scrollable/virtualized like every other large list in
Fathom (FR-017), reusing the existing virtualization already used for pod lists.

**Constraints**: Read-only — no mutating command variants, no `mutation.rs` changes (spec.md
FR-018). No Helm release-storage reads (FR-019). No topology graph (FR-020). No Argo CD (FR-021).

**Scale/Scope**: One new protocol module (`applications.rs`), 3 new commands, 4 new events/types,
one new backend synthesis module, one new generic resolver module, one new frontend nav section +
descriptor, breadcrumb/back-stack state added to the existing Inspector pane state.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **I. Operator-First, Explicit Mutation**: Trivially satisfied — this feature is 100% read/navigate.
  No mutating action is added, so there is nothing to gate behind confirmation. Existing
  mutations reachable once drilled into a resource (e.g. deleting a pod from its inspector) are
  unchanged and already gated by their own existing confirmation flow. **PASS**.
- **II. One Contract, Backend Owns the Cluster**: All new read logic (application synthesis,
  Helm/Flux attribution, the related-resources resolver) lives in the Rust backend
  (`src/backend/applications.rs`, `src/backend/related.rs`); the frontend only sends the new
  `BackendCmdV1` variants and renders the new event payloads via generated TS bindings, same as
  every existing kind. **PASS**.
- **III. Tested at the Boundaries**: Every new command/event variant gets a round-trip test in
  `tests/protocol_serde.rs` (contracts/protocol.md); each user story gets a scripted-fake-backend
  E2E test (quickstart.md "Automated coverage"); the resolver and attribution logic get unit
  tests. **PASS** (tasks.md will enumerate the concrete test tasks).
- **IV. Density Scales With Width**: The Applications list and Inspector reuse the existing
  list-pane/Inspector components and column-priority system unchanged — no new layout primitive
  is introduced. **PASS**.
- **V. Fast and Quiet**: No new watch/poll loop (research.md R4) — applications are computed
  on-demand from an explicit `RefreshApplications`, consistent with the majority existing pattern.
  Related-resources lookups prefer already-fetched data over new API calls (research.md R5).
  **PASS**.
- **VI. User Data Is Sacred**: No new `~/.fathom/` persisted state; the back-stack is
  session-transient (data-model.md). **PASS**.

No violations. Complexity Tracking table omitted (nothing to justify).

## Project Structure

### Documentation (this feature)

```text
specs/004-applications-interlinking/
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
│   ├── applications.rs        # NEW: ApplicationSummary, ApplicationDetails, ApplicationManagedBy,
│   │                            RelatedResourceRef
│   ├── cmd.rs                 # + RefreshApplications, SelectApplication, RefreshRelatedResources
│   ├── event.rs                # + ApplicationsUpdated, ApplicationDetailsUpdated,
│   │                            ApplicationMetricsUpdated, RelatedResourcesUpdated
│   └── route.rs                 # + "applications" SectionKey (mirrors "custom-resources")
├── backend/
│   ├── applications.rs         # NEW: Helm/Flux/label-heuristic synthesis (research.md R1–R4)
│   ├── related.rs              # NEW: generic related-resources resolver (research.md R5,
│   │                            data-model.md relationship vocabulary)
│   ├── flux.rs                 # + read status.inventory on Kustomization (research.md R2)
│   └── cmd.rs / cmd/           # + command dispatch wiring for the 3 new commands (ReadOnly)
tests/
└── protocol_serde.rs           # + round-trip tests for every new variant/type

ui/src/
├── resources/
│   ├── applications.tsx        # NEW: ResourceDescriptor<ApplicationSummary, ApplicationDetails>
│   ├── nav.ts                  # + 'applications' NAV_GROUPS entry
│   └── types.ts                # (no change — ObjectRef/DetailBlock already sufficient)
├── store/
│   └── index.ts                # + PaneState.history: ObjectRef[]; openInspector push behavior;
│                                  goBack(paneId) action
├── components/
│   ├── Inspector.tsx            # + breadcrumb strip render; related-resources block renders
│   │                             clickable entries instead of plain text
│   └── panels/
│       └── RelatedResourcesTab.tsx  # NEW (or a DetailSection, depending on Inspector's existing
│                                      tab convention — resolved during implementation, not a
│                                      structural decision this plan needs to pre-commit to)
├── protocol/generated/          # ts-rs regenerates ApplicationSummary.ts, ApplicationDetails.ts,
│                                  RelatedResourceRef.ts, ApplicationManagedBy.ts, BackendCmdV1.ts,
│                                  ResourceListEvent.ts, ResourceReadEvent.ts, SectionKey.ts
```

**Structure Decision**: Single-project Tauri app (existing layout, unchanged) — `src/protocol/v1/`
for the shared contract, `src/backend/` for the Rust implementation, `ui/src/` for the frontend.
No new top-level directory. Every new file follows the existing one-module-per-kind convention
(`src/protocol/v1/flux.rs` / `src/backend/flux.rs` as the closest precedent for a
synthesized-rather-than-single-CRD-backed feature).

## Complexity Tracking

*No entries — Constitution Check passed with no violations.*
