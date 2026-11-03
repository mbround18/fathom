/**
 * Times as people read them (Keel's approval and build panels, Fathom's ages, Depths' stack
 * times): relative in the text ("in 23 hours", "5 minutes ago"), the exact UTC time in the title,
 * and the machine-readable value in `dateTime`. `now` is a prop wherever it matters, so tests and
 * fixtures are deterministic.
 */
import { useEffect, useState } from 'react'

const RELATIVE = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })
const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['day', 86_400],
  ['hour', 3_600],
  ['minute', 60],
]

/** Milliseconds since the epoch for an RFC 3339 string, a number or a Date; NaN when unreadable. */
export function toMillis(at: string | number | Date): number {
  if (typeof at === 'number') return at
  if (at instanceof Date) return at.getTime()
  return Date.parse(at)
}

/** `at` against `now`: "in 23 hours", "5 minutes ago", "just now"; "" when `at` is unreadable. */
export function formatRelative(at: string | number | Date, now: number = Date.now()): string {
  const ms = toMillis(at)
  if (Number.isNaN(ms)) return ''
  const secs = Math.round((ms - now) / 1000)
  for (const [unit, size] of UNITS) {
    if (Math.abs(secs) >= size) return RELATIVE.format(Math.trunc(secs / size), unit)
  }
  return 'just now'
}

/** "2026-11-02 14:05 UTC"; "" when `at` is unreadable. */
export function formatUtc(at: string | number | Date): string {
  const ms = toMillis(at)
  if (Number.isNaN(ms)) return ''
  const iso = new Date(ms).toISOString()
  return `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`
}

/** The current time, refreshed every `everyMs` (a minute by default) while mounted. */
export function useNow(everyMs = 60_000): number {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const t = setInterval(() => {
      setNow(Date.now())
    }, everyMs)
    return () => {
      clearInterval(t)
    }
  }, [everyMs])
  return now
}

/**
 * A point in time. `relative` shows "5 minutes ago" and keeps it fresh; otherwise the UTC time.
 * The exact UTC time is always in the title. An unreadable value says "unknown".
 */
export function Time({ at, relative = false, now }: { at: string | number | Date; relative?: boolean; now?: number }) {
  if (Number.isNaN(toMillis(at))) return <span className="text-muted-foreground">unknown</span>
  return relative ? <RelativeTime at={at} now={now} /> : <AbsoluteTime at={at} />
}

function AbsoluteTime({ at }: { at: string | number | Date }) {
  const utc = formatUtc(at)
  return (
    <time dateTime={new Date(toMillis(at)).toISOString()} title={utc} className="tabular-nums">
      {utc}
    </time>
  )
}

function RelativeTime({ at, now }: { at: string | number | Date; now?: number | undefined }) {
  const ticking = useNow()
  return (
    <time dateTime={new Date(toMillis(at)).toISOString()} title={formatUtc(at)} className="tabular-nums">
      {formatRelative(at, now ?? ticking)}
    </time>
  )
}
