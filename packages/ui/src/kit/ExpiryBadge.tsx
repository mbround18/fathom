/**
 * When something stops working: a certificate's notAfter, a token's or a grant's expiry (spec
 * 059). The tone moves from ok to soon (inside the renewal window, 30 days by default) to urgent
 * (7 days) to expired, and the words and the icon carry it too, never the colour alone. The exact
 * UTC time is in the title.
 */
import { ClockIcon, SealCheckIcon, WarningIcon, XCircleIcon } from '@phosphor-icons/react'
import { cn } from '@companion/theme'

import { STATUS } from './form'
import { formatRelative, formatUtc, toMillis, useNow } from './time'

export type ExpiryTone = 'ok' | 'soon' | 'urgent' | 'expired'

export interface ExpiryWindow {
  /** Days before expiry that count as soon: the renewal window. */
  soonDays?: number
  /** Days before expiry that count as urgent. */
  urgentDays?: number
}

const DAY_MS = 86_400_000

/** Where `notAfter` falls against `now`; `null` when it is unreadable. */
export function expiryTone(
  notAfter: string | number | Date,
  now: number = Date.now(),
  { soonDays = 30, urgentDays = 7 }: ExpiryWindow = {},
): ExpiryTone | null {
  const ms = toMillis(notAfter)
  if (Number.isNaN(ms)) return null
  const left = ms - now
  if (left <= 0) return 'expired'
  if (left <= urgentDays * DAY_MS) return 'urgent'
  if (left <= soonDays * DAY_MS) return 'soon'
  return 'ok'
}

const LOOK: Record<ExpiryTone, { className: string; Icon: typeof ClockIcon }> = {
  ok: { className: 'text-status-ok', Icon: SealCheckIcon },
  soon: { className: 'text-status-warn', Icon: ClockIcon },
  urgent: { className: 'text-status-err', Icon: WarningIcon },
  expired: { className: 'text-status-err', Icon: XCircleIcon },
}

export function ExpiryBadge({
  notAfter,
  now,
  thresholds,
  label = 'Expires',
  pastLabel = 'Expired',
}: {
  notAfter: string | number | Date
  /** Fixes the clock, for tests and fixtures; otherwise it ticks every minute. */
  now?: number
  thresholds?: ExpiryWindow
  label?: string
  pastLabel?: string
}) {
  const ticking = useNow()
  const at = now ?? ticking
  const tone = expiryTone(notAfter, at, thresholds)
  if (tone === null) return <span className="text-xs text-muted-foreground">{label} unknown</span>
  const { className, Icon } = LOOK[tone]
  const utc = formatUtc(notAfter)
  return (
    <span data-expiry={tone} title={utc} className={cn(STATUS, 'inline-flex items-center gap-1', className)}>
      <Icon aria-hidden weight={tone === 'ok' ? 'regular' : 'bold'} className="size-3.5 shrink-0" />
      <time dateTime={new Date(toMillis(notAfter)).toISOString()}>
        {tone === 'expired' ? pastLabel : label} {formatRelative(notAfter, at)}
      </time>
    </span>
  )
}
