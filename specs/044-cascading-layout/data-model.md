# Phase 1 data model: cascading wide-screen layout

All of it is UI state. No protocol type changes, no persisted field added.

## Cascade (new, in `layout`)

```ts
/** The chain the operator opened, from the section list outward. Never persisted. */
cascade: PaneId[]
```

- `cascade[0]` is always the section list pane (the pane `navigate` retargets).
- `cascade[1…]` are drill-down steps, oldest first, one per operator click.
- Invariants:
  - every id exists in `panes`;
  - ids are unique (following a link back to an open item truncates rather than duplicating);
  - at wide and above, `slotPanes = [navPaneId, ...cascade.slice(-(slotCount - 1))]`;
  - below wide, `cascade` is ignored and `slotPanes` is derived exactly as it is today.
- Lifecycle: reset by a section click, a context change and a kubeconfig reload; truncated by
  selecting a row in a column that is not the last; appended by a drill-in; unchanged by a
  width-class change.

## Cascade step (per pane, existing fields)

A step is an ordinary pane. It already carries everything the constitution requires to survive a
layout change, and this feature adds no field to it:

| Field | Meaning here |
|---|---|
| `kind` | `'list'` for `cascade[0]`, `'inspector'` for the steps |
| `route` | section, context, scope, namespace — the section for `cascade[0]` |
| `target` | the object the step shows (absent = an empty cascade column) |
| `selection` / `focusedRow` | the row this step drilled in from |
| `inspectorTab`, `scrollTop`, `sort` | preserved across width-class changes as today |

An empty column is simply an inspector pane with no `target`. A step whose object no longer exists
keeps its `target` and renders the "no longer exists" state (FR-014) rather than being cleared.

## Pending auto-open (new, transient)

```ts
/** Armed by `navigate` at wide and above; consumed once the list's first page is in. */
pendingAutoOpen?: { paneId: PaneId; route: Route }
```

Cleared by: consuming it, any row click, a section change, a context change, dropping below wide
width. It is never persisted and never survives a reload.

## Slot profile (existing type, new default values)

`SlotConfig` and `LayoutProfile` are unchanged. The default `PROFILES` values change:

| Class | Before | After |
|---|---|---|
| `wide` | nav, list, list, inspector | nav, list, inspector, inspector |
| `ultra` | nav, list, list, inspector, tool:overview | nav, list, inspector, inspector, inspector |
| `ultra-plus` (6th) | tool:flux | inspector |

Slot counts are identical, so `visibleSlots` and Principle IV are untouched. `phone`, `tablet`,
`laptop` and `desktop` are byte-for-byte unchanged.

## Saved profile normalisation

`resolveProfile(cls, saved, ultraPlus)` keeps its current contract (validate, truncate to
`visibleSlots`, renormalise sizes, filter the dock) and adds one pass for `wide` and `ultra`:

- index 0 → `nav`, index 1 → `list`, index ≥ 2 → cascade slot (`inspector`, no tool);
- `sizePct` is taken from the saved slot at that index;
- an invalid saved profile still falls back to the default, as today.

## Relevance order

Pure, no state: `mostRelevantRow(descriptor, rows)` → `RowKey | undefined`.

- `undefined` when `rows` is empty.
- Otherwise the first row whose status tone is `err`, else the first whose tone is `warn`, else
  `rows[0]`.
- Tone: `statusTone(statusColumn.text(row)).tone`, where `statusColumn` is the descriptor's column
  with `status: true`; with no such column every row counts as `ok`, so the first row wins.
- The row order is the list's current order (the operator's sort and filters included), so what
  opens is always the first row they can see.
