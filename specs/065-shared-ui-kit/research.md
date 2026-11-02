# Research: Shared UI kit

Each entry gives the decision, why it was made, and what else was considered.

## R1. Where the kit lives: `@companion/ui/kit`

- **Decision**: add the `"./kit": "./src/kit/index.ts"` export to `packages/ui/package.json`. The
  root export (`"."`) keeps the shadcn primitives, unchanged.
- **Rationale**: three of Keel's pieces share a name with a primitive: `Dialog` and `Tabs` wrap the
  primitives of the same name, and `Select` styles a native `<select>`. A separate entry keeps
  every existing import of the primitives working, and lets the copy check (R10) read one index to
  get the kit's names.
- **Alternatives**:
  - Rename the kit pieces (`KitDialog`): this changes what tests and developers search for.
  - A new package `@companion/kit`: the spec's assumption is to reuse the existing package, and a
    new package would add workspace wiring for no gain.

## R2. App content reaches the kit through `KitProvider`, or through props

- **Decision**:
  - Per-call content comes as props: the text, the `what` of "Failed to load {what}.", the
    refusal, `canAct`, the steps and the who-can line.
  - App-wide services come from one `<KitProvider value={{ guideHref, isGuide, glossary, Link }}>`
    at the app root.
  - Every provider field is optional:
    - With no `guideHref`, `GuideLink` renders nothing and Markdown leaves guide links as plain
      links. This covers the spec's edge case of an app with no guides.
    - With no `Link`, the kit's own `Link` is used.
- **Rationale**:
  - Keel ties `GuideLink`, `Markdown`, `TermTip`, `EmptyState`, `PageHeader` and `form` to
    `@/help/guides` and `@/app/Link`. A context removes those imports without threading a prop
    through every page.
  - The kit's `GuideRef` is `{ guide: string; anchor?: string }`. Keel's narrower
    `{ guide: GuideSlug }` is assignable to it, so Keel's literal refs keep their type checking,
    and `guide-links.test.ts` keeps checking that they resolve.
- **Alternatives**:
  - Props everywhere: too much churn across about 60 call sites, and Keel's screens would change
    in more than their imports.
  - Module-level configuration (`configureKit()`): hidden global state, and awkward in tests.

## R3. The refusal shape and `loadList`

- **Decision**: the kit owns the UI shape below, plus `isTransportFailure(r)`, which is
  `r.kind === 'error' && r.code === 'offline'`.

  ```ts
  type Refusal =
    | { kind: 'denied'; action: string; reason: string }
    | { kind: 'error'; code: string; message: string }
  ```

  Some things stay in each app, because they read that app's protocol:
  - the `OFFLINE` constant, with its app-specific wording;
  - `refusalOf(event)`;
  - `loadList(cmd, …)`.

  The kit exports `ListHandlers<T>` and `LoadState`/`LoadFailed`, which are the visible half of
  "load a list into that state".
- **Rationale**: FR-002 says the refusal shape comes from the app. Constitution II forbids protocol
  types in shared UI code. Keel's `Refusal` is already protocol-free, so it moves as is, and
  Keel's `load.ts` re-exports the type.
- **Alternatives**: a generic `loadList` that takes a send function. Depths' transport is HTTP plus
  WebSocket, unlike Keel's command socket, so a generic version would be a lowest-common-
  denominator API that neither app wants.

## R4. Router and Link

- **Decision**:
  - Keel's `router.ts` moves to the kit whole: `navigate`, `usePath`, `useSearchParam`, `useHash`,
    `matchPath` and `setNavigationGuard`. `Link.tsx` moves with it.
  - The in-app navigation event becomes `companion:navigate`.
  - Depths drops its own copy. It gains `useSearchParam` and `useHash` for free.
  - `apps/depths/web/tests/e2e/us5-access/users.spec.ts` refers to `depths:navigate` and is
    updated.
- **Rationale**: the two routers are the same apart from the event name and Keel's two extra
  hooks. Each app is its own document, so one event name cannot collide.
- **Alternatives**: a per-app event name passed to `createRouter()`. That is configuration with no
  purpose.

## R5. Markdown, slug, react-markdown

- **Decision**: `Markdown.tsx` and `slug.ts` move to the kit. `react-markdown@^10.1.0` and
  `remark-gfm@^4.0.1` move into `@companion/ui`'s `dependencies`, at the versions Keel uses today.
  The Markdown component stays a default export, so Keel can keep loading it lazily with
  `React.lazy`.
- **Rationale**: both packages are already in the lockfile, so the workspace gets no new
  third-party dependency (FR-004). The primitives entry does not import them, so the apps' main
  bundles don't grow.
- **Alternatives**: keep Markdown in Keel. FR-001 lists Markdown rendering among the kit's pieces,
  and Depths' editor help could use it.

## R6. Glossary popover (`TermTip`)

- **Decision**:
  - The kit's `TermTip` takes `term: string` and looks up the entry in `useKit().glossary`, where
    the type is `Record<string, { term; text; guide? }>`.
  - Keel uses the kit's `TermTip` directly. Its `glossary.test.ts` (extended if needed) checks
    that every `term="…"` in Keel's source is a `TermId`, which replaces the type check Keel's
    own wrapper gave.
  - A term the app's glossary doesn't have renders nothing, and logs a warning in development.
- **Rationale**: the terms stay in each app (FR-002). One component, with no factory, keeps the
  copy check simple.
- **Alternatives**: a `makeTermTip(glossary)` factory. It produces app-side bindings that carry a
  kit name, which complicates the copy check.

## R7. Navigation shell: `NavShell`

- **Decision**: a presentational shell:
  - a header row with a brand slot and an end slot (account menu, theme);
  - an optional banner slot (Keel's enrolment banner);
  - grouped side navigation with section headings, collapsing to a sheet below 768 px as Keel
    does today;
  - the main area.

  Items are `{ path, label, icon, badge?: { count; label }, dot?: { label } }`, and badge and dot
  keep Keel's markers: `data-nav-group`, `data-nav-badge`, `data-update-dot`, an `aria-hidden`
  chip, and screen-reader-only text on the link.

  Keel's `Shell.tsx` keeps the wiring: `AccountMenu`, `EnrolBanner`, the approvals count, the
  update state and `visibleGroups(me)`. It then renders `NavShell`.

  Depths' `Shell.tsx` does the same with its own sections. That is a visible change for Depths
  (from a top admin bar and dock to grouped side navigation), and it is listed in the change notes
  (FR-011).
- **Rationale**: the markup is generic. The data and the menus belong to each app.
- **Alternatives**: move Keel's whole `Shell.tsx`. It imports approvals, passkeys, updates and the
  session, so it is app-bound.

## R8. Tables: `DataTable` (Keel, as is) and `DataGrid` (from Fathom's engine, in Keel's look)

- **Decision**:
  - **`DataTable` moves unchanged.** It is a semantic `<table>` at 640 px and wider, cards below
    that, and an empty row. It gains three optional, controlled props:
    - `sort` (`{ key, dir }` plus `onSort`), shown as `aria-sort` buttons in the header;
    - `selection` (a `Set` plus `onChange`), shown as a checkbox column with a select-all header;
    - `rowActions(row)`, shown as a trailing ⋯ menu button made from the `DropdownMenu`
      primitive.

    Keel passes none of them, so Keel's output is the same, byte for byte.
  - **`DataGrid` is new in the kit.** It is the generic part of Fathom's `ResourceTable`: a
    virtualized ARIA grid with a CSS-grid row layout, columns hidden by priority
    (`column-budget.ts` moves to the kit), roving focus, keyboard movement, a long-press menu, and
    the same `sort`, `selection` and `rowActions` props.

    It is fully controlled: rows, sort, selection and the focused row come in as props. It uses
    the same header, row, status, empty and failed-state styling as `DataTable`, taken from shared
    class constants.
  - **Fathom's `ResourceTable` becomes a thin adapter.** It maps the store, the descriptors,
    compiled search, capabilities and its action menu onto `DataGrid`.
- **Rationale**:
  - FR-012 says the shared table gains sorting, selection and row actions, and that Keel's tables
    keep their behaviour.
  - Constitution V says Fathom's 10,000-row lists stay virtualized.
  - A single component would force one of those to give.
- **Alternatives**:
  - Make `DataTable` virtualized. That changes Keel's markup (roles and cards), and Keel's tests
    rely on its `<table>` and `<tr>` structure.
  - Leave `ResourceTable` alone. The owner asked for headers and tables to move.

## R9. Fathom's pane header and confirm dialog

- **Decision**:
  - **Pane header.** The kit's `PageHeader` gains `size: 'page' | 'pane'`, where `'page'` is the
    default and Keel's H1. `'pane'` is a compact single-row header with a title, a count, an
    `actions` slot and a `lead` slot. Fathom puts its drag props (`paneDragProps`) on the root
    through `rootProps`, and its `PaneSlotMenu` in `lead`.
  - **Confirm dialog.** The kit's `ConfirmDialog` gains `typedConfirm?: { word: string }`. It
    takes Fathom's rule: deleting at or above `TYPED_CONFIRM_THRESHOLD` items needs the name
    typed. Fathom's `ConfirmDialog.tsx` becomes an adapter, or is removed if its callers can use
    the kit directly.
- **Rationale**: Keel wins on look. Fathom keeps the behaviour that constitution I protects.

## R10. Copy check (FR-015)

- **Decision**: `scripts/check-kit-copies.mjs` is a plain Node script with no dependencies.
  1. It reads the export names from `packages/ui/src/kit/index.ts`.
  2. It scans `apps/keel/web/src`, `apps/depths/web/src` and `ui/src` for top-level declarations
     with those names (`function X`, `class X`, `const X =`, `export { X }` from a local file).
  3. For each hit, it prints `<file>:<line> defines <X>; import it from @companion/ui/kit` and
     exits 1.

  Exceptions live in `packages/ui/kit-allow.json`, as `[{ "file", "name", "reason" }]`.

  The root `lint:packages` script becomes `eslint packages && node scripts/check-kit-copies.mjs`,
  so `ci.sh shared` runs it. A unit test runs the script against fixture directories.
- **Rationale**: the owner asked for a lint check. A script is smaller than a custom ESLint plugin
  wired into three configs, and it reads like `ci.sh`'s other guards.
- **Alternatives**: an ESLint `no-restricted-syntax` rule per app. It can't read the kit's export
  list, so the names would go stale.

## R11. Isolated tests and the harness (FR-017 to FR-019)

- **Decision**:
  - **The harness.** It is served by `vite.harness.config.ts` on port `KIT_E2E_PORT`, which
    defaults to 4190. It is one page that reads `?piece=<id>&state=<state>&theme=light|dark` and
    renders `harness/pieces/<id>.tsx`'s fixture for that state inside a `KitProvider`, with a fake
    `guideHref` and glossary.
  - **The specs.** Each piece has `tests/e2e/<id>.spec.ts`, which:
    - drives the piece's states and keyboard use;
    - calls `expectAccessible(page, { piece, state })` for every state;
    - in `tests/support/a11y.ts`, that check loops theme (light, dark) × width (360, 1280) and
      runs `new AxeBuilder({ page }).withTags(['wcag2a','wcag2aa','wcag21aa','wcag22aa'])`,
      expecting zero violations.
  - **Running.** Playwright has one project, `kit`. A single piece runs with
    `pnpm --filter @companion/ui e2e tests/e2e/<id>.spec.ts`.
- **Rationale**: each piece is tested alone, with no app and no server (FR-019). The fixtures
  double as the later gallery's content.
- **Alternatives**: Playwright component testing (`@playwright/experimental-ct-react`). That is
  another new dependency, and it is still experimental.

## R12. Keel's visual proof (SC-003)

- **Decision**:
  - Before anything moves, a new Playwright project `s065` in Keel
    (`tests/e2e/s065/visual.spec.ts`) captures goldens with `toHaveScreenshot`, using
    `maxDiffPixelRatio: 0`, animations disabled and the clock fixed.
  - It covers one screen per kit piece in use:
    - runs, with an empty state and with a list;
    - run detail, including the confirm dialog;
    - teams, for forms and tabs;
    - users, failed to load;
    - help, for Markdown, a guide link and a term popover;
    - settings tabs;
    - the shell, with a badge and an update dot;
    - a crash.
  - Each is captured at 360 and 1280 px, in light and dark: about 36 images.
  - The goldens are committed. The move must pass them unchanged.
- **Rationale**: the goldens are a durable check, not a one-off comparison, and they run on the
  same machine and browser.
- **Alternatives**: comparing `KEEL_SHOTS` folders by eye. That is not repeatable.

## R13. CI wiring

- **Decision**: `job_shared` gains these steps:
  - `pnpm --filter @companion/ui typecheck`, which is already there via `typecheck:packages`;
  - `pnpm --filter @companion/ui test`;
  - `pnpm --filter @companion/ui e2e`.

  `changed_slices` maps `packages/ui/**` to `shared`, `keel`, `depths` and `fathom`, because
  every app consumes the kit.
- **Rationale**: the kit is shared, so a change to it has to run every consumer's suite.
