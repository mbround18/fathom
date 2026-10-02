/** 027-fleet-rollup-dashboard: one context's rollup card. Clicking switches the active context
 * through the existing `selectContext` action (FR-007) — no new switch mechanism.
 *
 * 046-fleet-model US1 (T028): the card also carries the member's identity. The display name is
 * shown *beside* the context string, never instead of it (FR-002) — the context name is what
 * everything is keyed on and what the operator pastes into `kubectl`, so it stays whole and
 * copyable verbatim. The member's labels render here because this is where a cluster is
 * identified. An `Absent` member — its context has left the kubeconfig — reads as unavailable
 * with every byte of its metadata intact; only `ForgetFleetMember` removes it (FR-009).
 */
import { useState } from 'react'

import type { FleetContextSummary, FleetHealthState } from '@/protocol/types'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { FleetMemberEditor } from './FleetMemberEditor'
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
  const member = useStore((s) => s.session.fleetMembers[summary.context])
  const [editing, setEditing] = useState(false)

  const { health } = summary
  const flux = summary.flux === 'Synced' || summary.flux === 'OutOfSync' ? summary.flux : null
  const updated = health === 'Pending' ? null : formatUpdatedAgo(summary.last_updated, now)
  const showCounts = health === 'Healthy' || health === 'Degraded'
  const noAccess = (group: string) => summary.restricted.includes(group)

  const availability = member?.availability ?? 'Present'
  const absent = availability === 'Absent'
  const labels = Object.entries(member?.labels ?? {})
  const copy = () => void navigator.clipboard.writeText(summary.context).catch(() => undefined)

  return (
    // No fixed width: the card is a grid item and reflows with the dashboard's
    // `grid-cols-1 sm:grid-cols-2 lg:grid-cols-3` (Principle IV).
    <div
      data-testid="fleet-card"
      data-context-name={summary.context}
      data-availability={availability}
      className={cn(
        'flex min-w-0 flex-col rounded-md border bg-background/30',
        absent && 'opacity-70',
        active ? 'border-accent-neon/60' : 'border-border/60',
      )}
    >
      <button
        type="button"
        data-testid="fleet-context-card"
        data-context-name={summary.context}
        data-health={health}
        aria-pressed={active}
        onClick={() => onSelect(summary.context)}
        className="flex min-h-[var(--touch-target)] min-w-0 flex-col gap-1.5 rounded-md p-3 text-left"
      >
        <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          {member?.display_name && (
            <span data-testid="fleet-display-name" className="truncate text-sm font-medium">
              {member.display_name}
            </span>
          )}
          <span
            data-testid="fleet-tier"
            data-tier={summary.live ? 'live' : 'polled'}
            className="ml-auto shrink-0 text-xs text-muted-foreground"
          >
            {summary.live ? 'Live' : 'Polled'}
          </span>
          {absent && (
            <span
              data-testid="fleet-availability"
              className="shrink-0 rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground"
            >
              Unavailable
            </span>
          )}
          <span
            data-testid="fleet-health-badge"
            className={cn('shrink-0 rounded border px-1.5 py-0.5 text-xs', HEALTH_CLASS[health])}
          >
            {HEALTH_LABEL[health]}
          </span>
        </span>
        {/* The key, verbatim. Truncation is CSS; the text node is the whole context name. */}
        <span
          data-testid="fleet-context-string"
          title={summary.context}
          className="truncate font-mono text-xs text-muted-foreground"
        >
          {summary.context}
        </span>
        {summary.error && (
          <span
            data-testid="fleet-card-error"
            className="text-xs break-words text-red-400"
            title={summary.plugin_error?.hint ?? undefined}
          >
            {summary.error}
          </span>
        )}
        {showCounts && (
          <dl className="grid grid-cols-3 gap-2 text-xs text-muted-foreground">
            <div>
              <dt>Nodes</dt>
              <dd data-testid="fleet-nodes" className="text-foreground">
                {noAccess('nodes')
                  ? 'no access'
                  : `${summary.node_ready_count}/${summary.node_total_count}`}
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
      {labels.length > 0 && (
        <ul data-testid="fleet-member-labels" className="flex flex-wrap gap-1 px-3 pb-2">
          {labels.map(([k, v]) => (
            <li
              key={k}
              data-testid="fleet-member-label"
              data-label-key={k}
              className="rounded border border-border/60 px-1.5 py-0.5 font-mono text-xs text-muted-foreground"
            >
              {k}={v}
            </li>
          ))}
        </ul>
      )}
      {/* Outside the card button: a button may not nest inside a button. */}
      <span className="flex items-center gap-2 px-3 pb-2 text-xs">
        <button
          type="button"
          data-testid="fleet-context-copy"
          aria-label={`Copy context name ${summary.context}`}
          onClick={copy}
          className="text-muted-foreground hover:text-foreground"
        >
          Copy context
        </button>
        <button
          type="button"
          data-testid="fleet-member-edit"
          aria-expanded={editing}
          aria-label={`Name and label ${summary.context}`}
          onClick={() => setEditing((e) => !e)}
          className="text-muted-foreground hover:text-foreground"
        >
          {editing ? 'Done' : 'Name and label'}
        </button>
      </span>
      {editing && <FleetMemberEditor context={summary.context} member={member} />}
    </div>
  )
}
