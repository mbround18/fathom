import { useEffect, useMemo, useState } from 'react'

import { StatusBadge } from '@/components/StatusBadge'
import { FirstRunState } from '@/components/shell/FirstRunState'
import { Disconnected, Empty } from '@companion/terminal'
import { useFirstRun } from '@/lib/use-first-run'
import { SECTION } from '@/protocol/commands'
import type { NodeSummary, PodSummary, WorkloadSummary } from '@/protocol/types'
import { statusTone } from '@/resources/search'
import { deniedKey, listKey, useStore, type ListRow, type PaneId } from '@/store'

import {
  asEvents,
  eventKey,
  involvedRef,
  isWarning,
  newestFirst,
  relativeTime,
} from './eventsModel'

const RECENT_WARNINGS = 8

export interface OverviewCounts {
  nodes: number
  nodesUnhealthy: number
  pods: number
  podsUnhealthy: number
  deployments: number
  deploymentsUnhealthy: number
}

const objects = <T,>(rows: readonly ListRow[] | undefined, key: string): T[] =>
  (rows ?? []).filter((r) => typeof r === 'object' && key in r) as unknown as T[]

const unhealthyTone = (status: string): boolean => {
  const { tone } = statusTone(status)
  return tone === 'err' || tone === 'warn'
}

/** Pure summary of the cached lists (exported for tests). */
export function summarize(
  nodeRows: readonly ListRow[] | undefined,
  podRows: readonly ListRow[] | undefined,
  deploymentRows: readonly ListRow[] | undefined,
): OverviewCounts {
  const nodes = objects<NodeSummary>(nodeRows, 'pod_capacity')
  const pods = objects<PodSummary>(podRows, 'phase')
  const deployments = objects<WorkloadSummary>(deploymentRows, 'desired')
  return {
    nodes: nodes.length,
    nodesUnhealthy: nodes.filter((n) => statusTone(n.status).tone !== 'ok').length,
    pods: pods.length,
    podsUnhealthy: pods.filter(
      (p) => unhealthyTone(p.phase) || (p.phase === 'Running' && p.ready < p.total),
    ).length,
    deployments: deployments.length,
    deploymentsUnhealthy: deployments.filter((d) => d.ready < d.desired).length,
  }
}

/** Cluster summary: counts, unhealthy tallies, namespaces, server version, recent warnings. */
export default function OverviewPanel({ paneId }: { paneId?: PaneId } = {}) {
  const ctx = useStore((s) => s.session.activeContext)
  const gone = useStore((s) => s.conn.status === 'backend-gone')
  const firstRun = useFirstRun()
  const nodeRows = useStore((s) => s.lists[listKey(ctx, SECTION.nodes, 'Cluster', null)]?.rows)
  const podRows = useStore((s) => s.lists[listKey(ctx, SECTION.pods, 'Namespaced', null)]?.rows)
  const depRows = useStore(
    (s) => s.lists[listKey(ctx, SECTION.deployments, 'Namespaced', null)]?.rows,
  )
  const eventRows = useStore((s) => s.lists[listKey(ctx, SECTION.events, 'Cluster', null)]?.rows)
  const namespaceCount = useStore((s) => s.session.namespaces.length)
  // 031: a namespace-scoped identity cannot list nodes; that card says so instead of "0".
  const nodesDenied = useStore(
    (s) =>
      s.denied[deniedKey(ctx, SECTION.nodes)] !== undefined ||
      s.lists[listKey(ctx, SECTION.nodes, 'Cluster', null)]?.forbidden === true,
  )
  const namespacesRestricted = useStore((s) => s.session.namespacesRestricted === true)
  const version = useStore((s) => s.clusterVersion)
  const refreshEvents = useStore((s) => s.refreshEvents)
  const openInspector = useStore((s) => s.openInspector)

  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(id)
  }, [])
  useEffect(() => {
    refreshEvents()
  }, [refreshEvents, ctx])
  // The counts read the Nodes/Pods/Deployments lists, which are otherwise only fetched by their
  // own panes: without this, Nodes read 0 until the Nodes pane had been opened once.
  const refreshSection = useStore((s) => s.refreshSection)
  const missingNodes = nodeRows === undefined
  const missingPods = podRows === undefined
  const missingDeps = depRows === undefined
  useEffect(() => {
    if (ctx === undefined || firstRun) return
    if (missingNodes) refreshSection(SECTION.nodes)
    if (missingPods) refreshSection(SECTION.pods)
    if (missingDeps) refreshSection(SECTION.deployments)
  }, [ctx, firstRun, missingNodes, missingPods, missingDeps, refreshSection])

  const counts = useMemo(() => summarize(nodeRows, podRows, depRows), [nodeRows, podRows, depRows])
  const warnings = useMemo(
    () => newestFirst(asEvents(eventRows).filter(isWarning)).slice(0, RECENT_WARNINGS),
    [eventRows],
  )

  if (gone) return <Disconnected />
  if (firstRun) return <FirstRunState state={firstRun} />
  if (ctx === undefined) {
    return <Empty title="Not connected" detail="Connect to a cluster to see its overview." />
  }

  return (
    <div data-testid="overview-panel" className="flex flex-col gap-4 p-3">
      <dl className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-6">
        {nodesDenied ? (
          <NotPermittedStat label="Nodes" />
        ) : (
          <Stat label="Nodes" value={counts.nodes} bad={counts.nodesUnhealthy} />
        )}
        <Stat label="Pods" value={counts.pods} bad={counts.podsUnhealthy} />
        <Stat label="Deployments" value={counts.deployments} bad={counts.deploymentsUnhealthy} />
        <Stat
          label="Namespaces"
          value={namespaceCount}
          {...(namespacesRestricted ? { note: 'reachable by you' } : {})}
        />
        <Stat label="Warnings" value={warnings.length} bad={warnings.length} />
        <div className="border border-border bg-card p-2">
          <dt className="text-xs text-muted-foreground">Server version</dt>
          <dd className="font-mono text-sm" data-stat="server-version">
            {version?.server_version ?? 'unknown'}
            {version?.skew_detected && (
              <span className="ml-1 text-status-warn"> (version skew)</span>
            )}
          </dd>
        </div>
      </dl>
      <section aria-label="Recent warnings">
        <h3 className="mb-1 font-display text-xs tracking-wider uppercase">Recent warnings</h3>
        {warnings.length === 0 ? (
          <p className="text-xs text-muted-foreground">No warning events.</p>
        ) : (
          <ul className="flex flex-col divide-y divide-border border border-border">
            {warnings.map((e) => {
              const ref = involvedRef(e)
              const label = `${e.involved_object_kind}/${e.involved_object_name}`
              return (
                <li
                  key={eventKey(e)}
                  className="flex flex-wrap items-baseline gap-x-2 px-2 py-1 text-xs"
                >
                  <StatusBadge status={isWarning(e) ? 'Degraded' : 'Ready'} label={e.event_type} />
                  <span className="font-semibold">{e.reason}</span>
                  {ref ? (
                    <button
                      type="button"
                      onClick={() => openInspector(ref, paneId)}
                      aria-label={`Open ${label}`}
                      className="min-h-[var(--touch-target)] font-mono text-accent-neon hover:underline sm:min-h-[24px]"
                    >
                      {label}
                    </button>
                  ) : (
                    <span className="font-mono">{label}</span>
                  )}
                  <span className="min-w-0 flex-1 basis-40 break-words text-muted-foreground">
                    {e.message}
                  </span>
                  <span className="text-muted-foreground">
                    {relativeTime(e.last_timestamp || e.first_timestamp, now)}
                  </span>
                </li>
              )
            })}
          </ul>
        )}
      </section>
    </div>
  )
}

function Stat({
  label,
  value,
  bad,
  note,
}: {
  label: string
  value: number
  bad?: number
  note?: string
}) {
  return (
    <div className="border border-border bg-card p-2" data-stat={label.toLowerCase()}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="font-mono text-lg">{value}</dd>
      {note && <dd className="text-xs text-muted-foreground">{note}</dd>}
      {bad !== undefined && (
        <dd className={bad > 0 ? 'text-xs text-status-warn' : 'text-xs text-muted-foreground'}>
          {bad > 0 ? `▲ ${String(bad)} unhealthy` : '● all healthy'}
        </dd>
      )}
    </div>
  )
}

/** 031: a count the identity may not read (RBAC 403). */
function NotPermittedStat({ label }: { label: string }) {
  return (
    <div
      className="border border-border bg-card p-2"
      data-stat={label.toLowerCase()}
      data-state="not-permitted"
    >
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="font-mono text-lg text-muted-foreground">—</dd>
      <dd className="text-xs text-muted-foreground">not permitted</dd>
    </div>
  )
}
