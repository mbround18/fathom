# Tasks: Cascading wide-screen layout

Everything is UI-only (`ui/`). There are no protocol or backend changes. `[P]` marks tasks that touch different files and can run in parallel. Test tasks come first in each story and must fail before the implementation.

Input: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/ui-cascade.md`, `quickstart.md`.

## Phase 1: Foundations (pure logic, no UI)

- [x] T001 [P] Add the cascade algebra in `ui/src/layout/cascade.ts`: `resetTo(sectionPaneId)`, `pushStep(cascade, paneId)` (a paneId already in the chain truncates to it instead of duplicating), `truncateAt(cascade, index)`, and `slotsFor(cascade, navPaneId, slotCount, spare)` returning `[navPaneId, cascade[0], ...cascade.slice(1).slice(-(slotCount - 2))]` — the navigator and the section list are pinned (FR-001), so only the drill-in steps scroll — padded from `spare` (the free empty inspector panes) to `slotCount`. Export `CASCADE_FIRST_SLOT = 2`. Pure functions only, no React and no store import
- [x] T002 [P] Unit tests for the cascade algebra in `ui/tests/unit/cascade.test.ts`: append, truncate on an earlier click, reset on a section click, the shift-left window when the steps are more numerous than `slotCount - 2`, with the navigator and the section list still in slots 0 and 1, re-revealing earlier steps when `slotCount` grows, ids stay unique, and a chain shorter than the slots leaves the tail empty
- [x] T003 [P] Add `mostRelevantRow(descriptor, rows)` in `ui/src/resources/relevance.ts`: `undefined` for an empty list; otherwise the first row whose tone is `err`, else the first whose tone is `warn`, else `rows[0]`. Tone is `statusTone(statusColumn.text(row)).tone` from `ui/src/resources/search.ts`, where `statusColumn` is the descriptor's column with `status: true`; with no status column every row counts as `ok`. Returns a `RowKey`
- [x] T004 [P] Unit tests for `mostRelevantRow` in `ui/tests/unit/relevance.test.ts`: an `err` row beats an earlier `warn`, a `warn` beats a later `ok`, all-`ok` gives the first row, no status column gives the first row, `muted` counts as not-unhealthy, an empty list gives `undefined`, and the list's current order (sorted or filtered) is what decides
- [x] T005 Change the default profiles in `ui/src/layout/defaults.ts` per data-model.md: `wide` → `[nav, list, inspector, inspector]`, `ultra` → `[nav, list, inspector, inspector, inspector]`, `ULTRA_PLUS_SLOT` → `{ kind: 'inspector' }`, keeping today's `sizePct` values at each index so `normalize` output is unchanged. Do not touch `phone`, `tablet`, `laptop`, `desktop`, `visibleSlots`, `hasDock` or the dock defaults
- [x] T006 Add the saved-profile normalisation to `resolveProfile` in `ui/src/layout/defaults.ts`: for `wide` and `ultra` only, index 0 resolves to `nav`, index 1 to `list` and every index ≥ 2 to a cascade slot (`kind: 'inspector'`, no `tool`), each keeping the saved slot's `sizePct`. Keep the existing validate → truncate → renormalise → dock-filter contract, and keep `UiConfig.schema` at `1`
- [x] T007 Extend `ui/tests/unit/layout-defaults.test.ts`: the new wide/ultra/ultra-plus defaults contain no second `list` and no `tool` slot; a saved wide profile whose slots are `[nav, list, list, inspector]` resolves to cascade slots with its sizes preserved; a saved `ultra` profile with `tool:overview` and `tool:flux` normalises; an invalid saved profile still falls back to the default; `laptop`/`desktop`/`tablet`/`phone` saved profiles are untouched; slot counts per class are unchanged (Principle IV)
- [x] T008 Add `cascade: PaneId[]` to the layout state in `ui/src/store/index.ts` (initial: the section list pane alone) and derive `slotPanes` from it through `slotsFor` at `wide` and above, leaving the below-wide derivation exactly as it is. `assignPanes` claims an inspector pane per cascade slot, so panes keep their identity across a profile or class change
- [x] T009 Unit tests for the store wiring in `ui/tests/unit/layoutCascade.test.ts`: at wide the slot panes are nav + chain tail; below wide `cascade` is ignored and `slotPanes` matches today's snapshot; a width-class change adds and drops columns from the tail only and never remounts (pane ids stay identical); a context change resets the chain

## Phase 2: US1 clean start on a wide screen (P1) 🎯 MVP

**Goal**: a wide or larger window opens with the navigator, the selected section and the section's most relevant item — nothing else.

**Independent test**: `cascade-clean-start.spec.ts` on the wide, ultra and ultra-plus projects.

- [x] T010 [US1] E2E `ui/tests/e2e/cascade-clean-start.spec.ts` against the scripted fake backend, covering contract items 1–3: after one section click exactly slots 0–2 are occupied and the rest carry `cascade-empty`; no `tool` pane and no second list is present at wide, ultra or ultra-plus; the auto-opened step is the first `err` row, then the first `warn` row, then the first row; an empty section and a listless destination (Settings, Topology) leave slot 2 as `cascade-empty`; the auto-open emits only the same `read` commands a row click emits
- [x] T011 [US1] Reset the chain in `navigate` (`ui/src/store/index.ts`): `cascade = resetTo(listPaneId)` and, at `wide` and above only, arm `pendingAutoOpen = { paneId, route }`. Keep the existing clearing of selection, focused row, sort and scroll, and keep the 043 setup-detour behaviour
- [x] T012 [US1] Consume `pendingAutoOpen` where the section list renders: once that pane's list state has its first page, clear the arm and open `mostRelevantRow` in the next cascade column through the existing inspect path. One O(n) scan per section open, after the first page, never on the render path (SC-003). Nothing is opened when the list is empty
- [x] T013 [US1] Clear `pendingAutoOpen` on any operator action that supersedes it — a row click, a section change, a context change, dropping below wide width — in `ui/src/store/index.ts`, so a slow list never steals the operator's own selection
- [x] T014 [P] [US1] Mark the auto-opened step with `data-auto-open="true"` in `ui/src/components/shell/PaneHost.tsx`, dropped as soon as the operator clicks anywhere in the cascade (contract "Slots and test ids")
- [x] T015 [P] [US1] Unit tests for the arm/consume/clear rules in `ui/tests/unit/autoOpen.test.ts`: armed only at wide and above, consumed once, cleared by each superseding action, and no scan before the first page lands

## Phase 3: US2 drilling in cascades to the right (P2)

**Goal**: each drill-in fills the next column, the chain stays visible, and an overflowing chain shifts left while the navigator and section list stay put.

**Independent test**: `cascade-drill-in.spec.ts` on the wide, ultra and ultra-plus projects.

- [ ] T020 [US2] E2E `ui/tests/e2e/cascade-drill-in.spec.ts` covering contract items 4–9: a drill-in occupies the next empty column with `data-step` one higher; the column it came from still shows its own item; with every column occupied one more drill-in keeps slots 0–1, shifts the steps left and puts the deepest step last; clicking another row in slot 1 replaces slot 2 and empties the rest; clicking another section returns to the US1 state; a context change returns to the US1 state for the new context with no step showing the old cluster; the dock is unaffected
- [ ] T021 [US2] Route the push path (`openRef` / `inspect` in `ui/src/store/index.ts`) through `pushStep` at `wide` and above instead of retargeting the single slotted inspector, keeping the phone stack path and the below-wide fast path exactly as they are
- [ ] T022 [US2] Truncate the chain with `truncateAt` when the operator selects a row in a column that is not the last, in `ui/src/store/index.ts`, so later columns return to empty
- [ ] T023 [P] [US2] Render an occupied cascade column as `data-testid="cascade-step"` with `data-step="<n>"` (n from 1 at slot 2) in `ui/src/components/shell/PaneHost.tsx`
- [ ] T024 [P] [US2] E2E width-boundary test in `ui/tests/e2e/cascade-drill-in.spec.ts` (contract item 7, SC-004): with a three-step cascade, type in a column's search box, scroll it, then cross 1920 and 2560 px in both directions and assert every step, its selection, its search text and its scroll position survive, and that hidden steps reappear on the way back up

## Phase 4: US3 small windows untouched (P2)

**Goal**: phone, tablet, laptop and desktop behave exactly as before, with no test edits needed (SC-005).

**Independent test**: the existing phone/tablet/laptop/desktop projects pass unchanged, plus `cascade-small-windows.spec.ts`.

- [ ] T030 [US3] E2E `ui/tests/e2e/cascade-small-windows.spec.ts`: at phone, tablet, laptop and desktop widths no `cascade-*` test id is present, a section click leaves the inspector waiting for the operator's click (no auto-open), and the phone stack still pushes and pops
- [ ] T031 [US3] Run the phone, tablet, laptop and desktop Playwright projects unchanged and fix any fallout in `ui/src/`, not in the tests. Any test that must change is a defect in this feature's gating — record the reason in the PR if one truly must
- [ ] T032 [P] [US3] Confirm in `ui/tests/unit/layoutCascade.test.ts` that the below-wide `slotPanes` derivation is byte-identical to today's, as a regression guard for the gating

## Phase 5: US4 the empty columns invite the next click (P3)

**Goal**: waiting columns explain themselves, and filling one does not reflow the columns already on screen.

**Independent test**: `cascade-placeholders.spec.ts` on the wide and ultra projects.

- [ ] T040 [US4] E2E `ui/tests/e2e/cascade-placeholders.spec.ts`: every empty column carries `data-testid="cascade-empty"` with the visible text "Follow a link to open the next level here"; after dragging a divider and then filling a column, the columns' widths are unchanged (FR-013); a step whose object has been deleted shows `cascade-gone` with its namespace/name and does not jump to another item (FR-014)
- [ ] T041 [P] [US4] Add the empty-cascade placeholder to `ui/src/components/shell/PaneHost.tsx`: an inspector pane with no target at wide and above renders `cascade-empty` with the contract's copy, WCAG 2.2 AA contrast, and no change to the pane's size constraints
- [ ] T042 [P] [US4] Add the "no longer exists" state for a cascade step whose object has gone from the list or the cluster in `ui/src/components/shell/PaneHost.tsx` (`data-testid="cascade-gone"`), keeping the step's `target` so nothing is silently re-selected
- [ ] T043 [P] [US4] Check that a filling or emptying column never writes sizes back to the profile: `SlotGrid`'s `onRowLayout` already ignores non-user interactions, so add the assertion to `ui/tests/unit/layout-defaults.test.ts` rather than new code

## Phase 6: Polish and cross-cutting

- [ ] T050 [P] Update `docs/MVP.md`: the wide and ultra layouts cascade, with the relevance rule and the statement that small windows are unchanged
- [ ] T051 [P] Tick Group 3 (#6, #7, #8) in `docs/FEEDBACK_2026-09-27.md` with "Addressed by `044-cascading-layout`"
- [ ] T052 [P] Check the accessibility of the new states: the placeholder and the "gone" state are announced, the cascade columns keep their resize handles' accessible names, and keyboard-only navigation reaches every occupied column
- [ ] T053 Run `pnpm -C ui lint`, `pnpm -C ui typecheck`, `pnpm -C ui exec vitest run` and `pnpm exec prettier --check` from inside `ui/`, then the full Playwright suite alone on the machine with `free -g` showing ≥ 8 GB
- [ ] T054 Run `scripts/hygiene.sh` and clean up any worktree, branch or build cache this batch created
- [ ] T055 **Last step of the batch**: promo captures — add a wide cascade scene to `ui/tests/promo/capture.spec.ts` and its `SHOT_META` entry in `ui/tests/promo/compose.spec.ts`, then shoot with `FATHOM_LIVE_CONTEXT=kind-fathom-dev pnpm promo` and review `promo-out/fathom-promo.pdf`

## Dependencies

- Phase 1 blocks every story. Within it: T001–T004 are parallel; T005–T007 are parallel with them but T006 depends on T005; T008 depends on T001 and T005; T009 depends on T008.
- US1 (Phase 2) is the MVP and depends only on Phase 1.
- US2 (Phase 3) depends on US1 (the chain must start clean before drilling in means anything).
- US3 (Phase 4) depends on Phase 1 only and can run beside US1.
- US4 (Phase 5) depends on US1 for the placeholder to be visible anywhere.
- Phase 6 runs last; T055 is the final task of the batch.

## Parallel execution

```text
Phase 1:  T001+T002 | T003+T004 | T005→T006→T007
Then:     US1 (T010→T015)  ||  US3 (T030→T032)
Then:     US2 (T020→T024)  ||  US4 (T040→T043)
Finally:  T050+T051+T052 → T053 → T054 → T055
```

With other agents on the machine, use `--workers=6` or fewer; run the full suite alone.

## Implementation strategy

Ship US1 first: it is the whole of feedback #6/#7's complaint and is releasable on its own — a wide
window that opens on one thing. US3 lands beside it as the regression gate. US2 makes the width pay
off, and US4 is the finish.
