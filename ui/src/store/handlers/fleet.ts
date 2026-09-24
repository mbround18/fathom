/** 027-fleet-rollup-dashboard: `SystemEvent::FleetSummaryUpdated` handling. */
import type { FleetContextSummary } from '@/protocol/types'

import type { Draft } from '../draft'

/** Last write per context wins; other contexts' entries are untouched. */
export function applyFleetSummary(d: Draft, summary: FleetContextSummary): void {
  const session = d.edit('session')
  session.fleetSummaries = { ...session.fleetSummaries, [summary.context]: summary }
}
