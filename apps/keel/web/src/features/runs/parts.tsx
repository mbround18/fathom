/** Small pieces the run pages share (spec 055 US2, T072). */
import { ErrorLine as Alert, STATUS } from '@companion/ui/kit'

import type { HostPhase, RunPhase } from '@/protocol/wire'

import { useRuns } from './store'

function tone(state: RunPhase | HostPhase): string {
  switch (state) {
    case 'succeeded':
    case 'covered':
      return 'text-status-ok'
    case 'waiting_checks':
      return 'text-status-warn'
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

/** States whose code reads poorly with its underscores replaced (spec 068). */
const WORD: Partial<Record<RunPhase | HostPhase, string>> = {
  waiting_checks: 'waiting for checks',
}

/** A state, in words; `label` replaces them (spec 064: "Waiting · 1 of 2" for a held run). */
export function StateBadge({ state, label }: { state: RunPhase | HostPhase; label?: string | null }) {
  return <span className={`${STATUS} ${tone(state)}`}>{label ?? WORD[state] ?? state.replace(/_/g, ' ')}</span>
}

export function ErrorLine() {
  const error = useRuns((s) => s.error)
  const clear = useRuns((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}
