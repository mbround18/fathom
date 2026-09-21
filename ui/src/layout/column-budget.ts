/**
 * Per-pane column budget (contracts/layout.md § Column budgets): columns hide, never squash.
 *
 * Priority 1 always shows. The rest are taken in ascending priority (declaration order breaks
 * ties) while their `minWidth`s fit, stopping at the first that does not, so a lower-priority
 * column never shows while a higher-priority one is hidden. Returned in declaration order.
 */
export interface BudgetColumn {
  id: string
  priority: number
  minWidth: number
}

export function computeVisibleColumns(cols: readonly BudgetColumn[], width: number): string[] {
  const shown = new Set<string>()
  let used = 0
  for (const c of cols) {
    if (c.priority === 1) {
      shown.add(c.id)
      used += c.minWidth
    }
  }
  const rest = cols
    .map((c, i) => ({ c, i }))
    .filter(({ c }) => c.priority !== 1)
    .sort((a, b) => a.c.priority - b.c.priority || a.i - b.i)
  for (const { c } of rest) {
    if (used + c.minWidth > width) break
    shown.add(c.id)
    used += c.minWidth
  }
  return cols.filter((c) => shown.has(c.id)).map((c) => c.id)
}
