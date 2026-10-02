/**
 * 046-fleet-model US2 (T034): what the fleet view shows, decided in one place.
 *
 * Pure functions, no I/O: which members a label filter keeps, how many of how many that is
 * (FR-016), which members a group holds, which members no group holds (FR-017), and how a
 * rollup's coverage reads (FR-010). Every number a caller can render comes with the total it was
 * taken from — there is no exported shape here that states a count without its denominator.
 *
 * The selector *grammar* is the backend's (Principle II): `src/backend/fleet_model.rs` parses an
 * expression, refuses an invalid one with its own message, and is the only validator. What this
 * module does is the membership join the protocol does not yet carry — `FleetModelUpdated` sends
 * members and groups but no mapping between them (`src/protocol/v1/fleet.rs`: `FleetMember` has no
 * `groups` field). `matchesSelector` below therefore evaluates only expressions the backend has
 * already accepted, and it must be deleted the moment the event carries membership.
 */
import type { FleetCoverageGap, FleetGapReason, FleetGroup, FleetMember } from '@/protocol/types'

/** A group and the members it currently holds. Empty is a legitimate state (FR-005). */
export interface FleetGroupView {
  group: FleetGroup
  members: FleetMember[]
  /**
   * `false` when the stored expression could not be read here. A group whose selector no longer
   * parses is **not** a group that matches nothing: the caller must say which it is, or it repeats
   * the `nav.ts:64-69` mistake of reporting "nothing found" where the truth is "I could not look".
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

/** One parsed requirement of the `kubectl -l` subset the backend documents. */
type Requirement =
  | { op: 'eq' | 'ne'; key: string; value: string }
  | { op: 'in' | 'notin'; key: string; values: string[] }
  | { op: 'exists' | 'missing'; key: string }

/** Split on commas that are not inside a `(...)` value set. */
function splitRequirements(selector: string): string[] {
  const out: string[] = []
  let depth = 0
  let start = 0
  for (let i = 0; i < selector.length; i += 1) {
    const ch = selector[i]
    if (ch === '(') depth += 1
    else if (ch === ')') depth = Math.max(0, depth - 1)
    else if (ch === ',' && depth === 0) {
      out.push(selector.slice(start, i))
      start = i + 1
    }
  }
  out.push(selector.slice(start))
  return out.map((part) => part.trim()).filter((part) => part.length > 0)
}

const SET_RE = /^([^!=<>\s]+)\s+(in|notin)\s*\((.*)\)$/s

/**
 * A key or value the backend's validator would have accepted: no whitespace, and none of the
 * characters the grammar reserves. This is a shape check, not a second validator — the backend
 * refuses an invalid label or selector at entry (FR-003, FR-006), so anything failing here is an
 * expression this module cannot read rather than one it may interpret.
 */
const LEGAL = /^[^\s=!,()]*$/

function parseRequirement(raw: string): Requirement | null {
  const set = SET_RE.exec(raw)
  if (set) {
    const values = (set[3] ?? '')
      .split(',')
      .map((v) => v.trim())
      .filter((v) => v.length > 0)
    const key = (set[1] ?? '').trim()
    if (key.length === 0 || !LEGAL.test(key) || values.some((v) => !LEGAL.test(v))) return null
    return { op: set[2] === 'in' ? 'in' : 'notin', key, values }
  }
  const equality = (op: 'eq' | 'ne', at: number, width: number): Requirement | null => {
    const key = raw.slice(0, at).trim()
    const value = raw.slice(at + width).trim()
    if (key.length === 0 || !LEGAL.test(key) || !LEGAL.test(value)) return null
    return { op, key, value }
  }
  const ne = raw.indexOf('!=')
  if (ne > 0) return equality('ne', ne, 2)
  const double = raw.indexOf('==')
  if (double > 0) return equality('eq', double, 2)
  const eq = raw.indexOf('=')
  if (eq > 0) return equality('eq', eq, 1)
  if (raw.startsWith('!')) {
    const key = raw.slice(1).trim()
    return key.length === 0 || !LEGAL.test(key) ? null : { op: 'missing', key }
  }
  const key = raw.trim()
  return LEGAL.test(key) && key.length > 0 ? { op: 'exists', key } : null
}

function satisfies(req: Requirement, labels: Record<string, string>): boolean {
  const value = Object.prototype.hasOwnProperty.call(labels, req.key) ? labels[req.key] : undefined
  switch (req.op) {
    case 'eq':
      return value === req.value
    case 'ne':
      return value !== req.value
    case 'in':
      return value !== undefined && req.values.includes(value)
    case 'notin':
      return value === undefined || !req.values.includes(value)
    case 'exists':
      return value !== undefined
    case 'missing':
      return value === undefined
  }
}

/** The requirements of an expression, or `null` when this module cannot read it. */
function requirementsOf(selector: string): Requirement[] | null {
  const parsed = splitRequirements(selector).map(parseRequirement)
  return parsed.some((r) => r === null) ? null : (parsed as Requirement[])
}

/**
 * Can this expression be read here at all? The backend's parser is the authority — it refuses an
 * invalid selector at entry — so a stored selector that cannot be read now is a state to report,
 * never one to silently resolve as "matches nothing".
 */
export function selectorReadable(selector: string): boolean {
  return requirementsOf(selector) !== null
}

/**
 * Does a selector the backend already accepted match these labels? An unreadable expression
 * matches nothing *and* is reported as unreadable by `FleetGroupView.readable`, so the UI never
 * guesses a membership the backend did not resolve.
 */
export function matchesSelector(selector: string, labels: Record<string, string>): boolean {
  const reqs = requirementsOf(selector)
  if (reqs === null) return false
  return reqs.every((r) => satisfies(r, labels))
}

/** The members a group holds: every member whose labels its selector matches (FR-004). */
export function membersOfGroup(members: readonly FleetMember[], group: FleetGroup): FleetMember[] {
  return members.filter((m) => matchesSelector(group.selector, m.labels))
}

/** Is this member held by any group? */
export function isGrouped(member: FleetMember, groups: readonly FleetGroup[]): boolean {
  return groups.some((g) => matchesSelector(g.selector, member.labels))
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
      readable: selectorReadable(group.selector),
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
