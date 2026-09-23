# Implementation Plan: Multi-Cluster Context Federation & Optional In-Cluster Daemon

**Branch**: `012-incluster-daemon-federation` (worktree branch: `spec-012-incluster-daemon-federation`) | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/012-incluster-daemon-federation/spec.md`

**Scope of this plan**: **P1 only** (User Story 1 — "See and switch between every cluster context at a
glance"). P2 (simultaneous federation + aggregate overview), P3 (cloud auth-plugin shellouts), and P4
(optional in-cluster daemon) are deliberately out of scope for this plan and are not designed here beyond
noting compatibility. Per the task's scope guidance, P1 ships to a high standard first; P2-P4 are only
attempted afterward if time/context remain, and only as a clean follow-on to this plan (a new
`/speckit-plan` pass), never as partial/stubbed code.

## Summary

Today Fathom loads every kubeconfig context (`SystemEvent::ContextsUpdated`) and lets the operator pick
one via a dropdown (`ContextPicker`, `BackendCmd::SetContext`), maintaining exactly one live `kube::Client`
in `ClusterContext`. P1 makes the existing context list a **persistent, always-visible** surface and adds
a **live per-context connection status** (connected / connecting / error / not connected), without
changing the single-active-client model. The backend gains lightweight, bounded background reachability
probes for every context that is *not* the active one, and explicit status transitions for the active
context's own connect/switch attempts. All new state flows through a new `SystemEvent::ContextStatusUpdated`
variant (Principle II). The frontend adds a new, always-rendered `FederationContextSwitcher` component
(new `federation-`-prefixed test ids) alongside — not replacing — the existing `ContextPicker` combobox, so
existing `us1-context-switch.spec.ts` coverage is undisturbed.

## Technical Context

**Language/Version**: Rust (edition 2024, existing `Cargo.toml`), TypeScript strict (existing `ui/tsconfig.json`)

**Primary Dependencies**: `kube` 2.0.1 (`Client::apiserver_version`), `tokio` (spawn + timeout), `crossbeam-channel`
(existing event bus) on the backend; React/Zustand store, existing `ts-rs`-generated protocol bindings on
the frontend. **No new Cargo or npm dependencies** — confirmed against `Cargo.toml` and `ui/package.json`.

**Storage**: N/A for P1 (no new persisted state; per-context status is derived live, never written to
`~/.fathom/`). P2's "simultaneously active set" and P4's daemon opt-in are the persisted-state items
flagged in spec.md's Edge Cases, deferred with P2-P4.

**Testing**: `cargo test` (protocol round-trip + status-derivation unit tests), `cargo clippy --all-targets
-- -D warnings`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, Vitest component tests, Playwright e2e
(`ui/tests/e2e/us1-federation-*.spec.ts`).

**Target Platform**: Existing Tier 1 desktop targets (Ubuntu 26.04, Windows 11, macOS 26) — no new
platform-specific behavior.

**Project Type**: Desktop app (Tauri v2 + Rust backend, React/TS frontend) — existing single-project layout.

**Performance Goals**: Status probes must not threaten Constitution V's idle-CPU budget: probes are
one-shot (not periodic pollers), bounded by a 5s `tokio::time::timeout` per context, fired only on
`Initialize`, `RefreshContexts`, and `SetContext` (i.e. exactly when the context list is (re)loaded).

**Constraints**: Zero-in-cluster-footprint default path must keep working unchanged (P4 concern, but the
P1 probe mechanism must not regress it — probes use the same `kube-rs` direct-connect path Fathom already
uses, nothing in-cluster). Mutation gating (Principle I) is not implicated: connecting/probing/switching is
connection management, not a cluster mutation.

**Scale/Scope**: Kubeconfig context counts in the tens, not thousands; no concurrency cap needed for P1's
one-shot fire-and-forget probes (P2's sustained-connection budget warning, FR-011, is out of scope here).

## Constitution Check

*GATE: checked before Phase 0 research and re-checked after Phase 1 design.*

- **Principle I (Explicit Mutation)**: PASS. No mutating actions are added; context switch/probe already
  follow existing non-optimistic, backend-authoritative behavior.
- **Principle II (One Contract, Backend Owns the Cluster)**: PASS. New state (`ContextStatusUpdated`) is
  added to the versioned protocol in `src/protocol/v1/event.rs`; the UI reads it only through the existing
  store/ingest pipeline, never touching `~/.fathom/` or backend internals directly. No protocol version
  bump needed — this is an additive new event variant, not a breaking change to an existing one.
- **Principle III (Tested at Boundaries)**: PASS (design commits to it): protocol round-trip test for the
  new event/status enum, backend unit tests for status-derivation/probe-skip logic using fixture context
  lists (no live cluster), a Vitest test for the new store handler, and a new Playwright e2e spec
  (`us1-federation-context-status.spec.ts`) driving the fake backend.
- **Principle IV (Density Scales With Width)**: PASS. The new switcher is rendered identically (a
  persistent, locally horizontally-scrollable strip) at every width class — it does not reduce the number
  of visible panes, and does not introduce page-level horizontal scroll (local `overflow-x-auto` on the
  strip only, consistent with existing tab-strip patterns in `StackTabs.tsx`).
- **Principle V (Fast and Quiet)**: PASS. Probes are one-shot and timeout-bounded (5s); no new periodic
  polling loop is introduced. Idle CPU is unaffected once probes settle.
- **Principle VI (User Data Is Sacred)**: PASS. No new persisted state in P1; nothing under `~/.fathom/`
  changes shape.

No violations to record in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/012-incluster-daemon-federation/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── context-status.md # Phase 1 output: protocol + component contract
└── tasks.md              # Phase 2 output (speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/event.rs         # + ContextConnectionStatus, SystemEvent::ContextStatusUpdated
├── backend/
│   ├── kube.rs                  # load_kubeconfig() made pub(super); load_and_send_contexts()
│   │                             #   returns the (contexts, current) tuple to callers
│   ├── context_status.rs        # NEW: spawn_status_probes(), probe_context(), emit_status()
│   └── cmd/cluster.rs           # initialize()/set_context() emit Connecting/Connected/Error
│                                 #   for the active context; call context_status::spawn_status_probes
tests/                           # Rust unit/integration tests colocated per existing convention
  (protocol round-trip test lives in src/protocol/v1/event.rs's #[cfg(test)] mod, matching
  existing pattern for other event variants)

ui/src/
├── protocol/generated/          # ts-rs regenerated bindings (SystemEvent.ts, ContextConnectionStatus.ts)
├── store/handlers/system.ts     # + ContextStatusUpdated case -> session.contextStatuses
├── store/index.ts               # + SessionState.contextStatuses: Record<string, ContextStatusEntry>
├── components/shell/
│   └── FederationContextSwitcher.tsx   # NEW: always-visible context list + live status
└── components/shell/AppShell.tsx       # mounts FederationContextSwitcher below TopBar

ui/tests/
├── store/handlers/system.federation.test.ts   # NEW: Vitest unit test for the new handler case
└── e2e/us1-federation-context-status.spec.ts   # NEW: Playwright e2e (FR-001..FR-005)
```

**Structure Decision**: Single existing project (Tauri v2 desktop app); no new top-level directories. All
P1 changes are additive files/functions inside the existing `src/backend`, `src/protocol/v1`, and `ui/src`
trees, following established per-feature file-naming conventions (`us1-federation-*.spec.ts`,
`federation-` test-id prefix) called out in the task brief.

## Complexity Tracking

*No entries — no Constitution violations.*
