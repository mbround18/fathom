# Implementation Plan: Shared UI kit

**Branch**: `main` (agents work on `065-*` branches in worktrees) | **Date**: 2026-11-01 |
**Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/065-shared-ui-kit/spec.md`

## Summary

Keel's screen patterns move out of `apps/keel/web/src/ui` and `src/app` into a new `kit` entry of
the existing `@companion/ui` package (`@companion/ui/kit`). The primitives at `@companion/ui` stay
where they are. Anything that belongs to one app reaches the kit in one of two ways:

- a `KitProvider` at the app root, for the guide address builder, the glossary and the link
  component;
- props, for the text and the refusal.

Keel switches over first, with only its import paths changing. Committed screenshot goldens prove
that nothing moved visibly. Depths then adopts the shell, the page header, the empty,
failed-to-load and crash states, the dialogs and the tables. Fathom adopts the leaf pieces, the
pane header, and a shared virtualized grid that takes over the engine of its `ResourceTable`.

Every kit piece gets an isolated test page in `packages/ui/harness` and its own Playwright spec in
`packages/ui/tests/e2e`. Each spec checks the piece's states, keyboard use and focus, and runs an
axe scan in light and dark at 360 and 1280 px. A copy check (`scripts/check-kit-copies.mjs`, run
by `lint:packages`) fails when an app defines its own piece under a kit name.

## Technical Context

**Language/Version**: TypeScript 6 (typescript7 preview for the package typecheck), React 19

**Primary Dependencies**:
- Already in use: React 19, radix-ui, Tailwind 4, @phosphor-icons/react, react-markdown,
  remark-gfm, @tanstack/react-virtual.
- The last three are already in the workspace lockfile; they move into `@companion/ui`'s
  dependencies.
- New: `@axe-core/playwright`, as a dev dependency only (owner-approved, FR-004).

**Storage**: N/A

**Testing**:
- Vitest with jsdom, for the kit's unit tests.
- Playwright with axe, against the isolated harness (`packages/ui`).
- Each app's existing Vitest and Playwright suites.
- Keel visual goldens: Playwright `toHaveScreenshot`.

**Target Platform**: browsers (Keel and Depths web) and the Tauri v2 webview (Fathom desktop)

**Project Type**: shared front-end library in a pnpm workspace, plus three consuming apps

**Performance Goals**: Fathom's grid stays virtualized. It must still meet constitution V: input
handled in under 100 ms with 10,000 live rows, with no regression against `ResourceTable` today.

**Constraints**:
- Keel shows no visual or behavioural change (FR-007).
- The kit's runtime dependencies add nothing new to the workspace (FR-004).
- The kit never imports from an app, and never imports a protocol crate's bindings.
- Existing `data-` markers and accessible names are kept (FR-003).

**Scale/Scope**:
- About 25 kit pieces.
- Keel: about 20 files move, and imports are rewritten across the app.
- Depths: about 10 feature pages.
- Fathom: about 6 adoption points (`PaneHost`, `ResourceTable`, `ConfirmDialog`, copy fields,
  empty and failed panes).

## Constitution Check

*GATE: must pass before Phase 0 research, and again after Phase 1 design.*

| Principle | Effect of this feature | Status |
|---|---|---|
| I. Operator-first, explicit mutation | The kit's ConfirmDialog keeps preview-then-confirm, passkey step-up, and the refusal shown verbatim (`RefusalText`). It gains Fathom's typed confirmation for bulk deletes, so Fathom loses nothing. | Pass |
| II. One contract per app | The kit holds no protocol types. `Refusal` is a plain UI shape, and each app maps its own `Denied`/`OpError` to it. `loadList` stays in each app, because it sends that app's commands. | Pass |
| III. Tested at the boundaries | Each piece gets unit tests and an isolated Playwright spec (FR-016 to FR-019). The apps' suites keep covering how each app uses the pieces. Keel gets screenshot goldens. CI runs the kit's tests in the `shared` slice. | Pass |
| IV. Density scales with width | DataTable keeps its table at 640 px and wider, and cards below that. The grid keeps column priorities, and no page scrolls sideways. Every piece is checked by axe (WCAG 2.2 AA) at 360 and 1280 px, in light and dark. | Pass |
| V. Fast and quiet | Fathom's grid stays virtualized (`@tanstack/react-virtual`), and its store keeps owning search, sort and selection. The kit's grid is controlled, so it holds no second copy of that state. | Pass, re-checked after US3 |
| VI. User data is sacred | Not touched. There are no data or format changes. | Pass |
| Workflow | Every commit is signed, and no message or file carries a session link. One CI run, at the end. | Pass |

**Post-design re-check (after Phase 1)**: still passes. The one item to watch is constitution V
for Fathom's grid. The US3 tasks add a virtualization assertion: no more than about 60 rows are
in the DOM at 10,000 rows. They also rerun Fathom's existing perf spec.

## Project Structure

### Documentation (this feature)

```text
specs/065-shared-ui-kit/
├── plan.md
├── research.md
├── data-model.md        # kit pieces, app content, allow-list
├── quickstart.md
├── contracts/
│   ├── kit-api.md       # every exported piece, its props and its markers
│   └── harness.md       # the isolated test page and spec contract
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
packages/ui/
├── package.json             # exports "." (primitives) and "./kit"; test, e2e, harness scripts
├── src/
│   ├── index.ts             # primitives, unchanged
│   └── kit/
│       ├── index.ts         # the kit's public surface (contracts/kit-api.md)
│       ├── provider.tsx     # KitProvider, useKit
│       ├── router.ts  Link.tsx
│       ├── refusal.ts  RefusalText.tsx  LoadFailed.tsx  LoadState.tsx  ErrorLine.tsx
│       ├── Page.tsx  PageHeader.tsx  PageErrorBoundary.tsx
│       ├── EmptyState.tsx  empty-variant.ts  NoMatch.tsx
│       ├── Dialog.tsx  ConfirmDialog.tsx  CopyBlock.tsx  clipboard.ts  reveal.ts
│       ├── form.tsx  Tabs.tsx  DataTable.tsx  DataGrid.tsx  column-budget.ts
│       ├── Markdown.tsx  slug.ts  GuideLink.tsx  TermTip.tsx
│       └── NavShell.tsx
├── harness/                 # Vite test pages, dev only, never shipped
│   ├── index.html  main.tsx  harness.css
│   └── pieces/<piece>.tsx   # the fixtures for each piece, chosen by ?piece=&state=
├── tests/
│   ├── unit/<piece>.test.tsx
│   ├── e2e/<piece>.spec.ts  # one per piece (contracts/harness.md)
│   └── support/a11y.ts      # axe scan across theme × width
├── vite.harness.config.ts  vitest.config.ts  playwright.config.ts
└── kit-allow.json           # the copy check's allow-list

apps/keel/web/src/            # ui/ and app/{Link,router}.ts are gone; imports point at the kit.
                              # Keel keeps load.ts (loadList, OFFLINE), guides.ts, glossary.ts,
                              # nav.ts and a thin Shell over NavShell.
apps/keel/web/tests/e2e/s065/ # visual goldens (SC-003)
apps/depths/web/src/          # Shell over NavShell; pages use the kit's pieces
ui/src/                       # PaneHost header -> kit PageHeader (pane size); ResourceTable ->
                              # kit DataGrid; ConfirmDialog -> kit; copy, empty and failed -> kit
scripts/check-kit-copies.mjs  # FR-015
CONTRIBUTING.md               # "Shared UI kit" section (FR-014)
```

**Structure Decision**: the kit is a subpath (`./kit`) of the existing `@companion/ui` package,
not a new package (spec Assumptions). The harness is a dev-only folder inside the package. The
browsable gallery app comes later, as `apps/ui-gallery` (owner, 2026-11-01).

## Delivery order and agents

1. **Wave 0 (main session).** Take Keel's screenshot goldens on the current main, before anything
   moves. Scaffold the kit entry, its test setup and the harness skeleton.
2. **Wave 1 (one agent, US1).** Move every piece and switch Keel over. The goldens and Keel's
   suites must pass. This is sequential, because every move touches Keel's imports.
3. **Wave 2 (parallel agents).**
   - Isolated specs for pieces A (states and feedback).
   - Isolated specs for pieces B (dialogs, forms, copy, tabs, markdown, help and nav).
   - Depths adoption (US2).
   - Kit grid and pane-header extensions, plus their isolated specs.
   - Copy check and contributor guide (US4).
4. **Wave 3.** Fathom adoption (US3) on top of the grid, then an impeccable review of every
   harness page, with fixes in the kit.
5. **Wave 4 (main session).** Merge everything, run `scripts/ci.sh all` once, report its timing,
   and run hygiene.

## Complexity Tracking

| Item | Why needed | Simpler alternative rejected because |
|---|---|---|
| Two table pieces (`DataTable`, `DataGrid`) | Keel's small lists show cards on phones. Fathom's live lists need virtualization, an ARIA grid and column budgets (constitution V). | A single table would either make Keel's lists virtualized grids, which changes Keel (FR-007), or drop virtualization from Fathom, which breaks constitution V. Both share header, row, empty and failed styling. |
| `./kit` subpath export | Keel's `Dialog`, `Tabs` and `Select` would clash with the primitives of the same names. | Renaming either side breaks imports and test names in all three apps. |
| One dev dependency (`@axe-core/playwright`) | FR-017 requires an automated accessibility scan. | Hand-written checks cover only what someone thought to check. The owner chose axe. |
