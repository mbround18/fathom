/**
 * Knative Serving tool pane (028-knative-pane): renders `store.knative.overview`
 * (SystemEvent::KnativeOverviewUpdated) — Services, Revisions (with the autoscaler "why") and
 * Routes (with a traffic-split graph). Read-only. State arrives from a 20s background scan; opening
 * the pane also sends `RefreshKnative`.
 */
import { RocketLaunchIcon } from '@phosphor-icons/react'
import { useEffect, type ReactNode } from 'react'

import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { MetricsUnavailable } from '../shell/PaneStates'
import { KNATIVE_READY_TONE, scaleReasonText } from './knative-format'
import { KnativeTrafficGraph } from './KnativeTrafficGraph'

const DOT = { ok: 'bg-status-ok', err: 'bg-status-err', muted: 'bg-muted-foreground' } as const

function Section({ title, testId, children }: { title: string; testId: string; children: ReactNode }) {
  return (
    <section data-testid={testId} className="flex flex-col gap-1">
      <h4 className="font-display text-[0.7rem] tracking-wider text-muted-foreground uppercase">
        {title}
      </h4>
      {children}
    </section>
  )
}

function Row({
  tone,
  primary,
  secondary,
  testId,
  children,
}: {
  tone: keyof typeof DOT
  primary: string
  secondary?: string
  testId: string
  children?: ReactNode
}) {
  return (
    <div data-testid={testId} className="rounded-sm border border-border/60 bg-card/60 px-2 py-1.5">
      <div className="flex items-center gap-2">
        <span aria-hidden="true" className={cn('size-2 shrink-0 rounded-full', DOT[tone])} />
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{primary}</span>
      </div>
      {secondary && <div className="text-[0.7rem] text-muted-foreground">{secondary}</div>}
      {children}
    </div>
  )
}

export function KnativePanel() {
  const overview = useStore((s) => s.knative.overview)
  const refreshKnative = useStore((s) => s.refreshKnative)

  useEffect(() => {
    refreshKnative()
  }, [refreshKnative])

  if (!overview) {
    return <MetricsUnavailable detail="Waiting for the Knative scan to complete." />
  }
  if (!overview.installed) {
    return <MetricsUnavailable detail="Knative Serving CRDs were not found in this cluster." />
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-3" data-testid="knative-panel">
      <div className="flex items-center gap-2">
        <RocketLaunchIcon aria-hidden="true" className="size-4 text-accent-neon" />
        <h3 className="font-display text-xs font-semibold tracking-wider uppercase">Knative</h3>
      </div>

      <Section title={`Services (${String(overview.services.length)})`} testId="knative-services">
        {overview.services.map((s) => (
          <Row
            key={`${s.namespace}/${s.name}`}
            testId="knative-service-row"
            tone={KNATIVE_READY_TONE[s.ready]}
            primary={`${s.namespace}/${s.name}`}
            secondary={`${s.ready}${s.latest_ready_revision ? ` — latest ready ${s.latest_ready_revision}` : ''}`}
          />
        ))}
      </Section>

      <Section title={`Revisions (${String(overview.revisions.length)})`} testId="knative-revisions">
        {overview.revisions.map((r) => (
          <Row
            key={`${r.namespace}/${r.name}`}
            testId="knative-revision-row"
            tone={KNATIVE_READY_TONE[r.ready]}
            primary={`${r.namespace}/${r.name}`}
            secondary={`${r.ready} — replicas ${String(r.actual_replicas)}/${String(r.desired_replicas)} — traffic ${String(r.traffic_percent)}%`}
          >
            <div data-testid="knative-scale-reason" className="text-[0.7rem]">
              {scaleReasonText(r)}
            </div>
          </Row>
        ))}
      </Section>

      <Section title={`Routes (${String(overview.routes.length)})`} testId="knative-routes">
        {overview.routes.map((r) => (
          <Row
            key={`${r.namespace}/${r.name}`}
            testId="knative-route-row"
            tone={KNATIVE_READY_TONE[r.ready]}
            primary={`${r.namespace}/${r.name}`}
            secondary={`${r.ready}${r.url ? ` — ${r.url}` : ''}`}
          >
            <KnativeTrafficGraph overview={overview} namespace={r.namespace} name={r.name} />
          </Row>
        ))}
      </Section>
    </div>
  )
}
