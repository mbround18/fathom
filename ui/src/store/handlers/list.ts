/** `BackendEvent::ResourceList` arm: list replacement with row-identity reuse (rule 3). */
import type { BackendEventEnvelope, PortForward, ResourceListEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { pendingPortForwardOriginKey } from '../index'
import type { ListRow, ListState } from '../index'
import { listKeyOf } from '../keys'

export function handleList(d: Draft, env: BackendEventEnvelope, event: ResourceListEvent): void {
  if (event.type === 'NamespacesUpdated') {
    d.edit('session').namespaces = event.data
    return
  }

  const key = listKeyOf(env.route)
  const prev = d.read('lists')[key]
  const incoming: readonly ListRow[] =
    event.type === 'EventsUpdated' ? event.data.events : event.data
  const next: ListState = {
    eventType: event.type,
    rows: reuseRows(prev?.rows ?? [], incoming),
    status: 'ready',
    updatedAt: Date.now(),
  }
  if (event.type === 'EventsUpdated') next.total = event.data.total_count
  const latest = env.correlation.type === 'Request' ? env.correlation.data : prev?.latestRequest
  if (latest !== undefined) next.latestRequest = latest
  d.edit('lists')[key] = next

  if (event.type === 'PortForwardsUpdated') tagPortForwardOrigins(d, event.data)
}

/**
 * 009 (data-model.md): match each not-yet-tagged `PortForward` row against a pending
 * Service-launched start (by namespace/pod/local_port/remote_port) and, on a match, tag
 * `portForwardOrigins[row.id]` and drop the pending entry so it is consumed at most once.
 */
function tagPortForwardOrigins(d: Draft, rows: readonly PortForward[]): void {
  const pending = d.read('pendingPortForwardOrigins')
  if (Object.keys(pending).length === 0) return
  const origins = d.read('portForwardOrigins')
  let nextPending: Record<string, string> | undefined
  let nextOrigins: Record<string, { service: string }> | undefined
  for (const row of rows) {
    if (row.id in origins) continue
    const key = pendingPortForwardOriginKey(
      row.namespace,
      row.pod,
      row.local_port,
      row.remote_port,
    )
    const service = (nextPending ?? pending)[key]
    if (service === undefined) continue
    nextOrigins ??= { ...origins }
    nextOrigins[row.id] = { service }
    nextPending ??= { ...pending }
    nextPending = Object.fromEntries(Object.entries(nextPending).filter(([k]) => k !== key))
  }
  if (nextOrigins) d.put('portForwardOrigins', nextOrigins)
  if (nextPending) d.put('pendingPortForwardOrigins', nextPending)
}

/**
 * Identity of a row across refreshes. Protocol v1 summaries carry no `uid`/`resourceVersion`,
 * so rows are matched by `id` (port forwards) or `namespace/name`, and reused only when every
 * field is unchanged.
 */
export function rowId(row: ListRow): string | undefined {
  if (typeof row === 'string') return row
  if ('id' in row) return `id:${row.id}`
  if ('namespace' in row) return `${row.namespace}/${row.name}`
  return row.name
}

/** Data-model rule 3: keep the previous object for unchanged rows so memoized rows skip rendering. */
export function reuseRows(prev: readonly ListRow[], next: readonly ListRow[]): readonly ListRow[] {
  if (prev.length === 0) return next
  const byId = new Map<string, ListRow>()
  for (const row of prev) {
    const id = rowId(row)
    if (id !== undefined) byId.set(id, row)
  }
  let changed = prev.length !== next.length
  const out = next.map((row, i) => {
    const id = rowId(row)
    const old = id === undefined ? undefined : byId.get(id)
    const kept = old !== undefined && sameRow(old, row) ? old : row
    if (kept !== prev[i]) changed = true
    return kept
  })
  // Nothing changed at all: keep the old array too.
  return changed ? out : prev
}

function sameRow(a: ListRow, b: ListRow): boolean {
  if (a === b) return true
  if (typeof a !== 'object' || typeof b !== 'object') return false
  const ra = a as unknown as Record<string, unknown>
  const rb = b as unknown as Record<string, unknown>
  const keys = Object.keys(ra)
  if (keys.length !== Object.keys(rb).length) return false
  for (const k of keys) {
    const va = ra[k]
    const vb = rb[k]
    if (va === vb) continue
    if (typeof va !== 'object' || typeof vb !== 'object' || va === null || vb === null) return false
    if (JSON.stringify(va) !== JSON.stringify(vb)) return false
  }
  return true
}
