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
import type {
  AdoptedFleetDefinition,
  FleetCoverageGap,
  FleetGapReason,
  FleetGroup,
  FleetMember,
  FleetMemberShare,
  SettingOrigin,
  SharedFleetDefinition,
} from '@/protocol/types'

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

/**
 * 046-fleet-model US3 (T043): one shared definition as the sharing panel renders it.
 *
 * A row exists for every definition the store reported **and** for every definition this machine
 * adopted, even one the store no longer lists. Dropping an adopted row because the store could not
 * be read is the failure FR-014 exists to forbid: the copy is local, it still works, and only the
 * operator removes it.
 */
export interface SharedDefinitionRow {
  definition: SharedFleetDefinition
  /** Is there a local adopted copy of this definition? */
  adopted: boolean
  /** RFC 3339, from the adopted row; `null` when nothing was adopted. */
  adoptedAt: string | null
  /** Is the adopted copy still following the shared one? */
  following: boolean
  /** Why it is not following — the three causes read differently, so they stay three. */
  cause: 'following' | 'removedUpstream' | 'storeUnreachable' | 'notFollowing'
}

/**
 * The rows to render: the store's list in order, then anything adopted that the store did not
 * list. An adopted copy is preferred as the rendering source for a definition the store still
 * lists, because the adopted copy is what this machine actually holds.
 */
export function sharedDefinitionRows(
  definitions: readonly SharedFleetDefinition[],
  adopted: readonly AdoptedFleetDefinition[],
  storeUnreachable = false,
): SharedDefinitionRow[] {
  const byId = new Map<string, AdoptedFleetDefinition>()
  for (const a of adopted) byId.set(a.definition.id, a)

  const row = (definition: SharedFleetDefinition, held?: AdoptedFleetDefinition) => {
    const cause: SharedDefinitionRow['cause'] = definition.removed
      ? 'removedUpstream'
      : storeUnreachable
        ? 'storeUnreachable'
        : held !== undefined && !held.still_shared
          ? 'notFollowing'
          : 'following'
    return {
      definition,
      adopted: held !== undefined,
      adoptedAt: held?.adopted_at ?? null,
      following: cause === 'following',
      cause,
    }
  }

  const listed = definitions.map((d) => {
    const held = byId.get(d.id)
    // `removed` is the store's own flag, so the store's copy decides that; the rest renders from
    // whichever copy is at hand.
    return row(held === undefined ? d : { ...held.definition, removed: d.removed }, held)
  })
  const seen = new Set(definitions.map((d) => d.id))
  const orphans = adopted.filter((a) => !seen.has(a.definition.id)).map((a) => row(a.definition, a))
  return [...listed, ...orphans]
}

/**
 * Why a row is no longer following, in words that say what is true: the copy is **here**. None of
 * these sentences may suggest the adopted definition was removed, reset or lost (FR-014,
 * Constitution Principle VI).
 */
export function notFollowingReason(row: SharedDefinitionRow): string {
  switch (row.cause) {
    case 'removedUpstream':
      return (
        'No longer shared: whoever published it took it out of the store. ' +
        'Your adopted copy is kept here and still in use — it goes away only when you say so.'
      )
    case 'storeUnreachable':
      return (
        'No longer following the shared copy: the store could not be reached. ' +
        'Your adopted copy is kept here and still in use.'
      )
    case 'notFollowing':
      return 'No longer following the shared copy. Your adopted copy is kept here and still in use.'
    case 'following':
      return ''
  }
}

/** One value, where it came from, and — when it is a local override — what it overrode (FR-013). */
export interface FleetValueOrigin {
  /** `display_name` or `labels/<key>`, as `FleetMember.origins` keys it. */
  field: string
  /** What to call the field on screen. */
  label: string
  /** The value in force — the local one, when there is one. */
  value: string
  origin: SettingOrigin
  /** The shared value this one overrides, when it differs; `null` when there is nothing behind it. */
  shared: string | null
}

/**
 * The shared slice for one context, as the adopted definitions describe it. Later adoptions win on
 * a key they both carry. A definition that is no longer followed still answers here — it is the
 * origin a local value overrode, and FR-013 says that stays visible.
 */
export function sharedShareFor(
  context: string,
  adopted: readonly AdoptedFleetDefinition[],
): FleetMemberShare | null {
  let display_name: string | null = null
  const labels: Record<string, string> = {}
  let found = false
  for (const a of adopted) {
    const share = a.definition.members[context]
    if (share === undefined) continue
    found = true
    if (share.display_name != null) display_name = share.display_name
    Object.assign(labels, share.labels)
  }
  return found ? { display_name, labels } : null
}

/**
 * Per-value provenance for one member: the local winner, marked local, beside the shared value it
 * overrode (FR-013).
 *
 * A field appears when the backend reported an origin for it or when a shared value exists for it;
 * a purely local fleet that nobody shares anything with therefore says nothing about origins at
 * all, which is the honest rendering of "there is no shared copy to compare against".
 */
export function valueOrigins(
  member: FleetMember,
  shared: FleetMemberShare | null,
): FleetValueOrigin[] {
  const rows: FleetValueOrigin[] = []
  const at = (field: string, label: string, value: string, sharedValue: string | null) => {
    const origin = member.origins[field]
    if (origin === undefined && sharedValue === null) return
    const resolved: SettingOrigin = origin ?? 'local'
    rows.push({
      field,
      label,
      value,
      origin: resolved,
      shared:
        resolved === 'local' && sharedValue !== null && sharedValue !== value ? sharedValue : null,
    })
  }

  if (member.display_name !== null) {
    at('display_name', 'Display name', member.display_name, shared?.display_name ?? null)
  }
  for (const [k, v] of Object.entries(member.labels)) {
    at(`labels/${k}`, k, v, shared?.labels[k] ?? null)
  }
  return rows
}
