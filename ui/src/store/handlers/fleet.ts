/** 027-fleet-rollup-dashboard: `SystemEvent::FleetSummaryUpdated` handling; 046-fleet-model:
 * `FleetModelUpdated` and `FleetModelRejected`. */
import type { FleetContextSummary, FleetGroup, FleetMember } from '@/protocol/types'

import type { Draft } from '../draft'

/** Last write per context wins; other contexts' entries are untouched. */
export function applyFleetSummary(d: Draft, summary: FleetContextSummary): void {
  const session = d.edit('session')
  session.fleetSummaries = { ...session.fleetSummaries, [summary.context]: summary }
}

/** 046-fleet-model FR-002: the identity model, replaced wholesale on every `FleetModelUpdated`.
 * Keyed by context name, which is the only thing a member is keyed on. An accepted update also
 * clears the last rejection — the model the backend reports is now the truth. */
export function applyFleetModel(
  d: Draft,
  data: { members: FleetMember[]; groups: FleetGroup[] },
): void {
  const session = d.edit('session')
  const members: Record<string, FleetMember> = {}
  for (const m of data.members) members[m.context] = m
  session.fleetMembers = members
  session.fleetGroups = data.groups
  session.fleetRejection = null
}

/** 046 FR-003/FR-006: the backend's refusal, held verbatim. The UI never composes its own
 * message and never validates a label itself (Principle II). */
export function applyFleetRejection(
  d: Draft,
  data: { reason: string; field: string | null },
): void {
  d.edit('session').fleetRejection = { reason: data.reason, field: data.field }
}
