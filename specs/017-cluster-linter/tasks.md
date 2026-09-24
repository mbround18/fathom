# Tasks: Cluster Sanitizer / Linter

**Input**: spec.md, plan.md, research.md, data-model.md, contracts/linter-rules-module.md
Format: `- [ ] T### [P?] [US?] description (file)`. [P] = parallelizable (different files, no dependency).

## Phase 1: Setup / Protocol

- [ ] T001 Add `ToolKind::Linter` (additive) in `src/protocol/v1/ui_config.rs`; add round-trip coverage in `tests/protocol_serde.rs`; regenerate ts-rs bindings (`cargo test -p fathom --test export_bindings`)
- [ ] T002 Wire `linter` ToolKind in the frontend: `TOOL_LABEL` (labels.ts), `TOOL_KINDS` + dock default tab (layout/defaults.ts, tests/unit/layout-defaults.test.ts), `LINTER_TAB` (stack-tabs.ts, TabletTabs.tsx)
- [ ] T003 Add dormant sections (configmaps, secrets, poddisruptionbudgets, jobs, cronjobs, replicasets, statefulsets, daemonsets) to `SECTION`/`REFRESH` and `Select*` cases (StatefulSet, DaemonSet, ReplicaSet, PodDisruptionBudget) to `selectCmd` in `ui/src/protocol/commands.ts`; unit test

## Phase 2: Foundational (blocks all rules)

- [ ] T004 Create `ui/src/linter/types.ts` (Finding, Severity, Category, RuleId, RuleResult, LintReport, LintInput) and `ui/src/linter/util.ts` (safe `raw_json` parse, list/detail helpers, container extraction)
- [ ] T005 Create `ui/src/linter/evaluate.ts` aggregator (counts, score, skipped/pending) and `ui/src/linter/fetchPlan.ts` (detail targets, throttled batch selection)

## Phase 3: User Story 1 - findings at a glance (P1) MVP

- [ ] T006 [P] [US1] Rule `rules/resourceLimits.ts` + `tests/unit/linter/resource-limits.test.ts`
- [ ] T007 [P] [US1] Rule `rules/probes.ts` + test
- [ ] T008 [P] [US1] Rule `rules/nakedPod.ts` + test
- [ ] T009 [P] [US1] Rule `rules/rbacWildcard.ts` + test
- [ ] T010 [P] [US1] Rule `rules/missingPdb.ts` + test
- [ ] T011 [P] [US1] Rule `rules/unusedConfig.ts` + test
- [ ] T012 [P] [US1] Rule `rules/unusedServiceAccount.ts` (always skipped) + test
- [ ] T013 [P] [US1] `deprecatedApiTable.ts` + rule `rules/deprecatedApi.ts` + test
- [ ] T014 [US1] `evaluate` + `fetchPlan` unit tests (`tests/unit/linter/evaluate.test.ts`, `fetch-plan.test.ts`)
- [ ] T015 [US1] `components/panels/LinterPanel.tsx`: mount refreshes, throttled detail fetch, live recompute, clean/skipped/pending states; dispatch in `PaneHost.tsx`

## Phase 4: User Story 2 - click-through (P1)

- [ ] T016 [US2] Finding rows call `openInspector(ref)` (namespaced and cluster-scoped); e2e `tests/e2e/us1-linter-findings.spec.ts` (open pane, finding appears, click opens Inspector)

## Phase 5: User Story 3 - triage filters (P2)

- [ ] T017 [US3] Severity + category filters and summary counts/score in `LinterPanel.tsx`; unit test for filter helper (`linter/filter.ts`)

## Phase 6: Polish

- [ ] T018 Read-only audit: assert LinterPanel/linter module reference only `Refresh*`/`Select*` commands (unit test); full `cargo test`, clippy, `pnpm typecheck/lint/test`, Playwright chromium
- [ ] T019 Mark shipped in `docs/MVP.md`

## Dependencies

T001 -> T002; T003, T004 -> T005 -> rules (T006-T013 parallel) -> T014 -> T015 -> T016 -> T017 -> T018 -> T019.
