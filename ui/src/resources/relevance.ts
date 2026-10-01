/**
 * Relevance order (044, FR-003): which row a section's auto-open should show. Pure — the caller
 * passes the rows in the order the operator sees them, so their sort and filter decide.
 */
import { statusTone } from './search'
import type { ResourceDescriptor, RowKey } from './types'

/** Everything the rule needs from a descriptor, so any kind's descriptor is accepted as is. */
type Relevant<R> = Pick<ResourceDescriptor<R>, 'columns' | 'rowKey'>

/**
 * The first row whose status tone is `err`, else the first whose tone is `warn`, else the first
 * row; `undefined` for an empty list. `statusTone` is the only notion of health in the app, so a
 * kind with no `status` column simply counts every row as healthy and the first row wins.
 *
 * One O(n) pass, run once per section open and never on the render path (SC-003).
 */
export function mostRelevantRow<R>(
  descriptor: Relevant<R>,
  rows: readonly R[],
): RowKey | undefined {
  const first = rows[0]
  if (first === undefined) return undefined
  const status = descriptor.columns.find((c) => c.status === true)
  if (!status) return descriptor.rowKey(first)
  let warn: R | undefined
  for (const row of rows) {
    const { tone } = statusTone(status.text(row))
    if (tone === 'err') return descriptor.rowKey(row)
    if (tone === 'warn' && warn === undefined) warn = row
  }
  return descriptor.rowKey(warn ?? first)
}
