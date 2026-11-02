# Contract: the isolated test harness and per-piece specs

## Harness page

- **Server.** `pnpm --filter @companion/ui harness` serves `packages/ui/harness` with
  `vite.harness.config.ts` on `KIT_E2E_PORT`, which defaults to 4190. Playwright's `webServer`
  starts it with `vite --config vite.harness.config.ts --port $KIT_E2E_PORT --strictPort`.
- **URL.** `/?piece=<id>&state=<state>&theme=<light|dark>`.
  - `theme` sets `html.dark` before the first paint.
  - There is no `state` default: a missing or unknown `state` renders a list of the piece's
    states as links.
  - With no `piece`, the page lists every piece. This is the seed of the later gallery.
- **CSS.** `harness.css` imports exactly what Keel's `index.css` imports: Tailwind, tw-animate,
  shadcn, the theme tokens and the effects. It also has the same `@source` lines and `html` base
  rules, including `font-size: 16px`, so a piece looks as it does in Keel.
- **Wrapping.** Each fixture renders inside the following provider and frame.

  ```tsx
  <KitProvider value={{ guideHref: (r) => `/help/${r.guide}${r.anchor ? `#${r.anchor}` : ''}`,
                        isGuide: (s) => s === 'example', glossary: HARNESS_GLOSSARY }}>
    <main aria-label="Harness" className="p-4">{fixture}</main>
  </KitProvider>
  ```

  Landmarks and the heading order are part of the page, so axe judges the piece in a realistic
  frame.
- **Fixtures.** `harness/pieces/<id>.tsx` exports
  `{ id, states: Record<string, () => ReactNode> }`, with plausible content that never mentions
  any app ("Failed to load widgets.", "Widgets", …).
- **Settling.** `window.__kitReady = true` is set after the fixture mounts and fonts have loaded
  (`document.fonts.ready`).

## Per-piece spec

`packages/ui/tests/e2e/<id>.spec.ts`, one file per piece in contracts/kit-api.md. For a piece with
no visual output, such as `router`, `slug`, `reveal` or `refusal`, the spec drives a fixture that
shows its effect. For example, the `router` fixture renders a `Link` and the current path.

Every spec:

1. Visits each state, and asserts the piece's role, accessible name and markers (FR-003).
2. Drives the piece's behaviour:
   - keyboard only, where the piece is interactive: Tab order, Enter and Space, Escape closes,
     arrow keys in tabs and the grid, focus returns to the trigger after a dialog;
   - pointer where touch matters, at 360 px.
3. Calls `await expectAccessible(page, { piece, state })` for every state.

## `tests/support/a11y.ts`

```ts
export async function expectAccessible(page: Page, at: { piece: string; state: string }): Promise<void>
```

- For each theme (light, dark) and width (360×740 and 1280×800):
  1. Go to the URL.
  2. Wait for `__kitReady`.
  3. Run `new AxeBuilder({ page }).withTags(['wcag2a','wcag2aa','wcag21a','wcag21aa','wcag22aa']).analyze()`.
  4. Expect `violations` to equal `[]`. On failure, report each rule id, its impact and the first
     node's target.
- It also asserts that `document.documentElement.scrollWidth <= innerWidth`, so the page never
  scrolls sideways (constitution IV).
- No rule is disabled globally. A rule disabled for one piece needs a comment with its reason, and
  is listed in the contributor guide.

## Playwright config

- `packages/ui/playwright.config.ts` has one project, `kit` (Desktop Chrome).
- Workers come from `autoWorkers()` (`ui/tests/support/workers.ts`), overridable with
  `PLAYWRIGHT_WORKERS`.
- `KIT_SHOTS=<dir>` saves a screenshot of every state, theme and width visited. Impeccable reviews
  these (FR-018).

## Commands

| Command | Runs |
|---|---|
| `pnpm --filter @companion/ui test` | kit unit tests (Vitest, jsdom) |
| `pnpm --filter @companion/ui e2e` | every piece's isolated spec |
| `pnpm --filter @companion/ui e2e tests/e2e/empty-state.spec.ts` | one piece alone (FR-019) |
| `pnpm --filter @companion/ui harness` | the harness in a browser |
