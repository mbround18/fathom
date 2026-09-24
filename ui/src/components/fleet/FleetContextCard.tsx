/** 027-fleet-rollup-dashboard: one context's rollup card. Clicking switches the active context
 * through the existing `selectContext` action (FR-007) — no new switch mechanism. */
import type { FleetContextSummary, FleetHealthState } from '@/protocol/types'
import { cn } from '@/lib/utils'

import { formatUpdatedAgo } from './relative-time'

const HEALTH_LABEL: Record<FleetHealthState, string> = {
  Healthy: 'Healthy',
  Degraded: 'Degraded',
  Unreachable: 'Unreachable',
  AuthError: 'Auth error',
  Pending: 'Pending',
}

const HEALTH_CLASS: Record<FleetHealthState, string> = {
  Healthy: 'border-emerald-500/50 text-emerald-400',
  Degraded: 'border-amber-500/60 text-amber-400',
  Unreachable: 'border-red-500/60 text-red-400',
  AuthError: 'border-fuchsia-500/60 text-fuchsia-400',
  Pending: 'border-border text-muted-foreground',
}

const FLUX_LABEL = { Synced: 'Flux: synced', OutOfSync: 'Flux: out of sync' } as const

export function FleetContextCard({
  summary,
  active,
  onSelect,
  now,
}: {
  summary: FleetContextSummary
  active: boolean
  onSelect: (context: string) => void
  /** Epoch ms, ticked by the dashboard view. */
  now: number
}) {
  const { health } = summary
  const flux = summary.flux === 'Synced' || summary.flux === 'OutOfSync' ? summary.flux : null
  const updated = health === 'Pending' ? null : formatUpdatedAgo(summary.last_updated, now)
  const showCounts = health === 'Healthy' || health === 'Degraded'
  const noAccess = (group: string) => summary.restricted.includes(group)
  return (
    <button
      type="button"
      data-testid="fleet-context-card"
      data-context-name={summary.context}
      data-health={health}
      aria-pressed={active}
      onClick={() => onSelect(summary.context)}
      className={cn(
        'flex min-h-[var(--touch-target)] flex-col gap-1.5 rounded-md border bg-background/30 p-3 text-left',
        active ? 'border-accent-neon/60' : 'border-border/60 hover:border-accent-neon/40',
      )}
    >
      <span className="flex items-center justify-between gap-2">
        <span className="truncate font-mono text-sm">{summary.context}</span>
        <span
          data-testid="fleet-health-badge"
          className={cn('shrink-0 rounded border px-1.5 py-0.5 text-xs', HEALTH_CLASS[health])}
        >
          {HEALTH_LABEL[health]}
        </span>
      </span>
      {summary.error && (
        <span data-testid="fleet-card-error" className="text-xs break-words text-red-400" title={summary.plugin_error?.hint ?? undefined}>
          {summary.error}
        </span>
      )}
      {showCounts && (
        <dl className="grid grid-cols-3 gap-2 text-xs text-muted-foreground">
          <div>
            <dt>Nodes</dt>
            <dd data-testid="fleet-nodes" className="text-foreground">
              {noAccess('nodes') ? 'no access' : `${summary.node_ready_count}/${summary.node_total_count}`}
            </dd>
          </div>
          <div>
            <dt>Unhealthy pods</dt>
            <dd data-testid="fleet-unhealthy-pods" className="text-foreground">
              {noAccess('pods') ? 'no access' : summary.unhealthy_pod_count}
            </dd>
          </div>
          <div>
            <dt>Version</dt>
            <dd data-testid="fleet-version" className="text-foreground">
              {summary.cluster_version}
            </dd>
          </div>
        </dl>
      )}
      {flux && (
        <span
          data-testid="fleet-flux"
          data-flux={flux}
          className={cn('text-xs', flux === 'Synced' ? 'text-emerald-400' : 'text-amber-400')}
        >
          {FLUX_LABEL[flux]}
        </span>
      )}
      {updated && (
        <span data-testid="fleet-updated" className="text-xs text-muted-foreground">
          {updated}
        </span>
      )}
    </button>
  )
}
