/**
 * Steps in order, with where things stand: a certificate's issuance chain (Fathom's cert-manager
 * view), a key rotation's overlap, a trust chain from leaf to root. A stuck stage is marked in
 * words and an icon as well as colour, with its reason under it. Stages with `onOpen` are buttons.
 */
import { CheckCircleIcon, CircleIcon, CircleNotchIcon, WarningOctagonIcon } from '@phosphor-icons/react'
import { cn } from '@companion/theme'
import type { ReactNode } from 'react'

export type StageState = 'done' | 'current' | 'pending' | 'stuck'

export interface Stage {
  key: string
  title: ReactNode
  /** A short state after the title: "Ready", "Pending", "Issued 3 days ago". */
  status?: ReactNode
  state: StageState
  /** Why it is stuck, or anything else worth a line under the title. */
  note?: ReactNode
  onOpen?: () => void
}

const ICON: Record<StageState, { Icon: typeof CircleIcon; className: string; word: string }> = {
  done: { Icon: CheckCircleIcon, className: 'text-status-ok', word: 'Done' },
  current: { Icon: CircleNotchIcon, className: 'text-primary', word: 'In progress' },
  pending: { Icon: CircleIcon, className: 'text-muted-foreground', word: 'Not started' },
  stuck: { Icon: WarningOctagonIcon, className: 'text-status-err', word: 'Stuck' },
}

export function StageList({ label, stages }: { label: string; stages: readonly Stage[] }) {
  return (
    <ol aria-label={label} className="flex min-w-0 flex-col gap-1">
      {stages.map((s) => {
        const { Icon, className, word } = ICON[s.state]
        const body = (
          <>
            <Icon aria-hidden weight={s.state === 'pending' ? 'regular' : 'bold'} className={cn('mt-0.5 size-3.5 shrink-0', className)} />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                <span className="min-w-0 font-mono break-all">{s.title}</span>
                {s.status !== undefined && <span className="text-muted-foreground">{s.status}</span>}
                <span className={cn('font-medium', s.state === 'stuck' ? 'text-status-err' : 'sr-only')}>{word}</span>
              </span>
              {s.note !== undefined && <span className="text-muted-foreground break-words">{s.note}</span>}
            </span>
          </>
        )
        const box = cn(
          'flex w-full min-w-0 items-start gap-2 border px-2 py-1.5 text-left text-xs',
          s.state === 'stuck' ? 'border-status-err bg-status-err/10' : 'border-border/60 bg-card/60',
        )
        return (
          <li key={s.key} data-stage={s.key} data-state={s.state} aria-current={s.state === 'current' ? 'step' : undefined}>
            {s.onOpen ? (
              <button type="button" className={cn(box, 'hover:border-foreground/40 focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none')} onClick={s.onOpen}>
                {body}
              </button>
            ) : (
              <div className={box}>{body}</div>
            )}
          </li>
        )
      })}
    </ol>
  )
}
