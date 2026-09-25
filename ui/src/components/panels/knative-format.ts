import type { KnativeReadyState, KnativeRevisionSummary } from '@/protocol/types'

export const KNATIVE_READY_TONE: Record<KnativeReadyState, 'ok' | 'err' | 'muted'> = {
  Ready: 'ok',
  NotReady: 'err',
  Unknown: 'muted',
}

/** Human text for a Revision's autoscaler state; never fabricates a reason when unknown. */
export function scaleReasonText(r: KnativeRevisionSummary): string {
  const reason = r.scale_reason
  switch (reason.kind) {
    case 'AtDesiredScale':
      return 'At desired scale'
    case 'ScaledToZero':
      return reason.data.idle_since
        ? `Scaled to zero: no traffic (idle since ${reason.data.idle_since})`
        : 'Scaled to zero: no traffic'
    case 'ScalingUp':
      return reason.data.pending_requests !== null
        ? `Scaling up (${String(reason.data.pending_requests)} pending requests)`
        : 'Scaling up'
    case 'Unavailable':
      return `Reason unavailable (actual ${String(r.actual_replicas)} / desired ${String(r.desired_replicas)})`
  }
}
