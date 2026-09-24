---

description: "Task list for the Command Palette feature"
---

# Tasks: Command Palette

**Input**: Design documents from `specs/016-command-palette/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, quickstart.md

**Tests**: Included — the parent task explicitly requires a focused Playwright e2e spec, and the
constitution (Principle III) requires unit tests for UI search/ranking logic.

**Organization**: Tasks are grouped by user story (P1/P2/P3 from spec.md) so each can be
implemented and verified independently. All work is in `ui/` — no backend/protocol changes.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no unmet dependency)
- **[Story]**: US1, US2, or US3 (maps to spec.md priorities)

## Phase 1: Setup

**Purpose**: Confirm the ground this feature builds on, no new dependencies needed.

- [x] T001 Confirm `cmdk` (already in `ui/package.json`) and `ui/src/components/ui/command.tsx`'s
      `CommandDialog`/`CommandGroup`/`CommandItem`/`CommandEmpty` exports build cleanly with a
      throwaway `pnpm -C ui typecheck` run before writing new code (no file changes; a sanity
      checkpoint only).

---

## Phase 2: Foundational (blocking prerequisites for all user stories)

**Purpose**: The shared search-index module and its types that every user story's results are
built from. No user story can be verified until this phase is done.

**⚠️ CRITICAL**: Do not start Phase 3+ until this phase's tasks are complete.

- [x] T002 [P] Define `PaletteQuery`, `PaletteResultCategory`, `PaletteResult`,
      `PaletteResourcePayload`, `PaletteNamespacePayload`, `PaletteSectionPayload` types in
      `ui/src/resources/paletteIndex.ts`, exactly matching the shapes in
      `specs/016-command-palette/data-model.md` (including the `id`, `category`, `label`,
      `sublabel`, `matchScore`, `payload` fields on `PaletteResult`).
- [x] T003 [US-shared] Implement `parsePaletteQuery(raw: string): PaletteQuery` in
      `ui/src/resources/paletteIndex.ts`: split on the *first* `:` only; `kind` is the text before
      it (or `null` if no `:` present); `name` is the text after it (or the full `raw` string if
      no `:`); an empty `name` after a `kind:` prefix (e.g. `"pod:"`) is valid and yields an empty
      string to match against (data-model.md's parsing rule).
- [x] T004 [P] [US-shared] Add a Vitest unit test file `ui/tests/unit/paletteIndex.test.ts`
      covering `parsePaletteQuery`: no colon → `{ kind: null, name: raw }`; `"pod:my-app"` →
      `{ kind: "pod", name: "my-app" }`; `"pod:"` → `{ kind: "pod", name: "" }`; a name containing
      a second colon (e.g. `"pod:a:b"`) → `{ kind: "pod", name: "a:b" }` (splits on first `:` only).

---

## Phase 3: User Story 1 - Jump to a known resource by name (Priority: P1) 🎯 MVP

**Goal**: Cmd/Ctrl+K opens a palette; fuzzy-searching (optionally `kind:name`-scoped) a
currently-loaded resource and selecting it opens the Inspector on that exact resource.

**Independent Test**: With the app connected and resources loaded, press Cmd/Ctrl+K, type a
substring of a known resource's name, select the top match, and confirm the Inspector opens for
that exact resource.

- [x] T005 [US1] Implement `buildResourceCandidates(store): PaletteResult[]` in
      `ui/src/resources/paletteIndex.ts`: iterate `store.lists` (the `ListKey`s as defined in
      `ui/src/store/keys.ts`), keep only entries whose context segment matches
      `store.session`'s active context (FR-012 — current context only), resolve each entry's
      `SectionKey` via `descriptorFor(section)` from `ui/src/resources/index.ts`, and for each row
      in `ListState.rows` call `descriptor.ref(row)` to build a `PaletteResourcePayload`; set
      `label` to the ref's `name`, `sublabel` to `` `${descriptor.kind} · ${ref.namespace ?? '—'}` ``,
      and `id` to `` `resource:${ref.section}:${ref.namespace}:${ref.name}` ``. Do not read any
      field beyond what `descriptor.ref(row)` and the descriptor's own `kind`/`label` expose (no
      labels/annotations/spec/status — FR-013).
- [x] T006 [US1] Implement `rankAndFilter(candidates: PaletteResult[], query: PaletteQuery, limit: number): PaletteResult[]`
      in `ui/src/resources/paletteIndex.ts`: when `query.kind` is set, first keep only resource
      candidates whose descriptor `kind` fuzzily matches `query.kind` (case-insensitive substring
      or subsequence match — no match on an unrecognized kind means zero resource results per the
      spec's edge case, not an unfiltered fallback); then fuzzy/subsequence-match `query.name`
      against each remaining candidate's `label`, drop non-matches, sort matches by `matchScore`
      descending, and cap the result to `limit` entries (FR-007).
- [x] T007 [P] [US1] Add unit tests in `ui/tests/unit/paletteIndex.test.ts` for
      `buildResourceCandidates` (given a fake `lists` map with multiple contexts, only the active
      context's rows appear) and `rankAndFilter` (kind-prefix filtering narrows correctly; an
      unknown kind prefix yields zero resource results; matches are ranked strongest-first; result
      count never exceeds the passed `limit`).
- [x] T008 [US1] Create `ui/src/components/CommandPalette.tsx`: a component that takes
      `open`/`onOpenChange` props, renders `CommandDialog` (`ui/src/components/ui/command.tsx`)
      containing a `CommandInput` bound to local query state, a `CommandList` with a `CommandEmpty`
      ("No results.") fallback, and a `CommandGroup` (heading "Resources") of `CommandItem`s built
      from `rankAndFilter(buildResourceCandidates(...), parsePaletteQuery(query), CAP)`; selecting
      a resource item calls `store.openInspector(ref)` (no `from`/`push` args — a fresh jump per
      research.md) and then `onOpenChange(false)`.
- [x] T009 [US1] Create `ui/src/components/shell/CommandPaletteHost.tsx`: a small component
      mounted once in the app shell (alongside `TopBar.tsx`) that owns the palette's `open` boolean
      state, attaches one `document.addEventListener('keydown', ...)` listener on mount (removed on
      unmount) matching `(e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k'`, calls
      `e.preventDefault()`, and opens the palette (toggling closed if already open), mirroring the
      `useEscapeToClose` pattern in `ui/src/components/Inspector.tsx:472-485`; renders
      `<CommandPalette open={open} onOpenChange={setOpen} />`.
- [x] T010 [US1] Wire `CommandPaletteHost` into the app shell so it mounts globally (find where
      `TopBar.tsx` is mounted, e.g. in the shell/root layout component, and render
      `CommandPaletteHost` as a sibling there so Cmd/Ctrl+K works from any pane).
- [x] T011 [US1] Write `ui/tests/e2e/us1-command-palette.spec.ts` following the existing
      `us<N>-<feature>.spec.ts` + `helpers.ts` scripted-fake-backend convention (see
      `ui/tests/e2e/us1-pods-list.spec.ts` for the pattern): `open(page)` with a scenario that
      seeds at least one known pod, `page.keyboard.press('Meta+k')` (or `Control+k` per platform),
      assert the palette dialog is visible, type a substring of the seeded pod's name into the
      palette input, assert the matching result renders, click/Enter-select it, assert the palette
      closes and the Inspector (via existing `data-testid`/`data-slot` hooks) shows that exact
      pod's namespace/name. Keep this to one tight scenario, not a full regression suite, per the
      parent task's instruction.

**Checkpoint**: User Story 1 is independently testable and deployable as the MVP at this point.

---

## Phase 4: User Story 2 - Jump to a namespace (Priority: P2)

**Goal**: Searching a namespace substring in the palette and selecting it sets the active
namespace filter.

**Independent Test**: With multiple namespaces available, open the palette, type a namespace
substring, select the matching result, and confirm the active namespace filter updates — without
depending on resource or section results.

- [x] T012 [US2] Implement `buildNamespaceCandidates(store): PaletteResult[]` in
      `ui/src/resources/paletteIndex.ts`: read the store's already-loaded namespace list rows
      (the `namespaces` section's `ListState.rows`, if loaded — same "already loaded" constraint
      as resources, per data-model.md), map each to a `PaletteResult` with `category: 'namespace'`,
      `label` = the namespace name, `payload.namespace` = the namespace name, `id` =
      `` `namespace:${name}` `` (no `sublabel` per data-model.md).
- [x] T013 [P] [US2] Add a unit test in `ui/tests/unit/paletteIndex.test.ts` for
      `buildNamespaceCandidates` confirming it returns an empty array (not an error) when the
      namespace list hasn't been loaded yet (FR-016's graceful-degradation requirement).
- [x] T014 [US2] In `ui/src/components/CommandPalette.tsx`, add a second `CommandGroup` (heading
      "Namespaces") rendering `rankAndFilter(buildNamespaceCandidates(...), query, CAP)` results,
      visually distinguished from the Resources group by its separate heading (FR-006); selecting a
      namespace item calls `store.selectNamespace(namespace)` and then `onOpenChange(false)`.
- [x] T015 [US2] Extend `ui/tests/e2e/us1-command-palette.spec.ts` — or add
      `ui/tests/e2e/us2-command-palette-namespace.spec.ts` if keeping US1's spec single-scenario is
      preferred — with one scenario: open the palette, type a seeded namespace substring, select
      it, assert the active namespace filter (existing namespace picker UI) reflects that
      namespace.

**Checkpoint**: User Story 2 works independently of US3, and does not regress US1.

---

## Phase 5: User Story 3 - Jump to a tool pane (Priority: P3)

**Goal**: Searching a sidebar section's name in the palette and selecting it navigates to that
section.

**Independent Test**: Open the palette, type a known section's display name (e.g. "settings"),
select the matching result, and confirm the app navigates to that section — independent of
resource/namespace results.

- [x] T016 [US3] Implement `buildSectionCandidates(): PaletteResult[]` in
      `ui/src/resources/paletteIndex.ts`: flatten `NAV_GROUPS` (`ui/src/resources/nav.ts`) into one
      `PaletteResult` per `{ key, label }` entry across all groups, with `category: 'section'`,
      `label` = the section's label, `sublabel` = the owning `NavGroup.label`, `payload.section` =
      the `SectionKey`, `id` = `` `section:${key}` ``. This is static data (no store dependency),
      per research.md's decision to scope pane search to `NAV_GROUPS` only (not `ToolKind` dock
      tabs).
- [x] T017 [P] [US3] Add a unit test in `ui/tests/unit/paletteIndex.test.ts` confirming
      `buildSectionCandidates` returns a non-empty, static result set with no store/connection
      dependency (covers the edge case: pane search works with zero resources loaded).
- [x] T018 [US3] In `ui/src/components/CommandPalette.tsx`, add a third `CommandGroup` (heading
      "Sections") rendering `rankAndFilter(buildSectionCandidates(), query, CAP)` results; selecting
      a section item calls `store.navigate(section)` and then `onOpenChange(false)`.
- [x] T019 [US3] Add one scenario to `ui/tests/e2e/us1-command-palette.spec.ts` (or a dedicated
      `ui/tests/e2e/us3-command-palette-section.spec.ts`): open the palette, type a known section's
      label (e.g. "settings"), select it, assert the app navigated to that section (via existing
      DOM hooks for the active section/pane).

**Checkpoint**: All three user stories work independently and together.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final correctness/consistency pass across all three stories.

- [x] T020 [P] In `ui/src/components/CommandPalette.tsx`, ensure full keyboard operability
      (FR-008): arrow-key highlight movement and Enter-to-select are handled by `cmdk`'s built-in
      behavior (verify no custom handler suppresses it); Escape closes via the underlying `Dialog`.
- [x] T021 [P] Verify the empty-state message (FR-015) renders once across all three groups
      combined (a single `CommandEmpty` shown only when zero results exist in total, not one per
      group) and add a unit/e2e assertion for it if not already covered by T011.
- [x] T022 Run `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, and
      `pnpm -C ui e2e us1-command-palette.spec.ts` (plus any US2/US3 e2e specs added), fixing any
      failures, per `specs/016-command-palette/quickstart.md`'s Automated Checks section.

---

## Dependencies & Execution Order

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: no user story work starts before T002–T004
  land (shared types + query parsing).
- **Phase 3 (US1)** depends only on Phase 2. It is the MVP slice.
- **Phase 4 (US2)** depends only on Phase 2 (reuses `rankAndFilter`/`PaletteResult` from
  Foundational, and `CommandPalette.tsx` from US1's T008 as the file it extends) — implementable
  and testable independent of US3.
- **Phase 5 (US3)** depends only on Phase 2 and `CommandPalette.tsx` (from US1's T008) — 
  implementable and testable independent of US2.
- **Phase 6 (Polish)** runs after all desired stories are complete.

Note: US2 and US3 both add a `CommandGroup` to the same `CommandPalette.tsx` file created in US1
(T008), so while their *logic* (`buildNamespaceCandidates`/`buildSectionCandidates`) is
independent and parallelizable, their UI wiring tasks (T014, T018) touch the same file
sequentially — do not run T014 and T018 concurrently against uncommitted changes to avoid merge
conflicts in that one file.

## Parallel Execution Examples

- Phase 2: T002 and T004 can run in parallel with each other (different concerns), but T003
  depends on T002's types existing first.
- Phase 3: T007 (US1 unit tests) can be written in parallel with T008 (component) once T005/T006
  land, since they target different files.
- Across stories: T005/T006 (US1 logic), T012 (US2 logic), and T016 (US3 logic) are all in the
  same file (`paletteIndex.ts`) but are additive, independent functions — safe to parallelize as
  long as each is appended without touching another's function body.

## Implementation Strategy

**MVP first**: Complete Phase 1 → Phase 2 → Phase 3 (US1) and stop there for an initial,
independently shippable increment — resource jump is the feature's entire stated value
proposition. Add Phase 4 (US2, namespace jump) and Phase 5 (US3, section jump) as incremental,
independently testable follow-ups in priority order. Phase 6 polish applies once all desired
stories are in.
