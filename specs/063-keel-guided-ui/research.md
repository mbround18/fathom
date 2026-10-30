# Research: Keel guided UI

No NEEDS CLARIFICATION remained after the spec. These are the design decisions the plan rests on.

## R1. Navigation groups as data, the flat list kept

- **Decision**:
  - `app/nav.ts` exports
    `NAV_GROUPS: readonly NavGroup[] = [{ id: 'pipelines', label: 'Pipelines', sections: [...] }, ...]`.
  - `SECTIONS` becomes `NAV_GROUPS.flatMap((g) => g.sections)`.
  - Each `Section` may carry `visible?: (me: Me) => boolean`. It is absent today, so every section
    shows.
  - `visibleGroups(me)` drops hidden sections, then empty groups.
  - The Shell renders `<div role="group" aria-labelledby={headingId}>` per group, with the heading
    as a `LABEL`-styled `<span>`:
    - On desktop it's visible.
    - On a phone it's `sr-only`, so screen readers still announce the group (FR-006), and every
      group after the first has a left border as its divider (FR-003).
  - Settings stays outside the groups, pinned (FR-005).
- **Rationale**:
  - Other code reads `SECTIONS` (routes, Home, tests).
  - Deriving it keeps a single order.
  - `visible` makes FR-004 testable now. No section is role-gated today, but Settings already is,
    and Migration may become so.
- **Alternatives considered**:
  - Hard-coding groups in the Shell. Rejected: the order would live in two places.
  - A `<ul>` per group with headings. Rejected: `role="group"` plus `aria-labelledby` is the
    lighter landmark, and works the same in the strip and the column.

## R2. One `EmptyState`, extended rather than a second component

- **Decision**:
  - Extend `ui/EmptyState.tsx` with optional props:
    - `reason` (one line);
    - `steps` (an array of two or three items, rendered as an `<ol>`);
    - `guide` (a `GuideRef`, rendered as a `GuideLink`);
    - `whoCan` (for example "An owner or admin"), shown when `action` is absent as
      "{whoCan} can {verb}", with `verb` supplied.
  - Existing callers (Settings, Passkeys, IdentityTab, SystemTab) keep working unchanged.
  - The empty state sits centred in the content area, and the steps are left-aligned inside it, so
    they read as a list.
  - The action is the page's existing primary button or dialog trigger, passed in.
  - Pages decide whether the person can act with the checks they already use: `canOperate(me, ws)`
    for hosts and receivers, `canSeeSettings(role)` for instance-level actions, and the existing
    grant check on Bindings.
- **Rationale**: one visual language for "nothing here". FR-012 forbids a dead button, so a page
  passes either an `action` or a `whoCan`, never a disabled button.
- **Alternatives considered**: a separate `GuidedEmpty`. Rejected: it would duplicate the frame,
  and drift.

## R3. Picking the variant is a pure function

- **Decision**: `ui/empty-variant.ts` exports
  `emptyVariant({ filtered, canAct }): 'no-match' | 'guided' | 'restricted'`.
  - `filtered` wins: a list emptied by a filter shows `NoMatch`, with a Clear button that resets
    the page's filter (FR-014).
  - Otherwise `canAct` chooses between guided and restricted.
  - Loading and error stay in `LoadState`/`ListState` as they are (edge case).
  - The DOM carries `data-empty="guided|restricted|no-match"` for tests.
- **Filters in scope today**:
  - Hosts' "changed outside Keel" toggle.
  - Any search or filter a page in FR-010 already has. The tasks list each one found.
- **Rationale**: Principle III asks for unit tests of logic that decides what the person sees.
- **Alternatives considered**: deciding inline in each page. Rejected: that can't be unit tested,
  and it is easy to get wrong in nine places.

## R4. InfoTip is a shared toggletip on Radix Popover

- **Decision**: `packages/ui/src/info-tip.tsx` exports
  `<InfoTip label="trusted signers" footer={…}>{text}</InfoTip>`, and `@companion/ui` re-exports it.
  It is generic, with no glossary in it.
  - Keel's `help/TermTip.tsx` exports `<TermTip term="trusted-signers" />`. It looks up
    `GLOSSARY`, passes the text, passes a `GuideLink` as the footer, and passes the `data-infotip*`
    attributes through.
  - The owner chose the shared package (2026-10-29), so Depths and Fathom can adopt it later.
  - **The trigger** is a ghost icon button (`InfoIcon`, `size-3.5`, muted, not accent):
    - its `aria-label` is "About {term}";
    - it has a hit area of at least 24 × 24 px (WCAG 2.2 target size), padded so it doesn't push
      text.
  - **Opening**:
    - on `pointerenter` when `pointerType === 'mouse'`;
    - on `focus`;
    - on `click` or tap, which toggles it.
  - **Closing**:
    - on `pointerleave` (mouse), unless it was opened by click;
    - on `blur`, on Escape, and on a pointer-down outside it (Radix `onInteractOutside`).
  - **The content** is `PopoverContent`:
    - `onOpenAutoFocus` is prevented, so focus stays on the trigger;
    - `role="note"`, and the trigger has `aria-describedby` set to the content id while it is open,
      so a screen reader reads the explanation (FR-022);
    - its width is `max-w-[min(20rem,calc(100vw-2rem))]`, with `collisionPadding={16}`, so it fits
      at 360 px;
    - it has an optional "Learn more" link when the entry names a guide;
    - its animation is `motion-reduce:animate-none`.
- **Rationale**:
  - The Radix Tooltip in `@companion/ui` deliberately ignores touch, and a tooltip can't hold a
    link. A toggletip covers hover, focus and tap.
  - `radix-ui` 1.8 is already a peer dependency of `packages/ui` and a direct dependency of
    keel-web, so this adds nothing.
  - Keel's Vitest covers it (`info-tip.test.tsx`), because `packages/ui` has no test runner.
- **Alternatives considered**:
  - `@companion/ui` Tooltip. Rejected: there is no tap support (FR-022).
  - A native `popover` attribute. Rejected: there is no collision handling at 360 px, and focus
    and Escape handling would have to be hand-rolled.

## R5. The glossary is a typed module

- **Decision**:
  - `help/glossary.ts` exports `GLOSSARY: Record<TermId, GlossaryEntry>`, with
    `GlossaryEntry = { term: string; text: string; guide?: GuideRef }`.
  - `TermId` is a string-literal union, so `<InfoTip term="…">` is type-checked.
  - The draft text is in [contracts/glossary.md](contracts/glossary.md). Implementation checks
    each entry against the code and the server's behaviour before committing it.
- **Rationale**:
  - FR-024 asks for a single source.
  - The type makes a misspelt term a compile error.
  - The unit test enforces the FR-020 term list and the length limits.
- **Alternatives considered**: putting the glossary in Markdown and parsing it at runtime.
  Rejected: that is more code, and it loses the type check.

## R6. Guides are shown inside Keel, from the same Markdown files

- **Decision**:
  - `help/guides.ts` builds the registry with
    `import.meta.glob('../../../../../docs/guides/keel/*.md', { query: '?raw', import: 'default' })`,
    one lazy chunk per guide.
  - The registry exports `GuideSlug` (a literal union, checked by the test), `GuideRef = { guide:
    GuideSlug; anchor?: string }` and `guideHref(ref)`, which returns `/help/<slug>#<anchor>`.
  - `HelpPage.tsx` serves two routes:
    - `/help` renders `README.md` as the index;
    - `/help/:guide` renders the guide.
  - It renders through the existing safe Markdown renderer, which moves from `features/settings/`
    to `ui/Markdown.tsx` with the same rules: raw HTML as text, and links in a new tab.
  - `h2` and `h3` get `id={slug(text)}`. On load the page scrolls to the hash.
  - Relative links between guides (`hosts.md#enroll`) are rewritten to `/help/hosts#enroll`.
  - Links outside the guides open in a new tab.
  - Images resolve only against images bundled from `docs/guides/keel/img/`. Others become links,
    as they do today.
  - The Keel Dockerfile's `web` stage adds `COPY docs/guides/keel/ ./docs/guides/keel/`.
  - The account menu gains a "Help" item, which goes to `/help`.
- **Rationale**:
  - Keel is self-hosted, often on a private network.
  - The repository may not be reachable, and a GitHub link would show `main`, not the deployed
    version.
  - Bundling makes SC-006 checkable in a unit test, and keeps one source for the in-app guides and
    the repository's.
- **Alternatives considered**:
  - Linking to GitHub. Rejected: the link breaks offline, shows the wrong version, and the repo
    may be private.
  - Serving `docs/` from keel-server. Rejected: that is a server change (FR-033).

## R7. Heading slugs

- **Decision**: `help/slug.ts`:
  - lower-cases the heading;
  - strips anything that isn't a letter, a digit, a space or a hyphen;
  - turns spaces into hyphens;
  - collapses repeated hyphens.
  - On a duplicate it appends `-2`, `-3` and so on, per page.
  - This matches GitHub's anchors for the guides' plain headings, so the same `#anchor` works in
    the repository too.
- **Rationale**: it avoids adding `rehype-slug` and `github-slugger` for about 15 lines of code.
- **Alternatives considered**: `rehype-slug`. Rejected: it is a new dependency.

## R8. Long paragraphs become a subtitle plus "Learn more"

- **Decision**:
  - `PageHeader` gains `guide?: GuideRef`. The description is clamped to one line on desktop
    (`md:line-clamp-1`) and followed by `GuideLink`.
  - Section headers with paragraphs get the same treatment, through a small `SectionIntro`
    component in `ui/form.tsx`'s neighbourhood.
  - The tasks inventory each paragraph found: Reviews' trusted signers, Bindings' credentials,
    Receivers, Hosts, Inventory and Teams. Each one's full text moves into its guide.
- **Rationale**: FR-023. The text isn't lost, it moves to where it can be longer.
- **Alternatives considered**: a collapsible "details" block. Rejected: the owner declined
  collapsing for the nav, and the critique asked for less text on the page.

## R9. Copy buttons

- **Decision**:
  - Reuse `ui/CopyBlock.tsx`, which already gives "Copied" feedback and has `masked` with Reveal.
  - Audit two places:
    - `features/hosts/EnrollDialog.tsx`: token, command and fingerprint, if shown;
    - receiver creation in `features/receivers/ReceiversPage.tsx`: URL and secret.
  - Each value to be pasted elsewhere gets its own `CopyBlock`.
  - Values shown once keep their "shown once" warning next to the copy row (FR-015).
  - The Hosts and Receivers empty states don't hold the values themselves, because those don't
    exist until the action runs. Their steps say where the values will appear.
- **Rationale**: the component exists and is tested. The gap is coverage, not capability.

## R10. Tests and layout

- **Decision**:
  - A new Playwright project, `s063`, at `tests/e2e/s063/`:
    - `nav.spec.ts`: groups, order, phone strip dividers, accessible names.
    - `empty-states.spec.ts`: each FR-010 page in the guided, restricted and no-match variants,
      seeded from the fake backend as owner, operator and viewer.
    - `copy-values.spec.ts`: the enrol dialog and a new receiver, clicking each Copy and reading
      the clipboard.
    - `glossary.spec.ts`: hover, focus, tap and Escape on one term per page.
    - `help.spec.ts`: each guide route renders, and each anchor link scrolls to its heading.
  - `polish-screens.spec.ts` gains the empty states, the grouped nav and an open InfoTip, at 360
    and 1280 px, in light and dark (SC-005).
  - SC-001's 15-minute walk-through is a manual check in quickstart.md. Its steps are covered
    automatically by `empty-states` and `copy-values`.
- **Rationale**: Principle III, one boundary test per story, plus the layout guard that already
  exists.
