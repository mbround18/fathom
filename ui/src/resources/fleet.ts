/**
 * 046-fleet-model US2 (T034): what the fleet view shows, decided in one place.
 *
 * Pure functions, no I/O: which members a label filter keeps, how many of how many that is
 * (FR-016), which members a group holds, which members no group holds (FR-017), and how a
 * rollup's coverage reads (FR-010). Every number a caller can render comes with the total it was
 * taken from — there is no exported shape here that states a count without its denominator.
 *
 * The selector grammar is the backend's and *only* the backend's (Principle II):
 * `src/backend/fleet_model.rs` parses an expression, refuses an invalid one with its own message,
 * and resolves membership. `FleetModelUpdated` now carries the answer — `FleetMember.groups` names
 * every group a member is in, and `FleetGroup.selector_error` holds the parser's verbatim words
 * when a *stored* selector no longer parses. This module reads those two fields. It parses
 * nothing: an evaluator here could disagree with the one that writes `fleet.json`, which is
 * exactly the drift Principle II forbids (T052 deleted the one that had grown).
 */
import type { FleetCoverageGap, FleetGapReason, FleetGroup, FleetMember } from '@/protocol/types'

/** A group and the members it currently holds. Empty is a legitimate state (FR-005). */
export interface FleetGroupView {
  group: FleetGroup
  members: FleetMember[]
  /**
   * `false` when the backend could not read the group's stored expression — `selector_error` is
   * set. A group whose selector no longer parses is **not** a group that matches nothing: the
   * caller must say which it is, or it repeats the `nav.ts:64-69` mistake of reporting "nothing
   * found" where the truth is "I could not look".
   */
  readable: boolean
}

/** The whole view: groups in definition order, then whatever no group matched (FR-017). */
export interface FleetPartition {
  groups: FleetGroupView[]
  ungrouped: FleetMember[]
  /** Members the filter kept. */
  shown: number
  /** Members the fleet has, filter or no filter. */
  total: number
}

/**
 * The members a group holds, as the backend resolved it (FR-004).
 *
 * The membership is read from `member.groups`, never recomputed from the expression.
 */
export function membersOfGroup(members: readonly FleetMember[], group: FleetGroup): FleetMember[] {
  return members.filter((m) => m.groups.includes(group.name))
}

/** Is this member held by any of these groups? */
export function isGrouped(member: FleetMember, groups: readonly FleetGroup[]): boolean {
  return groups.some((g) => member.groups.includes(g.name))
}

/**
 * Does a member match the operator's filter text? Matching is over what identifies a member and
 * what groups it — context name, display name and `key=value` labels — by case-insensitive
 * substring, the precedent `resources/search.ts` sets for a typed filter over a list.
 */
export function matchesFilter(member: FleetMember, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (q.length === 0) return true
  if (member.context.toLowerCase().includes(q)) return true
  if ((member.display_name ?? '').toLowerCase().includes(q)) return true
  return Object.entries(member.labels).some(
    ([k, v]) =>
      `${k}=${v}`.toLowerCase().includes(q) ||
      k.toLowerCase().includes(q) ||
      v.toLowerCase().includes(q),
  )
}

/** The members a filter keeps, in the order the model listed them. */
export function filterMembers(members: readonly FleetMember[], query: string): FleetMember[] {
  return members.filter((m) => matchesFilter(m, query))
}

/**
 * The whole view in one pass: every group with the filtered members it holds — kept even when
 * that is none (FR-005) — the filtered members no group holds (FR-017), and the shown/total
 * counts a filter must state (FR-016). `total` counts the unfiltered fleet, so the denominator
 * never moves as the operator types.
 */
export function partitionFleet(
  members: readonly FleetMember[],
  groups: readonly FleetGroup[],
  query = '',
): FleetPartition {
  const shown = filterMembers(members, query)
  return {
    groups: groups.map((group) => ({
      group,
      members: membersOfGroup(shown, group),
      // The backend's verdict, not ours: an unparseable stored selector names its own reason.
      readable: group.selector_error === null,
    })),
    ungrouped: shown.filter((m) => !isGrouped(m, groups)),
    shown: shown.length,
    total: members.length,
  }
}

/** "Showing 2 of 3 clusters" — a filter always says how many of how many (FR-016). */
export function showingLabel(shown: number, total: number): string {
  return `Showing ${shown} of ${total} ${total === 1 ? 'cluster' : 'clusters'}`
}

/** "1 of 4 clusters" — a rollup's own coverage, never a bare sum (FR-010, SC-004). */
export function coverageLabel(covered: number, total: number): string {
  return `${covered} of ${total} ${total === 1 ? 'cluster' : 'clusters'}`
}

/** `complete` only when every member was read; anything else is `partial` and names its gaps. */
export function coverageState(covered: number, total: number): 'complete' | 'partial' {
  return covered >= total ? 'complete' : 'partial'
}

/**
 * How a gap reads. The four variants stay four (FR-011): a denied read is not an empty cluster,
 * an unreachable cluster is not a missing one, and a member nobody has polled yet is neither.
 */
export function gapHeadline(reason: FleetGapReason): string {
  switch (reason.kind) {
    case 'notPermitted':
      return 'not permitted'
    case 'unreachable':
      return 'unreachable'
    case 'absent':
      return 'absent from the kubeconfig'
    case 'pending':
      return 'not polled yet'
  }
}

/** The API server's own words, where it gave any. Never paraphrased. */
export function gapDetail(reason: FleetGapReason): string | null {
  return reason.kind === 'notPermitted' || reason.kind === 'unreachable' ? reason.detail : null
}

/** Gaps in a stable order — by context name — so a rollup renders the same way twice. */
export function orderedGaps(gaps: readonly FleetCoverageGap[]): FleetCoverageGap[] {
  return [...gaps].sort((a, b) => a.context.localeCompare(b.context))
}
