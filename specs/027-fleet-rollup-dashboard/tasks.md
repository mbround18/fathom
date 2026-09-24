---
description: "Task list for Fleet Rollup Dashboard"
---

# Tasks: Fleet Rollup Dashboard

**Input**: Design documents from `specs/027-fleet-rollup-dashboard/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/fleet-protocol.md, quickstart.md

**Tests**: Included per Constitution III (mandatory boundary tests) and the feature brief's
explicit request for one hyper-focused Playwright e2e spec.

**Organization**: Tasks are grouped by user story (spec.md P1/P2/P3) to enable independent
implementation and testing of each.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Create `src/protocol/v1/fleet.rs` with module skeleton and register it in
  `src/protocol/v1/mod.rs` (empty types for now — filled in Phase 2).
- [ ] T002 [P] Create `src/backend/fleet.rs` module skeleton and register it in
  `src/backend/mod.rs` (empty — filled in Phase 2).
- [ ] T003 [P] Create `ui/src/components/fleet/` directory and `ui/src/store/handlers/fleet.ts`
  placeholder files (empty — filled in Phase 3).

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types and backend state plumbing every user story's poller/UI needs.
**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T004 In `src/protocol/v1/fleet.rs`, define `FleetHealthState` enum
  (`Healthy | Degraded | Unreachable | AuthError | Pending`), fieldless, `Serialize`/
  `Deserialize`/`TS`, serialized as a bare string per data-model.md.
- [ ] T005 In `src/protocol/v1/fleet.rs`, define `FluxRollupStatus` enum
  (`Synced | OutOfSync | NotInstalled`), fieldless, `Serialize`/`Deserialize`/`TS`.
- [ ] T006 In `src/protocol/v1/fleet.rs`, define `FleetContextSummary` struct per data-model.md
  exactly: `context: String`, `health: FleetHealthState`, `error: Option<String>` (MUST be
  `None` unless `health` is `Unreachable` or `AuthError`), `node_ready_count: Option<u32>`,
  `node_total_count: Option<u32>`, `unhealthy_pod_count: Option<u32>`,
  `cluster_version: Option<String>`, `flux: Option<FluxRollupStatus>`, `last_updated: String`
  (ISO8601). Derive `Serialize`/`Deserialize`/`TS`/`Debug`/`Clone`.
- [ ] T007 [P] In `src/protocol/v1/cmd.rs`, add `BackendCmd::StartFleetPolling` and
  `BackendCmd::StopFleetPolling` fieldless variants (contracts/fleet-protocol.md).
- [ ] T008 [P] In `src/protocol/v1/event.rs`, add `SystemEvent::FleetSummaryUpdated(FleetContextSummary)`
  variant.
- [ ] T009 [US-shared] Add serde round-trip tests in `tests/protocol_serde.rs` for
  `FleetHealthState`, `FluxRollupStatus`, `FleetContextSummary` (including the invariant: a
  sample with `health: Healthy` and `error: Some(...)` is a case the *type* allows but the
  *invariant test in T014 below* forbids at construction — this test is pure (de)serialization
  round-trip, not the invariant check), `BackendCmd::StartFleetPolling`/`StopFleetPolling`,
  `SystemEvent::FleetSummaryUpdated`.
- [ ] T010 In `src/backend/state.rs`, add `pub(super) struct FleetState { pollers:
  HashMap<String, tokio::task::JoinHandle<()>>, summaries: HashMap<String, FleetContextSummary>
  }`, add `fleet: FleetState` field to `BackendState` (default-constructed empty in `::new()`),
  and add `FleetState::abort_all()` (aborts every handle in `pollers`, clears both maps) called
  from `cleanup_all_tasks()` alongside the existing `watch_tasks.abort_all()` etc.
- [ ] T011 In `src/backend/fleet.rs`, implement `pub(super) fn build_probe_client(context_name:
  &str) -> anyhow::Result<kube::Client>` reusing the exact construction
  `context_status::probe_context` uses (`load_kubeconfig` → `Config::from_custom_kubeconfig` →
  `KubeConfigOptions{ context: Some(name) }` → `Client::try_from`), factored so both modules can
  share it without duplicating the kubeconfig-load/auth path (research.md R1). Refactor
  `context_status.rs::probe_context` to call this shared helper.
- [ ] T012 In `src/backend/cmd/cluster.rs`, add `BackendCmd::StartFleetPolling` and
  `BackendCmd::StopFleetPolling` arms to `handle()`'s match, dispatching to new
  `fleet::start_polling(state, sender)` / `fleet::stop_polling(state)` functions (stubs
  returning `Ok(())` for now — real bodies land in Phase 3).

**Checkpoint**: Protocol compiles, `cargo test` and `cargo build --workspace --all-targets`
pass with the new types/commands wired to no-op stubs. Foundation ready for User Story 1.

---

## Phase 3: User Story 1 - See fleet-wide health at a glance (Priority: P1) 🎯 MVP

**Goal**: Opening the Fleet Dashboard shows a card per connected context with node counts,
unhealthy-pod count, and cluster version, without switching the active context; clicking a
card switches context via the existing mechanism.

**Independent Test**: With two connected contexts, one healthy and one with an unhealthy pod,
open the Fleet Dashboard and confirm both contexts' summaries render simultaneously with
correct health, without changing the active context.

### Tests for User Story 1

- [ ] T013 [P] [US1] Unit test in `src/backend/fleet.rs` (`#[cfg(test)] mod tests`): a poll
  attempt that succeeds with 0 unhealthy pods and all nodes ready maps to
  `FleetHealthState::Healthy` with `error: None`.
- [ ] T014 [P] [US1] Unit test in `src/backend/fleet.rs`: a poll attempt with at least one
  unhealthy pod maps to `FleetHealthState::Degraded`; a timed-out/connection-refused poll maps
  to `FleetHealthState::Unreachable` with `error: Some(_)` populated (verbatim message, per
  Constitution I); assert the invariant "`error` MUST be `None` unless `health` is
  `Unreachable`/`AuthError`" holds for every constructed `FleetContextSummary` in these cases
  (data-model.md validation rule).
- [ ] T015 [P] [US1] Unit test in `src/backend/fleet.rs`: `start_polling` is idempotent — calling
  it twice for the same connected-context set does not spawn a second poller task for a context
  that already has one running (assert `FleetState.pollers.len()` unchanged on the second call).
- [ ] T016 [P] [US1] Vitest unit test in `ui/src/store/handlers/fleet.test.ts` (new file): a
  `FleetSummaryUpdated` event for context "prod" updates `fleetSummaries.prod` and leaves other
  entries untouched (last-write-per-context-wins, mirrors `contextStatuses`' existing handler
  test style).

### Implementation for User Story 1

- [ ] T017 [US1] In `src/backend/fleet.rs`, implement `pub(super) async fn poll_once(context:
  &str, client: &kube::Client) -> FleetContextSummary`: fetches node summaries (reuse
  `nodes::fetch_node_summaries`/equivalent list logic — `Api::<Node>::list`), counts
  ready/total; fetches pod health (reuse existing pod-health classification used elsewhere in
  `src/backend/pods.rs` for "unhealthy" — do not reinvent the definition); fetches
  `client.apiserver_version()`; wraps the whole attempt in `tokio::time::timeout(Duration::from_secs(10),
  ...)` (research.md R3); maps success/timeout/error to `FleetHealthState` and populates
  `last_updated` with the current UTC ISO8601 timestamp on every attempt (data-model.md rule:
  set on every poll attempt, success or failure).
- [ ] T018 [US1] In `src/backend/fleet.rs`, implement the per-context poller loop: `fn
  spawn_poller(context: String, seq: Arc<AtomicU64>, sender: Sender<BackendEventEnvelope>) ->
  tokio::task::JoinHandle<()>` — builds one `kube::Client` via `build_probe_client` (T011) held
  for the task's lifetime, then `loop { poll_once(...); emit FleetSummaryUpdated; sleep(30s) }`
  (research.md R3's 30s interval), with the client build failure itself mapped to
  `FleetHealthState::Unreachable`/`AuthError` (do not panic the task) so a single bad context
  never kills its own loop or any other context's.
- [ ] T019 [US1] In `src/backend/fleet.rs`, implement `pub(super) fn start_polling(state: &mut
  BackendState, sender: &Sender<BackendEventEnvelope>)`: for every context in
  `state.cluster` context-status-tracked as `Connected` (reuse whatever the backend already
  uses to know which contexts are connected — see `context_status.rs`/how `SetContext`/
  `RefreshContexts` determine the connected set) without an existing entry in
  `state.fleet.pollers`, immediately emit a `Pending` `FleetSummaryUpdated` and insert
  `spawn_poller(...)`'s handle into `state.fleet.pollers`.
- [ ] T020 [US1] In `src/backend/fleet.rs`, implement `pub(super) fn stop_polling(state: &mut
  BackendState)`: abort and clear every handle in `state.fleet.pollers`, clear
  `state.fleet.summaries`.
- [ ] T021 [US1] Wire `T012`'s stub arms in `src/backend/cmd/cluster.rs` to call the real
  `fleet::start_polling`/`fleet::stop_polling` from T019/T020.
- [ ] T022 [US1] In `src/backend/cmd/cluster.rs`, extend `set_context()` and wherever a context
  disconnects/becomes non-`Connected` to also stop that one context's fleet poller specifically
  (remove+abort its single entry from `state.fleet.pollers`, drop its `state.fleet.summaries`
  entry) so a context leaving the connected set while the dashboard is open is reflected
  without requiring `StopFleetPolling`+`StartFleetPolling` to be resent (FR-006).
- [ ] T023 [US1] Add `ui/src/protocol/types` regeneration (run whatever the repo's existing
  `ts-rs`/bindings-generation step is, per Constitution II, so `FleetContextSummary` etc. are
  available as TS types) — confirm generated output lands in the same location existing
  generated protocol types already do (check `ui/src/protocol/` or wherever `ts-rs`'s
  `#[ts(export)]` output directory is configured).
- [ ] T024 [US1] In `ui/src/store/index.ts`, add `fleetSummaries: Record<string,
  FleetContextSummary>` to the store's session/runtime state (not persisted, matches
  `contextStatuses`), plus `startFleetPolling()`/`stopFleetPolling()` actions sending
  `StartFleetPolling`/`StopFleetPolling` commands.
- [ ] T025 [US1] In `ui/src/store/handlers/fleet.ts`, implement the `FleetSummaryUpdated`
  handler: upsert `fleetSummaries[event.context] = event` (last-write-wins), register it in the
  store's event dispatch table alongside the existing `ContextStatusUpdated` handler
  registration in `ui/src/store/handlers/system.ts`'s sibling wiring.
- [ ] T026 [US1] Create `ui/src/components/fleet/FleetContextCard.tsx`: renders one
  `FleetContextSummary` — context name, health badge (Healthy/Degraded/Unreachable/AuthError/
  Pending, each visually distinct per FR-005), node ready/total, unhealthy-pod count, cluster
  version; `onClick` calls the existing `selectContext(name)` store action (same one
  `FederationContextSwitcher.tsx` uses) — no new context-switch mechanism (FR-007).
- [ ] T027 [US1] Create `ui/src/components/fleet/FleetDashboardView.tsx`: mobile-first grid of
  `FleetContextCard`s driven by `session.contexts` × `fleetSummaries` (a context present in
  `session.contexts` but absent from `fleetSummaries` renders as a "Pending" placeholder card,
  not omitted); calls `startFleetPolling()` on mount and `stopFleetPolling()` on unmount
  (research.md R2 — pollers only run while this view is open); renders the FR-011 empty state
  ("connect a context to see fleet health" or similar) when `session.contexts` is empty.
- [ ] T028 [US1] Register the Fleet Dashboard as a new nav entry/section in the existing shell
  routing (`ui/src/components/shell/` — find how other top-level dashboard views are registered
  in `AppShell.tsx`/`section_keys` and follow the same pattern) so it's reachable from the UI.
- [ ] T029 [US1] Add `ui/tests/e2e/us1-fleet-dashboard.spec.ts`: using the scripted fake backend
  (per Constitution III), script two fixture contexts already `Connected` (one healthy poll
  response, one with `unhealthyPodCount > 0`), open the Fleet Dashboard, assert both contexts'
  cards render with the correct health status/values, and assert the active-context indicator
  (e.g. `FederationContextSwitcher`'s active item or `session.activeContext`) is unchanged
  throughout the scenario. Keep this to one tight scenario per the feature brief — do not
  expand it into a multi-scenario suite.

**Checkpoint**: User Story 1 fully functional and independently testable — this is the MVP.

---

## Phase 4: User Story 2 - Spot GitOps drift across the fleet (Priority: P2)

**Goal**: Each connected context's card shows Flux sync status when Flux is installed, and no
GitOps indicator (not an error) when it isn't.

**Independent Test**: With one Flux-enabled context (failed reconciliation) and one non-Flux
context connected, open the Fleet Dashboard and confirm the Flux context shows "out of sync"
while the non-Flux context shows no Flux indicator.

### Tests for User Story 2

- [ ] T030 [P] [US2] Unit test in `src/backend/fleet.rs`: reducing a `FluxOverview{installed:
  false, ...}` yields `FluxRollupStatus::NotInstalled`; `installed: true` with every resource
  `Ready` yields `Synced`; `installed: true` with at least one non-`Ready` resource yields
  `OutOfSync` (data-model.md's exact reduction rule).

### Implementation for User Story 2

- [ ] T031 [US2] In `src/backend/fleet.rs`, implement `fn reduce_flux_overview(overview:
  &FluxOverview) -> FluxRollupStatus` per T030's rule, reusing `src/protocol/v1/flux.rs`'s
  existing `FluxOverview`/`FluxResourceSummary`/`FluxReadyState` types — no new Flux-detection
  logic (research.md R4).
- [ ] T032 [US2] In `src/backend/fleet.rs`'s `poll_once` (T017), call the same Flux-overview
  fetch logic `sync.rs::poll_flux_overview` uses (parameterized on the per-context client
  instead of the single active client) and set `FleetContextSummary.flux =
  Some(reduce_flux_overview(&overview))`; on any error fetching Flux state specifically (e.g.
  CRD list forbidden), fall back to `None` (data-model.md: "`None` == not yet known") rather
  than failing the whole poll attempt.
- [ ] T033 [US2] In `ui/src/components/fleet/FleetContextCard.tsx`, render a Flux indicator when
  `flux` is `Some(Synced | OutOfSync)`, distinguishable from the base health badge (FR-003); no
  indicator (or an explicit "n/a" only if design calls for it) when `flux` is `NotInstalled` or
  `None`.

**Checkpoint**: User Stories 1 AND 2 both work independently.

---

## Phase 5: User Story 3 - Keep watching the fleet without babysitting it (Priority: P3)

**Goal**: The dashboard refreshes automatically on an interval and reacts to contexts
connecting/disconnecting while open, without a manual reload.

**Independent Test**: Open the dashboard, wait one poll interval, confirm at least one card's
data updates without interaction; connect/disconnect a context via the existing switcher and
confirm the card list updates automatically.

### Tests for User Story 3

- [ ] T034 [P] [US3] Unit test in `src/backend/fleet.rs`: a context newly transitioning to
  `Connected` while pollers are already running (simulate by calling `start_polling` again
  after adding a context) gets its own new poller started, without disturbing existing
  pollers' handles (`FleetState.pollers` keeps prior entries, gains exactly one).
- [ ] T035 [P] [US3] Playwright e2e assertion (extend `us1-fleet-dashboard.spec.ts` minimally,
  or add a focused case) is NOT required beyond what T029 already covers per the feature
  brief's "keep it tight, one scenario" instruction — this phase's automated coverage is the
  backend unit test T034 plus the existing T016 handler test; do not add a second e2e spec file.

### Implementation for User Story 3

- [ ] T036 [US3] Confirm/adjust T019 (`start_polling`) and T022 (disconnect handling) so that
  the reactive triggers already used for `context_status::spawn_status_probes`
  (`Initialize`, `RefreshContexts`, `SetContext`) also call `fleet::start_polling`/the
  per-context stop path when the Fleet Dashboard's polling is currently active (track an
  `is_active: bool` or non-empty-`pollers`-as-proxy on `FleetState` so this is a no-op when the
  dashboard isn't open) — this is what makes FR-006 ("reflects connects/disconnects... without
  requiring a manual page reload") hold for contexts that connect *after* `StartFleetPolling`
  was sent, not just the initial set.
- [ ] T037 [US3] In `ui/src/components/fleet/FleetDashboardView.tsx`, confirm the store's
  reactive subscription to `session.contexts` naturally adds/removes rendered cards as the
  array changes (Zustand selector re-render) — add this explicitly if the current
  implementation from T027 only reads `session.contexts` once at mount rather than
  subscribing.

**Checkpoint**: All three user stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T038 [P] Run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; fix any
  findings in the new `fleet.rs`/`fleet.rs` (protocol) modules.
- [ ] T039 [P] `pnpm -C ui lint` and `pnpm -C ui typecheck`; fix any findings in the new fleet
  components/store files.
- [ ] T040 Run `cargo test`, `pnpm -C ui test`, and `ui/tests/e2e/us1-fleet-dashboard.spec.ts`;
  fix any failures.
- [ ] T041 Walk through `quickstart.md` manually (or via the e2e harness) end to end, confirming
  Constitution V: after closing the Fleet Dashboard view, no further `FleetSummaryUpdated`
  events are emitted (pollers actually stopped) — verify via logs/test assertion, not just code
  inspection.
- [ ] T042 Update `docs/MVP.md`/feature backlog docs if this repo's convention (seen in recent
  commits, e.g. spec 015) is to mark shipped features there.

---

## Dependencies & Execution Order

- **Setup (Phase 1)** → **Foundational (Phase 2)**: strictly sequential, blocks everything else.
- **User Story 1 (Phase 3)**: depends only on Phase 2. This is the MVP.
- **User Story 2 (Phase 4)**: depends on Phase 2 and on `poll_once`/`FleetContextSummary`
  existing from Phase 3 (T017), since it adds a field to the same struct's population logic —
  implement after Phase 3, not fully parallel with it.
- **User Story 3 (Phase 5)**: depends on Phase 3's `start_polling`/`stop_polling` (T019/T020)
  and the disconnect-handling in T022 — implement after Phase 3.
- **Polish (Phase 6)**: after all desired stories are complete.

### Parallel Opportunities

- T001–T003 (Setup) can run in parallel.
- T007, T008 (Phase 2, different files) can run in parallel; T004–T006 are sequential (same
  file, `fleet.rs` protocol module, building up one struct/two enums).
- T013–T016 (US1 tests) can run in parallel with each other, before their corresponding
  implementation tasks.
- T030 (US2 test) is independent of US1's remaining implementation once T017 exists.

## Implementation Strategy

**MVP first**: Phases 1–3 (Setup, Foundational, User Story 1) deliver the entire core value —
a live, view-only, multi-context health rollup. Ship/demo at that checkpoint before adding
Flux (Phase 4) and auto-refresh/reactivity polish (Phase 5), per the spec's own priority
ordering.
