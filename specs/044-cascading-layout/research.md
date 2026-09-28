# Phase 0 research: cascading wide-screen layout

Date: 2026-09-27. All findings come from reading the current code, not from changing it.

## Starting point in the code

- `ui/src/layout/defaults.ts` `PROFILES` is the prefill the feedback complains about:
  `wide = [nav, list, list, inspector]`, `ultra = [nav, list, list, inspector, tool:overview]`, and
  `ULTRA_PLUS_SLOT = tool:flux` is appended at ≥ 3840 px.
- `visibleSlots` in `ui/src/layout/width-class.ts` gives 1/2/3/3/4/5 (+1 for ultra-plus) and is
  documented as monotonic. Nothing in this feature touches it.
- `assignPanes` (`ui/src/store/index.ts`) maps each `SlotConfig` to a pane, reusing an existing pane
  of the same kind/tool by id prefix. Panes therefore survive a profile change already.
- `navigate(section)` retargets the focused (or first) **list** pane and clears its selection,
  focused row, sort and scroll. It does not open anything else — so below wide width there is no
  auto-open today, and there must not be one after this change.
- `SlotGrid.tsx` never remounts the row: a class change applies sizes through the group's imperative
  `setLayout` (the feedback #2 fix). This is exactly the property FR-010 needs.
- `resolveProfile(cls, saved, ultraPlus)` validates a saved profile, truncates it to `visibleSlots`
  and renormalises sizes; an invalid profile falls back to the default. This is the natural place for
  the saved-layout migration.

## R1 — How to represent a cascade column

**Decision**: reuse `kind: 'inspector'` for every cascade slot; a cascade slot with nothing open is
an inspector pane with no target. A new UI-only `layout.cascade: PaneId[]` holds the chain, and the
slot list is derived from it.

**Rationale**: `SlotConfig.kind` is generated from `src/protocol/v1`, so a new `cascade` kind means a
protocol change, regenerated bindings, round-trip tests and a version bump — for a distinction the
operator never sees. Inspector panes already render a resource, already keep their own tab, selection
and scroll, and `assignPanes` already reuses them by prefix.

**Alternatives considered**: a new `cascade` slot kind (rejected: protocol churn for no user-visible
gain, and old builds would reject the saved file); leaving the `tool` slots in place but empty
(rejected: a tool slot is bound to one tool, so it cannot hold a drill-down step).

## R2 — What "most relevant item" means

**Decision**: `mostRelevantRow(descriptor, rows)` returns the first row whose status tone is `err`,
else the first whose tone is `warn`, else `rows[0]`. The tone comes from the descriptor's column with
`status: true` (`ColumnDef.status`, `ui/src/resources/types.ts`), passed through `statusTone` from
`ui/src/resources/search.ts` — the same classifier list rows, badges and the topology graph use.

**Rationale**: the spec's rule is "first not-healthy item, else the first item". `statusTone` already
maps every kind's status text to `ok | warn | err | muted`, so there is no second notion of health to
maintain (the comment at the top of `ui/src/topology/status.ts` warns against exactly that). Sections
whose descriptor has no `status` column fall back to the first row, which is the spec's fallback.

**Alternatives considered**: a per-descriptor `relevance()` hook (rejected: ~40 descriptors to keep
in sync); reusing the linter or the Overview's unhealthy counts (rejected: both are section-specific
and one of them is asynchronous).

## R3 — Cascade algebra and the shift-left rule

**Decision**: a pure `ui/src/layout/cascade.ts` owning the chain:

- `resetTo(sectionPaneId)` — a section click: the chain is just the section.
- `pushStep(cascade, paneId)` — a drill-in appends.
- `truncateAt(cascade, index)` — clicking a row in an earlier column drops everything to its right.
- `slotsFor(cascade, slotCount)` — the visible window: `[nav, section, ...chain.slice(-(slotCount - 2))]`.

Shift-left is therefore not a mutation: the chain keeps growing and `slotsFor` shows its tail, which
is what makes FR-006 and FR-010 the same code path.

**Rationale**: keeping the full chain means widening the window re-reveals earlier steps instead of
having lost them, which the spec's edge cases require. Dropping steps on overflow would make the
width-class round trip lossy.

**Alternatives considered**: mutating a fixed-length array of slot targets (rejected: loses the
history, so growing the window cannot restore it).

## R4 — When the auto-open fires

**Decision**: `navigate` arms a pending auto-open `{paneId, route}` on the section's list pane at
wide and above. An effect beside the list view clears the arm and opens the chosen row once the
pane's list state has its first page. Any operator action (clicking a row, changing section, changing
context) clears the arm first.

**Rationale**: SC-003 says the list must not wait for the auto-open, and the edge case forbids
flashing the wrong item, so the choice has to happen after the first page and before anything is
rendered in column 3. Arming rather than polling keeps it to one O(n) scan per section open.

**Alternatives considered**: choosing inside the list reducer on every list update (rejected: an
O(n) scan per stream batch on a 10,000-row list, against Principle V); auto-opening from the
navigator click with a guessed key (rejected: the rows are not loaded yet).

## R5 — Migrating saved layouts without a schema bump

**Decision**: `resolveProfile` gains a normalisation pass for `wide` and `ultra`: index 0 stays
`nav`, index 1 stays `list`, and every slot from index 2 becomes a cascade slot, keeping its
`sizePct`. `UiConfig.schema` stays `1`.

**Rationale**: `src/protocol/v1/ui_config.rs` validates `schema` against a single supported value, so
bumping it is a protocol change and would make the backend reject files written by the new UI on an
older build. Normalising by shape needs neither. Sizes are kept, which is FR-013; the file itself is
only rewritten when the operator next drags a divider, which is Principle VI.

**Alternatives considered**: a `schema: 2` migration (rejected as above); ignoring saved wide/ultra
profiles entirely (rejected: throws away the operator's column widths).

## R6 — Keeping state across width-class changes

**Decision**: nothing new. The cascade holds pane ids, `assignPanes` reuses panes by kind and id
prefix, and `SlotGrid` applies sizes without remounting. Columns are gained and lost from the tail of
the chain only.

**Rationale**: this is the property feedback #2 already bought; the tests for FR-010 assert it rather
than new code implementing it.

## R7 — Which classes change

**Decision**: the visible change starts at `wide`. `phone`, `tablet`, `laptop` and `desktop` keep
their profiles, their behaviour and their existing tests; the auto-open is gated to `wide` and above.

**Rationale**: feedback #8 and the keep-as-is list both ask for this, and `desktop` has only three
slots, so a cascade there would be today's inspector under another name.

## Open questions

None. The one decision that could have gone either way — what "most relevant" means — was settled by
the user on 2026-09-27 and is recorded in `docs/FEEDBACK_2026-09-27.md` under "Group 3".
