---
description: "Task list for spec 063: Keel guided UI"
---

# Tasks: Keel guided UI

**Input**: Design documents from `specs/063-keel-guided-ui/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/ui.md,
contracts/glossary.md, quickstart.md

**Tests**: Required. Constitution III asks for one boundary test per story, plus unit tests for the
logic that decides what is shown. Within each phase, write the tests first. They must fail before
the implementation lands. Playwright runs against the fake backend. Nothing reaches the network.

**Gate rules for every task**:
- Run targeted checks only, one at a time, from `apps/keel/web`:
  - `pnpm run -s typecheck`, `pnpm run -s lint`;
  - `pnpm exec vitest run <file>`;
  - `pnpm exec playwright test --project s063 <file>`.
- Never run two Playwright suites at once.
- Subagents never run `scripts/ci.sh`, `paws`, `dagger` or `make gate`. The main session runs the
  full gate once (T052).
- No server, protocol, permission or column changes (FR-033). No new dependencies.
- New text uses sentence case and the `ui/form.tsx` tokens. Accent is only for primary actions and
  the current nav item (FR-031).

**Paths**: `web/` means `apps/keel/web/`, and `guides/` means `docs/guides/keel/`.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: the user story this task serves (US1 to US3)

---

## Phase 1: Setup

- [X] T001 Add Playwright project `s063` to `web/playwright.config.ts`: `testDir: './tests/e2e/s063'`, Desktop Chrome. Add a comment that its specs must not be named `enroll.spec.ts` or `settings-*.spec.ts`, because the `s060` `testMatch` would also run them. Create `web/tests/e2e/s063/`.
- [X] T002 [P] In `apps/keel/Dockerfile`'s `web` stage, add `COPY docs/guides/keel/ ./docs/guides/keel/` next to the existing `specs/060…/template-vectors.json` copy, so the build can bundle the guides (research R6).

---

## Phase 2: Foundational (blocks every story)

Guide links, used by US1 and US3, need the guide registry, the slug function and guides that
exist with their anchored headings.

- [X] T003 [P] Write `web/tests/unit/slug.test.ts` for `slug()` (R7). Cases:
  - "Enroll a host" → `enroll-a-host`;
  - punctuation is stripped ("What's a binding?" → `whats-a-binding`);
  - repeated spaces and hyphens collapse;
  - `slugger()` returns `x`, `x-2`, `x-3` for repeated headings in one page.
- [X] T004 [P] Implement `web/src/help/slug.ts`: `slug(text)` and a per-page `slugger()` (R7).
- [X] T005 Write skeletons for the six new guides, each with a title, a one-paragraph intro, and every `##`/`###` heading that a link in data-model.md or contracts/glossary.md targets. Full content comes in Phase 6.
  - `guides/getting-started.md`: Create a workspace, Give people access, Enroll your first host, Create a receiver, Bind a repository, Your first run.
  - `guides/hosts.md`: Enroll a host, Check the fingerprint, Run the base profile.
  - `guides/receivers.md`: Create a receiver, Add the webhook at your Git host, Deliveries.
  - `guides/bindings.md`: Credentials, Bind a repository.
  - `guides/reviews.md`: The early check, Trusted signers, Approve or deny a change.
  - `guides/inventory.md`: Managed and repository inventory.
  - Link all six from `guides/README.md`.
- [X] T006 Write `web/tests/unit/guide-links.test.ts`. It should:
  - read every `guides/*.md` with `node:fs`;
  - collect each file's heading slugs with `slugger()`;
  - grep `web/src/**/*.{ts,tsx}` for `guide: '…'` / `anchor: '…'` pairs, using `GLOSSARY` and an exported list of the refs each page uses;
  - assert that each one resolves. A missing guide or anchor fails with the file and ref named (SC-006).
- [X] T007 Implement `web/src/help/guides.ts` per contracts/ui.md:
  - `GuideSlug` (literal union of the ten guides) and `GuideRef`;
  - `guideHref(ref)` → `/help/<slug>[#anchor]`;
  - `loadGuide(slug | 'README')` from `import.meta.glob('../../../../../docs/guides/keel/*.md', { query: '?raw', import: 'default' })`, which gives one lazy chunk per guide.
- [X] T008 Move `web/src/features/settings/Markdown.tsx` to `web/src/ui/Markdown.tsx`, keeping its safety rules (raw HTML as text, no fetched images, `noopener` links), and update its importers (`EnrollmentTab.tsx`, `EnrollDialog.tsx`, `tests/unit/markdown.test.tsx`). Then add an optional `guideLinks` mode that:
  - gives `h2`/`h3` an `id` from one `slugger()` per render;
  - rewrites relative `*.md[#anchor]` links to `guideHref` routes in the same tab;
  - maps images under `img/` to bundled URLs (`import.meta.glob('…/docs/guides/keel/img/*', { query: '?url' })`), leaving other images as links.
  - Extend `markdown.test.tsx` to cover the new mode.
- [X] T009 Implement `web/src/ui/GuideLink.tsx`: an internal `Link` to `guideHref(to)`, with default text "Learn more", `data-guide-link={to.guide}`, and an `ArrowRightIcon` that is `aria-hidden`.
- [X] T010 Implement `web/src/help/HelpPage.tsx` and register `/help` and `/help/:guide` (lazy) in `web/src/app/routes.tsx`.
  - Render with `ui/Markdown` in `guideLinks` mode, inside `Page` with a sentence-case title taken from the guide's `#` heading.
  - Set `data-help-guide`.
  - An unknown slug renders NotFound.
  - After render, scroll to `location.hash`.
- [X] T011 Add **Help** to the account menu in `web/src/app/Shell.tsx`, after Passkeys, with `QuestionIcon`, going to `/help`.
- [X] T012 Write `web/tests/e2e/s063/help.spec.ts`:
  - `/help` lists the guides;
  - each of the six new guides renders;
  - `/help/reviews#trusted-signers` scrolls its heading into view;
  - an unknown guide shows NotFound;
  - the account menu's Help item opens `/help`.

**Checkpoint**: the guide links resolve (T006 green), and the help route works.

---

## Phase 3: User Story 1, a first-time owner sets Keel up from inside Keel (P1) 🎯 MVP

**Goal**: every FR-010 page's empty main list guides the person to the next action, or names who
can act. Values to paste elsewhere are one click to copy.

**Independent test**: quickstart.md, "empty-states" and "copy-values". The manual SC-001 walk-through.

### Tests for US1

- [ ] T013 [P] [US1] Write `web/tests/unit/empty-variant.test.ts`: `emptyVariant({filtered:true, canAct:*})` → `'no-match'`; `{filtered:false, canAct:true}` → `'guided'`; `{filtered:false, canAct:false}` → `'restricted'`.
- [ ] T014 [P] [US1] Write `web/tests/e2e/s063/empty-states.spec.ts` against the fake backend with empty lists. Cover each page in data-model.md's table (`data-empty-page`):
  - as owner: `data-empty="guided"`, exactly one primary button under `[data-empty-action]`, an `<ol>` with 2–3 items, and a `[data-guide-link]` whose slug matches the table;
  - as a viewer (no operate grant): `data-empty="restricted"`, `[data-who-can]` shown, and no button inside the empty state;
  - with no workspace: the owner sees "Create a workspace", which goes to `/teams`; a plain user sees the "owner or admin grants workspaces from Teams" line and no button (FR-013);
  - Hosts with "changed outside Keel" on: `data-empty="no-match"`, and Clear returns to the guided state (FR-014);
  - while loading or after an error, no empty state shows;
  - secondary lists (deliveries, revisions, run hosts) still show their one-line message (FR-016).
- [ ] T015 [P] [US1] Write `web/tests/e2e/s063/copy-values.spec.ts`, granting clipboard permissions:
  - in the enrol dialog, every value to paste (token, command and fingerprint if shown) has its own Copy button;
  - after creating a receiver, its URL and secret each have one;
  - each click shows "Copied", and `navigator.clipboard.readText()` equals the value;
  - the "shown once" warnings are still visible (FR-015).

### Implementation for US1

- [ ] T016 [US1] Implement `web/src/ui/empty-variant.ts` (R3).
- [ ] T017 [US1] Extend `web/src/ui/EmptyState.tsx` per contracts/ui.md:
  - new props `reason`, `steps` (2–3, as an `<ol>` with visible numbers), `whoCan` and `verb`, and `guide` (rendered as `GuideLink`);
  - `data-empty`, `data-empty-page` and `data-empty-action` / `data-who-can`;
  - centre it in the content area, with steps left-aligned in a `max-w-prose` block;
  - never render `action` and `whoCan` together, and never use illustrations (FR-011, FR-012, FR-017).
  - Existing callers (Settings, Passkeys, IdentityTab, SystemTab, ReceiversPage) must render unchanged; check with `pnpm run -s typecheck`.
- [ ] T018 [P] [US1] Implement `web/src/ui/NoMatch.tsx`: "No {what} match." plus a ghost "Clear filter" button, with `data-pane-state="empty"` and `data-empty="no-match"`.
- [ ] T019 [US1] Workspace picker, no workspace: in `web/src/app/WorkspacePicker.tsx`, replace the one-liner (line ~85) with `EmptyState` (`data-empty-page="workspace-picker"`). For owners and admins (`canSeeSettings(role)`) show the "Create a workspace" action → `/teams`. For everyone else, `whoCan="An owner or admin"`, `verb="grant workspaces from Teams"`. Guide: `getting-started#create-a-workspace`. Apply the same change to Hosts' no-workspace message (`HostsPage.tsx:164`) (FR-013).
- [ ] T020 [P] [US1] Hosts: in `web/src/features/hosts/HostsPage.tsx` (~line 313), use `emptyVariant({ filtered: changedOnly, canAct: canOperate(me, ws) })`.
  - Guided: "Enroll a host", with the three steps from data-model.md and the dialog's existing open action. Guide: `hosts#enroll-a-host`.
  - No match: `NoMatch` clearing `changedOnly`.
  - Restricted: `whoCan="An operator or workspace admin"`.
- [ ] T021 [P] [US1] Receivers: in `web/src/features/receivers/ReceiversPage.tsx` (~line 228), replace the current `EmptyState` with the guided one: "Create a receiver", the three steps, and the existing create action under the page's existing guard. Guide: `receivers#create-a-receiver`. Restricted otherwise. Deliveries (~line 205) keep their one-liner.
- [ ] T022 [P] [US1] Bindings: in `web/src/features/bindings/BindingsPage.tsx`, give the bindings list (~line 380) the guided "Bind a repository" state, using the existing grant check (the one behind `disabled={!grant}` at ~line 238) to choose guided or restricted, never a disabled button. Guide: `bindings#bind-a-repository`. The credentials list (~line 298) gets the guided "Add a credential" state, with guide `bindings#credentials`.
- [ ] T023 [P] [US1] Runs: in `web/src/features/runs/RunsPage.tsx` (~line 51), "No runs yet", with three steps and a "Go to bindings" link button for whoever may bind (else restricted). Guide: `getting-started#your-first-run`. `RunPage.tsx:121` keeps its one-liner.
- [ ] T024 [P] [US1] Reviews queue: in `web/src/features/reviews/ReviewsPage.tsx` (~line 141), "Nothing to review", with two steps and a "Trusted signers" action that scrolls to the page's trusted-signers section (only for whoever may edit signers, else restricted). Guide: `reviews#trusted-signers`.
- [ ] T025 [P] [US1] Inventory: in `web/src/features/inventory/InventoryPage.tsx` (~line 483), "No inventory yet", with three steps and a "Go to hosts" action (`canOperate`). Guide: `inventory`. Revisions (~line 236) keep their one-liner. If the page has a search or filter, an emptied result shows `NoMatch`.
- [ ] T026 [P] [US1] Teams: in `web/src/features/teams/TeamsPage.tsx`:
  - workspaces (~line 145): "Create a workspace", using the existing create action for owners and admins, else restricted;
  - teams (~line 316): "Create a team", using the existing guard;
  - guides `getting-started#create-a-workspace` and `#give-people-access`.
- [ ] T027 [US1] Copy values: audit `web/src/features/hosts/EnrollDialog.tsx` and the receiver-created result in `ReceiversPage.tsx` (R9). Every value to paste elsewhere gets its own `CopyBlock` (`data` = `enroll-token`, `enroll-command`, `receiver-url`, `receiver-secret`, keeping any existing names that tests already use). Secrets use `masked`, and their "shown once" warnings stay beside them.
- [ ] T028 [US1] Run `/impeccable clarify` and `/impeccable onboard` over the nine empty states: one-line reasons, at most three steps, verbs on buttons, sentence case. Settle the final copy in the components, and update data-model.md's table to match.
- [ ] T029 [US1] Make T013, T014 and T015 pass. Then run `tests/e2e/us*` specs that assert the old empty strings, one project at a time, and update only their assertions on empty text.

**Checkpoint**: US1 is complete, and can be shipped on its own as the MVP.

---

## Phase 4: User Story 2, the navigation shows how Keel fits together (P2)

**Goal**: three always-open headed groups on desktop, and one divided strip on a phone.

**Independent test**: quickstart.md, "nav".

- [X] T030 [P] [US2] Write `web/tests/unit/nav-groups.test.ts`:
  - `NAV_GROUPS` has ids and labels in order (Pipelines, Infrastructure, Administration), each with the sections listed in data-model.md;
  - `SECTIONS` equals the flattened order;
  - `visibleGroups()` drops a section whose `visible` returns false, and drops a group left empty (FR-001, FR-004).
- [X] T031 [P] [US2] Write `web/tests/e2e/s063/nav.spec.ts`:
  - **At 1280 px**: the headings appear in order, each followed by its links, with Settings last for an owner and absent for a user. The heading colour is not the accent (compare the computed colour with a current link's).
  - **At 360 px**: there are no visible headings, a divider sits before the first link of groups 2 and 3, and no horizontal page scroll.
  - **Accessibility**: `getByRole('group', { name: 'Pipelines' })` and the others exist, and `aria-current="page"` is on the current link (FR-002, FR-003, FR-005, FR-006).
- [X] T032 [US2] In `web/src/app/nav.ts`, add `NavGroup`, `NAV_GROUPS`, the optional `Section.visible` and `visibleGroups(me)`, and derive `SECTIONS` (R1). Update the file's doc comment.
- [X] T033 [US2] In `web/src/app/Shell.tsx`, render `Nav` from `visibleGroups(me)`:
  - one `<div role="group" aria-labelledby data-nav-group>` per group;
  - the heading uses the `LABEL` token in `text-muted-foreground`, is `sr-only` below `md`, and has spacing above each group after the first on desktop;
  - below `md`, groups 2 and 3 get a left border as a divider, with the strip's current gaps and scrolling unchanged;
  - Settings stays pinned with its dot (FR-005).
- [X] T034 [US2] Run `/impeccable polish` on the nav at 360 and 1280 px, light and dark. Make T030 and T031 pass, then run `tests/e2e/shell` (its own project) to confirm the existing shell tests still pass.

**Checkpoint**: US2 works on its own, on top of either earlier phase.

---

## Phase 5: User Story 3, jargon explains itself (P3)

**Goal**: each FR-020 term has an info icon wherever it appears in a heading, a column heading or
a label. Its text comes from one glossary. Long paragraphs become one line plus "Learn more".

**Independent test**: quickstart.md, "glossary".

- [ ] T035 [P] [US3] Write `web/tests/unit/glossary.test.ts`. It should check:
  - every FR-020 `TermId` is present;
  - each `text` has at most two sentences (split on `. `) and at most 240 characters, in sentence case;
  - each `guide` ref resolves (reusing T006's resolver).
- [ ] T036 [P] [US3] Write `web/tests/e2e/s063/glossary.spec.ts`. For one term per page (Reviews `trusted-signers`, Hosts `fingerprint`, Bindings `preview-credential`, Receivers `receiver`, Inventory `inventory-managed`, Teams `grant`):
  - mouse hover opens `[data-infotip-content]`;
  - Tab focus opens it, and the trigger's `aria-describedby` points at it;
  - Escape closes it;
  - a tap opens it, and a tap elsewhere closes it;
  - at 360 px the content fits the viewport;
  - the same term on two pages shows the same text (FR-021, FR-022, FR-024).
- [ ] T037 [US3] Implement `web/src/help/glossary.ts`: the `TermId` union, `GlossaryEntry` and `GLOSSARY`, using contracts/glossary.md as the draft. **Check each entry against the server's behaviour** (grep `apps/keel/server/src` and the 055–060 specs) before committing it, and fix the draft's wording where the facts differ.
- [ ] T038 [US3] Implement the shared `packages/ui/src/info-tip.tsx` and export it from `packages/ui/src/index.ts` (R4, contracts/ui.md). It is generic, with no Keel imports.
  - First write `web/tests/unit/info-tip.test.tsx`: hover, focus and click open it; Escape and blur close it; `aria-describedby` is set while open.
  - Then implement `web/src/help/TermTip.tsx`, which maps `GLOSSARY[term]` to `InfoTip`, with a `GuideLink` footer and the `data-infotip` / `data-infotip-content` attributes.
  - Run `pnpm --filter @companion/ui typecheck`.
  - The shared component:
  - Radix `Popover` from `radix-ui`;
  - a ghost trigger with `InfoIcon` at `size-3.5`, muted, a hit area of at least 24 × 24 px, and `aria-label="About {term}"`;
  - it opens on mouse `pointerenter`, `focus` and click or tap, and closes on mouse leave (unless click-opened), blur, Escape and an outside pointer-down;
  - `onOpenAutoFocus` is prevented;
  - the content has `role="note"`, `max-w-[min(20rem,calc(100vw-2rem))]`, `collisionPadding={16}` and `motion-reduce:animate-none`;
  - it takes an optional `footer` and passes `data-*` attributes to the trigger.
- [ ] T039 [P] [US3] Add TermTips in Reviews and Bindings (`web/src/features/reviews/ReviewsPage.tsx`, `web/src/features/bindings/`): at most once per section, after `trusted-signers`, `early-check`, `review`, `binding` and `preview-credential` wherever they appear in a heading, column heading or label (FR-021).
- [ ] T040 [P] [US3] Add TermTips in Hosts and Receivers (`web/src/features/hosts/`, `web/src/features/receivers/`) for `enrolment-token`, `fingerprint`, `base-profile`, `profile` and `receiver`.
- [ ] T041 [P] [US3] Add TermTips in Inventory, Teams and the picker (`web/src/features/inventory/`, `web/src/features/teams/`, `web/src/app/WorkspacePicker.tsx`) for `inventory-managed`, `inventory-repository`, `workspace`, `team` and `grant`.
- [ ] T042 [US3] Add the `guide?: GuideRef` prop to `web/src/ui/PageHeader.tsx`. With it set, the description is one line (`md:line-clamp-1`) and followed by `GuideLink`. Add a `SectionIntro` (one line plus `GuideLink`) beside the `ui/form.tsx` tokens for section-level text.
- [ ] T043 [US3] Shorten the long paragraphs (FR-023): inventory every page and section description over one line at 1280 px. Known ones:
  - Reviews' trusted signers (`ReviewsPage.tsx:449`);
  - Bindings' credentials;
  - Receivers, Hosts, Inventory and Teams intros.
  - Turn each into a one-line subtitle plus "Learn more" to its guide, and move the removed sentences into that guide's section so nothing is lost.
- [ ] T044 [US3] Run `/impeccable clarify` on the glossary and subtitles. Make T035 and T036 pass.

**Checkpoint**: all three stories work independently.

---

## Phase 6: Guides (FR-030)

- [ ] T045 [P] Write `guides/getting-started.md` in full. It covers workspace → access → host → receiver → binding → first run, in the guides' current voice: short numbered steps, each linking to the topic guide.
- [ ] T046 [P] Write `guides/hosts.md` in full: enrolment, the token's lifetime and single use, checking the fingerprint, running the base profile, what "changed outside Keel" means. Link `enrollment-template.md` for admins.
- [ ] T047 [P] Write `guides/receivers.md` in full: creating one, adding the webhook at the Git host (generic steps, plus notes for GitHub, GitLab and Forgejo), reading deliveries, rotating the secret.
- [ ] T048 [P] Write `guides/bindings.md` and `guides/reviews.md` in full:
  - credentials, including the preview credential;
  - binding a branch;
  - the early check, trusted signers and how they relate to `keel_runner_trusted_signers` (the text moved out of ReviewsPage in T043);
  - approving with a passkey.
- [ ] T049 [P] Write `guides/inventory.md` in full: managed vs repository inventory, revisions, importing.
- [ ] T050 Re-run `web/tests/unit/guide-links.test.ts` and `tests/e2e/s063/help.spec.ts`. Check that `guides/README.md` lists every guide.

---

## Phase 7: Polish and cross-cutting

- [ ] T051 Extend `web/tests/e2e/polish-screens.spec.ts` to cover the nine guided empty states, the grouped nav and an open InfoTip, at 360 and 1280 px in light and dark: no horizontal scroll, and no accent on group headings (SC-005, FR-032). Run it alone, with `--project s060`.
- [ ] T052 Main session only, after all agent work is merged:
  - run `scripts/ci.sh` once;
  - report its wall time, the slowest steps, the change against the last pass, and one speed-up idea;
  - confirm Depths' and Fathom's web typecheck, lint and unit tests pass with the new `@companion/ui` export;
  - run `scripts/hygiene.sh`.
- [ ] T053 Run the SC-001 manual walk-through from quickstart.md on a fresh Keel, and record the time in this file.
- [ ] T054 Last: take the brochure screenshots of the guided Hosts and Receivers empty states, the grouped nav and `/help/getting-started`, and add any guide images to `guides/img/`.

---

## Dependencies and execution order

- **Setup** (T001–T002) → **Foundational** (T003–T012) → the user stories.
- **US1** (T013–T029) depends on Foundational: it needs GuideLink and resolvable guides.
- **US2** (T030–T034) is independent of US1 and US3. It needs only Setup, and can start in
  parallel with Foundational.
- **US3** (T035–T044) depends on Foundational. It shares page files with US1, so run it after US1
  merges (T039–T041 and T043 touch the same pages as T019–T026).
- **Guides** (T045–T049) can start once T005 lands. T043 feeds T048, which needs the moved
  trusted-signers text.
- **Polish** (T051–T054) comes after everything else. Screenshots are last.

## Parallel opportunities

- **Foundational**: T003 and T004 (slug) can run alongside T005 (guides).
- **US1**: T020–T026 each edit a different page and run in parallel once T016–T018 land. T014 and
  T015 can be written in parallel with them.
- **US2**: can run as a separate agent alongside US1 (it touches only `nav.ts`, `Shell.tsx` and its
  tests). T033 and T011 both edit `Shell.tsx`, so merge T011 first.
- **US3**: T039, T040 and T041 run in parallel after T037 and T038.
- **Guides**: T045–T049 run in parallel.

## Implementation strategy

1. **MVP**: Setup, then Foundational, then US1. That is the first-run path the critique hurt most.
   Ship it and let the owner try it.
2. Add US2, which is small, and can be done by a parallel agent during US1.
3. Add US3, then complete the guides (Phase 6).
4. Polish, one CI run, then screenshots.
