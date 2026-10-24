import type { StackStatus } from '@/protocol/generated'

import { STATUS_LABEL } from './model'

/**
 * Status as colour *and* text (WCAG 1.4.1): the label is always present, the dot and border
 * only reinforce it. Colours are the theme's contrast-tested status tokens.
 */
const TONE: Record<StackStatus, string> = {
  running: 'border-status-ok text-status-ok',
  partial: 'border-status-warn/60 text-status-warn',
  stopped: 'border-border text-muted-foreground',
  not_deployed: 'border-dashed border-border text-muted-foreground',
  invalid: 'border-status-err/60 text-status-err',
}

const DOT: Record<StackStatus, string> = {
  running: 'bg-status-ok',
  partial: 'bg-status-warn',
  stopped: 'bg-muted-foreground',
  not_deployed: 'border border-muted-foreground',
  invalid: 'bg-status-err',
}

export function StatusBadge({ status, unhealthy = false }: { status: StackStatus; unhealthy?: boolean }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1">
      <span
        data-status={status}
        className={`inline-flex h-5 items-center gap-1 border px-1.5 text-xs whitespace-nowrap ${TONE[status]}`}
      >
        <span aria-hidden className={`size-1.5 rounded-full ${DOT[status]}`} />
        {STATUS_LABEL[status]}
      </span>
      {unhealthy && (
        <span
          data-health="unhealthy"
          className="inline-flex h-5 items-center border border-status-err/60 px-1.5 text-xs whitespace-nowrap text-status-err"
        >
          Unhealthy
        </span>
      )}
    </span>
  )
}
