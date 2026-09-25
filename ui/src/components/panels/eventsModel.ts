/** Pure helpers shared by the Events and Overview panels (batch E). */
import type { KubernetesEvent } from '@/protocol/types'
import { ALL_DESCRIPTORS, type ObjectRef } from '@/resources'
import type { ListRow } from '@/store'

export type EventTypeFilter = 'all' | 'warning'

export function asEvents(rows: readonly ListRow[] | undefined): KubernetesEvent[] {
  return (rows ?? []).filter(
    (r): r is KubernetesEvent => typeof r === 'object' && 'involved_object_kind' in r,
  )
}

export function isWarning(e: KubernetesEvent): boolean {
  return e.event_type.toLowerCase() === 'warning'
}

/** Best timestamp for ordering: last seen, else first seen; `NaN` when neither parses. */
export function eventTime(e: KubernetesEvent): number {
  const t = Date.parse(e.last_timestamp || e.first_timestamp)
  return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t
}

export function newestFirst(events: readonly KubernetesEvent[]): KubernetesEvent[] {
  return [...events].sort((a, b) => eventTime(b) - eventTime(a))
}

export interface EventFilter {
  type: EventTypeFilter
  /** `null` = all namespaces. */
  namespace: string | null
  search: string
}

export function filterEvents(
  events: readonly KubernetesEvent[],
  f: EventFilter,
): KubernetesEvent[] {
  const q = f.search.trim().toLowerCase()
  return events.filter((e) => {
    if (f.type === 'warning' && !isWarning(e)) return false
    if (f.namespace !== null && e.namespace !== f.namespace) return false
    if (q === '') return true
    return [
      e.reason,
      e.message,
      e.involved_object_kind,
      e.involved_object_name,
      e.namespace,
      e.source,
    ]
      .join('\u0000')
      .toLowerCase()
      .includes(q)
  })
}

/** `5s`, `3m`, `2h`, `4d` ago; `—` when the timestamp is unusable. */
export function relativeTime(iso: string, now: number): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return '—'
  const s = Math.max(0, Math.round((now - t) / 1000))
  if (s < 60) return `${String(s)}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${String(m)}m`
  const h = Math.floor(m / 60)
  if (h < 48) return `${String(h)}h`
  return `${String(Math.floor(h / 24))}d`
}

/** The inspector target for an event's involved object, if the UI has a descriptor for its kind. */
export function involvedRef(e: KubernetesEvent): ObjectRef | undefined {
  const d = ALL_DESCRIPTORS.find((x) => x.kind === e.involved_object_kind)
  if (!d) return undefined
  return {
    section: d.section,
    kind: d.kind,
    namespace: d.namespaced ? e.namespace : null,
    name: e.involved_object_name,
  }
}

export function eventKey(e: KubernetesEvent): string {
  return `${e.namespace}/${e.name}`
}
