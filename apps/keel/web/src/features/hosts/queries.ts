/**
 * Typed host queries (spec 067 US5, T043): the pure part of the inspector's Queries panel. Which
 * parameter each kind takes, the same checks Keel and the runner make on it, how an outcome reads,
 * and merging an answered query into the list.
 */
import type { HostQuery, QueryKind, QueryOutcome } from '@/protocol/wire'

/** Every kind, in the order the picker offers them. */
export const KINDS: readonly QueryKind[] = ['unit_state', 'path_free', 'disk_health', 'facts', 'applied_revision']

export const KIND_LABEL: Record<QueryKind, string> = {
  facts: 'Facts',
  applied_revision: 'Applied revision',
  unit_state: 'Unit state',
  disk_health: 'Disk health',
  path_free: 'Free space',
}

/** The one parameter a kind takes, if any. */
export function paramOf(kind: QueryKind): 'unit' | 'path' | null {
  if (kind === 'unit_state') return 'unit'
  if (kind === 'path_free') return 'path'
  return null
}

/** A systemd unit name as the runner accepts it. */
export function validUnit(unit: string): boolean {
  return /^[A-Za-z0-9@._:-]{1,256}$/.test(unit)
}

/** An absolute, normalised path: no `.`, `..` or empty component, no trailing `/`, ≤ 4096 bytes. */
export function validPath(path: string): boolean {
  if (!path.startsWith('/') || path.includes('\0') || new TextEncoder().encode(path).length > 4096) return false
  if (path === '/') return true
  return path
    .slice(1)
    .split('/')
    .every((c) => c !== '' && c !== '.' && c !== '..')
}

/** Why the parameter can't be sent as typed, or `null` when it can (or the kind takes none). */
export function paramProblem(kind: QueryKind, value: string): string | null {
  const param = paramOf(kind)
  if (param === null) return null
  if (value === '') return param === 'unit' ? 'Name a unit, like nginx.service.' : 'Name a path, like /var/lib.'
  if (param === 'unit' && !validUnit(value)) return 'Letters, digits and @ . _ : - only.'
  if (param === 'path' && !validPath(value)) return 'An absolute path without . or .. parts or a trailing /.'
  return null
}

/** The `params` map `AskHost` sends. */
export function paramsFor(kind: QueryKind, value: string): Record<string, string> {
  const param = paramOf(kind)
  return param === null ? {} : { [param]: value }
}

const REFUSAL: Record<string, string> = {
  not_allowed: 'not allowed by the host’s queries.toml',
  bad_params: 'bad parameter',
  unavailable: 'the host could not answer',
  too_large: 'answer too large',
  busy: 'host busy, try again',
  invalid_answer: 'the runner answered something invalid',
}

const OUTCOME: Record<QueryOutcome, string> = {
  answered: 'answered',
  refused: 'refused',
  timeout: 'no answer in time',
  no_capability: 'runner too old for queries',
  disconnected: 'host not connected',
}

/** How a query's outcome reads; `null` outcome is still waiting. */
export function outcomeText(q: Pick<HostQuery, 'outcome' | 'refusal'>): string {
  if (q.outcome === null) return 'waiting'
  if (q.outcome === 'refused' && q.refusal !== null) return `refused: ${REFUSAL[q.refusal] ?? q.refusal}`
  return OUTCOME[q.outcome]
}

export type OutcomeTone = 'ok' | 'warn' | 'err' | 'quiet'

export function outcomeTone(outcome: QueryOutcome | null): OutcomeTone {
  if (outcome === null) return 'quiet'
  if (outcome === 'answered') return 'ok'
  if (outcome === 'refused') return 'err'
  return 'warn'
}

/** What was asked: the kind and its parameter. */
export function askedText(q: Pick<HostQuery, 'kind' | 'params'>): string {
  const param = paramOf(q.kind)
  const params = q.params as Record<string, unknown> | null
  const value = param !== null && params !== null && typeof params[param] === 'string' ? params[param] : null
  return value === null ? KIND_LABEL[q.kind] : `${KIND_LABEL[q.kind]} ${value}`
}

/** `q` merged into `items` (newest first, by `asked_at` then id), replacing the same query. */
export function upsertQuery(items: readonly HostQuery[], q: HostQuery): HostQuery[] {
  const rest = items.filter((i) => i.id !== q.id)
  return [q, ...rest].sort((a, b) => {
    const d = Date.parse(b.asked_at) - Date.parse(a.asked_at)
    if (d !== 0) return d
    return a.id < b.id ? 1 : a.id > b.id ? -1 : 0
  })
}

/** The answer as indented JSON, for the expanded row. */
export function answerText(answer: unknown): string {
  return JSON.stringify(answer, null, 2)
}
