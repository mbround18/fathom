---
description: "Task list for Simultaneous Multi-Context Federation"
---

# Tasks: Simultaneous Multi-Context Federation

**Input**: specs/034-simultaneous-federation/ (plan, spec, research, data-model, contracts, quickstart)
**Tests**: Included per Constitution III.
**Format**: `[ID] [P?] [Story] Description` ([P] = parallelizable, different files)

## Phase 1: Setup

- [X] T001 Create `src/protocol/v1/federation.rs` skeleton and register in `src/protocol/v1/mod.rs`
- [X] T002 [P] Create `src/backend/federation.rs` skeleton and register in `src/backend/mod.rs`
- [X] T003 [P] Create `ui/src/components/fleet/` federation files and `ui/src/store/handlers/federation.ts` placeholders
- [X] T004 [P] Verify the `kube` `runtime` feature in `Cargo.toml`; enable if missing and record in research.md R2

## Phase 2: Foundational (blocks all stories)

- [X] T005 Define `FederationConnectionState`, `FederationContextRollup`, `FederationCombinedRollup` in `src/protocol/v1/federation.rs` with ts-rs derives
- [X] T006 Add `FederateContext`, `UnfederateContext`, `RequestFederationSnapshot` to `src/protocol/v1/cmd.rs` and `FederationStateUpdated`, `FederationRollupUpdated` to `src/protocol/v1/event.rs`
- [X] T007 [P] Serde round-trip tests for every new variant in `tests/protocol_serde.rs`; regenerate bindings and confirm the drift check
- [X] T008 Extract shared helpers (client construction, unhealthy-pod classifier, concurrency cap, stagger) from `src/backend/fleet.rs` into reusable functions with no behavior change; the 027 test suite MUST stay green (done additively: shared helpers exposed from fleet.rs, poll_inner untouched to avoid conflicts with 035)
- [X] T009 Add `FederationState` to `src/backend/state.rs` and the `ContextTier` arbiter shared with `FleetState`
- [X] T010 [P] Add `federation.json` load/save in `src/backend/persistence.rs` (atomic, additive, empty fallback on parse error, names only) with tests including corrupt-file preservation
- [X] T011 Dispatch the new commands in `src/backend/cmd/cluster.rs`
- [X] T012 [P] Add `federation` store slice and handler skeleton in `ui/src/store/` with generation-based stale-event dropping and Vitest tests

**Checkpoint**: protocol, state and persistence are in place.

## Phase 3: User Story 1 - Live cross-cluster overview (P1) MVP

- [X] T013 [P] [US1] Unit tests for counter reducers (pods by phase, unhealthy via 027 classifier, nodes, deployments, warning events) in `src/backend/federation.rs`
- [X] T014 [P] [US1] Unit tests (tokio paused time) for 500ms debounce and exponential backoff with jitter
- [X] T015 [US1] Implement per-context live tier in `src/backend/federation.rs`: persistent client, watchers for Pods/Nodes/Deployments/Events reduced to counters, relist on 410, RBAC-denied metrics marked `restricted`
- [X] T016 [US1] Implement state machine and reconnect/backoff, verbatim errors, no stale numbers as live (FR-004, FR-005)
- [X] T017 [US1] Implement combined rollup (Live contexts only) and emit `FederationRollupUpdated` on change only
- [X] T018 [US1] Implement `FederateContext`/`UnfederateContext` handlers: validate context, bump generation, abort tasks and drop client on removal, emit `FederationStateUpdated`
- [X] T019 [P] [US1] Build `FederationView.tsx`, `FederationRow.tsx`, `FederationToggle.tsx` inside the Fleet dialog (mobile-first, no horizontal scroll, keyboard/touch operable, tokens per theme)
- [X] T020 [US1] Wire `RequestFederationSnapshot` on mount and handlers into the store
- [X] T021 [US1] Playwright e2e `ui/tests/e2e/us1-federation-live.spec.ts`: two contexts, injected rollup update, one context errors while the other stays live, primary context unchanged
- [X] T022 [US1] Test that an empty federated set spawns no clients or tasks (SC-003)

**Checkpoint**: US1 is independently demonstrable.

## Phase 4: User Story 2 - Reuse the fleet rollup (P2)

- [X] T023 [US2] Implement tier arbitration: federating stops that context's 027 poller; un-federating resumes it if the dialog is open; unit tests for both transitions and no double-poll
- [X] T024 [US2] Emit `FleetSummaryUpdated` (with additive `live` marker in `src/protocol/v1/fleet.rs`) for federated contexts; update round-trip tests
- [X] T025 [P] [US2] Show a live/polled indicator on `FleetContextCard.tsx`; Vitest for both states
- [X] T026 [US2] Playwright e2e `ui/tests/e2e/us2-federation-fleet.spec.ts`: one live card, one polled card, fallback on un-federate without duplicate cards

## Phase 5: User Story 3 - Persisted, bounded (P3)

- [X] T027 [US3] Restore the persisted set on `Initialize` (reconnect only previously chosen contexts, staggered using the 027 cap); show unavailable state for contexts missing from kubeconfig, prune only on operator action
- [X] T028 [US3] Soft-cap warning: `over_cap` in `FederationStateUpdated` and a UI banner without auto-disconnect; tests
- [X] T029 [P] [US3] Playwright e2e `ui/tests/e2e/us3-federation-persist.spec.ts`: restore after restart, over-cap warning, no-access metric rendering

## Phase 6: Polish

- [ ] T030 [P] Idle-CPU/soak check with 5 federated contexts against local kind (SC-002); record the result in research.md
- [ ] T031 [P] Reachability recovery test (SC-004) against a stopped/started kind control plane
- [X] T032 Update `docs/MVP.md` §14 Tier D and the status table (P2 shipped) and note federation in the fleet docs
- [X] T033 Run quickstart.md end to end (gates only; not exercised against a live cluster); `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui lint typecheck test`, Playwright
- [ ] T034 Brochure/marketing screenshots of the Federation view (final step of the batch)

## Dependencies

- Phase 2 blocks all stories. T008 blocks T015 and T023. T005/T006 block T007, T011, T015.
- US1 (T013-T022) is the MVP. US2 depends on US1's live tier (T015). US3 depends on T010 and T018.
- Parallelizable groups: T002/T003/T004; T007/T010/T012; T013/T014/T019; T025; T029/T030/T031.
