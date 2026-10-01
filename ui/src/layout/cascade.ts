/**
 * The cascade chain algebra (044, FR-005/FR-006/FR-007). Pure functions over pane ids: no React,
 * no store, so the store owns *which* panes exist and this owns the order they are shown in.
 *
 * The chain is `[sectionListPane, ...drillDownSteps]`, oldest first, and it is never shortened by
 * the window running out of room: `slotsFor` shows its tail, so shift-left (FR-006) and a
 * width-class change (FR-010) are the same code path and widening re-reveals earlier steps.
 */
import type { PaneId } from '@/store'

/** First slot a cascade step can occupy: 0 is the navigator, 1 the section list (FR-001). */
export const CASCADE_FIRST_SLOT = 2

/** A section click: the chain is just that section's list pane again (FR-007). */
export function resetTo(sectionPaneId: PaneId): PaneId[] {
  return [sectionPaneId]
}

/**
 * A drill-in appends. Following a link back to a step that is already open re-enters it instead of
 * opening a second column for it, which keeps the ids unique (data-model invariant).
 */
export function pushStep(cascade: readonly PaneId[], paneId: PaneId): PaneId[] {
  const at = cascade.indexOf(paneId)
  if (at >= 0) return cascade.slice(0, at + 1)
  return [...cascade, paneId]
}

/** Selecting a row in a column that is not the last drops every column to its right (FR-007). */
export function truncateAt(cascade: readonly PaneId[], index: number): PaneId[] {
  return cascade.slice(0, Math.max(index, 0) + 1)
}

/**
 * The panes to show in the `slotCount` visible slots: the navigator, the section list, then the
 * tail of the drill-in steps that fits. The navigator and the section list are pinned (FR-001), so
 * an overflowing chain scrolls only its steps (FR-006). `spare` — the free inspector panes the
 * chain does not use — pads the row so the remaining columns are the empty cascade placeholders
 * (FR-002, FR-012), in their slot order so a filling column never reshuffles the ones beside it.
 */
export function slotsFor(
  cascade: readonly PaneId[],
  navPaneId: PaneId,
  slotCount: number,
  spare: readonly PaneId[] = [],
): PaneId[] {
  if (slotCount <= 0) return []
  const slots = [navPaneId]
  if (slotCount === 1) return slots
  const [section, ...steps] = cascade
  if (section !== undefined) slots.push(section)
  if (slotCount > 2) slots.push(...steps.slice(-(slotCount - 2)))
  for (const id of spare) {
    if (slots.length >= slotCount) break
    if (!slots.includes(id)) slots.push(id)
  }
  return slots
}
