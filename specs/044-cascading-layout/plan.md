# Implementation Plan: Cascading wide-screen layout

**Branch**: `044-cascading-layout` | **Date**: 2026-09-27 | **Spec**: [spec.md](spec.md)

## Summary

Wide and larger windows stop opening panes the operator did not ask for. The row becomes
`[navigator] [selected section] [cascade…]`: the cascade columns are empty until a drill-in fills
them, except that choosing a section auto-opens that section's most relevant item (first `err` row,
else first `warn` row, else the first row) in column 3.

- **Default profiles.** `wide` loses its second `list` slot; `ultra` loses the `overview` tool slot;
  `ultra-plus` loses the `flux` tool slot. Those positions become cascade slots (`kind: 'inspector'`,
  no target). Slot counts per width class are untouched, so Principle IV still holds.
- **Cascade state.** A new UI-only `layout.cascade: PaneId[]` is the chain from the section list to
  the deepest open item. Slots are derived from it; panes keep their identity, so a width-class
  change adds or drops columns from the right end without losing anything.
- **Auto-open.** A pure `mostRelevantRow(descriptor, rows)` picks the row; an effect opens it once
  per (pane, route) after the list's first page lands, so the list never waits for it.
- **Saved layouts.** `resolveProfile` normalises a saved `wide`/`ultra` profile's slots from index 2
  onward to cascade slots, keeping the operator's sizes. No `UiConfig.schema` bump, no protocol
  change, no backend change.
- **Small windows.** `phone`, `tablet`, `laptop` and `desktop` profiles and behaviour are unchanged;
  the auto-open is gated to wide and above.

Everything is UI-only. There are no protocol changes and no backend changes.

## Technical Context

**Language/Version**: TypeScript 5 (strict), React 19. No Rust change.

**Primary Dependencies**: Zustand store (`ui/src/store/index.ts`), `react-resizable-panels`
(`SlotGrid.tsx`), the existing pane/route model. Nothing new is added.

**Storage**: `UiConfig.layouts[widthClass]` as it exists today (`schema: 1`, unchanged), read and
written through `SaveUiConfig`.

**Testing**: Vitest for the pure layers, Playwright against the scripted fake backend for each user
story, per width-class project (`{chromium,webkit}-{phone,tablet,laptop,desktop,wide,ultra,ultra-plus}`).

**Target Platform**: Tauri v2 desktop (Tier 1) and the hosted web build; identical UI code.

**Project Type**: Desktop app frontend (single `ui/` package, Rust backend untouched).

**Performance Goals**: A section opens its list within 100 ms of data availability with 10,000 rows;
the relevance scan is one O(n) pass, run after the first page lands and never on the render path.

**Constraints**: No horizontal page scroll at any width; no pane remount on a width-class change
(feedback #2 regression guard); saved layouts keep loading (Principle VI).

**Scale/Scope**: 7 width classes, up to 6 slots, ~40 navigator sections. Roughly 6 UI source files
plus tests.

### Files in play

- `ui/src/layout/defaults.ts` — `PROFILES.wide`/`PROFILES.ultra`, `ULTRA_PLUS_SLOT`, `resolveProfile`
  (new cascade normalisation), and a new `CASCADE_FIRST_SLOT = 2` constant.
- `ui/src/layout/cascade.ts` *(new)* — pure cascade algebra: `pushStep`, `truncateAt`, `resetTo`,
  `slotsFor(cascade, slotCount)`; the shift-left rule when the chain outgrows the columns.
- `ui/src/resources/relevance.ts` *(new)* — `mostRelevantRow(descriptor, rows)` using the
  `ColumnDef.status` column's `text(row)` through `statusTone` from `ui/src/resources/search.ts`.
- `ui/src/store/index.ts` — `layout.cascade`, `assignPanes` (cascade slots claim inspector panes),
  `navigate` (reset the cascade, arm the auto-open), the `openRef`/`inspect` push path (append a
  step instead of retargeting the single slotted inspector at wide and above), context change
  (reset), width-class change (re-derive slots from the cascade).
- `ui/src/components/shell/SlotGrid.tsx` — unchanged mechanics; only the empty-slot render path.
- `ui/src/components/shell/PaneHost.tsx` — the cascade placeholder for an inspector pane with no
  target at wide and above (FR-012), and the "this item is gone" state (FR-014).

## Constitution Check

*GATE: passed before Phase 0; re-checked after Phase 1 design (below).*

- **I. Operator-first, explicit mutation.** The auto-open reads one resource; it mutates nothing and
  issues only the same detail read a click would. No wizard, no hidden Kubernetes object. ✔
- **II. One contract, backend owns the cluster.** No protocol or backend change; the cascade is UI
  state derived from panes that already exist, persisted through the existing `SaveUiConfig`. ✔
- **III. Tested at the boundaries.** Four user stories, four Playwright specs against the scripted
  fake backend, plus Vitest for `cascade.ts`, `relevance.ts` and the `resolveProfile` migration.
  Each behaviour change lands with a test that fails without it. No test is removed: the phone,
  tablet and laptop suites must pass untouched (SC-005). ✔
- **IV. Density scales with width.** Slot counts per class are unchanged and still monotonic
  (`visibleSlots` untouched); wider windows show more of the chain, never less. Columns keep their
  selection, search text and scroll position across a class change because the panes are the same
  objects. No horizontal page scroll: cascade columns live in the existing panel group. ✔
- **V. Fast and quiet.** One O(n) relevance pass per section open, after the first page, off the
  render path; no new subscriptions, no polling, no extra redraws. Events from a superseded context
  are already discarded, and the cascade resets on a context change (FR-015). ✔
- **VI. User data is sacred.** `UiConfig.schema` stays `1`; a saved profile keeps loading and its
  column sizes are preserved. The file is only rewritten on the operator's next layout change. ✔

No violations, so Complexity Tracking is omitted.

## Project Structure

### Documentation (this feature)

```text
specs/044-cascading-layout/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── ui-cascade.md    # Phase 1 output: the layout/cascade UI contract
├── checklists/
│   └── requirements.md  # From /speckit-specify
└── tasks.md             # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
ui/src/
├── layout/
│   ├── defaults.ts          # profiles + saved-profile normalisation (changed)
│   ├── cascade.ts           # pure cascade algebra (new)
│   └── width-class.ts       # unchanged
├── resources/
│   └── relevance.ts         # mostRelevantRow (new)
├── store/
│   └── index.ts             # layout.cascade, assignPanes, navigate, openRef (changed)
└── components/shell/
    ├── SlotGrid.tsx         # empty-slot render path (changed)
    └── PaneHost.tsx         # cascade placeholder + missing-item state (changed)

ui/tests/
├── unit/
│   ├── cascade.test.ts          # new
│   ├── relevance.test.ts        # new
│   └── layout-defaults.test.ts  # extended: saved-profile normalisation
└── e2e/
    ├── cascade-clean-start.spec.ts   # US1 (new)
    ├── cascade-drill-in.spec.ts      # US2 (new)
    ├── cascade-small-windows.spec.ts # US3 (new)
    └── cascade-placeholders.spec.ts  # US4 (new)
```

**Structure Decision**: the existing single-package frontend layout. The two new pure modules sit
beside the code they serve (`ui/src/layout/`, `ui/src/resources/`) so they are unit-testable without
React, which is how the rest of the layout logic is already split.

## Phase 1 re-check

The design adds no protocol surface, no new dependency and no new persisted field; it moves the
prefill decision out of the default profiles and into the operator's clicks. The one deliberate
scope cut is recorded in the spec's Assumptions: pinning an arbitrary tool into a cascade column by
hand is not part of this feature. Constitution Check above still passes unchanged.
