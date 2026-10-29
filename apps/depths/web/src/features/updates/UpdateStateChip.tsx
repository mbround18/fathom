import type { UpdateState } from '@/protocol/generated'

import { STATE_LABEL, STATE_TONE } from './model'

/** A service's update state as text and tone, like the stack status badge (WCAG 1.4.1). */
export function UpdateStateChip({ state }: { state: UpdateState }) {
  return (
    <span
      data-update-state={state}
      className={`inline-flex h-5 shrink-0 items-center border px-1.5 text-xs whitespace-nowrap ${STATE_TONE[state]}`}
    >
      {STATE_LABEL[state]}
    </span>
  )
}
