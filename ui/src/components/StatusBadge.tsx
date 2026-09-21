import { cn } from '@/lib/utils'
import { statusTone, type Tone } from '@/resources/search'

const TONE: Record<Tone, string> = {
  ok: 'text-status-ok',
  warn: 'text-status-warn',
  err: 'text-status-err',
  muted: 'text-muted-foreground',
}

/**
 * Glyph + word, never colour alone (FR-018). Inside a selected (inverse-video) row the tone
 * colour gives way to the row's foreground so it stays legible.
 */
export function StatusBadge({ status, label }: { status: string; label?: string }) {
  const { tone, glyph } = statusTone(status)
  return (
    <span
      data-tone={tone}
      className={cn(
        'inline-flex min-w-0 items-center gap-1.5',
        TONE[tone],
        'group-aria-selected/row:text-primary-foreground',
      )}
    >
      <span aria-hidden="true" className="shrink-0 text-[0.8em]">
        {glyph}
      </span>
      <span className="truncate">{label ?? status}</span>
    </span>
  )
}
