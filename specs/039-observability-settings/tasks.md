---

description: "Task list for 039 observability settings"
---

# Tasks: Persist and Hydrate Observability Settings

**Input**: `/specs/039-observability-settings/` (plan.md, spec.md, research.md, data-model.md, contracts/)

**Tests**: Required (spec FR-011): backend unit and protocol tests, Vitest, fake-mode e2e, live e2e.

**Format**: `- [ ] T### [P?] [US?] Description with file path`

## Phase 1: Setup

- [x] T001 Add `secret_store.rs` seam in `src/backend/secret_store.rs`: `SecretStore` trait (`get/set/delete` by service+user), keyring impl (service `fathom.grafana`, user `<context>`, reuse the `Secret` wrapper pattern from `src/backend/oidc/store.rs`), and an in-memory impl for tests; register the module in `src/backend/mod.rs`
- [x] T002 [P] Add protocol types in `src/protocol/v1/cmd.rs` (`GetObservabilitySettings`, `ClearGrafanaKey`) and `src/protocol/v1/event.rs` (`ObservabilitySettings { context, prometheus: Option<..>, grafana: Option<{base_url, key_set}> }`, no secret field); regenerate ts-rs bindings into `ui/src/protocol/generated`
- [x] T003 Classify commands in `src/protocol/v1/mutation.rs`: `GetObservabilitySettings` read-only; `ClearGrafanaKey` mutating and mapped to the same integrations section keys as `ConfigureGrafana`; add mutation-table tests

## Phase 2: Foundational (blocks all stories)

- [x] T004 Per-context storage in `src/backend/integrations.rs`: add `by_context` map (`ContextIntegrations`), make `load(context)` honor its argument, read legacy top-level record as that context's settings, keep original file on unreadable input (Constitution VI); `GrafanaConfig.api_key` becomes read-only legacy (never serialized, `skip_serializing`)
- [x] T005 Legacy secret migration in `src/backend/integrations.rs`: a plaintext legacy `api_key` moves into the secret store on first read and is dropped on next save
- [x] T006 [P] Backend tests in `src/backend/integrations.rs` (test module): per-context isolation, legacy file loads, corrupt file falls back without deletion, legacy key migrates, no secret in serialized output

**Checkpoint**: storage and secret seam ready.

## Phase 3: User Story 1 - Grafana persists and shows configured (P1) MVP

- [x] T007 [US1] Implement `ConfigureGrafana` in `src/backend/cmd/integrations.rs`: persist URL for active context, non-empty key replaces stored key, blank keeps it, keyring failure keeps URL, reports `key_set: false` and emits an error notification (FR-010); remove the TODO stub
- [x] T008 [US1] Implement `GetObservabilitySettings` and `ClearGrafanaKey` handlers in `src/backend/cmd/integrations.rs`; emit `ObservabilitySettings` after read, configure, and clear
- [x] T009 [P] [US1] Backend tests in `src/backend/cmd/integrations.rs` using the in-memory store: save then read, blank key keeps, clear removes, keyring failure path, event never contains the key
- [x] T010 [US1] Store slice in `ui/src/store/` (`integrations.settings` keyed by context) and handler for `ObservabilitySettings`; ignore events whose `context` differs from the active context; request `GetObservabilitySettings` on panel open and on context change
- [x] T011 [US1] Update `ui/src/components/panels/GrafanaPanel.tsx`: prefill URL from store, "configured"/"not configured" indicator, key shown as set/unset (input never prefilled), blank key on save keeps existing, keep existing validation and gating
- [x] T012 [P] [US1] Vitest for the store handler and GrafanaPanel hydration in `ui/tests/unit/`
- [x] T013 [US1] Fake-mode e2e `ui/tests/e2e/observability-settings.spec.ts`: Grafana save then reopen, context switch isolation, phone and laptop, no horizontal scroll

**Checkpoint**: Grafana is usable end to end.

## Phase 4: User Story 2 - Prometheus form remembers its endpoint (P1)

- [x] T014 [US2] Emit `ObservabilitySettings` after `ConfigurePrometheus` and include saved Prometheus in reads in `src/backend/cmd/integrations.rs`
- [x] T015 [US2] Update `ui/src/features/metrics/PrometheusForm.tsx`: prefill saved namespace/service/port, indicator, defaults marked "not configured", keep validation
- [x] T016 [P] [US2] Vitest for PrometheusForm hydration in `ui/tests/unit/`
- [x] T017 [US2] Extend `ui/tests/e2e/observability-settings.spec.ts` with Prometheus prefill, edit, and re-save cases

## Phase 5: User Story 3 - Secrets stay in the credential store (P2)

- [x] T018 [US3] Add Clear key control to `ui/src/components/panels/GrafanaPanel.tsx` sending `ClearGrafanaKey`; indicator flips to "key not set"
- [x] T019 [P] [US3] Secrecy tests: protocol serialization test asserting no key field in any settings payload, log-capture test in `src/backend/cmd/integrations.rs` tests, and e2e assertion that the key text never appears in the DOM or sent frames (backend half done: protocol serialization and log-capture tests; e2e assertion done in ui/tests/e2e/observability-settings.spec.ts)
- [x] T020 [US3] Fake-mode e2e cases for clear key and keyring-unavailable error in `ui/tests/e2e/observability-settings.spec.ts`

## Phase 6: Polish and Live

- [x] T021 Live e2e in `ui/tests/live/` (kind-fathom-dev): save Grafana URL and key, reopen, key shown set, switch context isolation; devbridge pins one context and builds without os-keyring, so live covers URL persistence and re-select, key-set checks run only with a real store
- [x] T022 Protocol round-trip tests and ts-rs drift check pass (`cargo test --workspace`, regenerated bindings committed)
- [x] T023 Gate: `cargo fmt --check`, clippy `--workspace --all-targets`, `cargo test --workspace`, ui typecheck/lint/prettier/vitest, chromium e2e on phone/tablet/laptop/wide
  - Follow-up (gaps fixed): keyring write is now verified by read-back (FR-010); auto-discover no longer drops a legacy Grafana key; devbridge uses a debug-only memory secret store (FATHOM_DEV_MEMORY_SECRETS). Rust gate green; UI gate and live spec NOT run in this pass (no cluster, no node_modules).
  - 2026-09-26: full gate green. fmt, clippy -D warnings, cargo test --workspace (921 passed), typecheck, eslint (0 errors), prettier and vitest (1283) all pass. Playwright chromium phone/tablet/laptop/wide + webkit-laptop: 1658 passed; the 3 load flakes pass serially, and the 2 WebKit clipboard harness failures are fixed. See checklists/live-validation.md.
- [ ] T024 Run `quickstart.md` steps 1-5 including the manual secrecy grep of `~/.fathom/integrations.json` and logs
  - (Partial, 2026-09-26: steps 1-5 run and pass. Step 1's filter matched 0 tests and is corrected to `settings_tests` (17/17). Step 5 count-only greps found the key 0 times in integrations.json, the debug devbridge log and ~/.fathom. Step 4 did not cover switching to a *different* context or the real OS keyring, because the devbridge pins one context and uses the in-memory store; isolation is covered by unit and fake e2e only. See checklists/live-validation.md.)

## Dependencies

- T001 -> T004 -> T005 -> T007/T008. T002 and T003 precede T008 and T010.
- US1 (T007-T013) is the MVP; US2 depends only on Foundational and T010; US3 depends on US1.
- Parallel: T002 with T001; T006 with T005 finishing; T009, T012, T016, T019 alongside implementation of their story.

## Implementation Strategy

MVP is Phases 1-3 (Grafana persisted and hydrated). Then Prometheus (US2), then secrecy hardening (US3), then live e2e and gate. Fan out backend (T001-T009) and UI (T010-T013) across two agents once the protocol types (T002) land.
