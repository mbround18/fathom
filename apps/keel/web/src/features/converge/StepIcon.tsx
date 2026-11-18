/**
 * A converge step's state as an icon in its tone (spec 068 contracts/ui.md): blocked is hollow,
 * running spins, the rest each have their own glyph. Decorative: the caller names the state in
 * text.
 */
import {
  CheckCircleIcon,
  CircleDashedIcon,
  CircleNotchIcon,
  ClockIcon,
  LinkSimpleIcon,
  MinusCircleIcon,
  ProhibitIcon,
  StopIcon,
  WarningCircleIcon,
  XCircleIcon,
  type Icon,
} from '@phosphor-icons/react'

import type { StepState } from '@/protocol/wire'

import { toneClass } from './derive'

const ICON: Record<StepState, Icon> = {
  blocked: CircleDashedIcon,
  queued: ClockIcon,
  sent: CircleNotchIcon,
  running: CircleNotchIcon,
  succeeded: CheckCircleIcon,
  failed: XCircleIcon,
  refused: ProhibitIcon,
  lost: WarningCircleIcon,
  cancelled: StopIcon,
  skipped: MinusCircleIcon,
  covered: LinkSimpleIcon,
}

export function StepIcon({ state }: { state: StepState }) {
  const Glyph = ICON[state]
  const spin = state === 'running' || state === 'sent'
  return (
    <Glyph
      aria-hidden
      weight={state === 'succeeded' || state === 'failed' ? 'fill' : 'regular'}
      className={`size-4 shrink-0 ${toneClass(state)} ${spin ? 'animate-spin motion-reduce:animate-none' : ''}`}
    />
  )
}
