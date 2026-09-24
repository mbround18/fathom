---
description: "Task list for Managed-Cloud Auth Plugins"
---
# Tasks: Managed-Cloud Auth Plugins

**Input**: specs/035-cloud-auth-plugins/. Tests are mandatory (Constitution III). [P] = parallel, [USn] = story.

## Phase 1: Setup
- [X] T001 Create `src/backend/cloud_auth/` skeleton and register in `src/backend/mod.rs`
- [X] T002 [P] Create `src/protocol/v1/cloud_auth.rs` skeleton and register in `mod.rs`
- [X] T003 [P] Add fake-plugin test binary (emits ExecCredential JSON, counts calls; modes ok/fail/hang/expired) under `tests/support/`
- [X] T004 Spike: confirm kube 2.0.1 mechanism to inject an externally obtained token (research R1); record in research.md

## Phase 2: Foundational
- [X] T005 Define protocol types from data-model.md (`CloudProvider`, `CloudAuthInfo`, `PluginError`, `ExecPolicy`, event, command) with serde round-trip tests
- [X] T006 Regenerate TS bindings; verify drift check passes
- [X] T007 Implement `redact.rs` (token-shape redaction, truncation) with unit tests
- [X] T008 Implement `exec.rs` runner (argv only, no TTY, timeout, output caps, Windows CREATE_NO_WINDOW, GUI PATH augmentation) with tests

## Phase 3: US1 Actionable errors (P1)
- [X] T009 [US1] Map failures to `PluginError` kinds in `exec.rs` with a test per kind
- [X] T010 [US1] Wire into the kube connect path (`kube.rs`) and `context_status.rs` probe
- [X] T011 [US1] Wire into `fleet.rs::poll_inner`; distinguish plugin error from 401/403; tests
- [X] T012 [US1] UI: render typed error with hint in status/inspector; Vitest + Playwright e2e (missing plugin)

## Phase 4: US2 Detection (P2)
- [X] T013 [US2] `detect.rs`: classify eks/gke/aks/generic/none, resolve binary without spawn; fixtures + tests
- [X] T014 [US2] Emit `CloudAuthDetected` on Initialize/RefreshContexts; UI store handler
- [X] T015 [US2] UI: provider/installed badge in sidebar tooltip + inspector with copyable install hint; e2e

## Phase 5: US3 Caching (P2)
- [X] T016 [US3] `cache.rs`: identity hash, expiry+skew, default TTL, single-flight, secret wrapper; unit tests
- [X] T017 [US3] Route fleet/probe/connect through cache; 401 invalidation once; drop entries on kubeconfig reload
- [X] T018 [US3] Integration test: N polls => 1 spawn; past expiry => 2; concurrency => 1

## Phase 6: US4 Safety (P3)
- [X] T019 [US4] `ExecPolicy` persistence (additive config, defaults on invalid) + `SetExecPolicy` command + tests
- [X] T020 [US4] Enforce allowlist -> `pluginBlocked`; UI opt-in control; e2e
- [X] T021 [US4] Interactive-mode detection -> `pluginInteractiveUnsupported`
- [X] T022 [US4] Leak test: fixture token absent from logs/events/MCP/export
- [X] T023 [US4] Confirm MCP path uses the same policy and cannot trigger arbitrary exec

## Phase 7: Polish
- [X] T024 Justify/add any new deps (`which`, `secrecy`) in research.md
- [ ] T025 Run quickstart manual checks on Ubuntu 26.04, Windows 11, macOS 26
- [X] T026 Update docs/MVP.md Tier D P3 status
- [X] T027 fmt, clippy -D warnings, cargo test, UI lint/typecheck/vitest/playwright green
- [ ] T028 (last, per project convention) brochure screenshots of new UI surfaces

## Dependencies
Phases 1-2 block all. US1 first (MVP); US2 and US3 parallel after Phase 2 (US3 needs T008); US4 after US3 (T017). T028 last.
