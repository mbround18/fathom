# Implementation Plan: Simultaneous Multi-Context Federation

**Branch**: `034-simultaneous-federation` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

## Summary

Add an opt-in federated set of contexts, each with a persistent `kube::Client` and bounded watches (Pods, Nodes, Deployments, Warning Events) reduced to counters, plus a cross-cluster aggregate overview. Built on 027: shared client construction, classifier, concurrency cap and `FleetContextSummary` are extracted from `src/backend/fleet.rs` and reused; a tier arbiter guarantees a context is polled OR live, never both. See research.md.

## Technical Context

**Language**: Rust (edition 2024), TypeScript strict.
**Dependencies**: `kube`/`k8s-openapi`, `tokio`, `crossbeam-channel`, `ts-rs`; kube `runtime` feature for `watcher` (verify enabled; no new crate expected). Frontend: React, Zustand, shadcn/ui, Tailwind.
**Storage**: new additive `~/.fathom/federation.json` (names only).
**Testing**: cargo tests (tokio paused-time for backoff/debounce, serde round-trips in `tests/protocol_serde.rs`), Vitest for store/handlers, Playwright e2e on the scripted fake backend.
**Platform**: Tier 1 Ubuntu 26.04 / Windows 11 / macOS 26.
**Performance**: SC-001 5s freshness; SC-002 <2% CPU idle with 5 federated; zero cost when the set is empty.
**Scale**: soft cap 8 contexts.

## Constitution Check

- I: PASS. Read-only; drill-down uses existing SetContext.
- II: PASS. All cluster access in Rust; new commands/events in `src/protocol/v1/`, ts-rs generated.
- III: PASS (planned). Round-trip tests per variant, unit tests for reducers/backoff/arbiter, e2e per story.
- IV: PASS (planned). Mobile-first rows, no horizontal scroll, keyboard/touch toggles.
- V: CONDITIONAL. Persistent clients and watches are a steady-state cost; mitigated by opt-in, counters-only reduction, 500ms debounce, cap and an idle-CPU test. Recorded below.
- VI: PASS. Additive persisted file, empty fallback on parse error, names only, no secrets.

Re-check after design: unchanged.

## Project Structure

```text
specs/034-simultaneous-federation/  spec, plan, research, data-model, quickstart, tasks, contracts/federation-protocol.md
src/backend/federation.rs        # NEW: live tier (clients, watches, reducers, backoff)
src/backend/fleet.rs             # refactor: extract shared helpers, tier arbiter
src/backend/state.rs             # + FederationState
src/backend/persistence.rs       # + federation.json load/save
src/backend/cmd/cluster.rs       # dispatch new commands
src/protocol/v1/federation.rs    # NEW types; cmd.rs/event.rs additive variants
ui/src/components/fleet/FederationView.tsx, FederationRow.tsx, FederationToggle.tsx
ui/src/store/handlers/federation.ts (+ store slice)
ui/tests/e2e/us1-federation-live.spec.ts, us2-federation-fleet.spec.ts, us3-federation-persist.spec.ts
```

**Structure Decision**: extend the existing single project; one new backend module and a new UI section; no new crate.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Persistent clients + watches for up to N contexts (Principle V) | "Live" simultaneous state is the feature | Faster polling costs more API load and is not live; always-on for all contexts is worse. Mitigated by opt-in, counters-only, debounce, cap |
