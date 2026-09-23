# Tasks: Multi-Cluster Context Federation & Optional In-Cluster Daemon — P1

**Input**: plan.md, research.md, data-model.md, contracts/context-status.md, quickstart.md in this
directory. **Scope**: P1 (User Story 1) only — see plan.md.

**Tests**: explicitly required by the task brief (protocol round-trip, backend unit tests, Vitest, Playwright e2e).

## Phase 1: Setup

- [ ] T001 Confirm no new Cargo/npm deps needed: re-check `Cargo.toml` (`kube` has `apiserver_version`) and
  `ui/package.json` — no edits expected, just verification before coding starts.

## Phase 2: Foundational (blocking prerequisites for US1)

- [ ] T002 Add `ContextConnectionStatus` fieldless enum (`Connected | Connecting | Error | NotConnected`,
  `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS`, matching `NotificationLevel`'s pattern)
  and `SystemEvent::ContextStatusUpdated { context: String, status: ContextConnectionStatus, error:
  Option<String> }` in `src/protocol/v1/event.rs`.
- [ ] T003 [P] Add a serde round-trip test for `ContextStatusUpdated` (both `Error{Some(msg)}` and
  non-error `None` shapes) in `src/protocol/v1/event.rs`'s `#[cfg(test)] mod tests` (create the mod if it
  doesn't exist yet in that file; otherwise extend it), per Constitution III.
- [ ] T004 Regenerate TS bindings (`ts-rs`) so `ui/src/protocol/generated/SystemEvent.ts` and a new
  `ContextConnectionStatus.ts` exist — run the project's existing binding-generation step (`cargo test` in
  this repo runs the `ts-rs` export tests that write `ui/src/protocol/generated/`; confirm via `git status`
  after `cargo test`).
- [ ] T005 Make `load_kubeconfig` in `src/backend/kube.rs` `pub(super)` (currently private) so
  `context_status.rs` can reuse it.
- [ ] T006 Change `load_and_send_contexts` in `src/backend/kube.rs` to return
  `anyhow::Result<(Vec<String>, Option<String>)>` (the same `(contexts, current)` tuple it already computes
  internally), updating its 3 call sites in `src/backend/cmd/cluster.rs` (`RefreshContexts` handler,
  `initialize()`, `set_context()`) to capture the tuple instead of discarding it with `let _ =`.

## Phase 3: User Story 1 — See and switch between every cluster context at a glance (P1)

**Goal**: every kubeconfig context is always visible with a live connected/connecting/error/not-connected
status; switching still uses today's reconnect behavior; kubeconfig reloads update the list without a
restart.

**Independent Test**: open Fathom against a kubeconfig with 3 contexts (one unreachable); all 3 show in a
persistent list with live status; switching updates status and resource views; the unreachable one's error
is visible without switching to it first.

### Backend

- [ ] T007 [US1] Create `src/backend/context_status.rs`: `pub(super) fn spawn_status_probes(state:
  &BackendState, sender: &Sender<BackendEventEnvelope>, contexts: &[String], active: Option<&str>)` —
  for every `contexts` entry `!= active`, `tokio::spawn` a task that emits `ContextStatusUpdated{Connecting}`
  immediately, then builds a short-lived `Client` for that named context via `load_kubeconfig()` +
  `Config::from_custom_kubeconfig` + `KubeConfigOptions{context: Some(name)}` (mirrors
  `connect_client_for_current_context` in `kube.rs`), calls `client.apiserver_version()` wrapped in
  `tokio::time::timeout(Duration::from_secs(5), ..)`, and emits `NotConnected` on success or
  `Error{Some(format!("{e:#}"))}` on failure/timeout (research.md R1/R2/R3). Register the new module in
  `src/backend/cmd.rs` or wherever `mod` declarations for `backend/*.rs` live (check `src/backend.rs`'s
  `mod` list) — do not duplicate module wiring.
- [ ] T008 [US1] In `src/backend/cmd/cluster.rs`'s `initialize()`: after computing which context will be
  active (before `connect_and_share_client`), emit `ContextStatusUpdated{context: <target>, Connecting,
  None}`; after `connect_and_share_client` resolves, emit `Connected` on success or
  `Error{Some(format!("{e:#}"))}` on failure (still propagate the `Err` via `?` afterward so the existing
  `run_cmd` error/`CommandFinished` path is unaffected — see `src/backend.rs::run_cmd`). Then call
  `context_status::spawn_status_probes(state, sender, &contexts, current.as_deref())` using the tuple from
  T006's `load_and_send_contexts` call.
- [ ] T009 [US1] Apply the same Connecting/Connected/Error bracketing plus `spawn_status_probes` call to
  `set_context()` in `src/backend/cmd/cluster.rs`, around its own `connect_and_share_client` call and its
  post-switch `load_and_send_contexts` reload.
- [ ] T010 [US1] Wire the plain `RefreshContexts` branch in `src/backend/cmd/cluster.rs`'s `handle()` to
  also call `context_status::spawn_status_probes` with the returned tuple (no connect bracketing needed
  here — the active context isn't reconnecting, only the list is refreshed).
- [ ] T011 [P] [US1] Add `src/backend/context_status.rs` unit tests (no live cluster): (a) given a
  `contexts` slice and `active`, assert `spawn_status_probes` never emits any event synchronously for the
  `active` entry (only for the others) — test the emission-skip logic as a pure function extracted from the
  spawn wrapper if needed for testability (e.g. a `fn contexts_to_probe(contexts: &[String], active:
  Option<&str>) -> Vec<String>` helper you can unit test directly without spawning tasks); (b) a
  fixture-based test that `Error` status events always carry `Some(_)` and non-`Error` always carry `None`
  (construction-level invariant from data-model.md).

### Frontend

- [ ] T012 [US1] Add `contextStatuses: Record<string, { status: ContextConnectionStatus; error?: string }>`
  to `SessionState` in `ui/src/store/index.ts`, initialized to `{}` in the store's initial `session` value.
- [ ] T013 [US1] Add a `case 'ContextStatusUpdated':` to `handleSystem` in
  `ui/src/store/handlers/system.ts`: `session.contextStatuses = { ...session.contextStatuses, [event.data.context]:
  { status: event.data.status, ...(event.data.error ? { error: event.data.error } : {}) } }` (last-write-wins
  per data-model.md — no reconciliation needed).
- [ ] T014 [P] [US1] Add `ui/tests/store/handlers/system.federation.test.ts` (Vitest, follow the existing
  pattern of sibling `system.*.test.ts`/handler test files in that directory): feed a `ContextStatusUpdated`
  envelope through the handler and assert `session.contextStatuses` updates correctly, including the
  error-only-on-Error-status invariant and last-write-wins across two envelopes for the same context.
- [ ] T015 [US1] Create `ui/src/components/shell/FederationContextSwitcher.tsx`: always-rendered `<nav
  aria-label="Cluster contexts" data-testid="federation-context-switcher">`; renders nothing (`null`) when
  `session.contexts` is empty; otherwise a horizontally-scrollable (`overflow-x-auto`, no page-level
  scroll — Constitution IV) list of buttons, one per context, each `data-testid="federation-context-item"`,
  `data-context-name={name}`, `data-context-status={<snake_case of the status>}` (`connected` |
  `connecting` | `error` | `not_connected`, defaulting to `not_connected` when `contextStatuses[name]` is
  absent per data-model.md), `aria-pressed={name === activeContext}`, a `title` attribute with the error
  text or a human status label, and a `federation-context-status-dot` `aria-hidden` indicator span. Clicking
  calls the store's existing `selectContext` action (same one `ContextPicker` already uses) — no new
  command type. When `data-context-status="error"`, also render a `federation-context-error` element
  containing the verbatim error text (contract: "available on inspection... without switching",
  `context-status.md`).
- [ ] T016 [US1] Mount `<FederationContextSwitcher />` in `ui/src/components/shell/AppShell.tsx`, directly
  below `<TopBar />` and above `<main>`, at every width class (do not gate it behind `narrow`/`SlotGrid`
  branching — it must render identically at phone/tablet/wide/ultra per Constitution IV and research.md R4).
  Leave `TopBar.tsx`'s existing `<ContextPicker />` untouched.

### Tests (boundary coverage, Constitution III)

- [ ] T017 [US1] Extend `ui/tests/fixtures/batches.ts` (or add alongside `twoContexts`) with a fixture
  batch that includes `ContextStatusUpdated` events for a 3-context scenario (one `Connected`, one
  `NotConnected`, one `Error` with a distinctive message) for e2e use; wire it into
  `ui/tests/fixtures/index.ts`/`scenarios.ts` following the existing `twoContexts` pattern, without
  modifying any existing scenario's current behavior.
- [ ] T018 [US1] Create `ui/tests/e2e/us1-federation-context-status.spec.ts` (naming per task brief) using
  `ui/tests/e2e/helpers.ts` conventions: (a) all contexts appear in `federation-context-switcher` without
  any open/click action (FR-001); (b) each `federation-context-item`'s `data-context-status` matches the
  fixture (FR-002); (c) the `error`-status item exposes its message via `federation-context-error` without
  switching to it (FR-004); (d) clicking a different context sends `SetContext` and updates
  `data-context-status`/`aria-pressed` on both the old and new active items, mirroring the assertions in
  `us1-context-switch.spec.ts` (FR-003); (e) re-emitting `ContextsUpdated` with an added/removed context
  updates the strip without a reload (FR-005). Verify `us1-context-switch.spec.ts` still passes unmodified.

## Phase 4: Polish

- [ ] T019 Run `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui
  typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, and the new e2e spec; fix anything red.
- [ ] T020 Update `docs/MVP.md`'s §14 Tier D (or the relevant status row) to reflect P1 shipped, P2-P4
  deferred — precise, not overstated.

## Dependencies

- Phase 1 → Phase 2 → Phase 3 (US1) → Phase 4. Within Phase 3, backend tasks T007-T011 can proceed in
  parallel with frontend tasks T012-T016 once T002-T004 (protocol + generated bindings) land, since the
  frontend only needs the generated types, not the backend logic, to be typed correctly — but e2e (T018)
  needs both sides done.
- T003, T011, T014 are `[P]` (independent test files) relative to their sibling implementation tasks.

## Deferred

P2 (simultaneous federation + aggregate overview), P3 (cloud auth-plugin shellouts), P4 (optional
in-cluster daemon) are out of scope for this tasks.md — see spec.md; to be planned separately if/when
pursued.
