/**
 * KEDA Autoscalers Inspector (029-keda-autoscaler-pane): renders a selected ScaledObject/ScaledJob's
 * triggers (type, generic key/value metadata, optional live value next to its threshold) and the
 * backend-derived plain-language scale reasoning. Read-only; auth refs show a name only, never a
 * secret value.
 */
import type { KedaResourceDetails, KedaTrigger, ScaleReasoningState } from '@/protocol/types'

const REASON_LABEL: Record<ScaleReasoningState, string> = {
  AtFloor: 'At floor',
  AtCeiling: 'At ceiling',
  MidRange: 'Mid-range',
  Paused: 'Paused',
  ScaledToZero: 'Scaled to zero',
  TargetNotFound: 'Target missing',
}

/** "prometheus" -> "Prometheus". */
function triggerTypeLabel(type: string): string {
  return type ? type.charAt(0).toUpperCase() + type.slice(1) : 'Unknown'
}

function TriggerCard({ trigger, index }: { trigger: KedaTrigger; index: number }) {
  const meta = Object.entries(trigger.metadata)
  return (
    <div
      data-testid={`keda-trigger-${String(index)}`}
      className="rounded-sm border border-border/60 bg-card/60 px-2 py-1.5"
    >
      <div className="font-mono text-xs">{triggerTypeLabel(trigger.trigger_type)}</div>
      {trigger.current_value != null && (
        <div data-testid="keda-trigger-live" className="text-xs">
          current: {String(trigger.current_value)}
          {trigger.threshold != null && ` / threshold: ${trigger.threshold}`}
        </div>
      )}
      {trigger.current_value == null && trigger.threshold != null && (
        <div className="text-xs text-muted-foreground">threshold: {trigger.threshold}</div>
      )}
      {meta.length > 0 && (
        <dl className="mt-1 grid grid-cols-[minmax(6rem,max-content)_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-[0.7rem]">
          {meta.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted-foreground">{k}</dt>
              <dd className="font-mono break-all">{v}</dd>
            </div>
          ))}
        </dl>
      )}
      {trigger.auth_ref && (
        <div className="mt-1 text-[0.7rem] text-muted-foreground">
          auth: {trigger.auth_ref.name}
          {trigger.auth_ref.cluster_scoped ? ' (cluster)' : ''}
        </div>
      )}
    </div>
  )
}

export function KedaInspector({ details }: { details: KedaResourceDetails }) {
  const { summary, triggers, reasoning } = details
  return (
    <div data-testid="keda-inspector" className="flex flex-col gap-2">
      <h4 className="font-mono text-xs">
        {summary.kind} {summary.namespace}/{summary.name}
      </h4>
      <div className="text-xs text-muted-foreground">
        {summary.target_kind}/{summary.target_name} · {String(summary.current_replicas)} replicas
        (min {String(summary.min_replicas)}, max {String(summary.max_replicas)})
      </div>
      <div data-testid="keda-reasoning" className="rounded-sm border border-border/60 px-2 py-1.5">
        <div className="text-[0.7rem] tracking-wider text-muted-foreground uppercase">
          {REASON_LABEL[reasoning.state]}
        </div>
        <div className="text-xs">{reasoning.message}</div>
      </div>
      <h5 className="font-display text-[0.7rem] tracking-wider text-muted-foreground uppercase">
        Triggers
      </h5>
      {triggers.length === 0 && <p className="text-xs text-muted-foreground">No triggers.</p>}
      {triggers.map((t, i) => (
        <TriggerCard key={`${t.trigger_type}-${String(i)}`} trigger={t} index={i} />
      ))}
    </div>
  )
}
