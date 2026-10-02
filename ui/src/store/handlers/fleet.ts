/** 027-fleet-rollup-dashboard: `SystemEvent::FleetSummaryUpdated` handling; 046-fleet-model:
 * `FleetModelUpdated`, `FleetModelRejected`, `FleetModelLoadFailed`, `FleetGroupRollupUpdated`,
 * `SharedFleetDefinitionsUpdated` and `FleetStoreUnavailable`. */
import type {
  AdoptedFleetDefinition,
  FleetContextSummary,
  FleetGroup,
  FleetGroupRollup,
  FleetMember,
  SharedFleetDefinition,
} from '@/protocol/types'

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
  // The file was read this time, so the last read failure is no longer the truth (FR-007).
  session.fleetLoadFailure = null
}

/** 046 FR-003/FR-006: the backend's refusal, held verbatim. The UI never composes its own
 * message and never validates a label itself (Principle II). */
export function applyFleetRejection(
  d: Draft,
  data: { reason: string; field: string | null },
): void {
  d.edit('session').fleetRejection = { reason: data.reason, field: data.field }
}

/**
 * 046-fleet-model US2 (FR-010): one group's rollup, stored whole and keyed by group name. The
 * coverage fields travel with the sums because the type cannot express a sum without them — the
 * UI stores what arrived and derives no total of its own.
 */
export function applyFleetGroupRollup(d: Draft, rollup: FleetGroupRollup): void {
  const session = d.edit('session')
  session.fleetRollups = { ...session.fleetRollups, [rollup.group]: rollup }
}

/**
 * 046-fleet-model US3 (FR-012/FR-014): the shared store's list and this machine's adopted copies,
 * stored as they arrived. Rows with `still_shared: false` are kept — the backend includes them on
 * purpose and dropping one here would delete an operator's adopted definition from the screen while
 * it still exists on disk.
 *
 * A successful read also clears the last store error: the store answered.
 */
export function applySharedFleetDefinitions(
  d: Draft,
  data: { definitions: SharedFleetDefinition[]; adopted: AdoptedFleetDefinition[] },
): void {
  const session = d.edit('session')
  session.fleetDefinitions = data.definitions
  session.fleetAdopted = data.adopted
  session.fleetStoreError = null
}

/**
 * 046 FR-014: the store could not be read. The adopted copies are **left exactly where they are** —
 * only the reason is recorded, and the UI renders "no longer following", never "gone".
 */
export function applyFleetStoreUnavailable(d: Draft, data: { reason: string }): void {
  d.edit('session').fleetStoreError = data.reason
}

/**
 * 046 FR-007: `fleet.json` could not be read, so the backend fell back to defaults **without
 * touching the file**. The path and the loader's verbatim reason are held so the view can say the
 * file could not be read, rather than letting an empty-looking fleet imply the operator's data was
 * discarded. Cleared by the next model the backend manages to report.
 */
export function applyFleetLoadFailure(d: Draft, data: { path: string; reason: string }): void {
  d.edit('session').fleetLoadFailure = { path: data.path, reason: data.reason }
}
