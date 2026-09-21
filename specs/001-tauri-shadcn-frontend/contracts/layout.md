# Contract: Width Classes, Pane Slots, Column Budgets

**Plan**: [../plan.md](../plan.md) · **Decision**: research R5 · **Spec**: FR-006–FR-012, SC-002

## Width classes

The class is computed from the **app viewport's logical width**, in CSS px after OS scaling.

| Class | Min width | Visible slots | Default arrangement |
|---|---|---|---|
| `phone` | 0 | 1 | One pane at a time, from a stack: nav → list → inspector/tool. A bottom tab bar holds Resources, Logs, Forwards, and More. |
| `tablet` | 640 | 2 | [list \| inspector]. The nav is a drawer. |
| `laptop` | 1024 | 3 | [nav rail (icons + tooltips) \| list \| inspector] |
| `desktop` | 1440 | 3 + dock | [nav tree \| list \| inspector], plus a collapsible bottom dock for logs/events. |
| `wide` | 1920 | 4 + dock | [nav tree \| list A \| list B (pinned) \| inspector], plus the dock. |
| `ultra` | 2560 | 5–6 + dock | [nav tree \| list A \| list B \| inspector \| metrics/overview column], plus the dock (logs + events side by side). At ≥ 3840 px a 6th slot holds another pinned list or the Flux dashboard. |

**Invariant (FR-006)**: `visibleSlots(class)` never goes down as the class goes up. A unit test asserts this for every class and for the ≥3840 sub-step.

## Panes and slots

- A **Pane** is `{ id, kind: "nav" | "list" | "inspector" | "tool", route?, pinned: boolean }`.
- A **Slot** holds at most one pane. When the class shrinks, panes that no longer fit move onto the phone stack, or into tabs at tablet width. They are **not destroyed**, so selection, search text, and scroll are preserved (FR-013, US2-5).
- Selecting a row targets the nearest inspector slot. If there isn't one (phone), the inspector is pushed onto the stack, and Back pops it (US2-1).
- On `wide` and `ultra`, users can pin a list pane, drag it between list slots, and resize slots through the resizable handles. The result is saved as a **LayoutProfile** for that class through `SaveUiConfig` (FR-011, US2-6).

## Column budgets (per pane, not per viewport)

- Each column in a resource descriptor declares `priority: 1..5` and `minWidth` (px). Priority 1 is always shown. Name and status are always priority 1.
- A pane measures its own content width with a ResizeObserver. Columns are added in ascending priority while the sum of `minWidth` fits.
- If the last column to fit is priority N, every priority ≤ N column is shown, and **no higher-priority column is ever hidden while a lower-priority one shows**.
- Columns are never squeezed below `minWidth`. Hidden columns stay available in the inspector (FR-009).
- The table body scrolls vertically and never horizontally. Cells truncate with an ellipsis and show the full value on hover and on long-press (FR-010, FR-012).

## Density

| Class | Row height | Touch target | Base font |
|---|---|---|---|
| phone, tablet | 44 px | ≥ 44 px | 15 px |
| laptop, desktop | 28 px | — | 13 px |
| wide, ultra | 24 px | — | 13 px |

## Test hooks

- `data-width-class` on `<html>`, and `data-slot` / `data-pane-kind` on pane hosts, are used by Playwright.
- Playwright projects:
  - `phone` 390×844
  - `laptop` 1280×800
  - `wide` 2560×1440
  - `ultra` 5120×1440
- Every project asserts `document.scrollingElement.scrollWidth <= innerWidth`.
