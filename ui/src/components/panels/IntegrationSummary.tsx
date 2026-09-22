/**
 * Reusable "integration summary" panel: a grid of stat tiles plus a list of rows, styled like
 * `PaneStates.tsx`'s `StateFrame` (uppercase display labels, mono values, tone colors). Used for
 * both the Flux and Hubble tool panes so they share one visual language.
 */
import type { Icon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import { cn } from '@/lib/utils'

export interface SummaryTile {
  label: string
  value: string
  tone?: 'ok' | 'warn' | 'err' | 'muted'
}

export interface SummaryRow {
  key: string
  primary: ReactNode
  secondary?: ReactNode
  tone?: 'ok' | 'warn' | 'err' | 'muted'
}

const TONE_TEXT: Record<NonNullable<SummaryTile['tone']>, string> = {
  ok: 'text-status-ok',
  warn: 'text-status-warn',
  err: 'text-status-err',
  muted: 'text-muted-foreground',
}

const TONE_DOT: Record<NonNullable<SummaryRow['tone']>, string> = {
  ok: 'bg-status-ok',
  warn: 'bg-status-warn',
  err: 'bg-status-err',
  muted: 'bg-muted-foreground',
}

export function IntegrationSummary({
  icon: IconCmp,
  title,
  tiles,
  rows,
  emptyLabel = 'Nothing to show yet.',
}: {
  icon?: Icon
  title: string
  tiles: SummaryTile[]
  rows: SummaryRow[]
  emptyLabel?: string
}) {
  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-3">
      <div className="flex items-center gap-2">
        {IconCmp && <IconCmp aria-hidden="true" className="size-4 text-accent-neon" />}
        <h3 className="font-display text-xs font-semibold tracking-wider uppercase">{title}</h3>
      </div>
      {tiles.length > 0 && (
        <div className="grid grid-cols-2 gap-2">
          {tiles.map((t) => (
            <div key={t.label} className="rounded-sm border border-border bg-card p-2">
              <div
                className={cn(
                  'font-mono text-lg font-semibold',
                  TONE_TEXT[t.tone ?? 'muted'],
                )}
              >
                {t.value}
              </div>
              <div className="font-display text-[0.65rem] tracking-wider text-muted-foreground uppercase">
                {t.label}
              </div>
            </div>
          ))}
        </div>
      )}
      <div className="flex min-h-0 flex-1 flex-col gap-1">
        {rows.length === 0 ? (
          <p className="text-xs text-muted-foreground">{emptyLabel}</p>
        ) : (
          rows.map((r) => (
            <div
              key={r.key}
              className="flex items-center gap-2 rounded-sm border border-border/60 bg-card/60 px-2 py-1.5"
            >
              <span
                aria-hidden="true"
                className={cn('size-2 shrink-0 rounded-full', TONE_DOT[r.tone ?? 'muted'])}
              />
              <div className="min-w-0 flex-1">
                <div className="truncate font-mono text-xs">{r.primary}</div>
                {r.secondary && (
                  <div className="truncate text-[0.7rem] text-muted-foreground">{r.secondary}</div>
                )}
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}
