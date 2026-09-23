# Implementation Plan: Agent SDK Ops (AI-Assist Troubleshooting)

**Branch**: `013-agent-sdk-ops` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/013-agent-sdk-ops/spec.md`

## Summary

Add an in-app, per-object troubleshooting assistant (starting with Pods) that answers
"what's wrong with this pod?" using only data already streamed into the frontend store
(events, conditions, container statuses, log tail, lifecycle detail). All model/API
orchestration lives in the Rust backend (Principle II); the UI only exchanges versioned
protocol commands/events. The operator picks an auth mode (BYOK Anthropic key or local
Ollama) before the assistant is usable. When the assistant proposes a corrective action,
it is represented as an inert `SuggestedAction` distinct from an `ExecutedAction`
(FR-006); the operator reviews it through the *existing* confirmation dialog for that
action type (delete/scale/restart/Helm upgrade-rollback), recomputed fresh at
confirm-time, and only the operator's confirm click sends it down the existing
`BackendCmdV1` mutation path already classified by `MutationTarget`
(`src/protocol/v1/mutation.rs`). No new mutation path is introduced.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace `Cargo.toml`) backend; TypeScript
(strict) / React 19 frontend under `ui/`.

**Primary Dependencies**:
- Backend: `reqwest` (already a workspace dependency) for both the Anthropic Messages
  API and the local Ollama REST API — see research.md R1/R2 for why no new SDK crate is
  added. `tokio`, `serde`/`serde_json`, `ts-rs` (existing). No new crate added to
  `Cargo.toml` beyond what already exists.
- Frontend: existing `zustand` store, existing confirmation-dialog components (reused,
  not duplicated), no new npm dependency required for v1 (a lightweight markdown-ish
  renderer is not needed — assistant text is rendered as plain text/structured cards).

**Storage**: `~/.fathom/agent_conversations.json` (per-object conversation history) and
`~/.fathom/credentials.json` (existing file, extended with an `agent` section for the
BYOK key / Ollama config) — both via the existing `src/backend/persistence.rs`
load/save-JSON-file convention (Principle VI).

**Testing**: `cargo test` (protocol round-trip + state-machine unit tests, fixture-only,
no live cluster/LLM calls), Vitest (component tests), Playwright (`ui/tests/e2e/us1-agent-ops-*.spec.ts`)
against the existing scripted fake backend.

**Target Platform**: Same as the rest of Fathom — Tauri v2 desktop, Tier 1
Ubuntu 26/Windows 11/macOS 26.

**Project Type**: Desktop app (existing single Rust workspace + `ui/` frontend).

**Performance Goals**: Assistant UI interactions (opening panel, rendering a response)
follow Principle V's 100ms input-handling budget; streaming model responses are exempt
(network/model-bound) but MUST render incrementally, not block the UI thread.

**Constraints**: Zero cluster-mutating commands from question-asking/response-generation
(SC-002); zero external network calls while in local Ollama mode (SC-007); BYOK key never
logged/echoed in protocol payloads (SC-004); offline-capable when Ollama mode is chosen.

**Scale/Scope**: v1 ships scoped to Pods only (extensible pattern, not built out for other
kinds yet, per spec Assumptions). One conversation per object, persisted locally.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

- **I. Operator-First, Explicit Mutation** — PASS. `SuggestedAction` is inert data; only
  the existing confirmation dialogs can produce a `BackendCmdV1` mutation. No optimistic
  UI update from an assistant suggestion.
- **II. One Contract, Backend Owns the Cluster** — PASS. All Anthropic/Ollama HTTP calls
  live in a new `src/backend/agent/` module. The UI never imports an LLM client; it only
  sends `BackendCmdV1::AgentAsk`/reviews `BackendEventV1::AgentMessage` etc.
- **III. Tested at the Boundaries** — PASS (see Testing). New protocol variants get
  round-trip tests; the Suggested→Executed transition gets fixture-driven unit tests;
  E2E covers the confirm/execute flow via the fake backend, no live LLM required.
- **IV. Density Scales With Width** — PASS. The assistant panel is an additional pane
  that appears at wider widths and collapses to a modal/sheet at phone width, following
  the existing inspector-pane pattern; it does not replace existing panes.
- **V. Fast and Quiet** — PASS. No polling; the assistant only acts on explicit operator
  request. Streaming response chunks are batched into the existing event-store update
  path, not one React re-render per token where avoidable.
- **VI. User Data Is Sacred** — PASS. Credentials follow the exact existing
  `credentials.json` load/save convention (`src/backend/persistence.rs`); an unreadable
  file falls back to "not configured" rather than deleting anything; conversation history
  is additive JSON, safe to ignore on old builds.

No violations requiring Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/013-agent-sdk-ops/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── protocol-v1-agent.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── agent.rs            # NEW: AgentAuthMode, SuggestedAction, ExecutedAction,
│   │                       #      AgentConversation, AgentMessage, grounding refs
│   ├── cmd.rs               # EXTENDED: BackendCmdV1::Agent* variants
│   ├── event.rs / events.rs # EXTENDED: BackendEventV1::Agent* variants
│   └── mutation.rs          # EXTENDED: mutation_target() arms for new Agent* commands
│                             #           (all ReadOnly or Local — see data-model.md)
├── backend/
│   ├── agent/
│   │   ├── mod.rs           # orchestration: builds grounding context, drives a turn
│   │   ├── anthropic.rs     # BYOK Anthropic Messages API client (reqwest)
│   │   ├── ollama.rs        # local Ollama /api/chat client (reqwest)
│   │   ├── grounding.rs     # reads events/conditions/logs/lifecycle already in backend state
│   │   ├── actions.rs       # maps a proposed action -> existing BackendCmdV1 preview/apply pair
│   │   └── state_machine.rs # Suggested -> (reviewed) -> Executed transition + tests
│   └── persistence.rs       # EXTENDED: agent_conversations.json + credentials.json `agent` section

ui/src/
├── features/agent-ops/      # NEW: panel, message list, suggested-action card,
│                             #      auth-setup form — reuses existing confirm dialogs
├── store/                    # EXTENDED: agent conversation slice
└── tests/
    ├── unit/agent-ops/*.test.tsx
    └── e2e/us1-agent-ops-*.spec.ts, us2-agent-ops-*.spec.ts, us3-agent-ops-*.spec.ts, us4-agent-ops-*.spec.ts
```

**Structure Decision**: Single Rust workspace + `ui/` frontend (existing layout).
Backend gets a new `src/backend/agent/` module (mirrors the existing per-domain module
layout, e.g. `src/backend/integrations.rs`, `src/backend/metrics.rs`). Frontend gets a
new `ui/src/features/agent-ops/` feature folder (mirrors existing `features/` layout),
reusing existing confirmation-dialog components rather than forking them.

## Complexity Tracking

No Constitution Check violations. Table intentionally empty.
