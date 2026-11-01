/**
 * A list that failed to load (spec 063 follow-up): "Failed to load {what}." and the reason as the
 * server sent it, with Retry. It stands in for the loading and empty states while the list is
 * failed, so a failure never reads as "nothing here yet". `data-pane-state="error"`.
 */
import { ArrowClockwiseIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'

import type { Refusal } from '@/app/load'

import { RefusalText } from './RefusalText'

export function LoadFailed({
  what,
  refusal,
  onRetry,
}: {
  what: string
  refusal: Refusal
  onRetry?: (() => void) | undefined
}) {
  return (
    <div data-pane-state="error" className="flex min-w-0 flex-col items-start gap-2">
      <RefusalText refusal={refusal} lead={`Failed to load ${what}.`} />
      {onRetry && (
        <Button type="button" variant="outline" size="xs" onClick={onRetry}>
          <ArrowClockwiseIcon aria-hidden />
          Retry
        </Button>
      )}
    </div>
  )
}
