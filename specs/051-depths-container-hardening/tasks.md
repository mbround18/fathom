# Tasks: Depths container hardening (051)

**Input**: spec.md, plan.md, research.md, data-model.md, contracts/
**Tests**: required (constitution III). Gate on targeted tests only; the main session runs `scripts/ci.sh` once.

## Phase 1: Setup

- [x] T001 Add `serde_yaml`/dev deps where needed in crates/depths-core/Cargo.toml and crates/depths-runtime-cli/Cargo.toml

## Phase 2: Foundational

- [x] T002 Profile types, `profile()`, `rollup`, `DaemonMode`, `ShellPolicy` and table tests in crates/depths-core/src/security.rs
- [x] T003 Batched security inspect and daemon-mode detection in crates/depths-runtime-cli/src/lib.rs
- [x] T004 Hardened exec argv (no `--privileged`, no client `--user`/`--workdir`, validated shell path and id) in crates/depths-runtime-cli/src/lib.rs and crates/container-runtime/src/session.rs
- [ ] T005 Protocol types for ratings, lint findings, refusal kinds and `acknowledge_findings` in crates/depths-protocol/src/{types,cmd,event}.rs; regenerate apps/depths/web/src/protocol/generated
- [ ] T006 `DEPTHS_SHELL_POLICY` / `DEPTHS_COMPOSE_POLICY` parsing (fail to start on invalid) in apps/depths/server/src/config.rs

## Phase 3: US1 Ratings (P1)

- [ ] T007 [US1] Attach profiles to inventory containers and stack roll-up in apps/depths/server/src/inventory (or equivalent) with daemon mode and admin rootful notice
- [ ] T008 [P] [US1] SecurityChip component with reasons tooltip in apps/depths/web/src/components/SecurityChip.tsx
- [ ] T009 [US1] Chips on container rows, console picker and stack list in apps/depths/web/src/features/
- [ ] T010 [P] [US1] vitest for chip text/reasons; Playwright chip assertions in apps/depths/web/tests/e2e/us4-shells/

## Phase 4: US2 Shell guard (P1)

- [ ] T011 [US2] Pure `shell_decision(policy, level, is_admin)` with table test in crates/depths-core/src/security.rs
- [ ] T012 [US2] Fresh inspect + decision after `permits` in the shell-open path in apps/depths/server/src/streams.rs; fail closed on inspect error
- [ ] T013 [US2] Audit `shell.open` / `shell.refused` in apps/depths/server/src/streams.rs
- [ ] T014 [US2] End shells when their container is recreated or destroyed (FR-014) in apps/depths/server/src/streams.rs
- [ ] T015 [US2] Server tests per level × policy × role and sweep rows in apps/depths/server/tests/sweep.rs
- [ ] T016 [US2] UI mirrors the decision (disabled control with reason) and fake backend support in apps/depths/web/tests/support/fake-backend.ts

## Phase 5: US3 Compose lint (P2)

- [ ] T017 [US3] Lint findings from parsed compose in crates/depths-core/src/compose_lint.rs with table tests
- [ ] T018 [US3] `LintCompose` command and lint before save/up; enforce blocks escape findings unless admin acknowledges (audited) in apps/depths/server/src/ws.rs
- [ ] T019 [US3] Lint panel in the editor and acknowledgement dialog in apps/depths/web/src/features/
- [ ] T020 [US3] Playwright: warn shows findings; enforce blocks user, admin acknowledgement succeeds

## Phase 6: Polish

- [ ] T021 docs/guides/depths/security.md (levels, policies, lint, rootless daemon guide) and contracts/config.md in specs/050
- [ ] T022 `.env.example` entries for both policies in apps/depths/.env.example

## Dependencies

T002–T006 before all stories. US1 and US2 share T005; US3 independent after T005/T006. 052's network gate composes with T012.
