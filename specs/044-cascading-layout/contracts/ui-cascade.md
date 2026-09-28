# UI layout contract: Cascading wide-screen layout

No protocol changes. This contract fixes the observable UI surface that the E2E tests assert.

## Commands emitted (all existing)

| Trigger | Command |
|---|---|
| Section click (wide+) | the section's existing `refresh(route)` list command |
| Auto-open of the most relevant item | the same `read(ref)` commands a row click emits — nothing more |
| Drill-in to a cascade step | that kind's existing `read(ref)` / `RefreshRelatedResources` |
| Dragging a column divider | `SaveUiConfig` with the class's slot sizes, as today |

Nothing in this feature emits a mutating cluster command, and the auto-open emits no command a
manual click would not.

## Slots and test ids

| Element | Contract |
|---|---|
| Slot | `ResizablePanel` with `id="slot-<i>"`, `i` from 0; count = `visibleSlots(class, px)` as today |
| Slot 0 (wide+) | the navigator |
| Slot 1 (wide+) | the selected section's list; `aria-current="page"` on its navigator entry |
| Slots 2+ (wide+) | cascade columns, filled left to right |
| Empty cascade column | `data-testid="cascade-empty"`, with visible text "Follow a link to open the next level here" |
| Occupied cascade column | `data-testid="cascade-step"` plus `data-step="<n>"`, `n` from 1 at slot 2 |
| Auto-opened step | `data-auto-open="true"` on the slot-2 step until the operator clicks anywhere in the cascade |
| Missing item | `data-testid="cascade-gone"` with the object's namespace/name in its text |
| Laptop / desktop / tablet / phone | no `cascade-*` test id appears at these widths |

## Behaviour the tests assert

1. Fresh start at wide, ultra and ultra-plus: after one section click, exactly slots 0–2 are
   occupied and every remaining slot carries `cascade-empty` (FR-002, FR-003, SC-001).
2. With an `err` row present, the auto-opened step is that row; with only `warn` rows, the first
   `warn` row; with all rows `ok`, the first row (FR-003).
3. An empty section and a listless destination leave slot 2 as `cascade-empty` (FR-004).
4. Each drill-in occupies the next empty column; `data-step` increases by one (FR-005).
5. With every column occupied, one more drill-in keeps slots 0 and 1, shifts the steps left, and the
   deepest step is in the last slot (FR-006).
6. Clicking another row in slot 1 replaces slot 2 and returns later slots to `cascade-empty`;
   clicking another section returns the whole row to state 1 (FR-007).
7. Crossing every width boundary in both directions with a three-step cascade preserves each step's
   selection, search text and scroll position, and re-reveals hidden steps on the way back up
   (FR-010, SC-004).
8. A context change returns the row to state 1 for the new context, with no step showing the previous
   context's object (FR-015).
9. The dock's tabs, height and open state are unchanged by any of the above (FR-016).

## Persistence

- `layouts.wide` / `layouts.ultra` keep their shape and their sizes; slots from index 2 resolve to
  cascade slots regardless of what the file says they were (FR-011, SC-006).
- `UiConfig.schema` stays `1`. A profile written by this build still loads on the previous build,
  where it resolves to that build's slot kinds.
- The cascade itself is not persisted: a restart opens at state 1 for the remembered context.
