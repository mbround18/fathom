/** Small pieces the run pages share (spec 055 US2, T072). */
import { Button } from '@companion/ui'

import type { HostPhase, RunPhase } from '@/protocol/wire'

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
  return <span className={`text-[11px] uppercase ${tone(state)}`}>{state.replace(/_/g, ' ')}</span>
}

export function ErrorLine() {
  const error = useRuns((s) => s.error)
  const clear = useRuns((s) => s.clearError)
  if (!error) return null
  return (
    <p role="alert" className="flex items-center justify-between gap-2 border border-status-err/40 px-2 py-1 text-xs text-status-err">
      <span>{error}</span>
      <Button variant="ghost" size="xs" onClick={clear}>
        Dismiss
      </Button>
    </p>
  )
}
