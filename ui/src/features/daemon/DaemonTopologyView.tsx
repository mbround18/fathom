/**
 * 036 US5: topology fed by the in-cluster daemon. Rendered as an accessible grouped list with
 * relations and per-object trend lines (mobile-first, keyboard/touch friendly); the topology
 * pane uses the existing client-built graph whenever the daemon is off/absent.
 */
import { lazy, Suspense, useEffect, useMemo, useState, useSyncExternalStore } from 'react'

import { cn } from '@/lib/utils'
import type { TopologyHealth, TopologyNode } from '@/protocol/types'
import { useStore } from '@/store'
import { metricKey } from '@/store/handlers/daemon'

import { Sparkline } from './Sparkline'
import { useActiveDaemon } from './state'

const HEALTH: Record<TopologyHealth, { glyph: string; label: string; cls: string }> = {
  ok: { glyph: '●', label: 'OK', cls: 'text-status-ok' },
  warning: { glyph: '▲', label: 'Warning', cls: 'text-status-warn' },
  error: { glyph: '✕', label: 'Error', cls: 'text-status-err' },
  unknown: { glyph: '○', label: 'Unknown', cls: 'text-muted-foreground' },
}

const REL: Record<string, string> = {
  owns: 'owns',
  selects: 'selects',
  scheduledOn: 'scheduled on',
  scales: 'scales',
}

const CytoscapeGraph = lazy(() => import('./CytoscapeGraph'))
const WIDE = '(min-width: 768px)'
function useWide(): boolean {
  return useSyncExternalStore(
    (cb) => {
      if (typeof matchMedia !== 'function') return () => undefined
      const m = matchMedia(WIDE)
      m.addEventListener('change', cb)
      return () => {
        m.removeEventListener('change', cb)
      }
    },
    () => typeof matchMedia === 'function' && matchMedia(WIDE).matches,
    () => false,
  )
}

const nodeLabel = (n: TopologyNode): string => `${n.namespace ? `${n.namespace}/` : ''}${n.name}`

export function DaemonTopologyView() {
  const { context, daemon } = useActiveDaemon()
  const requestMetrics = useStore((s) => s.requestDaemonMetrics)
  const [selected, setSelected] = useState<string | null>(null)
  const wide = useWide()
  const nodes = daemon.topology.nodes
  const caps = daemon.status?.capabilities
  const metricsOk = caps?.metrics === 'available'

  const grouped = useMemo(() => {
    const byKind = new Map<string, TopologyNode[]>()
    for (const n of Object.values(nodes)) {
      const list = byKind.get(n.kind) ?? []
      list.push(n)
      byKind.set(n.kind, list)
    }
    return [...byKind.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(
        ([kind, list]) =>
          [kind, list.sort((a, b) => nodeLabel(a).localeCompare(nodeLabel(b)))] as const,
      )
  }, [nodes])

  const sel = selected ? nodes[selected] : undefined
  const canTrend = sel !== undefined && (sel.kind === 'Pod' || sel.kind === 'Node') && metricsOk

  useEffect(() => {
    if (!canTrend || !context || !selected) return
    requestMetrics(context, selected, 'cpu')
    requestMetrics(context, selected, 'memory')
  }, [canTrend, context, selected, requestMetrics])

  const relations = sel
    ? daemon.topology.edges.flatMap((e) => {
        if (e.from === sel.uid) {
          const other = nodes[e.to]
          return other ? [{ dir: REL[e.relation] ?? e.relation, other }] : []
        }
        if (e.to === sel.uid) {
          const other = nodes[e.from]
          return other ? [{ dir: `${REL[e.relation] ?? e.relation} (by)`, other }] : []
        }
        return []
      })
    : []

  return (
    <div
      className="flex h-full min-h-0 w-full min-w-0 flex-col md:flex-row"
      data-testid="daemon-topology"
      data-source="daemon"
    >
      {wide && Object.keys(nodes).length > 0 && (
        <div className="min-h-0 min-w-0 md:flex-[2]">
          <Suspense
            fallback={<p className="p-3 text-xs text-muted-foreground">Loading graph...</p>}
          >
            <CytoscapeGraph
              nodes={nodes}
              edges={daemon.topology.edges}
              selected={selected}
              onSelect={setSelected}
            />
          </Suspense>
        </div>
      )}
      <div className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
        <p className="border-b border-border px-3 py-1 text-xs text-muted-foreground">
          Live topology from the in-cluster daemon ({Object.keys(nodes).length} objects)
        </p>
        {grouped.map(([kind, list]) => (
          <section key={kind} aria-label={kind}>
            <h3 className="px-3 pt-2 font-display text-xs font-semibold tracking-wide uppercase">
              {kind} <span className="text-muted-foreground">({list.length})</span>
            </h3>
            <ul>
              {list.map((n) => {
                const h = HEALTH[n.health]
                return (
                  <li key={n.uid}>
                    <button
                      type="button"
                      data-testid="daemon-topology-node"
                      data-uid={n.uid}
                      data-health={n.health}
                      aria-pressed={selected === n.uid}
                      onClick={() => {
                        setSelected(n.uid)
                      }}
                      className={cn(
                        'flex min-h-[var(--touch-target)] w-full min-w-0 items-center gap-2 px-3 text-left text-sm hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none',
                        selected === n.uid && 'bg-muted',
                      )}
                    >
                      <span className={cn('flex shrink-0 items-center gap-1 text-xs', h.cls)}>
                        <span aria-hidden="true">{h.glyph}</span>
                        <span>{h.label}</span>
                      </span>
                      <span className="min-w-0 truncate">{nodeLabel(n)}</span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </section>
        ))}
      </div>
      {sel && (
        <aside
          className="min-w-0 shrink-0 overflow-y-auto border-t border-border p-3 md:w-72 md:border-t-0 md:border-l"
          aria-label={`Details for ${sel.name}`}
          data-testid="daemon-topology-detail"
        >
          <h3 className="font-semibold break-words">
            {sel.kind} {nodeLabel(sel)}
          </h3>
          <ul className="mt-2 flex flex-col gap-1 text-xs" data-testid="daemon-topology-relations">
            {relations.length === 0 && (
              <li className="text-muted-foreground">No visible relations.</li>
            )}
            {relations.map((r) => (
              <li key={`${r.dir}-${r.other.uid}`} className="break-words">
                <span className="text-muted-foreground">{r.dir}</span> {r.other.kind}{' '}
                {nodeLabel(r.other)}
              </li>
            ))}
          </ul>
          {canTrend && (
            <div className="mt-3 flex flex-col gap-2">
              {(['cpu', 'memory'] as const).map((m) => {
                const series = daemon.metrics[metricKey(sel.uid, m)]
                return series ? (
                  <Sparkline
                    key={m}
                    label={m === 'cpu' ? 'CPU' : 'Memory'}
                    samples={series.samples}
                    truncatedOldestTime={series.truncatedOldestTime}
                  />
                ) : (
                  <p key={m} className="text-xs text-muted-foreground">
                    Loading {m} trend
                  </p>
                )
              })}
            </div>
          )}
          {!metricsOk && (sel.kind === 'Pod' || sel.kind === 'Node') && (
            <p
              className="mt-3 text-xs text-muted-foreground"
              data-testid="daemon-metrics-unavailable"
            >
              Trend history unavailable: the cluster has no metrics API.
            </p>
          )}
        </aside>
      )}
    </div>
  )
}
