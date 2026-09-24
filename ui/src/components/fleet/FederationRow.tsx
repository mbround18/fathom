/** 034-simultaneous-federation: one federated context's live counters. Numbers are shown only
 * for `Live`; `Reconnecting` marks last-known values stale; failures show the verbatim error and
 * never a stale number as live. */
import type { FederationConnectionState, FederationContextRollup } from '@/protocol/types'
import { cn } from '@/lib/utils'

const STATE_LABEL: Record<FederationConnectionState, string> = {
  Pending: 'Connecting',
  Live: 'Live',
  Reconnecting: 'Reconnecting',
  Unreachable: 'Unreachable',
  AuthError: 'Auth error',
}

const STATE_CLASS: Record<FederationConnectionState, string> = {
  Pending: 'border-border text-muted-foreground',
  Live: 'border-emerald-500/50 text-emerald-400',
  Reconnecting: 'border-amber-500/60 text-amber-400',
  Unreachable: 'border-red-500/60 text-red-400',
  AuthError: 'border-fuchsia-500/60 text-fuchsia-400',
}

function metric(
  rollup: FederationContextRollup,
  group: string,
  value: number | null,
  fmt: (n: number) => string = String,
): string {
  if (rollup.restricted.includes(group)) return 'no access'
  return value === null ? '-' : fmt(value)
}

export function FederationRow({
  rollup,
  toggle,
}: {
  rollup: FederationContextRollup
  toggle: React.ReactNode
}) {
  const { state } = rollup
  const showNumbers = state === 'Live' || state === 'Reconnecting'
  const stale = state === 'Reconnecting'
  return (
    <div
      data-testid="federation-row"
      data-context-name={rollup.context}
      data-state={state}
      className="flex flex-col gap-1.5 rounded-md border border-border/60 bg-background/30 p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate font-mono text-sm">{rollup.context}</span>
        <span className="flex shrink-0 items-center gap-2">
          <span
            data-testid="federation-state"
            className={cn('rounded border px-1.5 py-0.5 text-xs', STATE_CLASS[state])}
          >
            {STATE_LABEL[state]}
          </span>
          {toggle}
        </span>
      </div>
      {rollup.error && (
        <span data-testid="federation-error" className="text-xs break-words text-red-400">
          {rollup.error}
        </span>
      )}
      {showNumbers && (
        <dl
          data-stale={stale || undefined}
          className={cn(
            'grid grid-cols-2 gap-2 text-xs text-muted-foreground sm:grid-cols-4',
            stale && 'opacity-60',
          )}
        >
          <div>
            <dt>Nodes</dt>
            <dd data-testid="federation-nodes" className="text-foreground">
              {rollup.restricted.includes('nodes')
                ? 'no access'
                : rollup.nodes_ready === null || rollup.nodes_total === null
                  ? '-'
                  : `${rollup.nodes_ready}/${rollup.nodes_total}`}
            </dd>
          </div>
          <div>
            <dt>Pods running</dt>
            <dd data-testid="federation-pods-running" className="text-foreground">
              {metric(rollup, 'pods', rollup.pods_running)}
            </dd>
          </div>
          <div>
            <dt>Unhealthy pods</dt>
            <dd data-testid="federation-pods-unhealthy" className="text-foreground">
              {metric(rollup, 'pods', rollup.pods_unhealthy)}
            </dd>
          </div>
          <div>
            <dt>Warnings (1h)</dt>
            <dd data-testid="federation-warnings" className="text-foreground">
              {metric(rollup, 'events', rollup.warning_events_1h)}
            </dd>
          </div>
        </dl>
      )}
      {stale && (
        <span data-testid="federation-stale" className="text-xs text-amber-400">
          Showing last-known values while reconnecting
        </span>
      )}
    </div>
  )
}
