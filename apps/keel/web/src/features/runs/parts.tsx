/** Small pieces the run pages share (spec 055 US2, T072). */
import type { HostPhase, RunPhase } from '@/protocol/wire'
import { ErrorLine as Alert } from '@/ui/ErrorLine'
import { STATUS } from '@/ui/form'

import { useRuns } from './store'

function tone(state: RunPhase | HostPhase): string {
  switch (state) {
    case 'succeeded':
      return 'text-status-ok'
    case 'failed':
    case 'refused':
    case 'denied':
    case 'lost':
      return 'text-status-err'
    case 'running':
    case 'sent':
      return 'text-accent-neon'
    default:
      return 'text-muted-foreground'
  }
}

export function StateBadge({ state }: { state: RunPhase | HostPhase }) {
  return <span className={`${STATUS} ${tone(state)}`}>{state.replace(/_/g, ' ')}</span>
}

export function ErrorLine() {
  const error = useRuns((s) => s.error)
  const clear = useRuns((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}
