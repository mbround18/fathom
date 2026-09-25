import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useMemo, useRef, useState } from 'react'

import { StatusBadge } from '@/components/StatusBadge'
import { Disconnected, Empty, Loading } from '@/components/shell/PaneStates'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { SECTION } from '@/protocol/commands'
import type { KubernetesEvent } from '@/protocol/types'
import { listKey, useStore } from '@/store'

import {
  asEvents,
  eventKey,
  filterEvents,
  isWarning,
  involvedRef,
  newestFirst,
  relativeTime,
  type EventTypeFilter,
} from './eventsModel'

/** Above this many rows the list is virtualized; below it a plain list keeps a11y trivial. */
const VIRTUALIZE_AT = 150
const ROW_HEIGHT = 28

/**
 * Cluster events (nav `events` section and the Events dock tool): newest first, filterable by
 * type, namespace and text. Click-through opens the involved object in the inspector.
 */
export default function EventsPanel() {
  const ctx = useStore((s) => s.session.activeContext)
  const list = useStore((s) => s.lists[listKey(ctx, SECTION.events, 'Cluster', null)])
  const gone = useStore((s) => s.conn.status === 'backend-gone')
  const phone = useStore((s) => s.layout.widthClass === 'phone')
  const refreshEvents = useStore((s) => s.refreshEvents)
  const openInspector = useStore((s) => s.openInspector)

  const [type, setType] = useState<EventTypeFilter>('all')
  const [namespace, setNamespace] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    refreshEvents()
  }, [refreshEvents, ctx])
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(id)
  }, [])

  const all = useMemo(() => newestFirst(asEvents(list?.rows)), [list?.rows])
  const namespaces = useMemo(() => [...new Set(all.map((e) => e.namespace))].sort(), [all])
  const shown = useMemo(
    () => filterEvents(all, { type, namespace, search }),
    [all, type, namespace, search],
  )

  if (gone && all.length === 0) return <Disconnected />
  if (!list || (list.status !== 'ready' && all.length === 0 && list.status !== 'error')) {
    return <Loading label="Loading events" />
  }
  if (list.status === 'error' && all.length === 0) {
    return <Empty title="Cannot list events" detail={list.error} />
  }

  return (
    <div data-testid="events-panel" className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border p-2">
        <div role="group" aria-label="Event type" className="flex">
          {(['all', 'warning'] as const).map((t) => (
            <button
              key={t}
              type="button"
              aria-pressed={type === t}
              onClick={() => setType(t)}
              className={cn(
                'min-h-[var(--touch-target)] border border-border px-3 text-xs first:rounded-l last:rounded-r',
                type === t
                  ? 'bg-accent-neon text-primary-foreground'
                  : 'bg-card text-muted-foreground hover:text-foreground',
              )}
            >
              {t === 'all' ? 'All' : 'Warning'}
            </button>
          ))}
        </div>
        <select
          aria-label="Namespace"
          value={namespace ?? ''}
          onChange={(e) => setNamespace(e.target.value === '' ? null : e.target.value)}
          className="min-h-[var(--touch-target)] rounded border border-border bg-card px-2 text-xs"
        >
          <option value="">All namespaces</option>
          {namespaces.map((n) => (
            <option key={n} value={n}>
              {n === '' ? '(cluster)' : n}
            </option>
          ))}
        </select>
        <Input
          type="search"
          aria-label="Search events"
          placeholder="search reason, message, object…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="h-[var(--touch-target)] min-w-40 flex-1"
        />
        <span aria-live="polite" className="text-xs text-muted-foreground">
          {shown.length} of {list.total ?? all.length}
        </span>
      </div>
      {shown.length === 0 ? (
        <Empty
          title={all.length === 0 ? 'No events' : 'No matching events'}
          detail={
            all.length === 0
              ? 'The cluster has reported no events.'
              : 'Adjust the type, namespace or search filters.'
          }
        />
      ) : shown.length > VIRTUALIZE_AT && !phone ? (
        <VirtualRows events={shown} now={now} onOpen={openInspector} />
      ) : (
        <ul aria-label="Events" className="min-h-0 flex-1 overflow-auto">
          {shown.map((e) =>
            phone ? (
              <EventCard key={eventKey(e)} e={e} now={now} onOpen={openInspector} />
            ) : (
              <li key={eventKey(e)} style={{ height: ROW_HEIGHT }}>
                <EventRow e={e} now={now} onOpen={openInspector} />
              </li>
            ),
          )}
        </ul>
      )}
    </div>
  )
}

type Open = ReturnType<typeof useStore.getState>['openInspector']
interface RowProps {
  e: KubernetesEvent
  now: number
  onOpen: Open
}

function VirtualRows({
  events,
  now,
  onOpen,
}: {
  events: KubernetesEvent[]
  now: number
  onOpen: Open
}) {
  const ref = useRef<HTMLUListElement>(null)
  const v = useVirtualizer({
    count: events.length,
    getScrollElement: () => ref.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  })
  return (
    <ul aria-label="Events" ref={ref} className="min-h-0 flex-1 overflow-auto">
      <li aria-hidden="true" style={{ height: v.getTotalSize(), position: 'relative' }}>
        {v.getVirtualItems().map((item) => {
          const e = events[item.index]
          if (!e) return null
          return (
            <div
              key={eventKey(e)}
              role="listitem"
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                height: item.size,
                transform: `translateY(${String(item.start)}px)`,
              }}
            >
              <EventRow e={e} now={now} onOpen={onOpen} />
            </div>
          )
        })}
      </li>
    </ul>
  )
}

function ObjectLink({
  e,
  onOpen,
  className,
}: {
  e: KubernetesEvent
  onOpen: Open
  className?: string
}) {
  const ref = involvedRef(e)
  const label = `${e.involved_object_kind}/${e.involved_object_name}`
  if (!ref) return <span className={cn('truncate font-mono', className)}>{label}</span>
  return (
    <button
      type="button"
      onClick={() => onOpen(ref)}
      aria-label={`Open ${label}`}
      className={cn('truncate text-left font-mono text-accent-neon hover:underline', className)}
    >
      {label}
    </button>
  )
}

/** Dense one-line row (tablet and up). */
function EventRow({ e, now, onOpen }: RowProps) {
  return (
    <div
      data-event-row={eventKey(e)}
      className="grid h-full [grid-template-columns:5.5rem_3rem_9rem_minmax(8rem,14rem)_1fr_4rem] items-center gap-2 border-b border-border/50 px-2 text-xs"
    >
      <StatusBadge status={isWarning(e) ? 'Degraded' : 'Ready'} label={e.event_type} />
      <span className="text-muted-foreground" title={e.last_timestamp}>
        {relativeTime(e.last_timestamp || e.first_timestamp, now)}
      </span>
      <span className="truncate font-semibold" title={e.reason}>
        {e.reason}
      </span>
      <ObjectLink e={e} onOpen={onOpen} />
      <span className="truncate" title={e.message}>
        {e.message}
      </span>
      <span className="text-right text-muted-foreground" title="Occurrences">
        {e.count > 1 ? `×${String(e.count)}` : ''}
      </span>
    </div>
  )
}

/** Card layout for phones. */
function EventCard({ e, now, onOpen }: RowProps) {
  return (
    <li
      data-event-row={eventKey(e)}
      className="flex flex-col gap-1 border-b border-border p-3 text-sm"
    >
      <div className="flex items-center gap-2">
        <StatusBadge status={isWarning(e) ? 'Degraded' : 'Ready'} label={e.event_type} />
        <span className="font-semibold">{e.reason}</span>
        <span className="ml-auto text-xs text-muted-foreground">
          {relativeTime(e.last_timestamp || e.first_timestamp, now)}
          {e.count > 1 ? ` ×${String(e.count)}` : ''}
        </span>
      </div>
      <ObjectLink
        e={e}
        onOpen={onOpen}
        className="min-h-[var(--touch-target)] self-start text-xs"
      />
      <p className="break-words text-muted-foreground">{e.message}</p>
      {e.namespace && <span className="text-xs text-muted-foreground">{e.namespace}</span>}
    </li>
  )
}
