# Implementation Plan: Keel guided UI

**Branch**: `main` (agent work lands on worktree branches, merged into `main`) | **Date**: 2026-10-29 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/063-keel-guided-ui/spec.md`

## Summary

A web-only change to `apps/keel/web`, with no server, protocol, permission or column changes
(FR-033).

1. **Guided empty states (US1)**:
   - `ui/EmptyState.tsx` grows a guided form: a reason line, two or three numbered steps, one
     primary action, or the role that can act when the person can't, and a guide link (R2).
   - Each page in FR-010 picks one of three variants from data it already has: **guided**,
     **restricted** (no action, names who can) or **no match** (a filter emptied the list, with a
     Clear button). The choice is a pure function with unit tests (R3).
   - The "who can act" decision reuses the checks pages already make (`canOperate`,
     `canSeeSettings`, the grant checks on Bindings). No new permission logic (R2).
   - Copy buttons: `CopyBlock` already exists. Tasks audit the enrol dialog and receiver creation,
     and add a copy row to every value that has to be pasted elsewhere (R9).
2. **Grouped navigation (US2)**:
   - `app/nav.ts` gains `NAV_GROUPS` (Pipelines, Infrastructure, Administration). `SECTIONS` stays
     as the flattened list, so its current users don't change (R1).
   - In the Shell, each group is a `role="group"` labelled by its heading.
     - On desktop the heading shows as a muted label.
     - On a phone the heading is visually hidden, and the group draws a left divider in the strip.
   - A group with no visible section isn't rendered.
3. **Help for jargon (US3)**:
   - `help/glossary.ts` is the only source of explanation text (R5).
   - `InfoTip` is a toggletip in the shared `@companion/ui` (`packages/ui/src/info-tip.tsx`),
     built on the Radix Popover from `radix-ui`, which `packages/ui` already takes as a peer
     dependency (no new dependency). It opens on mouse hover, keyboard focus and tap, and closes on
     Escape, blur and an outside tap (R4).
     - It knows nothing about Keel: it takes a label and content.
     - Keel's `help/TermTip.tsx` feeds it the glossary text and the "Learn more" link.
     - Depths and Fathom can use it, but this feature doesn't change them.
   - `PageHeader` and the section headers gain a `guide` prop, which renders "Learn more". Long
     paragraphs become one line (R8).
4. **Guides (FR-030)**:
   - There are six new Markdown guides in `docs/guides/keel/`.
   - Keel shows them itself, on a lazily loaded `/help/:guide` route. The route renders the same
     files, bundled at build time, with the existing safe Markdown renderer.
   - So every link resolves against the version that is deployed, offline and without access to
     the repository. A unit test fails if a link names a guide or heading that doesn't exist
     (R6, SC-006).

## Technical Context

**Language/Version**: TypeScript 6 and React 19, built with Vite, for `apps/keel/web`. Markdown for
`docs/guides/keel`. No Rust changes.

**Primary Dependencies**:

- Existing: `@companion/ui` (Button, and the new shared `InfoTip`), `radix-ui` 1.8 (Popover for `InfoTip`),
  `@phosphor-icons/react` (`InfoIcon`), `react-markdown` and `remark-gfm` (guides, already lazily
  loaded by `features/settings/Markdown.tsx`), `zustand`.
- **No new dependencies.**
  - Heading anchors come from a small slug function with unit tests, not `rehype-slug`.
  - Guides load with Vite's `import.meta.glob(..., { query: '?raw' })`.

**Storage**: None.

**Testing**:

- Vitest:
  - `nav-groups.test.ts`: order, membership and hiding.
  - `empty-variant.test.ts`: guided, restricted or no match, for each page's inputs.
  - `glossary.test.ts`: the required terms are present; each entry is at most two sentences and at
    most 240 characters; each guide reference resolves.
  - `guide-links.test.ts`: every `GuideRef` in the source resolves to a bundled guide and heading
    (SC-006).
  - `slug.test.ts`.
  - `info-tip.test.tsx`: the shared `InfoTip` opens on hover, focus and click, closes on Escape
    and blur, and sets `aria-describedby` (`packages/ui` has no test runner of its own, so Keel's
    Vitest covers it).
- Playwright: a new project, `s063` (`tests/e2e/s063/`), with one spec per story and a help-route
  spec, against the fake backend.
  - Don't name a spec `enroll.spec.ts` or `settings-*.spec.ts`: the `s060` project's `testMatch`
    would pick it up too.
- Layout:
  - Extend `tests/e2e/polish-screens.spec.ts` with the new empty states, the grouped nav and an
    open InfoTip.
  - Check the 360 px and 1280 px widths, in light and dark (SC-005).

**Target Platform**: Evergreen browsers, from a phone (360 px) to desktop. The page is served by
the Keel server container.

**Project Type**: The existing Keel web app (spec 055) and its user guides.

**Performance Goals**:

- The glossary is a small static module.
- Each guide is its own lazy chunk, loaded only when `/help/:guide` opens.
- Pages load no help text until an InfoTip opens. The text itself is static and already in the
  page's chunk.
- The main chunk must not grow by more than the nav groups and the `InfoTip`/`EmptyState` code.

**Constraints**:

- No horizontal page scroll at 360 px (FR-032, Principle IV).
- Every new control is operable by touch, mouse and keyboard.
- Accent stays reserved for primary actions and the current nav item (FR-031).
- Popover motion respects `prefers-reduced-motion`.
- Sentence case, and the H1/H2/H3/LABEL tokens from `ui/form.tsx`.

**Scale/Scope**:

- Ten nav sections in three groups.
- Nine empty states: Runs, the Reviews queue, Bindings, Receivers, Inventory, Hosts, Teams
  workspaces, Teams teams, and the workspace picker.
- Fifteen glossary terms and six new guides.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Operator-first, explicit mutation | No new mutations. Each empty-state action opens the dialog or page that already performs that action, with its existing confirmation and step-up. | Pass |
| II. One contract per app | No protocol or server change (FR-033). Guides ship in the web bundle. | Pass |
| III. Tested at the boundaries | `s063` Playwright specs, one per story, against the fake backend. Unit tests for every function that decides what the person sees: group visibility, empty variant, glossary shape, guide-link resolution, slugs. | Pass |
| IV. Density scales with width | Phone: one strip with dividers. Desktop: headed groups, which adds information and removes nothing. No horizontal scroll at 360 px. The InfoTip works by touch, mouse and keyboard. Contrast comes from existing tokens. Motion is reduced when the person asks for that. | Pass |
| V. Fast and quiet | Static glossary and lazy guide chunks. No timers and no idle work. | Pass |
| VI. User data is sacred | No data is touched. | Pass |
| Workflow | Spec → plan → tasks → implement, signed commits, no new dependencies. | Pass |
| Shared packages | `InfoTip` is a new export in `@companion/ui`. Nothing existing changes, so Depths and Fathom behave the same; their typecheck is part of the batch-end gate. | Pass |

**Post-design re-check (after Phase 1)**: Pass.

- The `/help/:guide` route is a client-only page, and `radix-ui` Popover is already a dependency.
- The Keel Dockerfile's web stage copies `docs/guides/keel/` so the build can bundle the guides.
  That is a build-input change, not a server change.
- Nothing needs Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/063-keel-guided-ui/
├── plan.md              # This file
├── research.md          # R1–R10 decisions
├── data-model.md        # Nav groups, empty-state table, glossary, guide refs
├── quickstart.md        # Validation run
├── contracts/
│   ├── ui.md            # Component props, routes, data-* hooks for tests
│   └── glossary.md      # The glossary text (draft; the TS module is the source once built)
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
apps/keel/
├── Dockerfile                         # web stage: COPY docs/guides/keel/ (R6)
└── web/
    ├── playwright.config.ts           # + project s063
    ├── src/
    │   ├── app/
    │   │   ├── nav.ts                 # + NAV_GROUPS; SECTIONS derived from it (R1)
    │   │   ├── Shell.tsx              # grouped Nav; Help in the account menu
    │   │   ├── WorkspacePicker.tsx    # no-workspace guided / restricted state (FR-013)
    │   │   └── routes.tsx             # + /help, /help/:guide (lazy)
    │   ├── help/
    │   │   ├── glossary.ts            # GLOSSARY: the only tooltip text (FR-024)
    │   │   ├── guides.ts              # GUIDES registry, GuideRef, guideHref(), import.meta.glob
    │   │   ├── slug.ts                # heading → anchor id
    │   │   ├── TermTip.tsx            # <TermTip term>: glossary text in the shared InfoTip
    │   │   └── HelpPage.tsx           # /help and /help/:guide, renders bundled Markdown
    │   ├── ui/
    │   │   ├── EmptyState.tsx         # + reason, steps, guide, whoCan (R2)
    │   │   ├── empty-variant.ts       # guided | restricted | no-match (R3)
    │   │   ├── NoMatch.tsx            # "Nothing matches" + Clear
    │   │   ├── GuideLink.tsx          # "Learn more" / guide link
    │   │   └── PageHeader.tsx         # + guide prop (FR-023)
    │   └── features/                  # runs, reviews, bindings, receivers, inventory, hosts,
    │                                  # teams: empty states, InfoTips, one-line subtitles
    └── tests/
        ├── unit/                      # nav-groups, empty-variant, glossary, guide-links, slug
        └── e2e/
            ├── s063/                  # nav, empty-states, copy-values, glossary, help
            └── polish-screens.spec.ts # + new layout cases (SC-005)
packages/ui/src/
├── info-tip.tsx                       # new shared toggletip (R4)
└── index.ts                           # + export
docs/guides/keel/
├── README.md                          # index: + the six new guides
├── getting-started.md                 # new
├── hosts.md                           # new
├── receivers.md                       # new
├── bindings.md                        # new
├── reviews.md                         # new
└── inventory.md                       # new
```

**Structure Decision**:

- The work stays inside the existing Keel web app. Help code gets its own folder (`src/help/`)
  because it is shared by every feature.
- `InfoTip` goes in `packages/ui`, as the owner chose (2026-10-29), so every app can use it. It
  stays generic: the glossary and guide links stay in Keel.
- `EmptyState` stays in Keel's `src/ui/`. It can be promoted later if Depths or Fathom want it.

## Complexity Tracking

No violations.
