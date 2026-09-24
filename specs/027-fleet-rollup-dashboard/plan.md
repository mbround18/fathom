# Implementation Plan: Fleet Rollup Dashboard

**Branch**: `027-fleet-rollup-dashboard` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/027-fleet-rollup-dashboard/spec.md`

## Summary

Add a new, additive "Fleet Dashboard" view that shows a read-only rollup (node counts,
unhealthy-pod count, cluster version, Flux GitOps status if present) for every currently
*connected* kubeconfig context at once — no context switching required. This requires holding
more than one live `kube::Client` at a time, which the existing single-active-context model
(`src/backend/state.rs::ClusterContext`, unchanged since spec 012 P1) does not support today.
Per research.md R0, spec 012 explicitly deferred concurrent liveness rather than half-building
it, so this plan adds a small, isolated, opt-in-while-the-view-is-open polling subsystem
(`src/backend/fleet.rs`) with its own per-context `kube::Client`s and task handles, keyed by
context name — reusing spec 012's client-construction pattern and this codebase's existing
keyed-task-handle idiom (`NodeActionState`), but touching none of the ~50 existing
single-client resource modules.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace `Cargo.toml`), TypeScript (strict) for the frontend.

**Primary Dependencies**: `kube` (kube-rs) + `k8s-openapi` for cluster access, `tokio` for async
tasks/timers, existing `crossbeam-channel` backend event bus, `ts-rs` for protocol type
generation; frontend: React, Zustand, shadcn/ui, Tailwind, Vite.

**Storage**: N/A — no new persisted state. `session.contexts`/`contextStatuses` (already
runtime-only, spec 012) are the source of truth for which contexts are connected; the fleet
dashboard's per-context summaries are runtime-only and rebuilt on each poll.

**Testing**: `cargo test` (unit tests for `fleet.rs`'s poll-loop error mapping and lifecycle,
serde round-trip tests for new protocol types in `tests/protocol_serde.rs`), `pnpm -C ui test`
(Vitest for the new store slice/handler), one Playwright e2e spec (`ui/tests/e2e/`) against the
scripted fake backend per Constitution III.

**Target Platform**: Same as the rest of Fathom — Tauri v2 desktop, Tier 1 Ubuntu 26.04 /
Windows 11 / macOS 26 Tahoe.

**Project Type**: Desktop app (existing Tauri v2 + Rust backend / React frontend structure) —
no new project, this extends the existing single project.

**Performance Goals**: SC-001 (identify unhealthy clusters within 5s of opening the view),
SC-004 (each card no more than one 30s poll interval stale). Idle cost when the dashboard is
closed MUST remain zero (Constitution V) — no poller exists unless the view is open.

**Constraints**: Constitution V idle-CPU budget (<2% of one core when idle) — since fleet
pollers only run while the dashboard view is mounted, this is satisfied by construction when
the view is closed; while open, N contexts means N concurrent lightweight polls every 30s,
each bounded by a 10s timeout (research.md R3), which is a deliberate, scoped exception to
"one client" that the operator opts into by opening the view, not a background default.

**Scale/Scope**: Handful to a few dozen connected contexts (kubeconfig realistically has
single/low-double-digit entries); SC-002 targets comfortable display up to 10 without scrolling.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **I. Operator-First, Explicit Mutation** — PASS. This feature is view-only (FR-008); it adds
  no mutating actions. Clicking a card switches the active context via the *existing*,
  already-confirmed-free `SetContext` mechanism (FR-007) — not a new mutation.
- **II. One Contract, Backend Owns the Cluster** — PASS. All fleet polling lives in the Rust
  backend (`src/backend/fleet.rs`); the UI only reads new protocol types generated from Rust
  (`ts-rs`) via new commands/events in `src/protocol/v1/`. No cluster access from the frontend.
- **III. Tested at the Boundaries** — PASS (planned). New protocol variants get serde
  round-trip tests; the new store slice/handler gets unit tests; one Playwright e2e scenario
  drives the scripted fake backend per the feature brief.
- **IV. Density Scales With Width** — PASS (planned). The fleet card grid is designed
  mobile-first (single column, then a responsive grid at wider widths), consistent with
  existing dashboard patterns; no horizontal scroll.
- **V. Fast and Quiet** — CONDITIONAL PASS, justified in Complexity Tracking below: this is the
  one principle this feature puts real pressure on (N live clients + N recurring pollers is a
  new, real resource cost class for this codebase). Mitigated by making it strictly opt-in
  (only while the dashboard view is open), bounding each poll with a timeout, and using a
  30s interval (not an aggressive one) — but it is an honest, real addition to steady-state
  cost while the view is open, not a wash, so it is recorded below rather than asserted away.
- **VI. User Data Is Sacred** — PASS. No new persisted files; no new credential handling beyond
  reusing each context's existing kubeconfig entry, exactly as `context_status.rs`'s probes
  already do today.

## Project Structure

### Documentation (this feature)

```text
specs/027-fleet-rollup-dashboard/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/             # Phase 1 output
│   └── fleet-protocol.md
└── tasks.md               # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── fleet.rs                 # NEW: per-context poll-task pool + summary cache
│   ├── state.rs                 # + FleetState field on BackendState
│   └── cmd/
│       └── cluster.rs           # + StartFleetPolling/StopFleetPolling dispatch
├── protocol/v1/
│   ├── fleet.rs                 # NEW: FleetContextSummary, FleetHealthState, events/cmds
│   ├── cmd.rs                   # + StartFleetPolling / StopFleetPolling
│   └── event.rs                 # + FleetSummaryUpdated (SystemEvent variant)
└── ...

ui/src/
├── components/
│   └── fleet/
│       ├── FleetDashboardView.tsx   # NEW: the rollup grid
│       └── FleetContextCard.tsx     # NEW: one context's card
├── store/
│   ├── index.ts                  # + fleetSummaries slice, startFleetPolling/stop actions
│   └── handlers/
│       └── fleet.ts               # NEW: FleetSummaryUpdated handler
└── ...

ui/tests/e2e/
└── us1-fleet-dashboard.spec.ts     # NEW: hyper-focused two-context e2e scenario
```

**Structure Decision**: Extends the existing single Tauri+Rust project in place — no new crate,
package, or top-level directory. The backend gets one new module (`fleet.rs`) plus small,
additive touch points in `state.rs` and `cmd/cluster.rs`; the frontend gets one new view
directory plus a store slice, following the same file organization every prior spec
(001–015) already uses in this repo.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|--------------------------------------|
| N simultaneous live `kube::Client`s + N recurring poll tasks while the Fleet Dashboard is open (a real, new steady-state resource cost under Constitution V, scoped to when this specific view is mounted) | The feature's entire value (spec User Story 1) is seeing every connected context's health *without* switching the single active context, which is structurally impossible with the existing one-client model (research.md R0). | (a) Reusing the existing single-active `ClusterContext.client` and round-robin-switching it to fetch each context in turn — rejected: violates FR-004's "MUST NOT block or delay other contexts," would visibly disrupt whatever the operator is doing in the *actual* active context (contradicts Principle I's "no implicit changes to what's shown"), and cannot run in the background while the operator works elsewhere. (b) Making fleet polling always-on for the app's lifetime regardless of whether the dashboard is open — rejected as a strictly worse Constitution V violation with no offsetting benefit; the scoped, view-open-only design (research.md R2) is the smaller and more honest version of this same tradeoff. |

## Clarification update (2026-09-24)

Scope additions from spec Clarifications (FR-012 to FR-016): `restricted` protocol field, concurrency cap + staggered start, extended unhealthy-pod definition, data-recency on cards, dialog closes on card click. `Cargo.toml` gains a `tokio` dev-dependency with `test-util` for paused-time poller tests. See tasks.md Phase 7.
