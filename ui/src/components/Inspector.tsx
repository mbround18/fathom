/**
 * Inspector pane (T063): Details, YAML, and Events for the object in `pane.target`.
 * The pane header (PaneHost) carries the object name as its heading.
 */
import { useEffect, useMemo, useState } from 'react'

import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { SECTION } from '@/protocol/commands'
import type {
  HelmReleaseDetails,
  HelmReleaseSummary,
  KubernetesEvent,
  PodSummary,
  RelatedResourceRef,
} from '@/protocol/types'
import { descriptorFor, type ObjectRef } from '@/resources'
import { formatAge } from '@/resources/format'
import { jsonToYaml } from '@/resources/yaml'
import { detailKey, listKey, useStore, type ListRow, type PaneId, type PaneState } from '@/store'
import { cn } from '@/lib/utils'

import { DetailSections } from './DetailSections'
import { HelmReleaseActionsPanel } from './panels/HelmReleaseActionsPanel'
import { NodeActionsPanel } from './panels/NodeActionsPanel'
import { PodLogsTab } from './panels/PodLogsTab'
import { PodTerminalTab } from './panels/PodTerminalTab'
import { YamlEditTab } from './panels/YamlEditTab'
import { Empty, Loading, NoLongerExists } from './shell/PaneStates'

const DISCARD_EDIT_PROMPT =
  'You have unsaved YAML changes that have not been applied. Leave anyway and discard them?'

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
  const listScope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const list = useStore((s) => s.lists[listKey(ctx, target.section, listScope, null)])
  const detailState = useStore(
    (s) => s.details[detailKey(ctx, target.kind, target.namespace, target.name)],
  )
  // US3: `RefreshRelatedResources`'s reply, keyed like every other `ResourceReadEvent` (its
  // derived kind is `RelatedResources`, from `readKind()` stripping the trailing "Updated").
  const relatedState = useStore(
    (s) => s.details[detailKey(ctx, 'RelatedResources', target.namespace, target.name)],
  )
  const related =
    relatedState?.eventType === 'RelatedResourcesUpdated'
      ? (relatedState.data as { related: RelatedResourceRef[] }).related
      : undefined
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

  // T026: block navigating away from the YAML tab while an unsaved (dirty, not yet applied)
  // edit session is open, mirroring the constitution's "explicit confirmation" stance for
  // discarding in-progress work rather than silently losing it on tab switch. Declared before
  // the early returns below so hook order stays stable regardless of `descriptor`/`row`.
  const editKey = detailKey(ctx, target.kind, target.namespace, target.name)
  const editSession = useStore((s) => s.editSessions[editKey])
  const [tab, setTab] = useState('details')
  const handleTabChange = (next: string) => {
    if (tab === 'yaml' && next !== 'yaml' && editSession?.dirty && editSession.status !== 'applying') {
      if (!window.confirm(DISCARD_EDIT_PROMPT)) return
    }
    setTab(next)
  }

  if (!descriptor)
    return <Empty title="Not ported yet" detail={`${target.kind} has no inspector yet.`} />
  if (list?.status === 'ready' && !row) return <NoLongerExists name={target.name} />
  // Flux origin click-through (spec 005 FR-013): a deleted origin resolves to an explicit
  // `FluxResourceNotFound` reply, not a blank/frozen panel.
  if (detailState?.eventType === 'FluxResourceNotFound') return <NoLongerExists name={target.name} />

  const detail = detailState ? descriptor.selectDetail(detailState) : undefined
  const raw = detail !== undefined && descriptor.rawJson ? descriptor.rawJson(detail) : undefined
  const editable = descriptor.rawJson !== undefined

  // Terminal tab (FR-007): only for Pod resources currently in a "Running" phase.
  const isRunningPod = target.kind === 'Pod' && (row as PodSummary | undefined)?.phase === 'Running'

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
      {pane.history && pane.history.length > 0 && (
        <Breadcrumb paneId={pane.id} history={pane.history} target={target} />
      )}
      <Tabs value={tab} onValueChange={handleTabChange} className="min-h-0 flex-1 gap-0">
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
          {target.kind === 'Pod' && (
            <TabsTrigger
              value="logs"
              className="h-full flex-none px-3 font-display tracking-wider uppercase"
            >
              Logs
            </TabsTrigger>
          )}
          {isRunningPod && (
            <TabsTrigger
              value="terminal"
              className="h-full flex-none px-3 font-display tracking-wider uppercase"
            >
              Terminal
            </TabsTrigger>
          )}
          {target.kind === 'Node' && (
            <TabsTrigger
              value="node-actions"
              data-testid="node-actions-tab"
              className="h-full flex-none px-3 font-display tracking-wider uppercase"
            >
              Actions
            </TabsTrigger>
          )}
          {target.kind === 'HelmRelease' && (
            <TabsTrigger
              value="helm-actions"
              data-testid="helm-actions-tab"
              className="h-full flex-none px-3 font-display tracking-wider uppercase"
            >
              Actions
            </TabsTrigger>
          )}
        </TabsList>
        <TabsContent value="details" className="min-h-0 overflow-y-auto text-sm">
          <DetailSections
            descriptor={descriptor}
            row={row}
            detail={detail}
            hiddenColumns={hiddenColumns}
            related={related}
          />
        </TabsContent>
        <TabsContent value="yaml" className="min-h-0 overflow-auto">
          {editable ? (
            <YamlEditTab target={target} raw={raw} />
          ) : raw === undefined ? (
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
        {target.kind === 'Pod' && (
          <TabsContent value="logs" className="flex min-h-0 flex-1 flex-col">
            <PodLogsTab target={target} />
          </TabsContent>
        )}
        {isRunningPod && (
          <TabsContent value="terminal" className="flex min-h-0 flex-1 flex-col">
            <PodTerminalTab target={target} />
          </TabsContent>
        )}
        {target.kind === 'Node' && (
          <TabsContent value="node-actions" className="min-h-0 flex-1 overflow-y-auto">
            <NodeActionsPanel nodeName={target.name} />
          </TabsContent>
        )}
        {target.kind === 'HelmRelease' && target.namespace && (
          <TabsContent value="helm-actions" className="min-h-0 flex-1 overflow-y-auto">
            {(() => {
              const helmDetail = detail as HelmReleaseDetails | undefined
              const revision = helmDetail?.revision ?? (row as HelmReleaseSummary | undefined)?.revision
              if (revision === undefined) return <Loading rows={4} label="Loading release" />
              return (
                <HelmReleaseActionsPanel
                  namespace={target.namespace}
                  release={target.name}
                  revision={revision}
                  origin={helmDetail?.origin ?? null}
                />
              )
            })()}
          </TabsContent>
        )}
      </Tabs>
    </div>
  )
}

/** US3: drill-down back-stack (`pane.history`), each prior segment clickable to jump back to it,
 * plus a "Back" control for the single most recent step. */
function Breadcrumb({
  paneId,
  history,
  target,
}: {
  paneId: PaneId
  history: ObjectRef[]
  target: ObjectRef
}) {
  const goBack = useStore((s) => s.goBack)
  const updatePane = useStore((s) => s.updatePane)
  const segments = [...history, target]
  return (
    <nav
      aria-label="Breadcrumb"
      className="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-border px-3 py-1 font-mono text-xs text-muted-foreground"
    >
      <button
        type="button"
        onClick={() => {
          goBack(paneId)
        }}
        className="mr-1 shrink-0 text-accent-neon hover:underline"
      >
        ← Back
      </button>
      {segments.map((seg, i) => {
        const isLast = i === segments.length - 1
        return (
          <span key={`${seg.section}-${seg.namespace ?? ''}-${seg.name}-${String(i)}`} className="flex items-center gap-1">
            {i > 0 && <span aria-hidden="true">/</span>}
            {isLast ? (
              <span className="text-foreground">{seg.name}</span>
            ) : (
              <button
                type="button"
                onClick={() => {
                  updatePane(paneId, { target: seg, history: segments.slice(0, i) })
                }}
                className="hover:text-accent-neon hover:underline"
              >
                {seg.name}
              </button>
            )}
          </span>
        )
      })}
    </nav>
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
