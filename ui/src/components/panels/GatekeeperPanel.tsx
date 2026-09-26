/**
 * OPA Gatekeeper policy pane (020-gatekeeper-policy-pane): renders `store.gatekeeper.overview`
 * (SystemEvent::GatekeeperOverviewUpdated) through `IntegrationSummary`. State arrives from a 20s
 * backend scan (see `sync.rs::ensure_gatekeeper_poll`); opening the pane also sends
 * `RefreshGatekeeper`. Read-only: selecting a constraint lists its violating resources, each of
 * which links out to the existing Inspector. Every install state renders its own message; a
 * failed scan (`ScanFailed`) renders the verbatim error with a Retry that shows "Scanning" again
 * until the next overview arrives.
 */
import { ShieldCheckIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'

import type { GatekeeperConstraintSummary } from '@/protocol/types'
import { useStore } from '@/store'

import { IntegrationSummary, type SummaryRow, type SummaryTile } from './IntegrationSummary'
import { ErrorState, MetricsUnavailable } from '../shell/PaneStates'

const constraintKey = (c: GatekeeperConstraintSummary) => `${c.kind}/${c.name}`

function countLabel(c: GatekeeperConstraintSummary): string {
  if (c.total_violations === null) return 'awaiting status'
  return `${String(c.total_violations)} ${c.total_violations === 1 ? 'violation' : 'violations'}`
}

function toneFor(c: GatekeeperConstraintSummary): SummaryRow['tone'] {
  if (c.total_violations === null) return 'muted'
  return c.total_violations === 0 ? 'ok' : 'err'
}

export function GatekeeperPanel() {
  const overview = useStore((s) => s.gatekeeper.overview)
  const refreshGatekeeper = useStore((s) => s.refreshGatekeeper)
  const openInspector = useStore((s) => s.openInspector)
  const [selected, setSelected] = useState<string | null>(null)
  // The failed overview a Retry was sent for: until a newer overview replaces it, the pane shows
  // "Scanning" again rather than the old error.
  const [retriedFrom, setRetriedFrom] = useState<typeof overview>(undefined)

  useEffect(() => {
    refreshGatekeeper()
  }, [refreshGatekeeper])

  if (!overview || overview === retriedFrom) {
    return (
      <MetricsUnavailable title="Scanning" detail="Waiting for the Gatekeeper scan to complete." />
    )
  }
  if (overview.state === 'ScanFailed') {
    return (
      <ErrorState
        title="Scan failed"
        detail={`Could not scan for Gatekeeper constraints: ${overview.error}`}
        onRetry={() => {
          setRetriedFrom(overview)
          refreshGatekeeper()
        }}
      />
    )
  }
  if (overview.state === 'NotInstalled') {
    return (
      <MetricsUnavailable
        title="Not installed"
        detail="OPA Gatekeeper is not installed on this cluster. Install OPA Gatekeeper to see constraints and violations here."
      />
    )
  }
  if (overview.state === 'PermissionDenied') {
    return (
      <MetricsUnavailable
        title="Not permitted"
        detail={`Not permitted to read Gatekeeper constraints: ${overview.error}`}
      />
    )
  }
  if (overview.state === 'InstalledNoConstraints') {
    return (
      <MetricsUnavailable
        title="No constraints"
        detail="Gatekeeper is installed but no constraints are defined. Add a ConstraintTemplate and a Constraint to see violations here."
      />
    )
  }

  const constraints = overview.constraints
  const withViolations = constraints.filter((c) => (c.total_violations ?? 0) > 0).length
  const totalViolations = constraints.reduce((n, c) => n + (c.total_violations ?? 0), 0)
  const tiles: SummaryTile[] = [
    { label: 'Constraints', value: String(constraints.length), tone: 'muted' },
    { label: 'Violating', value: String(withViolations), tone: withViolations > 0 ? 'err' : 'ok' },
    {
      label: 'Violations',
      value: String(totalViolations),
      tone: totalViolations > 0 ? 'err' : 'ok',
    },
  ]

  const rows: SummaryRow[] = constraints.map((c) => {
    const key = constraintKey(c)
    return {
      key,
      tone: toneFor(c),
      primary: (
        <button
          type="button"
          data-testid="gatekeeper-constraint"
          aria-pressed={selected === key}
          className="text-left hover:text-accent-neon hover:underline"
          onClick={() => {
            setSelected(selected === key ? null : key)
          }}
        >
          {c.name}
        </button>
      ),
      secondary: [
        c.kind,
        c.enforcement_action || 'enforcement unset',
        countLabel(c),
        c.template_missing ? 'references a missing ConstraintTemplate' : null,
      ]
        .filter(Boolean)
        .join(' · '),
    }
  })

  const active = constraints.find((c) => constraintKey(c) === selected)

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto">
      <IntegrationSummary
        icon={ShieldCheckIcon}
        title="Gatekeeper"
        tiles={tiles}
        rows={rows}
        emptyLabel="No constraints."
      />
      {active && (
        <div data-testid="gatekeeper-violations" className="flex flex-col gap-1 p-3 pt-0">
          <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">
            {active.name} violations
          </h4>
          {active.violations_truncated && active.total_violations !== null && (
            <p role="status" className="text-xs text-status-warn">
              showing {active.violations.length} of {active.total_violations} violations
            </p>
          )}
          {active.violations.length === 0 && (
            <p className="text-xs text-muted-foreground">No violations reported.</p>
          )}
          {active.violations.map((v, i) => (
            <button
              key={`${v.kind}/${v.namespace}/${v.name}/${String(i)}`}
              type="button"
              data-testid="gatekeeper-violation"
              className="rounded-sm border border-border/60 bg-card/60 px-2 py-1.5 text-left hover:border-accent-neon"
              onClick={() => {
                openInspector(
                  {
                    section: v.section,
                    kind: v.kind,
                    namespace: v.namespace || null,
                    name: v.name,
                  },
                  undefined,
                  true,
                )
              }}
            >
              <div className="truncate font-mono text-xs">
                {v.kind} {v.namespace ? `${v.namespace}/` : ''}
                {v.name}
              </div>
              <div className="text-xs text-muted-foreground">
                {v.enforcement_action} · {v.message}
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
