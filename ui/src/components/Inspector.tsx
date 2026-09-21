/**
 * Inspector pane (T063): Details, YAML, and Events for the object in `pane.target`.
 * The pane header (PaneHost) carries the object name as its heading.
 */
import { useEffect, useMemo } from 'react'

import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { SECTION } from '@/protocol/commands'
import type { KubernetesEvent } from '@/protocol/types'
import { descriptorFor, type ObjectRef } from '@/resources'
import { formatAge } from '@/resources/format'
import { jsonToYaml } from '@/resources/yaml'
import { detailKey, listKey, useStore, type ListRow, type PaneState } from '@/store'
import { cn } from '@/lib/utils'

import { DetailSections } from './DetailSections'
import { Empty, Loading, NoLongerExists } from './shell/PaneStates'

const NO_ROWS: readonly ListRow[] = []

export function Inspector({ pane }: { pane: PaneState }) {
  useEscapeToClose(pane.target !== undefined)
  if (!pane.target) {
    return <Empty title="Nothing selected" detail="Select a resource to inspect it." />
  }
  return <InspectorBody key={refKey(pane.target)} pane={pane} target={pane.target} />
}

function refKey(r: ObjectRef): string {
  return `${r.section}|${r.namespace ?? ''}|${r.name}`
}

function InspectorBody({ pane, target }: { pane: PaneState; target: ObjectRef }) {
  const descriptor = descriptorFor(target.section)
  const ctx = useStore((s) => s.session.activeContext)
  const list = useStore((s) => s.lists[listKey(ctx, target.section, 'Namespaced', null)])
  const detailState = useStore(
    (s) => s.details[detailKey(ctx, target.kind, target.namespace, target.name)],
  )
  const hidden = useStore((s) => {
    const src = pane.source ? s.panes[pane.source] : undefined
    return src?.visibleColumns
  })

  const rowKey = target.namespace ? `${target.namespace}/${target.name}` : target.name
  const row = useMemo(
    () =>
      descriptor ? (list?.rows ?? NO_ROWS).find((r) => descriptor.rowKey(r) === rowKey) : undefined,
    [descriptor, list, rowKey],
  )
  const hiddenColumns = useMemo(
    () =>
      descriptor && hidden
        ? descriptor.columns.filter((c) => !hidden.includes(c.id)).map((c) => c.id)
        : undefined,
    [descriptor, hidden],
  )

  if (!descriptor)
    return <Empty title="Not ported yet" detail={`${target.kind} has no inspector yet.`} />
  if (list?.status === 'ready' && !row) return <NoLongerExists name={target.name} />

  const detail = detailState ? descriptor.selectDetail(detailState) : undefined
  const raw = detail !== undefined && descriptor.rawJson ? descriptor.rawJson(detail) : undefined

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <p className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5 font-mono text-xs text-muted-foreground">
        <span className="text-accent-neon">{target.kind}</span>
        {target.namespace && (
          <>
            <span aria-hidden="true">/</span>
            <span>{target.namespace}</span>
          </>
        )}
      </p>
      <Tabs defaultValue="details" className="min-h-0 flex-1 gap-0">
        <TabsList
          variant="line"
          className="h-[var(--touch-target)] w-full shrink-0 justify-start border-b border-border px-2"
        >
          <TabsTrigger
            value="details"
            className="h-full flex-none px-3 font-display tracking-wider uppercase"
          >
            Details
          </TabsTrigger>
          <TabsTrigger
            value="yaml"
            className="h-full flex-none px-3 font-display tracking-wider uppercase"
          >
            YAML
          </TabsTrigger>
          <TabsTrigger
            value="events"
            className="h-full flex-none px-3 font-display tracking-wider uppercase"
          >
            Events
          </TabsTrigger>
        </TabsList>
        <TabsContent value="details" className="min-h-0 overflow-y-auto text-sm">
          <DetailSections
            descriptor={descriptor}
            row={row}
            detail={detail}
            hiddenColumns={hiddenColumns}
          />
        </TabsContent>
        <TabsContent value="yaml" className="min-h-0 overflow-auto">
          {raw === undefined ? (
            <Loading rows={10} label="Loading YAML" />
          ) : (
            <pre className="min-w-0 p-3 font-mono text-xs leading-relaxed whitespace-pre text-foreground/90">
              {jsonToYaml(raw)}
            </pre>
          )}
        </TabsContent>
        <TabsContent value="events" className="min-h-0 overflow-y-auto">
          <EventsFor target={target} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function EventsFor({ target }: { target: ObjectRef }) {
  const list = useStore(
    (s) => s.lists[listKey(s.session.activeContext, SECTION.events, 'Cluster', null)],
  )
  const events = useMemo(
    () =>
      (list?.rows ?? NO_ROWS)
        .filter(
          (e): e is KubernetesEvent =>
            typeof e === 'object' &&
            'involved_object_kind' in e &&
            e.involved_object_kind === target.kind &&
            e.involved_object_name === target.name &&
            (target.namespace === null || e.namespace === target.namespace),
        )
        .sort((a, b) => Date.parse(b.last_timestamp) - Date.parse(a.last_timestamp)),
    [list, target],
  )
  if (!list || (list.status === 'loading' && list.rows.length === 0)) {
    return <Loading rows={4} label="Loading events" />
  }
  if (events.length === 0) {
    return <Empty title="No events" detail={`No recent events for ${target.name}.`} />
  }
  return (
    <ol className="flex flex-col">
      {events.map((e) => {
        const warn = e.event_type === 'Warning'
        return (
          <li
            key={`${e.name}-${e.last_timestamp}`}
            className="flex gap-2 border-b border-border/50 px-3 py-2 text-sm"
          >
            <span
              aria-hidden="true"
              className={cn('mt-0.5 shrink-0', warn ? 'text-status-warn' : 'text-muted-foreground')}
            >
              {warn ? '▲' : '●'}
            </span>
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <p className="flex flex-wrap items-baseline gap-x-2">
                <span className="font-medium">{e.reason}</span>
                <span className="text-xs text-muted-foreground">{e.event_type}</span>
                {e.count > 1 && (
                  <span className="font-mono text-xs text-muted-foreground">×{e.count}</span>
                )}
                <span className="ml-auto font-mono text-xs text-muted-foreground">
                  {formatAge(e.last_timestamp)} ago
                </span>
              </p>
              <p className="text-xs break-words text-muted-foreground">{e.message}</p>
            </div>
          </li>
        )
      })}
    </ol>
  )
}

/** Esc closes the inspector, unless a menu or dialog is open (it handles Esc itself). */
function useEscapeToClose(active: boolean) {
  const closeInspector = useStore((s) => s.closeInspector)
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return
      if (document.querySelector('[role="listbox"], [role="dialog"], [role="menu"]')) return
      closeInspector()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [active, closeInspector])
}
