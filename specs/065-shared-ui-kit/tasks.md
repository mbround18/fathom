# Tasks: Shared UI kit

**Input**: design documents in `specs/065-shared-ui-kit/`:
- plan.md
- research.md (R1–R13)
- data-model.md
- contracts/kit-api.md
- contracts/harness.md
- quickstart.md

**Tests**: requested (spec FR-016 to FR-019, SC-003, SC-007). Each piece has a unit test and an
isolated Playwright spec with axe.

**Agents**: do targeted checks only, one at a time, with `CARGO_BUILD_JOBS=4`. Never run
`scripts/ci.sh`, `paws`, `dagger` or a full Playwright suite of another app. Use signed gitops
commits. Tick tasks in a final, separate commit.

## Phase 1: Setup

- [X] T001 Add the `"./kit": "./src/kit/index.ts"` export and the `test`, `e2e` and `harness` scripts to `packages/ui/package.json`. Add the dev dependencies (workspace versions): vitest, jsdom, @testing-library/react, @testing-library/jest-dom, @testing-library/user-event, @playwright/test, vite, @vitejs/plugin-react, @tailwindcss/vite, tailwindcss, tw-animate-css, shadcn, and `@axe-core/playwright`, the one new package (owner-approved). Move `react-markdown`, `remark-gfm` and `@tanstack/react-virtual` into its `dependencies`. Run `pnpm install`.
- [X] T002 [P] Create `packages/ui/vitest.config.ts` (jsdom, forks pool, `VITEST_WORKERS`, mirroring `apps/keel/web/vitest.config.ts`) and `packages/ui/tests/unit/setup.ts`.
- [X] T003 [P] Create the harness per contracts/harness.md:
  - `packages/ui/vite.harness.config.ts`;
  - `packages/ui/harness/index.html`, `main.tsx` and `harness.css` (the same imports and base rules as `apps/keel/web/src/index.css`);
  - the piece index and the `?piece&state&theme` routing, with `window.__kitReady`.
- [X] T004 [P] Create `packages/ui/playwright.config.ts` (project `kit`, `KIT_E2E_PORT` defaulting to 4190, `autoWorkers()`, `webServer` running the harness) and `packages/ui/tests/support/a11y.ts` (`expectAccessible` looping light/dark × 360/1280 with AxeBuilder and the WCAG 2.2 AA tags, plus a no-sideways-scroll assertion, and `KIT_SHOTS` capture).
- [X] T005 Create `packages/ui/src/kit/index.ts` and `packages/ui/src/kit/provider.tsx` (`KitProvider`, `useKit`, and the `KitContext`, `GuideRef` and `GlossaryEntry` types from data-model.md), with a unit test in `packages/ui/tests/unit/provider.test.tsx`.

## Phase 2: Foundational (blocks every story)

- [X] T006 Add the Keel visual goldens **on the current code, before anything moves** (R12):
  - add the `s065` project to `apps/keel/web/playwright.config.ts`;
  - write `apps/keel/web/tests/e2e/s065/visual.spec.ts`, covering the runs list (empty and with rows), run detail with the cancel confirm dialog open, teams (forms), users (failed to load), help (Markdown, a guide link, an open term popover), settings tabs, the shell (approvals badge and update dot), and a page crash;
  - capture each at 360 and 1280 px, light and dark, with `toHaveScreenshot` (`maxDiffPixelRatio: 0`, animations disabled, the clock fixed);
  - generate with `--update-snapshots` and commit the goldens.
- [X] T007 Wire the kit into CI in `scripts/ci.sh` (R13): add `pnpm --filter @companion/ui test` and `e2e` to `job_shared`, and in `changed_slices`, map `packages/ui/**` to the shared, keel, depths and fathom slices.

## Phase 3: User Story 1 – Keel keeps working, from shared parts (P1) 🎯 MVP

**Goal**: every generic Keel piece lives in `@companion/ui/kit`, and Keel imports it from there.

**Independent test**:
- `s065` goldens pass unchanged;
- Keel's unit and e2e suites pass;
- `git diff` of `apps/keel/web/tests` shows import-path changes only, plus `s065`.

- [X] T008 [US1] Move the router and link into `packages/ui/src/kit/router.ts` and `Link.tsx`, with the event `companion:navigate` (R4). Delete `apps/keel/web/src/app/router.ts` and `Link.tsx`, and rewrite Keel's imports.
- [X] T009 [US1] Move the refusal type and `isTransportFailure` into `packages/ui/src/kit/refusal.ts`, with the `ListHandlers` and `ListLoad` types (R3). `apps/keel/web/src/app/load.ts` keeps `OFFLINE`, `refusalOf` and `loadList`, and re-exports the type.
- [X] T010 [US1] Move `RefusalText`, `LoadFailed`, `LoadState`, `ErrorLine`, `Page`, `PageErrorBoundary`, `NoMatch`, `EmptyState`, `empty-variant`, `Dialog`, `ConfirmDialog`, `CopyBlock`, `clipboard`, `reveal`, `form`, `Tabs` and `DataTable` from `apps/keel/web/src/ui/` into `packages/ui/src/kit/`, unchanged except imports. Coupled pieces take `guideHref` and `Link` from `useKit()`.
- [X] T011 [US1] Move `PageHeader`, `GuideLink`, `Markdown` and `help/slug.ts` into `packages/ui/src/kit/`. `GuideLink` renders nothing without `guideHref`. Markdown reads `isGuide` and `guideHref` from `useKit()` and stays a default export.
- [X] T012 [US1] Move `TermTip` into `packages/ui/src/kit/TermTip.tsx`, taking `term: string` from `useKit().glossary` (R6). Extend `apps/keel/web/tests/unit/glossary.test.ts` so every `term="…"` used in Keel's source is a `TermId`.
- [X] T013 [US1] Extract `NavShell` into `packages/ui/src/kit/NavShell.tsx` from the markup in `apps/keel/web/src/app/Shell.tsx` (R7, data-model Navigation). Keel's `Shell.tsx` keeps `AccountMenu`, `EnrolBanner` and the data wiring, and renders `NavShell`.
- [X] T014 [US1] Wrap Keel's root in `KitProvider` with `guideHref`, `isGuideSlug` and `GLOSSARY` in `apps/keel/web/src/app/App.tsx`. Delete `apps/keel/web/src/ui/` and every remaining moved file, then export every piece from `packages/ui/src/kit/index.ts` per contracts/kit-api.md.
- [X] T015 [US1] Check the move:
  - `pnpm --filter @companion/ui typecheck`;
  - Keel's `lint`, `typecheck`, `test` and `build`;
  - `e2e --project s065`;
  - then the full Keel e2e suite (`KEEL_E2E_PORT=4264`, `PLAYWRIGHT_WORKERS=2`).

  Fix the kit, never the goldens.

## Phase 4: Isolated piece tests (FR-016 to FR-019; cuts across stories, after US1)

Each task below writes `packages/ui/harness/pieces/<id>.tsx` (every state), `packages/ui/tests/unit/<id>.test.tsx` and `packages/ui/tests/e2e/<id>.spec.ts`, per contracts/harness.md. Keyboard and focus are asserted, and so is `expectAccessible` for every state.

- [ ] T016 [P] Pieces A, states and feedback:
  - `refusal-text` (denied, error, offline);
  - `load-failed` (with Retry, and without);
  - `load-state` (loading, failed, loaded);
  - `error-line` (with and without dismiss);
  - `empty-state` (guided, plain, restricted, page);
  - `no-match`;
  - `page-error-boundary` (thrown, with Reload);
  - `page`;
  - `page-header` (page, with intro and guide).
- [ ] T017 [P] Pieces B, interaction:
  - `dialog` (open and close, focus trap, return focus);
  - `confirm-dialog` (preview, affected, step-up, denied, done);
  - `copy-block` (copy, masked reveal);
  - `form` (field, label, select, panel);
  - `tabs` (arrow keys);
  - `markdown` (headings and anchors, a guide link, an image link, a table);
  - `guide-link` (with and without `guideHref`);
  - `term-tip` (opens on keyboard and pointer, Escape closes);
  - `router`/`link` (a plain click navigates, a modified click doesn't);
  - `nav-shell` (groups, badge, dot, the 360 px sheet).
- [ ] T018 Run impeccable over the harness: `KIT_SHOTS=<dir> pnpm --filter @companion/ui e2e`, then an impeccable audit and polish of every piece at 360 and 1280 px, light and dark (FR-018). Fix findings in the kit, once. If a fix changes Keel's look, regenerate Keel's `s065` goldens, and list the change in `CHANGELOG.md`.

## Phase 5: User Story 2 – Depths looks and behaves like Keel (P2)

**Goal**: Depths uses the kit's shell, header, states, dialogs and tables.

**Independent test**:
- Depths' unit and e2e suites pass;
- the new failed-to-load and crash specs pass.

- [ ] T019 [US2] Replace `apps/depths/web/src/app/router.ts` and `Link.tsx` with the kit's. Update `apps/depths/web/tests/e2e/us5-access/users.spec.ts` from `depths:navigate` to `companion:navigate`.
- [ ] T020 [US2] Wrap Depths' root in `KitProvider` in `apps/depths/web/src/app/App.tsx`, with no guides and no glossary. Rebuild `apps/depths/web/src/app/Shell.tsx` on `NavShell`, with grouped sections and the updates dot. Wrap every routed page in `PageErrorBoundary` in `apps/depths/web/src/app/routes.tsx` (FR-010).
- [ ] T021 [US2] Every Depths list read (stacks, files, sessions, passkeys, users, audit, network, updates):
  - map its HTTP and socket errors to the kit's `Refusal` in `apps/depths/web/src/protocol/` (a new `refusal.ts`), with `OFFLINE` in Depths' own words;
  - show `LoadState`/`LoadFailed` with Retry in each feature page under `apps/depths/web/src/features/*/` (FR-009).
- [ ] T022 [US2] Adopt the following in `apps/depths/web/src/features/*/`, keeping Depths' text and its existing `data-` markers:
  - `PageHeader` and `Page` on every page;
  - `EmptyState` and `NoMatch` where lists are empty or filtered;
  - `ConfirmDialog` for Depths' confirmations, with passkey step-up where the server asks for it;
  - `DataTable` for the tables;
  - `CopyBlock` for copyable values.
- [ ] T023 [US2] Add `apps/depths/web/tests/e2e/shell/kit-states.spec.ts`:
  - one failed load per list, each showing "Failed to load {what}." with the reason and Retry (SC-004);
  - one crashing page, showing the crash state with the navigation still usable (SC-005).
- [ ] T024 [US2] Run Depths' `lint`, `typecheck`, `test` and `build`, and its e2e suite one project at a time (`PLAYWRIGHT_WORKERS=2`). List Depths' visible changes under 065 in `CHANGELOG.md` (FR-011).

## Phase 6: User Story 3 – Fathom uses the shared pieces inside its panes (P3)

**Goal**: Fathom's panes use the kit's header, grid, states, confirm dialog and copy.

**Independent test**:
- Fathom's unit and e2e suites pass;
- the perf-10k spec passes;
- the new pane failed-to-load and pane crash specs pass.

- [ ] T025 [US3] Extend the kit's `DataTable` with the optional `sort`, `selection`, `rowKey` and `rowActions` props, and `Column.sortable` (data-model.md). Keel passes none of them, so `s065` must still pass. Add harness states (sortable, selectable, with actions) to `packages/ui/harness/pieces/data-table.tsx`, and specs in `packages/ui/tests/e2e/data-table.spec.ts`.
- [ ] T026 [US3] Build the kit's `DataGrid` in `packages/ui/src/kit/DataGrid.tsx`, with `column-budget.ts` moved from `ui/src/layout/column-budget.ts` (R8). It is a controlled, virtualized ARIA grid, with:
  - column priorities;
  - roving focus and keyboard movement;
  - long-press and ⋯ row actions;
  - sort and selection;
  - empty and failed slots;
  - Keel's header, row and status styling.

  Add a harness fixture with 10,000 rows. The spec asserts no more than about 60 rows in the DOM, keyboard navigation, sort, selection, actions, and axe.
- [ ] T027 [US3] Add `PageHeader` `size="pane"`, with `lead`, `count` and `rootProps` (R9). Add `typedConfirm` to `ConfirmDialog` (R9). Add harness states and spec cases for both.
- [ ] T028 [US3] Rewrite `ui/src/components/ResourceTable.tsx` as an adapter over `DataGrid`. The store, descriptors, compiled search, capabilities and `RowActionMenu` stay in Fathom. Keep every `data-testid` and role its suite uses.
- [ ] T029 [US3] Adopt the following in Fathom:
  - the kit's pane header in `ui/src/components/shell/PaneHost.tsx`, with `paneDragProps` on `rootProps` and `PaneSlotMenu` in `lead`;
  - the kit's `ConfirmDialog` (with `typedConfirm`) in place of `ui/src/components/ConfirmDialog.tsx`;
  - `CopyBlock` for copyable values;
  - `LoadFailed`, `EmptyState` and `NoMatch` in list panes;
  - a `PageErrorBoundary` around each pane, with `resetKey` set to the pane's view (FR-013).
- [ ] T030 [US3] Add `ui/tests/e2e/kit-panes.spec.ts`: one pane that fails to load shows the shared state while the others work, and one pane that throws is contained. Run `perf-10k.spec.ts` and `perf-containers-500.spec.ts`, then Fathom's `lint`, `typecheck` and `test`, then its e2e suite.

## Phase 7: User Story 4 – A developer builds a new screen from the kit (P3)

**Goal**: the kit is documented, and copies are caught.

**Independent test**:
- `pnpm -w run lint:packages` passes;
- a planted copy fails it.

- [ ] T031 [P] [US4] Write `scripts/check-kit-copies.mjs` (R10): it reads the export names from `packages/ui/src/kit/index.ts`, scans `apps/keel/web/src`, `apps/depths/web/src` and `ui/src`, and honours `packages/ui/kit-allow.json`. An allow-list entry must have a non-empty `reason`, and a stale entry fails. Hook it into the root `package.json` `lint:packages`, and add `packages/ui/tests/unit/check-kit-copies.test.ts` with fixture directories.
- [ ] T032 [P] [US4] Add a "Shared UI kit" section to `CONTRIBUTING.md`, with one line per piece on when to use it, a short example for empty, failed-to-load and confirm, how to run the harness and a single piece's spec, the `KitProvider` fields, and the allow-list rule (FR-014).

## Phase 8: Polish and the gate

- [ ] T033 Run the `s065` goldens again after every merge. Run `scripts/hygiene.sh`.
- [ ] T034 Main session, once: `scripts/ci.sh all`. Report the wall time, the slowest steps, the change from the last pass, and one idea to speed it up.
- [ ] T035 Before handing the walk-through to the owner, check `CHANGELOG.md` for 065 against what shipped.

## Dependencies

- Setup (T001–T005) comes first. Then foundational: T006 (goldens on the old code) and T007.
- US1 (T008–T015) is sequential, in one agent. It blocks everything after.
- After US1, these run in parallel:
  - T016 and T017;
  - US2 (T019–T024);
  - T025–T027 (kit table and header extensions);
  - US4 (T031 and T032).
- T018 (impeccable) follows T016, T017 and T025–T027.
- T028–T030 (Fathom) follow T025–T027.
- T033–T035 come last.

## Parallel waves

| Wave | Who | Tasks |
|---|---|---|
| 0 | main | T001–T007 |
| 1 | agent A | T008–T015 |
| 2 | agents B, C, D, E | B: T016 + T017 · C: T019–T024 · D: T025–T027 · E: T031 + T032 |
| 3 | agent F, then main | F: T028–T030 · main: T018 |
| 4 | main | T033–T035 |

## Implementation strategy

The MVP is US1: the kit exists, Keel uses it, and nothing visible moved. Each later wave lands on
main in signed commits as it passes its targeted checks.
