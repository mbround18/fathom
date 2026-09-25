# Implementation Plan: Cluster Sanitizer / Linter

**Branch**: `017-cluster-linter` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/017-cluster-linter/spec.md`

## Summary

A new "Linter" tool pane that runs a fixed set of eight read-only static-analysis rules (resource
requests/limits, probes, deprecated API versions, unused ConfigMaps/Secrets/ServiceAccounts, naked
pods, RBAC wildcard grants, missing PodDisruptionBudgets) purely over data already present in the
Zustand store's `lists` cache — the same rows already fetched for existing list panes (Pods,
Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, ConfigMaps, Secrets,
ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings, PodDisruptionBudgets). No
new backend command or cluster read is introduced; the only backend touch is adding a `Linter`
variant to the existing `ToolKind` enum (`src/protocol/v1/ui_config.rs`) so the pane can be docked
and its layout persisted like `Topology`/`Flux`/`Hubble` — mirroring exactly how spec 007 added
`Topology`. Findings recompute reactively whenever `store.lists` changes (the store already updates
live from backend watch events), and clicking a finding calls the existing
`openInspector(ref: ObjectRef, from?, push?)` store action, exactly as `TopologyCanvas.tsx` already
does for node clicks.

## Technical Context

**Language/Version**: Rust 2024 edition (one additive enum variant only); TypeScript 5 strict
(frontend, all new rule-evaluation logic)

**Primary Dependencies**: React 19, Zustand 5, existing `@/store`, `@/resources`,
`@/protocol/commands`, `@/protocol/generated` (regenerated `ToolKind` after the Rust enum change,
via the existing `ts-rs` codegen build step — same mechanism spec 007 used for `Topology`). No new
third-party dependency is required (unlike spec 007's `d3-force`): rule evaluation is plain
data-transform logic (filters/maps/joins over already-typed arrays), not a layout or rendering
problem. Per research.md R2, six sections (ConfigMaps, Secrets, PodDisruptionBudgets, Jobs,
CronJobs, ReplicaSets) need their already-existing-but-unused `RefreshX` backend commands wired
into `ui/src/protocol/commands.ts`'s `SECTION`/`REFRESH` tables — zero backend/protocol changes,
pure frontend wiring of dormant capability. Per research.md R3, ServiceAccounts have no backend
capability at all; the "unused ServiceAccounts" rule ships as an always-skipped rule using the
spec's own FR-010 graceful-degradation contract rather than adding new cluster-read capability.

**Storage**: N/A (no persistence beyond the existing `PaneState`/dock layout persistence that
`ToolKind` already participates in; findings are recomputed, never stored)

**Testing**: `cargo test` (round-trip serde test for the new `ToolKind::Linter` variant, following
the existing pattern for `ToolKind::Topology`), Vitest (unit tests per rule in the pure
`ui/src/linter/rules/*.ts` modules — each rule gets fixture-driven positive/negative cases),
Playwright (one focused e2e spec, `ui/tests/e2e/us1-linter-findings.spec.ts`, against the fake
backend: seed a known-bad Pod fixture, open the Linter pane, assert a finding appears, click it,
assert the Inspector opens on that Pod)

**Target Platform**: Desktop (Tauri v2, Tier 1 platforms), mobile-width web layout per Principle IV

**Project Type**: Desktop app (existing Tauri + React monorepo); frontend-only feature, plus one
additive backend enum variant

**Performance Goals**: Recompute all eight rules over a 10,000-object cache under the existing
100ms input-handling budget (Principle V); no additional idle CPU when the pane isn't mounted
(rules only run while the Linter pane is visible, same lifecycle pattern as `TopologyPanel`)

**Constraints**: Zero new Kubernetes API reads (FR-002); zero mutation surface (FR-003); fixed rule
set only, no rule-authoring UI (FR-012); no historical trend storage (FR-013); rules must degrade
gracefully (report "skipped", not false negatives/positives) when a dependent resource kind isn't
present in the current cache (FR-010)

**Scale/Scope**: One new tool pane, one new pure-TS rule-evaluation module (8 rules), one additive
backend enum variant, no other protocol changes

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (see below).*

- **I. Operator-First, Explicit Mutation**: PASS. The Linter pane exposes no mutating action
  whatsoever (FR-003); it only reads from the store and calls the existing read-only
  `openInspector`. Nothing here is a "wizard that hides objects" — every finding names the exact
  offending resource.
- **II. One Contract, Backend Owns the Cluster**: PASS. No new Kubernetes API access is added
  anywhere. The only backend change is an additive `ToolKind::Linter` enum variant in
  `src/protocol/v1/ui_config.rs` (pure UI-layout/persistence metadata, not a cluster read), which
  is regenerated into `ui/src/protocol/generated/ToolKind.ts` via the existing `ts-rs` build step —
  the same non-breaking, additive pattern spec 007 used for `ToolKind::Topology`. All eight rules
  read exclusively from `store.lists`. Six sections needed by the rules had no frontend consumer
  yet despite full backend/protocol support (research.md R2); wiring their already-existing
  `RefreshX` commands into the frontend's generic `refreshCmd` fallback table is not a new cluster
  read, just a new caller of dormant backend capability. ServiceAccounts have no backend capability
  at all; that one rule ships permanently "skipped" per FR-010 rather than adding new capability
  (research.md R3). The frontend never talks to the cluster directly.
- **III. Tested at the Boundaries**: PASS (planned). New enum variant gets a serde round-trip test.
  Each of the 8 rules is pure logic and gets unit tests (this is exactly the "UI logic that decides
  what the operator sees" category Principle III calls out). One focused Playwright e2e spec covers
  the pane-open -> finding -> inspector flow end to end against the fake backend.
- **IV. Density Scales With Width**: PASS (planned). The Linter pane is a tool pane like
  `topology`/`flux`/`hubble`, participating in the same responsive slot/dock/stack layout; it must
  stay usable and non-horizontally-scrolling at phone width, with severity/category filters
  reachable by touch.
- **V. Fast and Quiet**: PASS (planned). Rule evaluation is a pure, synchronous pass over already-
  in-memory arrays (no network, no polling); it only runs while the pane is mounted and only
  recomputes on actual `store.lists` changes (subscribed via Zustand selector, not a timer), holding
  the idle-CPU budget.
- **VI. User Data Is Sacred**: PASS (N/A). No `~/.fathom/` state read or written; no secrets are
  displayed (the "unused Secret" rule flags a Secret's existence/reference status only, never its
  contents).

No violations; Complexity Tracking table is empty.

## Project Structure

### Documentation (this feature)

```text
specs/017-cluster-linter/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── linter-rules-module.md
└── tasks.md
```

### Source Code (repository root)

```text
src/protocol/v1/
└── ui_config.rs                          # edit: ToolKind gains `Linter` variant (additive)

ui/src/
├── protocol/commands.ts                  # edit: SECTION + REFRESH gain 6 dormant sections
│                                          # (configmaps, secrets, poddisruptionbudgets, jobs,
│                                          # cronjobs, replicasets) — see research.md R2
├── components/
│   ├── panels/
│   │   └── LinterPanel.tsx               # new: tool-pane body, mirrors TopologyPanel.tsx
│   └── shell/PaneHost.tsx                # edit: dispatch pane.tool === 'linter'
├── linter/                               # new module: pure, unit-testable, no React coupling
│   ├── types.ts                          # Finding, Severity, Category, LintReport, RuleId
│   ├── evaluate.ts                       # store.lists -> LintReport (runs all rules, aggregates)
│   ├── rules/
│   │   ├── resourceLimits.ts             # missing requests/limits
│   │   ├── probes.ts                     # missing liveness/readiness/startup
│   │   ├── deprecatedApi.ts              # deprecated/removed apiVersion+kind table
│   │   ├── unusedConfig.ts               # unreferenced ConfigMaps/Secrets
│   │   ├── unusedServiceAccount.ts       # always-skipped in v1 (research.md R3)
│   │   ├── nakedPod.ts                   # pods with no ownerReferences
│   │   ├── rbacWildcard.ts               # wildcard verbs/resources in Role/ClusterRole
│   │   └── missingPdb.ts                 # multi-replica workloads without a matching PDB
│   └── deprecatedApiTable.ts             # static ships-with-the-app deprecation data
└── store/index.ts                        # edit: newPane/claimPane recognize 'linter' ToolKind

ui/tests/unit/linter/                     # Vitest: one fixture-driven spec per rule + evaluate.ts
ui/tests/e2e/
└── us1-linter-findings.spec.ts           # open pane -> finding appears -> click -> inspector opens

docs/MVP.md                               # status row update after implementation
```

**Structure Decision**: Frontend-only rule engine inside the existing `ui/` React app, following
spec 007's precedent exactly. The single backend touch is the additive `ToolKind::Linter` enum
variant needed for dock/layout persistence (no new command, no new cluster read). New pure-TS rule
logic lives outside `components/` (like `ui/src/topology/`) so each rule can be unit-tested in
isolation without mounting React.

## Complexity Tracking

*No entries — no constitutional violations.*
