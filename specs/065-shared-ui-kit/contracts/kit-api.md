# Contract: `@companion/ui/kit`

The kit's public surface is `packages/ui/src/kit/index.ts`. **Moved** means the piece's signature,
markup, classes, accessible names and markers are exactly what Keel's file has today (FR-003). Only
its imports change: `@/app/*` and `@/help/*` become kit-local, or `useKit()`.

The additions below are all optional props. When an app leaves them out, the output is the same
as today.

## Pieces

Keel files are under `apps/keel/web/src`.

| Id | Exports | Source in Keel | Markers kept | App content | Change |
|---|---|---|---|---|---|
| `kit-provider` | `KitProvider`, `useKit`, types `KitContext`, `GuideRef`, `GlossaryEntry` | new | none | everything in `KitContext` | new |
| `router` | `navigate`, `usePath`, `useSearchParam`, `useHash`, `matchPath`, `setNavigationGuard`, type `Params` | `app/router.ts` | none | none | event becomes `companion:navigate` |
| `link` | `Link` | `app/Link.tsx` | none | none | moved |
| `refusal` | type `Refusal`, `isTransportFailure`, types `ListHandlers`, `ListLoad` | `app/load.ts` (types only) | none | `OFFLINE` stays in the app | moved, then split |
| `refusal-text` | `RefusalText` | `ui/RefusalText.tsx` | `data-refusal`, `data-denied-action`, `data-denied-reason`, `data-error-code` | `refusal`, `lead` | moved |
| `load-failed` | `LoadFailed` | `ui/LoadFailed.tsx` | `data-pane-state="error"` | `what`, `refusal`, `onRetry` | moved |
| `load-state` | `LoadState` | `ui/LoadState.tsx` | `data-pane-state` | as today | moved |
| `error-line` | `ErrorLine` | `ui/ErrorLine.tsx` | none | `message`, `onDismiss` | moved |
| `page` | `Page` | `ui/Page.tsx` | `data-page` | `name`, `labelledBy` | moved |
| `page-header` | `PageHeader` | `ui/PageHeader.tsx` | `data-page-intro` | title, intro, guide, actions | adds `size?: 'page' \| 'pane'`, `lead?`, `count?`, `rootProps?` (R9) |
| `page-error-boundary` | `PageErrorBoundary` | `ui/PageErrorBoundary.tsx` | `data-pane-state="crashed"`, `data-error-message` | none | adds `resetKey?` (pane reset) |
| `empty-state` | `EmptyState`, `emptyVariant`, type `EmptyVariant` | `ui/EmptyState.tsx`, `ui/empty-variant.ts` | `data-empty`, `data-empty-page`, `data-empty-action`, `data-who-can`, `data-pane-state` | text, steps, who-can, guide | moved |
| `no-match` | `NoMatch` | `ui/NoMatch.tsx` | `data-empty`, `data-pane-state` | `what`, `message`, `onClear` | moved |
| `dialog` | `Dialog` | `ui/Dialog.tsx` | `data-summary` | as today | moved |
| `confirm-dialog` | `ConfirmDialog`, `PreviewDetails`, type `Preview` | `ui/ConfirmDialog.tsx` | `data-preview-summary`, `data-affected`, `data-step-up` | as today | adds `typedConfirm?: { word: string }` (R9) |
| `copy-block` | `CopyBlock`, `copyText` | `ui/CopyBlock.tsx`, `ui/clipboard.ts` | `data-copy-block`, `data-copy`, `data-masked` | as today | moved |
| `reveal` | `useReveal` | `ui/reveal.ts` | none | none | moved |
| `form` | `FIELD`, `FIELD_DENSE`, `LABEL`, `STATUS`, `FORM_WIDTH`, `LINK`, `H1`, `H2`, `H3`, `SectionIntro`, `Select`, `Field`, `FormPanel` | `ui/form.tsx` | `data-section-intro` | guide | moved |
| `tabs` | `Tabs`, type `TabItem` | `ui/Tabs.tsx` | `data-tab` | as today | moved |
| `data-table` | `DataTable`, type `Column` | `ui/DataTable.tsx` | `data-pane-state` | columns, rows | adds `sort?`, `selection?`, `rowKey?`, `rowActions?`, and `Column.sortable?` (data-model.md) |
| `data-grid` | `DataGrid`, type `GridColumn`, `computeVisibleColumns` | Fathom `ui/src/components/ResourceTable.tsx` (engine), `ui/src/layout/column-budget.ts` | Fathom's grid markers and `data-testid`s that its suite uses | rows, columns, sort, selection, focus, `rowActions`, `onOpen` | new (R8) |
| `markdown` | `Markdown` (default export) | `ui/Markdown.tsx` | `data-markdown`, `data-image-link` | `isGuide`, `guideHref`, `guideImage` | moved |
| `slug` | `slug`, `slugger` | `help/slug.ts` | none | none | moved |
| `guide-link` | `GuideLink` | `ui/GuideLink.tsx` | `data-guide-link` | `guideHref` | renders nothing without `guideHref` |
| `term-tip` | `TermTip` | `help/TermTip.tsx` | `data-infotip`, `data-infotip-content` | `glossary` | takes `term: string` (R6) |
| `nav-shell` | `NavShell`, types `NavGroup`, `NavItem` | markup from `app/Shell.tsx` | `data-nav-group`, `data-nav-badge`, `data-update-dot` | brand, groups, path, `headerEnd`, `banner`, `pinned`, `overlay` (rendered after the frame) | new (R7) |

## Rules

- **Imports.** The kit imports only:
  - `react`;
  - the `@companion/ui` primitives (relatively, from `../`);
  - `@companion/theme`;
  - `@phosphor-icons/react`;
  - `react-markdown` and `remark-gfm` (Markdown only);
  - `@tanstack/react-virtual` (DataGrid only).
- **Bundles.** Nothing in `src/index.ts` (the primitives) imports from `src/kit`, so apps that
  don't use the kit don't bundle it.
- **No app content.** No piece contains app words or rules. Text that seems generic counts too:
  "Failed to load {what}." is the kit's sentence, but `what` comes from the app.
- **Status classes.** Every piece keeps the theme's status tokens (`text-status-err`, …) rather
  than raw colours.
- **App code** imports from `@companion/ui/kit`, never from a deep path such as
  `@companion/ui/src/kit/...`.
