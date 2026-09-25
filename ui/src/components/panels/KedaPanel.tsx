/**
 * KEDA Autoscalers tool pane (029-keda-autoscaler-pane): lists ScaledObjects/ScaledJobs from
 * `store.keda.overview` (SystemEvent::KedaOverviewUpdated, 20s poll) and shows the selected row's
 * Inspector in place. Read-only.
 */
import { GaugeIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'

import { cn } from '@/lib/utils'
import type { KedaResourceSummary } from '@/protocol/types'
import { KedaInspector } from '@/resources/keda'
import { useStore } from '@/store'

import { MetricsUnavailable } from '../shell/PaneStates'

function Row({ row, selected }: { row: KedaResourceSummary; selected: boolean }) {
  const select = useStore((s) => s.selectKedaResource)
  return (
    <button
      type="button"
      data-testid="keda-row"
      aria-pressed={selected}
      onClick={() => {
        select(row.kind, row.namespace, row.name)
      }}
      className={cn(
        'flex w-full flex-col gap-0.5 rounded-sm border border-border/60 bg-card/60 px-2 py-1.5 text-left',
        selected && 'border-primary',
      )}
    >
      <span className="truncate font-mono text-xs">
        {row.kind} {row.namespace}/{row.name}
      </span>
      <span className="text-xs text-muted-foreground">
        {row.target_kind}/{row.target_name} · {String(row.current_replicas)} replicas (min{' '}
        {String(row.min_replicas)}, max {String(row.max_replicas)})
        {row.paused ? ' · paused' : ''}
      </span>
    </button>
  )
}

export function KedaPanel() {
  const overview = useStore((s) => s.keda.overview)
  const selected = useStore((s) => s.keda.selected)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const refreshKeda = useStore((s) => s.refreshKeda)

  useEffect(() => {
    refreshKeda()
  }, [refreshKeda])

  if (!overview) return <MetricsUnavailable detail="Waiting for the KEDA scan to complete." />
  if (!overview.installed) {
    return <MetricsUnavailable detail="KEDA not detected: its CRDs were not found in this cluster." />
  }

  const inScope = (r: KedaResourceSummary) => activeNamespace === null || r.namespace === activeNamespace
  const rows = [...overview.scaled_objects, ...overview.scaled_jobs].filter(inScope)

  if (overview.scaled_objects.length === 0 && overview.scaled_jobs.length === 0) {
    return (
      <div data-testid="keda-empty" className="p-3 text-sm text-muted-foreground">
        <GaugeIcon aria-hidden="true" className="mb-1 size-5" />
        KEDA is installed, but no autoscalers are configured.
      </div>
    )
  }

  return (
    <div data-testid="keda-panel" className="flex flex-col gap-2 p-2">
      {rows.length === 0 && (
        <p className="text-xs text-muted-foreground">No autoscalers in the selected namespace.</p>
      )}
      {rows.map((r) => (
        <Row
          key={`${r.kind}/${r.namespace}/${r.name}`}
          row={r}
          selected={
            selected?.state === 'ready' &&
            selected.details.summary.kind === r.kind &&
            selected.details.summary.namespace === r.namespace &&
            selected.details.summary.name === r.name
          }
        />
      ))}
      {selected?.state === 'ready' && <KedaInspector details={selected.details} />}
      {selected?.state === 'gone' && (
        <p data-testid="keda-gone" className="text-xs text-muted-foreground">
          {selected.kind} {selected.namespace}/{selected.name} no longer exists.
        </p>
      )}
    </div>
  )
}
