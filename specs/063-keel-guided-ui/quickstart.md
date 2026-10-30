# Quickstart: validating Keel guided UI

## Prerequisites

- The repository root, with `pnpm install` done.
- No other Playwright suite or `scripts/ci.sh` running (see CLAUDE.md, "One CI at a time").

## Fast checks, while iterating

Run these from `apps/keel/web`, one at a time:

```sh
pnpm run -s typecheck
pnpm run -s lint
pnpm exec vitest run tests/unit/nav-groups.test.ts tests/unit/empty-variant.test.ts \
  tests/unit/glossary.test.ts tests/unit/guide-links.test.ts tests/unit/slug.test.ts
```

What each test proves:

- **nav-groups**: three groups in order, the members of each, a group with nothing visible is
  dropped, and `SECTIONS` keeps its order.
- **empty-variant**: filtered gives `no-match`, otherwise `canAct` decides between `guided` and
  `restricted`.
- **glossary**: every FR-020 term is present, each entry has at most two sentences and at most
  240 characters, and each guide reference resolves.
- **guide-links**: every `GuideRef` used in `src/` resolves to a bundled guide and an existing
  heading (SC-006).

## Story checks (Playwright, fake backend)

Run these from `apps/keel/web`:

```sh
pnpm exec playwright test --project s063 tests/e2e/s063/empty-states.spec.ts   # US1
pnpm exec playwright test --project s063 tests/e2e/s063/copy-values.spec.ts    # US1, FR-015
pnpm exec playwright test --project s063 tests/e2e/s063/nav.spec.ts            # US2
pnpm exec playwright test --project s063 tests/e2e/s063/glossary.spec.ts       # US3
pnpm exec playwright test --project s063 tests/e2e/s063/help.spec.ts           # FR-030, SC-006
pnpm exec playwright test --project s060 tests/e2e/polish-screens.spec.ts      # SC-005
```

What to expect:

- **empty-states**:
  - Signed in as owner, each FR-010 page shows `data-empty="guided"` with one primary button,
    numbered steps and a guide link.
  - As a viewer, it shows `data-empty="restricted"` with `data-who-can` and no button.
  - Hosts with "changed outside Keel" on shows `data-empty="no-match"`, and Clear restores the
    guided state.
- **copy-values**: every value in the enrol dialog and the new-receiver result has a Copy button.
  The clipboard holds the exact value, and "shown once" warnings stay.
- **nav**:
  - At 1280 px: the headings Pipelines, Infrastructure and Administration, in order, with their
    sections.
  - At 360 px: one strip with dividers, no visible headings, and the groups still exposed as named
    groups.
- **glossary**: one InfoTip per page opens on hover, on Tab focus and on tap, closes on Escape, and
  shows the same text for the same term on two pages.
- **help**: `/help` and each `/help/<guide>` render, and `/help/reviews#trusted-signers` scrolls to
  its heading.
- **polish-screens**: no horizontal scroll at 360 or 1280 px, in light or dark, including an open
  InfoTip.

## Manual check (SC-001)

1. Start from an empty Keel: a fresh database, signed in as the first owner.
2. Follow only the on-page guidance:
   - create a workspace;
   - enroll a host, and run the copied command on it;
   - create a receiver, and paste its URL and secret into the Git host;
   - push.
3. Time it. The target is under 15 minutes, leaving Keel only to paste values and to run the
   command.

## Batch end (main session only)

Run `scripts/ci.sh` once, on the combined branch. Then take the brochure screenshots of the new
empty states and the help page, last.
