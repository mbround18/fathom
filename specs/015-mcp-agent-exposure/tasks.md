# Tasks: MCP Agent Exposure

**Input**: Design documents from `specs/015-mcp-agent-exposure/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Explicitly requested by spec.md's Testing Requirements — included throughout.

## Format: `[ID] [P?] [Story] Description`

---

## Phase 1: Setup

- [X] T001 Confirm `rmcp = { version = "0.12.0", features = ["server"] }` in `Cargo.toml` builds cleanly as a real dependency: `cargo check -p fathom` from repo root after adding a trivial `use rmcp::*;` smoke reference; remove the smoke reference once real usage lands in Phase 2.
- [X] T002 [P] Grep-audit exact current `Mcp*`/`mcp_*` identifiers in `src/protocol/v1/{cmd.rs,event.rs,mutation.rs,route.rs,ui_config.rs}` and `src/backend/persistence.rs` and record the precise list (variant names, field names, line numbers) in a scratch note at the top of this task's commit message — this list is the ground truth for Phase 2's removal tasks, since research.md's file/line references may drift as the worktree is edited.

---

## Phase 2: Foundational — retire the old HTTP MCP prototype (BLOCKS all user stories)

**Purpose**: Delete `src/mcp_server.rs`'s HTTP surface and everything wired to it, and repurpose the one piece of state (the `mcp_*` boolean preference) that the new feature reuses, so Phase 3+ starts from a clean slate with no dead/duplicate MCP surface. Per research.md R2/R3/R4.

- [X] T003 Delete `src/mcp_server.rs` in its entirety (HTTP axum server on port 18288, `McpAppState`, `restart_deployment_tool`, `get_pod_logs_tool`, `list_deployments_tool`, all `handle_*`/`serve_mcp_http`/`run_http_mcp_server` functions).
- [X] T004 Remove `pub mod mcp_server;` from `src/lib.rs`.
- [X] T005 Remove all `McpAppState`/`initialize_mcp_server`/`shutdown_mcp_server`/`run_http_mcp_server` references from `src/main.rs` and `src/backend.rs` (startup/shutdown wiring for the old HTTP server).
- [X] T006 [P] Remove the `McpAppState` field and any accessor methods from `src/backend/state.rs`.
- [X] T007 [P] Remove the `McpAppState` status/error message push calls from `src/backend/cmd/integrations.rs`.
- [X] T008 [P] Remove the `McpAppState` context/namespace push calls from `src/backend/cmd/cluster.rs`.
- [X] T009 [P] Remove the `McpAppState` deployment-cache push call from `src/backend/deployments.rs`.
- [X] T010 Delete `src/app/views/mcp_integration.rs`; remove its `McpIntegrationView` import and the view-switch arm (`Box::new(McpIntegrationView::new())`) from `src/app/mod.rs` (research.md R3 — confirmed dead-from-Tauri, but must stay non-broken for the still-buildable legacy `egui-ui` binary).
- [X] T011 In `src/backend/persistence.rs`: rename `AutoStartConfig.mcp_http` → `AutoStartConfig.mcp_stdio` (same struct, same `load()`/`save()` pattern at `persistence.rs:17-49`); rename `load_mcp_auto_start`/`save_mcp_auto_start` doc comments to describe the new stdio server instead of "legacy HTTP" (research.md R4). Do not add a new config file.
- [X] T012 In `src/protocol/v1/event.rs`: rename `SystemEvent::UiConfigLoaded.mcp_auto_start` → `mcp_stdio_enabled` (same field position/type, `bool`); update `src/backend/cmd.rs`'s construction of that event (currently reads `loaded.mcp_auto_start`) to the new field name.
- [X] T013 [P] In `src/protocol/v1/route.rs` and `src/protocol/v1/ui_config.rs`: remove any remaining references to the old HTTP server's routing/config surface uncovered by T002's audit.
- [X] T014 Retire `docs/mcp/*.md`: delete `MCP_IMPLEMENTATION_SUMMARY.md`, `MCP_INTEGRATION.md`, `MCP_QUICK_START.md` (fully superseded); rewrite `docs/mcp/MCP_ARCHITECTURE.md` in place to describe the new stdio/`rmcp` architecture (transport, tool list, guard/RBAC model) rather than the deleted HTTP design.
- [X] T015 Run `cargo build --workspace` and `cargo build --workspace --features egui-ui` (or equivalent default-features build for the legacy `fathom` binary) to confirm both the Tauri-path crate and the legacy egui binary compile clean after all removals in T003-T014, before starting Phase 3.

**Checkpoint**: No `mcp_server.rs`/`mcp_integration.rs`/stale `docs/mcp/*` remain; `cargo build` clean on both feature configurations; `AutoStartConfig.mcp_stdio` exists and is currently unused (ready for Phase 3 to wire up).

---

## Phase 3: User Story 1 — Attach an external agent client to live cluster context (Priority: P1) 🎯 MVP

**Goal**: Operator turns the MCP server on; an external MCP client can attach over stdio and call RBAC-scoped read tools.

**Independent Test**: With server on and a kube context connected, list tools and call a read tool from a real `rmcp`-based (or manually scripted JSON-RPC) test client; verify RBAC-out-of-scope denial with a fixture context that has restricted access.

### Tests for User Story 1

- [X] T016 [P] [US1] Round-trip serde tests in `tests/protocol_serde.rs` for `GetMcpServerStatus`/`LoadMcpServerStatus` (ReadOnly) and `SetMcpServerEnabled { enabled: bool }` (Local) commands, and `McpServerStatusChanged { enabled, running, connection_command }` event — matching the existing per-command `roundtrip(...)` helper pattern already used for `LoadClusterSettings`/`SaveClusterSettings` (`cmd.rs:1070-1100`).
- [X] T017 [P] [US1] Unit test `mcp_tool_calls_respect_namespace_rbac` in `src/backend/mcp/rbac_guard.rs` (`#[cfg(test)]` module): fixture kube-access state (modeled on `rbac_access.rs`'s existing test fixtures — `role`, `role_binding`, `subject_matches` helpers) where the current context has access to namespace `allowed` but not `denied`; assert a read-tool-call scoping check against `denied` returns a specific denial reason and never proceeds to a cluster read.
- [X] T018 [P] [US1] Unit test in `src/backend/mcp/tools_read.rs` mapping at least one read tool (`describe_pod`) to `grounding::collect_pod_grounding`, asserting the tool handler returns the same shaped data `collect_pod_grounding` produces for a fixture pod (no live cluster required — fake `kube::Client`/test fixture per existing agent test conventions in `src/backend/agent/`).

### Implementation for User Story 1

- [X] T019 [US1] Add `SetMcpServerEnabled { enabled: bool }`, `GetMcpServerStatus`/`LoadMcpServerStatus` variants to `src/protocol/v1/cmd.rs`; add corresponding `mutation_target()` arms in `src/protocol/v1/mutation.rs` classifying both as documented in contracts/protocol.md (`Local` for the setter, `ReadOnly` for the getter).
- [X] T020 [US1] Add `McpServerStatusChanged { enabled: bool, running: bool, connection_command: String }` event variant to `src/protocol/v1/event.rs`.
- [X] T021 [US1] Create `src/backend/mcp/mod.rs`: lifecycle manager holding an `Option<(rmcp server task handle, tokio_util::sync::CancellationToken)>`; `start()` spawns the stdio server via `rmcp::service::server::serve_server_with_ct` over `rmcp::transport::io::stdio()` (research.md R1) only when called from the `SetMcpServerEnabled { enabled: true }` handler; `stop()` cancels the token and awaits the task, called from `enabled: false` and app shutdown.
- [X] T022 [US1] Create `src/backend/mcp/rbac_guard.rs`: a function wrapping `crate::backend::rbac_access::resolve_namespace_access` (`pub(super)`, callable from `crate::backend::mcp` per research.md R5) that, given the current kube context and a target namespace + verb, returns `Ok(())` or a `McpToolDenial`-shaped rejection (per data-model.md).
- [X] T023 [US1] Create `src/backend/mcp/tools_read.rs`: implement the `rmcp` `ServerHandler`'s read tools (`get_resource`, `list_resources`, `get_pod_logs`, `get_events`, `describe_pod` per contracts/protocol.md) as `#[tool_router]`/`#[tool(description = "...")]` methods, each calling the RBAC guard (T022) before delegating to the same backend functions the existing `BackendCmd` read handlers use (or `grounding::collect_pod_grounding` for `describe_pod`, per research.md R6).
- [X] T024 [US1] Wire `src/backend/cmd/mcp.rs` (new file): `BackendCmd` handlers for `SetMcpServerEnabled` (persists via `persistence::save_mcp_auto_start` [renamed target from T011] + calls `mcp::start()`/`mcp::stop()`) and `GetMcpServerStatus`/`LoadMcpServerStatus` (reads current state, builds the `connection_command` string — the Fathom binary path plus its stdio-invocation flag); emits `McpServerStatusChanged` on every change.
- [X] T025 [US1] Register the new command handlers in the backend's command dispatch table (wherever `src/backend/cmd.rs` routes `BackendCmdV1` variants to handler functions — follow the existing pattern for `LoadClusterSettings`/`SaveClusterSettings`).
- [X] T026 [US1] Regenerate ts-rs bindings: `cargo test --test export_bindings` from repo root; commit the resulting changed files under `ui/src/protocol/generated/`.
- [X] T027 [P] [US1] Create `ui/src/components/panels/agent-ops/AvailableAgentsSection.tsx`: renders MCP server status (on/off toggle sending `SetMcpServerEnabled`, running/not-running indicator, connection snippet display) and the Anthropic/Ollama auth summary (reused read from whatever store slice `AuthSetup.tsx` already populates — no new detection call). All new testids prefixed `mcp-`.
- [X] T028 [US1] Mount `AvailableAgentsSection` inside `ui/src/components/panels/ClusterSettingsPanel.tsx`'s "Available Agents" section (new section, alongside the existing §10 metrics fields — per spec FR-007, home confirmed in plan.md).
- [X] T029 [US1] Add a Zustand store slice/handler (`ui/src/store/handlers/` — follow the existing per-event-type handler file convention) for `McpServerStatusChanged`, and a `send`-based action for `SetMcpServerEnabled`/`GetMcpServerStatus`, matching the existing `ClusterSettings` load/save action pattern in `ui/src/store/actions`.
- [X] T030 [P] [US1] Vitest component test for `AvailableAgentsSection` in `ui/tests/unit/` (or colocated per this project's existing convention — check where `ClusterSettingsPanel`'s own tests live and mirror it): covers off-by-default rendering, toggle firing `SetMcpServerEnabled`, and running-state connection-snippet display.
- [X] T031 [US1] e2e Playwright spec `ui/tests/e2e/mcp-agent-status.spec.ts`: drives a scripted fake backend (per Principle III's "scripted fake backend" convention) through off → toggle-on → running states in the Available Agents section; testids `mcp-`-prefixed.

**Checkpoint**: MCP server can be turned on from Settings, a real `rmcp` stdio server starts, read tools are RBAC-scoped, and this is independently demonstrable without any mutation-tool or demo-chat code existing yet.

---

## Phase 4: User Story 2 — Gated mutation from an external agent (Priority: P2)

**Goal**: Mutation tools require both client-side approval signaling (via tool schema/description) and independent backend re-validation through the same `expected_resource_version` guard `apply_resource` already uses.

**Independent Test**: Fixture-based test constructs a mutation tool call with a stale `expected_resource_version` and asserts no write occurs; a valid one succeeds and emits the standard mutation event.

### Tests for User Story 2

- [X] T032 [P] [US2] Unit test `mcp_mutation_tools_require_explicit_approval` in `src/backend/mcp/tools_mutate.rs` (`#[cfg(test)]`, mirroring 013's `agent_commands_never_write_cluster` structure): using a fake/test `kube::Client` (existing test-fixture convention in `src/backend/apply.rs`'s own tests), assert that calling the mutation-tool handler with (a) a missing `expected_resource_version`, and (b) a stale one that doesn't match the fixture's live object, in both cases returns a rejection `CallToolResult` and the fixture backend records zero write calls.
- [X] T033 [P] [US2] Unit test in `src/backend/mcp/rbac_guard.rs` extending T017's fixtures: a mutation-scoped call (`update`/`delete` verb) against a namespace the context can only `get`/`list` (not mutate) is denied, distinct from the read-only RBAC test.
- [X] T034 [P] [US2] Round-trip serde test in `tests/protocol_serde.rs`: none required for new `BackendCmdV1` variants here (per contracts/protocol.md, mutation tools are external-MCP-protocol-only, not `BackendCmd`) — instead add a unit test asserting `src/backend/mcp/tools_mutate.rs`'s tool schema marks `expected_resource_version` as a required JSON schema field (no default), verified via `rmcp`'s generated schema for the tool struct.

### Implementation for User Story 2

- [X] T035 [US2] Create `src/backend/mcp/tools_mutate.rs`: implement `scale_workload`, `apply_resource_yaml`, `delete_resource` as `#[tool]` methods; each tool's Rust input struct has a mandatory (non-`Option`) `expected_resource_version: String` field (contracts/protocol.md's schema contract) and a `description` string stating "This action modifies the cluster. Confirm with the operator before calling."
- [X] T036 [US2] Each mutation tool handler, before any write: (1) calls `rbac_guard` (T022) for the required verb; (2) re-fetches the live target and compares `resourceVersion`/`uid`; (3) on both passing, calls `crate::backend::apply::apply_resource(..., expected_resource_version)` directly (the exact function `ApplyResourceYaml`'s handler already calls) for `apply_resource_yaml`, and the equivalent existing scale/delete backend functions for the other two tools — no new mutation code path, only a new caller of the existing guarded functions.
- [X] T037 [US2] On successful mutation, emit the same `BackendEvent` the equivalent UI-driven mutation already emits (reuse existing event-emission call, not a new event type), so the operator's own Fathom UI reflects the change in real time (Principle I).
- [X] T038 [P] [US2] Update `docs/mcp/MCP_ARCHITECTURE.md` (rewritten in T014) with the finalized mutation-tool guard sequence diagram/description from contracts/protocol.md.

**Checkpoint**: Mutation tools exist, cannot execute without passing the resource-version guard and RBAC check, and successful mutations show up in Fathom's own UI — independently testable without the demo-chat feature.

---

## Phase 5: User Story 3 — Settings visibility and explicit control (Priority: P1)

**Note**: Core toggle/status plumbing (protocol commands, persistence, Settings section mount) was already built in Phase 3 (T019-T031) because US1 and US3 share the same foundational Settings surface per plan.md. This phase adds the remaining US3-specific acceptance criteria: default-off persistence-across-restart proof, and the Anthropic/Ollama auth summary reuse.

### Tests for User Story 3

- [X] T039 [P] [US3] Unit test in `src/backend/persistence.rs`'s existing test module: `mcp_stdio` defaults to `false` on a fresh `AutoStartConfig::load()` (no file present), and round-trips `true` after `save()`.
- [X] T040 [P] [US3] Vitest test extending T030: `AvailableAgentsSection` displays the existing `AgentAuthMode` summary (Anthropic configured / Ollama configured / none) sourced from the same store slice `AuthSetup.tsx` reads, asserting no duplicate credential-detection network/IPC call is made by the new component (spy on `send`/`request` and assert no new auth-check command fires).

### Implementation for User Story 3

- [X] T041 [US3] In `AvailableAgentsSection.tsx` (T027), read the existing Anthropic/Ollama auth state from the store (whatever slice backs `AuthSetup.tsx` — confirm exact selector during implementation) and render it read-only alongside the MCP toggle; do not add an edit affordance here (that stays `AuthSetup.tsx`'s job, per spec non-goals).
- [X] T042 [US3] e2e Playwright spec addition (extend `mcp-agent-status.spec.ts` or add `mcp-settings-persistence.spec.ts`): toggle on, simulate app restart (re-mount against the scripted fake backend re-reporting `mcp_stdio_enabled: true` in `UiConfigLoaded`), assert the toggle renders on without operator re-input.

**Checkpoint**: Settings section fully satisfies spec.md User Story 3's three acceptance scenarios independently of mutation tooling or demo chat.

---

## Phase 6: User Story 4 — Demo/example chat launch (Priority: P3)

**Goal**: A real, functional "Launch example chat" affordance using a detected local `claude` CLI, with an honest not-detected fallback.

**Independent Test**: With a stubbed/real `claude` binary on PATH and the MCP server on, launching produces a real subprocess and visible output; removing the binary from PATH produces the not-detected state.

### Tests for User Story 4

- [X] T043 [P] [US4] Unit test in `src/backend/mcp/demo_chat.rs`: PATH-detection function returns `Some(path)` when a fixture `claude` executable is placed in a temp dir added to `PATH` for the test, and `None` when it is not — no network/subprocess spawn in this test, detection only.
- [X] T044 [P] [US4] Unit test in `src/backend/mcp/demo_chat.rs`: `StartMcpDemoChat` handler refuses to spawn (returns an error/event indicating the server must be enabled first) when `McpServerStatus.running` is `false`, without attempting `Command::spawn`.
- [X] T045 [P] [US4] Vitest test for the demo-chat portion of `AvailableAgentsSection.tsx`: detected state renders a "Launch example chat" button; not-detected state renders instructions and no button; server-off state renders the "enable the server first" message — three distinct fixture-driven renders.

### Implementation for User Story 4

- [X] T046 [US4] Add `StartMcpDemoChat`, `StopMcpDemoChat`, `SendMcpDemoChatInput { data: String }`, `ResizeMcpDemoChat { cols: u16, rows: u16 }` commands to `src/protocol/v1/cmd.rs` (`MutationTarget::Local` for all, per contracts/protocol.md) and `McpDemoChatReady { session_id: u32 }`, `McpDemoChatOutputChunk { session_id: u32, data: String }`, `McpDemoChatEnded { session_id: u32, reason: String }`, `McpDemoChatNotDetected { instructions: String }` events to `src/protocol/v1/event.rs`.
- [X] T047 [US4] Create `src/backend/mcp/demo_chat.rs`: PATH detection for a `claude` binary (`which`-equivalent, no caching — checked fresh on each `StartMcpDemoChat`/status query per spec FR-009's "never fabricate" requirement); on start, spawns `claude` via `tokio::process::Command` pre-configured with Fathom's MCP connection command (from T024's `connection_command` string) as its MCP server config argument, wires stdout/stderr into chunked `McpDemoChatOutputChunk` events keyed by a monotonic `session_id` (mirrors `PodTerminalTab.tsx`'s "track the highest stream id you started" convention, research.md R8), and stdin fed from `SendMcpDemoChatInput`.
- [X] T048 [US4] Wire the new commands/events into `src/backend/cmd/mcp.rs` and the dispatch table (extends T024/T025).
- [X] T049 [US4] Round-trip serde tests in `tests/protocol_serde.rs` for all four new commands and four new events from T046.
- [X] T050 [US4] Regenerate ts-rs bindings (`cargo test --test export_bindings`) for the new demo-chat protocol surface.
- [X] T051 [US4] Extend `AvailableAgentsSection.tsx` with the demo-chat affordance: detected/not-detected/server-off states (per T045), and an embedded `xterm.js` session view reusing `PodTerminalTab.tsx`'s rendering pattern (new small component, e.g. `DemoChatTerminal.tsx`, sharing the `xterm` + `FitAddon` setup but driven by the new `McpDemoChat*` events instead of `PodExec*`).
- [X] T052 [US4] e2e Playwright spec `ui/tests/e2e/mcp-demo-chat.spec.ts`: covers detected-launch, not-detected, and server-off states against the scripted fake backend; testids `mcp-`-prefixed.

**Checkpoint**: All four user stories independently functional and tested.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [X] T053 [P] Run `cargo fmt --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`; fix any findings across all new/modified files from Phases 2-6.
- [X] T054 [P] Run `pnpm -C ui typecheck` and `pnpm -C ui lint`; fix findings in new frontend files.
- [X] T055 Run the full `cargo test` suite and `pnpm -C ui test` (vitest) and confirm all pass, including T016-T018, T032-T034, T039, T043-T044, T049.
- [X] T056 Run the full new e2e suite (`mcp-agent-status.spec.ts`, `mcp-settings-persistence.spec.ts` or equivalent, `mcp-demo-chat.spec.ts`) plus a smoke pass of two adjacent already-shipped e2e suites (e.g. 011's RBAC suite, 013's agent-ops suite) to confirm no regression from this feature's protocol/Settings-panel changes.
- [X] T057 Execute quickstart.md's manual validation scenarios end-to-end against a real kube context and, where feasible, a real MCP client attach, and record results.
- [X] T058 Update `docs/MVP.md` marking 015-mcp-agent-exposure shipped, matching the exact format used for specs 011/014.

---

## Dependencies & Execution Order

- **Phase 1 (Setup)**: No dependencies.
- **Phase 2 (Foundational — removal)**: Depends on Phase 1. **BLOCKS all of Phase 3-6** — the new server/settings code assumes no old `mcp_server.rs`/`mcp_integration.rs` remain, and repurposes (not duplicates) `AutoStartConfig`'s MCP field.
- **Phase 3 (US1, P1)**: Depends on Phase 2. No dependency on US2/US3/US4.
- **Phase 4 (US2, P2)**: Depends on Phase 2 and on `src/backend/mcp/mod.rs`/`rbac_guard.rs` existing from Phase 3 (T021-T022) — mutation tools are registered on the same `ServerHandler`/`ToolRouter` the read tools use.
- **Phase 5 (US3, P1)**: Depends on Phase 2; shares Phase 3's Settings-section scaffolding (T027-T029) — sequence Phase 3 before Phase 5 even though both are P1.
- **Phase 6 (US4, P3)**: Depends on Phase 2 and Phase 3 (needs `connection_command`/server-running status from T024).
- **Phase 7 (Polish)**: Depends on all desired story phases being complete.

## Parallel Example: Phase 2 (Foundational)

```
Task: "Remove the McpAppState field from src/backend/state.rs" (T006)
Task: "Remove the McpAppState status/error push calls from src/backend/cmd/integrations.rs" (T007)
Task: "Remove the McpAppState context/namespace push calls from src/backend/cmd/cluster.rs" (T008)
Task: "Remove the McpAppState deployment-cache push call from src/backend/deployments.rs" (T009)
```
(T003-T005 and T010-T014 touch overlapping/ordered files and should run sequentially.)

## Implementation Strategy

### MVP First (User Story 1)

Phase 1 → Phase 2 (removal, mandatory) → Phase 3 (US1) → stop and validate read-only external attach end-to-end before proceeding to gated mutation.

### Incremental Delivery

Phase 2 → Phase 3 (US1, read-only MVP) → Phase 4 (US2, mutation) → Phase 5 (US3, settings polish — mostly already done alongside US1) → Phase 6 (US4, demo chat) → Phase 7 (polish/verification/merge readiness).
