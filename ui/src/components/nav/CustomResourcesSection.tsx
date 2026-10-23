/**
 * Generic, read-only Custom Resource Definition browsing (spec 003, User Story 3): a CRD
 * catalog list, one selected kind's instance list, and instance details — reusing the same
 * Details/YAML/Events tab shapes and `jsonToYaml` the built-in-kind Inspector uses, rather
 * than a parallel bespoke view (T056). This is a single self-contained pane rather than going
 * through the closed `descriptorFor`/`ToolKind` registry, since that registry is a compile-time
 * set of built-in kinds and isn't designed for kinds discovered at runtime.
 */
import { useEffect, useMemo, useRef, useState } from 'react'

import { Tabs, TabsContent, TabsList, TabsTrigger } from '@companion/ui'
import { SECTION } from '@/protocol/commands'
import type { CrdCatalogEntry, CrdInstanceDetails, KubernetesEvent } from '@/protocol/types'
import { formatAge } from '@/resources/format'
import { jsonToYaml } from '@/resources/yaml'
import { detailKey, listKey, useStore, type CrdInstancesState, type DetailState } from '@/store'
import { cn } from '@companion/theme'

import { Empty, ErrorState, Loading } from '../shell/PaneStates'

function catalogKey(e: Pick<CrdCatalogEntry, 'group' | 'kind'>): string {
  return `${e.group}/${e.kind}`
}

export function CustomResourcesSection() {
  const catalog = useStore((s) => s.crds.catalog)
  const catalogLoaded = useStore((s) => s.crds.catalogLoaded)
  const selected = useStore((s) => s.crds.selected)
  const selectCrdKind = useStore((s) => s.selectCrdKind)
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const context = useStore((s) => s.session.activeContext)
  const send = useStore((s) => s.send)

  // #36: a context switch resets the catalog (it described the previous cluster). Navigating
  // here already asked for one, so only a switch made while this pane is open asks again.
  const shownContext = useRef(context)
  useEffect(() => {
    if (shownContext.current === context) return
    shownContext.current = context
    if (context !== undefined) send({ type: 'RefreshCrds' })
  }, [context, send])

  const [selectedInstance, setSelectedInstance] = useState<
    { namespace: string | null; name: string } | undefined
  >(undefined)

  const selectedEntry = selected
    ? catalog.find((e) => e.group === selected.group && e.kind === selected.kind)
    : undefined

  if (disconnected) return null // PaneHost's own Disconnected state covers this section too.

  return (
    <div className="flex min-h-0 flex-1 flex-col md:flex-row">
      <div className="flex min-h-0 w-full shrink-0 flex-col overflow-y-auto border-b border-border md:w-64 md:border-r md:border-b-0">
        <CrdCatalogList
          catalog={catalog}
          catalogLoaded={catalogLoaded}
          selected={selected}
          onSelect={(entry) => {
            setSelectedInstance(undefined)
            selectCrdKind(entry)
          }}
        />
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        {!selected ? (
          <Empty title="Pick a Custom Resource" detail="Choose a kind from the catalog." />
        ) : (
          <CrdInstancesList
            group={selected.group}
            kind={selected.kind}
            entry={selectedEntry}
            selectedInstance={selectedInstance}
            onSelect={setSelectedInstance}
          />
        )}
      </div>
      {selected && selectedInstance && (
        <div className="flex min-h-0 w-full shrink-0 flex-col overflow-hidden border-t border-border md:w-[28rem] md:border-t-0 md:border-l">
          <CrdInstanceDetail
            group={selected.group}
            kind={selected.kind}
            version={selected.version}
            namespace={selectedInstance.namespace}
            name={selectedInstance.name}
          />
        </div>
      )}
    </div>
  )
}

function CrdCatalogList({
  catalog,
  catalogLoaded,
  selected,
  onSelect,
}: {
  catalog: readonly CrdCatalogEntry[]
  catalogLoaded: boolean
  selected: { group: string; kind: string } | undefined
  onSelect: (entry: CrdCatalogEntry) => void
}) {
  if (!catalogLoaded) return <Loading rows={6} label="Discovering Custom Resource Definitions" />
  if (catalog.length === 0) {
    return (
      <Empty title="No Custom Resource Definitions" detail="This cluster has no CRDs installed." />
    )
  }
  return (
    <ul role="listbox" aria-label="Custom Resource kinds" className="flex flex-col">
      {catalog.map((entry) => {
        const active = selected?.group === entry.group && selected.kind === entry.kind
        return (
          <li key={catalogKey(entry)}>
            <button
              type="button"
              role="option"
              aria-selected={active}
              onClick={() => onSelect(entry)}
              className={cn(
                'flex w-full flex-col items-start gap-0.5 border-b border-border/50 px-3 py-2 text-left text-sm hover:bg-accent/50',
                active && 'bg-accent-neon/10 text-accent-neon',
              )}
            >
              <span className="font-mono font-medium">{entry.kind}</span>
              <span className="truncate text-xs text-muted-foreground">{entry.group}</span>
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                <span>{entry.scope}</span>
                {entry.permission_denied && (
                  <span
                    role="status"
                    className="text-status-warn"
                    title="Not permitted to list instances of this kind"
                  >
                    ▲ permission denied
                  </span>
                )}
              </span>
            </button>
          </li>
        )
      })}
    </ul>
  )
}

function CrdInstancesList({
  group,
  kind,
  entry,
  selectedInstance,
  onSelect,
}: {
  group: string
  kind: string
  entry: CrdCatalogEntry | undefined
  selectedInstance: { namespace: string | null; name: string } | undefined
  onSelect: (instance: { namespace: string | null; name: string }) => void
}) {
  const key = `${group}/${kind}`
  const state = useStore((s) => s.crds.instances[key])
  const refresh = useStore((s) => s.refreshCrdInstances)
  // Selecting the kind (`selectCrdKind`) fetches its instances; polling is not in scope for US3.

  const parsed = useMemo(() => parseInstances(state), [state])
  const retry = entry ? () => refresh(entry) : undefined

  if (state?.status === 'unavailable') {
    return (
      <Empty
        title={`${kind} is no longer available`}
        detail="This kind was removed from the CRD catalog (the CRD may have been deleted). Pick another kind."
      />
    )
  }
  if (state?.status === 'forbidden' || (entry?.permission_denied && state?.status !== 'ready')) {
    return (
      <Empty
        title="Permission denied"
        detail={
          state?.message
            ? `Not permitted to list ${kind} instances: ${state.message}`
            : `Not permitted to list ${kind} instances in this cluster.`
        }
      />
    )
  }
  if (state?.status === 'notFound') {
    return (
      <ErrorState
        title={`${kind} not found`}
        detail={`The cluster does not serve ${kind}.${group} (${state.message ?? 'not found'}). The CRD may have been removed.`}
        onRetry={retry}
      />
    )
  }
  if (state?.status === 'error') {
    return (
      <ErrorState
        title={`Could not list ${kind} instances`}
        detail={state.message ?? 'The list could not be read.'}
        onRetry={retry}
      />
    )
  }
  if (!state) return <Loading rows={6} label={`Loading ${kind} instances`} />
  if (parsed.length === 0) {
    return <Empty title="No instances" detail={`No ${kind} instances found.`} />
  }
  return (
    <table
      data-testid="resource-table"
      data-section="custom-resources"
      data-row-count={parsed.length}
      className="w-full text-sm"
    >
      <thead className="sticky top-0 border-b border-border bg-card text-left text-xs text-muted-foreground uppercase">
        <tr>
          <th className="px-3 py-2 font-medium">Name</th>
          <th className="px-3 py-2 font-medium">Namespace</th>
        </tr>
      </thead>
      <tbody>
        {parsed.map(({ namespace, name }) => {
          const rowKey = namespace ? `${namespace}/${name}` : name
          const active = selectedInstance?.name === name && selectedInstance.namespace === namespace
          return (
            <tr
              key={rowKey}
              role="row"
              data-row-key={rowKey}
              onClick={() => onSelect({ namespace, name })}
              className={cn(
                'cursor-pointer border-b border-border/50 hover:bg-accent/50',
                active && 'bg-accent-neon/10',
              )}
            >
              <td className="px-3 py-1.5 font-mono">{name}</td>
              <td className="px-3 py-1.5 font-mono text-muted-foreground">{namespace ?? '—'}</td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

/** Narrows a generic `DetailState` to `CrdInstanceDetails` by its event type. */
function asCrdInstanceDetails(detail: DetailState | undefined): CrdInstanceDetails | undefined {
  return detail?.eventType === 'CrdInstanceDetailsUpdated'
    ? (detail.data as CrdInstanceDetails)
    : undefined
}

function parseInstances(
  state: CrdInstancesState | undefined,
): { namespace: string | null; name: string }[] {
  if (!state) return []
  const out: { namespace: string | null; name: string }[] = []
  for (const raw of state.items) {
    try {
      const obj = JSON.parse(raw) as { metadata?: { name?: string; namespace?: string } }
      const name = obj.metadata?.name
      if (name) out.push({ namespace: obj.metadata?.namespace ?? null, name })
    } catch {
      // Malformed entry: skip rather than failing the whole list.
    }
  }
  return out
}

function CrdInstanceDetail({
  group,
  kind,
  version,
  namespace,
  name,
}: {
  group: string
  kind: string
  version: string
  namespace: string | null
  name: string
}) {
  const send = useStore((s) => s.send)
  const ctx = useStore((s) => s.session.activeContext)
  // `SelectCrdInstance` emits over `Route::global`, so this is stored with a null context.
  const detail = useStore((s) => s.details[detailKey(null, 'CrdInstance', namespace, name)])

  useEffect(() => {
    send({ type: 'SelectCrdInstance', data: { group, version, kind, namespace, name } })
  }, [group, version, kind, namespace, name, send])

  const crdDetail = asCrdInstanceDetails(detail)
  const raw = crdDetail?.raw_json

  return (
    <div data-testid="crd-instance-detail" className="flex min-h-0 flex-1 flex-col">
      <p className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5 font-mono text-xs text-muted-foreground">
        <span className="text-accent-neon">{kind}</span>
        {namespace && (
          <>
            <span aria-hidden="true">/</span>
            <span>{namespace}</span>
          </>
        )}
        <span aria-hidden="true">/</span>
        <span>{name}</span>
      </p>
      <Tabs defaultValue="details" className="min-h-0 flex-1 gap-0">
        <TabsList
          variant="line"
          className="h-[var(--touch-target)] w-full shrink-0 justify-start border-b border-border px-2"
        >
          <TabsTrigger value="details" className="h-full flex-none px-3 uppercase">
            Details
          </TabsTrigger>
          <TabsTrigger value="yaml" className="h-full flex-none px-3 uppercase">
            YAML
          </TabsTrigger>
          <TabsTrigger value="events" className="h-full flex-none px-3 uppercase">
            Events
          </TabsTrigger>
        </TabsList>
        <TabsContent value="details" className="min-h-0 overflow-y-auto p-3 text-sm">
          {raw === undefined ? (
            <Loading rows={6} label="Loading details" />
          ) : (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-xs">
              <dt className="text-muted-foreground">Group</dt>
              <dd>{group}</dd>
              <dt className="text-muted-foreground">Kind</dt>
              <dd>{kind}</dd>
              <dt className="text-muted-foreground">Version</dt>
              <dd>{version}</dd>
              <dt className="text-muted-foreground">Namespace</dt>
              <dd>{namespace ?? '—'}</dd>
              <dt className="text-muted-foreground">Name</dt>
              <dd>{name}</dd>
              {crdDetail && (
                <>
                  <dt className="text-muted-foreground">Resource Version</dt>
                  <dd>{crdDetail.resource_version}</dd>
                </>
              )}
            </dl>
          )}
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
          <CrdInstanceEvents kind={kind} namespace={namespace} name={name} ctx={ctx} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function CrdInstanceEvents({
  kind,
  namespace,
  name,
  ctx,
}: {
  kind: string
  namespace: string | null
  name: string
  ctx: string | undefined
}) {
  const list = useStore((s) => s.lists[listKey(ctx, SECTION.events, 'Cluster', null)])
  const events = useMemo(
    () =>
      (list?.rows ?? [])
        .filter(
          (e): e is KubernetesEvent =>
            typeof e === 'object' &&
            'involved_object_kind' in e &&
            e.involved_object_kind === kind &&
            e.involved_object_name === name &&
            (namespace === null || e.namespace === namespace),
        )
        .sort((a, b) => Date.parse(b.last_timestamp) - Date.parse(a.last_timestamp)),
    [list, kind, namespace, name],
  )
  if (!list || (list.status === 'loading' && list.rows.length === 0)) {
    return <Loading rows={4} label="Loading events" />
  }
  if (events.length === 0) {
    return <Empty title="No events" detail={`No recent events for ${name}.`} />
  }
  return (
    <ol className="flex flex-col">
      {events.map((e) => (
        <li
          key={`${e.name}-${e.last_timestamp}`}
          className="flex gap-2 border-b border-border/50 px-3 py-2 text-sm"
        >
          <span
            aria-hidden="true"
            className={cn(
              'mt-0.5 shrink-0',
              e.event_type === 'Warning' ? 'text-status-warn' : 'text-muted-foreground',
            )}
          >
            {e.event_type === 'Warning' ? '▲' : '●'}
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
      ))}
    </ol>
  )
}
